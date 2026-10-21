use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use super::Entry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Modified,
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
                    SortKey::Modified => a.modified.cmp(&b.modified),
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
    use std::path::PathBuf;

    fn entry(name: &str, kind: EntryKind, size: u64) -> Entry {
        Entry {
            name: name.into(),
            path: PathBuf::from(name),
            kind,
            size,
            modified: None,
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
}
