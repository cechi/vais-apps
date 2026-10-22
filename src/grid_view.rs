//! One grid of file panels: layout, focus between panels, divider sizes, and
//! the file operations between the active and the target panel.

use std::path::PathBuf;

use crate::vfs::{Location, backend_for};
use std::rc::Rc;

use gpui_kit::component::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::clipboard;
use crate::config::{LayoutConfig, TabConfig, TabKind};
use crate::keymap::*;
use crate::layout::grid::{GridState, OnResize, render_grid};
use crate::layout::{Dir, Layout, MAX_PANELS};
use crate::ops::manager::{ManagerEvent, OperationsManager};
use crate::ops::{self, OpKind, Request};
use crate::palette::{self, Palette, PaletteItem};
use crate::panel::{FilePanel, PanelEvent};
use crate::settings::{self, DropAction};

/// The Move / Copy menu shown at the mouse after a drop.
struct DropMenu {
    menu: Entity<PopupMenu>,
    position: Point<Pixels>,
    _subscription: Subscription,
}

pub enum GridEvent {
    /// Something worth persisting (or showing in the tab title) changed.
    Changed,
    /// The file clipboard changed (the status bar shows it).
    ClipboardChanged,
    /// F2 with several entries marked: open the batch rename form for them.
    BatchRename(Vec<Location>),
    /// F3 / F4: open the file under the cursor in an editor tab.
    OpenFile { path: PathBuf, readonly: bool },
}

