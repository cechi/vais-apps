//! Directory watching: panels register the local directories they show and
//! get them reloaded when something else changes them (a download finishing,
//! a build writing files, another file manager).
//!
//! One `notify` watcher serves every panel; its callback runs on notify's own
//! thread and only forwards the changed paths. A foreground task maps them to
//! watched directories, waits for the burst to settle and emits one
//! [`WatchEvent::Changed`] the workspace turns into panel refreshes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::future::{Either, select};
use gpui_kit::*;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::vfs::Location;

/// Quiet time after the last event before panels reload.
const SETTLE: Duration = Duration::from_millis(400);
/// A directory that never goes quiet (a build in progress) still reloads
/// this often.
const MAX_WAIT: Duration = Duration::from_secs(2);

pub enum WatchEvent {
    /// These watched directories changed.
    Changed(Vec<Location>),
}

struct Inner {
    watcher: Mutex<Option<RecommendedWatcher>>,
    /// Watched directories and how many panels want each.
    counts: Mutex<HashMap<PathBuf, usize>>,
}

/// A cheap handle panels keep: adds and removes directories from the watch.
#[derive(Clone)]
pub struct Handle(Arc<Inner>);

impl Handle {
    fn new(tx: UnboundedSender<Vec<PathBuf>>) -> Self {
        let watcher = RecommendedWatcher::new(
            move |result: notify::Result<notify::Event>| match result {
                Ok(event) => {
                    if matches!(event.kind, EventKind::Access(_) | EventKind::Other) {
                        return;
                    }
                    if !event.paths.is_empty() {
                        tx.unbounded_send(event.paths).ok();
                    }
                }
                Err(err) => log::warn!("watch: {err}"),
            },
            notify::Config::default(),
        );
        let watcher = match watcher {
            Ok(w) => Some(w),
            Err(err) => {
                log::warn!("directory watching unavailable: {err}");
                None
            }
        };
        Self(Arc::new(Inner {
            watcher: Mutex::new(watcher),
            counts: Mutex::new(HashMap::new()),
        }))
    }

    /// Start watching `dir` (non-recursively) for one more panel.
    pub fn watch(&self, dir: &Path) {
        let Ok(mut counts) = self.0.counts.lock() else {
            return;
        };
        let count = counts.entry(dir.to_path_buf()).or_insert(0);
        *count += 1;
        if *count == 1
            && let Ok(mut watcher) = self.0.watcher.lock()
            && let Some(watcher) = watcher.as_mut()
            && let Err(err) = watcher.watch(dir, RecursiveMode::NonRecursive)
        {
            log::debug!("watch {}: {err}", dir.display());
        }
    }

    /// One panel fewer shows `dir`; the watch goes once nobody does.
    pub fn unwatch(&self, dir: &Path) {
        let Ok(mut counts) = self.0.counts.lock() else {
            return;
        };
        let Some(count) = counts.get_mut(dir) else {
            return;
        };
        *count -= 1;
        if *count == 0 {
            counts.remove(dir);
            if let Ok(mut watcher) = self.0.watcher.lock()
                && let Some(watcher) = watcher.as_mut()
            {
                let _ = watcher.unwatch(dir);
            }
        }
    }

    /// The watched directory a changed path belongs to: its parent for an
    /// entry, the directory itself when that is what changed.
    fn watched_dir(&self, path: &Path) -> Option<PathBuf> {
        let counts = self.0.counts.lock().ok()?;
        if let Some(parent) = path.parent()
            && counts.contains_key(parent)
        {
            return Some(parent.to_path_buf());
        }
        counts.contains_key(path).then(|| path.to_path_buf())
    }
}

pub struct DirWatcher {
    handle: Handle,
}

impl EventEmitter<WatchEvent> for DirWatcher {}

struct Global_(Entity<DirWatcher>);
impl Global for Global_ {}

pub fn init(cx: &mut App) -> Entity<DirWatcher> {
    let (tx, rx) = unbounded();
    let handle = Handle::new(tx);
    let watcher = cx.new(|cx| {
        cx.spawn({
            let handle = handle.clone();
            async move |this, cx| pump(this, handle, rx, cx).await
        })
        .detach();
        DirWatcher { handle }
    });
    cx.set_global(Global_(watcher.clone()));
    watcher
}

pub fn watcher(cx: &App) -> Entity<DirWatcher> {
    cx.global::<Global_>().0.clone()
}

/// The handle panels register their directories with.
pub fn handle(cx: &App) -> Handle {
    watcher(cx).read(cx).handle.clone()
}

/// Collect changed directories until the burst settles, then emit them.
async fn pump(
    this: WeakEntity<DirWatcher>,
    handle: Handle,
    mut rx: UnboundedReceiver<Vec<PathBuf>>,
    cx: &mut AsyncApp,
) {
    let mut pending: HashSet<PathBuf> = HashSet::new();
    loop {
        let Some(paths) = rx.next().await else {
            return;
        };
        let started = Instant::now();
        collect(&handle, &mut pending, paths);
        loop {
            let remaining = MAX_WAIT.saturating_sub(started.elapsed()).min(SETTLE);
            let timer = cx.background_executor().timer(remaining);
            match select(std::pin::pin!(rx.next()), std::pin::pin!(timer)).await {
                Either::Left((Some(paths), _)) => {
                    collect(&handle, &mut pending, paths);
                    if started.elapsed() >= MAX_WAIT {
                        break;
                    }
                }
                Either::Left((None, _)) => return,
                Either::Right(_) => break,
            }
        }
        if pending.is_empty() {
            continue;
        }
        let dirs: Vec<Location> = pending.drain().map(Location::Local).collect();
        if this
            .update(cx, |_, cx| cx.emit(WatchEvent::Changed(dirs)))
            .is_err()
        {
            return;
        }
    }
}

fn collect(handle: &Handle, pending: &mut HashSet<PathBuf>, paths: Vec<PathBuf>) {
    for path in paths {
        if let Some(dir) = handle.watched_dir(&path) {
            pending.insert(dir);
        }
    }
}
