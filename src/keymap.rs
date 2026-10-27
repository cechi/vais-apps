//! Actions and the default key bindings (Commander style).

use gpui_kit::*;

use crate::fs::SortKey;
use crate::layout::Layout;
use crate::panel::view::{Column, ViewMode};

pub const WORKSPACE_CONTEXT: &str = "Workspace";
pub const GRID_CONTEXT: &str = "Grid";
pub const PANEL_CONTEXT: &str = "FilePanel";
pub const PANEL_FILTER_CONTEXT: &str = "PanelFilter";
pub const COMMAND_LINE_CONTEXT: &str = "CommandLine";
/// The embedded terminal in the Agents panel. Workspace bindings a terminal
/// program would want for itself (Ctrl+T, F10, …) are bound with
/// [`WORKSPACE_NOT_TERMINAL`] so they never fire while it has the focus.
#[allow(dead_code)] // the terminal view (next step) sets it
pub const TERMINAL_CONTEXT: &str = "Terminal";
pub const WORKSPACE_NOT_TERMINAL: &str = "Workspace && !Terminal";
pub const PALETTE_CONTEXT: &str = "Palette";
pub const RENAME_CONTEXT: &str = "RenameForm";
pub const SETTINGS_CONTEXT: &str = "SettingsForm";
pub const EDITOR_CONTEXT: &str = "FileEditor";
pub const CONNECTIONS_CONTEXT: &str = "ConnectionsForm";
pub const CHANGELOG_CONTEXT: &str = "Changelog";
/// The portal picker window around its file panel.
pub const PICKER_CONTEXT: &str = "Picker";

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
        CommandPalette,
        ToggleAgents,
        FocusAgents,
        NewAgentSession,
        ToggleFavorite,
        FocusCommandLine,
        /// Show or hide the command line (the ☰ menu; no default key).
        ToggleCommandLine,
        /// Show or hide the function key bar (the ☰ menu; no default key).
        ToggleKeyBar,
        /// Show or hide the status bars under the panels (the ☰ menu; no default key).
        TogglePanelStatusBar,
        InsertNameToCommandLine,
        InsertPathToCommandLine,
        CommandHistoryPrev,
        CommandHistoryNext,
        CommandLineCancel,
        CloseSettings,
        /// Open the What's new tab (the ☰ menu; no default key).
        ShowChangelog,
        CloseChangelog,
        // Files in the editor tab
        ViewFile,
        EditFile,
        SaveFile,
        ToggleReadOnly,
        // Connections
        OpenConnections,
        CloseConnections,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        Quit,
        // Grid: file operations
        CopyFiles,
        MoveFiles,
        /// The entry menu of an archive: unpack it into a panel's directory.
        ExtractArchive,
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
        MarkByPattern,
        UnmarkByPattern,
        CancelOrClear,
        // Panel: navigation
        Open,
        OpenWithApp,
        GoUp,
        GoBack,
        GoForward,
        EditPath,
        ToggleHidden,
        Reload,
        CopyPath,
        OpenTerminal,
        /// Windows: the Explorer context menu of the selection.
        WindowsContextMenu,
        /// Windows: the Explorer context menu of the panel's directory.
        WindowsFolderMenu,
        ExpandEntry,
        CollapseEntry,
        OpenFilter,
        CloseFilter,
        ToggleFilterRecursive,
        ToggleFilterContent,
        // Palette (the command-center style chooser)
        PaletteUp,
        PaletteDown,
        PaletteConfirm,
        PaletteCancel,
        // Batch rename form
        ApplyRename,
        CancelRename,
        // Portal picker
        PickerAccept,
        PickerCancel,
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
    // Workspace bindings the terminal must be able to swallow.
    let wt = Some(WORKSPACE_NOT_TERMINAL);
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
        KeyBinding::new("secondary-t", NewTab, wt),
        KeyBinding::new("secondary-w", CloseTab, wt),
        KeyBinding::new("ctrl-tab", NextTab, wt),
        KeyBinding::new("ctrl-shift-tab", PrevTab, wt),
        KeyBinding::new("ctrl-pagedown", NextTab, wt),
        KeyBinding::new("ctrl-pageup", PrevTab, wt),
        KeyBinding::new("secondary-shift-d", ToggleTheme, w),
        KeyBinding::new("secondary-shift-t", ChooseTheme, w),
        KeyBinding::new("secondary-j", ToggleOperationsPanel, wt),
        KeyBinding::new("secondary-b", ToggleSidebar, wt),
        KeyBinding::new("secondary-,", OpenSettings, wt),
        KeyBinding::new("secondary-shift-p", CommandPalette, w),
        KeyBinding::new("secondary-shift-b", ToggleAgents, w),
        KeyBinding::new("secondary-`", FocusAgents, w),
        KeyBinding::new("secondary-shift-n", NewAgentSession, w),
        KeyBinding::new("secondary-d", ToggleFavorite, wt),
        KeyBinding::new("secondary-e", FocusCommandLine, wt),
        KeyBinding::new("secondary-enter", InsertNameToCommandLine, p),
        KeyBinding::new("secondary-shift-enter", InsertPathToCommandLine, p),
        KeyBinding::new("up", CommandHistoryPrev, Some(COMMAND_LINE_CONTEXT)),
        KeyBinding::new("down", CommandHistoryNext, Some(COMMAND_LINE_CONTEXT)),
        KeyBinding::new("escape", CommandLineCancel, Some(COMMAND_LINE_CONTEXT)),
        KeyBinding::new("secondary-k", OpenConnections, wt),
        KeyBinding::new("escape", CloseConnections, Some(CONNECTIONS_CONTEXT)),
        KeyBinding::new("escape", CloseSettings, Some(SETTINGS_CONTEXT)),
        KeyBinding::new("escape", CloseChangelog, Some(CHANGELOG_CONTEXT)),
        KeyBinding::new("secondary-s", SaveFile, Some(EDITOR_CONTEXT)),
        KeyBinding::new("secondary-shift-e", ToggleReadOnly, Some(EDITOR_CONTEXT)),
        KeyBinding::new("secondary-=", ZoomIn, w),
        KeyBinding::new("secondary-+", ZoomIn, w),
        KeyBinding::new("secondary--", ZoomOut, w),
        KeyBinding::new("secondary-0", ZoomReset, w),
        KeyBinding::new("secondary-q", Quit, wt),
        KeyBinding::new("f10", Quit, wt),
        // Grid: file operations (Commander F-keys)
        KeyBinding::new("f3", ViewFile, g),
        KeyBinding::new("f4", EditFile, g),
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
        KeyBinding::new("secondary-m", MarkByPattern, p),
        KeyBinding::new("secondary-u", UnmarkByPattern, p),
        KeyBinding::new("escape", CancelOrClear, p),
        // Panel: navigation
        KeyBinding::new("enter", Open, p),
        KeyBinding::new("shift-enter", OpenWithApp, p),
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
        KeyBinding::new("secondary-shift-g", ToggleFilterContent, p),
        KeyBinding::new("escape", CloseFilter, Some(PANEL_FILTER_CONTEXT)),
        // Palette: the single-line input lets Up/Down through; Enter and Esc
        // are handled here before they reach the hosting dialog.
        KeyBinding::new("up", PaletteUp, Some(PALETTE_CONTEXT)),
        KeyBinding::new("down", PaletteDown, Some(PALETTE_CONTEXT)),
        KeyBinding::new("enter", PaletteConfirm, Some(PALETTE_CONTEXT)),
        KeyBinding::new("escape", PaletteCancel, Some(PALETTE_CONTEXT)),
        // Portal picker: Enter and Escape inside the panel are the panel's own
        // (Open / CancelOrClear, reported to the picker); these cover the rest.
        KeyBinding::new("ctrl-enter", PickerAccept, Some(PICKER_CONTEXT)),
        KeyBinding::new("escape", PickerCancel, Some(PICKER_CONTEXT)),
        KeyBinding::new("secondary-enter", ApplyRename, Some(RENAME_CONTEXT)),
        KeyBinding::new("escape", CancelRename, Some(RENAME_CONTEXT)),
        KeyBinding::new("left", CollapseEntry, p),
    ];
    if cfg!(windows) {
        // As in Explorer, Shift+F10 is the full shell menu.
        v.push(KeyBinding::new("shift-f10", WindowsContextMenu, p));
    }
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
            wt,
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