pub struct GridView {
    panels: Vec<Entity<FilePanel>>,
    layout: Layout,
    grid: GridState,
    active: usize,
    prev_active: Option<usize>,
    /// Fractions loaded from the config, applied once the grid has been laid out.
    pending_sizes: Option<(Vec<f32>, Vec<Vec<f32>>)>,
    ops: Entity<OperationsManager>,
    drop_menu: Option<DropMenu>,
    /// The panel the destination palette currently points at.
    highlight: Option<usize>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<GridEvent> for GridView {}

impl GridView {
    pub fn new(
        cfg: &TabConfig,
        ops: Entity<OperationsManager>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let panels: Vec<Entity<FilePanel>> = (0..MAX_PANELS)
            .map(|ix| cx.new(|cx| FilePanel::new(ix, cfg.panel(ix), window, cx)))
            .collect();

        let mut subscriptions = Vec::new();
        for (ix, panel) in panels.iter().enumerate() {
            subscriptions.push(cx.subscribe_in(
                panel,
                window,
                move |this, _panel, event, window, cx| match event {
                    PanelEvent::Focused => this.set_active(ix, cx),
                    PanelEvent::Changed => cx.emit(GridEvent::Changed),
                    PanelEvent::CancelRequested => {
                        this.ops.update(cx, |m, cx| {
                            m.cancel_latest(cx);
                        });
                    }
                    PanelEvent::Dropped { paths, dest } => {
                        this.handle_drop(paths.clone(), dest.clone(), window, cx);
                    }
                },
            ));
        }
        // Any finished operation may have changed what the panels show.
        subscriptions.push(cx.subscribe(&ops, |this, _ops, event, cx| {
            if matches!(event, ManagerEvent::Finished(_)) {
                for panel in &this.panels {
                    panel.update(cx, |p, cx| p.refresh(None, cx));
                }
            }
        }));

        Self {
            panels,
            layout: cfg.layout.layout(),
            grid: GridState::new(cx),
            active: 0,
            prev_active: None,
            pending_sizes: cfg.layout.fractions(),
            ops,
            drop_menu: None,
            highlight: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    /// The panel an operation acts on.
    pub fn active_panel(&self) -> &Entity<FilePanel> {
        &self.panels[self.active]
    }

    /// The panel an operation targets: the previously focused visible panel,
    /// else the next one in reading order.
    pub fn target_panel(&self) -> &Entity<FilePanel> {
        let count = self.layout.count();
        let ix = self
            .prev_active
            .filter(|&p| p != self.active && p < count)
            .unwrap_or_else(|| self.layout.next(self.active));
        &self.panels[ix]
    }

    /// Short name of the active panel's directory, used as the tab title.
    pub fn title(&self, cx: &App) -> String {
        self.active_panel().read(cx).state.path.display_name()
    }

    pub fn config(&self, cx: &App) -> TabConfig {
        let (row_sizes, col_sizes) = self
            .grid
            .fractions(self.layout, cx)
            .or_else(|| self.pending_sizes.clone())
            .unwrap_or_default();
        TabConfig {
            kind: TabKind::Grid,
            layout: LayoutConfig {
                rows: self.layout.rows,
                cols: self.layout.cols,
                row_sizes,
                col_sizes,
            },
            panels: self.panels.iter().map(|p| p.read(cx).config()).collect(),
        }
    }

    pub fn focus_active(&self, window: &mut Window, cx: &mut App) {
        let handle = self.panels[self.active].focus_handle(cx);
        handle.focus(window, cx);
    }

    fn set_active(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix != self.active {
            self.prev_active = Some(self.active);
            self.active = ix;
            cx.emit(GridEvent::Changed);
        }
        cx.notify();
    }

    fn focus_panel(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix < self.layout.count() {
            let handle = self.panels[ix].focus_handle(cx);
            handle.focus(window, cx);
        }
    }

    fn set_layout(&mut self, layout: Layout, window: &mut Window, cx: &mut Context<Self>) {
        if layout != self.layout {
            self.layout = layout;
            // Fresh states so the new grid starts evenly distributed.
            self.grid = GridState::new(cx);
            self.pending_sizes = None;
        }
        let count = layout.count();
        if self.active >= count {
            self.active = count - 1;
        }
        if self.prev_active.is_some_and(|p| p >= count) {
            self.prev_active = None;
        }
        self.focus_panel(self.active, window, cx);
        cx.emit(GridEvent::Changed);
        cx.notify();
    }

    // ----- file operations ----------------------------------------------

    fn notify(&self, note: Notification, window: &mut Window, cx: &mut App) {
        window.push_notification(note, cx);
    }

    /// Marked (or cursor) paths of the active panel, or a warning when empty.
    fn selection(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<Vec<Location>> {
        let paths = self.active_panel().read(cx).selected_paths();
        if paths.is_empty() {
            self.notify(Notification::warning("Nothing selected"), window, cx);
            return None;
        }
        Some(paths)
    }

    /// F5 / F6: ask for the destination, then run. The chooser lists the
    /// other panels (prefilled with the target panel) and lets the user type
    /// any path instead.
    pub fn start_transfer(&mut self, kind: OpKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sources) = self.selection(window, cx) else {
            return;
        };
        let dest_default = self.target_panel().read(cx).path().display();
        let title = format!("{} {}", kind.verb(), ops::describe(&sources));

        // Rows in reading order; `panel_of[row]` maps a row back to its panel.
        let mut panel_of = Vec::new();
        let mut items = Vec::new();
        for ix in (0..self.layout.count()).filter(|&ix| ix != self.active) {
            let path = self.panels[ix].read(cx).path().display();
            panel_of.push(ix);
            items.push(
                PaletteItem::new(path)
                    .detail(format!("Panel {}", ix + 1))
                    .icon(IconName::Folder),
            );
        }

        let view = cx.entity();
        let palette = cx.new(|cx| {
            let (confirm_view, select_view, cancel_view) = (view.clone(), view.clone(), view);
            let panel_of = Rc::new(panel_of);
            Palette::new(items, &dest_default, "Destination directory", window, cx)
                .on_select(move |row, _window, cx| {
                    let target = row.and_then(|row| panel_of.get(row).copied());
                    select_view.update(cx, |this, cx| this.set_highlight(target, cx));
                })
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    let (view, sources) = (confirm_view.clone(), sources.clone());
                    window.defer(cx, move |window, cx| {
                        view.update(cx, |this, cx| {
                            this.set_highlight(None, cx);
                            let dest = crate::panel::expand_path(&choice.value);
                            this.confirm_transfer(kind, sources, dest, window, cx);
                        });
                    });
                })
                .on_cancel(move |window, cx| {
                    window.close_dialog(cx);
                    cancel_view.update(cx, |this, cx| this.set_highlight(None, cx));
                })
                .hint("No other panel in this layout — type a path")
        });
        let initial = palette.read(cx).selected().and_then(|row| {
            (0..self.layout.count())
                .filter(|&ix| ix != self.active)
                .nth(row)
        });
        self.set_highlight(initial, cx);

        palette::open(title, palette, window, cx);
    }

