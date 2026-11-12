//! The trash as a place: [`Location::Trash`] lists what was deleted, under
//! the original names, with where it came from and when it was deleted.
//!
//! The entries point at the real files inside the trash (on Linux
//! `<trash>/files/<name>`, on Windows `$Recycle.Bin\<sid>\$R…`), so viewing,
//! copying out and browsing a deleted directory go through the local disk
//! like anything else. What the trash adds: restoring to the original place,
//! purging (the record of the item goes with it, see [`forget`]) and
//! emptying it all. Listing and restoring use the `trash` crate, which can do
//! it on Linux (every freedesktop trash: the home one and those on other
//! mounts) and on Windows; macOS has no API for it, so there the sidebar
//! shows `~/.Trash` as a plain directory instead.

use std::io;
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "linux", windows))]
use std::sync::Mutex;
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;

#[cfg(any(target_os = "linux", windows))]
use std::time::{Duration, UNIX_EPOCH};

use super::{Backend, Location, Meta};
use crate::fs::{Entry, EntryKind};

/// What the trash knows about one of its items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashInfo {
    /// Where the item was before it was deleted.
    pub original: PathBuf,
    pub deleted: Option<SystemTime>,
}

/// Whether this platform can list the trash (else the sidebar shows the
/// trash directory, if any).
pub const SUPPORTED: bool = cfg!(any(target_os = "linux", windows));

/// `trash:///`, as GVfs writes it.
pub const URI: &str = "trash:///";

pub fn read_only() -> io::Error {
    io::Error::other(t!("The trash is read-only: restore an item to change it"))
}

pub struct TrashBackend;

/// The one shared instance.
pub fn shared() -> Arc<dyn Backend> {
    static TRASH: OnceLock<Arc<dyn Backend>> = OnceLock::new();
    TRASH.get_or_init(|| Arc::new(TrashBackend)).clone()
}

impl Backend for TrashBackend {
    fn read_dir(&self, dir: &Location, _show_hidden: bool) -> io::Result<Vec<Entry>> {
        if *dir != Location::Trash {
            return Err(read_only());
        }
        list()
    }

    fn stat(&self, loc: &Location) -> io::Result<Meta> {
        if *loc != Location::Trash {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        Ok(Meta {
            kind: EntryKind::Dir,
            size: 0,
            modified: None,
        })
    }

    fn mkdir_all(&self, _: &Location) -> io::Result<()> {
        Err(read_only())
    }

    fn rename(&self, _: &Location, _: &Location) -> io::Result<()> {
        Err(read_only())
    }

    fn remove(&self, _: &Location) -> io::Result<()> {
        Err(read_only())
    }

    fn open_read(&self, _: &Location) -> io::Result<Box<dyn io::Read + Send>> {
        Err(read_only())
    }

    fn open_write(&self, _: &Location) -> io::Result<Box<dyn io::Write + Send>> {
        Err(read_only())
    }
}

fn trash_error(err: trash::Error) -> io::Error {
    io::Error::other(err.to_string())
}

/// The directories whose changes change the listing, from the last [`list`].
#[cfg(any(target_os = "linux", windows))]
static WATCH_DIRS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Where to watch for changes of the trash (known once it was listed).
pub fn watch_dirs() -> Vec<PathBuf> {
    #[cfg(any(target_os = "linux", windows))]
    {
        WATCH_DIRS.lock().map(|d| d.clone()).unwrap_or_default()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Vec::new()
    }
}

/// Everything in the trash, as panel entries.
#[cfg(any(target_os = "linux", windows))]
fn list() -> io::Result<Vec<Entry>> {
    let items = trash::os_limited::list().map_err(trash_error)?;
    let mut watch: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "linux")]
    if let Ok(folders) = trash::os_limited::trash_folders() {
        watch.extend(folders.into_iter().map(|f| f.join("files")));
    }
    let mut entries = Vec::with_capacity(items.len());
    for item in items {
        let Some(path) = item_path(&item) else {
            continue;
        };
        // An info record whose file is gone (deleted behind its back).
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if let Some(parent) = path.parent()
            && !watch.iter().any(|w| w == parent)
        {
            watch.push(parent.to_path_buf());
        }
        let (kind, size) = super::local::kind_of(&meta, &path);
        let (mode, uid) = super::local::unix_ids(&meta);
        let deleted = u64::try_from(item.time_deleted)
            .ok()
            .map(|secs| UNIX_EPOCH + Duration::from_secs(secs));
        entries.push(Entry {
            name: item.name.to_string_lossy().into_owned(),
            location: Location::Local(path),
            kind,
            size,
            modified: meta.modified().ok(),
            created: meta.created().ok(),
            mode,
            uid,
            is_parent: false,
            depth: 0,
            trash: Some(Arc::new(TrashInfo {
                original: item.original_path(),
                deleted,
            })),
        });
    }
    if let Ok(mut dirs) = WATCH_DIRS.lock() {
        *dirs = watch;
    }
    Ok(entries)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn list() -> io::Result<Vec<Entry>> {
    Err(io::Error::other(t!("The trash cannot be listed here")))
}

