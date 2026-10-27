//! The element that paints a terminal's grid: cell backgrounds, one shaped
//! line of text per row, the selection and the cursor. It also sizes the
//! grid to its box, registers the mouse handlers and the IME hook.

use alacritty_terminal::index::Point;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{CursorShape, NamedColor};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

use super::colors::Palette;
use super::{Metrics, TerminalView};

pub struct TerminalElement {
    view: Entity<TerminalView>,
    focus: FocusHandle,
}

impl TerminalElement {
    pub fn new(view: Entity<TerminalView>, focus: FocusHandle) -> Self {
        Self { view, focus }
    }
}

/// What prepaint worked out for paint.
pub struct Prepaint {
    hitbox: Hitbox,
    font: Font,
    font_size: Pixels,
    metrics: Metrics,
}

/// One cell as it will be drawn.
#[derive(Clone)]
struct DrawCell {
    c: char,
    fg: Hsla,
    bg: Option<Hsla>,
    flags: Flags,
}

struct Frame {
    rows: Vec<Vec<DrawCell>>,
    cursor: Option<(usize, usize, CursorShape, char)>,
    focused: bool,
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let theme = cx.theme();
        let font = Font {
            family: theme.mono_font_family.clone(),
            ..window.text_style().font()
        };
        let font_size = theme.mono_font_size;
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font);
        let cell_width = text_system
            .advance(font_id, font_size, 'M')
            .map(|s| s.width.as_f32())
            .unwrap_or(font_size.as_f32() * 0.6)
            .max(1.);
        let cell_height = (font_size.as_f32() * 1.3).round().max(1.);
        let metrics = Metrics {
            cols: ((bounds.size.width.as_f32() / cell_width).floor() as usize).max(2),
            rows: ((bounds.size.height.as_f32() / cell_height).floor() as usize).max(1),
            cell_width,
            cell_height,
        };
        self.view.update(cx, |view, _| view.resize(metrics, bounds));
        Prepaint {
            hitbox,
            font,
            font_size,
            metrics,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        prepaint: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.handle_input(
            &self.focus,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        self.register_mouse(prepaint, window);

        let palette = Palette::from_theme(cx.theme());
        let focused = self.focus.is_focused(window);
        let Some(frame) = snapshot(&self.view, &palette, focused, cx) else {
            return;
        };
        let m = prepaint.metrics;
        let (cw, ch) = (px(m.cell_width), px(m.cell_height));
        let origin = bounds.origin;

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            // Backgrounds: runs of equal colour per row.
            for (row_ix, row) in frame.rows.iter().enumerate() {
                let y = origin.y + ch * row_ix as f32;
                let mut col = 0;
                while col < row.len() {
                    let Some(bg) = row[col].bg else {
                        col += 1;
                        continue;
                    };
                    let start = col;
                    while col < row.len() && row[col].bg == Some(bg) {
                        col += 1;
                    }
                    let x = origin.x + cw * start as f32;
                    window.paint_quad(fill(
                        Bounds::new(point(x, y), size(cw * (col - start) as f32, ch)),
                        bg,
                    ));
                }
            }
            // The cursor block goes under the glyphs.
            if let Some((row, col, shape, _)) = frame.cursor {
                let x = origin.x + cw * col as f32;
                let y = origin.y + ch * row as f32;
                let cell = Bounds::new(point(x, y), size(cw, ch));
                match (shape, frame.focused) {
                    (CursorShape::Hidden, _) => {}
                    (CursorShape::Block, true) => window.paint_quad(fill(cell, palette.cursor)),
                    (CursorShape::Block, false) => {
                        window.paint_quad(outline(cell, palette.cursor, BorderStyle::Solid))
                    }
                    (CursorShape::Underline, _) => window.paint_quad(fill(
                        Bounds::new(point(x, y + ch - px(2.)), size(cw, px(2.))),
                        palette.cursor,
                    )),
                    (CursorShape::Beam, _) => window.paint_quad(fill(
                        Bounds::new(point(x, y), size(px(2.), ch)),
                        palette.cursor,
                    )),
                    (CursorShape::HollowBlock, _) => {
                        window.paint_quad(outline(cell, palette.cursor, BorderStyle::Solid))
                    }
                }
            }
            // Text: one shaped line per row, a run per cell.
            for (row_ix, row) in frame.rows.iter().enumerate() {
                let y = origin.y + ch * row_ix as f32;
                let mut text = String::with_capacity(row.len());
                let mut runs: Vec<TextRun> = Vec::with_capacity(row.len());
                let mut wide: Vec<(usize, char, TextRun)> = Vec::new();
                for (col, cell) in row.iter().enumerate() {
                    let mut c = cell.c;
                    if cell.flags.contains(Flags::HIDDEN) || c == '\0' {
                        c = ' ';
                    }
                    let under_cursor = frame.cursor.is_some_and(|(r, cc, shape, _)| {
                        r == row_ix && cc == col && shape == CursorShape::Block && frame.focused
                    });
                    let color = if under_cursor {
                        palette.background
                    } else {
                        cell.fg
                    };
                    let font = Font {
                        weight: if cell.flags.intersects(Flags::BOLD) {
                            FontWeight::BOLD
                        } else {
                            prepaint.font.weight
                        },
                        style: if cell.flags.contains(Flags::ITALIC) {
                            FontStyle::Italic
                        } else {
                            FontStyle::Normal
                        },
                        ..prepaint.font.clone()
                    };
                    let run = TextRun {
                        len: 0,
                        font,
                        color,
                        background_color: None,
                        underline: cell.flags.intersects(Flags::ALL_UNDERLINES).then(|| {
                            UnderlineStyle {
                                thickness: px(1.),
                                color: Some(color),
                                wavy: cell.flags.contains(Flags::UNDERCURL),
                            }
                        }),
                        strikethrough: cell.flags.contains(Flags::STRIKEOUT).then(|| {
                            StrikethroughStyle {
                                thickness: px(1.),
                                color: Some(color),
                            }
                        }),
                    };
                    if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                        text.push(' ');
                        runs.push(TextRun { len: 1, ..run });
                    } else if cell.flags.contains(Flags::WIDE_CHAR) {
                        // Drawn on its own: two cells wide.
                        text.push(' ');
                        runs.push(TextRun {
                            len: 1,
                            ..run.clone()
                        });
                        wide.push((col, c, run));
                    } else {
                        text.push(c);
                        runs.push(TextRun {
                            len: c.len_utf8(),
                            ..run
                        });
                    }
                }
                if text.trim().is_empty() && wide.is_empty() {
                    continue;
                }
                let line = window.text_system().shape_line(
                    text.into(),
                    prepaint.font_size,
                    &runs,
                    Some(cw),
                );
                let _ = line.paint(point(origin.x, y), ch, TextAlign::Left, None, window, cx);
                for (col, c, run) in wide {
                    let run = TextRun {
                        len: c.len_utf8(),
                        ..run
                    };
                    let glyph = window.text_system().shape_line(
                        c.to_string().into(),
                        prepaint.font_size,
                        &[run],
                        None,
                    );
                    let _ = glyph.paint(
                        point(origin.x + cw * col as f32, y),
                        ch,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
            }
        });
    }
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl TerminalElement {
    fn register_mouse(&self, prepaint: &Prepaint, window: &mut Window) {
        let hitbox = prepaint.hitbox.clone();
        let line_h = px(prepaint.metrics.cell_height);
        let view = self.view.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if !phase.bubble() || !hitbox.should_handle_scroll(window) {
                return;
            }
            let lines = match event.delta {
                ScrollDelta::Lines(lines) => (lines.y * 3.).round() as i32,
                ScrollDelta::Pixels(pixels) => (pixels.y / line_h).round() as i32,
            };
            if lines != 0 {
                view.update(cx, |view, cx| view.scroll(lines, cx));
            }
        });

        let hitbox = prepaint.hitbox.clone();
        let view = self.view.clone();
        let focus = self.focus.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if !phase.bubble() || !hitbox.is_hovered(window) {
                return;
            }
            if !focus.is_focused(window) {
                focus.focus(window, cx);
            }
            if event.button == MouseButton::Left {
                view.update(cx, |view, cx| {
                    let (point, side) = view.point_at(event.position);
                    view.selection_start(point, side, event.click_count, cx);
                });
            }
        });

        let view = self.view.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _window, cx| {
            if !phase.bubble() || event.pressed_button != Some(MouseButton::Left) {
                return;
            }
            view.update(cx, |view, cx| {
                let (point, side) = view.point_at(event.position);
                view.selection_update(point, side, cx);
            });
        });

        let view = self.view.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _window, cx| {
            if !phase.bubble() || event.button != MouseButton::Left {
                return;
            }
            view.update(cx, |view, cx| view.selection_end(cx));
        });
    }
}

