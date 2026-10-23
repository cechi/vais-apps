//! An embedded terminal: alacritty_terminal does the emulation and the PTY,
//! [`element::TerminalElement`] paints its grid, this view owns the session,
//! turns key presses into bytes and answers the terminal's own requests.

pub mod colors;
pub mod element;
pub mod keys;
mod pty;

use std::borrow::Cow;
use std::sync::Arc;

use alacritty_terminal::event::{Event, OnResize, WindowSize};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::Rgb;
use futures::StreamExt;
use futures::channel::mpsc::unbounded;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::Button;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::keymap::*;
use crate::ui::{h_flex, v_flex};
use colors::Palette;
use element::TerminalElement;
pub use pty::Launch;
use pty::{EventProxy, Session};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Running,
    Exited(Option<i32>),
    Failed(String),
}

pub enum TerminalEvent {
    /// The program set (or reset) the window title.
    TitleChanged,
    /// The child process ended.
    Exited,
}

/// Cell geometry of the last layout, for resizing and the IME caret.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Metrics {
    pub cols: usize,
    pub rows: usize,
    pub cell_width: f32,
    pub cell_height: f32,
}

pub struct TerminalView {
    launch: Launch,
    session: Option<Session>,
    state: SessionState,
    title: String,
    metrics: Metrics,
    bounds: Bounds<Pixels>,
    focus_handle: FocusHandle,
    selecting: bool,
    _pump: Option<Task<()>>,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl TerminalView {
    pub fn new(launch: Launch, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            launch,
            session: None,
            state: SessionState::Exited(None),
            title: String::new(),
            metrics: Metrics {
                cols: 80,
                rows: 24,
                cell_width: 8.,
                cell_height: 16.,
            },
            bounds: Bounds::default(),
            focus_handle: cx.focus_handle(),
            selecting: false,
            _pump: None,
        };
        this.start(cx);
        this
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let (tx, mut rx) = unbounded();
        let proxy = EventProxy(tx);
        match pty::spawn(&self.launch, self.metrics.cols, self.metrics.rows, proxy) {
            Ok(session) => {
                self.session = Some(session);
                self.state = SessionState::Running;
            }
            Err(err) => {
                self.session = None;
                self.state = SessionState::Failed(format!("{}: {err}", self.launch.program));
                cx.notify();
                return;
            }
        }
        // Terminal events arrive on the PTY thread; bursts of `Wakeup`
        // collapse into one redraw here.
        self._pump = Some(cx.spawn(async move |this, cx| {
            while let Some(first) = rx.next().await {
                let mut batch = vec![first];
                while let Ok(event) = rx.try_recv() {
                    batch.push(event);
                }
                if this
                    .update(cx, |view, cx| view.on_events(batch, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
        cx.notify();
    }

    fn on_events(&mut self, events: Vec<Event>, cx: &mut Context<Self>) {
        let mut dirty = false;
        for event in events {
            match event {
                Event::Wakeup | Event::CursorBlinkingChange | Event::MouseCursorDirty => {
                    dirty = true;
                }
                Event::Title(title) => {
                    self.title = title;
                    cx.emit(TerminalEvent::TitleChanged);
                }
                Event::ResetTitle => {
                    self.title.clear();
                    cx.emit(TerminalEvent::TitleChanged);
                }
                // Replies to the program's queries (cursor position, device
                // attributes…): without them full-screen programs hang.
                Event::PtyWrite(text) => self.write(text.into_bytes()),
                Event::ClipboardStore(_, text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                Event::ClipboardLoad(_, format) => {
                    let text = cx
                        .read_from_clipboard()
                        .and_then(|item| item.text())
                        .unwrap_or_default();
                    self.write(format(&text).into_bytes());
                }
                Event::ColorRequest(index, format) => {
                    let palette = Palette::from_theme(cx.theme());
                    let color = if index < 256 {
                        palette.indexed_default(index)
                    } else if index == 257 {
                        palette.background
                    } else {
                        palette.foreground
                    };
                    self.write(format(hsla_to_rgb(color)).into_bytes());
                }
                Event::TextAreaSizeRequest(format) => {
                    self.write(format(self.window_size()).into_bytes());
                }
                Event::ChildExit(status) => self.finish(status.code(), cx),
                Event::Exit => {
                    if self.state == SessionState::Running {
                        self.finish(None, cx);
                    }
                }
                Event::Bell => {}
            }
        }
        if dirty {
            cx.notify();
        }
    }

    fn finish(&mut self, code: Option<i32>, cx: &mut Context<Self>) {
        self.state = SessionState::Exited(code);
        self.session = None;
        self.selecting = false;
        cx.emit(TerminalEvent::Exited);
        cx.notify();
    }

    fn window_size(&self) -> WindowSize {
        WindowSize {
            num_lines: self.metrics.rows.max(1) as u16,
            num_cols: self.metrics.cols.max(1) as u16,
            cell_width: self.metrics.cell_width.round().max(1.) as u16,
            cell_height: self.metrics.cell_height.round().max(1.) as u16,
        }
    }

    pub fn restart(&mut self, cx: &mut Context<Self>) {
        if self.state != SessionState::Running {
            self.start(cx);
        }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    pub fn is_running(&self) -> bool {
        self.state == SessionState::Running
    }

    /// The title the program set, if any.
    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn term(&self) -> Option<&Arc<FairMutex<Term<EventProxy>>>> {
        self.session.as_ref().map(|s| &s.term)
    }

    pub fn metrics(&self) -> Metrics {
        self.metrics
    }

    /// Send bytes to the program.
    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        if let Some(session) = &self.session {
            use alacritty_terminal::event::Notify;
            session.notifier.notify(bytes);
        }
    }

    fn mode(&self) -> TermMode {
        self.session
            .as_ref()
            .map(|s| *s.term.lock().mode())
            .unwrap_or_else(TermMode::empty)
    }

    /// The element laid out: keep the grid the size of the box.
    pub fn resize(&mut self, metrics: Metrics, bounds: Bounds<Pixels>) {
        self.bounds = bounds;
        if metrics == self.metrics {
            return;
        }
        self.metrics = metrics;
        let size = self.window_size();
        if let Some(session) = &mut self.session {
            session
                .term
                .lock()
                .resize(TermSize::new(metrics.cols.max(1), metrics.rows.max(1)));
            session.notifier.on_resize(size);
        }
    }

    pub fn scroll(&self, lines: i32, cx: &mut Context<Self>) {
        if lines == 0 {
            return;
        }
        if let Some(session) = &self.session {
            let mut term = session.term.lock();
            let mode = *term.mode();
            if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
                // Full-screen programs get the wheel as arrow keys.
                drop(term);
                let key = if lines > 0 { b"\x1b[A" } else { b"\x1b[B" };
                let bytes: Vec<u8> = key
                    .iter()
                    .cycle()
                    .take(3 * lines.unsigned_abs() as usize)
                    .copied()
                    .collect();
                self.write(bytes);
            } else {
                term.scroll_display(Scroll::Delta(lines));
            }
        }
        cx.notify();
    }

    pub fn copy_selection(&self, cx: &mut Context<Self>) {
        if let Some(text) = self
            .session
            .as_ref()
            .and_then(|s| s.term.lock().selection_to_string())
            && !text.is_empty()
        {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub fn paste_clipboard(&self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.write(keys::paste(&text, self.mode()));
            self.scroll_to_bottom();
        }
    }

    fn scroll_to_bottom(&self) {
        if let Some(session) = &self.session {
            session.term.lock().scroll_display(Scroll::Bottom);
        }
    }

    // ----- mouse selection (driven by the element) -------------------------

    pub fn selection_start(
        &mut self,
        point: Point,
        side: Side,
        clicks: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = &self.session else {
            return;
        };
        let ty = match clicks {
            2 => SelectionType::Semantic,
            3 => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        session.term.lock().selection = Some(Selection::new(ty, point, side));
        self.selecting = true;
        cx.notify();
    }

    pub fn selection_update(&mut self, point: Point, side: Side, cx: &mut Context<Self>) {
        if !self.selecting {
            return;
        }
        if let Some(session) = &self.session
            && let Some(selection) = session.term.lock().selection.as_mut()
        {
            selection.update(point, side);
        }
        cx.notify();
    }

    pub fn selection_end(&mut self, cx: &mut Context<Self>) {
        self.selecting = false;
        if let Some(session) = &self.session {
            let mut term = session.term.lock();
            if term.selection.as_ref().is_some_and(|s| s.is_empty()) {
                term.selection = None;
            }
        }
        cx.notify();
    }

    pub fn clear_selection(&self) {
        if let Some(session) = &self.session {
            session.term.lock().selection = None;
        }
    }

    /// The grid point under a pixel position of the last layout.
    pub fn point_at(&self, position: Point2D) -> (Point, Side) {
        let m = self.metrics;
        let x = (position.x - self.bounds.origin.x).as_f32().max(0.);
        let y = (position.y - self.bounds.origin.y).as_f32().max(0.);
        let col_f = x / m.cell_width.max(1.);
        let col = (col_f.floor() as usize).min(m.cols.saturating_sub(1));
        let row = ((y / m.cell_height.max(1.)).floor() as usize).min(m.rows.saturating_sub(1));
        let display_offset = self
            .session
            .as_ref()
            .map(|s| s.term.lock().grid().display_offset())
            .unwrap_or(0);
        let line = Line(row as i32 - display_offset as i32);
        let side = if col_f.fract() < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        (Point::new(line, Column(col)), side)
    }

    // ----- keys ----------------------------------------------------------------

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_running() {
            return;
        }
        let ks = &event.keystroke;
        let m = &ks.modifiers;
        let secondary_shift = m.secondary() && m.shift && !m.alt;
        match ks.key.as_str() {
            "c" if secondary_shift => {
                self.copy_selection(cx);
                cx.stop_propagation();
                return;
            }
            "v" if secondary_shift => {
                self.paste_clipboard(cx);
                cx.stop_propagation();
                return;
            }
            "pageup" if m.shift && !m.control && !m.alt => {
                if let Some(session) = &self.session {
                    session.term.lock().scroll_display(Scroll::PageUp);
                }
                cx.notify();
                cx.stop_propagation();
                return;
            }
            "pagedown" if m.shift && !m.control && !m.alt => {
                if let Some(session) = &self.session {
                    session.term.lock().scroll_display(Scroll::PageDown);
                }
                cx.notify();
                cx.stop_propagation();
                return;
            }
            _ => {}
        }
        let _ = window;
        let encoded = keys::encode(ks, self.mode());
        log::trace!("terminal key {:?} -> {:?}", ks, encoded);
        if let Some(bytes) = encoded {
            self.clear_selection();
            self.write(bytes);
            self.scroll_to_bottom();
            cx.stop_propagation();
        }
    }

    fn on_restart(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.restart(cx);
        self.focus_handle.focus(window, cx);
    }
}

/// A pixel position (gpui's `Point<Pixels>`, named apart from the grid point).
pub type Point2D = gpui_kit::Point<Pixels>;

fn hsla_to_rgb(color: Hsla) -> Rgb {
    let rgba = Rgba::from(color);
    Rgb {
        r: (rgba.r * 255.).round() as u8,
        g: (rgba.g * 255.).round() as u8,
        b: (rgba.b * 255.).round() as u8,
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _range: std::ops::Range<usize>,
        _adjusted: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        None
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    /// Text committed by the input method (or typed on a layout gpui
    /// routes through the IME): straight to the program.
    fn replace_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        text: &str,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        if self.is_running() && !text.is_empty() {
            self.write(text.as_bytes().to_vec());
            self.scroll_to_bottom();
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        _new_text: &str,
        _new_selected_range: Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        // Pre-edit text is not shown; the commit arrives through
        // `replace_text_in_range`.
    }

    /// Where the IME popup goes: the cursor cell.
    fn bounds_for_range(
        &mut self,
        _range: std::ops::Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let session = self.session.as_ref()?;
        let term = session.term.lock();
        let content = term.renderable_content();
        let row = content.cursor.point.line.0 + content.display_offset as i32;
        let col = content.cursor.point.column.0;
        drop(term);
        let m = self.metrics;
        Some(Bounds::new(
            point(
                element_bounds.origin.x + px(col as f32 * m.cell_width),
                element_bounds.origin.y + px(row.max(0) as f32 * m.cell_height),
            ),
            size(px(m.cell_width), px(m.cell_height)),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui_kit::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }

    fn accepts_text_input(&self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
        self.is_running()
    }
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::from_theme(cx.theme());
        let muted = cx.theme().muted_foreground;
        let note = match &self.state {
            SessionState::Running => None,
            SessionState::Exited(code) => Some(match code {
                Some(code) => format!("Process exited with code {code}"),
                None => "Process exited".to_string(),
            }),
            SessionState::Failed(err) => Some(format!("Could not start: {err}")),
        };
        div()
            .id("terminal")
            .key_context(TERMINAL_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(palette.background)
            .child(TerminalElement::new(cx.entity(), self.focus_handle.clone()))
            .when_some(note, |d, note| {
                d.child(
                    v_flex()
                        .absolute()
                        .inset_0()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .bg(palette.background.alpha(0.85))
                        .child(div().text_sm().text_color(muted).child(note))
                        .child(
                            h_flex().gap_2().child(
                                Button::new("terminal-restart")
                                    .small()
                                    .label("Restart")
                                    .on_click(cx.listener(Self::on_restart)),
                            ),
                        ),
                )
            })
    }
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            use alacritty_terminal::event_loop::Msg;
            let _ = session.notifier.0.send(Msg::Shutdown);
            session.term.lock().exit();
        }
    }
}
