//! Themes: the built-in light/dark pair plus any gpui-kit theme files dropped
//! into `<config dir>/themes/*.json` (watched, so edits apply live).

use std::path::PathBuf;

use gpui_kit::component::theme::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::*;

use crate::config::{Config, ThemeChoice};

/// Where user theme files live.
pub fn themes_dir() -> Option<PathBuf> {
    Config::path().and_then(|p| p.parent().map(|d| d.join("themes")))
}

/// Start loading and watching the themes directory. `on_load` runs on the app
/// context every time the directory has been (re)loaded.
pub fn watch(cx: &mut App, on_load: impl Fn(&mut App) + 'static) {
    let Some(dir) = themes_dir() else {
        return;
    };
    if let Err(err) = std::fs::create_dir_all(&dir) {
        log::warn!("cannot create themes dir {}: {err}", dir.display());
    }
    if let Err(err) = ThemeRegistry::watch_dir(dir.clone(), cx, on_load) {
        log::warn!("cannot watch themes dir {}: {err}", dir.display());
    }
}

/// Names of all registered themes with their mode, sorted by name.
pub fn available(cx: &App) -> Vec<(String, ThemeMode)> {
    let mut themes: Vec<(String, ThemeMode)> = ThemeRegistry::global(cx)
        .themes()
        .values()
        .map(|t| (t.name.to_string(), t.mode))
        .collect();
    themes.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    themes
}

/// Apply the configured theme. A named theme that is not (yet) registered
/// falls back to the system appearance; returns whether the name resolved.
pub fn apply(choice: &ThemeChoice, window: Option<&mut Window>, cx: &mut App) -> bool {
    match choice {
        ThemeChoice::Light => {
            Theme::change(ThemeMode::Light, window, cx);
            true
        }
        ThemeChoice::Dark => {
            Theme::change(ThemeMode::Dark, window, cx);
            true
        }
        ThemeChoice::System => {
            Theme::sync_system_appearance(window, cx);
            true
        }
        ThemeChoice::Named(name) => {
            let config = ThemeRegistry::global(cx)
                .themes()
                .get(name.as_str())
                .cloned();
            match config {
                Some(config) => {
                    Theme::update(cx, |theme| theme.apply_config(&config));
                    Theme::change(config.mode, window, cx);
                    true
                }
                None => {
                    Theme::sync_system_appearance(window, cx);
                    false
                }
            }
        }
    }
}
