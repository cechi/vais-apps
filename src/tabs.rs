//! A workspace tab and what it can show. New kinds (settings, …) go here.

use gpui_kit::*;

use crate::config::TabConfig;
use crate::grid_view::GridView;
use crate::rename::view::RenameView;

pub enum TabContent {
    Grid(Entity<GridView>),
    /// A batch rename form; transient, not saved in the config.
    Rename(Entity<RenameView>),
}

pub struct Tab {
    pub content: TabContent,
}

impl Tab {
    pub fn grid(view: Entity<GridView>) -> Self {
        Self {
            content: TabContent::Grid(view),
        }
    }

    pub fn rename(view: Entity<RenameView>) -> Self {
        Self {
            content: TabContent::Rename(view),
        }
    }

    pub fn grid_view(&self) -> Option<&Entity<GridView>> {
        match &self.content {
            TabContent::Grid(view) => Some(view),
            TabContent::Rename(_) => None,
        }
    }

    pub fn rename_view(&self) -> Option<&Entity<RenameView>> {
        match &self.content {
            TabContent::Rename(view) => Some(view),
            TabContent::Grid(_) => None,
        }
    }

    pub fn title(&self, cx: &App) -> String {
        match &self.content {
            TabContent::Grid(view) => view.read(cx).title(cx),
            TabContent::Rename(view) => view.read(cx).title(),
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        match &self.content {
            TabContent::Grid(view) => view.update(cx, |grid, cx| grid.focus_active(window, cx)),
            TabContent::Rename(view) => view.update(cx, |form, cx| form.focus(window, cx)),
        }
    }

    /// What to save; `None` for tabs that are not persisted.
    pub fn config(&self, cx: &App) -> Option<TabConfig> {
        match &self.content {
            TabContent::Grid(view) => Some(view.read(cx).config(cx)),
            TabContent::Rename(_) => None,
        }
    }

    pub fn render(&self) -> AnyElement {
        match &self.content {
            TabContent::Grid(view) => view.clone().into_any_element(),
            TabContent::Rename(view) => view.clone().into_any_element(),
        }
    }
}
