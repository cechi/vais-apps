//! Archives (zip, tar, compressed tar and 7z) browsed as directories: a
//! read-only [`Backend`] over one archive file. A zip is indexed from its
//! central directory and entries are extracted into the cache when first
//! read; a tar has no index, so it is unpacked into the cache in the one pass
//! that lists it, and so is a 7z (usually solid: one entry cannot be read
//! without decoding everything before it).
//!
//! Opened archives are kept in a process-wide table keyed by the archive's
//! location and invalidated when its size or modification time changes.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zip::ZipArchive;

use super::{Backend, Location, Meta};
use crate::fs::{Entry, EntryKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Zip,
    Tar(Compression),
    SevenZ,
}

/// File name endings of the formats we read (lowercase).
const SUFFIXES: &[(&str, Kind)] = &[
    (".zip", Kind::Zip),
    (".jar", Kind::Zip),
    (".war", Kind::Zip),
    (".ear", Kind::Zip),
    (".apk", Kind::Zip),
    (".ipa", Kind::Zip),
    (".xpi", Kind::Zip),
    (".whl", Kind::Zip),
    (".crx", Kind::Zip),
    (".tar.gz", Kind::Tar(Compression::Gzip)),
    (".tgz", Kind::Tar(Compression::Gzip)),
    (".tar.bz2", Kind::Tar(Compression::Bzip2)),
    (".tbz2", Kind::Tar(Compression::Bzip2)),
    (".tbz", Kind::Tar(Compression::Bzip2)),
    (".tar.xz", Kind::Tar(Compression::Xz)),
    (".txz", Kind::Tar(Compression::Xz)),
    (".tar.zst", Kind::Tar(Compression::Zstd)),
    (".tzst", Kind::Tar(Compression::Zstd)),
    (".tar", Kind::Tar(Compression::None)),
    (".7z", Kind::SevenZ),
];

/// The matching ending of `name` (its length in bytes) and the format.
fn suffix_of(name: &str) -> Option<(usize, Kind)> {
    let lower = name.to_ascii_lowercase();
    SUFFIXES
        .iter()
        .find(|(suffix, _)| lower.ends_with(suffix))
        .map(|(suffix, kind)| (suffix.len(), *kind))
}

/// The archive format a file name says it is, if any.
pub fn kind_of(name: &str) -> Option<Kind> {
    suffix_of(name).map(|(_, kind)| kind)
}

/// The name of an archive without its format ending (`site.tar.gz` →
/// `site`): the folder its contents go into. `None` for other files and for
/// a bare ending (`.zip`).
pub fn stem(name: &str) -> Option<&str> {
    let (len, _) = suffix_of(name)?;
    let stem = &name[..name.len() - len];
    (!stem.is_empty()).then_some(stem)
}

/// One entry of the archive, keyed by its inner path without a leading
/// slash (the root is the empty string).
#[derive(Debug, Clone)]
struct Node {
    kind: EntryKind,
    size: u64,
    modified: Option<SystemTime>,
    mode: Option<u32>,
    /// Index in the zip's central directory (zip only).
    index: Option<usize>,
    children: Vec<String>,
}

impl Node {
    fn dir() -> Self {
        Node {
            kind: EntryKind::Dir,
            size: 0,
            modified: None,
            mode: Some(0o040755),
            index: None,
            children: Vec::new(),
        }
    }
}

pub struct ArchiveBackend {
    /// The archive file as a location (root of every inner path).
    outer: Location,
    /// Where extracted entries live.
    cache_dir: PathBuf,
    nodes: HashMap<String, Node>,
    zip: Option<Mutex<ZipArchive<File>>>,
}

fn read_only() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        t!("archives are read-only"),
    )
}

fn not_found() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, t!("not in the archive"))
}

/// The cache directory for archives.
fn cache_root() -> io::Result<PathBuf> {
    directories::ProjectDirs::from("", "", "vais-commander")
        .map(|d| d.cache_dir().join("archives"))
        .ok_or_else(|| io::Error::other(t!("no cache directory")))
}

/// Remove extracted archives nobody has touched for a week.
pub fn prune_cache() {
    let Ok(root) = cache_root() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    let cutoff = SystemTime::now() - Duration::from_secs(7 * 24 * 3600);
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t < cutoff);
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

// ----- the table of open archives --------------------------------------------

