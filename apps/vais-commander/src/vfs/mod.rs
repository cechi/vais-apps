//! The virtual file system: where an entry lives (`Location`) and the
//! operations a storage backend offers (`Backend`). The local disk is the
//! first backend; remote connections (SFTP, FTP) plug in behind the same
//! trait so panels and operations never talk to `std::fs` directly.

pub mod archive;
pub mod ftp;
pub mod local;
pub mod sftp;
pub mod trash;

use std::fmt;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::fs::{Entry, EntryKind};
use crate::platform;

/// Identifier of a saved connection (a slug from the config).
#[allow(dead_code)] // the remote backends (next steps) use the rest
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConnId(pub String);

impl fmt::Display for ConnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a file or directory is.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Location {
    Local(PathBuf),
    Remote {
        conn: ConnId,
        path: String,
    },
    /// A path inside an archive file (itself local, remote or in another
    /// archive). Written `<archive>!/<path>`.
    Archive {
        archive: Box<Location>,
        path: String,
    },
    /// The trash: what was deleted, listed under the original names (see
    /// [`trash`]). Written `trash:///`.
    Trash,
}

/// Parent of a slash-separated absolute path: `/a/b` → `/a`, `/a` → `/`,
/// `/` → none.
fn string_parent(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    Some(match trimmed.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => trimmed[..i].to_string(),
    })
}

fn string_join(path: &str, name: &str) -> String {
    format!(
        "{}/{}",
        path.trim_end_matches('/'),
        name.trim_start_matches('/')
    )
}

fn string_starts_with(path: &str, base: &str) -> bool {
    let base = base.trim_end_matches('/');
    path == base || path.starts_with(&format!("{base}/"))
}

