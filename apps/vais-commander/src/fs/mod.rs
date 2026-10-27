//! Directory reading: synchronous, `Send`, meant to run on the background executor.

pub mod filter;
pub mod format;
pub mod search;
pub mod sort;

use std::{collections::BTreeSet, io, time::SystemTime};

pub use sort::{Sort, SortDir, SortKey, extension};

use crate::vfs::{Location, backend_for};

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
    pub location: Location,
    pub kind: EntryKind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    /// Unix permission bits (`st_mode`), when known.
    pub mode: Option<u32>,
    /// Unix owner id, when known.
    pub uid: Option<u32>,
    /// The synthetic ".." entry.
    pub is_parent: bool,
    /// Nesting level when shown inside an expanded directory (0 = top level).
    pub depth: usize,
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

/// Read `dir` through its backend, sort entries (directories first) and
/// prepend ".." unless at a root.
pub fn read_dir_sorted(dir: &Location, opts: ReadOptions) -> io::Result<Vec<Entry>> {
    let backend = backend_for(dir)?;
    let mut entries = backend.read_dir(dir, opts.show_hidden)?;
    sort::sort_entries(&mut entries, opts.sort);
    if let Some(parent) = dir.parent() {
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
            location: Location::Local(root),
            kind: EntryKind::Dir,
            size: 0,
            modified: None,
            created: None,
            mode: None,
            uid: None,
            is_parent: false,
            depth: 0,
        })
        .collect()
}

/// Read `path` and, recursively, every directory listed in `expanded`, as one
/// flattened list where children follow their parent with `depth + 1`.
pub fn read_tree(
    path: &Location,
    opts: ReadOptions,
    expanded: &BTreeSet<Location>,
) -> io::Result<Vec<Entry>> {
    let mut entries = read_dir_sorted(path, opts)?;
    if !expanded.is_empty() {
        expand_into(&mut entries, opts, expanded);
    }
    Ok(entries)
}

fn expand_into(entries: &mut Vec<Entry>, opts: ReadOptions, expanded: &BTreeSet<Location>) {
    let mut i = 0;
    while i < entries.len() {
        let entry = &entries[i];
        if !entry.is_parent && entry.is_dir() && expanded.contains(&entry.location) {
            let depth = entry.depth + 1;
            if let Ok(children) = read_dir_sorted(&entry.location, opts) {
                let mut children: Vec<Entry> = children
                    .into_iter()
                    .filter(|c| !c.is_parent)
                    .map(|mut c| {
                        c.depth = depth;
                        c
                    })
                    .collect();
                expand_into(&mut children, opts, expanded);
                let count = children.len();
                entries.splice(i + 1..i + 1, children);
                i += count;
            }
        }
        i += 1;
    }
}

/// The synthetic `..` row pointing at `parent`.
pub fn parent_entry(parent: Location) -> Entry {
    Entry {
        name: "..".to_string(),
        location: parent,
        kind: EntryKind::Dir,
        size: 0,
        modified: None,
        created: None,
        mode: None,
        uid: None,
        is_parent: true,
        depth: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn read_tree_flattens_expanded_directories() {
        let root = std::env::temp_dir().join(format!("vais-tree-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("a/inner")).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        fs::write(root.join("a/x.txt"), "x").unwrap();
        fs::write(root.join("a/inner/y.txt"), "y").unwrap();
        fs::write(root.join("z.txt"), "z").unwrap();

        let opts = ReadOptions::default();
        let mut expanded = BTreeSet::new();
        expanded.insert(Location::local(root.join("a")));
        expanded.insert(Location::local(root.join("a/inner")));
        let entries = read_tree(&Location::local(&root), opts, &expanded).unwrap();
        let rows: Vec<(String, usize)> =
            entries.iter().map(|e| (e.name.clone(), e.depth)).collect();
        assert_eq!(
            rows,
            vec![
                ("..".into(), 0),
                ("a".into(), 0),
                ("inner".into(), 1),
                ("y.txt".into(), 2),
                ("x.txt".into(), 1),
                ("b".into(), 0),
                ("z.txt".into(), 0),
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }
}