type Table = Mutex<HashMap<Location, (u64, Option<SystemTime>, Arc<ArchiveBackend>)>>;

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The backend for the archive at `outer`, opened (and for a tar, unpacked)
/// on first use. Blocks; callers run on a background thread.
pub fn open(outer: &Location) -> io::Result<Arc<dyn Backend>> {
    let name = outer
        .file_name()
        .ok_or_else(|| io::Error::other(t!("no archive name")))?;
    let kind = kind_of(&name)
        .ok_or_else(|| io::Error::other(t!("{name}: not an archive", name = name)))?;
    let meta = super::backend_for(outer)?.stat(outer)?;
    if let Ok(table) = table().lock()
        && let Some((size, modified, backend)) = table.get(outer)
        && *size == meta.size
        && *modified == meta.modified
    {
        return Ok(backend.clone());
    }
    let backend = Arc::new(ArchiveBackend::open(outer, kind, &meta)?);
    if let Ok(mut table) = table().lock() {
        table.insert(outer.clone(), (meta.size, meta.modified, backend.clone()));
    }
    Ok(backend)
}

/// Inner path of `loc` without the leading slash; `None` when `loc` is not
/// inside `outer`.
fn inner_path<'a>(outer: &Location, loc: &'a Location) -> Option<&'a str> {
    match loc {
        Location::Archive { archive, path } if **archive == *outer => Some(path.trim_matches('/')),
        _ => None,
    }
}

/// Components of an entry name as stored in the archive, or `None` for a
/// name that escapes the tree (`..`, absolute) or is the root itself.
fn normalise(name: &Path) -> Option<Vec<String>> {
    use std::path::Component;
    let mut parts = Vec::new();
    for component in name.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
            Component::ParentDir => return None,
        }
    }
    (!parts.is_empty()).then_some(parts)
}

