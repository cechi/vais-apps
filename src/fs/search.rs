//! Recursive name search below a directory, for the panel's "Subfolders" filter.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use super::filter::Matcher;
use super::{Entry, ReadOptions, read_dir_sorted};

/// Walk `root` depth first (sorted like the panel) and collect entries whose
/// name matches. Names become paths relative to `root`; directory symlinks are
/// not followed. Stops at `limit` results or when `cancel` is set; the second
/// value says whether the walk was cut short.
pub fn search_tree(
    root: &Path,
    opts: ReadOptions,
    matcher: &Matcher,
    limit: usize,
    cancel: &AtomicBool,
) -> (Vec<Entry>, bool) {
    let mut results = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return (results, true);
        }
        let Ok(entries) = read_dir_sorted(&dir, opts) else {
            continue;
        };
        let mut subdirs = Vec::new();
        for mut entry in entries.into_iter().filter(|e| !e.is_parent) {
            if entry.is_dir() && !entry.is_symlink() {
                subdirs.push(entry.path.clone());
            }
            if matcher.matches(&entry.name) {
                entry.name = entry
                    .path
                    .strip_prefix(root)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| entry.name.clone());
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
        let (found, truncated) = search_tree(
            &root,
            ReadOptions::default(),
            &Matcher::new("*.png"),
            100,
            &cancel,
        );
        // Relative names use the platform separator.
        let rel = |parts: &[&str]| {
            parts
                .iter()
                .collect::<PathBuf>()
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
        assert!(found.iter().all(|e| e.depth == 0 && e.path.is_absolute()));

        let (found, truncated) = search_tree(
            &root,
            ReadOptions::default(),
            &Matcher::new("png"),
            2,
            &cancel,
        );
        assert_eq!(found.len(), 2);
        assert!(truncated);
        let _ = fs::remove_dir_all(&root);
    }
}
