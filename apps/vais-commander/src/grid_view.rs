//! One grid of file panels: layout, focus between panels, divider sizes, and
//! the file operations between the active and the target panel.

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
use crate::ops::manager::{ManagerEvent, OperationId, OperationsManager};
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

fn read_only_archive() -> &'static str {
    t!("Archives are read-only")
}

/// The name of a single entry, `None` for several.
fn single_name(sources: &[Location]) -> Option<String> {
    match sources {
        [single] => Some(single.file_name().unwrap_or_else(|| single.display())),
        _ => None,
    }
}

/// "Copy report.pdf" / "Move 3 items": the destination palette's title.
fn transfer_title(kind: OpKind, sources: &[Location]) -> String {
    let n = sources.len();
    match (kind == OpKind::Move, single_name(sources)) {
        (false, Some(name)) => t!("Copy {name}", name = name),
        (false, None) => tn!("Copy {n} item", "Copy {n} items", n),
        (true, Some(name)) => t!("Move {name}", name = name),
        (true, None) => tn!("Move {n} item", "Move {n} items", n),
    }
}

/// The delete confirmation's title; `requester` names the agent asking.
fn delete_title(permanent: bool, sources: &[Location], requester: Option<&str>) -> String {
    let n = sources.len();
    let name = single_name(sources);
    match (requester, permanent, name) {
        (None, true, Some(name)) => t!("Delete {name} permanently?", name = name),
        (None, true, None) => tn!(
            "Delete {n} item permanently?",
            "Delete {n} items permanently?",
            n
        ),
        (None, false, Some(name)) => t!("Move {name} to trash?", name = name),
        (None, false, None) => tn!("Move {n} item to trash?", "Move {n} items to trash?", n),
        (Some(who), true, Some(name)) => t!(
            "{agent} wants to delete {name} permanently",
            agent = who,
            name = name
        ),
        (Some(who), true, None) => tn!(
            "{agent} wants to delete {n} item permanently",
            "{agent} wants to delete {n} items permanently",
            n,
            agent = who
        ),
        (Some(who), false, Some(name)) => t!(
            "{agent} wants to move {name} to trash",
            agent = who,
            name = name
        ),
        (Some(who), false, None) => tn!(
            "{agent} wants to move {n} item to trash",
            "{agent} wants to move {n} items to trash",
            n,
            agent = who
        ),
    }
}

pub enum GridEvent {
    /// Something worth persisting (or showing in the tab title) changed.
    Changed,
    /// The file clipboard changed (the status bar shows it).
    ClipboardChanged,
    /// F2 with several entries marked: open the batch rename form for them.
    BatchRename(Vec<Location>),
    /// F3 / F4: open the file under the cursor in an editor tab.
    OpenFile { location: Location, readonly: bool },
    /// The star next to a panel's path.
    ToggleFavorite(Location),
}

/// How a transfer or delete asked for by another party (the agents'
/// bridge) ended up: started as an operation, cancelled in the dialog, or
/// refused outright.
pub enum Outcome {
    Started(OperationId),
    Cancelled,
    Rejected(String),
}

pub type OnOutcome = Rc<dyn Fn(Outcome, &mut Window, &mut App)>;

pub struct GridView {
    panels: Vec<Entity<FilePanel>>,
    /// A title set by the user (Rename tab); `None` follows the active panel.
    title: Option<String>,
    /// A colour tag (`#rrggbb`) for the tab.
    color: Option<String>,
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
                    PanelEvent::ToggleFavorite(path) => {
                        cx.emit(GridEvent::ToggleFavorite(path.clone()));
                    }
                    // Only a picker's panel reports activations.
                    PanelEvent::Activated(_) => {}
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
            title: cfg.title.clone(),
            color: cfg.color.clone(),
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

    /// Every panel slot (only the first rows x cols are shown).
    pub fn panels(&self) -> &[Entity<FilePanel>] {
        &self.panels
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
        match &self.title {
            Some(title) => title.clone(),
            None => self.active_panel().read(cx).state.path.display_name(),
        }
    }

