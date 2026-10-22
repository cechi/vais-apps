//! Name and content search below a directory, for the panel's "Subfolders"
//! and "Contents" filter toggles.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};

use super::filter::Matcher;
use super::{Entry, ReadOptions, read_dir_sorted};
use crate::vfs::Location;

/// How much of a file the content search reads.
const CONTENT_LIMIT: u64 = 8 * 1024 * 1024;

/// What to look for: a name pattern, or text inside files.
#[derive(Debug, Clone)]
pub enum Needle {
    Name(Matcher),
    /// Lowercased text; files are matched case-insensitively.
    Content(String),
}

impl Needle {
    pub fn content(text: &str) -> Self {
        Needle::Content(text.trim().to_lowercase())
    }

    fn matches(&self, entry: &Entry) -> bool {
        match self {
            Needle::Name(matcher) => matcher.matches(&entry.name),
            Needle::Content(text) => {
                !entry.is_dir() && !text.is_empty() && file_contains(&entry.location, text)
            }
        }
    }
}

/// Whether the (text) file at `loc` contains `needle`; binary files and the
/// unreadable are skipped.
fn file_contains(loc: &Location, needle: &str) -> bool {
    let Ok(backend) = crate::vfs::backend_for(loc) else {
        return false;
    };
    let Ok(reader) = backend.open_read(loc) else {
        return false;
    };
    let mut bytes = Vec::new();
    if reader.take(CONTENT_LIMIT).read_to_end(&mut bytes).is_err() {
        return false;
    }
    if bytes.iter().take(8192).any(|&b| b == 0) {
        return false;
    }
    String::from_utf8_lossy(&bytes).to_lowercase().contains(needle)
}

/// Walk `root` depth first (sorted like the panel; only `root` itself when
/// `recursive` is off) and collect the entries `needle` matches. Names
/// become paths relative to `root`; directory symlinks are not followed.
/// Stops at `limit` results or when `cancel` is set; the second value says
/// whether the walk was cut short.
pub fn search_tree(
    root: &Location,
    opts: ReadOptions,
    needle: &Needle,
    recursive: bool,
    limit: usize,
    cancel: &AtomicBool,
) -> (Vec<Entry>, bool) {
    let mut results = Vec::new();
    let mut stack: Vec<Location> = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return (results, true);
        }
        let Ok(entries) = read_dir_sorted(&dir, opts) else {
            continue;
        };
        let mut subdirs = Vec::new();
        for mut entry in entries.into_iter().filter(|e| !e.is_parent) {
            if recursive && entry.is_dir() && !entry.is_symlink() {
                subdirs.push(entry.location.clone());
            }
            if needle.matches(&entry) {
                entry.name = entry
                    .location
                    .relative_to(root)
                    .unwrap_or_else(|| entry.name.clone());
                entry.depth = 0;
                results.push(entry);
                if results.len() >= limit {
                    return (results, true);
                }
            }
        }
        // Push in reverse so the sorted order is kept when popping.
        stack.extend(subdirs.into_iter().rev());
    }
    (results, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_nested_matches_with_relative_names() {
        let root = std::env::temp_dir().join(format!("vais-search-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("a/inner")).unwrap();
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join("a/x.png"), "").unwrap();
        fs::write(root.join("a/inner/y.png"), "").unwrap();
        fs::write(root.join("a/inner/z.txt"), "").unwrap();
        fs::write(root.join(".hidden/h.png"), "").unwrap();
        fs::write(root.join("top.png"), "").unwrap();

        let cancel = AtomicBool::new(false);
        let root_loc = Location::local(&root);
        let (found, truncated) = search_tree(
            &root_loc,
            ReadOptions::default(),
            &Needle::Name(Matcher::new("*.png")),
            true,
            100,
            &cancel,
        );
        // Relative names use the platform separator.
        let rel = |parts: &[&str]| {
            parts
                .iter()
                .collect::<std::path::PathBuf>()
                .to_string_lossy()
                .into_owned()
        };
        let names: Vec<_> = found.iter().map(|e| e.name.clone()).collect();
        assert_eq!(
            names,
            [
                rel(&["top.png"]),
                rel(&["a", "x.png"]),
                rel(&["a", "inner", "y.png"]),
            ]
        );
        assert!(!truncated);
        assert!(
            found
                .iter()
                .all(|e| e.depth == 0 && e.location.as_local().is_some_and(|p| p.is_absolute()))
        );

        let (found, truncated) = search_tree(
            &root_loc,
            ReadOptions::default(),
            &Needle::Name(Matcher::new("png")),
            true,
            2,
            &cancel,
        );
        assert_eq!(found.len(), 2);
        assert!(truncated);

        // Contents: case-insensitive text, binary files skipped, only the
        // top level without recursion.
        fs::write(root.join("a/notes.txt"), "Hello Needle").unwrap();
        fs::write(root.join("a/inner/deep.txt"), "needle again").unwrap();
        fs::write(root.join("a/blob.bin"), b"needle\0binary").unwrap();
        fs::write(root.join("top.txt"), "no match").unwrap();
        let (found, _) = search_tree(
            &root_loc,
            ReadOptions::default(),
            &Needle::content("NEEDLE"),
            true,
            100,
            &cancel,
        );
        let names: Vec<_> = found.iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, [rel(&["a", "notes.txt"]), rel(&["a", "inner", "deep.txt"])]);
        let (found, _) = search_tree(
            &Location::local(root.join("a")),
            ReadOptions::default(),
            &Needle::content("needle"),
            false,
            100,
            &cancel,
        );
        assert_eq!(found.len(), 1);
        let _ = fs::remove_dir_all(&root);
    }
}
