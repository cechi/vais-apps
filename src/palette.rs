//! A command-center style dialog body: an optional text input on top and a
//! list of rows below it. Two flavours share the look and the keys:
//!
//! * **Value** — the input holds the answer (a path, a name). The arrows move
//!   through the rows and copy the highlighted row's value into the input,
//!   which can also be edited by hand. Enter confirms whatever the input
//!   holds; a click confirms that row directly.
//! * **Pick** — the rows are the choices (Overwrite / Skip, a theme…). An
//!   optional input filters them. Enter or a click confirms the highlighted
//!   row.
//!
//! Esc cancels. [`open`] shows a palette in a dialog near the top of the
//! window, without buttons.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::keymap::*;
use crate::ui::{h_flex, v_flex};

/// One row.
pub struct PaletteItem {
    pub label: SharedString,
    /// Muted text at the right edge ("Panel 2", "dark").
    pub detail: SharedString,
    /// What choosing the row puts into a value input. Defaults to the label.
    pub value: String,
    pub icon: Option<IconName>,
    /// Shown with a check mark (the current theme).
    pub checked: bool,
    /// Shown in the danger colour (Overwrite, Delete permanently).
    pub danger: bool,
}

impl PaletteItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        let label = label.into();
        Self {
            value: label.to_string(),
            label,
            detail: SharedString::default(),
            icon: None,
            checked: false,
            danger: false,
        }
    }
    #[allow(dead_code)] // part of the builder; the destination palette may use it later
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = value.into();
        self
    }
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = detail.into();
        self
    }
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }
    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }
}

/// What the user confirmed: the text of the input (the row's value in pick
/// mode) and, when it matches a row, that row's index.
pub struct PaletteChoice {
    pub value: String,
    pub item: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Value,
    Pick,
}

type ConfirmFn = Rc<dyn Fn(PaletteChoice, &mut Window, &mut App)>;
type SelectFn = Rc<dyn Fn(Option<usize>, &mut Window, &mut App)>;
type CancelFn = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct Palette {
    mode: Mode,
    /// Absent for a plain choice list.
    input: Option<Entity<InputState>>,
    items: Vec<PaletteItem>,
    selected: Option<usize>,
    input_icon: Option<IconName>,
    /// Muted paragraph above the input / rows.
    description: Option<SharedString>,
    /// Muted text under the rows when there are none.
    hint: Option<SharedString>,
    /// Muted text under the rows, always.
    footer: Option<SharedString>,
    /// Byte range to select on focus; the whole text when `None`.
    select: Option<std::ops::Range<usize>>,
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    on_confirm: Option<ConfirmFn>,
    on_select: Option<SelectFn>,
    on_cancel: Option<CancelFn>,
    _subscriptions: Vec<Subscription>,
}

impl Palette {
    /// A value palette: the input starts with `initial`.
    pub fn new(
        items: Vec<PaletteItem>,
        initial: &str,
        placeholder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::build(Mode::Value, items, cx);
        this.selected = position_of(&this.items, initial);
        this.attach_input(initial, placeholder, window, cx);
        this
    }

    /// A choice list without an input; the first row is highlighted.
    pub fn pick(items: Vec<PaletteItem>, cx: &mut Context<Self>) -> Self {
        let mut this = Self::build(Mode::Pick, items, cx);
        this.selected = (!this.items.is_empty()).then_some(0);
        this.input_icon = None;
        this
    }

    /// A choice list with a filter input on top.
    pub fn pick_filtered(
        items: Vec<PaletteItem>,
        placeholder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::pick(items, cx);
        this.input_icon = Some(IconName::Search);
        this.attach_input("", placeholder, window, cx);
        this
    }

    fn build(mode: Mode, items: Vec<PaletteItem>, cx: &mut Context<Self>) -> Self {
        Self {
            mode,
            input: None,
            items,
            selected: None,
            input_icon: Some(IconName::Folder),
            description: None,
            hint: None,
            footer: None,
            select: None,
            focus_handle: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            on_confirm: None,
            on_select: None,
            on_cancel: None,
            _subscriptions: Vec::new(),
        }
    }

