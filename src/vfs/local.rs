//! The local disk as a [`Backend`]: a thin layer over `std::fs`.

use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::{Arc, OnceLock};

use super::{Backend, Location, Meta};
use crate::fs::{Entry, EntryKind};

pub struct LocalBackend;

/// The one shared instance.
pub fn shared() -> Arc<dyn Backend> {
    static LOCAL: OnceLock<Arc<dyn Backend>> = OnceLock::new();
    LOCAL.get_or_init(|| Arc::new(LocalBackend)).clone()
}

fn path_of(loc: &Location) -> io::Result<&Path> {
    loc.as_local()
        .ok_or_else(|| io::Error::other("not a local path"))
}

/// Permission bits and owner of a Unix file; nothing elsewhere.
#[cfg(unix)]
fn unix_ids(meta: &fs::Metadata) -> (Option<u32>, Option<u32>) {
    use std::os::unix::fs::MetadataExt;
    (Some(meta.mode()), Some(meta.uid()))
}

#[cfg(not(unix))]
fn unix_ids(_meta: &fs::Metadata) -> (Option<u32>, Option<u32>) {
    (None, None)
}

fn kind_of(meta: &fs::Metadata, path: &Path) -> (EntryKind, u64) {
    let file_type = meta.file_type();
    if file_type.is_symlink() {
        let to_dir = fs::metadata(path).map(|t| t.is_dir()).unwrap_or(false);
        (EntryKind::Symlink { to_dir }, meta.len())
    } else if file_type.is_dir() {
        (EntryKind::Dir, 0)
    } else if file_type.is_file() {
        (EntryKind::File, meta.len())
    } else {
        (EntryKind::Other, meta.len())
    }
}

impl Backend for LocalBackend {
    fn read_dir(&self, dir: &Location, show_hidden: bool) -> io::Result<Vec<Entry>> {
        let path = path_of(dir)?;
        let mut entries = Vec::new();
        for item in fs::read_dir(path)? {
            let Ok(item) = item else { continue };
            let name = item.file_name().to_string_lossy().into_owned();
            let entry_path = item.path();
            // `DirEntry::metadata` does not follow symlinks.
            let meta = item.metadata();
            if !show_hidden && crate::platform::is_hidden(&name, meta.as_ref().ok()) {
                continue;
            }
            let (kind, size, modified, created, mode, uid) = match meta {
                Ok(meta) => {
                    let (kind, size) = kind_of(&meta, &entry_path);
                    let (mode, uid) = unix_ids(&meta);
                    (
                        kind,
                        size,
                        meta.modified().ok(),
                        meta.created().ok(),
                        mode,
                        uid,
                    )
                }
                Err(_) => (EntryKind::Other, 0, None, None, None, None),
            };
            entries.push(Entry {
                name,
                location: Location::Local(entry_path),
                kind,
                size,
                modified,
                created,
                mode,
                uid,
                is_parent: false,
                depth: 0,
            });
        }
        Ok(entries)
    }

    fn stat(&self, loc: &Location) -> io::Result<Meta> {
        let path = path_of(loc)?;
        let meta = fs::symlink_metadata(path)?;
        let (kind, size) = kind_of(&meta, path);
        Ok(Meta {
            kind,
            size,
            modified: meta.modified().ok(),
        })
    }

    fn mkdir_all(&self, dir: &Location) -> io::Result<()> {
        fs::create_dir_all(path_of(dir)?)
    }

    fn rename(&self, from: &Location, to: &Location) -> io::Result<()> {
        fs::rename(path_of(from)?, path_of(to)?)
    }

    fn remove(&self, loc: &Location) -> io::Result<()> {
        let path = path_of(loc)?;
        let meta = fs::symlink_metadata(path)?;
        if meta.is_dir() && !meta.file_type().is_symlink() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        }
    }

    fn open_read(&self, loc: &Location) -> io::Result<Box<dyn Read + Send>> {
        Ok(Box::new(fs::File::open(path_of(loc)?)?))
    }

    fn open_write(&self, loc: &Location) -> io::Result<Box<dyn Write + Send>> {
        Ok(Box::new(fs::File::create(path_of(loc)?)?))
    }
}