impl ArchiveBackend {
    fn open(outer: &Location, kind: Kind, meta: &Meta) -> io::Result<Self> {
        // The archive has to be a file on disk: fetch a remote one.
        let file = super::local_copy(outer)?;
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            outer.display().hash(&mut h);
            meta.size.hash(&mut h);
            meta.modified
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .hash(&mut h);
            h.finish()
        };
        let cache_dir = cache_root()?.join(format!("{key:016x}"));
        std::fs::create_dir_all(&cache_dir)?;
        let mut this = ArchiveBackend {
            outer: outer.clone(),
            cache_dir,
            nodes: HashMap::from([(String::new(), Node::dir())]),
            zip: None,
        };
        match kind {
            Kind::Zip => this.index_zip(&file)?,
            Kind::Tar(compression) => this.unpack_tar(&file, compression)?,
            Kind::SevenZ => this.unpack_7z(&file)?,
        }
        this.sort_children();
        Ok(this)
    }

    /// Every directory lists each child once, in name order.
    fn sort_children(&mut self) {
        for node in self.nodes.values_mut() {
            node.children.sort();
            node.children.dedup();
        }
    }

    /// Insert an entry (and the directories above it) into the tree.
    fn insert(&mut self, parts: &[String], node: Node) {
        let mut path = String::new();
        for (i, part) in parts.iter().enumerate() {
            let parent = path.clone();
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(part);
            let last = i + 1 == parts.len();
            self.nodes
                .entry(parent)
                .or_insert_with(Node::dir)
                .children
                .push(part.clone());
            if last {
                // A directory entry may come after files inside it: keep the
                // children already collected.
                let children = self
                    .nodes
                    .get(&path)
                    .map(|n| n.children.clone())
                    .unwrap_or_default();
                let mut node = node.clone();
                node.children = children;
                self.nodes.insert(path.clone(), node);
            } else {
                self.nodes.entry(path.clone()).or_insert_with(Node::dir);
            }
        }
    }

    fn index_zip(&mut self, file: &Path) -> io::Result<()> {
        let mut zip = ZipArchive::new(File::open(file)?).map_err(io::Error::other)?;
        for index in 0..zip.len() {
            let entry = zip.by_index_raw(index).map_err(io::Error::other)?;
            let Some(parts) = entry.enclosed_name().and_then(|n| normalise(&n)) else {
                continue;
            };
            let kind = if entry.is_dir() {
                EntryKind::Dir
            } else if entry.is_symlink() {
                EntryKind::Symlink { to_dir: false }
            } else {
                EntryKind::File
            };
            // Zip times are local time without a zone.
            let modified = entry
                .last_modified()
                .and_then(|dt| chrono::NaiveDateTime::try_from(dt).ok())
                .and_then(|naive| {
                    use chrono::TimeZone;
                    chrono::Local.from_local_datetime(&naive).earliest()
                })
                .map(|local| UNIX_EPOCH + Duration::from_secs(local.timestamp().max(0) as u64));
            let mode = entry.unix_mode().or(Some(match kind {
                EntryKind::Dir => 0o040755,
                _ => 0o100644,
            }));
            let node = Node {
                kind,
                size: if kind == EntryKind::Dir {
                    0
                } else {
                    entry.size()
                },
                modified,
                mode,
                index: Some(index),
                children: Vec::new(),
            };
            drop(entry);
            self.insert(&parts, node);
        }
        self.zip = Some(Mutex::new(zip));
        Ok(())
    }

    fn unpack_tar(&mut self, file: &Path, compression: Compression) -> io::Result<()> {
        let raw = File::open(file)?;
        let reader: Box<dyn Read> = match compression {
            Compression::None => Box::new(raw),
            Compression::Gzip => Box::new(flate2::read::MultiGzDecoder::new(raw)),
            Compression::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(raw)),
            Compression::Xz => Box::new(liblzma::read::XzDecoder::new_multi_decoder(raw)),
            Compression::Zstd => Box::new(zstd::stream::read::Decoder::new(raw)?),
        };
        let mut archive = tar::Archive::new(reader);
        archive.set_preserve_permissions(false);
        archive.set_overwrite(true);
        for entry in archive.entries()? {
            let mut entry = entry?;
            let Some(parts) = entry.path().ok().and_then(|p| normalise(&p)) else {
                continue;
            };
            let header = entry.header();
            let entry_type = header.entry_type();
            let kind = if entry_type.is_dir() {
                EntryKind::Dir
            } else if entry_type.is_symlink() || entry_type.is_hard_link() {
                EntryKind::Symlink { to_dir: false }
            } else if entry_type.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            };
            if kind == EntryKind::Other {
                continue;
            }
            let node = Node {
                kind,
                size: if kind == EntryKind::Dir {
                    0
                } else {
                    header.size().unwrap_or(0)
                },
                modified: header
                    .mtime()
                    .ok()
                    .map(|secs| UNIX_EPOCH + Duration::from_secs(secs)),
                mode: header.mode().ok().map(|m| {
                    m & 0o7777
                        | match kind {
                            EntryKind::Dir => 0o040000,
                            EntryKind::Symlink { .. } => 0o120000,
                            _ => 0o100000,
                        }
                }),
                index: None,
                children: Vec::new(),
            };
            if let Err(err) = entry.unpack_in(&self.cache_dir) {
                log::warn!("{}: {}: {err}", self.outer.display(), parts.join("/"));
                continue;
            }
            self.insert(&parts, node);
        }
        Ok(())
    }

    fn unpack_7z(&mut self, file: &Path) -> io::Result<()> {
        use sevenz_rust2::{ArchiveReader, Error as SevenZError, Password};
        let describe = |err: SevenZError| match err {
            SevenZError::PasswordRequired | SevenZError::MaybeBadPassword(_) => {
                io::Error::other(t!("encrypted 7z archives are not supported"))
            }
            // Built without AES: an encrypted archive shows up as a method.
            SevenZError::UnsupportedCompressionMethod(method) if method.starts_with("AES") => {
                io::Error::other(t!("encrypted 7z archives are not supported"))
            }
            SevenZError::UnsupportedCompressionMethod(method) => {
                io::Error::other(t!("unsupported 7z compression: {method}", method = method))
            }
            other => io::Error::other(other.to_string()),
        };
        let mut reader = ArchiveReader::open(file, Password::empty()).map_err(describe)?;
        let mut found: Vec<(Vec<String>, Node)> = Vec::new();
        let cache_dir = self.cache_dir.clone();
        let outer = self.outer.display();
        reader
            .for_each_entries(|entry, data| {
                // Names use `/`; archives made on Windows may carry `\`.
                let name = entry.name.replace('\\', "/");
                let parts = match normalise(Path::new(&name)) {
                    Some(parts) if !entry.is_anti_item => parts,
                    _ => {
                        io::copy(data, &mut io::sink())?;
                        return Ok(true);
                    }
                };
                // The high 16 bits carry a Unix mode when bit 0x8000 says so.
                let unix_mode = (entry.has_windows_attributes
                    && entry.windows_attributes & 0x8000 != 0)
                    .then_some(entry.windows_attributes >> 16);
                let is_symlink = unix_mode.is_some_and(|m| m & 0o170000 == 0o120000);
                let kind = if entry.is_directory {
                    EntryKind::Dir
                } else if is_symlink {
                    EntryKind::Symlink { to_dir: false }
                } else {
                    EntryKind::File
                };
                let target = cache_dir.join(parts.join("/"));
                let written = match kind {
                    EntryKind::Dir => {
                        io::copy(data, &mut io::sink())?;
                        std::fs::create_dir_all(&target)
                    }
                    _ => write_7z_entry(&target, data, is_symlink),
                };
                if let Err(err) = written {
                    log::warn!("{outer}: {}: {err}", parts.join("/"));
                    return Ok(true);
                }
                let node = Node {
                    kind,
                    size: if kind == EntryKind::Dir {
                        0
                    } else {
                        entry.size
                    },
                    modified: entry
                        .has_last_modified_date
                        .then(|| SystemTime::from(entry.last_modified_date)),
                    mode: Some(unix_mode.map(|m| m & 0o177777).unwrap_or(match kind {
                        EntryKind::Dir => 0o040755,
                        _ => 0o100644,
                    })),
                    index: None,
                    children: Vec::new(),
                };
                found.push((parts, node));
                Ok(true)
            })
            .map_err(describe)?;
        for (parts, node) in found {
            self.insert(&parts, node);
        }
        Ok(())
    }

    fn node<'a>(&'a self, loc: &'a Location) -> io::Result<(&'a str, &'a Node)> {
        let inner = inner_path(&self.outer, loc).ok_or_else(not_found)?;
        self.nodes
            .get(inner)
            .map(|n| (inner, n))
            .ok_or_else(not_found)
    }

    /// The extracted copy of a zip entry, written on first use.
    fn extracted(&self, inner: &str, node: &Node) -> io::Result<PathBuf> {
        let target = self.cache_dir.join(inner);
        if std::fs::metadata(&target).is_ok_and(|m| m.is_file() && m.len() == node.size) {
            return Ok(target);
        }
        let (Some(zip), Some(index)) = (&self.zip, node.index) else {
            return Err(not_found());
        };
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut zip = zip.lock().map_err(|_| io::Error::other("zip lock"))?;
        let mut entry = zip.by_index(index).map_err(io::Error::other)?;
        let partial = target.with_extension("vais-part");
        let mut out = File::create(&partial)?;
        let result = io::copy(&mut entry, &mut out).and_then(|_| out.flush());
        drop(out);
        if let Err(err) = result {
            let _ = std::fs::remove_file(&partial);
            return Err(err);
        }
        std::fs::rename(&partial, &target)?;
        Ok(target)
    }
}