    /// The user-given title, if any.
    pub fn custom_title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn color(&self) -> Option<&str> {
        self.color.as_deref()
    }

    pub fn set_color(&mut self, color: Option<String>, cx: &mut Context<Self>) {
        if self.color != color {
            self.color = color;
            cx.emit(GridEvent::Changed);
            cx.notify();
        }
    }

    /// Give the tab a fixed title; `None` goes back to the directory name.
    pub fn set_title(&mut self, title: Option<String>, cx: &mut Context<Self>) {
        let title = title.filter(|t| !t.trim().is_empty());
        if self.title != title {
            self.title = title;
            cx.emit(GridEvent::Changed);
            cx.notify();
        }
    }

    pub fn config(&self, cx: &App) -> TabConfig {
        let (row_sizes, col_sizes) = self
            .grid
            .fractions(self.layout, cx)
            .or_else(|| self.pending_sizes.clone())
            .unwrap_or_default();
        TabConfig {
            kind: TabKind::Grid,
            title: self.title.clone(),
            color: self.color.clone(),
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

    /// The focus went to (or came back from) the Agents panel: the active
    /// panel keeps (or drops) its active look.
    pub fn set_focus_lent(&mut self, lent: bool, cx: &mut Context<Self>) {
        for (ix, panel) in self.panels.iter().enumerate() {
            let active = self.active;
            panel.update(cx, |panel, cx| {
                panel.set_focus_lent(lent && ix == active, cx)
            });
        }
    }

    pub fn focus_panel(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix < self.layout.count() {
            let handle = self.panels[ix].focus_handle(cx);
            handle.focus(window, cx);
        }
    }

    pub fn set_layout(&mut self, layout: Layout, window: &mut Window, cx: &mut Context<Self>) {
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
            self.notify(Notification::warning(t!("Nothing selected")), window, cx);
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
        let title = transfer_title(kind, &sources);

        // Rows in reading order; `panel_of[row]` maps a row back to its panel.
        let mut panel_of = Vec::new();
        let mut items = Vec::new();
        for ix in (0..self.layout.count()).filter(|&ix| ix != self.active) {
            let path = self.panels[ix].read(cx).path().display();
            panel_of.push(ix);
            items.push(
                PaletteItem::new(path)
                    .detail(t!("Panel {number}", number = ix + 1))
                    .icon(IconName::Folder),
            );
        }

        let view = cx.entity();
        let palette = cx.new(|cx| {
            let (confirm_view, select_view, cancel_view) = (view.clone(), view.clone(), view);
            let panel_of = Rc::new(panel_of);
            Palette::new(
                items,
                &dest_default,
                t!("Destination directory"),
                window,
                cx,
            )
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
            .hint(t!("No other panel in this layout — type a path"))
        });
        let initial = palette.read(cx).selected().and_then(|row| {
            (0..self.layout.count())
                .filter(|&ix| ix != self.active)
                .nth(row)
        });
        self.set_highlight(initial, cx);

        palette::open(title, palette, window, cx);
    }

    /// The archive under the cursor, else the first marked one.
    fn archive_at_cursor(&self, cx: &App) -> Option<(Location, String)> {
        let panel = self.active_panel().read(cx);
        let is_archive = |l: &Location| {
            l.file_name()
                .and_then(|n| crate::vfs::archive::stem(&n).map(str::to_string))
        };
        panel
            .current_path()
            .into_iter()
            .chain(panel.selected_paths())
            .find_map(|l| is_archive(&l).map(|stem| (l, stem)))
    }

    /// The entry menu's "Extract…": choose where the archive's contents go.
    /// Every panel of the layout offers its directory and a new folder named
    /// after the archive in it (the archive's own panel included); a typed
    /// path works too. The archive's own panel with the new folder is the
    /// default, so nothing is spilled loose by accident.
    pub fn start_extract(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((archive, stem)) = self.archive_at_cursor(cx) else {
            self.notify(
                Notification::warning(t!("No archive under the cursor")),
                window,
                cx,
            );
            return;
        };
        let title = t!(
            "Extract {name}",
            name = archive.file_name().unwrap_or_else(|| stem.clone())
        );

        // Two rows per panel in reading order; `panel_of[row]` maps a row
        // back to its panel for the highlight.
        let mut panel_of = Vec::new();
        let mut items = Vec::new();
        let mut default = String::new();
        for ix in 0..self.layout.count() {
            let dir = self.panels[ix].read(cx).path().clone();
            let number = ix + 1;
            let here = dir.display();
            let folder = dir.join(&stem).display();
            if ix == self.active {
                default = folder.clone();
            }
            items.push(
                PaletteItem::new(here)
                    .detail(t!("Panel {number}", number = number))
                    .icon(IconName::Folder),
            );
            items.push(
                PaletteItem::new(folder)
                    .detail(t!("Panel {number}, own folder", number = number))
                    .icon(IconName::FolderOpen),
            );
            panel_of.extend([ix, ix]);
        }

        let view = cx.entity();
        let palette = cx.new(|cx| {
            let (confirm_view, select_view, cancel_view) = (view.clone(), view.clone(), view);
            let panel_of = Rc::new(panel_of);
            Palette::new(items, &default, t!("Destination directory"), window, cx)
                .on_select(move |row, _window, cx| {
                    let target = row.and_then(|row| panel_of.get(row).copied());
                    select_view.update(cx, |this, cx| this.set_highlight(target, cx));
                })
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    let (view, archive) = (confirm_view.clone(), archive.clone());
                    window.defer(cx, move |window, cx| {
                        view.update(cx, |this, cx| {
                            this.set_highlight(None, cx);
                            let dest = crate::panel::expand_path(&choice.value);
                            this.extract_to(archive, dest, window, cx);
                        });
                    });
                })
                .on_cancel(move |window, cx| {
                    window.close_dialog(cx);
                    cancel_view.update(cx, |this, cx| this.set_highlight(None, cx));
                })
        });
        let initial = palette
            .read(cx)
            .selected()
            .map(|row| row / 2)
            .filter(|&ix| ix < self.layout.count());
        self.set_highlight(initial, cx);
        palette::open(title, palette, window, cx);
    }

