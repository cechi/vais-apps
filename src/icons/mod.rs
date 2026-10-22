//! File icons from the desktop.
//!
//! Linux (freedesktop.org): a file name is mapped to a MIME type through the
//! shared-mime-info glob database, the MIME type to an icon name
//! (`text-plain`, then the generic `text-x-generic`), and the icon name to a
//! file in the current icon theme or one it inherits from.
//!
//! Windows: the shell's icon for the extension, read through
//! `SHGetFileInfo` and handed to GPUI as a PNG (see `shell.rs`).
//!
//! Lookups are cached per extension. Everything is best effort: when
//! something is missing the caller draws a generic icon.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

#[cfg(target_os = "windows")]
mod shell;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::*;

/// Icons are rasterised at their own size, so ask for one that still looks
/// crisp when scaled down to a row.
const WANTED_SIZE: u32 = 32;

#[derive(Default)]
pub struct IconTheme {
    /// Theme chain (the chosen theme, what it inherits, hicolor), each with
    /// its directories.
    themes: Vec<Theme>,
    /// Where theme directories live.
    base_dirs: Vec<PathBuf>,
    /// `*.ext` (lowercase, without the dot) → MIME type.
    by_ext: HashMap<String, String>,
    /// Literal file names (`Makefile`) → MIME type.
    by_name: HashMap<String, String>,
    /// MIME type → icon name overrides from the `icons` file.
    mime_icons: HashMap<String, String>,
    /// MIME type → generic icon name.
    generic_icons: HashMap<String, String>,
    cache: RefCell<HashMap<String, Option<ImageSource>>>,
}

impl Global for IconTheme {}

struct Theme {
    name: String,
    dirs: Vec<ThemeDir>,
}

struct ThemeDir {
    path: String,
    size: u32,
    kind: DirKind,
    min: u32,
    max: u32,
    threshold: u32,
    scale: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DirKind {
    Fixed,
    Scalable,
    Threshold,
}

impl ThemeDir {
    fn matches(&self, size: u32) -> bool {
        match self.kind {
            DirKind::Fixed => self.size == size,
            DirKind::Scalable => self.min <= size && size <= self.max,
            DirKind::Threshold => self.size.abs_diff(size) <= self.threshold,
        }
    }

    fn distance(&self, size: u32) -> u32 {
        match self.kind {
            DirKind::Fixed => self.size.abs_diff(size),
            DirKind::Scalable => {
                if size < self.min {
                    self.min - size
                } else if size > self.max {
                    size - self.max
                } else {
                    0
                }
            }
            DirKind::Threshold => {
                if self.size.abs_diff(size) <= self.threshold {
                    0
                } else if size < self.size {
                    self.size - self.threshold - size
                } else {
                    size - self.size - self.threshold
                }
            }
        }
    }
}

pub fn init(cx: &mut App) {
    cx.set_global(IconTheme::load());
}

/// The desktop's icon for the file `name`, if it has one. Directories keep
/// the built-in folder icon, so only files are looked up.
pub fn file_icon(cx: &App, name: &str) -> Option<ImageSource> {
    cx.global::<IconTheme>().file_icon(name)
}

impl IconTheme {
    pub fn load() -> Self {
        if !cfg!(target_os = "linux") {
            return Self::default();
        }
        let mut this = Self {
            base_dirs: base_dirs(),
            ..Self::default()
        };
        this.load_mime_db();
        let name = current_theme_name();
        let mut seen = HashSet::new();
        this.add_theme(&name, &mut seen);
        this.add_theme("hicolor", &mut seen);
        log::info!(
            "icon theme {name}: {} themes in chain, {} mime globs",
            this.themes.len(),
            this.by_ext.len()
        );
        this
    }

    pub fn file_icon(&self, name: &str) -> Option<ImageSource> {
        let key = icon_key(name);
        if let Some(hit) = self.cache.borrow().get(&key) {
            return hit.clone();
        }
        let found = self.lookup(name);
        self.cache.borrow_mut().insert(key, found.clone());
        found
    }

    #[cfg(target_os = "linux")]
    fn lookup(&self, name: &str) -> Option<ImageSource> {
        if self.themes.is_empty() {
            return None;
        }
        let names = self.icon_names(name);
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let path = self.find_any(&refs)?;
        Some(ImageSource::from(path))
    }

