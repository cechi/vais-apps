//! One file panel: a directory listing with cursor, marks and a path bar.

pub mod state;

use std::ops::Range;
use std::path::PathBuf;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::config::PanelConfig;
use crate::fs::format::{format_time, human_size};
use crate::fs::{ReadOptions, Sort, SortDir, read_roots, read_tree};
use crate::keymap::*;
use crate::platform;
use crate::ui::{h_flex, v_flex};
use state::PanelState;

/// Row height in rems, so the list follows the UI scale.
pub const ROW_HEIGHT_REMS: f32 = 1.5;
const SIZE_WIDTH_REMS: f32 = 5.0;
const MODIFIED_WIDTH_REMS: f32 = 8.5;

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
    generation: u64,
    load_task: Option<Task<()>>,
    /// Last left mouse press in the body: where a rubber band would start, and
    /// whether it landed on a marked row (then the row's file drag must win).
    pending_press: Option<(Point<Pixels>, Modifiers, bool)>,
    /// Set by the row handler: the press landed on the row that already had the cursor.
    press_on_cursor: bool,
    marquee: Option<Marquee>,
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

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.on_focus_in(&focus_handle, window, |_this, _window, cx| {
            cx.emit(PanelEvent::Focused);
            cx.notify();
        }));
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
            generation: 0,
            load_task: None,
            pending_press: None,
            press_on_cursor: false,
            marquee: None,
            _subscriptions: subscriptions,
        };
        this.load(cfg.path, None, cx);
        this
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

    /// Name of the entry under the cursor, unless it is `..`.
    pub fn current_name(&self) -> Option<String> {
        self.state
            .current()
            .filter(|e| !e.is_parent)
            .map(|e| e.name.clone())
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
        let path = self.state.path.clone();
        self.load(path, select, cx);
    }

    pub fn config(&self) -> PanelConfig {
        PanelConfig {
            path: self.state.path.clone(),
            show_hidden: self.state.show_hidden,
            sort: self.state.sort.key,
            sort_desc: self.state.sort.dir == SortDir::Desc,
        }
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
        if let Some(prev) = self.state.history.pop() {
            self.load(prev, None, cx);
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.refresh(None, cx);
    }

    // ----- rubber band selection ------------------------------------------

    fn row_height(&self, window: &Window) -> Pixels {
        window.rem_size() * ROW_HEIGHT_REMS
    }

    /// Geometry of the list: its bounds and the current scroll offset.
    fn list_geometry(&self) -> (Bounds<Pixels>, Point<Pixels>) {
        let state = self.scroll.0.borrow();
        (state.base_handle.bounds(), state.base_handle.offset())
    }

    /// Index of the row at window position `y`, if any.
    fn row_at(&self, y: Pixels, window: &Window) -> Option<usize> {
        let (bounds, offset) = self.list_geometry();
        let row_h = self.row_height(window);
        if y < bounds.origin.y || row_h <= px(0.) {
            return None;
        }
        let ix = ((y - bounds.origin.y - offset.y) / row_h).floor() as usize;
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
        let row = self.row_at(ev.position.y, window);
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

        let Some(marquee) = self.marquee.as_mut() else {
            return;
        };
        marquee.current = position;
        let (top, bottom) = if marquee.start.y <= position.y {
            (marquee.start.y, position.y)
        } else {
            (position.y, marquee.start.y)
        };
        // Rows whose vertical span intersects the rectangle.
        let list_top = bounds.origin.y + offset.y;
        let first = ((top - list_top) / row_h).floor().max(0.0) as usize;
        let last = ((bottom - list_top) / row_h).floor().max(0.0) as usize;
        let len = self.state.len();
        let rows = if first < len && len > 0 {
            first..=last.min(len - 1)
        } else {
            1..=0
        };
        let base = marquee.base.clone();
        self.state.mark_rows(rows, &base);
        if let Some(ix) = self.row_at(
            position
                .y
                .clamp(bounds.origin.y, bounds.bottom_right().y - px(1.)),
            window,
        ) {
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

    fn on_expand(&mut self, _: &ExpandEntry, _: &mut Window, cx: &mut Context<Self>) {
        let ix = self.state.cursor;
        if !self.state.is_expanded(ix) {
            self.toggle_expand(ix, cx);
        }
    }

    /// Collapse the expanded directory under the cursor, or jump to the parent row.
    fn on_collapse(&mut self, _: &CollapseEntry, _: &mut Window, cx: &mut Context<Self>) {
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

    fn page_size(&self, window: &Window) -> isize {
        let height = self.scroll.0.borrow().base_handle.bounds().size.height;
        let row = window.rem_size() * ROW_HEIGHT_REMS;
        ((height / row).floor() as isize).max(1)
    }

    fn scroll_to_cursor(&self) {
        self.scroll
            .scroll_to_item(self.state.cursor, ScrollStrategy::Nearest);
    }

    fn after_cursor_move(&mut self, cx: &mut Context<Self>) {
        self.scroll_to_cursor();
        cx.notify();
    }

    // ----- action handlers ----------------------------------------------

    fn on_cursor_up(&mut self, _: &CursorUp, _: &mut Window, cx: &mut Context<Self>) {
        self.state.move_cursor_by(-1);
        self.after_cursor_move(cx);
    }
    fn on_cursor_down(&mut self, _: &CursorDown, _: &mut Window, cx: &mut Context<Self>) {
        self.state.move_cursor_by(1);
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
        self.state.extend_by(-1);
        self.after_cursor_move(cx);
    }
    fn on_extend_down(&mut self, _: &ExtendDown, _: &mut Window, cx: &mut Context<Self>) {
        self.state.extend_by(1);
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
    fn on_toggle_mark(&mut self, _: &ToggleMark, _: &mut Window, cx: &mut Context<Self>) {
        self.state.toggle_mark(self.state.cursor);
        cx.notify();
    }
    fn on_toggle_mark_and_down(
        &mut self,
        _: &ToggleMarkAndDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.toggle_mark(self.state.cursor);
        self.state.move_cursor_by(1);
        self.after_cursor_move(cx);
    }
    fn on_mark_all(&mut self, _: &MarkAll, _: &mut Window, cx: &mut Context<Self>) {
        self.state.mark_all();
        cx.notify();
    }
    fn on_invert_marks(&mut self, _: &InvertMarks, _: &mut Window, cx: &mut Context<Self>) {
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
        self.open(window, cx);
    }
    fn on_go_up(&mut self, _: &GoUp, _: &mut Window, cx: &mut Context<Self>) {
        self.go_up(cx);
    }
    fn on_go_back(&mut self, _: &GoBack, _: &mut Window, cx: &mut Context<Self>) {
        self.go_back(cx);
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
    fn panel_menu(menu: PopupMenu, focus: FocusHandle) -> PopupMenu {
        menu.action_context(focus)
            .menu("New folder…", Box::new(MakeDir))
            .menu("Open terminal here", Box::new(OpenTerminal))
            .menu("Copy directory path", Box::new(CopyPath))
            .separator()
            .menu("Mark all", Box::new(MarkAll))
            .menu("Clear marks", Box::new(CancelOrClear))
            .separator()
            .menu("Edit path…", Box::new(EditPath))
            .menu("Toggle hidden files", Box::new(ToggleHidden))
            .menu("Reload", Box::new(Reload))
            .separator()
            .menu("Go up", Box::new(GoUp))
            .menu("Back", Box::new(GoBack))
    }

    // ----- rendering -----------------------------------------------------

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
        let is_cursor = ix == self.state.cursor;
        let is_marked = self.state.marked.contains(&ix);

        // The cursor row gets a strong tint and a 1px outline in the accent
        // colour; marked rows get a yellow tint, yellow text and a yellow bar.
        let (bg, outline) = if is_cursor && active {
            (c.primary.alpha(0.28), c.list_active_border)
        } else if is_cursor {
            (
                c.muted_foreground.alpha(0.18),
                c.muted_foreground.alpha(0.5),
            )
        } else if is_marked {
            (c.yellow.alpha(0.16), transparent_black())
        } else if ix % 2 == 1 {
            (c.list_even, transparent_black())
        } else {
            (transparent_black(), transparent_black())
        };
        let fg = if is_marked { c.yellow } else { c.foreground };
        let bar = if is_marked {
            c.yellow
        } else {
            transparent_black()
        };

        let icon = if entry.is_parent {
            Icon::new(IconName::ArrowUp).text_color(c.muted_foreground)
        } else if is_marked {
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

        let name = if entry.is_symlink() {
            format!("{} →", entry.name)
        } else {
            entry.name.clone()
        };
        let size = if entry.is_parent {
            String::new()
        } else if entry.is_dir() {
            "<DIR>".to_string()
        } else {
            human_size(entry.size)
        };
        let modified = entry.modified.map(format_time).unwrap_or_default();
        let drag_paths: Vec<PathBuf> = if is_marked {
            self.selected_paths()
        } else {
            vec![entry.path.clone()]
        };
        let drop_bg = c.primary.alpha(0.35);

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

        h_flex()
            .id(("row", ix))
            .h(rems(ROW_HEIGHT_REMS))
            .w_full()
            .pl_1()
            .pr_2()
            .gap_2()
            .bg(bg)
            .text_color(fg)
            .text_sm()
            .whitespace_nowrap()
            .border_1()
            .border_color(outline)
            .when(is_marked, |d| d.font_weight(FontWeight::BOLD))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    this.focus_handle.focus(window, cx);
                    this.press_on_cursor = ix == this.state.cursor;
                    if ev.modifiers.shift {
                        // Shift+click marks the range from the cursor to this row.
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
            // Right-click acts on the clicked row: move the cursor there first.
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
            // Dragging a marked row drags all marked entries, the cursor row drags
            // itself; other rows bubble to the body and start a rubber band.
            .when(is_marked || is_cursor, |d| {
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
            .child(div().flex_none().w(px(3.)).h_full().bg(bar))
            .child(div().flex_none().w(rems(entry.depth as f32)))
            .child(chevron)
            .child(icon.size_4().flex_none())
            .child(div().flex_1().min_w_0().truncate().child(name))
            .child(
                div()
                    .w(rems(SIZE_WIDTH_REMS))
                    .flex_none()
                    .text_right()
                    .child(size),
            )
            .child(
                div()
                    .w(rems(MODIFIED_WIDTH_REMS))
                    .flex_none()
                    .text_color(c.muted_foreground)
                    .child(modified),
            )
            .context_menu({
                let focus = self.focus_handle.clone();
                let (is_parent, is_dir) = (entry.is_parent, entry.is_dir());
                move |menu, _, _| Self::entry_menu(menu, focus.clone(), is_parent, is_dir)
            })
            .into_any_element()
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
        let count = self.state.len();

        let path_text = if platform::is_roots_path(&self.state.path) {
            platform::display_name(&self.state.path)
        } else {
            self.state.path.to_string_lossy().into_owned()
        };
        let header: AnyElement = if self.editing_path {
            Input::new(&self.path_input).into_any_element()
        } else {
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_sm()
                .when(active, |d| d.font_weight(FontWeight::SEMIBOLD))
                .child(path_text)
                .into_any_element()
        };

        let footer_text = if self.state.loading {
            "Loading…".to_string()
        } else if let Some(err) = &self.state.error {
            err.clone()
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

        let list = uniform_list(
            ("files", self.index),
            count,
            cx.processor(move |this, range: Range<usize>, _window, cx| {
                range
                    .map(|ix| this.render_row(ix, active, &colors, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0()
        .w_full();

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
            .on_action(cx.listener(Self::on_edit_path))
            .on_action(cx.listener(Self::on_toggle_hidden))
            .on_action(cx.listener(Self::on_reload))
            .on_action(cx.listener(Self::on_copy_path))
            .on_action(cx.listener(Self::on_open_terminal))
            .on_action(cx.listener(Self::on_expand))
            .on_action(cx.listener(Self::on_collapse))
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
                    .border_color(if active { accent } else { transparent_black() })
                    .child(
                        h_flex()
                            .h(rems(1.875))
                            .flex_none()
                            .px_2()
                            .gap_2()
                            .bg(title_bar)
                            .border_b_1()
                            .border_color(border)
                            .child(header),
                    )
                    .child(
                        h_flex()
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
                            .child(div().flex_1().child("Name"))
                            .child(
                                div()
                                    .w(rems(SIZE_WIDTH_REMS))
                                    .flex_none()
                                    .text_right()
                                    .child("Size"),
                            )
                            .child(
                                div()
                                    .w(rems(MODIFIED_WIDTH_REMS))
                                    .flex_none()
                                    .child("Modified"),
                            ),
                    )
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
                                move |menu, _, _| Self::panel_menu(menu, focus.clone())
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