/// The file of a trash item on disk.
#[cfg(any(target_os = "linux", windows))]
fn item_path(item: &trash::TrashItem) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        // The id is `<trash>/info/<name>.trashinfo`; the file is `<trash>/files/<name>`.
        file_of_info(Path::new(&item.id))
    }
    #[cfg(windows)]
    {
        // The id is the item's parsing name: `C:\$Recycle.Bin\<sid>\$R…`.
        Some(PathBuf::from(&item.id))
    }
}

/// `<trash>/info/<name>.trashinfo` → `<trash>/files/<name>`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn file_of_info(info: &Path) -> Option<PathBuf> {
    let name = info.file_name()?.to_str()?.strip_suffix(".trashinfo")?;
    let trash = info.parent()?.parent()?;
    Some(trash.join("files").join(name))
}

/// The info record of a file directly inside a trash, when it has one:
/// `<trash>/files/<name>` → `<trash>/info/<name>.trashinfo` on Linux,
/// `…\$Recycle.Bin\<sid>\$R…` → `$I…` on Windows.
fn info_of(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let name = path.file_name()?.to_str()?;
    if cfg!(windows) {
        let rest = name.strip_prefix("$R")?;
        let in_bin = path.ancestors().any(|a| {
            a.file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("$Recycle.Bin"))
        });
        if !in_bin {
            return None;
        }
        return Some(parent.join(format!("$I{rest}")));
    }
    if parent.file_name()? != "files" {
        return None;
    }
    Some(
        parent
            .parent()?
            .join("info")
            .join(format!("{name}.trashinfo")),
    )
}

/// Whether `path` is an item of a trash (its record is there too).
pub fn is_item(path: &Path) -> bool {
    info_of(path).is_some_and(|info| info.is_file())
}

/// Whether `path` is an item of a trash or something inside one.
pub fn contains(path: &Path) -> bool {
    path.ancestors().any(is_item)
}

/// Drop the record of a trash item that was deleted or moved out, so the
/// trash does not list a ghost. Nothing for any other path.
pub fn forget(path: &Path) {
    if let Some(info) = info_of(path)
        && info.is_file()
        && let Err(err) = std::fs::remove_file(&info)
    {
        log::warn!("trash: cannot remove {}: {err}", info.display());
    }
}

/// Put trash items (their files, as listed) back where they came from. One
/// by one: a clash (something is there again) stops only that item. The
/// errors, one per item that stayed.
#[cfg(any(target_os = "linux", windows))]
pub fn restore(paths: &[PathBuf]) -> Vec<String> {
    let items = match trash::os_limited::list() {
        Ok(items) => items,
        Err(err) => return vec![err.to_string()],
    };
    let mut errors = Vec::new();
    for path in paths {
        let Some(item) = items
            .iter()
            .find(|item| item_path(item).as_deref() == Some(path.as_path()))
        else {
            errors.push(t!("{path}: not in the trash", path = path.display()));
            continue;
        };
        let original = item.original_path();
        if let Err(err) = trash::os_limited::restore_all([item.clone()]) {
            errors.push(match err {
                trash::Error::RestoreCollision { path, .. } => {
                    t!("{path} already exists", path = path.display())
                }
                other => format!("{}: {other}", original.display()),
            });
        }
    }
    errors
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn restore(_paths: &[PathBuf]) -> Vec<String> {
    vec![t!("The trash cannot be listed here").to_string()]
}

/// Delete everything in the trash for good.
#[cfg(any(target_os = "linux", windows))]
pub fn empty() -> io::Result<()> {
    let items = trash::os_limited::list().map_err(trash_error)?;
    trash::os_limited::purge_all(items).map_err(trash_error)
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn empty() -> io::Result<()> {
    Err(io::Error::other(t!("The trash cannot be listed here")))
}

/// The trash directory macOS keeps per user, shown as a plain directory.
pub fn macos_dir() -> Option<PathBuf> {
    let home = directories::UserDirs::new()?.home_dir().to_path_buf();
    Some(home.join(".Trash")).filter(|d| d.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vais-trash-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("files")).unwrap();
        std::fs::create_dir_all(dir.join("info")).unwrap();
        dir
    }

    #[cfg(not(windows))]
    #[test]
    fn items_meet_their_records() {
        let trash = sandbox("records");
        let file = trash.join("files").join("report.pdf");
        let info = trash.join("info").join("report.pdf.trashinfo");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(file_of_info(&info), Some(file.clone()));
        assert_eq!(info_of(&file), Some(info.clone()));
        // Without its record it is no trash item.
        assert!(!is_item(&file));
        std::fs::write(&info, "[Trash Info]\nPath=/tmp/report.pdf\n").unwrap();
        assert!(is_item(&file));
        assert!(contains(&file));
        assert!(contains(&file.join("inner/deep.txt")));
        assert!(!is_item(&trash.join("elsewhere").join("report.pdf")));
        assert!(!contains(Path::new("/tmp/report.pdf")));
        // Moved out or purged: the record goes too.
        std::fs::remove_file(&file).unwrap();
        forget(&file);
        assert!(!info.exists());
        std::fs::remove_dir_all(&trash).unwrap();
    }

    #[test]
    fn the_trash_is_read_only() {
        let backend = shared();
        assert!(backend.stat(&Location::Trash).unwrap().is_dir());
        assert!(backend.mkdir_all(&Location::Trash).is_err());
        assert!(backend.open_write(&Location::Trash).is_err());
    }
}