    #[cfg(target_os = "windows")]
    fn lookup(&self, name: &str) -> Option<ImageSource> {
        let png = shell::icon_png(name)?;
        Some(ImageSource::Image(Arc::new(Image::from_bytes(
            ImageFormat::Png,
            png,
        ))))
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    fn lookup(&self, _name: &str) -> Option<ImageSource> {
        None
    }

    /// Candidate icon names for a file, most specific first.
    fn icon_names(&self, name: &str) -> Vec<String> {
        let mut names = Vec::new();
        if let Some(mime) = self.mime_of(name) {
            if let Some(icon) = self.mime_icons.get(&mime) {
                names.push(icon.clone());
            }
            names.push(mime.replace('/', "-"));
            if let Some(icon) = self.generic_icons.get(&mime) {
                names.push(icon.clone());
            }
            if let Some((media, _)) = mime.split_once('/') {
                names.push(format!("{media}-x-generic"));
            }
        }
        names.push("unknown".into());
        names.push("text-x-generic".into());
        names
    }

    fn mime_of(&self, name: &str) -> Option<String> {
        let lower = name.to_lowercase();
        if let Some(mime) = self.by_name.get(&lower) {
            return Some(mime.clone());
        }
        // Longest matching suffix first: `tar.gz` before `gz`.
        let mut rest = lower.as_str();
        while let Some(dot) = rest.find('.') {
            rest = &rest[dot + 1..];
            if rest.is_empty() {
                break;
            }
            if let Some(mime) = self.by_ext.get(rest) {
                return Some(mime.clone());
            }
        }
        None
    }

    fn find_any(&self, names: &[&str]) -> Option<Arc<Path>> {
        for theme in &self.themes {
            for name in names {
                if let Some(path) = self.find_in_theme(theme, name) {
                    return Some(path);
                }
            }
        }
        None
    }

    /// The icon in `theme`: a directory matching the wanted size, else the
    /// closest one.
    fn find_in_theme(&self, theme: &Theme, icon: &str) -> Option<Arc<Path>> {
        let exact = theme
            .dirs
            .iter()
            .filter(|d| d.scale == 1 && d.matches(WANTED_SIZE));
        let mut candidates: Vec<&ThemeDir> = exact.collect();
        if candidates.is_empty() {
            let mut all: Vec<&ThemeDir> = theme.dirs.iter().collect();
            all.sort_by_key(|d| (d.distance(WANTED_SIZE), d.scale));
            candidates = all;
        }
        for dir in candidates {
            for base in &self.base_dirs {
                let dir_path = base.join(&theme.name).join(&dir.path);
                for ext in ["svg", "png"] {
                    let path = dir_path.join(format!("{icon}.{ext}"));
                    if path.is_file() {
                        return Some(Arc::from(path));
                    }
                }
            }
        }
        None
    }

    fn add_theme(&mut self, name: &str, seen: &mut HashSet<String>) {
        if !seen.insert(name.to_string()) {
            return;
        }
        let Some(index) = self
            .base_dirs
            .iter()
            .map(|b| b.join(name).join("index.theme"))
            .find(|p| p.is_file())
        else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&index) else {
            return;
        };
        let (dirs, inherits) = parse_index_theme(&text);
        self.themes.push(Theme {
            name: name.to_string(),
            dirs,
        });
        for parent in inherits {
            self.add_theme(&parent, seen);
        }
    }