/// One row of the command palette: an action usable from a file panel and
/// its key binding.
pub struct Command {
    pub label: String,
    pub keys: String,
    pub action: Box<dyn Action>,
}

/// The actions the command palette lists: every binding that applies while
/// a panel has the focus, first binding of each action wins.
pub fn commands() -> Vec<Command> {
    let mut out: Vec<Command> = Vec::new();
    for binding in panel_bindings() {
        let action = binding.action();
        let label = command_label(action);
        if out.iter().any(|c| c.label == label) {
            continue;
        }
        let keys = binding
            .keystrokes()
            .iter()
            .map(|k| pretty_keys(&k.to_string()))
            .collect::<Vec<_>>()
            .join(" ");
        out.push(Command {
            label,
            keys,
            action: action.boxed_clone(),
        });
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

/// The bindings usable from a file panel.
fn panel_bindings() -> Vec<KeyBinding> {
    let contexts: Vec<KeyContext> = [WORKSPACE_CONTEXT, GRID_CONTEXT, PANEL_CONTEXT]
        .iter()
        .filter_map(|c| KeyContext::parse(c).ok())
        .collect();
    bindings()
        .into_iter()
        .filter(|binding| {
            // As in dispatch: the predicate holds at some depth of the focus path.
            binding
                .predicate()
                .is_none_or(|predicate| predicate.depth_of(&contexts).is_some())
        })
        .collect()
}

/// The English texts of the [`commands`] labels, the catalog keys they
/// translate by: action names and the named data.
#[cfg(test)]
pub fn command_labels() -> Vec<String> {
    let mut labels = Vec::new();
    for binding in panel_bindings() {
        labels.push(command_name(binding.action()));
        if let Some((detail, true)) = action_detail(binding.action()) {
            labels.push(detail);
        }
    }
    // Bound on some platforms only, translated in every catalog all the same.
    labels.push(command_name(&WindowsContextMenu));
    labels.sort();
    labels.dedup();
    labels
}

/// `vais::CopyFiles` → `Copy Files`; an action with data adds it:
/// `SortBy { key: Name }` → `Sort By: Name`; in the interface language.
fn command_label(action: &dyn Action) -> String {
    let name = crate::i18n::tr_str(&command_name(action)).to_string();
    match action_detail(action) {
        Some((detail, true)) => format!("{name}: {}", crate::i18n::tr_str(&detail)),
        Some((detail, false)) => format!("{name}: {detail}"),
        None => name,
    }
}

/// `vais::CopyFiles` → `Copy Files`.
fn command_name(action: &dyn Action) -> String {
    let name = action.name().rsplit("::").next().unwrap_or("");
    let mut label = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            label.push(' ');
        }
        label.push(c);
    }
    label
}

