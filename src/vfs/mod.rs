//! The virtual file system: where an entry lives (`Location`) and the
//! operations a storage backend offers (`Backend`). The local disk is the
//! first backend; remote connections (SFTP, FTP) plug in behind the same
//! trait so panels and operations never talk to `std::fs` directly.

pub mod local;
pub mod sftp;

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
    /// A path on a connection; always absolute, `/` separated.
    Remote {
        conn: ConnId,
        path: String,
    },
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

    /// Parse what a user typed or the config stored: `conn:/path` is remote
    /// (two or more letters before the colon, so Windows drives stay local),
    /// anything else a local path.
    pub fn parse(text: &str) -> Self {
        if let Some((conn, path)) = text.split_once(":/")
            && conn.len() >= 2
            && conn
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Location::remote(conn, format!("/{path}"));
        }
        Location::Local(PathBuf::from(text))
    }

    pub fn is_local(&self) -> bool {
        matches!(self, Location::Local(_))
    }

    pub fn as_local(&self) -> Option<&Path> {
        match self {
            Location::Local(path) => Some(path),
            Location::Remote { .. } => None,
        }
    }

    #[allow(dead_code)]
    pub fn conn(&self) -> Option<&ConnId> {
        match self {
            Location::Local(_) => None,
            Location::Remote { conn, .. } => Some(conn),
        }
    }

    /// The synthetic listing of drives (Windows); never remote.
    pub fn is_roots(&self) -> bool {
        matches!(self, Location::Local(p) if platform::is_roots_path(p))
    }

    /// The last path segment, or the whole thing for a root.
    pub fn file_name(&self) -> Option<String> {
        match self {
            Location::Local(path) => path.file_name().map(|n| n.to_string_lossy().into_owned()),
            Location::Remote { path, .. } => path
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        }
    }

    /// Name shown in titles and breadcrumbs: the last segment, the connection
    /// for a remote root, "Computer" for the drive list.
    pub fn display_name(&self) -> String {
        match self {
            Location::Local(path) => platform::display_name(path),
            Location::Remote { conn, .. } => self.file_name().unwrap_or_else(|| conn.to_string()),
        }
    }

    pub fn parent(&self) -> Option<Location> {
        match self {
            Location::Local(path) => platform::parent_of(path).map(Location::Local),
            Location::Remote { conn, path } => {
                let trimmed = path.trim_end_matches('/');
                if trimmed.is_empty() {
                    return None;
                }
                let parent = match trimmed.rfind('/') {
                    Some(0) | None => "/".to_string(),
                    Some(i) => trimmed[..i].to_string(),
                };
                Some(Location::Remote {
                    conn: conn.clone(),
                    path: parent,
                })
            }
        }
    }

    pub fn join(&self, name: &str) -> Location {
        match self {
            Location::Local(path) => Location::Local(path.join(name)),
            Location::Remote { conn, path } => Location::Remote {
                conn: conn.clone(),
                path: format!(
                    "{}/{}",
                    path.trim_end_matches('/'),
                    name.trim_start_matches('/')
                ),
            },
        }
    }

    /// Whether `self` is `base` or inside it.
    pub fn starts_with(&self, base: &Location) -> bool {
        match (self, base) {
            (Location::Local(a), Location::Local(b)) => a.starts_with(b),
            (Location::Remote { conn: a, path: ap }, Location::Remote { conn: b, path: bp }) => {
                a == b && {
                    let bp = bp.trim_end_matches('/');
                    ap == bp || ap.starts_with(&format!("{bp}/"))
                }
            }
            _ => false,
        }
    }

    /// The path of `self` relative to `base`, `/` separated, when inside it.
    pub fn relative_to(&self, base: &Location) -> Option<String> {
        match (self, base) {
            (Location::Local(a), Location::Local(b)) => a
                .strip_prefix(b)
                .ok()
                .map(|p| p.to_string_lossy().into_owned()),
            _ => {
                if !self.starts_with(base) {
                    return None;
                }
                let (Location::Remote { path: a, .. }, Location::Remote { path: b, .. }) =
                    (self, base)
                else {
                    return None;
                };
                Some(
                    a[b.trim_end_matches('/').len()..]
                        .trim_start_matches('/')
                        .to_string(),
                )
            }
        }
    }

    /// Canonical form for the path bar, the config and the clipboard.
    pub fn display(&self) -> String {
        match self {
            Location::Local(path) => path.to_string_lossy().into_owned(),
            Location::Remote { conn, path } => format!("{conn}:{path}"),
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
        .ok_or_else(|| io::Error::other("no file name"))?;
    let partial = loc
        .parent()
        .ok_or_else(|| io::Error::other("no parent directory"))?
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
pub fn local_copy(loc: &Location) -> io::Result<PathBuf> {
    let Location::Remote { conn, path } = loc else {
        return Ok(loc.as_local().unwrap().to_path_buf());
    };
    let dir = directories::ProjectDirs::from("", "", "vais-commander")
        .map(|d| d.cache_dir().join("remote").join(&conn.0))
        .ok_or_else(|| io::Error::other("no cache directory"))?;
    std::fs::create_dir_all(&dir)?;
    let backend = backend_for(loc)?;
    let meta = backend.stat(loc)?;
    let name = loc.file_name().unwrap_or_else(|| "file".into());
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        path.hash(&mut h);
        h.finish()
    };
    let target = dir.join(format!("{key:016x}-{name}"));
    let fresh = std::fs::metadata(&target)
        .is_ok_and(|m| m.len() == meta.size && m.modified().ok() == meta.modified.map(|t| t));
    if !fresh {
        let bytes = read_all(loc)?;
        std::fs::write(&target, bytes)?;
        if let Some(mtime) = meta.modified {
            let _ = std::fs::File::options()
                .write(true)
                .open(&target)
                .and_then(|f| f.set_modified(mtime));
        }
    }
    Ok(target)
}

/// The backend that serves `loc`: the disk, or the open connection.
pub fn backend_for(loc: &Location) -> io::Result<Arc<dyn Backend>> {
    match loc {
        Location::Local(_) => Ok(local::shared()),
        Location::Remote { conn, .. } => crate::connections::backend(conn)
            .ok_or_else(|| io::Error::other(format!("{conn}: not connected"))),
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