fn string_file_name(path: &str) -> Option<String> {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl Location {
    pub fn local(path: impl Into<PathBuf>) -> Self {
        Location::Local(path.into())
    }

    pub fn remote(conn: impl Into<String>, path: impl Into<String>) -> Self {
        let mut path = path.into();
        if path.is_empty() {
            path.push('/');
        }
        Location::Remote {
            conn: ConnId(conn.into()),
            path,
        }
    }

    /// `path` inside the archive file at `archive`.
    pub fn archive(archive: Location, path: impl Into<String>) -> Self {
        let mut path = path.into();
        if !path.starts_with('/') {
            path.insert(0, '/');
        }
        Location::Archive {
            archive: Box::new(archive),
            path,
        }
    }

    pub fn parse(text: &str) -> Self {
        if text == trash::URI || text == "trash:/" {
            return Location::Trash;
        }
        if let Some((outer, inner)) = text.rsplit_once("!/")
            && !outer.is_empty()
        {
            return Location::archive(Location::parse(outer), format!("/{inner}"));
        }
        if let Some((conn, path)) = text.split_once(":/")
            && conn.len() >= 2
            && conn
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Location::remote(conn, format!("/{path}"));
        }
        // A network path, typed with either slash: `\\nas\data`, `//nas/data`.
        if cfg!(windows)
            && let Some(path) = crate::network::normalize(text)
        {
            return Location::Local(path);
        }
        Location::Local(PathBuf::from(text))
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Location::Local(_))
    }

    pub fn as_local(&self) -> Option<&Path> {
        match self {
            Location::Local(path) => Some(path),
            _ => None,
        }
    }

    pub fn is_archive(&self) -> bool {
        matches!(self, Location::Archive { .. })
    }

    /// The archive file when this is the root of an archive: what the
    /// listing of the parent directory shows for it.
    pub fn archive_file(&self) -> Option<&Location> {
        match self {
            Location::Archive { archive, path } if path == "/" => Some(archive),
            _ => None,
        }
    }

    /// The connection this depends on (through any archives).
    pub fn conn(&self) -> Option<&ConnId> {
        match self {
            Location::Local(_) => None,
            Location::Remote { conn, .. } => Some(conn),
            Location::Archive { archive, .. } => archive.conn(),
            Location::Trash => None,
        }
    }

    pub fn is_roots(&self) -> bool {
        matches!(self, Location::Local(p) if platform::is_roots_path(p))
    }

    pub fn file_name(&self) -> Option<String> {
        match self {
            Location::Local(path) => path.file_name().map(|n| n.to_string_lossy().into_owned()),
            Location::Remote { path, .. } => string_file_name(path),
            Location::Archive { archive, path } => {
                string_file_name(path).or_else(|| archive.file_name())
            }
            Location::Trash => None,
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            Location::Local(path) => platform::display_name(path),
            Location::Remote { conn, .. } => self.file_name().unwrap_or_else(|| conn.to_string()),
            Location::Archive { archive, .. } => {
                self.file_name().unwrap_or_else(|| archive.display_name())
            }
            Location::Trash => t!("Trash##place").to_string(),
        }
    }

    /// The directory above: the archive root's parent is the directory
    /// holding the archive file, a deleted directory's is the trash.
    pub fn parent(&self) -> Option<Location> {
        match self {
            Location::Local(path) if trash::is_item(path) => Some(Location::Trash),
            Location::Local(path) => platform::parent_of(path).map(Location::Local),
            Location::Remote { conn, path } => Some(Location::Remote {
                conn: conn.clone(),
                path: string_parent(path)?,
            }),
            Location::Archive { archive, path } => match string_parent(path) {
                Some(parent) => Some(Location::Archive {
                    archive: archive.clone(),
                    path: parent,
                }),
                None => archive.parent(),
            },
            Location::Trash => None,
        }
    }

    pub fn join(&self, name: &str) -> Location {
        match self {
            Location::Local(path) => Location::Local(path.join(name)),
            Location::Remote { conn, path } => Location::Remote {
                conn: conn.clone(),
                path: string_join(path, name),
            },
            Location::Archive { archive, path } => Location::Archive {
                archive: archive.clone(),
                path: string_join(path, name),
            },
            // Nothing can be made in the trash; its backend refuses it.
            Location::Trash => Location::Trash,
        }
    }

    pub fn starts_with(&self, base: &Location) -> bool {
        match (self, base) {
            (Location::Local(a), Location::Local(b)) => a.starts_with(b),
            (Location::Remote { conn: a, path: ap }, Location::Remote { conn: b, path: bp }) => {
                a == b && string_starts_with(ap, bp)
            }
            (
                Location::Archive {
                    archive: a,
                    path: ap,
                },
                Location::Archive {
                    archive: b,
                    path: bp,
                },
            ) => a == b && string_starts_with(ap, bp),
            // Everything inside an archive is below the archive file.
            (Location::Archive { archive, .. }, other) => archive.starts_with(other),
            (Location::Trash, Location::Trash) => true,
            _ => false,
        }
    }

    pub fn relative_to(&self, base: &Location) -> Option<String> {
        if let (Location::Local(a), Location::Local(b)) = (self, base) {
            return a
                .strip_prefix(b)
                .ok()
                .map(|p| p.to_string_lossy().into_owned());
        }
        if !self.starts_with(base) {
            return None;
        }
        let (a, b) = match (self, base) {
            (Location::Remote { path: a, .. }, Location::Remote { path: b, .. }) => (a, b),
            (Location::Archive { path: a, .. }, Location::Archive { path: b, .. }) => (a, b),
            _ => return None,
        };
        Some(
            a[b.trim_end_matches('/').len()..]
                .trim_start_matches('/')
                .to_string(),
        )
    }

    pub fn display(&self) -> String {
        match self {
            Location::Local(path) => path.to_string_lossy().into_owned(),
            Location::Remote { conn, path } => format!("{conn}:{path}"),
            Location::Archive { archive, path } => format!("{}!{path}", archive.display()),
            Location::Trash => trash::URI.to_string(),
        }
    }
}

impl From<PathBuf> for Location {
    fn from(path: PathBuf) -> Self {
        Location::Local(path)
    }
}

impl From<&Path> for Location {
    fn from(path: &Path) -> Self {
        Location::Local(path.to_path_buf())
    }
}

impl From<&str> for Location {
    fn from(text: &str) -> Self {
        Location::parse(text)
    }
}

impl Serialize for Location {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.display())
    }
}

impl<'de> Deserialize<'de> for Location {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Ok(Location::parse(&text))
    }
}