    fn load_mime_db(&mut self) {
        // System first, user last so the user's entries win.
        let mut roots = vec![PathBuf::from("/usr/share/mime")];
        roots.extend(
            std::env::var_os("XDG_DATA_DIRS")
                .map(|d| {
                    std::env::split_paths(&d)
                        .map(|p| p.join("mime"))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        );
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(Path::new(&home).join(".local/share/mime"));
        }
        for root in roots {
            if let Ok(text) = std::fs::read_to_string(root.join("globs2")) {
                self.parse_globs2(&text);
            }
            if let Ok(text) = std::fs::read_to_string(root.join("generic-icons")) {
                parse_pairs(&text, &mut self.generic_icons);
            }
            if let Ok(text) = std::fs::read_to_string(root.join("icons")) {
                parse_pairs(&text, &mut self.mime_icons);
            }
        }
    }

    /// `weight:mime/type:*.ext` lines; higher weights come first in the file,
    /// so the first entry for a pattern wins.
    fn parse_globs2(&mut self, text: &str) {
        for line in text.lines() {
            if line.starts_with('#') {
                continue;
            }
            let mut parts = line.splitn(3, ':');
            let (Some(_weight), Some(mime), Some(pattern)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let pattern = pattern.split(':').next().unwrap_or(pattern);
            if let Some(ext) = pattern.strip_prefix("*.") {
                if !ext.contains(['*', '?', '[']) {
                    self.by_ext
                        .entry(ext.to_lowercase())
                        .or_insert_with(|| mime.to_string());
                }
            } else if !pattern.contains(['*', '?', '[']) {
                self.by_name
                    .entry(pattern.to_lowercase())
                    .or_insert_with(|| mime.to_string());
            }
        }
    }
}

/// Cache key: the extension when there is one, else the whole name.
fn icon_key(name: &str) -> String {
    let lower = name.to_lowercase();
    match lower.find('.') {
        Some(dot) if dot > 0 => lower[dot + 1..].to_string(),
        _ => format!("/name/{lower}"),
    }
}

/// `mime/type:icon-name` lines.
fn parse_pairs(text: &str, into: &mut HashMap<String, String>) {
    for line in text.lines() {
        if let Some((mime, icon)) = line.split_once(':') {
            into.insert(mime.to_string(), icon.to_string());
        }
    }
}

/// Directories (with their size rules) and the inherited themes.
fn parse_index_theme(text: &str) -> (Vec<ThemeDir>, Vec<String>) {
    let mut section = String::new();
    let mut inherits = Vec::new();
    let mut listed: Vec<String> = Vec::new();
    let mut props: HashMap<String, HashMap<String, String>> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if section == "Icon Theme" {
            match key {
                "Inherits" => {
                    inherits = value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                }
                "Directories" | "ScaledDirectories" => listed.extend(
                    value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty()),
                ),
                _ => {}
            }
        } else {
            props
                .entry(section.clone())
                .or_default()
                .insert(key.to_string(), value.to_string());
        }
    }
    let dirs = listed
        .into_iter()
        .filter_map(|path| {
            let p = props.get(&path)?;
            let num = |key: &str, default: u32| {
                p.get(key).and_then(|v| v.parse().ok()).unwrap_or(default)
            };
            let size = num("Size", 0);
            if size == 0 {
                return None;
            }
            let kind = match p.get("Type").map(String::as_str) {
                Some("Scalable") => DirKind::Scalable,
                Some("Fixed") => DirKind::Fixed,
                _ => DirKind::Threshold,
            };
            Some(ThemeDir {
                path,
                size,
                kind,
                min: num("MinSize", size),
                max: num("MaxSize", size),
                threshold: num("Threshold", 2),
                scale: num("Scale", 1),
            })
        })
        .collect();
    (dirs, inherits)
}

fn base_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = Path::new(&home);
        dirs.push(home.join(".icons"));
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        dirs.push(data_home.join("icons"));
    }
    let data_dirs = std::env::var_os("XDG_DATA_DIRS")
        .map(|d| std::env::split_paths(&d).collect::<Vec<_>>())
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| {
            vec![
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share"),
            ]
        });
    dirs.extend(data_dirs.iter().map(|d| d.join("icons")));
    dirs.push(PathBuf::from("/usr/share/pixmaps"));
    dirs.into_iter().filter(|d| d.is_dir()).collect()
}

/// The desktop's icon theme: GTK settings files, then KDE's, then gsettings,
/// then hicolor.
fn current_theme_name() -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".config")));
    if let Some(config) = &config_home {
        for file in ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"] {
            if let Some(name) = ini_value(&config.join(file), "gtk-icon-theme-name") {
                return name;
            }
        }
        if let Some(name) = ini_value(&config.join("kdeglobals"), "Theme") {
            return name;
        }
    }
    if let Some(home) = &home
        && let Some(name) = ini_value(&home.join(".gtkrc-2.0"), "gtk-icon-theme-name")
    {
        return name;
    }
    if let Ok(out) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .output()
        && out.status.success()
    {
        let name = String::from_utf8_lossy(&out.stdout)
            .trim()
            .trim_matches('\'')
            .to_string();
        if !name.is_empty() {
            return name;
        }
    }
    "hicolor".into()
}

/// First `key=value` in an INI-ish file (any section), unquoted.
fn ini_value(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::{DirKind, icon_key, parse_index_theme};

    #[test]
    fn parses_index_theme_directories_and_inherits() {
        let text = "[Icon Theme]\nName=T\nInherits=parent,hicolor\nDirectories=a/16,b/32,c/scalable\n\n[a/16]\nSize=16\nType=Fixed\n\n[b/32]\nSize=32\nContext=MimeTypes\n\n[c/scalable]\nSize=64\nType=Scalable\nMinSize=16\nMaxSize=256\n";
        let (dirs, inherits) = parse_index_theme(text);
        assert_eq!(inherits, ["parent", "hicolor"]);
        assert_eq!(dirs.len(), 3);
        assert!(dirs[0].kind == DirKind::Fixed && dirs[0].matches(16) && !dirs[0].matches(32));
        assert!(dirs[1].kind == DirKind::Threshold && dirs[1].matches(33));
        assert!(dirs[2].kind == DirKind::Scalable && dirs[2].matches(32));
        assert_eq!(dirs[0].distance(32), 16);
        assert_eq!(dirs[2].distance(512), 256);
    }

    #[test]
    fn cache_keys_use_the_extension_or_the_name() {
        assert_eq!(icon_key("Photo.JPG"), "jpg");
        assert_eq!(icon_key("archive.tar.gz"), "tar.gz");
        assert_eq!(icon_key("Makefile"), "/name/makefile");
        assert_eq!(icon_key(".bashrc"), "/name/.bashrc");
    }
}
