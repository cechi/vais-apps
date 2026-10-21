//! Actions and the default key bindings (Commander style).

use gpui_kit::*;

use crate::layout::Layout;

pub const WORKSPACE_CONTEXT: &str = "Workspace";
pub const GRID_CONTEXT: &str = "Grid";
pub const PANEL_CONTEXT: &str = "FilePanel";

actions!(
    vais,
    [
        // Workspace: tabs
        NewTab,
        CloseTab,
        NextTab,
        PrevTab,
        ToggleTheme,
        ToggleOperationsPanel,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        Quit,
        // Grid: file operations
        CopyFiles,
        MoveFiles,
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
        EditPath,
        ToggleHidden,
        Reload,
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
        KeyBinding::new("secondary-j", ToggleOperationsPanel, w),
        KeyBinding::new("secondary-=", ZoomIn, w),
        KeyBinding::new("secondary-+", ZoomIn, w),
        KeyBinding::new("secondary--", ZoomOut, w),
        KeyBinding::new("secondary-0", ZoomReset, w),
        KeyBinding::new("secondary-q", Quit, w),
        KeyBinding::new("f10", Quit, w),
        // Grid: file operations (Commander F-keys)
        KeyBinding::new("f5", CopyFiles, g),
        KeyBinding::new("f6", MoveFiles, g),
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
        KeyBinding::new("secondary-l", EditPath, p),
        KeyBinding::new(hidden_key, ToggleHidden, p),
        KeyBinding::new("secondary-r", Reload, p),
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
