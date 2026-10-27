//! Translations of the user interface. English is the source language: the
//! text in the code is the key, `locales/<code>.toml` maps it to the
//! translation, and anything missing falls back to the English text.
//!
//! - `t!("Copy")` translates a literal; `t!("Copy {name}?", name = x)` also
//!   fills in the named placeholders (a `String` then, a `&'static str`
//!   otherwise).
//! - `tn!("{n} file", "{n} files", count)` picks the plural form the language
//!   needs for `count` (filled in as `{n}`); the catalog keys it by the
//!   English plural and holds a list of forms in CLDR order (one, few, many,
//!   other, as far as the language has them).
//! - A key may carry a context after `##` (`"Open##state"`) when one English
//!   text needs two translations; English shows the part before it.
//!
//! The test at the bottom checks every catalog against the keys in the code.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct Language {
    /// BCP 47 code, as in the catalog's file name.
    pub code: &'static str,
    /// The language's name in itself, for the chooser.
    pub name: &'static str,
    /// The name in English, as a detail in the chooser.
    pub english: &'static str,
    catalog: &'static str,
    /// Which of the catalog's plural forms a count takes.
    plural: fn(u64) -> usize,
}

pub const LANGUAGES: &[Language] = &[
    Language {
        code: "en",
        name: "English",
        english: "English",
        catalog: "",
        plural: one_other,
    },
    Language {
        code: "cs",
        name: "Čeština",
        english: "Czech",
        catalog: include_str!("../locales/cs.toml"),
        plural: czech,
    },
];

/// Index into [`LANGUAGES`].
static CURRENT: AtomicUsize = AtomicUsize::new(0);
static CATALOGS: [OnceLock<Catalog>; LANGUAGES.len()] =
    [const { OnceLock::new() }; LANGUAGES.len()];

enum Entry {
    Text(&'static str),
    Plural(Vec<&'static str>),
}

type Catalog = HashMap<&'static str, Entry>;

/// Switch the interface language: a code from [`LANGUAGES`], or `None` for
/// the system's. Texts already on screen change with the next render.
pub fn set_language(code: Option<&str>) {
    let system = sys_locale::get_locale();
    let index = code.or(system.as_deref()).and_then(find).unwrap_or(0);
    CURRENT.store(index, Ordering::Relaxed);
    // The components' own texts (input menus, dialogs); they fall back to
    // English for languages they lack.
    gpui_kit::component::set_locale(LANGUAGES[index].code);
}

/// The language a `None` setting gives: the system's, if there is a catalog.
pub fn system_language() -> &'static Language {
    let index = sys_locale::get_locale()
        .as_deref()
        .and_then(find)
        .unwrap_or(0);
    &LANGUAGES[index]
}

pub fn current() -> &'static Language {
    &LANGUAGES[CURRENT.load(Ordering::Relaxed)]
}

/// The language for a locale such as `cs-CZ`, `cs_CZ.UTF-8` or `pt-BR`: the
/// exact code first, then the language part alone.
fn find(locale: &str) -> Option<usize> {
    let locale = locale.split('.').next().unwrap_or(locale).replace('_', "-");
    let exact = LANGUAGES
        .iter()
        .position(|l| l.code.eq_ignore_ascii_case(&locale));
    exact.or_else(|| {
        let language = locale.split('-').next()?;
        LANGUAGES.iter().position(|l| {
            l.code.split('-').next() == Some(language) || l.code.eq_ignore_ascii_case(language)
        })
    })
}

fn catalog(index: usize) -> &'static Catalog {
    CATALOGS[index].get_or_init(|| parse(LANGUAGES[index].catalog, LANGUAGES[index].code))
}

/// Catalogs live for the whole run, so their strings are leaked once.
fn parse(source: &str, code: &str) -> Catalog {
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let table: toml::Table = match source.parse() {
        Ok(table) => table,
        Err(err) => {
            log::warn!("locales/{code}.toml: {err}");
            return Catalog::new();
        }
    };
    table
        .into_iter()
        .filter_map(|(key, value)| {
            let entry = match value {
                toml::Value::String(text) => Entry::Text(leak(text)),
                toml::Value::Array(forms) => Entry::Plural(
                    forms
                        .into_iter()
                        .filter_map(|f| f.as_str().map(|s| leak(s.to_string())))
                        .collect(),
                ),
                _ => return None,
            };
            Some((leak(key), entry))
        })
        .collect()
}

/// The English text of a key: without its `##context`.
fn english(key: &str) -> &str {
    key.split_once("##").map_or(key, |(text, _)| text)
}

/// The translation of `key` in the current language.
pub fn tr(key: &'static str) -> &'static str {
    tr_str(key)
}

/// Like [`tr`] for a key built at run time (command names).
pub fn tr_str(key: &str) -> &str {
    let index = CURRENT.load(Ordering::Relaxed);
    if index != 0 {
        if let Some(Entry::Text(text)) = catalog(index).get(key) {
            return text;
        }
    }
    english(key)
}

/// The form of a plural text for `n`.
pub fn trn(one: &'static str, other: &'static str, n: u64) -> &'static str {
    let index = CURRENT.load(Ordering::Relaxed);
    if index != 0 {
        if let Some(Entry::Plural(forms)) = catalog(index).get(other) {
            if let Some(form) = forms.get((LANGUAGES[index].plural)(n)) {
                return form;
            }
        }
    }
    english(if n == 1 { one } else { other })
}

