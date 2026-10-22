//! One file panel: a directory listing with cursor, marks and a path bar.

pub mod state;
pub mod view;

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::clipboard::FileClipboard;
use crate::config::PanelConfig;
use crate::fs::format::{format_mode, format_time, human_size, owner_name};
use crate::fs::search::search_tree;
use crate::fs::{
    Entry, ReadOptions, Sort, SortDir, SortKey, extension, parent_entry, read_roots, read_tree,
};
use crate::keymap::*;
use crate::platform;
use crate::settings::FileIcons;
use crate::ui::{h_flex, v_flex};
use state::{Filter, PanelState};
use view::{Column, Columns, ViewMode};

/// Row height in rems, so the list follows the UI scale.
pub const ROW_HEIGHT_REMS: f32 = 1.5;
/// Width of a name cell in the brief view.
const BRIEF_CELL_REMS: f32 = 14.0;
/// Size of a thumbnail cell (square image above a one-line label).
const THUMB_CELL_REMS: f32 = 8.0;
const THUMB_IMAGE_REMS: f32 = 6.0;
const THUMB_ROW_REMS: f32 = 7.75;

/// Payload of the rubber band selection drag: which panel owns it. Drag-move
/// events reach every panel, so the others must ignore it.
#[derive(Clone, Copy, Debug)]
pub struct MarqueeDrag {
    panel: usize,
}

/// Invisible drag preview for the rubber band.
struct EmptyPreview;

impl Render for EmptyPreview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// An in-progress rubber band selection, in window coordinates.
struct Marquee {
    start: Point<Pixels>,
    current: Point<Pixels>,
    /// Marks to keep (Ctrl/Shift drag adds to them).
    base: std::collections::BTreeSet<usize>,
}

/// Entries being dragged with the mouse.
#[derive(Clone, Debug)]
pub struct DragEntries {
    pub paths: Vec<PathBuf>,
}

/// The little label that follows the mouse while dragging.
pub struct DragPreview {
    label: String,
}

impl Render for DragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_3()
            .py_1()
            .rounded_md()
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .border_1()
            .border_color(theme.border)
            .text_sm()
            .child(self.label.clone())
    }
}

pub enum PanelEvent {
    /// The panel (or its path editor) received focus.
    Focused,
    /// Something worth persisting changed (path, hidden toggle).
    Changed,
    /// Escape with nothing left to clear: cancel a running operation, if any.
    CancelRequested,
    /// Entries were dropped onto `dest` (a directory shown in this panel).
    Dropped { paths: Vec<PathBuf>, dest: PathBuf },
}

pub struct FilePanel {
    pub index: usize,
    focus_handle: FocusHandle,
    scroll: UniformListScrollHandle,
    pub state: PanelState,
    path_input: Entity<InputState>,
    editing_path: bool,
    filter_input: Entity<InputState>,
    /// Cancel flag of the running recursive search, if any.
    search_cancel: Option<Arc<AtomicBool>>,
    search_debounce: Option<Task<()>>,
    searching: bool,
    generation: u64,
    load_task: Option<Task<()>>,
    /// Last left mouse press in the body: where a rubber band would start, and
    /// whether it landed on a marked row (then the row's file drag must win).
    pending_press: Option<(Point<Pixels>, Modifiers, bool)>,
    /// Set by the row handler: the press landed on the row that already had the cursor.
    press_on_cursor: bool,
    marquee: Option<Marquee>,
    /// The destination palette points at this panel.
    highlighted: bool,
    view: ViewMode,
    columns: Columns,
    /// Entries per line in a grid view (1 in the full view); measured from
    /// the body width at the last render.
    per_row: usize,
    /// Scroll to the cursor at the next render (after a view change).
    pending_scroll: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PanelEvent> for FilePanel {}

impl Focusable for FilePanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Theme colours copied out so render closures do not borrow `cx`.
#[derive(Clone)]
struct Colors {
    primary: Hsla,
    list_active_border: Hsla,
    list_even: Hsla,
    muted_foreground: Hsla,
    foreground: Hsla,
    list_hover: Hsla,
    list_head: Hsla,
    border: Hsla,
    yellow: Hsla,
    blue: Hsla,
}

impl FilePanel {
    pub fn new(
        index: usize,
        cfg: PanelConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let path_input = cx.new(|cx| InputState::new(window, cx).placeholder("Path"));
        let filter_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter… (* and ? for globs)"));

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.on_focus_in(&focus_handle, window, |_this, _window, cx| {
            cx.emit(PanelEvent::Focused);
            cx.notify();
        }));
        subscriptions.push(cx.subscribe_in(
            &filter_input,
            window,
            |this, _input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.on_filter_changed(cx),
                InputEvent::PressEnter { .. } => this.focus_handle.focus(window, cx),
                _ => {}
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &path_input,
            window,
            |this, _input, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.confirm_path(window, cx),
                InputEvent::Blur => {
                    if this.editing_path {
                        this.editing_path = false;
                        cx.notify();
                    }
                }
                _ => {}
            },
        ));