    /// Copy everything at the archive's root into `dest`, creating `dest`
    /// when it does not exist yet; existing files are asked about as with
    /// any copy.
    fn extract_to(
        &mut self,
        archive: Location,
        dest: Location,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if dest.is_archive() {
            self.notify(Notification::error(read_only_archive()), window, cx);
            return;
        }
        // Listing the root unpacks a tar or reads the zip index: not on the
        // UI thread.
        let root = Location::archive(archive.clone(), "/");
        let listing = cx.background_spawn(async move {
            backend_for(&root).and_then(|backend| backend.read_dir(&root, true))
        });
        cx.spawn_in(window, async move |this, cx| {
            let entries = listing.await;
            this.update_in(cx, |this, window, cx| {
                let sources: Vec<Location> = match entries {
                    Ok(entries) => entries
                        .into_iter()
                        .filter(|e| !e.is_parent)
                        .map(|e| e.location)
                        .collect(),
                    Err(err) => {
                        let message = format!("{}: {err}", archive.display());
                        this.notify(Notification::error(message), window, cx);
                        return;
                    }
                };
                if sources.is_empty() {
                    let message = t!("{archive} is empty", archive = archive.display());
                    this.notify(Notification::info(message), window, cx);
                    return;
                }
                let exists = backend_for(&dest).and_then(|b| b.stat(&dest)).is_ok();
                if !exists {
                    if let Err(err) = backend_for(&dest).and_then(|b| b.mkdir_all(&dest)) {
                        let message = format!("{}: {err}", dest.display());
                        this.notify(Notification::error(message), window, cx);
                        return;
                    }
                    if let Some(parent) = dest.parent() {
                        this.refresh_dir(&parent, cx);
                    }
                }
                this.confirm_transfer(OpKind::Copy, sources, dest, window, cx);
            })
            .ok();
        })
        .detach();
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
        self.confirm_transfer_with(kind, sources, dest, None, window, cx);
    }