/// Replace `{name}` placeholders.
pub fn fill(template: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = template.to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), &value.to_string());
    }
    out
}

macro_rules! t {
    ($text:literal) => {
        $crate::i18n::tr($text)
    };
    ($text:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::fill(
            $crate::i18n::tr($text),
            &[$((stringify!($name), &$value as &dyn std::fmt::Display)),+],
        )
    };
}

macro_rules! tn {
    ($one:literal, $other:literal, $n:expr $(, $name:ident = $value:expr)* $(,)?) => {{
        let n = $n;
        $crate::i18n::fill(
            $crate::i18n::trn($one, $other, n as u64),
            &[("n", &n as &dyn std::fmt::Display) $(, (stringify!($name), &$value as &dyn std::fmt::Display))*],
        )
    }};
}

// ----- plural rules (CLDR, integers only) ------------------------------------

fn one_other(n: u64) -> usize {
    usize::from(n != 1)
}

/// Czech and Slovak: 1 / 2–4 / the rest.
fn czech(n: u64) -> usize {
    match n {
        1 => 0,
        2..=4 => 1,
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    /// Every `t!` / `tn!` key in the sources, with how many plural forms the
    /// English side has (1 for plain texts, 2 for `tn!`).
    fn source_keys() -> BTreeMap<String, bool> {
        let call =
            regex::Regex::new(r#"\b(tn?)!\(\s*"((?:[^"\\]|\\.)*)"(?:\s*,\s*"((?:[^"\\]|\\.)*)")?"#)
                .unwrap();
        let mut keys = BTreeMap::new();
        let mut stack = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("i18n.rs")
                {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for c in call.captures_iter(&text) {
                        let plural = &c[1] == "tn";
                        let key = if plural {
                            c.get(3).map(|m| m.as_str())
                        } else {
                            Some(&c[2])
                        };
                        if let Some(key) = key {
                            keys.insert(unescape(key), plural);
                        }
                    }
                }
            }
        }
        for label in crate::keymap::command_labels() {
            keys.entry(label).or_insert(false);
        }
        keys
    }

    fn unescape(s: &str) -> String {
        s.replace("\\\"", "\"")
            .replace("\\n", "\n")
            .replace("\\\\", "\\")
    }

    fn placeholders(s: &str) -> BTreeSet<String> {
        let re = regex::Regex::new(r"\{([a-z_]+)\}").unwrap();
        re.captures_iter(s).map(|c| c[1].to_string()).collect()
    }

    fn toml_key(s: &str) -> String {
        format!("{:?}", s)
    }

    /// Each catalog has exactly the keys of the code, plain or plural as the
    /// code uses them, with the same placeholders and the language's number
    /// of plural forms. A failure prints the missing entries as TOML.
    #[test]
    fn catalogs_match_the_sources() {
        let keys = source_keys();
        let mut problems = Vec::new();
        for (index, language) in LANGUAGES.iter().enumerate().skip(1) {
            let catalog = catalog(index);
            let forms = (0..200).map(language.plural).max().unwrap() + 1;
            let mut missing = String::new();
            for (key, &plural) in &keys {
                match (catalog.get(key.as_str()), plural) {
                    (None, false) => missing.push_str(&format!("{} = \"\"\n", toml_key(key))),
                    (None, true) => missing.push_str(&format!(
                        "{} = [{}]\n",
                        toml_key(key),
                        vec!["\"\""; forms].join(", ")
                    )),
                    (Some(Entry::Text(text)), false) => {
                        if placeholders(text) != placeholders(key) {
                            problems
                                .push(format!("{}: placeholders differ in {key:?}", language.code));
                        }
                    }
                    (Some(Entry::Plural(list)), true) => {
                        if list.len() != forms {
                            problems.push(format!(
                                "{}: {key:?} has {} plural forms, the language has {forms}",
                                language.code,
                                list.len()
                            ));
                        }
                        if list
                            .iter()
                            .any(|f| !placeholders(f).is_subset(&placeholders(key)))
                        {
                            problems
                                .push(format!("{}: unknown placeholder in {key:?}", language.code));
                        }
                    }
                    _ => problems.push(format!(
                        "{}: {key:?} is {} in the code",
                        language.code,
                        if plural { "plural" } else { "not plural" }
                    )),
                }
            }
            for key in catalog.keys() {
                if !keys.contains_key(*key) {
                    problems.push(format!("{}: stale key {key:?}", language.code));
                }
            }
            if !missing.is_empty() {
                problems.push(format!("{}: missing keys:\n{missing}", language.code));
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn locales_resolve() {
        assert_eq!(find("cs_CZ.UTF-8"), Some(1));
        assert_eq!(find("cs-CZ"), Some(1));
        assert_eq!(find("en-US"), Some(0));
        assert_eq!(find("xx"), None);
    }

    #[test]
    fn czech_plurals() {
        assert_eq!([1, 2, 4, 5, 0, 11, 22].map(czech), [0, 1, 1, 2, 2, 2, 2]);
    }

    #[test]
    fn fill_replaces_named_placeholders() {
        assert_eq!(fill("{a} and {b}", &[("a", &1), ("b", &"x")]), "1 and x");
    }
}