/// The data of the actions that carry some, for the palette label, and
/// whether it is a name to translate (numbers are not).
fn action_detail(action: &dyn Action) -> Option<(String, bool)> {
    let any = action.as_any();
    if let Some(a) = any.downcast_ref::<SortBy>() {
        Some((format!("{:?}", a.key), true))
    } else if let Some(a) = any.downcast_ref::<SetView>() {
        Some((format!("{:?}", a.mode), true))
    } else if let Some(a) = any.downcast_ref::<ToggleColumn>() {
        Some((format!("{:?}", a.column), true))
    } else if let Some(a) = any.downcast_ref::<ActivateTab>() {
        Some(((a.index + 1).to_string(), false))
    } else {
        any.downcast_ref::<SetLayout>()
            .map(|a| (format!("{}x{}", a.rows, a.cols), false))
    }
}

/// `ctrl-shift-f3` → `Ctrl+Shift+F3`.
fn pretty_keys(keystroke: &str) -> String {
    let (mods, key) = match keystroke.rsplit_once('-') {
        Some((m, "")) => (m.trim_end_matches('-'), "-"),
        Some((m, k)) => (m, k),
        None => ("", keystroke),
    };
    let mut parts: Vec<String> = mods
        .split('-')
        .filter(|m| !m.is_empty())
        .map(|m| match m {
            "ctrl" => "Ctrl".to_string(),
            "alt" => "Alt".to_string(),
            "shift" => "Shift".to_string(),
            "cmd" => "Cmd".to_string(),
            "fn" => "Fn".to_string(),
            other => other.to_string(),
        })
        .collect();
    let key = match key {
        "pageup" => "PgUp".to_string(),
        "pagedown" => "PgDn".to_string(),
        "escape" => "Esc".to_string(),
        "enter" => "Enter".to_string(),
        "backspace" => "Backspace".to_string(),
        "delete" => "Del".to_string(),
        "insert" => "Ins".to_string(),
        "space" => "Space".to_string(),
        "tab" => "Tab".to_string(),
        "home" => "Home".to_string(),
        "end" => "End".to_string(),
        "up" => "↑".to_string(),
        "down" => "↓".to_string(),
        "left" => "←".to_string(),
        "right" => "→".to_string(),
        other => other.to_uppercase(),
    };
    parts.push(key);
    parts.join("+")
}