/// What a backend knows about one entry.
#[derive(Debug, Clone)]
pub struct Meta {
    pub kind: EntryKind,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl Meta {
    pub fn is_dir(&self) -> bool {
        matches!(
            self.kind,
            EntryKind::Dir | EntryKind::Symlink { to_dir: true }
        )
    }
}

/// A storage: the local disk or one remote connection. All calls block and
/// are meant for the background executor.
pub trait Backend: Send + Sync {
    /// The entries of a directory, unsorted and without `..`.
    fn read_dir(&self, dir: &Location, show_hidden: bool) -> io::Result<Vec<Entry>>;
    /// Metadata without following symlinks.
    fn stat(&self, loc: &Location) -> io::Result<Meta>;
    fn mkdir_all(&self, dir: &Location) -> io::Result<()>;
    fn rename(&self, from: &Location, to: &Location) -> io::Result<()>;
    /// Remove a file, a link or a whole directory tree.
    fn remove(&self, loc: &Location) -> io::Result<()>;
    fn open_read(&self, loc: &Location) -> io::Result<Box<dyn Read + Send>>;
    fn open_write(&self, loc: &Location) -> io::Result<Box<dyn Write + Send>>;

    fn exists(&self, loc: &Location) -> bool {
        self.stat(loc).is_ok()
    }
}

/// Read a whole file through its backend.
pub fn read_all(loc: &Location) -> io::Result<Vec<u8>> {
    let mut reader = backend_for(loc)?.open_read(loc)?;
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Write a whole file through its backend: to a sibling temporary name
/// first, then renamed over the target, so a failure leaves the old file.
pub fn write_all(loc: &Location, bytes: &[u8]) -> io::Result<()> {
    let backend = backend_for(loc)?;
    let name = loc
        .file_name()
        .ok_or_else(|| io::Error::other(t!("no file name")))?;
    let partial = loc
        .parent()
        .ok_or_else(|| io::Error::other(t!("no parent directory")))?
        .join(&format!(".{name}.vais-save"));
    let result = (|| {
        let mut writer = backend.open_write(&partial)?;
        writer.write_all(bytes)?;
        writer.flush()?;
        drop(writer);
        backend.rename(&partial, loc)
    })();
    if result.is_err() {
        let _ = backend.remove(&partial);
    }
    result
}

/// A local copy of `loc` for tools that need a real file (the default
/// application, thumbnails): the path itself when local, otherwise a
/// download into the cache directory, refreshed when size or mtime differ.
/// A copy of a remote (or archived) file on disk, in the cache, refreshed
/// when the original's size or time changed. A local file is itself.
pub fn local_copy(loc: &Location) -> io::Result<PathBuf> {
    let (sub, key_text) = match loc {
        Location::Local(path) => return Ok(path.clone()),
        Location::Remote { conn, path } => (conn.0.clone(), path.clone()),
        Location::Archive { .. } => ("archive".to_string(), loc.display()),
        Location::Trash => return Err(trash::read_only()),
    };
    let dir = directories::ProjectDirs::from("", "", "vais-commander")
        .map(|d| d.cache_dir().join("remote").join(sub))
        .ok_or_else(|| io::Error::other(t!("no cache directory")))?;
    std::fs::create_dir_all(&dir)?;
    let backend = backend_for(loc)?;
    let meta = backend.stat(loc)?;
    let name = loc.file_name().unwrap_or_else(|| "file".into());
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        key_text.hash(&mut h);
        h.finish()
    };
    let target = dir.join(format!("{key:016x}-{name}"));
    let fresh = std::fs::metadata(&target)
        .is_ok_and(|m| m.len() == meta.size && m.modified().ok() == meta.modified);
    if !fresh {
        let mut reader = backend.open_read(loc)?;
        let partial = target.with_extension("vais-part");
        let mut out = std::fs::File::create(&partial)?;
        if let Err(err) = io::copy(&mut reader, &mut out) {
            drop(out);
            let _ = std::fs::remove_file(&partial);
            return Err(err);
        }
        drop(out);
        std::fs::rename(&partial, &target)?;
        if let Some(mtime) = meta.modified {
            let _ = std::fs::File::options()
                .write(true)
                .open(&target)
                .and_then(|f| f.set_modified(mtime));
        }
    }
    Ok(target)
}

pub fn backend_for(loc: &Location) -> io::Result<Arc<dyn Backend>> {
    match loc {
        Location::Local(_) => Ok(local::shared()),
        Location::Remote { conn, .. } => crate::connections::backend(conn)
            .ok_or_else(|| io::Error::other(t!("{conn}: not connected", conn = conn))),
        Location::Archive { archive, .. } => archive::open(archive),
        Location::Trash => Ok(trash::shared()),
    }
}

#[cfg(test)]
mod tests {
    use super::Location;
    use std::path::PathBuf;

