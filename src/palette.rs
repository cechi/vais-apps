//! A command-center style chooser: a text input on top and a short list of
//! suggestions below it. The arrows move through the list and copy the
//! highlighted value into the input, which can also be edited by hand. Enter
//! confirms whatever the input holds; a click confirms that row directly.
//!
//! The first use is the destination of a copy / move, where the rows are the
//! other panels of the grid.

use std::rc::Rc;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::keymap::*;
use crate::ui::{h_flex, v_flex};

/// One suggestion row.
pub struct PaletteItem {
    /// The main text of the row.
    pub label: SharedString,
    /// Muted text at the right edge ("Panel 2").
    pub detail: SharedString,
    /// What choosing the row puts into the input.
    pub value: String,
    pub icon: Option<IconName>,
}

/// What the user confirmed: the text of the input and, when it matches a
/// row, that row's index.
pub struct PaletteChoice {
    pub value: String,
    /// Not every caller cares which row was chosen.
    #[allow(dead_code)]
    pub item: Option<usize>,
}

type ConfirmFn = Rc<dyn Fn(PaletteChoice, &mut Window, &mut App)>;
type SelectFn = Rc<dyn Fn(Option<usize>, &mut Window, &mut App)>;
type CancelFn = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct Palette {
    input: Entity<InputState>,
    items: Vec<PaletteItem>,
    selected: Option<usize>,
    /// Icon at the left of the input.
    input_icon: Option<IconName>,
    /// Muted text under the input when there are no rows.
    hint: Option<SharedString>,
    /// Byte range to select on focus; the whole text when `None`.
    select: Option<std::ops::Range<usize>>,
    on_confirm: Option<ConfirmFn>,
    on_select: Option<SelectFn>,
    on_cancel: Option<CancelFn>,
    _subscriptions: Vec<Subscription>,
}

impl Palette {
    pub fn new(
        items: Vec<PaletteItem>,
        initial: &str,
        placeholder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(initial.to_string())
                .placeholder(placeholder.to_string())
        });
        // Typing moves the highlight to the row whose value matches the text,
        // or clears it. Arrow keys write the input programmatically, which
        // emits no `Change`, so this does not fight with them.
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, input, event: &InputEvent, window, cx| {
                if let InputEvent::Change = event {
                    let text = input.read(cx).value();
                    this.set_selected(position_of(&this.items, text.as_ref()), window, cx);
                }
            },
        );
        Self {
            selected: position_of(&items, initial),
            input,
            items,
            input_icon: Some(IconName::Folder),
            hint: None,
            select: None,
            on_confirm: None,
            on_select: None,
            on_cancel: None,
            _subscriptions: vec![subscription],
        }
    }

    pub fn input_icon(mut self, icon: Option<IconName>) -> Self {
        self.input_icon = icon;
        self
    }

    /// Text shown under the input while the list is empty.
    pub fn hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Select only this byte range of the initial text on focus (e.g. the
    /// file name without its extension) instead of everything.
    pub fn select_range(mut self, range: std::ops::Range<usize>) -> Self {
        self.select = Some(range);
        self
    }

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

    /// Focus the input with its text (or the chosen range) selected, so
    /// typing replaces it.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        let select = self.select.clone();
        self.input.update(cx, |input, cx| {
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

    /// Move the highlight by `delta` rows (wrapping) and show its value.
    fn step(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = step(self.selected, self.items.len(), delta) else {
            return;
        };
        let value = self.items[ix].value.clone();
        self.input
            .update(cx, |input, cx| input.set_value(value, window, cx));
        self.set_selected(Some(ix), window, cx);
    }

    fn confirm_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value().to_string();
        let item = position_of(&self.items, &value).or(self.selected);
        self.confirm(PaletteChoice { value, item }, window, cx);
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
        self.confirm_input(window, cx);
    }
    fn on_cancel_action(&mut self, _: &PaletteCancel, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(on_cancel) = self.on_cancel.clone() {
            on_cancel(window, cx);
        }
    }

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
        h_flex()
            .id(("palette-row", ix))
            .h(rems(1.75))
            .px_2()
            .gap_2()
            .rounded_sm()
            .bg(bg)
            .border_1()
            .border_color(outline)
            .cursor_pointer()
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
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(muted)
                    .child(item.detail.clone()),
            )
            .into_any_element()
    }
}

impl Render for Palette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let rows: Vec<AnyElement> = (0..self.items.len())
            .map(|ix| self.render_row(ix, cx))
            .collect();
        v_flex()
            .key_context(PALETTE_CONTEXT)
            .on_action(cx.listener(Self::on_up))
            .on_action(cx.listener(Self::on_down))
            .on_action(cx.listener(Self::on_confirm_action))
            .on_action(cx.listener(Self::on_cancel_action))
            .w_full()
            .gap_2()
            .child(
                Input::new(&self.input)
                    .focus_bordered(false)
                    .when_some(self.input_icon.clone(), |input, icon| {
                        input.prefix(Icon::new(icon).text_color(muted))
                    }),
            )
            .child(v_flex().w_full().gap_0p5().children(rows).when_some(
                self.hint.clone().filter(|_| self.items.is_empty()),
                |list, hint| {
                    list.child(div().px_2().py_1().text_sm().text_color(muted).child(hint))
                },
            ))
    }
}

/// The row whose value equals `text`.
fn position_of(items: &[PaletteItem], text: &str) -> Option<usize> {
    items.iter().position(|item| item.value == text)
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
    use super::{PaletteItem, position_of, step};

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
            PaletteItem {
                label: "a".into(),
                detail: "".into(),
                value: "/a".into(),
                icon: None,
            },
            PaletteItem {
                label: "b".into(),
                detail: "".into(),
                value: "/b".into(),
                icon: None,
            },
        ];
        assert_eq!(position_of(&items, "/b"), Some(1));
        assert_eq!(position_of(&items, "/b/"), None);
    }
}