#[cfg(test)]
mod palette_tests {
    use super::{commands, pretty_keys};
    use gpui_kit::KeyContext;

    #[test]
    fn keys_read_like_a_manual() {
        assert_eq!(pretty_keys("ctrl-shift-f3"), "Ctrl+Shift+F3");
        assert_eq!(pretty_keys("ctrl--"), "Ctrl+-");
        assert_eq!(pretty_keys("enter"), "Enter");
        assert_eq!(pretty_keys("alt-left"), "Alt+←");
    }

    /// With the terminal focused, the bindings a terminal program needs
    /// (Ctrl+T, Ctrl+W, F10, …) must not match; global ones still do.
    #[test]
    fn terminal_swallows_workspace_keys() {
        let stack: Vec<KeyContext> = [super::WORKSPACE_CONTEXT, super::TERMINAL_CONTEXT]
            .iter()
            .map(|c| KeyContext::parse(c).unwrap())
            .collect();
        let matches = |name: &str| {
            super::bindings().iter().any(|b| {
                b.action().name().ends_with(name)
                    && b.predicate().is_none_or(|p| p.depth_of(&stack).is_some())
            })
        };
        for swallowed in [
            "::NewTab",
            "::CloseTab",
            "::Quit",
            "::ToggleSidebar",
            "::ActivateTab",
        ] {
            assert!(
                !matches(swallowed),
                "{swallowed} must not reach the workspace"
            );
        }
        for global in [
            "::CommandPalette",
            "::ToggleAgents",
            "::FocusAgents",
            "::ZoomIn",
        ] {
            assert!(matches(global), "{global} must still work");
        }
    }

    #[test]
    fn palette_lists_panel_actions_once() {
        let commands = commands();
        let labels: Vec<&str> = commands.iter().map(|c| c.label.as_str()).collect();
        assert!(labels.contains(&"Copy Files"));
        assert!(labels.contains(&"Sort By: Name"));
        assert!(!labels.contains(&"Palette Confirm"));
        assert_eq!(labels.iter().filter(|l| **l == "Open").count(), 1);
    }
}
