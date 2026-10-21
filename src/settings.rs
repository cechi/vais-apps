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

#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub drop_action: DropAction,
}

impl Global for Settings {}

pub fn init(config: &Config, cx: &mut App) {
    cx.set_global(Settings {
        drop_action: config.drop_action,
    });
}

pub fn get(cx: &App) -> &Settings {
    cx.global::<Settings>()
}
