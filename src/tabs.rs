//! A workspace tab and what it can show. New kinds (settings, …) go here.

use gpui_kit::*;

use crate::config::TabConfig;
use crate::grid_view::GridView;

pub enum TabContent {
    Grid(Entity<GridView>),
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

    pub fn grid_view(&self) -> Option<&Entity<GridView>> {
        match &self.content {
            TabContent::Grid(view) => Some(view),
        }
    }

    pub fn title(&self, cx: &App) -> String {
        match &self.content {
            TabContent::Grid(view) => view.read(cx).title(cx),
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        match &self.content {
            TabContent::Grid(view) => view.update(cx, |grid, cx| grid.focus_active(window, cx)),
        }
    }

    pub fn config(&self, cx: &App) -> TabConfig {
        match &self.content {
            TabContent::Grid(view) => view.read(cx).config(cx),
        }
    }

    pub fn render(&self) -> AnyElement {
        match &self.content {
            TabContent::Grid(view) => view.clone().into_any_element(),
        }
    }
}