        let sort = Sort {
            key: cfg.sort,
            dir: if cfg.sort_desc {
                SortDir::Desc
            } else {
                SortDir::Asc
            },
        };
        let mut this = Self {
            index,
            focus_handle,
            scroll: UniformListScrollHandle::new(),
            state: PanelState::new(cfg.path.clone(), cfg.show_hidden, sort),
            path_input,
            editing_path: false,
            filter_input,
            search_cancel: None,
            search_debounce: None,
            searching: false,
            generation: 0,
            load_task: None,
            pending_press: None,
            press_on_cursor: false,
            marquee: None,
            highlighted: false,
            view: cfg.view,
            columns: cfg.columns,
            per_row: 1,
            pending_scroll: false,
            _subscriptions: subscriptions,
        };
        this.load(cfg.path, None, cx);
        this
    }

    /// Draw the accent border even without focus (the destination palette
    /// points at this panel).
    pub fn set_highlighted(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.highlighted != on {
            self.highlighted = on;
            cx.notify();
        }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.state.path
    }

    /// Absolute paths an operation acts on (marked entries, else the cursor entry).
    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.state
            .selected_entries()
            .into_iter()
            .map(|e| e.path.clone())
            .collect()
    }

    /// Path of the entry under the cursor, unless it is `..`.
    pub fn current_path(&self) -> Option<PathBuf> {
        self.state
            .current()
            .filter(|e| !e.is_parent)
            .map(|e| e.path.clone())
    }

    /// Re-read the directory (and its expanded subdirectories), placing the
    /// cursor on `select` or keeping it on the current entry.
    pub fn refresh(&mut self, select: Option<PathBuf>, cx: &mut Context<Self>) {
        let select = select.or_else(|| self.state.current().map(|e| e.path.clone()));
        if self.recursive_search_active() {
            self.start_search(cx);
            return;
        }
        let path = self.state.path.clone();
        self.load(path, select, cx);
    }

    pub fn config(&self) -> PanelConfig {
        PanelConfig {
            path: self.state.path.clone(),
            show_hidden: self.state.show_hidden,
            sort: self.state.sort.key,
            sort_desc: self.state.sort.dir == SortDir::Desc,
            view: self.view,
            columns: self.columns,
        }
    }

    /// Switch the view. The tree (expanded directories) only exists in the
    /// full view, so leaving it collapses everything.
    pub fn set_view(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        if self.view == mode {
            return;
        }
        let leaving_full = self.view == ViewMode::Full;
        self.view = mode;
        if leaving_full && !self.state.expanded.is_empty() {
            self.state.expanded.clear();
            self.reload(cx);
        }
        self.pending_scroll = true;
        cx.emit(PanelEvent::Changed);
        cx.notify();
    }

    fn on_set_view(&mut self, action: &SetView, _: &mut Window, cx: &mut Context<Self>) {
        self.set_view(action.mode, cx);
    }

    fn on_toggle_column(&mut self, action: &ToggleColumn, _: &mut Window, cx: &mut Context<Self>) {
        self.columns.toggle(action.column);
        cx.emit(PanelEvent::Changed);
        cx.notify();
    }

    // ----- loading -------------------------------------------------------

    /// Read `path` on the background executor and apply the result unless a
    /// newer load has been started in the meantime.
    fn load(&mut self, path: PathBuf, select: Option<PathBuf>, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        if platform::is_roots_path(&path) {
            let roots = platform::roots();
            if roots.len() == 1 {
                // A single root (Unix): the roots listing would be pointless.
                let root = roots
                    .into_iter()
                    .next()
                    .unwrap_or_else(platform::default_root);
                return self.load(root, select, cx);
            }
            self.state.loading = false;
            self.state
                .set_entries(path, read_roots(), select.as_deref());
            self.scroll_to_cursor();
            cx.emit(PanelEvent::Changed);
            cx.notify();
            return;
        }
        self.state.loading = true;
        let opts = ReadOptions {
            show_hidden: self.state.show_hidden,
            sort: self.state.sort,
        };
        let read_path = path.clone();
        let expanded = self.state.expanded.clone();
        let task = cx.background_spawn(async move {
            read_tree(&read_path, opts, &expanded).map_err(|e| e.to_string())
        });
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |panel, cx| {
                if panel.generation != generation {
                    return;
                }
                panel.state.loading = false;
                match result {
                    Ok(entries) => {
                        panel.state.set_entries(path, entries, select.as_deref());
                        panel.scroll_to_cursor();
                        cx.emit(PanelEvent::Changed);
                    }
                    Err(err) => {
                        panel.state.error = Some(format!("{}: {err}", path.display()));
                    }
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    /// Go to `path` (recorded in the history), as if entered from the path bar.
    pub fn navigate_to(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.navigate(path, None, cx);
    }

    fn navigate(&mut self, path: PathBuf, select: Option<PathBuf>, cx: &mut Context<Self>) {
        if path != self.state.path {
            let current = self.state.path.clone();
            self.state.push_history(current);
            self.state.expanded.clear();
            self.drop_filter(cx);
        }
        self.load(path, select, cx);
    }

    fn open(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.state.current() else {
            return;
        };
        if entry.is_parent {
            self.go_up(cx);
        } else if entry.is_dir() {
            let path = entry.path.clone();
            self.navigate(path, None, cx);
        } else if let Err(err) = open::that_detached(&entry.path) {
            self.state.error = Some(format!("{}: {err}", entry.name));
            cx.notify();
        }
    }

    fn go_up(&mut self, cx: &mut Context<Self>) {
        let Some(parent) = platform::parent_of(&self.state.path) else {
            return;
        };
        let select = Some(self.state.path.clone());
        self.navigate(parent, select, cx);
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        if let Some(prev) = self.state.pop_back() {
            self.state.expanded.clear();
            self.drop_filter(cx);
            self.load(prev, None, cx);
        }
    }

    fn go_forward(&mut self, cx: &mut Context<Self>) {
        if let Some(next) = self.state.pop_forward() {
            self.state.expanded.clear();
            self.drop_filter(cx);
            self.load(next, None, cx);
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.refresh(None, cx);
    }

    /// Whether one of the panel's text inputs has the keyboard: list actions
    /// bound to keys the input does not consume (Enter, Space, Insert, …) must
    /// then stay out of the way.
    fn typing(&self, window: &Window, cx: &App) -> bool {
        self.filter_input.focus_handle(cx).is_focused(window)
            || self.path_input.focus_handle(cx).is_focused(window)
    }

    // ----- filter / search -------------------------------------------------

    const SEARCH_LIMIT: usize = 5000;
    const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);

    fn recursive_search_active(&self) -> bool {
        self.state
            .filter
            .as_ref()
            .is_some_and(|f| f.recursive && !f.matcher().is_all())
    }

    /// Ctrl+F / the magnifier: show the bar and put the caret into it.
    fn open_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.filter.is_none() {
            self.state.filter = Some(Filter::default());
        }
        self.filter_input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        cx.notify();
    }

    /// Esc / the close button: hide the bar and show the plain listing again.
    fn close_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let was_recursive = self.recursive_search_active();
        self.drop_filter(cx);
        self.filter_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        if was_recursive {
            self.refresh(None, cx);
        } else {
            self.state.refilter(None);
        }
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Forget the filter state without reloading (navigation does its own load).
    fn drop_filter(&mut self, cx: &mut Context<Self>) {
        self.cancel_search();
        if self.state.filter.take().is_some() {
            self.state.search_truncated = false;
            cx.notify();
        }
    }

    fn on_filter_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.filter_input.read(cx).value().to_string();
        let Some(filter) = self.state.filter.as_mut() else {
            return;
        };
        if filter.query == query {
            return;
        }
        filter.query = query;
        if filter.recursive {
            self.schedule_search(cx);
        } else {
            self.state.refilter(None);
            self.scroll_to_cursor();
            cx.notify();
        }
    }

    fn toggle_recursive(&mut self, cx: &mut Context<Self>) {
        let Some(filter) = self.state.filter.as_mut() else {
            return;
        };
        filter.recursive = !filter.recursive;
        if filter.recursive {
            self.schedule_search(cx);
        } else {
            self.cancel_search();
            // Back to the plain listing with the live filter applied.
            let path = self.state.path.clone();
            self.load(path, None, cx);
        }
        cx.notify();
    }

    fn cancel_search(&mut self) {
        if let Some(cancel) = self.search_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.search_debounce = None;
        self.searching = false;
    }

    /// Run the recursive search shortly after the user stops typing.
    fn schedule_search(&mut self, cx: &mut Context<Self>) {
        self.cancel_search();
        if !self.recursive_search_active() {
            // Empty query: show the plain listing.
            let path = self.state.path.clone();
            self.load(path, None, cx);
            return;
        }
        self.search_debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Self::SEARCH_DEBOUNCE).await;
            this.update(cx, |panel, cx| panel.start_search(cx)).ok();
        }));
    }

    fn start_search(&mut self, cx: &mut Context<Self>) {
        self.cancel_search();
        let Some(filter) = self.state.filter.clone() else {
            return;
        };
        if platform::is_roots_path(&self.state.path) {
            return;
        }
        self.generation += 1;
        let generation = self.generation;
        let cancel = Arc::new(AtomicBool::new(false));
        self.search_cancel = Some(cancel.clone());
        self.searching = true;
        let root = self.state.path.clone();
        let opts = ReadOptions {
            show_hidden: self.state.show_hidden,
            sort: self.state.sort,
        };
        let matcher = filter.matcher();
        let task = cx.background_spawn({
            let root = root.clone();
            async move { search_tree(&root, opts, &matcher, Self::SEARCH_LIMIT, &cancel) }
        });
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let (results, truncated) = task.await;
            this.update(cx, |panel, cx| {
                if panel.generation != generation {
                    return;
                }
                panel.searching = false;
                panel.search_cancel = None;
                let mut entries = Vec::with_capacity(results.len() + 1);
                if let Some(parent) = platform::parent_of(&root) {
                    entries.push(parent_entry(parent));
                }
                entries.extend(results);
                panel.state.search_truncated = truncated;
                panel.state.set_entries(root, entries, None);
                panel.scroll_to_cursor();
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn on_open_filter(&mut self, _: &OpenFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.open_filter(window, cx);
    }

    fn on_close_filter(&mut self, _: &CloseFilter, window: &mut Window, cx: &mut Context<Self>) {
        self.close_filter(window, cx);
    }

    /// Ctrl+Shift+F: open the bar if needed and flip the Subfolders toggle.
    fn on_toggle_filter_recursive(
        &mut self,
        _: &ToggleFilterRecursive,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.filter.is_none() {
            self.open_filter(window, cx);
        }
        self.toggle_recursive(cx);
    }

    /// The bar under the header: query input, Subfolders toggle, counts, close.
    fn render_filter_bar(&self, c: &Colors, cx: &mut Context<Self>) -> Option<AnyElement> {
        let filter = self.state.filter.as_ref()?;
        let shown = self.state.item_count();
        let total = self
            .state
            .all_entries
            .iter()
            .filter(|e| !e.is_parent)
            .count();
        let count = if filter.recursive {
            format!(
                "{shown} results{}{}",
                if self.state.search_truncated {
                    " (limit)"
                } else {
                    ""
                },
                if self.searching {
                    " · searching…"
                } else {
                    ""
                }
            )
        } else {
            format!("{shown} / {total}")
        };
        Some(
            h_flex()
                .key_context(PANEL_FILTER_CONTEXT)
                .on_action(cx.listener(Self::on_close_filter))
                .h(rems(1.75))
                .flex_none()
                .px_2()
                .gap_2()
                .bg(c.list_head)
                .border_b_1()
                .border_color(c.border)
                .child(
                    div().flex_1().min_w_0().child(
                        Input::new(&self.filter_input)
                            .small()
                            .focus_bordered(false)
                            .h(rems(1.375)),
                    ),
                )
                .child(
                    Button::new(("filter-recursive", self.index))
                        .xsmall()
                        .label("Subfolders")
                        .selected(filter.recursive)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_recursive(cx))),
                )
                .child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(c.muted_foreground)
                        .whitespace_nowrap()
                        .child(count),
                )
                .child(
                    Button::new(("filter-close", self.index))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .on_click(cx.listener(|this, _, window, cx| this.close_filter(window, cx))),
                )
                .into_any_element(),
        )
    }

    // ----- path bar ------------------------------------------------------

    /// The path as clickable segments; a click in the space after the last
    /// segment switches to the text editor (same as Ctrl+L).
    fn render_breadcrumbs(&self, active: bool, c: &Colors, cx: &mut Context<Self>) -> AnyElement {
        use std::path::Component;

        let path = &self.state.path;
        let mut crumbs: Vec<(String, PathBuf)> = Vec::new();
        if platform::is_roots_path(path) {
            crumbs.push((platform::display_name(path), path.clone()));
        } else {
            let mut acc = PathBuf::new();
            for component in path.components() {
                acc.push(component.as_os_str());
                let label = match component {
                    Component::RootDir => "/".to_string(),
                    other => other.as_os_str().to_string_lossy().into_owned(),
                };
                crumbs.push((label, acc.clone()));
            }
        }
        let last = crumbs.len().saturating_sub(1);
        let (hover, fg, muted) = (c.list_hover, c.foreground, c.muted_foreground);

        h_flex()
            .id(("crumbs", self.index))
            .flex_1()
            .min_w_0()
            .h_full()
            .text_sm()
            .overflow_x_hidden()
            .children(
                crumbs
                    .into_iter()
                    .enumerate()
                    .flat_map(|(i, (label, target))| {
                        let is_last = i == last;
                        let crumb = div()
                            .id(("crumb", i))
                            .flex_none()
                            .px_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .text_color(if is_last && active { fg } else { muted })
                            .hover(|s| s.bg(hover))
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                if target != this.state.path {
                                    this.navigate_to(target.clone(), cx);
                                }
                            }))
                            .child(label);
                        let separator = (!is_last && i > 0)
                            .then(|| div().flex_none().text_color(muted).child("›"));
                        std::iter::once(crumb.into_any_element())
                            .chain(separator.map(|s| s.into_any_element()))
                    }),
            )
            // The empty space after the path: click to type or copy the path.
            .child(
                div()
                    .id(("crumbs-edit", self.index))
                    .flex_1()
                    .min_w(rems(1.))
                    .h_full()
                    .cursor_text()
                    .on_click(cx.listener(|this, _, window, cx| this.edit_path(window, cx))),
            )
            .into_any_element()
    }

    // ----- rubber band selection ------------------------------------------

    fn row_height_rems(&self) -> f32 {
        match self.view {
            ViewMode::Full | ViewMode::Brief => ROW_HEIGHT_REMS,
            ViewMode::Thumbnails => THUMB_ROW_REMS,
        }
    }

    fn row_height(&self, window: &Window) -> Pixels {
        window.rem_size() * self.row_height_rems()
    }

    /// Width of one cell in a grid view; `None` means a full-width row.
    fn cell_width_rems(&self) -> Option<f32> {
        match self.view {
            ViewMode::Full => None,
            ViewMode::Brief => Some(BRIEF_CELL_REMS),
            ViewMode::Thumbnails => Some(THUMB_CELL_REMS),
        }
    }

    /// How many entries fit in one line at the body's current width.
    fn cells_per_row(&self, window: &Window) -> usize {
        let Some(cell) = self.cell_width_rems() else {
            return 1;
        };
        let width = self.scroll.0.borrow().base_handle.bounds().size.width;
        let cell_px = window.rem_size() * cell;
        if width <= px(0.) || cell_px <= px(0.) {
            1
        } else {
            ((width / cell_px).floor() as usize).max(1)
        }
    }

    fn line_count(&self) -> usize {
        self.state.len().div_ceil(self.per_row.max(1))
    }

    /// Geometry of the list: its bounds and the current scroll offset.
    fn list_geometry(&self) -> (Bounds<Pixels>, Point<Pixels>) {
        let state = self.scroll.0.borrow();
        (state.base_handle.bounds(), state.base_handle.offset())
    }

    /// Index of the entry at window position `pos`, if any.
    fn index_at(&self, pos: Point<Pixels>, window: &Window) -> Option<usize> {
        let (bounds, offset) = self.list_geometry();
        let row_h = self.row_height(window);
        if pos.y < bounds.origin.y || row_h <= px(0.) {
            return None;
        }
        let line = ((pos.y - bounds.origin.y - offset.y) / row_h).floor() as usize;
        let col = match self.cell_width_rems() {
            None => 0,
            Some(w) => {
                let cell = window.rem_size() * w;
                let col = ((pos.x - bounds.origin.x) / cell).floor();
                if col < 0.0 || col as usize >= self.per_row {
                    return None;
                }
                col as usize
            }
        };
        let ix = line * self.per_row.max(1) + col;
        (ix < self.state.len()).then_some(ix)
    }

    /// A press in the body: remember it for a possible rubber band; a plain
    /// press below the last row also clears the marks.
    fn on_body_mouse_down(
        &mut self,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);
        let row = self.index_at(ev.position, window);
        // Marked rows and the cursor row are dragged as files, not rubber-banded.
        let on_marked = row.is_some_and(|ix| self.state.marked.contains(&ix))
            || (row.is_some() && self.press_on_cursor);
        self.press_on_cursor = false;
        self.pending_press = Some((ev.position, ev.modifiers, on_marked));
        let plain = !(ev.modifiers.control || ev.modifiers.shift || ev.modifiers.platform);
        if plain && row.is_none() && !self.state.marked.is_empty() {
            self.state.clear_marks();
        }
        // Re-render before the first mouse move so the body only offers the
        // rubber band drag when the press was not on a marked row.
        cx.notify();
    }

    /// The rubber band drag moved: grow the rectangle, mark the rows it
    /// crosses, and scroll when the mouse is beyond the list's edge.
    fn on_marquee_move(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.marquee.is_none() {
            let Some((start, modifiers, on_marked)) = self.pending_press.take() else {
                return;
            };
            if on_marked {
                return;
            }
            let additive = modifiers.control || modifiers.shift || modifiers.platform;
            let base = if additive {
                self.state.marked.clone()
            } else {
                Default::default()
            };
            self.marquee = Some(Marquee {
                start,
                current: start,
                base,
            });
        }

        let (bounds, offset) = self.list_geometry();
        let row_h = self.row_height(window);
        // Autoscroll when dragging past the top or bottom edge.
        let edge = px(8.);
        let mut new_offset = offset;
        if position.y < bounds.origin.y + edge {
            new_offset.y = (offset.y + row_h).min(px(0.));
        } else if position.y > bounds.bottom_right().y - edge {
            let max = self.scroll.0.borrow().base_handle.max_offset();
            new_offset.y = (offset.y - row_h).max(-max.y);
        }
        if new_offset != offset {
            self.scroll.0.borrow().base_handle.set_offset(new_offset);
        }
        let offset = new_offset;

        let (start, base) = {
            let Some(marquee) = self.marquee.as_mut() else {
                return;
            };
            marquee.current = position;
            (marquee.start, marquee.base.clone())
        };
        let (top, bottom) = if start.y <= position.y {
            (start.y, position.y)
        } else {
            (position.y, start.y)
        };
        let (left, right) = if start.x <= position.x {
            (start.x, position.x)
        } else {
            (position.x, start.x)
        };
        // Lines whose vertical span intersects the rectangle, and in a grid
        // the columns whose horizontal span does.
        let list_top = bounds.origin.y + offset.y;
        let first_line = ((top - list_top) / row_h).floor().max(0.0) as usize;
        let last_line = ((bottom - list_top) / row_h).floor().max(0.0) as usize;
        let per_row = self.per_row.max(1);
        let cols = match self.cell_width_rems() {
            None => 0..=0,
            Some(w) => {
                let cell = window.rem_size() * w;
                let first = ((left - bounds.origin.x) / cell).floor().max(0.0) as usize;
                let last = ((right - bounds.origin.x) / cell).floor().max(0.0) as usize;
                first.min(per_row - 1)..=last.min(per_row - 1)
            }
        };
        let len = self.state.len();
        let indices = (first_line..=last_line)
            .flat_map(|line| cols.clone().map(move |col| line * per_row + col))
            .filter(|&ix| ix < len);
        self.state.mark_rows(indices, &base);
        let clamped = Point::new(
            position
                .x
                .clamp(bounds.origin.x, bounds.bottom_right().x - px(1.)),
            position
                .y
                .clamp(bounds.origin.y, bounds.bottom_right().y - px(1.)),
        );
        if let Some(ix) = self.index_at(clamped, window) {
            self.state.cursor = ix;
        }
        cx.notify();
    }

    fn end_marquee(&mut self, cx: &mut Context<Self>) {
        self.pending_press = None;
        if self.marquee.take().is_some() {
            cx.notify();
        }
    }

    /// The translucent rectangle drawn over the list while selecting.
    fn render_marquee(&self, color: Hsla) -> Option<AnyElement> {
        let marquee = self.marquee.as_ref()?;
        let (bounds, _) = self.list_geometry();
        let (a, b) = (marquee.start, marquee.current);
        let left = a.x.min(b.x) - bounds.origin.x;
        let top = a.y.min(b.y) - bounds.origin.y;
        let width = (a.x - b.x).abs();
        let height = (a.y - b.y).abs();
        Some(
            div()
                .absolute()
                .left(left)
                .top(top)
                .w(width)
                .h(height)
                .bg(color.alpha(0.15))
                .border_1()
                .border_color(color)
                .into_any_element(),
        )
    }

    // ----- tree view -----------------------------------------------------

    fn toggle_expand(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(path) = self.state.toggle_expanded(ix) {
            let dir = self.state.path.clone();
            self.load(dir, Some(path), cx);
        }
    }

    fn on_expand(&mut self, _: &ExpandEntry, window: &mut Window, cx: &mut Context<Self>) {
        if self.typing(window, cx) {
            return;
        }
        if self.view.is_grid() {
            self.state.move_cursor_by(1);
            self.after_cursor_move(cx);
            return;
        }
        let ix = self.state.cursor;
        if !self.state.is_expanded(ix) {
            self.toggle_expand(ix, cx);
        }
    }

    /// Collapse the expanded directory under the cursor, or jump to the parent row.
    fn on_collapse(&mut self, _: &CollapseEntry, window: &mut Window, cx: &mut Context<Self>) {
        if self.typing(window, cx) {
            return;
        }
        if self.view.is_grid() {
            self.state.move_cursor_by(-1);
            self.after_cursor_move(cx);
            return;
        }
        let ix = self.state.cursor;
        if self.state.is_expanded(ix) {
            self.toggle_expand(ix, cx);
        } else if let Some(parent) = self.state.parent_index(ix) {
            self.state.move_cursor_to(parent);
            self.after_cursor_move(cx);
        }
    }

    // ----- path editing --------------------------------------------------

    fn edit_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing_path = true;
        let text = self.state.path.to_string_lossy().into_owned();
        self.path_input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        cx.notify();
    }

    fn confirm_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.path_input.read(cx).value().to_string();
        self.editing_path = false;
        let expanded = expand_path(raw.trim());
        if expanded.is_dir() {
            self.navigate(expanded, None, cx);
        } else {
            self.state.error = Some(format!("{}: not a directory", expanded.display()));
        }
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing_path = false;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    // ----- cursor helpers ------------------------------------------------

    /// Entries one page up or down: the visible lines times the entries per line.
    fn page_size(&self, window: &Window) -> isize {
        let height = self.scroll.0.borrow().base_handle.bounds().size.height;
        let row = self.row_height(window);
        let lines = ((height / row).floor() as isize).max(1);
        lines * self.per_row.max(1) as isize
    }

    /// Entries one line up or down (one in the full view).
    fn line_step(&self) -> isize {
        self.per_row.max(1) as isize
    }

    fn scroll_to_cursor(&self) {
        let line = self.state.cursor / self.per_row.max(1);
        self.scroll.scroll_to_item(line, ScrollStrategy::Nearest);
    }

    fn after_cursor_move(&mut self, cx: &mut Context<Self>) {
        self.scroll_to_cursor();
        cx.notify();
    }

    // ----- action handlers ----------------------------------------------

    fn on_cursor_up(&mut self, _: &CursorUp, _: &mut Window, cx: &mut Context<Self>) {
        self.state.move_cursor_by(-self.line_step());
        self.after_cursor_move(cx);
    }
    fn on_cursor_down(&mut self, _: &CursorDown, _: &mut Window, cx: &mut Context<Self>) {
        self.state.move_cursor_by(self.line_step());
        self.after_cursor_move(cx);
    }
    fn on_cursor_page_up(&mut self, _: &CursorPageUp, window: &mut Window, cx: &mut Context<Self>) {
        self.state.move_cursor_by(-self.page_size(window));
        self.after_cursor_move(cx);
    }
    fn on_cursor_page_down(
        &mut self,
        _: &CursorPageDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.move_cursor_by(self.page_size(window));
        self.after_cursor_move(cx);
    }
    fn on_cursor_home(&mut self, _: &CursorHome, _: &mut Window, cx: &mut Context<Self>) {
        self.state.move_cursor_to(0);
        self.after_cursor_move(cx);
    }
    fn on_cursor_end(&mut self, _: &CursorEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.state.move_cursor_to(self.state.last_index());
        self.after_cursor_move(cx);
    }
    fn on_extend_up(&mut self, _: &ExtendUp, _: &mut Window, cx: &mut Context<Self>) {
        self.state.extend_by(-self.line_step());
        self.after_cursor_move(cx);
    }
    fn on_extend_down(&mut self, _: &ExtendDown, _: &mut Window, cx: &mut Context<Self>) {
        self.state.extend_by(self.line_step());
        self.after_cursor_move(cx);
    }
    fn on_extend_page_up(&mut self, _: &ExtendPageUp, window: &mut Window, cx: &mut Context<Self>) {
        self.state.extend_by(-self.page_size(window));
        self.after_cursor_move(cx);
    }
    fn on_extend_page_down(
        &mut self,
        _: &ExtendPageDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.extend_by(self.page_size(window));
        self.after_cursor_move(cx);
    }
    fn on_extend_home(&mut self, _: &ExtendHome, _: &mut Window, cx: &mut Context<Self>) {
        self.state.extend_to(0);
        self.after_cursor_move(cx);
    }
    fn on_extend_end(&mut self, _: &ExtendEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.state.extend_to(self.state.last_index());
        self.after_cursor_move(cx);
    }
    fn on_toggle_mark(&mut self, _: &ToggleMark, window: &mut Window, cx: &mut Context<Self>) {
        if self.typing(window, cx) {
            return;
        }
        self.state.toggle_mark(self.state.cursor);
        cx.notify();
    }
    fn on_toggle_mark_and_down(
        &mut self,
        _: &ToggleMarkAndDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.typing(window, cx) {
            return;
        }
        self.state.toggle_mark(self.state.cursor);
        self.state.move_cursor_by(1);
        self.after_cursor_move(cx);
    }
    fn on_mark_all(&mut self, _: &MarkAll, window: &mut Window, cx: &mut Context<Self>) {
        if self.typing(window, cx) {
            return;
        }
        self.state.mark_all();
        cx.notify();
    }
    fn on_invert_marks(&mut self, _: &InvertMarks, window: &mut Window, cx: &mut Context<Self>) {
        if self.typing(window, cx) {
            return;
        }
        self.state.invert_marks();
        cx.notify();
    }
    fn on_cancel_or_clear(
        &mut self,
        _: &CancelOrClear,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing_path {
            self.cancel_edit(window, cx);
        } else if self.state.filter.is_some() {
            self.close_filter(window, cx);
        } else if self.state.error.is_some() {
            self.state.error = None;
            cx.notify();
        } else if !self.state.marked.is_empty() {
            self.state.clear_marks();
            cx.notify();
        } else {
            cx.emit(PanelEvent::CancelRequested);
        }
    }
    fn on_open(&mut self, _: &Open, window: &mut Window, cx: &mut Context<Self>) {
        if self.typing(window, cx) {
            return;
        }
        self.open(window, cx);
    }
    fn on_go_up(&mut self, _: &GoUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.typing(window, cx) {
            return;
        }
        self.go_up(cx);
    }
    fn on_go_back(&mut self, _: &GoBack, _: &mut Window, cx: &mut Context<Self>) {
        self.go_back(cx);
    }
    fn on_go_forward(&mut self, _: &GoForward, _: &mut Window, cx: &mut Context<Self>) {
        self.go_forward(cx);
    }
    fn on_edit_path(&mut self, _: &EditPath, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_path {
            self.cancel_edit(window, cx);
        } else {
            self.edit_path(window, cx);
        }
    }
    fn on_toggle_hidden(&mut self, _: &ToggleHidden, _: &mut Window, cx: &mut Context<Self>) {
        self.state.show_hidden = !self.state.show_hidden;
        self.reload(cx);
        cx.emit(PanelEvent::Changed);
    }
    fn on_reload(&mut self, _: &Reload, _: &mut Window, cx: &mut Context<Self>) {
        self.reload(cx);
    }
    fn on_sort_by(&mut self, action: &SortBy, _: &mut Window, cx: &mut Context<Self>) {
        self.sort_by(action.key, cx);
    }
    /// Sort by `key` (the same key again flips the direction) and re-read.
    fn sort_by(&mut self, key: SortKey, cx: &mut Context<Self>) {
        self.state.sort = self.state.sort.toggled(key);
        self.refresh(None, cx);
        cx.emit(PanelEvent::Changed);
    }
    /// Copy the selected paths (or the directory) to the clipboard, one per line.
    fn on_copy_path(&mut self, _: &CopyPath, _: &mut Window, cx: &mut Context<Self>) {
        let mut paths = self.selected_paths();
        if paths.is_empty() {
            paths.push(self.state.path.clone());
        }
        let text = paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
    fn on_open_terminal(&mut self, _: &OpenTerminal, _: &mut Window, cx: &mut Context<Self>) {
        self.open_terminal(cx);
    }

    pub fn open_terminal(&mut self, cx: &mut Context<Self>) {
        if platform::is_roots_path(&self.state.path) {
            return;
        }
        if let Err(err) = platform::open_terminal(&self.state.path) {
            self.state.error = Some(format!("terminal: {err}"));
            cx.notify();
        }
    }

    /// Right-click menu for an entry; actions go to this panel's focus handle.
    fn entry_menu(menu: PopupMenu, focus: FocusHandle, is_parent: bool, is_dir: bool) -> PopupMenu {
        let menu = menu.action_context(focus);
        if is_parent {
            return menu
                .menu("Go up", Box::new(GoUp))
                .separator()
                .menu("New folder…", Box::new(MakeDir))
                .menu("Open terminal here", Box::new(OpenTerminal));
        }
        menu.menu(
            if is_dir {
                "Open"
            } else {
                "Open with default app"
            },
            Box::new(Open),
        )
        .separator()
        .menu("Copy to other panel…", Box::new(CopyFiles))
        .menu("Move to other panel…", Box::new(MoveFiles))
        .menu("Rename…", Box::new(RenameFile))
        .separator()
        .menu("Copy", Box::new(CopyToClipboard))
        .menu("Cut", Box::new(CutToClipboard))
        .menu("Paste", Box::new(PasteFromClipboard))
        .separator()
        .menu("Toggle mark", Box::new(ToggleMark))
        .menu("Mark all", Box::new(MarkAll))
        .separator()
        .menu("Move to trash", Box::new(DeleteFiles))
        .menu("Delete permanently", Box::new(DeleteFilesPermanently))
        .separator()
        .menu("Copy path", Box::new(CopyPath))
        .menu("New folder…", Box::new(MakeDir))
        .menu("Open terminal here", Box::new(OpenTerminal))
    }

    /// Right-click menu for the empty part of the panel.
    fn panel_menu(menu: PopupMenu, focus: FocusHandle, sort: Sort, view: ViewMode) -> PopupMenu {
        let menu = menu
            .action_context(focus)
            .menu("Paste", Box::new(PasteFromClipboard))
            .separator()
            .menu("New folder…", Box::new(MakeDir))
            .menu("Open terminal here", Box::new(OpenTerminal))
            .menu("Copy directory path", Box::new(CopyPath))
            .separator()
            .menu("Mark all", Box::new(MarkAll))
            .menu("Clear marks", Box::new(CancelOrClear))
            .separator()
            .menu("Filter…", Box::new(OpenFilter))
            .menu("Edit path…", Box::new(EditPath))
            .menu("Toggle hidden files", Box::new(ToggleHidden))
            .menu("Reload", Box::new(Reload))
            .separator();
        let menu = ViewMode::ALL.iter().fold(menu, |menu, &mode| {
            menu.menu_with_check(
                format!("{} view", mode.label()),
                mode == view,
                Box::new(SetView { mode }),
            )
        });
        SortKey::ALL
            .iter()
            .fold(menu.separator(), |menu, &key| {
                menu.menu_with_check(
                    sort_label(sort, key),
                    sort.key == key,
                    Box::new(SortBy { key }),
                )
            })
            .separator()
            .menu("Go up", Box::new(GoUp))
            .menu("Back", Box::new(GoBack))
            .menu("Forward", Box::new(GoForward))
    }

    /// Right-click menu of the column header: which columns to show.
    fn columns_menu(menu: PopupMenu, focus: FocusHandle, columns: Columns) -> PopupMenu {
        Column::ALL.iter().filter(|c| c.available()).fold(
            menu.action_context(focus),
            |menu, &column| {
                menu.menu_with_check(
                    column.label(),
                    columns.get(column),
                    Box::new(ToggleColumn { column }),
                )
            },
        )
    }

    // ----- rendering -----------------------------------------------------

    /// A column header: click sorts by that column (when it can), a second
    /// click flips the direction. The active column is highlighted and shows
    /// an arrow. `None` is the name column, which fills the rest.
    fn render_column_head(
        &self,
        column: Option<Column>,
        c: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sort = self.state.sort;
        let (label, key, width, right) = match column {
            None => ("Name", Some(SortKey::Name), None, false),
            Some(col) => (
                col.label(),
                col.sort_key(),
                Some(col.width_rems()),
                col.right_aligned(),
            ),
        };
        let active = key.is_some() && key == Some(sort.key);
        let id = column.map(|c| c as usize + 1).unwrap_or(0);
        let mut cell = h_flex()
            .id(("column-head", id))
            .h_full()
            .items_center()
            .gap_0p5()
            .when_some(key, |cell, key| {
                cell.cursor_pointer()
                    .hover(|s| s.text_color(c.foreground))
                    .on_click(cx.listener(move |this, _, _, cx| this.sort_by(key, cx)))
            });
        cell = match width {
            Some(w) => cell.w(rems(w)).flex_none(),
            None => cell.flex_1().min_w_0(),
        };
        if right {
            cell = cell.justify_end();
        }
        if active {
            cell = cell.text_color(c.foreground);
        }
        let arrow = active.then(|| {
            Icon::new(match sort.dir {
                SortDir::Asc => IconName::ChevronUp,
                SortDir::Desc => IconName::ChevronDown,
            })
            .size_3()
            .flex_none()
        });
        // Right-aligned columns put the arrow before the label so the label
        // stays flush with the numbers below it.
        if right {
            cell.children(arrow).child(label).into_any_element()
        } else {
            cell.child(label).children(arrow).into_any_element()
        }
    }

    /// Colours of an entry's row or cell: the cursor gets a strong tint and a
    /// 1px outline in the accent colour, marked entries a yellow tint and
    /// yellow text; rows alternate a faint stripe; cut entries are dimmed.
    fn entry_look(
        &self,
        ix: usize,
        entry: &Entry,
        active: bool,
        zebra: bool,
        c: &Colors,
        cx: &App,
    ) -> EntryLook {
        let is_cursor = ix == self.state.cursor;
        let is_marked = self.state.marked.contains(&ix);
        let is_cut = cx.global::<FileClipboard>().is_cut(&entry.path);
        let (bg, outline) = if is_cursor && active {
            (c.primary.alpha(0.28), c.list_active_border)
        } else if is_cursor {
            (
                c.muted_foreground.alpha(0.18),
                c.muted_foreground.alpha(0.5),
            )
        } else if is_marked {
            (c.yellow.alpha(0.16), transparent_black())
        } else if zebra && ix % 2 == 1 {
            (c.list_even, transparent_black())
        } else {
            (transparent_black(), transparent_black())
        };
        let fg = if is_marked { c.yellow } else { c.foreground };
        let fg = if is_cut { fg.alpha(0.5) } else { fg };
        EntryLook {
            is_cursor,
            is_marked,
            is_cut,
            bg,
            outline,
            fg,
        }
    }

    /// The entry's icon, filling its parent: the desktop's icon for files
    /// when wanted and available, else a generic one (folders always).
    fn entry_icon(&self, entry: &Entry, look: &EntryLook, c: &Colors, cx: &App) -> AnyElement {
        let system_icon = if entry.is_parent
            || entry.is_dir()
            || crate::settings::get(cx).file_icons != FileIcons::System
        {
            None
        } else {
            crate::icons::file_icon(cx, &entry.name)
        };
        match system_icon {
            Some(source) => img(source)
                .size_full()
                .object_fit(ObjectFit::Contain)
                .when(look.is_cut, |i| i.opacity(0.5))
                .into_any_element(),
            None => {
                let icon = if entry.is_parent {
                    Icon::new(IconName::ArrowUp).text_color(c.muted_foreground)
                } else if look.is_marked {
                    Icon::new(if entry.is_dir() {
                        IconName::Folder
                    } else {
                        IconName::File
                    })
                    .text_color(c.yellow)
                } else if entry.is_dir() {
                    Icon::new(IconName::Folder).text_color(c.blue)
                } else {
                    Icon::new(IconName::File).text_color(c.muted_foreground)
                };
                icon.size_full()
                    .when(look.is_cut, |i| i.opacity(0.5))
                    .into_any_element()
            }
        }
    }

    /// Mouse behaviour shared by rows and cells: click / Shift / Ctrl
    /// selection, right-click, dragging marked entries, dropping onto
    /// directories, and the entry's context menu.
    fn wire_entry(
        &self,
        el: Stateful<Div>,
        ix: usize,
        entry: &Entry,
        look: &EntryLook,
        c: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let drag_paths: Vec<PathBuf> = if look.is_marked {
            self.selected_paths()
        } else {
            vec![entry.path.clone()]
        };
        let drop_bg = c.primary.alpha(0.35);
        el.on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                this.focus_handle.focus(window, cx);
                this.press_on_cursor = ix == this.state.cursor;
                if ev.modifiers.shift {
                    // Shift+click marks the range from the cursor to this entry.
                    this.state.extend_to(ix);
                } else if ev.modifiers.control || ev.modifiers.platform {
                    // Ctrl/Cmd+click toggles the mark and moves the cursor.
                    this.state.toggle_mark(ix);
                    this.state.move_cursor_to(ix);
                } else {
                    this.state.move_cursor_to(ix);
                    if ev.click_count >= 2 {
                        this.open(window, cx);
                    }
                }
                cx.notify();
            }),
        )
        // Right-click acts on the clicked entry: move the cursor there first.
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                this.focus_handle.focus(window, cx);
                if !this.state.marked.contains(&ix) {
                    this.state.move_cursor_to(ix);
                }
                cx.notify();
            }),
        )
        // Dragging a marked entry drags all marked entries, the cursor entry
        // drags itself; others bubble to the body and start a rubber band.
        .when(look.is_marked || look.is_cursor, |d| {
            let drag = DragEntries { paths: drag_paths };
            d.on_drag(drag, |drag, _offset, _window, cx| {
                let label = crate::ops::describe(&drag.paths);
                cx.new(|_| DragPreview { label })
            })
        })
        // Directories (and `..`) accept drops into them.
        .when(entry.is_dir(), |d| {
            let dest = entry.path.clone();
            d.drag_over::<DragEntries>(move |style, _, _, _| style.bg(drop_bg))
                .on_drop(cx.listener(move |_this, drag: &DragEntries, _window, cx| {
                    cx.stop_propagation();
                    cx.emit(PanelEvent::Dropped {
                        paths: drag.paths.clone(),
                        dest: dest.clone(),
                    });
                }))
        })
        .context_menu({
            let focus = self.focus_handle.clone();
            let (is_parent, is_dir) = (entry.is_parent, entry.is_dir());
            move |menu, _, _| Self::entry_menu(menu, focus.clone(), is_parent, is_dir)
        })
        .into_any_element()
    }

    /// One line of the list: a full row, or a line of grid cells.
    fn render_line(
        &self,
        line: usize,
        active: bool,
        c: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.view.is_grid() {
            return self.render_row(line, active, c, cx);
        }
        let per_row = self.per_row.max(1);
        let start = line * per_row;
        let end = ((line + 1) * per_row).min(self.state.len());
        h_flex()
            .w_full()
            .h(rems(self.row_height_rems()))
            .items_start()
            .children((start..end).map(|ix| self.render_cell(ix, active, c, cx)))
            .into_any_element()
    }

    /// A cell of the brief or thumbnail view.
    fn render_cell(
        &self,
        ix: usize,
        active: bool,
        c: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(entry) = self.state.entries.get(ix) else {
            return div().into_any_element();
        };
        let look = self.entry_look(ix, entry, active, false, c, cx);
        let icon = self.entry_icon(entry, &look, c, cx);
        let name = if entry.is_symlink() {
            format!("{} →", entry.name)
        } else {
            entry.name.clone()
        };
        let cell = match self.view {
            ViewMode::Thumbnails => v_flex()
                .id(("cell", ix))
                .w(rems(THUMB_CELL_REMS))
                .h(rems(THUMB_ROW_REMS))
                .flex_none()
                .p_1()
                .gap_1()
                .items_center()
                .child(
                    div()
                        .size(rems(THUMB_IMAGE_REMS))
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(div().size(rems(3.)).child(icon)),
                )
                .child(
                    div()
                        .w_full()
                        .text_xs()
                        .text_center()
                        .truncate()
                        .child(name),
                ),
            _ => h_flex()
                .id(("cell", ix))
                .w(rems(BRIEF_CELL_REMS))
                .h(rems(ROW_HEIGHT_REMS))
                .flex_none()
                .px_1()
                .gap_1()
                .text_sm()
                .child(div().size_4().flex_none().child(icon))
                .child(div().flex_1().min_w_0().truncate().child(name)),
        };
        let cell = cell
            .bg(look.bg)
            .text_color(look.fg)
            .whitespace_nowrap()
            .border_1()
            .border_color(look.outline)
            .rounded_sm()
            .when(look.is_marked, |d| d.font_weight(FontWeight::BOLD));
        self.wire_entry(cell, ix, entry, &look, c, cx)
    }

    /// A row of the full view.
    fn render_row(
        &self,
        ix: usize,
        active: bool,
        c: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(entry) = self.state.entries.get(ix) else {
            return div().h(rems(ROW_HEIGHT_REMS)).into_any_element();
        };
        let look = self.entry_look(ix, entry, active, true, c, cx);
        let icon = self.entry_icon(entry, &look, c, cx);
        let bar = if look.is_marked {
            c.yellow
        } else {
            transparent_black()
        };
        let name = if entry.is_symlink() {
            format!("{} →", entry.name)
        } else {
            entry.name.clone()
        };

        // Directories get a chevron that expands them inline; other rows keep the space.
        let chevron: AnyElement = if !entry.is_parent && entry.is_dir() {
            let expanded = self.state.is_expanded(ix);
            div()
                .id(("chevron", ix))
                .flex_none()
                .w(rems(1.))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_color(c.muted_foreground)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                        this.toggle_expand(ix, cx);
                    }),
                )
                .child(
                    Icon::new(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size_3(),
                )
                .into_any_element()
        } else {
            div().flex_none().w(rems(1.)).into_any_element()
        };

        let row = h_flex()
            .id(("row", ix))
            .h(rems(ROW_HEIGHT_REMS))
            .w_full()
            .pl_1()
            .pr_2()
            .gap_2()
            .bg(look.bg)
            .text_color(look.fg)
            .text_sm()
            .whitespace_nowrap()
            .border_1()
            .border_color(look.outline)
            .when(look.is_marked, |d| d.font_weight(FontWeight::BOLD))
            .child(div().flex_none().w(px(3.)).h_full().bg(bar))
            .child(div().flex_none().w(rems(entry.depth as f32)))
            .child(chevron)
            .child(div().size_4().flex_none().child(icon))
            .child(div().flex_1().min_w_0().truncate().child(name))
            .children(self.columns.shown().map(|column| {
                let (text, color) = column_value(column, entry, &look, c);
                div()
                    .w(rems(column.width_rems()))
                    .flex_none()
                    .truncate()
                    .when(column.right_aligned(), |d| d.text_right())
                    .text_color(color)
                    .child(text)
            }));
        self.wire_entry(row, ix, entry, &look, c, cx)
    }
}

