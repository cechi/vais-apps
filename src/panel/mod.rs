//! One file panel: a directory listing with cursor, marks and a path bar.

pub mod state;

use std::ops::Range;
use std::path::PathBuf;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::config::PanelConfig;
use crate::fs::format::{format_time, human_size};
use crate::fs::{ReadOptions, Sort, SortDir, read_dir_sorted, read_roots};
use crate::keymap::*;
use crate::platform;
use crate::ui::{h_flex, v_flex};
use state::PanelState;

/// Row height in rems, so the list follows the UI scale.
pub const ROW_HEIGHT_REMS: f32 = 1.5;
const SIZE_WIDTH_REMS: f32 = 5.0;
const MODIFIED_WIDTH_REMS: f32 = 8.5;

pub enum PanelEvent {
    /// The panel (or its path editor) received focus.
    Focused,
    /// Something worth persisting changed (path, hidden toggle).
    Changed,
    /// Escape with nothing left to clear: cancel a running operation, if any.
    CancelRequested,
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
    list_active: Hsla,
    list_active_border: Hsla,
    list_even: Hsla,
    selection: Hsla,
    muted: Hsla,
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

    /// Re-read the directory, placing the cursor on `select` (or keeping it by name).
    pub fn refresh(&mut self, select: Option<String>, cx: &mut Context<Self>) {
        let select = select.or_else(|| self.state.current().map(|e| e.name.clone()));
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
    fn load(&mut self, path: PathBuf, select: Option<String>, cx: &mut Context<Self>) {
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
        let task = cx.background_spawn(async move {
            read_dir_sorted(&read_path, opts).map_err(|e| e.to_string())
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

    fn navigate(&mut self, path: PathBuf, select: Option<String>, cx: &mut Context<Self>) {
        if path != self.state.path {
            let current = self.state.path.clone();
            self.state.push_history(current);
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
        let select = if platform::is_roots_path(&parent) {
            Some(self.state.path.to_string_lossy().into_owned())
        } else {
            self.state
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        };
        self.navigate(parent, select, cx);
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        if let Some(prev) = self.state.history.pop() {
            self.load(prev, None, cx);
        }
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let select = self.state.current().map(|e| e.name.clone());
        let path = self.state.path.clone();
        self.load(path, select, cx);
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

        let bg = if is_cursor {
            if active { c.list_active } else { c.muted }
        } else if is_marked {
            c.selection
        } else if ix % 2 == 1 {
            c.list_even
        } else {
            transparent_black()
        };
        let fg = if is_marked { c.yellow } else { c.foreground };

        let icon = if entry.is_parent {
            Icon::new(IconName::ArrowUp).text_color(c.muted_foreground)
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

        h_flex()
            .id(("row", ix))
            .h(rems(ROW_HEIGHT_REMS))
            .w_full()
            .px_2()
            .gap_2()
            .bg(bg)
            .text_color(fg)
            .text_sm()
            .whitespace_nowrap()
            .when(is_cursor && active, |d| {
                d.border_l_2().border_color(c.list_active_border)
            })
            .when(is_dir_bold(entry), |d| d.font_weight(FontWeight::SEMIBOLD))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    this.focus_handle.focus(window, cx);
                    this.state.move_cursor_to(ix);
                    if ev.click_count >= 2 {
                        this.open(window, cx);
                    }
                    cx.notify();
                }),
            )
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
            .into_any_element()
    }
}

fn is_dir_bold(entry: &crate::fs::Entry) -> bool {
    entry.is_dir() && !entry.is_parent
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
            list_active: theme.list_active,
            list_active_border: theme.list_active_border,
            list_even: theme.list_even,
            selection: theme.selection,
            muted: theme.muted,
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
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(bg)
            .border_1()
            .border_color(if active { accent } else { border })
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
            .child(list)
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
            )
    }
}
