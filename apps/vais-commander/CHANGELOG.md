# Changelog

What changed in each release of VAIS Commander, newest first.

## 0.4.3 - 2026-10-29

### Added

- The columns of the full view can be resized by dragging the edges between
  their headers. After the first drag the list keeps its width and scrolls
  sideways when it is wider than the panel; a double click on an edge, or
  *Reset column widths* in the header's context menu, goes back to the
  default widths.

### Changed

- A narrow full view (a preview beside it) scrolls sideways instead of
  cutting off the columns on the right.

### Fixed

- Agents started from the Agents panel get the VAIS MCP server again after
  the program file was replaced while the app ran (an update or a rebuild);
  they were launched without it.
- Windows: clicking the drive letter in the path bar goes to the root of the
  drive; it went to the drive's current directory instead.

## 0.4.2 - 2026-10-27

### Added

- What's new: this changelog opens in a tab on the first start of a new
  version, and from the ☰ menu or Settings → About at any time. It is also in
  the release archives as `CHANGELOG.md`.
- The status bars under the panels can be turned off for all panels
  (Settings, or the ☰ menu).
- Preview of the entry under the cursor at the bottom or the right of a
  panel, with a resizable divider (the view button's menu, or Ctrl+Shift+Q):
  images, the start of text files with syntax highlighting, and an
  information card for directories and other files.

### Changed

- In a narrow full view the names keep their width; the columns on the right
  are cut off instead.
- Cechi Dark: the editor has a darker background than the panels. The
  editor colours of the Cechi themes now take effect at all (they were
  ignored, leaving the editor a light grey).

## 0.4.1 - 2026-10-27

### Added

- The interface in 12 languages: English, Czech, German, Spanish, French,
  Italian, Japanese, Polish, Portuguese (Brazil), Slovak, Ukrainian and
  Chinese (Simplified). The system language is used by default; Settings →
  Language changes it on the fly. Translations other than Czech were made by
  AI and have not been reviewed yet.
- Windows: *Windows context menu* in the panel menus (or Shift+F10) opens
  Explorer's own context menu for the selected entries or the folder.

## 0.4.0 - 2026-10-26

### Added

- System file picker on Linux: VAIS Commander can serve as the
  xdg-desktop-portal file chooser (`--portal`), so Firefox, GTK, Qt, Electron
  and Flatpak apps open and save files through it. `install-portal.sh` sets it
  up.
- Archives: *Extract…* in the entry menu, and 7z archives can be browsed.
- Favorites can be reordered.
- Settings: the function key bar can be hidden.

### Fixed

- The settings tab scrolls when it does not fit the window.

## 0.3.0 - 2026-10-23

### Added

- Agents panel at the right edge (Ctrl+Shift+B) with an embedded terminal;
  it detects installed AI coding CLIs and launches them with VAIS Commander's
  MCP server registered for the session.
- Bridge and MCP server: agents can read and drive the app (panels, paths,
  selection, file operations) through a local socket; operations that change
  files go through the app's own confirmations.
- Favorites can be renamed and coloured from the sidebar's context menu.
- Settings: the system places in Favorites can be turned off.

### Changed

- The cursor row is tinted with its outline colour, and the active panel's
  border uses the same colour. Panels keep the active look while the Agents
  panel has the focus.

### Fixed

- Windows: no console window opens next to the GUI.
- The side panel dividers stay under the pointer after a panel is hidden.

## 0.2.0 - 2026-10-23

### Added

- Remote connections: SFTP (with hosts from `~/.ssh/config`) and FTP, a
  connection manager (Ctrl+K) and a Connections group in the sidebar. Remote
  files open in the editor, get thumbnails and open in their default
  application.
- Archives as directories: zip and tar.* are browsed read-only.
- File editor tab: F3 views, F4 edits.
- Panel views: brief and thumbnail grids (with image previews made in the
  background) and configurable columns; a Type column and sorting by column.
- Command line under the panels (Ctrl+E) and quick search by typing.
- Command palette (Ctrl+Shift+P).
- Favorites of two kinds: directories (Ctrl+D, or the star by the path) and
  whole tab layouts (tab menu). Tabs can be renamed and coloured.
- Batch rename in its own tab, and a settings tab (Ctrl+,) behind the ☰ app
  menu.
- File clipboard: Ctrl+C / Ctrl+X / Ctrl+V; Ctrl+V also saves an image from
  the system clipboard.
- Directory sizes on Space, free space in the footer, mark by pattern.
- Content search in the filter bar (Ctrl+Shift+G).
- Panels refresh by themselves when files change on disk.
- File icons from the desktop icon theme (and from the shell on Windows).
- Sidebar: a Recent group with the last 10 directories, groups that fold on a
  click of their header, and a scrollbar.
- Bundled Cechi Dark and Cechi Light themes, and a font setting.
- Ctrl+mouse wheel changes the UI scale.

### Changed

- All dialogs, including copy / move destinations and rename, use the
  palette.

## 0.1.0 - 2026-10-22

The first release.

- A grid of file panels (1×2 up to 3×3) with resizable dividers, in tabs that
  each keep their own grid and paths.
- Commander keys: navigation, marking, hidden files, natural sort, and the
  function key bar.
- Copy, move, rename, new folder, trash and permanent delete, with progress
  in an operations panel.
- Breadcrumb path bar, back / forward history, a filter bar with glob
  patterns and recursive search.
- Context menus, inline tree expansion of directories, a places sidebar
  (Ctrl+B), mouse marking, rubber band selection and drag and drop.
- Light and dark themes, user theme files, UI scale.
- Everything is saved to a TOML config: layout, window, tabs, scale.
- Release builds for Linux, macOS and Windows.
