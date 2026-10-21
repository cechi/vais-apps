//! File operations (copy, move, delete). Pure `std::fs` work meant to run on
//! the background executor; progress and cancellation are shared through
//! `Mutex`/`AtomicBool` so the UI can poll them.

pub mod manager;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Copy,
    Move,
    Delete { permanent: bool },
}

impl OpKind {
    /// Imperative label for dialogs and buttons.
    pub fn verb(self) -> &'static str {
        match self {
            OpKind::Copy => "Copy",
            OpKind::Move => "Move",
            OpKind::Delete { permanent: true } => "Delete permanently",
            OpKind::Delete { permanent: false } => "Move to trash",
        }
    }

    /// Progressive label for the status bar.
    pub fn progress_verb(self) -> &'static str {
        match self {
            OpKind::Copy => "Copying",
            OpKind::Move => "Moving",
            OpKind::Delete { .. } => "Deleting",
        }
    }

    /// Past tense for the final notification.
    pub fn done_verb(self) -> &'static str {
        match self {
            OpKind::Copy => "Copied",
            OpKind::Move => "Moved",
            OpKind::Delete { permanent: true } => "Deleted",
            OpKind::Delete { permanent: false } => "Trashed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    pub kind: OpKind,
    pub sources: Vec<PathBuf>,
    /// Destination directory for copy and move.
    pub dest: Option<PathBuf>,
    /// Replace existing files at the destination; otherwise they are skipped.
    pub overwrite: bool,
}

impl Request {
    pub fn clone_with_overwrite(&self, overwrite: bool) -> Self {
        Self {
            overwrite,
            ..self.clone()
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Progress {
    /// Entries handled so far and in total (files, directories, links).
    pub done: usize,
    pub total: usize,
    /// Bytes copied so far and in total; zero for deletions.
    pub bytes: u64,
    pub total_bytes: u64,
    pub current: String,
}

#[derive(Debug, Default)]
pub struct Report {
    /// Top-level sources handled without error.
    pub done: usize,
    pub errors: Vec<String>,
    pub cancelled: bool,
}

/// "3 items" or the single file name, for dialog titles.
pub fn describe(sources: &[PathBuf]) -> String {
    match sources {
        [single] => single
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| single.to_string_lossy().into_owned()),
        many => format!("{} items", many.len()),
    }
}

/// Destination paths that already exist.
pub fn conflicts(sources: &[PathBuf], dest: &Path) -> Vec<PathBuf> {
    sources
        .iter()
        .filter_map(|src| src.file_name().map(|name| dest.join(name)))
        .filter(|dst| dst.symlink_metadata().is_ok())
        .collect()
}

pub fn run(req: &Request, progress: &Mutex<Progress>, cancel: &AtomicBool) -> Report {
    let mut report = Report::default();
    let (total, total_bytes) = match req.kind {
        OpKind::Copy | OpKind::Move => req.sources.iter().fold((0, 0), |(n, b), s| {
            let (items, bytes) = count_items(s);
            (n + items, b + bytes)
        }),
        OpKind::Delete { .. } => (req.sources.len(), 0),
    };
    if let Ok(mut p) = progress.lock() {
        p.total = total;
        p.total_bytes = total_bytes;
        p.done = 0;
        p.bytes = 0;
    }

    for src in &req.sources {
        if cancel.load(Ordering::Relaxed) {
            report.cancelled = true;
            break;
        }
        let result = match req.kind {
            OpKind::Copy => transfer(src, req, false, progress, cancel),
            OpKind::Move => transfer(src, req, true, progress, cancel),
            OpKind::Delete { permanent } => {
                set_current(progress, src);
                let r = delete_one(src, permanent);
                bump(progress);
                r
            }
        };
        match result {
            Ok(()) => report.done += 1,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {
                report.cancelled = true;
                break;
            }
            Err(err) => report.errors.push(format!("{}: {err}", src.display())),
        }
    }
    report
}

fn transfer(
    src: &Path,
    req: &Request,
    remove_source: bool,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let dest = req
        .dest
        .as_deref()
        .ok_or_else(|| io::Error::other("no destination"))?;
    let name = src
        .file_name()
        .ok_or_else(|| io::Error::other("source has no file name"))?;
    let dst = dest.join(name);
    if dst == src {
        return Err(io::Error::other("source and destination are the same"));
    }
    if src.is_dir() && dst.starts_with(src) {
        return Err(io::Error::other("cannot copy a directory into itself"));
    }
    if remove_source {
        move_tree(src, &dst, req.overwrite, progress, cancel)
    } else {
        copy_tree(src, &dst, req.overwrite, progress, cancel)
    }
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "cancelled")
}

fn set_current(progress: &Mutex<Progress>, path: &Path) {
    if let Ok(mut p) = progress.lock() {
        p.current = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
}

fn bump(progress: &Mutex<Progress>) {
    if let Ok(mut p) = progress.lock() {
        p.done += 1;
    }
}

fn bump_by(progress: &Mutex<Progress>, items: usize, bytes: u64) {
    if let Ok(mut p) = progress.lock() {
        p.done += items;
        p.bytes += bytes;
    }
}

fn add_bytes(progress: &Mutex<Progress>, bytes: u64) {
    if let Ok(mut p) = progress.lock() {
        p.bytes += bytes;
    }
}

/// Number of entries under `path` (itself included) and the bytes of regular files.
pub fn count_items(path: &Path) -> (usize, u64) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return (1, 0);
    };
    if meta.file_type().is_symlink() {
        return (1, 0);
    }
    if !meta.is_dir() {
        return (1, meta.len());
    }
    let (items, bytes) = fs::read_dir(path)
        .map(|rd| {
            rd.flatten().fold((0, 0), |(n, b), e| {
                let (items, bytes) = count_items(&e.path());
                (n + items, b + bytes)
            })
        })
        .unwrap_or((0, 0));
    (1 + items, bytes)
}