    /// Copy or move `sources` into `dest`, asking about existing files;
    /// `on_outcome` (the bridge) hears how it ended.
    pub fn confirm_transfer_with(
        &mut self,
        kind: OpKind,
        sources: Vec<Location>,
        dest: Location,
        on_outcome: Option<OnOutcome>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reject =
            |this: &mut Self, message: String, window: &mut Window, cx: &mut Context<Self>| {
                this.notify(Notification::error(message.clone()), window, cx);
                if let Some(cb) = &on_outcome {
                    cb(Outcome::Rejected(message), window, cx);
                }
            };
        if dest.is_archive() {
            reject(self, read_only_archive().to_string(), window, cx);
            return;
        }
        if kind == OpKind::Move && sources.iter().any(Location::is_archive) {
            reject(
                self,
                t!("Cannot move out of an archive: copy instead").to_string(),
                window,
                cx,
            );
            return;
        }
        let is_dir = backend_for(&dest)
            .and_then(|b| b.stat(&dest))
            .is_ok_and(|m| m.is_dir());
        if !is_dir {
            reject(
                self,
                t!("{path} is not a directory", path = dest.display()),
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
            let id = self.run_operation(request, cx);
            if let Some(cb) = &on_outcome {
                cb(Outcome::Started(id), window, cx);
            }
            return;
        }

        let names: Vec<String> = conflicts
            .iter()
            .take(5)
            .filter_map(Location::file_name)
            .collect();
        let mut list = names.join(", ");
        if conflicts.len() > names.len() {
            list.push_str(", …");
        }
        let description = tn!(
            "{n} already exists in {dir}: {names}",
            "{n} already exist in {dir}: {names}",
            conflicts.len(),
            dir = request
                .dest
                .as_ref()
                .map(Location::display)
                .unwrap_or_default(),
            names = list
        );
        let overwrite = request.clone_with_overwrite(true);
        let view = cx.entity();
        let on_cancel_outcome = on_outcome.clone();
        let title = if on_outcome.is_some() {
            t!("Agent: files already exist").to_string()
        } else {
            t!("Files already exist").to_string()
        };
        let palette = cx.new(|cx| {
            let items = vec![
                PaletteItem::new(t!("Overwrite")).danger(true),
                PaletteItem::new(t!("Skip existing")),
                PaletteItem::new(t!("Cancel")),
            ];
            Palette::pick(items, cx)
                .description(description)
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    let request = match choice.item {
                        Some(0) => overwrite.clone(),
                        Some(1) => request.clone(),
                        _ => {
                            if let Some(cb) = &on_outcome {
                                cb(Outcome::Cancelled, window, cx);
                            }
                            return;
                        }
                    };
                    let id = view.update(cx, |this, cx| this.run_operation(request, cx));
                    if let Some(cb) = &on_outcome {
                        cb(Outcome::Started(id), window, cx);
                    }
                })
                .on_cancel(move |window, cx| {
                    window.close_dialog(cx);
                    if let Some(cb) = &on_cancel_outcome {
                        cb(Outcome::Cancelled, window, cx);
                    }
                })
        });
        palette::open(title, palette, window, cx);
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
                PopupMenuItem::new(t!("Move here"))
                    .on_click(move |_, window, cx| run_move(OpKind::Move, window, cx)),
            )
            .item(
                PopupMenuItem::new(t!("Copy here"))
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
        self.confirm_delete(sources, permanent, None, None, window, cx);
    }

    /// Ask, then delete `sources`. `requester` names who asks (an agent)
    /// in the title; `on_outcome` hears how it ended.
    pub fn confirm_delete(
        &mut self,
        sources: Vec<Location>,
        permanent: bool,
        requester: Option<String>,
        on_outcome: Option<OnOutcome>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if sources.iter().any(Location::is_archive) {
            self.notify(Notification::error(read_only_archive()), window, cx);
            if let Some(cb) = &on_outcome {
                cb(
                    Outcome::Rejected(read_only_archive().to_string()),
                    window,
                    cx,
                );
            }
            return;
        }
        // Remote storages have no trash.
        let permanent = permanent || sources.iter().any(|s| !s.is_local());
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
        let title = delete_title(permanent, &sources, requester.as_deref());
        let request = Request {
            kind,
            sources,
            dest: None,
            overwrite: false,
        };
        let view = cx.entity();
        let on_cancel_outcome = on_outcome.clone();
        let palette = cx.new(|cx| {
            let items = vec![
                PaletteItem::new(if permanent {
                    t!("Delete permanently")
                } else {
                    t!("Move to trash")
                })
                .danger(permanent),
                PaletteItem::new(t!("Cancel")),
            ];
            Palette::pick(items, cx)
                .description(description)
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    if choice.item == Some(0) {
                        let request = request.clone();
                        let id = view.update(cx, |this, cx| this.run_operation(request, cx));
                        if let Some(cb) = &on_outcome {
                            cb(Outcome::Started(id), window, cx);
                        }
                    } else if let Some(cb) = &on_outcome {
                        cb(Outcome::Cancelled, window, cx);
                    }
                })
                .on_cancel(move |window, cx| {
                    window.close_dialog(cx);
                    if let Some(cb) = &on_cancel_outcome {
                        cb(Outcome::Cancelled, window, cx);
                    }
                })
        });
        palette::open(title, palette, window, cx);
    }

    pub fn start_mkdir(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = self.active_panel().read(cx).path().clone();
        let hint = t!("in {dir}", dir = base.display());
        let view = cx.entity();
        let palette = cx.new(|cx| {
            Palette::new(Vec::new(), "", t!("New folder name"), window, cx)
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
        palette::open(t!("New folder").to_string(), palette, window, cx);
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
        match self.make_dir_quiet(base, name, cx) {
            Ok(_) => {}
            Err(err) => self.notify(Notification::error(err), window, cx),
        }
    }

    /// Create `name` under `base` and show it; the error is returned, not shown.
    pub fn make_dir_quiet(
        &mut self,
        base: &Location,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Result<Location, String> {
        if base.is_archive() {
            return Err(read_only_archive().to_string());
        }
        let path = base.join(name);
        backend_for(base)
            .and_then(|b| b.mkdir_all(&path))
            .map_err(|err| format!("{}: {err}", path.display()))?;
        // Select the first component of a nested name.
        let first = name
            .split(['/', '\\'])
            .find(|s| !s.is_empty())
            .map(|c| base.join(c));
        self.refresh_active(first, cx);
        Ok(path)
    }

    /// F3 / F4: view or edit the file under the cursor. Directories open as
    /// usual; files the editor cannot show go to the default application.
    pub fn start_edit(&mut self, readonly: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.active_panel().read(cx).current_path() else {
            self.notify(
                Notification::warning(t!("Nothing under the cursor")),
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
        cx.emit(GridEvent::OpenFile {
            location: path,
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
        if marked.iter().any(Location::is_archive) {
            self.notify(Notification::error(read_only_archive()), window, cx);
            return;
        }
        if marked.len() > 1 {
            cx.emit(GridEvent::BatchRename(marked));
            return;
        }
        let panel = self.active_panel().read(cx);
        // The file name itself, not the relative path a recursive search shows.
        let old_name = panel.current_path().and_then(|p| p.file_name());
        let is_dir = panel.state.current().is_some_and(|e| e.is_dir());
        let (Some(old_name), Some(from)) = (old_name, panel.current_path()) else {
            self.notify(Notification::warning(t!("Nothing to rename")), window, cx);
            return;
        };
        let base = from.parent().unwrap_or_else(|| from.clone());
        let hint = t!("in {dir}", dir = base.display());

        let view = cx.entity();
        let palette = cx.new(|cx| {
            let (confirm_view, old) = (view, old_name.clone());
            let base_for_confirm = base.clone();
            Palette::new(Vec::new(), &old_name, t!("New name"), window, cx)
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
        palette::open(t!("Rename {name}", name = old_name), palette, window, cx);
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
        if let Err(err) = self.rename_quiet(base, old_name, new_name, cx) {
            self.notify(Notification::error(err), window, cx);
        }
    }

    /// Rename `old_name` to `new_name` under `base`, refusing to overwrite;
    /// the error is returned, not shown.
    pub fn rename_quiet(
        &mut self,
        base: &Location,
        old_name: &str,
        new_name: &str,
        cx: &mut Context<Self>,
    ) -> Result<Location, String> {
        if base.is_archive() {
            return Err(read_only_archive().to_string());
        }
        let (from, to) = (base.join(old_name), base.join(new_name));
        backend_for(base)
            .and_then(|backend| {
                if backend.exists(&to) {
                    Err(std::io::Error::other(t!(
                        "a file with that name already exists"
                    )))
                } else {
                    backend.rename(&from, &to)
                }
            })
            .map_err(|err| format!("{new_name}: {err}"))?;
        self.refresh_dir(base, cx);
        self.refresh_active(Some(to.clone()), cx);
        Ok(to)
    }

    pub fn run_operation(&mut self, request: Request, cx: &mut Context<Self>) -> OperationId {
        self.ops.update(cx, |m, cx| m.start(request, cx))
    }

    /// Reload every panel on connection `conn` (it connected or dropped).
    pub fn refresh_conn(&mut self, conn: &crate::vfs::ConnId, cx: &mut Context<Self>) {
        for panel in &self.panels {
            if panel.read(cx).path().conn() == Some(conn) {
                panel.update(cx, |p, cx| p.refresh(None, cx));
            }
        }
    }

    /// Reload the panels showing (or expanding) one of `dirs`, which changed
    /// on disk.
    pub fn refresh_changed(&mut self, dirs: &[Location], cx: &mut Context<Self>) {
        for panel in &self.panels {
            let shown = {
                let p = panel.read(cx);
                dirs.iter()
                    .any(|d| d == p.path() || p.state.expanded.contains(d))
            };
            if shown {
                panel.update(cx, |p, cx| p.refresh_from_disk(cx));
            }
        }
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
    fn on_extract(&mut self, _: &ExtractArchive, window: &mut Window, cx: &mut Context<Self>) {
        self.start_extract(window, cx);
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
            if !self.paste_image(window, cx) {
                self.notify(Notification::warning(t!("Nothing to paste")), window, cx);
            }
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
            self.notify(
                Notification::info(t!("Already in this directory")),
                window,
                cx,
            );
            return;
        }
        let kind = if clip.cut { OpKind::Move } else { OpKind::Copy };
        self.confirm_transfer(kind, sources, dest, window, cx);
    }

    /// The system clipboard holds an image (a screenshot, a copied picture):
    /// ask for a file name and save it into the active panel's directory.
    /// `false` when there is no image.
    fn paste_image(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let Some(image) = item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image.clone()),
            _ => None,
        }) else {
            return false;
        };
        let dest = self.active_panel().read(cx).path().clone();
        if dest.is_archive() {
            self.notify(Notification::error(read_only_archive()), window, cx);
            return true;
        }
        if dest.is_roots() {
            self.notify(
                Notification::error(t!("Pick a directory to save the image in")),
                window,
                cx,
            );
            return true;
        }
        let extension = image.format.extension();
        let default = format!(
            "clipboard-{}.{extension}",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let description = t!(
            "{format} image, {size}",
            format = extension.to_uppercase(),
            size = crate::fs::format::human_size(image.bytes.len() as u64)
        );
        let bytes = std::sync::Arc::new(image.bytes);
        let view = cx.entity();
        let stem = stem_len(&default, false);
        // Two buttons under the name: Enter on the input or the Save row
        // saves, the Cancel row (or Esc) does nothing.
        let rows = vec![
            PaletteItem::new(t!("Save")).value("").icon(IconName::Check),
            PaletteItem::new(t!("Cancel"))
                .value("")
                .icon(IconName::Close),
        ];
        const CANCEL_ROW: usize = 1;
        let palette = cx.new(|cx| {
            let (dest_for_confirm, hint) = (dest.clone(), t!("in {dir}", dir = dest.display()));
            Palette::new(rows, &default, t!("File name"), window, cx)
                .input_icon(Some(IconName::File))
                .description(description)
                .hint(hint)
                .select_range(0..stem)
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    if choice.item == Some(CANCEL_ROW) {
                        return;
                    }
                    let (view, dest, bytes) =
                        (view.clone(), dest_for_confirm.clone(), bytes.clone());
                    let name = choice.value.trim().to_string();
                    window.defer(cx, move |window, cx| {
                        view.update(cx, |this, cx| {
                            this.save_clipboard_image(dest, name, bytes, window, cx);
                        });
                    });
                })
                .on_cancel(|window, cx| window.close_dialog(cx))
        });
        palette::open(t!("Save image from clipboard").into(), palette, window, cx);
        true
    }

    fn save_clipboard_image(
        &mut self,
        dest: Location,
        name: String,
        bytes: std::sync::Arc<Vec<u8>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if name.is_empty() || name.contains(['/', '\\']) {
            self.notify(Notification::error(t!("Not a valid file name")), window, cx);
            return;
        }
        let target = dest.join(&name);
        if backend_for(&target).is_ok_and(|b| b.exists(&target)) {
            self.notify(
                Notification::error(t!(
                    "{name}: a file with that name already exists",
                    name = name
                )),
                window,
                cx,
            );
            return;
        }
        // Writing may go to a connection: keep it off the UI thread.
        let task = cx.background_spawn({
            let target = target.clone();
            async move { crate::vfs::write_all(&target, &bytes).map_err(|e| e.to_string()) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(()) => {
                    this.refresh_active(Some(target), cx);
                    this.notify(
                        Notification::success(t!("Saved {name}", name = name)),
                        window,
                        cx,
                    );
                }
                Err(err) => this.notify(Notification::error(format!("{name}: {err}")), window, cx),
            })
            .ok();
        })
        .detach();
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

    /// Mark (or unmark) by a pattern typed into a palette; the default is
    /// `*.ext` of the file under the cursor.
    fn start_mark_pattern(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let panel = self.active_panel().clone();
        let default = panel
            .read(cx)
            .state
            .current()
            .filter(|e| !e.is_parent && !e.is_dir())
            .map(|e| match crate::fs::extension(&e.name) {
                "" => "*".to_string(),
                ext => format!("*.{ext}"),
            })
            .unwrap_or_else(|| "*".to_string());
        let title = if on {
            t!("Mark entries matching")
        } else {
            t!("Unmark entries matching")
        };
        let len = default.len();
        let palette = cx.new(|cx| {
            Palette::new(Vec::new(), &default, t!("Pattern (* and ?)"), window, cx)
                .hint(t!(
                    "Case-insensitive; text without wildcards matches anywhere in the name"
                ))
                .select_range(0..len)
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    let (panel, value) = (panel.clone(), choice.value.trim().to_string());
                    window.defer(cx, move |_, cx| {
                        panel.update(cx, |panel, cx| panel.mark_pattern(&value, on, cx));
                    });
                })
                .on_cancel(|window, cx| window.close_dialog(cx))
        });
        palette::open(title.to_string(), palette, window, cx);
    }

    fn on_mark_pattern(&mut self, _: &MarkByPattern, window: &mut Window, cx: &mut Context<Self>) {
        self.start_mark_pattern(true, window, cx);
    }

    fn on_unmark_pattern(
        &mut self,
        _: &UnmarkByPattern,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_mark_pattern(false, window, cx);
    }

    fn on_rename(&mut self, _: &RenameFile, window: &mut Window, cx: &mut Context<Self>) {
        self.start_rename(window, cx);
    }
    fn on_mkdir(&mut self, _: &MakeDir, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_panel().read(cx).path().is_archive() {
            self.notify(Notification::error(read_only_archive()), window, cx);
            return;
        }
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
            .on_action(cx.listener(Self::on_extract))
            .on_action(cx.listener(Self::on_rename))
            .on_action(cx.listener(Self::on_view_file))
            .on_action(cx.listener(Self::on_edit_file))
            .on_action(cx.listener(Self::on_mkdir))
            .on_action(cx.listener(Self::on_delete))
            .on_action(cx.listener(Self::on_delete_permanently))
            .on_action(cx.listener(Self::on_mark_pattern))
            .on_action(cx.listener(Self::on_unmark_pattern))
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