/// Per-entry colours shared by rows and cells.
struct EntryLook {
    is_cursor: bool,
    is_marked: bool,
    is_cut: bool,
    bg: Hsla,
    outline: Hsla,
    fg: Hsla,
}

/// Text and colour of a column for `entry`.
fn column_value(column: Column, entry: &Entry, look: &EntryLook, c: &Colors) -> (String, Hsla) {
    let muted = c.muted_foreground;
    match column {
        Column::Size => {
            let text = if entry.is_parent || entry.is_dir() {
                String::new()
            } else {
                human_size(entry.size)
            };
            (text, look.fg)
        }
        // `<DIR>` for directories, the extension for files (the name keeps it too).
        Column::Kind => {
            if entry.is_parent {
                (String::new(), muted)
            } else if entry.is_dir() {
                ("<DIR>".to_string(), look.fg)
            } else {
                let color = if look.is_marked { look.fg } else { muted };
                (extension(&entry.name).to_string(), color)
            }
        }
        Column::Modified => (entry.modified.map(format_time).unwrap_or_default(), muted),
        Column::Created => (entry.created.map(format_time).unwrap_or_default(), muted),
        Column::Permissions => (
            entry
                .mode
                .filter(|_| !entry.is_parent)
                .map(|mode| format_mode(mode, &entry.kind))
                .unwrap_or_default(),
            muted,
        ),
        Column::Owner => (
            entry
                .uid
                .filter(|_| !entry.is_parent)
                .map(owner_name)
                .unwrap_or_default(),
            muted,
        ),
    }
}