    /// Mark `target` as the panel the destination palette points at.
    fn set_highlight(&mut self, target: Option<usize>, cx: &mut Context<Self>) {
        if self.highlight == target {
            return;
        }
        if let Some(old) = self.highlight.take() {
            self.panels[old].update(cx, |p, cx| p.set_highlighted(false, cx));
        }
        if let Some(new) = target {
            self.panels[new].update(cx, |p, cx| p.set_highlighted(true, cx));
        }
        self.highlight = target;
    }

    fn confirm_transfer(
        &mut self,
        kind: OpKind,
        sources: Vec<Location>,
        dest: Location,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_dir = backend_for(&dest)
            .and_then(|b| b.stat(&dest))
            .is_ok_and(|m| m.is_dir());
        if !is_dir {
            self.notify(
                Notification::error(format!("{} is not a directory", dest.display())),
                window,
                cx,
            );
            return;
        }
        let dest = canonical(dest);
        let conflicts = ops::conflicts(&sources, &dest);
        let request = Request {
            kind,
            sources,
            dest: Some(dest),
            overwrite: false,
        };
        if conflicts.is_empty() {
            self.run_operation(request, cx);
            return;
        }

        let names: Vec<String> = conflicts
            .iter()
            .take(5)
            .filter_map(Location::file_name)
            .collect();
        let description = format!(
            "{} already exist{} in {}: {}{}",
            conflicts.len(),
            if conflicts.len() == 1 { "s" } else { "" },
            request
                .dest
                .as_ref()
                .map(Location::display)
                .unwrap_or_default(),
            names.join(", "),
            if conflicts.len() > names.len() {
                ", …"
            } else {
                ""
            }
        );
        let overwrite = request.clone_with_overwrite(true);
        let view = cx.entity();
        let palette = cx.new(|cx| {
            let items = vec![
                PaletteItem::new("Overwrite").danger(true),
                PaletteItem::new("Skip existing"),
                PaletteItem::new("Cancel"),
            ];
            Palette::pick(items, cx)
                .description(description)
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    let request = match choice.item {
                        Some(0) => overwrite.clone(),
                        Some(1) => request.clone(),
                        _ => return,
                    };
                    view.update(cx, |this, cx| this.run_operation(request, cx));
                })
                .on_cancel(|window, cx| window.close_dialog(cx))
        });
        palette::open("Files already exist".to_string(), palette, window, cx);
    }

    /// Entries dropped onto `dest`: move or copy them there according to the
    /// `drop_action` setting, asking when it says so.
    fn handle_drop(
        &mut self,
        paths: Vec<Location>,
        dest: Location,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dest = canonical(dest);
        let sources: Vec<Location> = paths
            .into_iter()
            .filter(|p| p.parent().as_ref() != Some(&dest) && *p != dest && !dest.starts_with(p))
            .collect();
        if sources.is_empty() {
            return;
        }
        match settings::get(cx).drop_action {
            DropAction::Move => self.confirm_transfer(OpKind::Move, sources, dest, window, cx),
            DropAction::Copy => self.confirm_transfer(OpKind::Copy, sources, dest, window, cx),
            DropAction::Ask => self.ask_drop(sources, dest, window, cx),
        }
    }

    /// A two-item popup menu (Move / Copy) right where the entries were dropped.
    fn ask_drop(
        &mut self,
        sources: Vec<Location>,
        dest: Location,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = window.mouse_position();
        let view = cx.entity();
        let run = Rc::new(move |kind: OpKind, window: &mut Window, cx: &mut App| {
            let (view, sources, dest) = (view.clone(), sources.clone(), dest.clone());
            window.defer(cx, move |window, cx| {
                view.update(cx, |this, cx| {
                    this.drop_menu = None;
                    this.confirm_transfer(kind, sources, dest, window, cx);
                    cx.notify();
                });
            });
        });
        let menu = PopupMenu::build(window, cx, |menu, _window, _cx| {
            let (run_move, run_copy) = (run.clone(), run.clone());
            menu.item(
                PopupMenuItem::new("Move here")
                    .on_click(move |_, window, cx| run_move(OpKind::Move, window, cx)),
            )
            .item(
                PopupMenuItem::new("Copy here")
                    .on_click(move |_, window, cx| run_copy(OpKind::Copy, window, cx)),
            )
        });
        let subscription = cx.subscribe(&menu, |this, _menu, _: &DismissEvent, cx| {
            this.drop_menu = None;
            cx.notify();
        });
        // Focus the menu once it is on screen so Esc and the arrows work.
        let focus = menu.focus_handle(cx);
        window.defer(cx, move |window, cx| focus.focus(window, cx));
        self.drop_menu = Some(DropMenu {
            menu,
            position,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// F8 / Shift+F8: confirm, then delete.
    pub fn start_delete(&mut self, permanent: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sources) = self.selection(window, cx) else {
            return;
        };
        let kind = OpKind::Delete { permanent };
        let names: Vec<String> = sources
            .iter()
            .take(8)
            .filter_map(Location::file_name)
            .collect();
        let description = format!(
            "{}{}",
            names.join(", "),
            if sources.len() > names.len() {
                ", …"
            } else {
                ""
            }
        );
        let title = format!("{} {}?", kind.verb(), ops::describe(&sources));
        let request = Request {
            kind,
            sources,
            dest: None,
            overwrite: false,
        };
        let view = cx.entity();
        let palette = cx.new(|cx| {
            let items = vec![
                PaletteItem::new(if permanent {
                    "Delete permanently"
                } else {
                    "Move to trash"
                })
                .danger(permanent),
                PaletteItem::new("Cancel"),
            ];
            Palette::pick(items, cx)
                .description(description)
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    if choice.item == Some(0) {
                        let request = request.clone();
                        view.update(cx, |this, cx| this.run_operation(request, cx));
                    }
                })
                .on_cancel(|window, cx| window.close_dialog(cx))
        });
        palette::open(title, palette, window, cx);
    }

    pub fn start_mkdir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = self.active_panel().read(cx).path().clone();
        let hint = format!("in {}", base.display());
        let view = cx.entity();
        let palette = cx.new(|cx| {
            Palette::new(Vec::new(), "", "New folder name", window, cx)
                .hint(hint)
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    let (view, base) = (view.clone(), base.clone());
                    window.defer(cx, move |window, cx| {
                        view.update(cx, |this, cx| {
                            this.make_dir(&base, choice.value.trim(), window, cx)
                        });
                    });
                })
                .on_cancel(|window, cx| window.close_dialog(cx))
        });
        palette::open("New folder".to_string(), palette, window, cx);
    }

    fn make_dir(
        &mut self,
        base: &Location,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if name.is_empty() {
            return;
        }
        let path = base.join(name);
        match backend_for(base).and_then(|b| b.mkdir_all(&path)) {
            Ok(()) => {
                // Select the first component of a nested name.
                let first = name
                    .split(['/', '\\'])
                    .find(|s| !s.is_empty())
                    .map(|c| base.join(c));
                self.refresh_active(first, cx);
            }
            Err(err) => self.notify(
                Notification::error(format!("{}: {err}", path.display())),
                window,
                cx,
            ),
        }
    }

    /// F3 / F4: view or edit the file under the cursor. Directories open as
    /// usual; files the editor cannot show go to the default application.
    pub fn start_edit(&mut self, readonly: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.active_panel().read(cx).current_path() else {
            self.notify(
                Notification::warning("Nothing under the cursor"),
                window,
                cx,
            );
            return;
        };
        let is_dir = backend_for(&path)
            .and_then(|b| b.stat(&path))
            .is_ok_and(|m| m.is_dir());
        if is_dir {
            self.active_panel()
                .update(cx, |panel, cx| panel.navigate_to(path, cx));
            return;
        }
        let Some(local) = path.as_local() else {
            self.notify(
                Notification::info("Remote files cannot be edited yet"),
                window,
                cx,
            );
            return;
        };
        cx.emit(GridEvent::OpenFile {
            path: local.to_path_buf(),
            readonly,
        });
    }

    fn on_view_file(&mut self, _: &ViewFile, window: &mut Window, cx: &mut Context<Self>) {
        self.start_edit(true, window, cx);
    }
    fn on_edit_file(&mut self, _: &EditFile, window: &mut Window, cx: &mut Context<Self>) {
        self.start_edit(false, window, cx);
    }

    pub fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let marked = self.active_panel().read(cx).selected_paths();
        if marked.len() > 1 {
            cx.emit(GridEvent::BatchRename(marked));
            return;
        }
        let panel = self.active_panel().read(cx);
        // The file name itself, not the relative path a recursive search shows.
        let old_name = panel.current_path().and_then(|p| p.file_name());
        let is_dir = panel.state.current().is_some_and(|e| e.is_dir());
        let (Some(old_name), Some(from)) = (old_name, panel.current_path()) else {
            self.notify(Notification::warning("Nothing to rename"), window, cx);
            return;
        };
        let base = from.parent().unwrap_or_else(|| from.clone());
        let hint = format!("in {}", base.display());

        let view = cx.entity();
        let palette = cx.new(|cx| {
            let (confirm_view, old) = (view, old_name.clone());
            let base_for_confirm = base.clone();
            Palette::new(Vec::new(), &old_name, "New name", window, cx)
                .input_icon(Some(if is_dir {
                    IconName::Folder
                } else {
                    IconName::File
                }))
                .hint(hint)
                // Select the name without its extension, like a Commander.
                .select_range(0..stem_len(&old_name, is_dir))
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    let (view, old, base) =
                        (confirm_view.clone(), old.clone(), base_for_confirm.clone());
                    window.defer(cx, move |window, cx| {
                        view.update(cx, |this, cx| {
                            this.rename(&base, &old, choice.value.trim(), window, cx);
                        });
                    });
                })
                .on_cancel(|window, cx| window.close_dialog(cx))
        });
        palette::open(format!("Rename {old_name}"), palette, window, cx);
    }

    fn rename(
        &mut self,
        base: &Location,
        old_name: &str,
        new_name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if new_name.is_empty() || new_name == old_name {
            return;
        }
        let (from, to) = (base.join(old_name), base.join(new_name));
        let result = backend_for(base).and_then(|backend| {
            if backend.exists(&to) {
                Err(std::io::Error::other(
                    "a file with that name already exists",
                ))
            } else {
                backend.rename(&from, &to)
            }
        });
        match result {
            Ok(()) => self.refresh_active(Some(to), cx),
            Err(err) => self.notify(
                Notification::error(format!("{new_name}: {err}")),
                window,
                cx,
            ),
        }
    }

    fn run_operation(&mut self, request: Request, cx: &mut Context<Self>) {
        self.ops.update(cx, |m, cx| {
            m.start(request, cx);
        });
    }

    /// Reload every panel showing `dir` (after a batch rename).
    pub fn refresh_dir(&mut self, dir: &Location, cx: &mut Context<Self>) {
        for panel in &self.panels {
            if panel.read(cx).path() == dir {
                panel.update(cx, |p, cx| p.refresh(None, cx));
            }
        }
    }

    fn refresh_active(&mut self, select: Option<Location>, cx: &mut Context<Self>) {
        self.active_panel()
            .update(cx, |p, cx| p.refresh(select, cx));
    }

    // ----- action handlers ----------------------------------------------

    fn on_copy(&mut self, _: &CopyFiles, window: &mut Window, cx: &mut Context<Self>) {
        self.start_transfer(OpKind::Copy, window, cx);
    }
    fn on_move(&mut self, _: &MoveFiles, window: &mut Window, cx: &mut Context<Self>) {
        self.start_transfer(OpKind::Move, window, cx);
    }
    fn on_copy_to_clipboard(
        &mut self,
        _: &CopyToClipboard,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.fill_clipboard(false, window, cx);
    }
    fn on_cut_to_clipboard(
        &mut self,
        _: &CutToClipboard,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.fill_clipboard(true, window, cx);
    }
    fn on_paste(&mut self, _: &PasteFromClipboard, window: &mut Window, cx: &mut Context<Self>) {
        self.paste(window, cx);
    }

    /// Ctrl+C / Ctrl+X: remember the selection for a later paste.
    fn fill_clipboard(&mut self, cut: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(paths) = self.selection(window, cx) else {
            return;
        };
        clipboard::set(cx, paths, cut);
        self.clipboard_changed(cx);
    }

    /// Ctrl+V: copy or move the clipboard into the active panel's directory.
    /// Entries already there are skipped; a cut clipboard is spent.
    fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let clip = clipboard::get(cx);
        if clip.is_empty() {
            self.notify(Notification::warning("Nothing to paste"), window, cx);
            return;
        }
        let dest = canonical(self.active_panel().read(cx).path().clone());
        let sources: Vec<Location> = clip
            .paths
            .iter()
            .filter(|p| p.parent().as_ref() != Some(&dest))
            .cloned()
            .collect();
        if clip.cut {
            clipboard::clear(cx);
            self.clipboard_changed(cx);
        }
        if sources.is_empty() {
            self.notify(Notification::info("Already in this directory"), window, cx);
            return;
        }
        let kind = if clip.cut { OpKind::Move } else { OpKind::Copy };
        self.confirm_transfer(kind, sources, dest, window, cx);
    }

    /// Re-render every panel (cut entries are dimmed) and tell the workspace.
    fn clipboard_changed(&mut self, cx: &mut Context<Self>) {
        self.redraw_panels(cx);
        cx.emit(GridEvent::ClipboardChanged);
    }

    /// Re-render every panel (a display setting changed).
    pub fn redraw_panels(&mut self, cx: &mut Context<Self>) {
        for panel in &self.panels {
            panel.update(cx, |_, cx| cx.notify());
        }
    }

    fn on_rename(&mut self, _: &RenameFile, window: &mut Window, cx: &mut Context<Self>) {
        self.start_rename(window, cx);
    }
    fn on_mkdir(&mut self, _: &MakeDir, window: &mut Window, cx: &mut Context<Self>) {
        self.start_mkdir(window, cx);
    }
    fn on_delete(&mut self, _: &DeleteFiles, window: &mut Window, cx: &mut Context<Self>) {
        self.start_delete(false, window, cx);
    }
    fn on_delete_permanently(
        &mut self,
        _: &DeleteFilesPermanently,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_delete(true, window, cx);
    }
    fn on_focus_next(&mut self, _: &FocusNextPanel, window: &mut Window, cx: &mut Context<Self>) {
        let ix = self.layout.next(self.active);
        self.focus_panel(ix, window, cx);
    }
    fn on_focus_prev(&mut self, _: &FocusPrevPanel, window: &mut Window, cx: &mut Context<Self>) {
        let ix = self.layout.prev(self.active);
        self.focus_panel(ix, window, cx);
    }
    fn focus_dir(&mut self, dir: Dir, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.layout.neighbor(self.active, dir) {
            self.focus_panel(ix, window, cx);
        }
    }
    fn on_focus_up(&mut self, _: &FocusUp, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_dir(Dir::Up, window, cx);
    }
    fn on_focus_down(&mut self, _: &FocusDown, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_dir(Dir::Down, window, cx);
    }
    fn on_focus_left(&mut self, _: &FocusLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_dir(Dir::Left, window, cx);
    }
    fn on_focus_right(&mut self, _: &FocusRight, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_dir(Dir::Right, window, cx);
    }
    fn on_set_layout(&mut self, action: &SetLayout, window: &mut Window, cx: &mut Context<Self>) {
        self.set_layout(Layout::new(action.rows, action.cols), window, cx);
    }
}

