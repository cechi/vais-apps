//! Persistent configuration in `~/.config/vais-commander/config.toml`.

use std::path::PathBuf;

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use crate::connections::ConnectionConfig;
use crate::fs::SortKey;
use crate::layout::{Layout, MAX_PANELS};
use crate::panel::view::{Columns, ViewMode};
use crate::settings::{DropAction, FileIcons};
use crate::vfs::Location;

pub const CONFIG_VERSION: u32 = 2;

/// `"light"`, `"dark"`, `"system"` or the name of a theme from the themes directory.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum ThemeChoice {
    Light,
    Dark,
    #[default]
    System,
    Named(String),
}

impl From<String> for ThemeChoice {
    fn from(value: String) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "light" => ThemeChoice::Light,
            "dark" => ThemeChoice::Dark,
            "system" | "" => ThemeChoice::System,
            _ => ThemeChoice::Named(value.trim().to_string()),
        }
    }
}

impl From<ThemeChoice> for String {
    fn from(value: ThemeChoice) -> Self {
        match value {
            ThemeChoice::Light => "light".into(),
            ThemeChoice::Dark => "dark".into(),
            ThemeChoice::System => "system".into(),
            ThemeChoice::Named(name) => name,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub theme: ThemeChoice,
    /// UI scale factor; 1.0 is the default size.
    pub ui_scale: f32,
    /// Whether the places sidebar is shown.
    pub sidebar_open: bool,
    /// Sidebar groups folded away (by title: Favorites, Volumes, Connections, Recent).
    pub sidebar_collapsed: Vec<String>,
    /// Whether the command line under the panels is shown.
    pub command_line: bool,
    /// Whether the Agents panel at the right is shown, and how wide it is.
    pub agents_open: bool,
    pub agents_width: f32,
    /// Commands run from the command line, oldest first.
    pub command_history: Vec<String>,
    /// What dragging entries onto a directory does: ask, move or copy.
    pub drop_action: DropAction,
    /// File icons from the desktop theme or the built-in generic ones.
    pub file_icons: FileIcons,
    /// UI and monospace font family applied over the theme; `None` keeps the
    /// theme's (or the kit's) font.
    pub font_family: Option<String>,
    pub window: Option<WindowConfig>,
    pub active_tab: usize,
    pub tabs: Vec<TabConfig>,
    /// Directories the focused panel showed most recently, newest first.
    pub recent: Vec<Location>,
    /// Directories bookmarked with Ctrl+D (the sidebar's Favorites group).
    pub favorites: Vec<Location>,
    /// Whole tabs (layout + panels) saved from a tab's context menu.
    pub favorite_layouts: Vec<FavoriteLayout>,
    /// Saved remote connections.
    pub connections: Vec<ConnectionConfig>,
    /// Version 1 layout (single grid); read for migration, never written.
    #[serde(skip_serializing)]
    pub layout: Option<LayoutConfig>,
    /// Version 1 panels; read for migration, never written.
    #[serde(skip_serializing)]
    pub panels: Vec<PanelConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            theme: ThemeChoice::default(),
            ui_scale: 1.0,
            sidebar_open: false,
            sidebar_collapsed: Vec::new(),
            command_line: true,
            agents_open: false,
            agents_width: 420.0,
            command_history: Vec::new(),
            drop_action: DropAction::Ask,
            file_icons: FileIcons::System,
            font_family: None,
            window: None,
            active_tab: 0,
            tabs: Vec::new(),
            recent: Vec::new(),
            favorites: Vec::new(),
            favorite_layouts: Vec::new(),
            connections: Vec::new(),
            layout: None,
            panels: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabKind {
    #[default]
    Grid,
}

/// A tab saved under a name: opening it adds a tab with that layout and panels.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FavoriteLayout {
    pub name: String,
    pub tab: TabConfig,
}

/// One workspace tab. Only `Grid` exists so far; other kinds will add their own fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TabConfig {
    #[serde(rename = "type")]
    pub kind: TabKind,
    /// A fixed title given by the user; otherwise the tab shows the active
    /// panel's directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// A colour tag (`#rrggbb`) shown on the tab and on a saved layout.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub layout: LayoutConfig,
    pub panels: Vec<PanelConfig>,
}

impl TabConfig {
    /// Panel config for slot `ix`, defaulting to the home directory.
    pub fn panel(&self, ix: usize) -> PanelConfig {
        debug_assert!(ix < MAX_PANELS);
        self.panels.get(ix).cloned().unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutConfig {
    pub rows: u8,
    pub cols: u8,
    /// Fractions of the grid height, one per row.
    pub row_sizes: Vec<f32>,
    /// Fractions of the row width, one list per row.
    pub col_sizes: Vec<Vec<f32>>,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        let l = Layout::default();
        Self {
            rows: l.rows,
            cols: l.cols,
            row_sizes: Vec::new(),
            col_sizes: Vec::new(),
        }
    }
}

impl LayoutConfig {
    pub fn layout(&self) -> Layout {
        Layout::new(self.rows, self.cols)
    }