    fn attach_input(
        &mut self,
        initial: &str,
        placeholder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(initial.to_string())
                .placeholder(placeholder.to_string())
        });
        // Arrow keys write the input programmatically, which emits no
        // `Change`, so this only runs for typing.
        self._subscriptions.push(cx.subscribe_in(
            &input,
            window,
            |this, input, event: &InputEvent, window, cx| {
                if let InputEvent::Change = event {
                    let text = input.read(cx).value();
                    this.on_typed(text.as_ref(), window, cx);
                }
            },
        ));
        self.input = Some(input);
    }

    // ----- builders --------------------------------------------------------

    pub fn on_confirm(
        mut self,
        f: impl Fn(PaletteChoice, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_confirm = Some(Rc::new(f));
        self
    }

    pub fn on_select(mut self, f: impl Fn(Option<usize>, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }

    pub fn on_cancel(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Rc::new(f));
        self
    }

    pub fn input_icon(mut self, icon: Option<IconName>) -> Self {
        self.input_icon = icon;
        self
    }

    pub fn description(mut self, text: impl Into<SharedString>) -> Self {
        self.description = Some(text.into());
        self
    }

    /// Text shown under the rows while there are none.
    pub fn hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn footer(mut self, text: impl Into<SharedString>) -> Self {
        self.footer = Some(text.into());
        self
    }

    /// Select only this byte range of the initial text on focus (e.g. the
    /// file name without its extension) instead of everything.
    pub fn select_range(mut self, range: std::ops::Range<usize>) -> Self {
        self.select = Some(range);
        self
    }

    // ----- state -----------------------------------------------------------

    /// Focus the input (text or the chosen range selected) or, without one,
    /// the list itself.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = &self.input else {
            self.focus_handle.focus(window, cx);
            return;
        };
        let select = self.select.clone();
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            match select {
                Some(range) => input.set_selected_range(range, cx),
                None => input.select_all(window, cx),
            }
        });
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    fn query(&self, cx: &App) -> String {
        self.input
            .as_ref()
            .map(|i| i.read(cx).value().to_string())
            .unwrap_or_default()
    }

    /// Rows currently shown: all of them, or those matching the filter.
    fn visible(&self, cx: &App) -> Vec<usize> {
        match self.mode {
            Mode::Value => (0..self.items.len()).collect(),
            Mode::Pick => filter_rows(&self.items, &self.query(cx)),
        }
    }

    fn on_typed(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        match self.mode {
            // The highlight follows the text: the row with that value, or none.
            Mode::Value => self.set_selected(position_of(&self.items, text), window, cx),
            // Keep the highlight if it survived the filter, else the first match.
            Mode::Pick => {
                let visible = filter_rows(&self.items, text);
                let keep = self.selected.filter(|ix| visible.contains(ix));
                self.set_selected(keep.or(visible.first().copied()), window, cx);
                cx.notify();
            }
        }
    }

    fn set_selected(&mut self, ix: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected == ix {
            return;
        }
        self.selected = ix;
        if let Some(on_select) = self.on_select.clone() {
            on_select(ix, window, cx);
        }
        cx.notify();
    }

    /// Move the highlight by `delta` visible rows (wrapping); in value mode
    /// also show the row's value in the input.
    fn step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let visible = self.visible(cx);
        let pos = self
            .selected
            .and_then(|ix| visible.iter().position(|&v| v == ix));
        let Some(pos) = step(pos, visible.len(), delta) else {
            return;
        };
        let ix = visible[pos];
        if self.mode == Mode::Value
            && let Some(input) = &self.input
        {
            let value = self.items[ix].value.clone();
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.scroll.scroll_to_item(pos);
        self.set_selected(Some(ix), window, cx);
    }

    fn confirm_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.mode {
            Mode::Value => {
                let value = self.query(cx);
                let item = position_of(&self.items, &value).or(self.selected);
                self.confirm(PaletteChoice { value, item }, window, cx);
            }
            Mode::Pick => {
                if let Some(ix) = self.selected {
                    self.confirm_row(ix, window, cx);
                }
            }
        }
    }

    fn confirm_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.items.get(ix) else {
            return;
        };
        let choice = PaletteChoice {
            value: item.value.clone(),
            item: Some(ix),
        };
        self.confirm(choice, window, cx);
    }

    fn confirm(&mut self, choice: PaletteChoice, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(on_confirm) = self.on_confirm.clone() {
            on_confirm(choice, window, cx);
        }
    }

    fn on_up(&mut self, _: &PaletteUp, window: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, window, cx);
    }
    fn on_down(&mut self, _: &PaletteDown, window: &mut Window, cx: &mut Context<Self>) {
        self.step(1, window, cx);
    }
    fn on_confirm_action(
        &mut self,
        _: &PaletteConfirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_current(window, cx);
    }
    fn on_cancel_action(&mut self, _: &PaletteCancel, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(on_cancel) = self.on_cancel.clone() {
            on_cancel(window, cx);
        }
    }

    // ----- rendering -------------------------------------------------------

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let item = &self.items[ix];
        let selected = self.selected == Some(ix);
        let (bg, outline) = if selected {
            (theme.primary.alpha(0.28), theme.list_active_border)
        } else {
            (transparent_black(), transparent_black())
        };
        let hover = theme.list_hover;
        let muted = theme.muted_foreground;
        let fg = if item.danger {
            theme.danger
        } else {
            theme.foreground
        };
        h_flex()
            .id(("palette-row", ix))
            .h(rems(1.75))
            .flex_none()
            .px_2()
            .gap_2()
            .rounded_sm()
            .bg(bg)
            .border_1()
            .border_color(outline)
            .cursor_pointer()
            .text_color(fg)
            .when(!selected, |row| row.hover(move |s| s.bg(hover)))
            .on_click(cx.listener(move |this, _, window, cx| this.confirm_row(ix, window, cx)))
            .when_some(item.icon.clone(), |row, icon| {
                row.child(Icon::new(icon).text_color(theme.blue).flex_none())
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(item.label.clone()),
            )
            .when(!item.detail.is_empty(), |row| {
                row.child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(muted)
                        .child(item.detail.clone()),
                )
            })
            .when(item.checked, |row| {
                row.child(Icon::new(IconName::Check).flex_none())
            })
            .into_any_element()
    }
}

