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

/// Iterative wildcard matching with backtracking on the last `*`.
fn glob_match(pattern: &[char], name: &[char]) -> bool {
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
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
}