/// Expand `~` and turn user input into a path.
pub fn expand_path(raw: &str) -> PathBuf {
    if raw == "~" {
        crate::config::home_dir()
    } else if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        crate::config::home_dir().join(rest)
    } else if raw.is_empty() {
        platform::default_root()
    } else {
        PathBuf::from(raw)
    }
}

impl Render for FilePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = Colors {
            primary: theme.primary,
            list_active_border: theme.list_active_border,
            list_even: theme.list_even,
            muted_foreground: theme.muted_foreground,
            foreground: theme.foreground,
            list_hover: theme.list_hover,
            list_head: theme.list_head,
            border: theme.border,
            yellow: theme.yellow,
            blue: theme.blue,
        };
        let (bg, border, accent, title_bar, list_head, status_bar, danger) = (
            theme.background,
            theme.border,
            theme.primary,
            theme.title_bar,
            theme.list_head,
            theme.status_bar,
            theme.danger,
        );

        let active = self.focus_handle.contains_focused(window, cx);
        let body_drop_bg = colors.primary.alpha(0.12);
        let marquee_color = colors.primary;
        // A press on a marked row belongs to the file drag, not the rubber band.
        let marquee_allowed = !matches!(self.pending_press, Some((_, _, true)));
        // Grid geometry follows the body width measured at the last frame.
        self.per_row = self.cells_per_row(window);
        if std::mem::take(&mut self.pending_scroll) {
            self.scroll_to_cursor();
        }
        let count = self.line_count();

        let header: AnyElement = if self.editing_path {
            Input::new(&self.path_input)
                .small()
                .focus_bordered(false)
                .h(rems(1.5))
                .into_any_element()
        } else {
            self.render_breadcrumbs(active, &colors, cx)
        };

        let filter_bar = self.render_filter_bar(&colors, cx);
        let footer_text = if self.state.loading || self.searching {
            if self.searching {
                "Searching…"
            } else {
                "Loading…"
            }
            .to_string()
        } else if let Some(err) = &self.state.error {
            err.clone()
        } else if self.state.filter.is_some() {
            let total = self
                .state
                .all_entries
                .iter()
                .filter(|e| !e.is_parent)
                .count();
            format!(
                "{} / {} selected · {} of {} shown",
                self.state.marked.len(),
                self.state.item_count(),
                self.state.item_count(),
                total
            )
        } else {
            format!(
                "{} / {} selected · {}",
                self.state.marked.len(),
                self.state.item_count(),
                human_size(self.state.marked_bytes())
            )
        };
        let footer_color = if self.state.error.is_some() {
            danger
        } else {
            colors.muted_foreground
        };
        let muted_foreground = colors.muted_foreground;

        let row_colors = colors.clone();
        let list = uniform_list(
            ("files", self.index),
            count,
            cx.processor(move |this, range: Range<usize>, _window, cx| {
                range
                    .map(|line| this.render_line(line, active, &row_colors, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0()
        .w_full();

        // The column header belongs to the full view only.
        let column_header = if self.view == ViewMode::Full {
            let heads: Vec<AnyElement> = std::iter::once(None)
                .chain(self.columns.shown().map(Some))
                .map(|column| self.render_column_head(column, &colors, cx))
                .collect();
            Some(
                h_flex()
                    .id(("column-header", self.index))
                    .h(rems(1.375))
                    .flex_none()
                    .px_2()
                    .gap_2()
                    .bg(list_head)
                    .border_b_1()
                    .border_color(border)
                    .text_xs()
                    .whitespace_nowrap()
                    .text_color(muted_foreground)
                    .child(div().w(rems(1.)).flex_none())
                    .child(div().size_4().flex_none())
                    .children(heads)
                    .context_menu({
                        let focus = self.focus_handle.clone();
                        let columns = self.columns;
                        move |menu, _, _| Self::columns_menu(menu, focus.clone(), columns)
                    }),
            )
        } else {
            None
        };

        v_flex()
            .id(("panel", self.index))
            .key_context(PANEL_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_cursor_up))
            .on_action(cx.listener(Self::on_cursor_down))
            .on_action(cx.listener(Self::on_cursor_page_up))
            .on_action(cx.listener(Self::on_cursor_page_down))
            .on_action(cx.listener(Self::on_cursor_home))
            .on_action(cx.listener(Self::on_cursor_end))
            .on_action(cx.listener(Self::on_extend_up))
            .on_action(cx.listener(Self::on_extend_down))
            .on_action(cx.listener(Self::on_extend_page_up))
            .on_action(cx.listener(Self::on_extend_page_down))
            .on_action(cx.listener(Self::on_extend_home))
            .on_action(cx.listener(Self::on_extend_end))
            .on_action(cx.listener(Self::on_toggle_mark))
            .on_action(cx.listener(Self::on_toggle_mark_and_down))
            .on_action(cx.listener(Self::on_mark_all))
            .on_action(cx.listener(Self::on_invert_marks))
            .on_action(cx.listener(Self::on_cancel_or_clear))
            .on_action(cx.listener(Self::on_open))
            .on_action(cx.listener(Self::on_go_up))
            .on_action(cx.listener(Self::on_go_back))
            .on_action(cx.listener(Self::on_go_forward))
            .on_action(cx.listener(Self::on_edit_path))
            .on_action(cx.listener(Self::on_toggle_hidden))
            .on_action(cx.listener(Self::on_reload))
            .on_action(cx.listener(Self::on_sort_by))
            .on_action(cx.listener(Self::on_copy_path))
            .on_action(cx.listener(Self::on_open_terminal))
            .on_action(cx.listener(Self::on_expand))
            .on_action(cx.listener(Self::on_collapse))
            .on_action(cx.listener(Self::on_open_filter))
            .on_action(cx.listener(Self::on_toggle_filter_recursive))
            .on_action(cx.listener(Self::on_set_view))
            .on_action(cx.listener(Self::on_toggle_column))
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(bg)
            // Inset the highlight so the resize dividers (and the window edge)
            // cannot cover it; inactive panels show no extra border.
            .p(px(2.))
            .child(
                v_flex()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .border_1()
                    .border_color(if active || self.highlighted {
                        accent
                    } else {
                        transparent_black()
                    })
                    .child(
                        h_flex()
                            .h(rems(1.875))
                            .flex_none()
                            .px_2()
                            .gap_2()
                            .bg(title_bar)
                            .border_b_1()
                            .border_color(border)
                            .child(
                                h_flex()
                                    .flex_none()
                                    .gap_0p5()
                                    .child(
                                        Button::new(("nav-back", self.index))
                                            .ghost()
                                            .xsmall()
                                            .icon(IconName::ArrowLeft)
                                            .disabled(self.state.history.is_empty())
                                            .on_click(
                                                cx.listener(|this, _, _, cx| this.go_back(cx)),
                                            ),
                                    )
                                    .child(
                                        Button::new(("nav-forward", self.index))
                                            .ghost()
                                            .xsmall()
                                            .icon(IconName::ArrowRight)
                                            .disabled(self.state.forward.is_empty())
                                            .on_click(
                                                cx.listener(|this, _, _, cx| this.go_forward(cx)),
                                            ),
                                    ),
                            )
                            .child(header)
                            .child(
                                Button::new(("search", self.index))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Search)
                                    .selected(self.state.filter.is_some())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if this.state.filter.is_some() {
                                            this.close_filter(window, cx);
                                        } else {
                                            this.open_filter(window, cx);
                                        }
                                    })),
                            )
                            .child(
                                Button::new(("view", self.index))
                                    .ghost()
                                    .xsmall()
                                    .icon(self.view.icon())
                                    .dropdown_menu({
                                        let focus = self.focus_handle.clone();
                                        let current = self.view;
                                        move |menu, _, _| {
                                            ViewMode::ALL.iter().fold(
                                                menu.action_context(focus.clone()),
                                                |menu, &mode| {
                                                    menu.menu_with_check(
                                                        mode.label(),
                                                        mode == current,
                                                        Box::new(SetView { mode }),
                                                    )
                                                },
                                            )
                                        }
                                    }),
                            ),
                    )
                    .children(filter_bar)
                    .children(column_header)
                    .child(
                        div()
                            .id(("panel-body", self.index))
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .flex()
                            .flex_col()
                            .child(list)
                            .child(Scrollbar::vertical(&self.scroll).viewport_from_layout())
                            .children(self.render_marquee(marquee_color))
                            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_body_mouse_down))
                            .when(marquee_allowed, |d| {
                                d.on_drag(MarqueeDrag { panel: self.index }, |_, _, _, cx| {
                                    cx.new(|_| EmptyPreview)
                                })
                            })
                            .on_drag_move(cx.listener(
                                |this, ev: &DragMoveEvent<MarqueeDrag>, window, cx| {
                                    if ev.drag(cx).panel == this.index {
                                        this.on_marquee_move(ev.event.position, window, cx);
                                    }
                                },
                            ))
                            // A plain click (no drag) must not leave a press behind.
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _, _window, _cx| this.pending_press = None),
                            )
                            .on_drop(cx.listener(|this, _: &MarqueeDrag, _window, cx| {
                                this.end_marquee(cx);
                            }))
                            .on_mouse_up_out(
                                MouseButton::Left,
                                cx.listener(|this, _, _window, cx| this.end_marquee(cx)),
                            )
                            .drag_over::<DragEntries>(move |style, _, _, _| style.bg(body_drop_bg))
                            .on_drop(cx.listener(|this, drag: &DragEntries, _window, cx| {
                                cx.emit(PanelEvent::Dropped {
                                    paths: drag.paths.clone(),
                                    dest: this.state.path.clone(),
                                });
                            }))
                            .context_menu({
                                let focus = self.focus_handle.clone();
                                let sort = self.state.sort;
                                let view = self.view;
                                move |menu, _, _| Self::panel_menu(menu, focus.clone(), sort, view)
                            }),
                    )
                    .child(
                        h_flex()
                            .h(rems(1.375))
                            .flex_none()
                            .px_2()
                            .bg(status_bar)
                            .border_t_1()
                            .border_color(border)
                            .text_xs()
                            .text_color(footer_color)
                            .child(div().flex_1().min_w_0().truncate().child(footer_text)),
                    ),
            )
    }
}

/// Menu label like "Sort by size ▾" for the current key.
fn sort_label(sort: Sort, key: SortKey) -> String {
    let label = key.label().to_lowercase();
    if sort.key == key {
        let dir = match sort.dir {
            SortDir::Asc => "ascending",
            SortDir::Desc => "descending",
        };
        format!("Sort by {label} ({dir})")
    } else {
        format!("Sort by {label}")
    }
}
