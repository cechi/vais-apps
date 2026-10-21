# VAIS Commander

Part of the **VAIS** family of apps (VAIS = *vibecoded AI slop*, worn with pride).

Keyboard-driven file manager in the spirit of Total/Midnight Commander, with one twist:
instead of two fixed panes, the panels sit in a configurable **grid** (1x2, 2x1, 1x3, 3x1,
2x2, 2x3, 3x2, 3x3, 1x1), and the window has **tabs**, each holding its own grid.
Built in Rust on [gpui-kit](https://gpui-kit.com) (GPUI from Zed).

Status: navigation, marking, tabs, layouts, persistence and the basic file operations
(copy, move, rename, new folder, delete to trash) work. F3 view and F4 edit are not
implemented yet.

## Build & run

```sh
cargo run            # binary is target/debug/vais
cargo test           # unit tests (sorting, formatting, layout, marks, config, keymap)
```

Requires Rust 1.92+. Linux needs a Vulkan driver and a Wayland or X11 session;
Arch/CachyOS packages: `clang fontconfig wayland libxkbcommon libxkbcommon-x11 libxcb
libx11 openssl zstd vulkan-icd-loader` plus your GPU's Vulkan ICD.

## Platforms

Linux is the developed and tested platform. macOS and Windows are supported in code
(GPUI runs on both) but untested so far:

- Shortcuts use the platform's primary modifier: `Ctrl` on Linux/Windows, `Cmd` on macOS.
  On macOS panel focus is `Cmd+Alt+arrows`, hidden files `Cmd+Shift+.`, and Finder-style
  `Cmd+Up` / `Cmd+Down` go up / open.
- Hidden files: dot-prefix on Unix, the hidden attribute (or dot-prefix) on Windows.
- Windows drive roots: `..` above `C:\` opens a "Computer" listing of mounted drives.
- Packaging (`scripts/install-desktop.sh`) is Linux-only; a macOS `.app` bundle and a Windows
  shortcut/icon are still to do.
- CI (`.github/workflows/ci.yml`) builds and tests on Ubuntu, macOS and Windows.

## Launcher entry

```sh
scripts/install-desktop.sh        # release build, ~/.local/bin link, .desktop + icon
scripts/install-desktop.sh debug  # same, but linking the debug binary
```

Installs for the current user only (`~/.local/...`). The binary is a symlink into
`target/`, so a rebuild updates what the launcher starts.

## Keys

Keys below are for Linux/Windows; on macOS read `Ctrl` as `Cmd` (see Platforms).

| Keys | Action |
|---|---|
| `Ctrl+T` / `Ctrl+W` | New tab (copy of the current one) / close tab |
| `Ctrl+Tab` / `Ctrl+Shift+Tab`, `Ctrl+PgDn` / `Ctrl+PgUp` | Next / previous tab |
| `Ctrl+1 … Ctrl+9` | Activate tab |
| `Tab` / `Shift+Tab` | Next / previous panel (reading order, wraps) |
| `Ctrl+↑ ↓ ← →` | Move focus to the neighbouring panel |
| `Alt+1 … Alt+9` | Layout 1x2, 2x1, 1x3, 3x1, 2x2, 2x3, 3x2, 3x3, 1x1 |
| `↑ ↓ PgUp PgDn Home End` | Move cursor |
| `Shift` + the above | Move cursor and mark the range |
| `Insert` / `Space` | Toggle mark (Insert also moves down) |
| `Ctrl+A` / `Ctrl+Shift+A` | Mark all / invert marks |
| `Esc` | Cancel path edit, dismiss error, or clear marks |
| `Enter` | Enter directory (`..` goes up) |
| `Backspace` / `Alt+←` | Parent directory / back in history |
| `Ctrl+L` | Edit the path (`~` expands); `Enter` to go, `Esc` to cancel |
| `Ctrl+H` | Toggle hidden files |
| `Ctrl+R` | Reload |
| `F5` / `F6` | Copy / move the marked (or cursor) entries to the other panel; the destination is editable |
| `F2` / `Shift+F6` | Rename the entry under the cursor |
| `F7` | New folder |
| `F8` / `Delete` | Move to trash; `Shift+F8` / `Shift+Delete` deletes permanently |
| `Esc` | Cancel the latest running operation (when there is nothing else to clear) |
| `Ctrl+J` | Show / hide the operations panel |
| `Ctrl+Shift+D` | Toggle light / dark theme |
| `Ctrl+=` / `Ctrl+-` / `Ctrl+0` | UI scale up / down / reset (also the slider in the status bar) |
| `Ctrl+Q` / `F10` | Quit |

Mouse: click selects a row and focuses its panel, double-click opens, dragging the
dividers resizes the grid, tabs are clickable with a close button and a `+`.

File operations act on the marked entries of the active panel (or the entry under the
cursor), target the previously focused panel and ask before overwriting (overwrite or
skip). Each operation runs on its own background thread and gets a row in the
**operations panel** at the bottom of the window (independent of tabs, opens
automatically, resizable): progress bar, bytes and items, current file, a speed
sparkline, elapsed time, status, and a cancel / remove button. The status bar shows the
number of running operations and their combined speed. Enter on a file opens it with the
system's default application.

## Configuration

Saved automatically (debounced and on quit) to `~/.config/vais-commander/config.toml`
(version 2): theme, UI scale, window bounds, the active tab and one `[[tabs]]` entry per tab with
its layout, divider positions as fractions and each panel's path, hidden-files toggle and
sort. A version 1 file (single layout) is migrated into one tab. A missing or invalid file
falls back to defaults with a logged warning (`RUST_LOG=warn`).

## Layout of the code

```
src/main.rs        bootstrap: config, key bindings, theme, window
src/app.rs         Workspace: tabs, tab bar, theme, persistence
src/tabs.rs        Tab + TabContent enum (Grid today; settings etc. later)
src/grid_view.rs   GridView: 9 panels, grid layout, focus between panels
src/layout/        Layout {rows, cols} + grid rendering with nested resizable groups
src/panel/         FilePanel entity (rendering, actions) and PanelState (cursor, marks)
src/fs/            directory reading, natural sort, size/date formatting
src/ops/           copy / move / delete with byte-level progress and cancel; manager.rs runs them
src/ops_panel.rs   the operations panel (rows with progress, speed sparkline, cancel)
src/keymap.rs      actions and default bindings
src/config.rs      TOML config
```
