//! Themes: the built-in light/dark pair plus any gpui-kit theme files dropped
//! into `<config dir>/themes/*.json` (watched, so edits apply live).

use std::path::PathBuf;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::theme::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::*;

use crate::config::{Config, ThemeChoice};

/// Where user theme files live.
pub fn themes_dir() -> Option<PathBuf> {
    Config::path().and_then(|p| p.parent().map(|d| d.join("themes")))
}

/// Themes shipped with the app, besides the kit's Default Light / Dark.
const BUNDLED: &str = include_str!("../assets/themes/cechi.json");

/// Register the bundled themes. The registry rebuilds its list from the
/// defaults and the themes directory on every reload, so this runs after
/// each one (and once at startup).
pub fn load_bundled(cx: &mut App) {
    if let Err(err) = ThemeRegistry::global_mut(cx).load_themes_from_str(BUNDLED) {
        log::error!("bundled themes: {err:#}");
    }
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
    let on_load = move |cx: &mut App| {
        load_bundled(cx);
        on_load(cx);
    };
    if let Err(err) = ThemeRegistry::watch_dir(dir.clone(), cx, on_load) {
        log::warn!("cannot watch themes dir {}: {err}", dir.display());
    }
}

/// Apply the user's font family on top of the theme (themes are shared,
/// fonts are personal). `None` keeps what the theme or the kit sets.
/// Unscaled font sizes of the active theme: what its config declares, or
/// the kit defaults (16 / 13 px).
pub fn base_font_sizes(cx: &App) -> (Pixels, Pixels) {
    let theme = cx.theme();
    let config = if theme.is_dark() {
        &theme.dark_theme
    } else {
        &theme.light_theme
    };
    (
        config.font_size.map(px).unwrap_or(px(16.)),
        config.mono_font_size.map(px).unwrap_or(px(13.)),
    )
}

/// The UI scale: the theme's font sizes times `scale`, with the font family
/// applied again (the theme's sizes are the base, so this is idempotent).
pub fn apply_scale(scale: f32, family: Option<&str>, cx: &mut App) {
    let (base, base_mono) = base_font_sizes(cx);
    let (font, mono) = (base * scale, base_mono * scale);
    Theme::update(cx, |theme| {
        theme.font_size = font;
        theme.mono_font_size = mono;
    });
    apply_font(family, cx);
}

pub fn apply_font(family: Option<&str>, cx: &mut App) {
    let Some(family) = family.filter(|f| !f.trim().is_empty()) else {
        return;
    };
    let family: SharedString = family.trim().to_string().into();
    Theme::update(cx, |theme| {
        theme.font_family = family.clone();
        theme.mono_font_family = family;
    });
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
