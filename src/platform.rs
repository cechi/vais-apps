//! The few places where the file system differs between operating systems.
//!
//! Linux is the only tested platform so far; the macOS and Windows branches
//! follow the platform conventions but are untested.

use std::path::{Path, PathBuf};

/// Whether an entry is hidden by the platform's convention.
#[cfg(not(windows))]
pub fn is_hidden(name: &str, _meta: Option<&std::fs::Metadata>) -> bool {
    name.starts_with('.')
}

#[cfg(windows)]
pub fn is_hidden(name: &str, meta: Option<&std::fs::Metadata>) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    name.starts_with('.') || meta.is_some_and(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
}

/// Top-level roots: `/` on Unix, the mounted drive letters on Windows.
#[cfg(not(windows))]
pub fn roots() -> Vec<PathBuf> {
    vec![PathBuf::from("/")]
}

#[cfg(windows)]
pub fn roots() -> Vec<PathBuf> {
    let drives: Vec<PathBuf> = (b'A'..=b'Z')
        .map(|letter| PathBuf::from(format!("{}:\\", letter as char)))
        .filter(|p| p.exists())
        .collect();
    if drives.is_empty() {
        vec![PathBuf::from("C:\\")]
    } else {
        drives
    }
}

/// Where to go when nothing better is known (empty path input, no home dir).
pub fn default_root() -> PathBuf {
    roots()
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// The synthetic "list of roots" location, used above drive roots on Windows.
/// An empty path is the sentinel; it never names a real directory.
pub fn roots_path() -> PathBuf {
    PathBuf::new()
}

pub fn is_roots_path(path: &Path) -> bool {
    path.as_os_str().is_empty()
}

/// Whether a `..` entry above `path` makes sense: a parent directory, or the
/// roots listing when there is more than one root.
pub fn parent_of(path: &Path) -> Option<PathBuf> {
    if is_roots_path(path) {
        return None;
    }
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Some(parent.to_path_buf()),
        _ if roots().len() > 1 => Some(roots_path()),
        _ => None,
    }
}

/// Human readable label for a location, for the path bar and tab titles.
pub fn display_name(path: &Path) -> String {
    if is_roots_path(path) {
        return "Computer".to_string();
    }
    match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.to_string_lossy().into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_are_never_empty() {
        assert!(!roots().is_empty());
        assert!(roots().iter().all(|r| r.is_absolute()));
    }

    #[test]
    fn parent_chain_ends() {
        let mut p = default_root().join("a").join("b");
        let mut steps = 0;
        while let Some(parent) = parent_of(&p) {
            p = parent;
            steps += 1;
            assert!(steps < 10, "parent chain does not terminate");
        }
        assert!(steps >= 2);
    }

    #[test]
    fn roots_sentinel() {
        assert!(is_roots_path(&roots_path()));
        assert_eq!(display_name(&roots_path()), "Computer");
        assert_eq!(display_name(Path::new("/usr/lib")), "lib");
    }
}