const COPY_BUFFER: usize = 1 << 20;

/// Copy one regular file in chunks, reporting bytes as they land, so a long
/// copy shows progress and speed. Keeps the source permissions.
fn copy_file(
    src: &Path,
    dst: &Path,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    use std::io::{Read, Write};
    let mut reader = fs::File::open(src)?;
    let permissions = reader.metadata()?.permissions();
    let mut writer = fs::File::create(dst)?;
    let mut buffer = vec![0u8; COPY_BUFFER];
    loop {
        if cancel.load(Ordering::Relaxed) {
            drop(writer);
            let _ = fs::remove_file(dst);
            return Err(cancelled());
        }
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buffer[..n])?;
        add_bytes(progress, n as u64);
    }
    writer.flush()?;
    let _ = fs::set_permissions(dst, permissions);
    Ok(())
}

fn copy_tree(
    src: &Path,
    dst: &Path,
    overwrite: bool,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }
    set_current(progress, src);
    let meta = fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        copy_symlink(src, dst, overwrite)?;
        bump(progress);
        return Ok(());
    }
    if meta.is_dir() {
        fs::create_dir_all(dst)?;
        bump(progress);
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(
                &entry.path(),
                &dst.join(entry.file_name()),
                overwrite,
                progress,
                cancel,
            )?;
        }
        return Ok(());
    }
    if let Ok(existing) = fs::symlink_metadata(dst) {
        if !overwrite {
            bump_by(progress, 1, meta.len());
            return Ok(());
        }
        if existing.is_dir() {
            return Err(io::Error::other(format!(
                "{} exists and is a directory",
                dst.display()
            )));
        }
    }
    copy_file(src, dst, progress, cancel)?;
    bump(progress);
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(src: &Path, dst: &Path, overwrite: bool) -> io::Result<()> {
    let target = fs::read_link(src)?;
    if fs::symlink_metadata(dst).is_ok() {
        if !overwrite {
            return Ok(());
        }
        remove_any(dst)?;
    }
    std::os::unix::fs::symlink(target, dst)
}

#[cfg(not(unix))]
fn copy_symlink(src: &Path, dst: &Path, overwrite: bool) -> io::Result<()> {
    // Creating symlinks needs privileges on Windows; copy what the link points to.
    if fs::symlink_metadata(dst).is_ok() && !overwrite {
        return Ok(());
    }
    if src.is_dir() {
        fs::create_dir_all(dst)
    } else {
        fs::copy(src, dst).map(|_| ())
    }
}

fn move_tree(
    src: &Path,
    dst: &Path,
    overwrite: bool,
    progress: &Mutex<Progress>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }
    set_current(progress, src);
    let (items, bytes) = count_items(src);
    if let Ok(existing) = fs::symlink_metadata(dst) {
        let src_is_dir = src.is_dir() && !fs::symlink_metadata(src)?.file_type().is_symlink();
        if existing.is_dir() && src_is_dir {
            // Merge directories: copy over, then remove the source.
            copy_tree(src, dst, overwrite, progress, cancel)?;
            return remove_any(src);
        }
        if !overwrite {
            bump_by(progress, items, bytes);
            return Ok(());
        }
        remove_any(dst)?;
    }
    match fs::rename(src, dst) {
        Ok(()) => {
            bump_by(progress, items, bytes);
            Ok(())
        }
        Err(_) => {
            // Typically a different file system: copy, then remove the source.
            copy_tree(src, dst, overwrite, progress, cancel)?;
            remove_any(src)
        }
    }
}

