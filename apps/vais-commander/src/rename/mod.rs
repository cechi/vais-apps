//! Batch rename: the rules, the preview they produce and applying it. The
//! view lives in `view.rs`; this module is plain logic so it can be tested.
//!
//! A new name is built in three steps: the masks (`[N]` old name without
//! extension, `[E]` extension, `[C]` counter), then search & replace on the
//! result, then the case change.

pub mod view;

use std::io;

use crate::vfs::{Location, backend_for};

use regex::RegexBuilder;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaseMode {
    #[default]
    Keep,
    Lower,
    Upper,
    /// First letter of every word upper, the rest lower.
    Title,
}

impl CaseMode {
    pub const ALL: [CaseMode; 4] = [
        CaseMode::Keep,
        CaseMode::Lower,
        CaseMode::Upper,
        CaseMode::Title,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CaseMode::Keep => t!("Keep##case"),
            CaseMode::Lower => t!("lower"),
            CaseMode::Upper => t!("UPPER"),
            CaseMode::Title => t!("Title##case"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rules {
    /// Mask for the part before the extension; default `[N]`.
    pub name: String,
    /// Mask for the extension; default `[E]`. Empty drops the extension.
    pub ext: String,
    pub find: String,
    pub replace: String,
    pub regex: bool,
    pub ignore_case: bool,
    pub case: CaseMode,
    pub counter_start: i64,
    pub counter_step: i64,
    pub counter_digits: usize,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            name: "[N]".into(),
            ext: "[E]".into(),
            find: String::new(),
            replace: String::new(),
            regex: false,
            ignore_case: false,
            case: CaseMode::Keep,
            counter_start: 1,
            counter_step: 1,
            counter_digits: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    Empty,
    /// Contains a path separator (or is `.` / `..`).
    Invalid,
    /// Two entries would get the same name.
    Duplicate,
    /// Something else already has that name.
    Exists,
}

impl Problem {
    pub fn label(self) -> &'static str {
        match self {
            Problem::Empty => t!("empty name"),
            Problem::Invalid => t!("invalid name"),
            Problem::Duplicate => t!("duplicate"),
            Problem::Exists => t!("already exists"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub from: Location,
    pub old: String,
    pub new: String,
    pub problem: Option<Problem>,
}

impl Row {
    pub fn changes(&self) -> bool {
        self.old != self.new
    }
}

/// The preview: one row per path, in the given order. `regex_error` carries
/// the message for an invalid pattern (the find step is then skipped).
#[derive(Debug, Default)]
pub struct Plan {
    pub rows: Vec<Row>,
    pub regex_error: Option<String>,
}

impl Plan {
    pub fn problems(&self) -> usize {
        self.rows.iter().filter(|r| r.problem.is_some()).count()
    }

    pub fn changes(&self) -> usize {
        self.rows.iter().filter(|r| r.changes()).count()
    }

    pub fn can_apply(&self) -> bool {
        self.changes() > 0 && self.problems() == 0 && self.regex_error.is_none()
    }
}

/// Compute the new names for `paths` (all in the same directory).
pub fn plan(paths: &[Location], rules: &Rules) -> Plan {
    let mut plan = Plan::default();
    let finder = match (rules.regex, rules.find.is_empty()) {
        (_, true) => None,
        (true, false) => match RegexBuilder::new(&rules.find)
            .case_insensitive(rules.ignore_case)
            .build()
        {
            Ok(re) => Some(Finder::Regex(re)),
            Err(err) => {
                plan.regex_error = Some(err.to_string());
                None
            }
        },
        (false, false) => Some(Finder::Text),
    };

    let backend = paths.first().and_then(|p| backend_for(p).ok());
    for (i, path) in paths.iter().enumerate() {
        let old = path.file_name().unwrap_or_default();
        let is_dir = backend
            .as_ref()
            .and_then(|b| b.stat(path).ok())
            .is_some_and(|m| m.is_dir());
        let (stem, ext) = split(&old, is_dir);
        let counter = rules.counter_start + rules.counter_step * i as i64;
        let counter = format!("{counter:0width$}", width = rules.counter_digits);
        let expand = |mask: &str| {
            mask.replace("[N]", stem)
                .replace("[E]", ext)
                .replace("[C]", &counter)
        };
        let name = expand(&rules.name);
        let ext = expand(&rules.ext);
        let mut new = if ext.is_empty() {
            name
        } else {
            format!("{name}.{ext}")
        };
        new = match &finder {
            None => new,
            Some(Finder::Text) => {
                replace_text(&new, &rules.find, &rules.replace, rules.ignore_case)
            }
            Some(Finder::Regex(re)) => re.replace_all(&new, rules.replace.as_str()).into_owned(),
        };
        new = apply_case(&new, rules.case);
        plan.rows.push(Row {
            from: path.clone(),
            old,
            new,
            problem: None,
        });
    }

    // Problems: names that are no good, clash with each other, or with
    // something on disk that is not itself being renamed away.
    let olds: Vec<Location> = plan.rows.iter().map(|r| r.from.clone()).collect();
    let news: Vec<String> = plan.rows.iter().map(|r| r.new.clone()).collect();
    for (i, row) in plan.rows.iter_mut().enumerate() {
        row.problem = if row.new.is_empty() {
            Some(Problem::Empty)
        } else if row.new == "." || row.new == ".." || row.new.contains(['/', '\\']) {
            Some(Problem::Invalid)
        } else if news
            .iter()
            .enumerate()
            .any(|(j, n)| j != i && *n == row.new)
        {
            Some(Problem::Duplicate)
        } else if row.changes() {
            let target = row
                .from
                .parent()
                .map(|dir| dir.join(&row.new))
                .unwrap_or_else(|| row.from.clone());
            let taken =
                backend.as_ref().is_some_and(|b| b.exists(&target)) && !olds.contains(&target);
            taken.then_some(Problem::Exists)
        } else {
            None
        };
    }
    plan
}

enum Finder {
    Text,
    Regex(regex::Regex),
}

/// Rename every changed row. Names may swap, so each file first moves to a
/// temporary name, then to its final one. On an error the files already
/// moved are put back where they were, as far as possible.
pub fn apply(rows: &[Row]) -> io::Result<()> {
    let changed: Vec<&Row> = rows.iter().filter(|r| r.changes()).collect();
    let Some(first) = changed.first() else {
        return Ok(());
    };
    let backend = backend_for(&first.from)?;
    let dir = |row: &Row| row.from.parent().unwrap_or_else(|| row.from.clone());
    let temp = |i: usize, row: &Row| dir(row).join(&format!(".{}.vais-rename-{i}", row.old));

    let mut moved = Vec::new();
    let mut outcome = Ok(());
    for (i, row) in changed.iter().enumerate() {
        match backend.rename(&row.from, &temp(i, row)) {
            Ok(()) => moved.push(i),
            Err(err) => {
                outcome = Err(io::Error::new(err.kind(), format!("{}: {err}", row.old)));
                break;
            }
        }
    }
    if outcome.is_ok() {
        let mut finished = Vec::new();
        for &i in &moved {
            let row = changed[i];
            match backend.rename(&temp(i, row), &dir(row).join(&row.new)) {
                Ok(()) => finished.push(i),
                Err(err) => {
                    outcome = Err(io::Error::new(err.kind(), format!("{}: {err}", row.new)));
                    break;
                }
            }
        }
        if outcome.is_err() {
            // Undo the finished ones so that every file is back at a temp name.
            for &i in &finished {
                let row = changed[i];
                let _ = backend.rename(&dir(row).join(&row.new), &temp(i, row));
            }
        } else {
            return Ok(());
        }
    }
    for &i in &moved {
        let _ = backend.rename(&temp(i, changed[i]), &changed[i].from);
    }
    outcome
}

/// (stem, extension) of a file name; directories, dot-files and names
/// without a dot have no extension.
fn split(name: &str, is_dir: bool) -> (&str, &str) {
    if is_dir {
        return (name, "");
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 && dot + 1 < name.len() => (&name[..dot], &name[dot + 1..]),
        _ => (name, ""),
    }
}

/// Plain-text replace, optionally ignoring case (ASCII and Unicode simple
/// folding via lowercase comparison on char boundaries).
fn replace_text(text: &str, find: &str, replace: &str, ignore_case: bool) -> String {
    if !ignore_case {
        return text.replace(find, replace);
    }
    let lower_text = text.to_lowercase();
    let lower_find = find.to_lowercase();
    if lower_text.len() != text.len() {
        // Lowercasing changed byte lengths; fall back to an exact replace.
        return text.replace(find, replace);
    }
    let mut out = String::new();
    let mut i = 0;
    while let Some(pos) = lower_text[i..].find(&lower_find) {
        out.push_str(&text[i..i + pos]);
        out.push_str(replace);
        i += pos + lower_find.len();
    }
    out.push_str(&text[i..]);
    out
}

fn apply_case(name: &str, mode: CaseMode) -> String {
    match mode {
        CaseMode::Keep => name.to_string(),
        CaseMode::Lower => name.to_lowercase(),
        CaseMode::Upper => name.to_uppercase(),
        CaseMode::Title => {
            let mut out = String::with_capacity(name.len());
            let mut start = true;
            for c in name.chars() {
                if c.is_alphanumeric() {
                    if start {
                        out.extend(c.to_uppercase());
                    } else {
                        out.extend(c.to_lowercase());
                    }
                    start = false;
                } else {
                    out.push(c);
                    start = true;
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CaseMode, Problem, Rules, apply, apply_case, plan, replace_text, split};
    use crate::vfs::Location;

    fn paths(names: &[&str]) -> Vec<Location> {
        names
            .iter()
            .map(|n| Location::local(std::path::PathBuf::from("/nowhere").join(n)))
            .collect()
    }

    #[test]
    fn default_rules_keep_names() {
        let p = plan(&paths(&["a.txt", "b"]), &Rules::default());
        assert_eq!(p.rows[0].new, "a.txt");
        assert_eq!(p.rows[1].new, "b");
        assert_eq!(p.changes(), 0);
        assert!(!p.can_apply());
    }

    #[test]
    fn masks_and_counter() {
        let rules = Rules {
            name: "img_[C]_[N]".into(),
            ext: "[E]".into(),
            counter_start: 10,
            counter_step: 5,
            counter_digits: 3,
            ..Rules::default()
        };
        let p = plan(&paths(&["x.JPG", "y.png", "notes"]), &rules);
        let news: Vec<_> = p.rows.iter().map(|r| r.new.as_str()).collect();
        assert_eq!(news, ["img_010_x.JPG", "img_015_y.png", "img_020_notes"]);
        assert!(p.can_apply());
    }

    #[test]
    fn empty_extension_mask_drops_the_extension() {
        let rules = Rules {
            ext: String::new(),
            ..Rules::default()
        };
        let p = plan(&paths(&["a.txt"]), &rules);
        assert_eq!(p.rows[0].new, "a");
    }

    #[test]
    fn find_replace_text_and_regex() {
        let rules = Rules {
            find: "Photo".into(),
            replace: "img".into(),
            ignore_case: true,
            ..Rules::default()
        };
        let p = plan(&paths(&["photo1.jpg", "PHOTO2.jpg"]), &rules);
        assert_eq!(p.rows[0].new, "img1.jpg");
        assert_eq!(p.rows[1].new, "img2.jpg");

        let rules = Rules {
            find: r"^(\w+)-(\d+)".into(),
            replace: "$2-$1".into(),
            regex: true,
            ..Rules::default()
        };
        let p = plan(&paths(&["track-07.mp3"]), &rules);
        assert_eq!(p.rows[0].new, "07-track.mp3");
        assert!(p.regex_error.is_none());

        let bad = Rules {
            find: "(".into(),
            regex: true,
            ..Rules::default()
        };
        let p = plan(&paths(&["a"]), &bad);
        assert!(p.regex_error.is_some());
        assert_eq!(p.rows[0].new, "a");
    }

    #[test]
    fn case_modes() {
        assert_eq!(
            apply_case("my File-name.TXT", CaseMode::Lower),
            "my file-name.txt"
        );
        assert_eq!(apply_case("my File.txt", CaseMode::Upper), "MY FILE.TXT");
        assert_eq!(
            apply_case("my file-NAME.txt", CaseMode::Title),
            "My File-Name.Txt"
        );
        assert_eq!(replace_text("aXbxc", "x", "-", true), "a-b-c");
        assert_eq!(replace_text("aXbxc", "x", "-", false), "aXb-c");
    }

    #[test]
    fn problems_are_detected() {
        let rules = Rules {
            name: "same".into(),
            ext: String::new(),
            ..Rules::default()
        };
        let p = plan(&paths(&["a", "b"]), &rules);
        assert_eq!(p.rows[0].problem, Some(Problem::Duplicate));
        assert_eq!(p.rows[1].problem, Some(Problem::Duplicate));

        let rules = Rules {
            name: String::new(),
            ext: String::new(),
            ..Rules::default()
        };
        assert_eq!(
            plan(&paths(&["a"]), &rules).rows[0].problem,
            Some(Problem::Empty)
        );

        let rules = Rules {
            name: "x/[N]".into(),
            ..Rules::default()
        };
        assert_eq!(
            plan(&paths(&["a"]), &rules).rows[0].problem,
            Some(Problem::Invalid)
        );
    }

    #[test]
    fn split_handles_dirs_and_dotfiles() {
        assert_eq!(split("a.tar.gz", false), ("a.tar", "gz"));
        assert_eq!(split(".bashrc", false), (".bashrc", ""));
        assert_eq!(split("v1.0", true), ("v1.0", ""));
        assert_eq!(split("end.", false), ("end.", ""));
    }

    #[test]
    fn apply_swaps_names_through_temporaries() {
        let dir = std::env::temp_dir().join(format!("vais-rename-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a"), "A").unwrap();
        std::fs::write(dir.join("b"), "B").unwrap();
        // a -> b and b -> a at the same time.
        let rules = Rules {
            find: "a".into(),
            replace: "b".into(),
            ..Rules::default()
        };
        let mut p = plan(
            &[
                Location::local(dir.join("a")),
                Location::local(dir.join("b")),
            ],
            &rules,
        );
        p.rows[1].new = "a".into();
        p.rows[1].problem = None;
        p.rows[0].problem = None;
        apply(&p.rows).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("b")).unwrap(), "A");
        assert_eq!(std::fs::read_to_string(dir.join("a")).unwrap(), "B");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