    #[test]
    fn parses_local_and_remote() {
        assert_eq!(Location::parse("/home/x"), Location::local("/home/x"));
        assert_eq!(Location::parse("C:/Users"), Location::local("C:/Users"));
        assert_eq!(
            Location::parse("nas:/srv/www"),
            Location::remote("nas", "/srv/www")
        );
        assert_eq!(Location::parse("nas:/"), Location::remote("nas", "/"));
        assert_eq!(Location::parse("nas:/srv/www").display(), "nas:/srv/www");
    }

    /// Typed network paths and the paths `std` builds for the entries of a
    /// share are the same locations.
    #[cfg(windows)]
    #[test]
    fn network_paths_meet_the_listed_ones() {
        let share = Location::parse("//nas/data");
        assert_eq!(share, Location::local(r"\\nas\data\"));
        let dir = Location::local(PathBuf::from(r"\\nas\data\").join("a"));
        assert_eq!(dir, Location::parse(r"\\nas\data\a"));
        assert_eq!(dir.parent(), Some(share.clone()));
        assert_eq!(share.parent(), Some(Location::local(r"\\nas")));
        assert!(Location::local(r"\\nas").parent().unwrap().is_roots());
    }

    #[test]
    fn archive_paths() {
        let zip = Location::local("/tmp/a.zip");
        let root = Location::archive(zip.clone(), "/");
        assert_eq!(Location::parse("/tmp/a.zip!/"), root);
        assert_eq!(root.display(), "/tmp/a.zip!/");
        assert_eq!(root.file_name().as_deref(), Some("a.zip"));
        assert_eq!(root.parent(), Some(Location::local("/tmp")));
        assert_eq!(root.archive_file(), Some(&zip));
        let sub = root.join("src").join("main.rs");
        assert_eq!(sub.display(), "/tmp/a.zip!/src/main.rs");
        assert_eq!(Location::parse("/tmp/a.zip!/src/main.rs"), sub);
        assert_eq!(sub.file_name().as_deref(), Some("main.rs"));
        assert_eq!(sub.parent(), Some(root.join("src")));
        assert_eq!(sub.archive_file(), None);
        assert!(sub.starts_with(&root));
        assert!(sub.starts_with(&Location::local("/tmp")));
        assert_eq!(sub.relative_to(&root).as_deref(), Some("src/main.rs"));
        // On a connection, through to the connection.
        let remote = Location::parse("nas:/x.tar.gz!/dir");
        assert_eq!(remote.conn().map(|c| c.0.as_str()), Some("nas"));
        assert_eq!(remote.display(), "nas:/x.tar.gz!/dir");
        assert!(!sub.is_local());
        assert!(sub.as_local().is_none());
    }

    #[test]
    fn the_trash_is_a_place_of_its_own() {
        assert_eq!(Location::parse("trash:///"), Location::Trash);
        assert_eq!(Location::parse("trash:/"), Location::Trash);
        assert_eq!(Location::Trash.display(), "trash:///");
        assert_eq!(Location::Trash.parent(), None);
        assert!(!Location::Trash.is_local());
        assert_eq!(Location::Trash.conn(), None);
        // A connection called "trash" still has its paths.
        assert_eq!(
            Location::parse("trash:/srv"),
            Location::remote("trash", "/srv")
        );
    }

    #[test]
    fn remote_paths_navigate_like_local_ones() {
        let www = Location::remote("nas", "/srv/www");
        assert_eq!(www.file_name().as_deref(), Some("www"));
        assert_eq!(www.parent(), Some(Location::remote("nas", "/srv")));
        assert_eq!(
            Location::remote("nas", "/srv").parent(),
            Some(Location::remote("nas", "/"))
        );
        assert_eq!(Location::remote("nas", "/").parent(), None);
        assert_eq!(www.join("a.txt"), Location::remote("nas", "/srv/www/a.txt"));
        assert!(www.join("a").starts_with(&www));
        assert!(!Location::remote("nas", "/srv/www2").starts_with(&www));
        assert_eq!(www.join("a/b").relative_to(&www).as_deref(), Some("a/b"));
        assert_eq!(Location::remote("nas", "/").display_name(), "nas");
        assert_eq!(
            Location::local(PathBuf::from("/a/b")).relative_to(&Location::local("/a")),
            Some("b".into())
        );
    }
}
