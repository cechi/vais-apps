//! A workspace tab and what it can show. New kinds (settings, …) go here.

use gpui_kit::*;

use crate::config::TabConfig;
use crate::connections_view::ConnectionsView;
use crate::editor_view::EditorView;
use crate::grid_view::GridView;
use crate::rename::view::RenameView;
use crate::settings_view::SettingsView;

pub enum TabContent {
    Grid(Entity<GridView>),
    /// A batch rename form; transient, not saved in the config.
    Rename(Entity<RenameView>),
    /// The application settings; transient as well.
    Settings(Entity<SettingsView>),
    /// A text file in the editor.
    Editor(Entity<EditorView>),
    /// The connection manager.
    Connections(Entity<ConnectionsView>),
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

    pub fn settings(view: Entity<SettingsView>) -> Self {
        Self {
            content: TabContent::Settings(view),
        }
    }

    pub fn editor(view: Entity<EditorView>) -> Self {
        Self {
            content: TabContent::Editor(view),
        }
    }

    pub fn connections(view: Entity<ConnectionsView>) -> Self {
        Self {
            content: TabContent::Connections(view),
        }
    }

    pub fn connections_view(&self) -> Option<&Entity<ConnectionsView>> {
        match &self.content {
            TabContent::Connections(view) => Some(view),
            _ => None,
        }
    }

    pub fn editor_view(&self) -> Option<&Entity<EditorView>> {
        match &self.content {
            TabContent::Editor(view) => Some(view),
            _ => None,
        }
    }

    pub fn grid_view(&self) -> Option<&Entity<GridView>> {
        match &self.content {
            TabContent::Grid(view) => Some(view),
            _ => None,
        }
    }

    pub fn rename_view(&self) -> Option<&Entity<RenameView>> {
        match &self.content {
            TabContent::Rename(view) => Some(view),
            _ => None,
        }
    }

    pub fn settings_view(&self) -> Option<&Entity<SettingsView>> {
        match &self.content {
            TabContent::Settings(view) => Some(view),
            _ => None,
        }
    }

    /// Where actions from the tab bar menus are dispatched: the active
    /// panel of a grid, or the form itself.
    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.content {
            TabContent::Grid(view) => view.read(cx).active_panel().focus_handle(cx),
            TabContent::Rename(view) => view.read(cx).focus_handle(),
            TabContent::Settings(view) => view.read(cx).focus_handle(),
            TabContent::Editor(view) => view.read(cx).focus_handle(),
            TabContent::Connections(view) => view.read(cx).focus_handle(),
        }
    }

    pub fn title(&self, cx: &App) -> String {
        match &self.content {
            TabContent::Grid(view) => view.read(cx).title(cx),
            TabContent::Rename(view) => view.read(cx).title(),
            TabContent::Settings(view) => view.read(cx).title(),
            TabContent::Editor(view) => view.read(cx).title(),
            TabContent::Connections(_) => "Connections".to_string(),
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        match &self.content {
            TabContent::Grid(view) => view.update(cx, |grid, cx| grid.focus_active(window, cx)),
            TabContent::Rename(view) => view.update(cx, |form, cx| form.focus(window, cx)),
            TabContent::Settings(view) => view.read(cx).focus_handle().focus(window, cx),
            TabContent::Editor(view) => view.update(cx, |editor, cx| editor.focus(window, cx)),
            TabContent::Connections(view) => view.read(cx).focus_handle().focus(window, cx),
        }
    }

    /// What to save; `None` for tabs that are not persisted.
    pub fn config(&self, cx: &App) -> Option<TabConfig> {
        match &self.content {
            TabContent::Grid(view) => Some(view.read(cx).config(cx)),
            _ => None,
        }
    }

    pub fn render(&self) -> AnyElement {
        match &self.content {
            TabContent::Grid(view) => view.clone().into_any_element(),
            TabContent::Rename(view) => view.clone().into_any_element(),
            TabContent::Settings(view) => view.clone().into_any_element(),
            TabContent::Editor(view) => view.clone().into_any_element(),
            TabContent::Connections(view) => view.clone().into_any_element(),
        }
    }
}
