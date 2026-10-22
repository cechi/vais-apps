//! Actions and the default key bindings (Commander style).

use gpui_kit::*;

use crate::fs::SortKey;
use crate::layout::Layout;
use crate::panel::view::{Column, ViewMode};

pub const WORKSPACE_CONTEXT: &str = "Workspace";
pub const GRID_CONTEXT: &str = "Grid";
pub const PANEL_CONTEXT: &str = "FilePanel";
pub const PANEL_FILTER_CONTEXT: &str = "PanelFilter";
pub const PALETTE_CONTEXT: &str = "Palette";
pub const RENAME_CONTEXT: &str = "RenameForm";
pub const SETTINGS_CONTEXT: &str = "SettingsForm";

actions!(
    vais,
    [
        // Workspace: tabs
        NewTab,
        CloseTab,
        NextTab,
        PrevTab,
        ToggleTheme,
        ChooseTheme,
        ToggleOperationsPanel,
        ToggleSidebar,
        OpenSettings,
        CloseSettings,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        Quit,
        // Grid: file operations
        CopyFiles,
        MoveFiles,
        CopyToClipboard,
        CutToClipboard,
        PasteFromClipboard,
        RenameFile,
        MakeDir,
        DeleteFiles,
        DeleteFilesPermanently,
        // Grid
        FocusNextPanel,
        FocusPrevPanel,
        FocusUp,
        FocusDown,
        FocusLeft,
        FocusRight,
        // Panel: cursor
        CursorUp,
        CursorDown,
        CursorPageUp,
        CursorPageDown,
        CursorHome,
        CursorEnd,
        // Panel: cursor + extend marks
        ExtendUp,
        ExtendDown,
        ExtendPageUp,
        ExtendPageDown,
        ExtendHome,
        ExtendEnd,
        // Panel: marks
        ToggleMark,
        ToggleMarkAndDown,
        MarkAll,
        InvertMarks,
        CancelOrClear,
        // Panel: navigation
        Open,
        GoUp,
        GoBack,
        GoForward,
        EditPath,
        ToggleHidden,
        Reload,
        CopyPath,
        OpenTerminal,
        ExpandEntry,
        CollapseEntry,
        OpenFilter,
        CloseFilter,
        ToggleFilterRecursive,
        // Palette (the command-center style chooser)
        PaletteUp,
        PaletteDown,
        PaletteConfirm,
        PaletteCancel,
        // Batch rename form
        ApplyRename,
        CancelRename,
    ]
);

/// Activate the tab at `index` (0-based).
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = vais, no_json)]
pub struct ActivateTab {
    pub index: usize,
}

/// Switch the grid to `rows` x `cols`.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = vais, no_json)]
pub struct SetLayout {
    pub rows: u8,
    pub cols: u8,
}

/// Sort the panel by `key`; the same key again flips the direction.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = vais, no_json)]
pub struct SortBy {
    pub key: SortKey,
}

/// Switch the panel's view (full list, brief, thumbnails).
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = vais, no_json)]
pub struct SetView {
    pub mode: ViewMode,
}

/// Show or hide a column of the full view.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = vais, no_json)]
pub struct ToggleColumn {
    pub column: Column,
}