    /// Saved fractions, only if they are complete and sane for `layout`.
    pub fn fractions(&self) -> Option<(Vec<f32>, Vec<Vec<f32>>)> {
        let layout = self.layout();
        let ok = |v: &[f32], n: usize| {
            v.len() == n && v.iter().all(|f| f.is_finite() && *f > 0.0 && *f <= 1.0)
        };
        if !ok(&self.row_sizes, layout.rows as usize) {
            return None;
        }
        if self.col_sizes.len() != layout.rows as usize
            || !self.col_sizes.iter().all(|c| ok(c, layout.cols as usize))
        {
            return None;
        }
        Some((self.row_sizes.clone(), self.col_sizes.clone()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowConfig {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub maximized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelConfig {
    pub path: Location,
    pub show_hidden: bool,
    pub sort: SortKey,
    pub sort_desc: bool,
    pub view: ViewMode,
    pub columns: Columns,
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            path: Location::Local(home_dir()),
            show_hidden: false,
            sort: SortKey::Name,
            sort_desc: false,
            view: ViewMode::Full,
            columns: Columns::default(),
        }
    }
}

pub fn home_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(crate::platform::default_root)
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "vais-commander")
            .map(|d| d.config_dir().join("config.toml"))
    }

    /// Load the config, falling back to defaults (with a logged warning) on any error.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            log::warn!("no config directory available, using defaults");
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Config>(&text) {
                Ok(cfg) => cfg,
                Err(err) => {
                    log::warn!("ignoring invalid config {}: {err}", path.display());
                    Self::default()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(err) => {
                log::warn!("cannot read config {}: {err}", path.display());
                Self::default()
            }
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path().context("no config directory available")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let text = toml::to_string_pretty(self).context("serializing config")?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("renaming to {}", path.display()))?;
        Ok(())
    }

    /// The tabs to open: the saved ones, a version 1 config migrated into a
    /// single tab, or one default tab.
    pub fn tabs(&self) -> Vec<TabConfig> {
        if !self.tabs.is_empty() {
            return self.tabs.clone();
        }
        vec![TabConfig {
            kind: TabKind::Grid,
            title: None,
            color: None,
            layout: self.layout.clone().unwrap_or_default(),
            panels: self.panels.clone(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_partial_parse() {
        let mut cfg = Config::default();
        cfg.tabs.push(TabConfig {
            layout: LayoutConfig {
                rows: 2,
                cols: 2,
                ..Default::default()
            },
            panels: vec![PanelConfig {
                path: "/tmp".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        cfg.active_tab = 0;
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert!(text.contains("[[tabs]]"));
        assert!(
            !text.contains("[layout]"),
            "legacy fields must not be written"
        );
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.tabs.len(), 1);
        assert_eq!(back.tabs[0].layout.rows, 2);
        assert_eq!(back.tabs[0].panels[0].path, Location::local("/tmp"));

        let partial: Config = toml::from_str("[[tabs]]\n[tabs.layout]\nrows = 3\n").unwrap();
        assert_eq!(partial.tabs()[0].layout.rows, 3);
        assert_eq!(partial.tabs()[0].layout.cols, 2);
    }

    #[test]
    fn version1_config_migrates_to_one_tab() {
        let v1 = "version = 1\n[layout]\nrows = 2\ncols = 3\n[[panels]]\npath = \"/usr\"\n";
        let cfg: Config = toml::from_str(v1).unwrap();
        let tabs = cfg.tabs();
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].layout.rows, 2);
        assert_eq!(tabs[0].layout.cols, 3);
        assert_eq!(tabs[0].panel(0).path, Location::local("/usr"));
        assert_eq!(tabs[0].panel(5).path, Location::Local(home_dir()));

        let empty = Config::default();
        assert_eq!(empty.tabs().len(), 1);
    }

    #[test]
    fn theme_choice_roundtrips_names() {
        let cfg: Config = toml::from_str("theme = \"Dank Dark\"\n").unwrap();
        assert_eq!(cfg.theme, ThemeChoice::Named("Dank Dark".into()));
        let cfg: Config = toml::from_str("theme = \"dark\"\n").unwrap();
        assert_eq!(cfg.theme, ThemeChoice::Dark);
        let text = toml::to_string(&Config {
            theme: ThemeChoice::Named("X".into()),
            ..Default::default()
        })
        .unwrap();
        assert!(text.contains("theme = \"X\""));
    }

    #[test]
    fn fractions_are_validated() {
        let mut lc = LayoutConfig {
            rows: 1,
            cols: 2,
            row_sizes: vec![1.0],
            col_sizes: vec![vec![0.3, 0.7]],
        };
        assert!(lc.fractions().is_some());
        lc.col_sizes = vec![vec![0.3]];
        assert!(lc.fractions().is_none());
    }
}
