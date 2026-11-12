//! Directory watching: panels register the local directories they show and
//! get them reloaded when something else changes them (a download finishing,
//! a build writing files, another file manager).
//!
//! One `notify` watcher serves every panel. Its callback runs on notify's own
//! thread: it maps the changed paths to watched directories and collects
//! them, so a copy writing a big file (an event per chunk) costs the UI
//! thread nothing. Only the first change of a burst wakes a foreground task,
//! which waits for the burst to settle and emits one [`WatchEvent::Changed`]
//! the workspace turns into panel refreshes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
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

/// What notify's thread and the UI share.
#[derive(Default)]
struct Shared {
    /// Watched directories and how many panels want each.
    counts: Mutex<HashMap<PathBuf, usize>>,
    /// Changes not yet handed to the panels.
    pending: Mutex<Pending>,
}

#[derive(Default)]
struct Pending {
    dirs: HashSet<PathBuf>,
    /// When the last change came in.
    last: Option<Instant>,
}

impl Shared {
    /// The watched directory a changed path belongs to: its parent for an
    /// entry, the directory itself when that is what changed.
    fn watched_dir(&self, path: &Path) -> Option<PathBuf> {
        let counts = self.counts.lock().ok()?;
        if let Some(parent) = path.parent()
            && counts.contains_key(parent)
        {
            return Some(parent.to_path_buf());
        }
        counts.contains_key(path).then(|| path.to_path_buf())
    }

    /// Note the changed paths; true when they start a burst (nothing was
    /// pending), which is when the foreground task has to be woken.
    fn collect(&self, paths: &[PathBuf]) -> bool {
        let dirs: Vec<PathBuf> = paths.iter().filter_map(|p| self.watched_dir(p)).collect();
        if dirs.is_empty() {
            return false;
        }
        let Ok(mut pending) = self.pending.lock() else {
            return false;
        };
        let starts = pending.dirs.is_empty();
        pending.dirs.extend(dirs);
        pending.last = Some(Instant::now());
        starts
    }

    fn last_change(&self) -> Option<Instant> {
        self.pending.lock().ok().and_then(|p| p.last)
    }

    fn take(&self) -> Vec<PathBuf> {
        self.pending
            .lock()
            .map(|mut p| p.dirs.drain().collect())
            .unwrap_or_default()
    }
}

struct Inner {
    watcher: Mutex<Option<RecommendedWatcher>>,
    shared: Arc<Shared>,
}

/// A cheap handle panels keep: adds and removes directories from the watch.
#[derive(Clone)]
pub struct Handle(Arc<Inner>);

impl Handle {
    fn new(wake: UnboundedSender<()>) -> Self {
        let shared = Arc::new(Shared::default());
        let watcher = RecommendedWatcher::new(
            {
                let shared = shared.clone();
                move |result: notify::Result<notify::Event>| match result {
                    Ok(event) => {
                        if matches!(event.kind, EventKind::Access(_) | EventKind::Other) {
                            return;
                        }
                        if shared.collect(&event.paths) {
                            wake.unbounded_send(()).ok();
                        }
                    }
                    Err(err) => log::warn!("watch: {err}"),
                }
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
            shared,
        }))
    }

    /// Start watching `dir` (non-recursively) for one more panel.
    pub fn watch(&self, dir: &Path) {
        // The watcher lock orders watch and unwatch; the counts lock is not
        // held while notify works, as notify's thread takes it for events.
        let Ok(mut watcher) = self.0.watcher.lock() else {
            return;
        };
        let first = {
            let Ok(mut counts) = self.0.shared.counts.lock() else {
                return;
            };
            let count = counts.entry(dir.to_path_buf()).or_insert(0);
            *count += 1;
            *count == 1
        };
        if first
            && let Some(watcher) = watcher.as_mut()
            && let Err(err) = watcher.watch(dir, RecursiveMode::NonRecursive)
        {
            log::debug!("watch {}: {err}", dir.display());
        }
    }

    /// One panel fewer shows `dir`; the watch goes once nobody does.
    pub fn unwatch(&self, dir: &Path) {
        let Ok(mut watcher) = self.0.watcher.lock() else {
            return;
        };
        let last = {
            let Ok(mut counts) = self.0.shared.counts.lock() else {
                return;
            };
            let Some(count) = counts.get_mut(dir) else {
                return;
            };
            *count -= 1;
            *count == 0 && counts.remove(dir).is_some()
        };
        if last && let Some(watcher) = watcher.as_mut() {
            let _ = watcher.unwatch(dir);
        }
    }
}

pub struct DirWatcher {
    handle: Handle,
}

impl EventEmitter<WatchEvent> for DirWatcher {}

struct Global_(Entity<DirWatcher>);
impl Global for Global_ {}

pub fn init(cx: &mut App) -> Entity<DirWatcher> {
    let (wake, woken) = unbounded();
    let handle = Handle::new(wake);
    let watcher = cx.new(|cx| {
        cx.spawn({
            let shared = handle.0.shared.clone();
            async move |this, cx| pump(this, shared, woken, cx).await
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

/// Wait for each burst of changes to settle, then emit its directories.
async fn pump(
    this: WeakEntity<DirWatcher>,
    shared: Arc<Shared>,
    mut woken: UnboundedReceiver<()>,
    cx: &mut AsyncApp,
) {
    loop {
        if woken.next().await.is_none() {
            return;
        }
        let started = Instant::now();
        loop {
            let last = shared.last_change().unwrap_or(started);
            let due = (last + SETTLE).min(started + MAX_WAIT);
            let now = Instant::now();
            if now >= due {
                break;
            }
            cx.background_executor().timer(due - now).await;
        }
        let dirs: Vec<Location> = shared.take().into_iter().map(Location::Local).collect();
        if dirs.is_empty() {
            continue;
        }
        if this
            .update(cx, |_, cx| cx.emit(WatchEvent::Changed(dirs)))
            .is_err()
        {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's `test` attribute would shadow the plain one.
    use super::Shared;
    use std::path::PathBuf;

    #[test]
    fn a_burst_wakes_once() {
        let shared = Shared::default();
        let dir = PathBuf::from("/watched");
        shared.counts.lock().unwrap().insert(dir.clone(), 1);
        // Not watched: nothing to wake for.
        assert!(!shared.collect(&[PathBuf::from("/elsewhere/a")]));
        assert!(shared.collect(&[dir.join("a")]));
        assert!(!shared.collect(&[dir.join("b"), dir.clone()]));
        assert_eq!(shared.take(), vec![dir.clone()]);
        // Handed over: the next change starts a new burst.
        assert!(shared.collect(&[dir.join("c")]));
    }
}