/// One 7z entry into the cache: the data as a file, or (a Unix symlink) the
/// data as the link target.
fn write_7z_entry(target: &Path, data: &mut dyn Read, symlink: bool) -> io::Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    if symlink {
        let mut link = String::new();
        data.read_to_string(&mut link)?;
        let _ = std::fs::remove_file(target);
        return std::os::unix::fs::symlink(link, target);
    }
    let _ = symlink;
    let mut out = File::create(target)?;
    io::copy(data, &mut out)?;
    out.flush()
}

impl Backend for ArchiveBackend {
    fn read_dir(&self, dir: &Location, show_hidden: bool) -> io::Result<Vec<Entry>> {
        let (_, node) = self.node(dir)?;
        if node.kind != EntryKind::Dir {
            return Err(io::Error::other(t!("not a directory")));
        }
        let inner = inner_path(&self.outer, dir).unwrap_or_default();
        let mut entries = Vec::with_capacity(node.children.len());
        for name in &node.children {
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            let key = if inner.is_empty() {
                name.clone()
            } else {
                format!("{inner}/{name}")
            };
            let Some(child) = self.nodes.get(&key) else {
                continue;
            };
            entries.push(Entry {
                name: name.clone(),
                location: dir.join(name),
                kind: child.kind,
                size: child.size,
                modified: child.modified,
                created: None,
                mode: child.mode,
                uid: None,
                is_parent: false,
                depth: 0,
            });
        }
        Ok(entries)
    }

    fn stat(&self, loc: &Location) -> io::Result<Meta> {
        let (_, node) = self.node(loc)?;
        Ok(Meta {
            kind: node.kind,
            size: node.size,
            modified: node.modified,
        })
    }

    fn mkdir_all(&self, _dir: &Location) -> io::Result<()> {
        Err(read_only())
    }

    fn rename(&self, _from: &Location, _to: &Location) -> io::Result<()> {
        Err(read_only())
    }

    fn remove(&self, _loc: &Location) -> io::Result<()> {
        Err(read_only())
    }

    fn open_read(&self, loc: &Location) -> io::Result<Box<dyn Read + Send>> {
        let (inner, node) = self.node(loc)?;
        if node.kind == EntryKind::Dir {
            return Err(io::Error::other(t!("is a directory")));
        }
        let path = if self.zip.is_some() {
            self.extracted(inner, node)?
        } else {
            self.cache_dir.join(inner)
        };
        Ok(Box::new(File::open(path)?))
    }