fn delete_one(path: &Path, permanent: bool) -> io::Result<()> {
    if permanent {
        remove_any(path)
    } else {
        trash::delete(path).map_err(|e| io::Error::other(e.to_string()))
    }
}

/// Remove a file, symlink or directory tree.
fn remove_any(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn sandbox() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("vais-ops-test-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn run_ok(req: Request) -> (Report, Progress) {
        let progress = Mutex::new(Progress::default());
        let cancel = AtomicBool::new(false);
        let report = run(&req, &progress, &cancel);
        let p = progress.lock().unwrap().clone();
        (report, p)
    }

    #[test]
    fn copies_a_tree_and_counts_progress() {
        let root = sandbox();
        write(&root.join("src/a.txt"), "a");
        write(&root.join("src/sub/b.txt"), "b");
        let dest = root.join("dest");
        fs::create_dir_all(&dest).unwrap();

        let (report, progress) = run_ok(Request {
            kind: OpKind::Copy,
            sources: vec![root.join("src")],
            dest: Some(dest.clone()),
            overwrite: false,
        });
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.done, 1);
        assert_eq!(progress.total, 4);
        assert_eq!(progress.done, 4);
        assert_eq!(progress.total_bytes, 2);
        assert_eq!(progress.bytes, 2);
        assert_eq!(fs::read_to_string(dest.join("src/sub/b.txt")).unwrap(), "b");
        assert!(root.join("src/a.txt").exists(), "copy keeps the source");
    }

    #[test]
    fn skips_or_overwrites_conflicts() {
        let root = sandbox();
        write(&root.join("src/f.txt"), "new");
        write(&root.join("dest/f.txt"), "old");
        let dest = root.join("dest");
        assert_eq!(conflicts(&[root.join("src/f.txt")], &dest).len(), 1);

        run_ok(Request {
            kind: OpKind::Copy,
            sources: vec![root.join("src/f.txt")],
            dest: Some(dest.clone()),
            overwrite: false,
        });
        assert_eq!(fs::read_to_string(dest.join("f.txt")).unwrap(), "old");

        run_ok(Request {
            kind: OpKind::Copy,
            sources: vec![root.join("src/f.txt")],
            dest: Some(dest.clone()),
            overwrite: true,
        });
        assert_eq!(fs::read_to_string(dest.join("f.txt")).unwrap(), "new");
    }

    #[test]
    fn moves_and_deletes() {
        let root = sandbox();
        write(&root.join("src/dir/x.txt"), "x");
        let dest = root.join("dest");
        fs::create_dir_all(&dest).unwrap();

        let (report, _) = run_ok(Request {
            kind: OpKind::Move,
            sources: vec![root.join("src/dir")],
            dest: Some(dest.clone()),
            overwrite: false,
        });
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(!root.join("src/dir").exists());
        assert_eq!(fs::read_to_string(dest.join("dir/x.txt")).unwrap(), "x");

        let (report, _) = run_ok(Request {
            kind: OpKind::Delete { permanent: true },
            sources: vec![dest.join("dir")],
            dest: None,
            overwrite: false,
        });
        assert_eq!(report.done, 1);
        assert!(!dest.join("dir").exists());
    }

    #[test]
    fn refuses_self_copy_and_reports_errors() {
        let root = sandbox();
        write(&root.join("f.txt"), "f");
        let (report, _) = run_ok(Request {
            kind: OpKind::Copy,
            sources: vec![root.join("f.txt"), root.join("missing.txt")],
            dest: Some(root.clone()),
            overwrite: true,
        });
        assert_eq!(report.done, 0);
        assert_eq!(report.errors.len(), 2);
        assert_eq!(fs::read_to_string(root.join("f.txt")).unwrap(), "f");
    }

    #[test]
    fn cancel_stops_early() {
        let root = sandbox();
        write(&root.join("a.txt"), "a");
        write(&root.join("b.txt"), "b");
        let dest = root.join("dest");
        fs::create_dir_all(&dest).unwrap();
        let progress = Mutex::new(Progress::default());
        let cancel = AtomicBool::new(true);
        let report = run(
            &Request {
                kind: OpKind::Copy,
                sources: vec![root.join("a.txt"), root.join("b.txt")],
                dest: Some(dest.clone()),
                overwrite: false,
            },
            &progress,
            &cancel,
        );
        assert!(report.cancelled);
        assert!(!dest.join("a.txt").exists());
    }

    #[test]
    fn describes_selection() {
        assert_eq!(describe(&[PathBuf::from("/x/y.txt")]), "y.txt");
        assert_eq!(
            describe(&[PathBuf::from("/a"), PathBuf::from("/b")]),
            "2 items"
        );
    }
}
