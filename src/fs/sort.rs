use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use super::Entry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortKey {
    #[default]
    Name,
    Size,
    /// File extension (directories always come first anyway).
    Type,
    Modified,
    Created,
}

impl SortKey {
    pub const ALL: [SortKey; 5] = [
        SortKey::Name,
        SortKey::Size,
        SortKey::Type,
        SortKey::Modified,
        SortKey::Created,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Size => "Size",
            SortKey::Type => "Type",
            SortKey::Modified => "Modified",
            SortKey::Created => "Created",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDir {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Sort {
    pub key: SortKey,
    pub dir: SortDir,
}

impl Sort {
    /// Sort by `key`; choosing the current key again flips the direction.
    pub fn toggled(self, key: SortKey) -> Sort {
        if self.key == key {
            Sort {
                key,
                dir: match self.dir {
                    SortDir::Asc => SortDir::Desc,
                    SortDir::Desc => SortDir::Asc,
                },
            }
        } else {
            Sort {
                key,
                dir: SortDir::Asc,
            }
        }
    }
}

/// The extension of a file name (without the dot). Dot-files and names
/// without a dot have none. Only the final path component is considered, so
/// relative paths from recursive searches work too.
pub fn extension(name: &str) -> &str {
    let file_start = name.rfind(['/', '\\']).map(|i| i + 1).unwrap_or(0);
    let file = &name[file_start..];
    match file.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < file.len() => &file[dot + 1..],
        _ => "",
    }
}

/// Case-insensitive natural ordering: digit runs compare numerically
/// (`file2` < `file10`), everything else by lowercase char.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    // Case and leading zeros only decide between otherwise equal names.
    let mut tiebreak = Ordering::Equal;
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return tiebreak,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(|c| c.is_ascii_digit()) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(|c| c.is_ascii_digit()) {
                    nb.push(c);
                    bi.next();
                }
                let ta = na.trim_start_matches('0');
                let tb = nb.trim_start_matches('0');
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
                if tiebreak == Ordering::Equal {
                    tiebreak = na.len().cmp(&nb.len());
                }
            }
            (Some(ca), Some(cb)) => {
                ai.next();
                bi.next();
                let la = ca.to_lowercase().next().unwrap_or(ca);
                let lb = cb.to_lowercase().next().unwrap_or(cb);
                let ord = la.cmp(&lb);
                if ord != Ordering::Equal {
                    return ord;
                }
                if tiebreak == Ordering::Equal {
                    tiebreak = ca.cmp(&cb);
                }
            }
        }
    }
}

/// Directories first, then by `sort`; name is always the final tiebreaker.
pub fn sort_entries(entries: &mut [Entry], sort: Sort) {
    entries.sort_by(|a, b| {
        b.is_dir()
            .cmp(&a.is_dir())
            .then_with(|| {
                let primary = match sort.key {
                    SortKey::Name => natural_cmp(&a.name, &b.name),
                    SortKey::Size => a.size.cmp(&b.size),
                    SortKey::Type => natural_cmp(extension(&a.name), extension(&b.name)),
                    SortKey::Modified => a.modified.cmp(&b.modified),
                    SortKey::Created => a.created.cmp(&b.created),
                };
                match sort.dir {
                    SortDir::Asc => primary,
                    SortDir::Desc => primary.reverse(),
                }
            })
            .then_with(|| natural_cmp(&a.name, &b.name))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::EntryKind;
    use crate::vfs::Location;

    fn entry(name: &str, kind: EntryKind, size: u64) -> Entry {
        Entry {
            name: name.into(),
            location: Location::local(name),
            kind,
            size,
            modified: None,
            created: None,
            mode: None,
            uid: None,
            is_parent: false,
            depth: 0,
        }
    }

    #[test]
    fn natural_order_handles_numbers() {
        assert_eq!(natural_cmp("file2", "file10"), Ordering::Less);
        assert_eq!(natural_cmp("File10", "file9"), Ordering::Greater);
        assert_eq!(natural_cmp("a", "B"), Ordering::Less);
        assert_eq!(natural_cmp("x", "x"), Ordering::Equal);
        assert_eq!(natural_cmp("img007", "img7"), Ordering::Greater);
    }

    #[test]
    fn dirs_come_first_regardless_of_key() {
        let mut v = vec![
            entry("b.txt", EntryKind::File, 10),
            entry("zdir", EntryKind::Dir, 0),
            entry("a.txt", EntryKind::File, 5),
            entry("Adir", EntryKind::Dir, 0),
        ];
        sort_entries(
            &mut v,
            Sort {
                key: SortKey::Size,
                dir: SortDir::Desc,
            },
        );
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Adir", "zdir", "b.txt", "a.txt"]);
    }

    #[test]
    fn extension_is_the_last_dot_of_the_file_name() {
        assert_eq!(extension("report.pdf"), "pdf");
        assert_eq!(extension("archive.tar.gz"), "gz");
        assert_eq!(extension(".bashrc"), "");
        assert_eq!(extension("Makefile"), "");
        assert_eq!(extension("trailing."), "");
        assert_eq!(extension("sub.dir/notes.md"), "md");
        assert_eq!(extension("v1.0/README"), "");
    }

    #[test]
    fn type_sort_groups_by_extension_then_name() {
        let mut v = vec![
            entry("b.txt", EntryKind::File, 1),
            entry("a.zip", EntryKind::File, 1),
            entry("a.txt", EntryKind::File, 1),
            entry("Makefile", EntryKind::File, 1),
        ];
        sort_entries(
            &mut v,
            Sort {
                key: SortKey::Type,
                dir: SortDir::Asc,
            },
        );
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Makefile", "a.txt", "b.txt", "a.zip"]);
    }

    #[test]
    fn toggling_the_same_key_flips_direction() {
        let s = Sort::default().toggled(SortKey::Size);
        assert_eq!(
            s,
            Sort {
                key: SortKey::Size,
                dir: SortDir::Asc
            }
        );
        let s = s.toggled(SortKey::Size);
        assert_eq!(s.dir, SortDir::Desc);
        let s = s.toggled(SortKey::Name);
        assert_eq!(
            s,
            Sort {
                key: SortKey::Name,
                dir: SortDir::Asc
            }
        );
    }
}
