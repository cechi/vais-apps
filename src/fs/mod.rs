//! Directory reading: synchronous, `Send`, meant to run on the background executor.

pub mod format;
pub mod sort;

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

pub use sort::{Sort, SortDir, SortKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File,
    Symlink { to_dir: bool },
    Other,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub kind: EntryKind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// The synthetic ".." entry.
    pub is_parent: bool,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        matches!(
            self.kind,
            EntryKind::Dir | EntryKind::Symlink { to_dir: true }
        )
    }

    pub fn is_symlink(&self) -> bool {
        matches!(self.kind, EntryKind::Symlink { .. })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ReadOptions {
    pub show_hidden: bool,
    pub sort: Sort,
}

/// Read `path`, sort entries (directories first) and prepend ".." unless at root.
pub fn read_dir_sorted(path: &Path, opts: ReadOptions) -> io::Result<Vec<Entry>> {
    let read_dir = fs::read_dir(path)?;
    let mut entries = Vec::new();

    for item in read_dir {
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        let entry_path = item.path();
        // `DirEntry::metadata` does not follow symlinks.
        let meta = item.metadata();
        if !opts.show_hidden && crate::platform::is_hidden(&name, meta.as_ref().ok()) {
            continue;
        }
        let (kind, size, modified) = match meta {
            Ok(meta) => {
                let file_type = meta.file_type();
                let modified = meta.modified().ok();
                if file_type.is_symlink() {
                    let to_dir = fs::metadata(&entry_path)
                        .map(|t| t.is_dir())
                        .unwrap_or(false);
                    (EntryKind::Symlink { to_dir }, meta.len(), modified)
                } else if file_type.is_dir() {
                    (EntryKind::Dir, 0, modified)
                } else if file_type.is_file() {
                    (EntryKind::File, meta.len(), modified)
                } else {
                    (EntryKind::Other, meta.len(), modified)
                }
            }
            Err(_) => (EntryKind::Other, 0, None),
        };
        entries.push(Entry {
            name,
            path: entry_path,
            kind,
            size,
            modified,
            is_parent: false,
        });
    }

    sort::sort_entries(&mut entries, opts.sort);

    if let Some(parent) = crate::platform::parent_of(path) {
        entries.insert(0, parent_entry(parent));
    }

    Ok(entries)
}

/// The synthetic listing of roots (drive letters on Windows).
pub fn read_roots() -> Vec<Entry> {
    crate::platform::roots()
        .into_iter()
        .map(|root| Entry {
            name: root.to_string_lossy().into_owned(),
            path: root,
            kind: EntryKind::Dir,
            size: 0,
            modified: None,
            is_parent: false,
        })
        .collect()
}

fn parent_entry(parent: PathBuf) -> Entry {
    Entry {
        name: "..".to_string(),
        path: parent,
        kind: EntryKind::Dir,
        size: 0,
        modified: None,
        is_parent: true,
    }
}