/// Default key bindings.
///
/// `secondary` is Cmd on macOS and Ctrl elsewhere, so the application-level
/// shortcuts follow each platform's convention. A few keys differ on macOS:
/// Cmd+H hides the app and Ctrl+arrows switch spaces, so hidden files use
/// Finder's Cmd+Shift+. and panel focus uses Cmd+Alt+arrows there.
pub fn bindings() -> Vec<KeyBinding> {
    let w = Some(WORKSPACE_CONTEXT);
    let g = Some(GRID_CONTEXT);
    let p = Some(PANEL_CONTEXT);
    let mac = cfg!(target_os = "macos");
    let focus_mod = if mac { "secondary-alt" } else { "ctrl" };
    let hidden_key = if mac {
        "secondary-shift-."
    } else {
        "secondary-h"
    };

    let mut v = vec![
        // Workspace: tabs, theme, zoom, quit
        KeyBinding::new("secondary-t", NewTab, w),
        KeyBinding::new("secondary-w", CloseTab, w),
        KeyBinding::new("ctrl-tab", NextTab, w),
        KeyBinding::new("ctrl-shift-tab", PrevTab, w),
        KeyBinding::new("ctrl-pagedown", NextTab, w),
        KeyBinding::new("ctrl-pageup", PrevTab, w),
        KeyBinding::new("secondary-shift-d", ToggleTheme, w),
        KeyBinding::new("secondary-shift-t", ChooseTheme, w),
        KeyBinding::new("secondary-j", ToggleOperationsPanel, w),
        KeyBinding::new("secondary-b", ToggleSidebar, w),
        KeyBinding::new("secondary-,", OpenSettings, w),
        KeyBinding::new("escape", CloseSettings, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("secondary-=", ZoomIn, w),
        KeyBinding::new("secondary-+", ZoomIn, w),
        KeyBinding::new("secondary--", ZoomOut, w),
        KeyBinding::new("secondary-0", ZoomReset, w),
        KeyBinding::new("secondary-q", Quit, w),
        KeyBinding::new("f10", Quit, w),
        // Grid: file operations (Commander F-keys)
        KeyBinding::new("f5", CopyFiles, g),
        KeyBinding::new("f6", MoveFiles, g),
        KeyBinding::new("secondary-c", CopyToClipboard, g),
        KeyBinding::new("secondary-x", CutToClipboard, g),
        KeyBinding::new("secondary-v", PasteFromClipboard, g),
        KeyBinding::new("shift-f6", RenameFile, g),
        KeyBinding::new("f2", RenameFile, g),
        KeyBinding::new("f7", MakeDir, g),
        KeyBinding::new("f8", DeleteFiles, g),
        KeyBinding::new("delete", DeleteFiles, g),
        KeyBinding::new("shift-f8", DeleteFilesPermanently, g),
        KeyBinding::new("shift-delete", DeleteFilesPermanently, g),
        // Grid: focus between panels
        KeyBinding::new("tab", FocusNextPanel, g),
        KeyBinding::new("shift-tab", FocusPrevPanel, g),
        KeyBinding::new(&format!("{focus_mod}-up"), FocusUp, g),
        KeyBinding::new(&format!("{focus_mod}-down"), FocusDown, g),
        KeyBinding::new(&format!("{focus_mod}-left"), FocusLeft, g),
        KeyBinding::new(&format!("{focus_mod}-right"), FocusRight, g),
        // Panel: cursor
        KeyBinding::new("up", CursorUp, p),
        KeyBinding::new("down", CursorDown, p),
        KeyBinding::new("pageup", CursorPageUp, p),
        KeyBinding::new("pagedown", CursorPageDown, p),
        KeyBinding::new("home", CursorHome, p),
        KeyBinding::new("end", CursorEnd, p),
        KeyBinding::new("shift-up", ExtendUp, p),
        KeyBinding::new("shift-down", ExtendDown, p),
        KeyBinding::new("shift-pageup", ExtendPageUp, p),
        KeyBinding::new("shift-pagedown", ExtendPageDown, p),
        KeyBinding::new("shift-home", ExtendHome, p),
        KeyBinding::new("shift-end", ExtendEnd, p),
        // Panel: marks
        KeyBinding::new("space", ToggleMark, p),
        KeyBinding::new("insert", ToggleMarkAndDown, p),
        KeyBinding::new("secondary-a", MarkAll, p),
        KeyBinding::new("secondary-shift-a", InvertMarks, p),
        KeyBinding::new("escape", CancelOrClear, p),
        // Panel: navigation
        KeyBinding::new("enter", Open, p),
        KeyBinding::new("backspace", GoUp, p),
        KeyBinding::new("alt-left", GoBack, p),
        KeyBinding::new("alt-right", GoForward, p),
        KeyBinding::new("secondary-l", EditPath, p),
        KeyBinding::new(hidden_key, ToggleHidden, p),
        KeyBinding::new("secondary-r", Reload, p),
        // Views: Ctrl+F1 brief, Ctrl+F2 full (Commander style), Ctrl+F3 thumbnails.
        KeyBinding::new(
            "ctrl-f1",
            SetView {
                mode: ViewMode::Brief,
            },
            p,
        ),
        KeyBinding::new(
            "ctrl-f2",
            SetView {
                mode: ViewMode::Full,
            },
            p,
        ),
        KeyBinding::new(
            "ctrl-f3",
            SetView {
                mode: ViewMode::Thumbnails,
            },
            p,
        ),
        // Sorting: Ctrl+Shift+F3 name, F4 type, F5 time, F6 size.
        KeyBinding::new("ctrl-shift-f3", SortBy { key: SortKey::Name }, p),
        KeyBinding::new("ctrl-shift-f4", SortBy { key: SortKey::Type }, p),
        KeyBinding::new(
            "ctrl-shift-f5",
            SortBy {
                key: SortKey::Modified,
            },
            p,
        ),
        KeyBinding::new("ctrl-shift-f6", SortBy { key: SortKey::Size }, p),
        KeyBinding::new("secondary-shift-c", CopyPath, p),
        KeyBinding::new("f9", OpenTerminal, p),
        KeyBinding::new("right", ExpandEntry, p),
        KeyBinding::new("secondary-f", OpenFilter, p),
        KeyBinding::new("secondary-shift-f", ToggleFilterRecursive, p),
        KeyBinding::new("escape", CloseFilter, Some(PANEL_FILTER_CONTEXT)),
        // Palette: the single-line input lets Up/Down through; Enter and Esc
        // are handled here before they reach the hosting dialog.
        KeyBinding::new("up", PaletteUp, Some(PALETTE_CONTEXT)),
        KeyBinding::new("down", PaletteDown, Some(PALETTE_CONTEXT)),
        KeyBinding::new("enter", PaletteConfirm, Some(PALETTE_CONTEXT)),
        KeyBinding::new("escape", PaletteCancel, Some(PALETTE_CONTEXT)),
        KeyBinding::new("secondary-enter", ApplyRename, Some(RENAME_CONTEXT)),
        KeyBinding::new("escape", CancelRename, Some(RENAME_CONTEXT)),
        KeyBinding::new("left", CollapseEntry, p),
    ];
    if mac {
        // Finder habits: Cmd+Up goes to the parent, Cmd+Down opens.
        v.push(KeyBinding::new("secondary-up", GoUp, p));
        v.push(KeyBinding::new("secondary-down", Open, p));
    }
    for (i, layout) in Layout::PRESETS.iter().enumerate() {
        v.push(KeyBinding::new(
            &format!("alt-{}", i + 1),
            SetLayout {
                rows: layout.rows,
                cols: layout.cols,
            },
            g,
        ));
    }
    for i in 0..9 {
        v.push(KeyBinding::new(
            &format!("secondary-{}", i + 1),
            ActivateTab { index: i },
            w,
        ));
    }
    v
}

#[cfg(test)]
mod tests {
    /// `KeyBinding::new` panics on an unparsable keystroke, so constructing
    /// the whole table is the test.
    #[test]
    fn all_bindings_parse() {
        assert!(super::bindings().len() > 30);
    }
}