impl Render for GridView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some((rows, cols)) = self.pending_sizes.clone() {
            if self
                .grid
                .apply_fractions(self.layout, &rows, &cols, window, cx)
            {
                self.pending_sizes = None;
            }
        }
        // Emit through a deferred update so the callback never re-enters this view.
        let view = cx.weak_entity();
        let on_resize: OnResize = Rc::new(move |_state, _window, cx| {
            let view = view.clone();
            cx.defer(move |cx| {
                view.update(cx, |_this, cx| cx.emit(GridEvent::Changed))
                    .ok();
            });
        });

        div()
            .key_context(GRID_CONTEXT)
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_move))
            .on_action(cx.listener(Self::on_rename))
            .on_action(cx.listener(Self::on_view_file))
            .on_action(cx.listener(Self::on_edit_file))
            .on_action(cx.listener(Self::on_mkdir))
            .on_action(cx.listener(Self::on_delete))
            .on_action(cx.listener(Self::on_delete_permanently))
            .on_action(cx.listener(Self::on_copy_to_clipboard))
            .on_action(cx.listener(Self::on_cut_to_clipboard))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(Self::on_focus_next))
            .on_action(cx.listener(Self::on_focus_prev))
            .on_action(cx.listener(Self::on_focus_up))
            .on_action(cx.listener(Self::on_focus_down))
            .on_action(cx.listener(Self::on_focus_left))
            .on_action(cx.listener(Self::on_focus_right))
            .on_action(cx.listener(Self::on_set_layout))
            .size_full()
            .min_h_0()
            .child(render_grid(
                self.layout,
                &self.grid,
                &self.panels,
                on_resize,
            ))
            .when_some(self.drop_menu.as_ref(), |d, drop| {
                d.child(
                    deferred(
                        anchored()
                            .position(drop.position)
                            .snap_to_window_with_margin(px(8.))
                            .child(div().occlude().child(drop.menu.clone())),
                    )
                    .with_priority(gpui_kit::base::POPUP_PRIORITY),
                )
            })
    }
}

/// Resolve symlinks of a local directory so that "already there" checks
/// compare like with like; remote locations are used as they are.
fn canonical(loc: Location) -> Location {
    match loc {
        Location::Local(path) => Location::Local(std::fs::canonicalize(&path).unwrap_or(path)),
        other => other,
    }
}

/// Length of the part of `name` to preselect when renaming: everything
/// before the extension for files, the whole name for directories and
/// dot-files.
fn stem_len(name: &str, is_dir: bool) -> usize {
    if is_dir {
        return name.len();
    }
    match name.rfind('.') {
        Some(dot) if dot > 0 => dot,
        _ => name.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::stem_len;

    #[test]
    fn stem_excludes_only_a_real_extension() {
        assert_eq!(stem_len("report.pdf", false), 6);
        assert_eq!(stem_len("archive.tar.gz", false), 11);
        assert_eq!(stem_len(".bashrc", false), 7);
        assert_eq!(stem_len("Makefile", false), 8);
        assert_eq!(stem_len("v1.0", true), 4);
    }
}