    fn open_write(&self, _loc: &Location) -> io::Result<Box<dyn Write + Send>> {
        Err(read_only())
    }
}

#[cfg(test)]
mod tests {
    use super::{ArchiveBackend, Compression, Kind, Node, kind_of, stem};
    use crate::fs::EntryKind;
    use crate::vfs::Location;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    /// A backend over a fixture with its cache in a fresh temp directory.
    fn sevenz_backend(fixture: &str) -> (ArchiveBackend, PathBuf, std::io::Result<()>) {
        let cache =
            std::env::temp_dir().join(format!("vais-7z-test-{}-{fixture}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache);
        std::fs::create_dir_all(&cache).unwrap();
        let file = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/vfs/testdata")
            .join(fixture);
        let mut backend = ArchiveBackend {
            outer: Location::Local(file.clone()),
            cache_dir: cache.clone(),
            nodes: HashMap::from([(String::new(), Node::dir())]),
            zip: None,
        };
        let result = backend.unpack_7z(&file);
        backend.sort_children();
        (backend, cache, result)
    }

    #[test]
    fn sevenz_is_unpacked_with_its_tree() {
        let (backend, cache, result) = sevenz_backend("sample.7z");
        result.unwrap();
        let root = &backend.nodes[""];
        assert_eq!(root.children, ["photos"]);
        let readme = &backend.nodes["photos/readme.txt"];
        assert_eq!(readme.kind, EntryKind::File);
        assert_eq!(readme.size, 9);
        assert!(readme.modified.is_some());
        assert_eq!(readme.mode, Some(0o100644));
        assert_eq!(
            std::fs::read_to_string(cache.join("photos/readme.txt")).unwrap(),
            "hello 7z\n"
        );
        // Empty files have no stream of their own; names keep their accents.
        assert_eq!(
            std::fs::metadata(cache.join("photos/empty.txt"))
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            std::fs::read_to_string(cache.join("photos/sub/příliš.txt")).unwrap(),
            "žluťoučký kůň\n"
        );
        assert_eq!(backend.nodes["photos/sub"].kind, EntryKind::Dir);
        #[cfg(unix)]
        {
            assert_eq!(
                backend.nodes["photos/link.txt"].kind,
                EntryKind::Symlink { to_dir: false }
            );
            assert_eq!(
                std::fs::read_link(cache.join("photos/link.txt")).unwrap(),
                Path::new("readme.txt")
            );
        }
        std::fs::remove_dir_all(cache).unwrap();
    }

    #[test]
    fn encrypted_sevenz_is_reported() {
        let (_, cache, result) = sevenz_backend("locked.7z");
        let err = result.unwrap_err();
        assert!(err.to_string().contains("encrypted"), "{err}");
        std::fs::remove_dir_all(cache).unwrap();
    }

    #[test]
    fn stems_drop_the_whole_format_ending() {
        assert_eq!(stem("site.tar.gz"), Some("site"));
        assert_eq!(stem("Backup.2026.TGZ"), Some("Backup.2026"));
        assert_eq!(stem("app.zip"), Some("app"));
        assert_eq!(stem("x.tar.zst"), Some("x"));
        assert_eq!(stem("plain.tar"), Some("plain"));
        assert_eq!(stem("notes.txt"), None);
        assert_eq!(stem(".zip"), None);
        assert_eq!(kind_of("a.TAR.XZ"), Some(Kind::Tar(Compression::Xz)));
        assert_eq!(kind_of("Photos.7Z"), Some(Kind::SevenZ));
        assert_eq!(stem("photos.7z"), Some("photos"));
    }

    #[test]
    fn recognises_archive_names() {
        assert_eq!(kind_of("a.zip"), Some(Kind::Zip));
        assert_eq!(kind_of("lib.JAR"), Some(Kind::Zip));
        assert_eq!(kind_of("a.tar"), Some(Kind::Tar(Compression::None)));
        assert_eq!(kind_of("a.tar.gz"), Some(Kind::Tar(Compression::Gzip)));
        assert_eq!(kind_of("a.tgz"), Some(Kind::Tar(Compression::Gzip)));
        assert_eq!(kind_of("a.tar.xz"), Some(Kind::Tar(Compression::Xz)));
        assert_eq!(kind_of("a.tar.zst"), Some(Kind::Tar(Compression::Zstd)));
        assert_eq!(kind_of("a.tar.bz2"), Some(Kind::Tar(Compression::Bzip2)));
        assert_eq!(kind_of("a.gz"), None);
        assert_eq!(kind_of("notes.txt"), None);
    }
}
