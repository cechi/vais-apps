//! Pure helpers of the portal backend: `file://` URIs, names that do not
//! collide, button labels and the caller's file filters.

use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::fs::filter::Matcher;

/// `file://` URI of a local path: every byte outside the unreserved set and
/// `/` is percent-encoded, so spaces, `#`, `?` and non-ASCII names survive.
pub fn file_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// `name`, or `name (1)`, `name (2)`… (before the extension) when it exists in
/// `dir` already.
pub fn unique_name(dir: &Path, name: &str) -> String {
    if !dir.join(name).exists() {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(ix) if ix > 0 => (&name[..ix], &name[ix..]),
        _ => (name, ""),
    };
    (1u32..)
        .map(|n| format!("{stem} ({n}){ext}"))
        .find(|candidate| !dir.join(candidate).exists())
        .expect("an unused name exists")
}

/// A button label without its GTK mnemonic: `_Open` → `Open`, `__` → `_`.
pub fn strip_mnemonic(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                chars.next();
                out.push('_');
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// One file filter the caller offered: a label, glob patterns and MIME types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterSet {
    pub label: String,
    pub globs: Vec<String>,
    pub mimes: Vec<String>,
}

impl FilterSet {
    /// The matchers of the filter; `resolve` turns a MIME type into globs. A
    /// MIME type nothing is known about matches everything rather than
    /// hiding the files the caller asked for.
    pub fn matchers(&self, resolve: impl Fn(&str) -> Vec<String>) -> Vec<Matcher> {
        let mut matchers: Vec<Matcher> = self.globs.iter().map(|g| glob_matcher(g)).collect();
        for mime in &self.mimes {
            let globs = resolve(mime);
            if globs.is_empty() {
                return vec![Matcher::All];
            }
            matchers.extend(globs.iter().map(|g| glob_matcher(g)));
        }
        if matchers.is_empty() {
            matchers.push(Matcher::All);
        }
        matchers
    }
}

/// A portal glob as a matcher: always a wildcard match of the whole name (a
/// bare `Makefile` is exact, not a substring), case-insensitive like the
/// panel's own filter.
fn glob_matcher(pattern: &str) -> Matcher {
    Matcher::Glob(pattern.to_lowercase().chars().collect())
}

/// Whether `name` passes any of the matchers.
#[cfg(test)]
pub fn matches_any(matchers: &[Matcher], name: &str) -> bool {
    matchers.iter().any(|m| m.matches(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn uris_are_percent_encoded() {
        assert_eq!(file_uri(Path::new("/tmp/a.txt")), "file:///tmp/a.txt");
        assert_eq!(
            file_uri(Path::new("/tmp/my file #1?.txt")),
            "file:///tmp/my%20file%20%231%3F.txt"
        );
        assert_eq!(
            file_uri(Path::new("/tmp/žluť")),
            "file:///tmp/%C5%BElu%C5%A5"
        );
        assert_eq!(file_uri(Path::new("/a%b&c")), "file:///a%25b%26c");
    }

    #[test]
    fn unique_names_count_before_the_extension() {
        let dir = std::env::temp_dir().join(format!("vais-uri-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_name(&dir, "new.txt"), "new.txt");
        std::fs::write(dir.join("new.txt"), b"").unwrap();
        assert_eq!(unique_name(&dir, "new.txt"), "new (1).txt");
        std::fs::write(dir.join("new (1).txt"), b"").unwrap();
        assert_eq!(unique_name(&dir, "new.txt"), "new (2).txt");
        std::fs::write(dir.join(".hidden"), b"").unwrap();
        assert_eq!(unique_name(&dir, ".hidden"), ".hidden (1)");
        std::fs::remove_dir_all(&dir).unwrap();
        let _: PathBuf = dir;
    }

    #[test]
    fn mnemonics_are_stripped() {
        assert_eq!(strip_mnemonic("_Open"), "Open");
        assert_eq!(strip_mnemonic("Save _As"), "Save As");
        assert_eq!(strip_mnemonic("a__b"), "a_b");
        assert_eq!(strip_mnemonic("Plain"), "Plain");
    }

    #[test]
    fn filters_match_globs_and_resolved_mimes() {
        let images = FilterSet {
            label: "Images".into(),
            globs: vec!["*.png".into()],
            mimes: vec!["image/jpeg".into()],
        };
        let resolve = |mime: &str| match mime {
            "image/jpeg" => vec!["*.jpg".to_string(), "*.jpeg".to_string()],
            _ => Vec::new(),
        };
        let m = images.matchers(resolve);
        assert!(matches_any(&m, "shot.PNG"));
        assert!(matches_any(&m, "photo.jpeg"));
        assert!(!matches_any(&m, "notes.txt"));
        assert!(!matches_any(&m, "png"));

        let exact = FilterSet {
            label: "Makefiles".into(),
            globs: vec!["Makefile".into()],
            mimes: Vec::new(),
        };
        let m = exact.matchers(|_| Vec::new());
        assert!(matches_any(&m, "makefile"));
        assert!(!matches_any(&m, "Makefile.bak"));

        // An unknown MIME type does not hide anything.
        let unknown = FilterSet {
            label: "Odd".into(),
            globs: Vec::new(),
            mimes: vec!["application/x-unknown".into()],
        };
        assert!(matches_any(&unknown.matchers(|_| Vec::new()), "anything"));
    }
}