/// Copy what the grid shows out from under the terminal's lock.
fn snapshot(
    view: &Entity<TerminalView>,
    palette: &Palette,
    focused: bool,
    cx: &App,
) -> Option<Frame> {
    let view = view.read(cx);
    let term = view.term()?;
    let term = term.lock();
    let content = term.renderable_content();
    let m = view.metrics();
    let mut rows: Vec<Vec<DrawCell>> = vec![Vec::with_capacity(m.cols); m.rows];
    let offset = content.display_offset as i32;
    let selection = content.selection;
    for indexed in content.display_iter {
        let row = indexed.point.line.0 + offset;
        if row < 0 || row as usize >= m.rows {
            continue;
        }
        let cell = indexed.cell;
        let mut fg = palette.resolve(cell.fg, content.colors);
        let mut bg = match cell.bg {
            alacritty_terminal::vte::ansi::Color::Named(NamedColor::Background) => None,
            other => Some(palette.resolve(other, content.colors)),
        };
        if cell.flags.contains(Flags::INVERSE) {
            let old_bg = bg.unwrap_or(palette.background);
            bg = Some(fg);
            fg = old_bg;
        }
        if cell.flags.contains(Flags::DIM) {
            fg = fg.alpha(0.6);
        }
        if selection
            .is_some_and(|s| s.contains(Point::new(indexed.point.line, indexed.point.column)))
        {
            bg = Some(palette.selection);
        }
        rows[row as usize].push(DrawCell {
            c: cell.c,
            fg,
            bg,
            flags: cell.flags,
        });
    }
    let cursor = if content.mode.contains(TermMode::SHOW_CURSOR) {
        let row = content.cursor.point.line.0 + offset;
        let col = content.cursor.point.column.0;
        (row >= 0 && (row as usize) < m.rows && col < m.cols).then(|| {
            let c = rows[row as usize].get(col).map(|c| c.c).unwrap_or(' ');
            (row as usize, col, content.cursor.shape, c)
        })
    } else {
        None
    };
    Some(Frame {
        rows,
        cursor,
        focused,
    })
}