impl Render for Palette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let rows: Vec<AnyElement> = self
            .visible(cx)
            .into_iter()
            .map(|ix| self.render_row(ix, cx))
            .collect();
        let no_rows = rows.is_empty();
        v_flex()
            .key_context(PALETTE_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_up))
            .on_action(cx.listener(Self::on_down))
            .on_action(cx.listener(Self::on_confirm_action))
            .on_action(cx.listener(Self::on_cancel_action))
            .w_full()
            .gap_2()
            .when_some(self.description.clone(), |this, text| {
                this.child(div().text_sm().text_color(muted).child(text))
            })
            .when_some(self.input.as_ref(), |this, input| {
                this.child(
                    Input::new(input)
                        .focus_bordered(false)
                        .when_some(self.input_icon.clone(), |input, icon| {
                            input.prefix(Icon::new(icon).text_color(muted))
                        }),
                )
            })
            .child(
                v_flex()
                    .id("palette-rows")
                    .w_full()
                    .max_h(rems(18.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .gap_0p5()
                    .children(rows)
                    .when_some(self.hint.clone().filter(|_| no_rows), |list, hint| {
                        list.child(div().px_2().py_1().text_sm().text_color(muted).child(hint))
                    }),
            )
            .when_some(self.footer.clone(), |this, text| {
                this.child(div().text_xs().text_color(muted).child(text))
            })
    }
}

/// Show `palette` in a dialog near the top of the window: a title, no
/// buttons, the input (or the list) focused.
pub fn open(title: String, palette: Entity<Palette>, window: &mut Window, cx: &mut App) {
    let initial_focus = Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if initial_focus.replace(false) {
            palette.update(cx, |palette, cx| palette.focus(window, cx));
        }
        let palette = palette.clone();
        dialog
            .title(title.clone())
            .w(px(640.))
            .margin_top(px(64.))
            .close_button(false)
            .content(move |content, _, _| content.child(palette.clone()))
    });
}

/// The row whose value equals `text`.
fn position_of(items: &[PaletteItem], text: &str) -> Option<usize> {
    items.iter().position(|item| item.value == text)
}

/// Rows whose label contains `query`, case-insensitively; all of them for an
/// empty query.
fn filter_rows(items: &[PaletteItem], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| query.is_empty() || item.label.to_lowercase().contains(&query))
        .map(|(ix, _)| ix)
        .collect()
}

/// Move a highlight by `delta` through `len` rows, wrapping at both ends. With
/// no highlight, Down starts at the first row and Up at the last.
fn step(selected: Option<usize>, len: usize, delta: isize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let len = len as isize;
    let next = match selected {
        Some(ix) => (ix as isize + delta).rem_euclid(len),
        None if delta < 0 => len - 1,
        None => 0,
    };
    Some(next as usize)
}

#[cfg(test)]
mod tests {
    // Named imports: the gpui glob re-exports its own `test` attribute.
    use super::{PaletteItem, filter_rows, position_of, step};

    #[test]
    fn stepping_wraps_and_starts_at_the_ends() {
        assert_eq!(step(None, 0, 1), None);
        assert_eq!(step(None, 3, 1), Some(0));
        assert_eq!(step(None, 3, -1), Some(2));
        assert_eq!(step(Some(0), 3, -1), Some(2));
        assert_eq!(step(Some(2), 3, 1), Some(0));
        assert_eq!(step(Some(1), 3, 1), Some(2));
    }

    #[test]
    fn position_matches_the_exact_value() {
        let items = vec![
            PaletteItem::new("a").value("/a"),
            PaletteItem::new("b").value("/b"),
        ];
        assert_eq!(position_of(&items, "/b"), Some(1));
        assert_eq!(position_of(&items, "/b/"), None);
    }

    #[test]
    fn filtering_is_a_case_insensitive_substring_match() {
        let items = vec![
            PaletteItem::new("Dank Dark"),
            PaletteItem::new("Default Light"),
            PaletteItem::new("System"),
        ];
        assert_eq!(filter_rows(&items, ""), vec![0, 1, 2]);
        assert_eq!(filter_rows(&items, "dark"), vec![0]);
        assert_eq!(filter_rows(&items, " D"), vec![0, 1]);
        assert_eq!(filter_rows(&items, "zzz"), Vec::<usize>::new());
    }
}
