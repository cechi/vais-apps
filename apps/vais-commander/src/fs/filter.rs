//! Name matching for the panel filter: a case-insensitive substring, or a
//! glob pattern (`*`, `?`) when the query contains a wildcard.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Matcher {
    /// Match everything (empty query).
    All,
    /// Case-insensitive substring (lowercased query).
    Substring(String),
    /// Case-insensitive wildcard pattern (lowercased), `*` = any run, `?` = any char.
    Glob(Vec<char>),
}

impl Matcher {
    pub fn new(query: &str) -> Self {
        let query = query.trim();
        if query.is_empty() {
            Matcher::All
        } else if query.contains(['*', '?']) {
            Matcher::Glob(query.to_lowercase().chars().collect())
        } else {
            Matcher::Substring(query.to_lowercase())
        }
    }

    pub fn is_all(&self) -> bool {
        matches!(self, Matcher::All)
    }

    pub fn matches(&self, name: &str) -> bool {
        match self {
            Matcher::All => true,
            Matcher::Substring(needle) => name.to_lowercase().contains(needle.as_str()),
            Matcher::Glob(pattern) => {
                let name: Vec<char> = name.to_lowercase().chars().collect();
                glob_match(pattern, &name)
            }
        }
    }
}

/// Iterative wildcard matching with backtracking on the last `*`. `[...]`
/// is a character class (`a-z` ranges, `!` or `^` negates), the form GTK and
/// Qt use for case-insensitive portal filters (`*.[pP][dD][fF]`); a `[`
/// without its `]` is a literal.
fn glob_match(pattern: &[char], name: &[char]) -> bool {
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len()
            && pattern[p] != '*'
            && let Some((hit, next)) = match_one(pattern, p, name[n])
            && hit
        {
            p = next;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, n));
            p += 1;
        } else if let Some((sp, sn)) = star {
            p = sp + 1;
            n = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// Whether the pattern element at `p` (not `*`) matches `c`, and where the
/// next element starts.
fn match_one(pattern: &[char], p: usize, c: char) -> Option<(bool, usize)> {
    match pattern[p] {
        '?' => Some((true, p + 1)),
        '[' => match class_end(pattern, p) {
            Some(end) => Some((class_matches(&pattern[p + 1..end], c), end + 1)),
            None => Some((c == '[', p + 1)),
        },
        other => Some((other == c, p + 1)),
    }
}

/// The index of the `]` closing the class opened at `open`. A `]` right
/// after the `[` (or its negation) is a member, not the end.
fn class_end(pattern: &[char], open: usize) -> Option<usize> {
    let mut i = open + 1;
    if matches!(pattern.get(i), Some('!' | '^')) {
        i += 1;
    }
    if pattern.get(i) == Some(&']') {
        i += 1;
    }
    (i..pattern.len()).find(|&i| pattern[i] == ']')
}

fn class_matches(class: &[char], c: char) -> bool {
    let (negated, class) = match class.first() {
        Some('!' | '^') => (true, &class[1..]),
        _ => (false, class),
    };
    let mut hit = false;
    let mut i = 0;
    while i < class.len() {
        if i + 2 < class.len() && class[i + 1] == '-' {
            hit |= (class[i]..=class[i + 2]).contains(&c);
            i += 3;
        } else {
            hit |= class[i] == c;
            i += 1;
        }
    }
    hit != negated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substring_is_case_insensitive() {
        let m = Matcher::new("Rep");
        assert!(m.matches("reporting.pdf"));
        assert!(m.matches("Reporting_01_Master"));
        assert!(!m.matches("notes.txt"));
        assert!(Matcher::new("  ").is_all());
    }

    #[test]
    fn globs() {
        let m = Matcher::new("*.PNG");
        assert!(m.matches("shot.png"));
        assert!(!m.matches("shot.png.bak"));
        assert!(Matcher::new("report_??.csv").matches("Report_01.csv"));
        assert!(!Matcher::new("report_??.csv").matches("report_1.csv"));
        assert!(Matcher::new("a*b*c").matches("axxbyyc"));
        assert!(!Matcher::new("a*b*c").matches("axxbyy"));
        assert!(Matcher::new("*").matches("anything"));
    }

    #[test]
    fn glob_classes() {
        // GTK's add_suffix("pdf") and Qt send this to the file chooser portal.
        let m = Matcher::new("*.[pP][dD][fF]");
        assert!(m.matches("Faktura.PDF"));
        assert!(m.matches("faktura.pdf"));
        assert!(!m.matches("faktura.pdx"));
        assert!(Matcher::new("test-1[0-2].*").matches("test-11.pdf"));
        assert!(!Matcher::new("test-1[0-2].*").matches("test-13.pdf"));
        assert!(Matcher::new("*[!0-9]").matches("a.txt"));
        assert!(!Matcher::new("*[!0-9]").matches("test1"));
        assert!(Matcher::new("[]]*").matches("]x"));
        // An unclosed bracket is a literal.
        assert!(Matcher::new("a[b*").matches("a[bc"));
        assert!(!Matcher::new("a[b*").matches("abc"));
    }
}
