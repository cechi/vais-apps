//! User preferences that are not layout state. Stored in the config file and
//! available everywhere as a GPUI global; a settings UI can edit them later.

use gpui_kit::*;
use serde::{Deserialize, Serialize};

use crate::config::Config;

/// What a drag and drop of entries does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropAction {
    /// Ask whether to move or copy.
    #[default]
    Ask,
    Move,
    Copy,
}

/// Where file icons come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileIcons {
    /// The desktop's icon theme (Linux); generic where that is not available.
    #[default]
    System,
    Generic,
}

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub drop_action: DropAction,
    pub file_icons: FileIcons,
    /// The status bar under each panel.
    pub panel_status_bar: bool,
    /// Bookmarked directories, so a panel can show a filled star for its own.
    pub favorites: Vec<crate::vfs::Location>,
}

impl Global for Settings {}

pub fn init(config: &Config, cx: &mut App) {
    cx.set_global(Settings {
        drop_action: config.drop_action,
        file_icons: config.file_icons,
        panel_status_bar: config.panel_status_bar,
        favorites: config.favorites.iter().map(|f| f.path.clone()).collect(),
    });
}

pub fn get(cx: &App) -> &Settings {
    cx.global::<Settings>()
}

pub fn set_drop_action(cx: &mut App, action: DropAction) {
    cx.global_mut::<Settings>().drop_action = action;
}

pub fn set_file_icons(cx: &mut App, icons: FileIcons) {
    cx.global_mut::<Settings>().file_icons = icons;
}

pub fn set_panel_status_bar(cx: &mut App, show: bool) {
    cx.global_mut::<Settings>().panel_status_bar = show;
}

pub fn set_favorites(cx: &mut App, favorites: Vec<crate::vfs::Location>) {
    cx.global_mut::<Settings>().favorites = favorites;
}
