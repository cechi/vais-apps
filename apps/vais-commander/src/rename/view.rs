//! The batch rename form shown in its own tab: the rules on top, a live
//! preview of old → new names below, Rename / Cancel at the bottom.

use crate::vfs::Location;
use std::ops::Range;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, StyledExt as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{CaseMode, Plan, Rules, apply, plan};
use crate::keymap::*;
use crate::ui::{h_flex, v_flex};

pub enum RenameEvent {
    /// The files were renamed; panels showing `dir` should reload.
    Done {
        dir: Location,
        renamed: usize,
    },
    Cancelled,
}

pub struct RenameView {
    paths: Vec<Location>,
    dir: Location,
    name: Entity<InputState>,
    ext: Entity<InputState>,
    find: Entity<InputState>,
    replace: Entity<InputState>,
    start: Entity<InputState>,
    step: Entity<InputState>,
    digits: Entity<InputState>,
    regex: bool,
    ignore_case: bool,
    case: CaseMode,
    plan: Plan,
    /// The last failure of an apply.
    error: Option<String>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<RenameEvent> for RenameView {}

impl RenameView {
    pub fn new(paths: Vec<Location>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dir = paths
            .first()
            .and_then(|p| p.parent())
            .unwrap_or_else(|| Location::local(""));
        let defaults = Rules::default();
        let mut subscriptions = Vec::new();
        let mut input = |value: String, placeholder: &str| {
            let state = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value)
                    .placeholder(placeholder.to_string())
            });
            subscriptions.push(cx.subscribe_in(
                &state,
                window,
                |this, _, event: &InputEvent, _, cx| {
                    if let InputEvent::Change = event {
                        this.recompute(cx);
                    }
                },
            ));
            state
        };
        let name = input(defaults.name.clone(), "[N]");
        let ext = input(defaults.ext.clone(), "[E]");
        let find = input(String::new(), t!("Find##rename"));
        let replace = input(String::new(), t!("Replace##rename"));
        let start = input(defaults.counter_start.to_string(), "1");
        let step = input(defaults.counter_step.to_string(), "1");
        let digits = input(defaults.counter_digits.to_string(), "1");
        let mut this = Self {
            paths,
            dir,
            name,
            ext,
            find,
            replace,
            start,
            step,
            digits,
            regex: false,
            ignore_case: false,
            case: CaseMode::Keep,
            plan: Plan::default(),
            error: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        this.plan = plan(&this.paths, &this.rules(cx));
        this
    }

    pub fn title(&self) -> String {
        t!("Rename {count}", count = self.paths.len())
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.name.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    fn rules(&self, cx: &App) -> Rules {
        let text = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let number = |input: &Entity<InputState>, default: i64| {
            text(input).trim().parse::<i64>().unwrap_or(default)
        };
        Rules {
            name: text(&self.name),
            ext: text(&self.ext),
            find: text(&self.find),
            replace: text(&self.replace),
            regex: self.regex,
            ignore_case: self.ignore_case,
            case: self.case,
            counter_start: number(&self.start, 1),
            counter_step: number(&self.step, 1),
            counter_digits: number(&self.digits, 1).clamp(1, 12) as usize,
        }
    }

    fn recompute(&mut self, cx: &mut Context<Self>) {
        self.plan = plan(&self.paths, &self.rules(cx));
        cx.notify();
    }

    fn apply(&mut self, cx: &mut Context<Self>) {
        self.recompute(cx);
        if !self.plan.can_apply() {
            return;
        }
        match apply(&self.plan.rows) {
            Ok(()) => {
                let renamed = self.plan.changes();
                cx.emit(RenameEvent::Done {
                    dir: self.dir.clone(),
                    renamed,
                });
            }
            Err(err) => {
                self.error = Some(err.to_string());
                self.recompute(cx);
            }
        }
    }

    fn on_apply(&mut self, _: &ApplyRename, _: &mut Window, cx: &mut Context<Self>) {
        self.apply(cx);
    }

    fn on_cancel(&mut self, _: &CancelRename, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(RenameEvent::Cancelled);
    }

    // ----- rendering -------------------------------------------------------

    fn field(label: &'static str, input: &Entity<InputState>, muted: Hsla) -> Div {
        v_flex()
            .gap_1()
            .child(div().text_xs().text_color(muted).child(label))
            .child(Input::new(input).small().focus_bordered(false))
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(row) = self.plan.rows.get(ix) else {
            return div().into_any_element();
        };
        let (new_color, note) = match row.problem {
            Some(problem) => (theme.danger, Some(problem.label())),
            None if row.changes() => (theme.foreground, None),
            None => (theme.muted_foreground, None),
        };
        h_flex()
            .w_full()
            .h(rems(1.5))
            .px_2()
            .gap_2()
            .text_sm()
            .when(ix % 2 == 1, |r| r.bg(theme.list_even))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.muted_foreground)
                    .child(row.old.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(theme.muted_foreground)
                    .child("→"),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(new_color)
                            .child(row.new.clone()),
                    )
                    .when_some(note, |col, note| {
                        col.child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(theme.danger)
                                .child(note),
                        )
                    }),
            )
            .into_any_element()
    }
}

impl Render for RenameView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (bg, border, muted, fg, danger, list_head) = (
            theme.background,
            theme.border,
            theme.muted_foreground,
            theme.foreground,
            theme.danger,
            theme.list_head,
        );
        let count = self.plan.rows.len();
        let can_apply = self.plan.can_apply();
        let summary = format!(
            "{} · {}",
            tn!(
                "{n} of {total} changes",
                "{n} of {total} change",
                self.plan.changes(),
                total = count
            ),
            tn!("{n} problem", "{n} problems", self.plan.problems())
        );
        let error = self.error.clone().or_else(|| {
            self.plan
                .regex_error
                .as_ref()
                .map(|e| t!("Regex: {error}", error = e))
        });

        let case_buttons = CaseMode::ALL.iter().map(|&mode| {
            let button = Button::new(("case", mode as usize))
                .small()
                .label(mode.label());
            let button = if mode == self.case {
                button.primary()
            } else {
                button.ghost()
            };
            button.on_click(cx.listener(move |this, _, _, cx| {
                this.case = mode;
                this.recompute(cx);
            }))
        });

        let list = uniform_list(
            "rename-preview",
            count,
            cx.processor(move |this, range: Range<usize>, _window, cx| {
                range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
            }),
        )
        .size_full();

        v_flex()
            .id("rename-form")
            .key_context(RENAME_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_apply))
            .on_action(cx.listener(Self::on_cancel))
            .size_full()
            .min_h_0()
            .bg(bg)
            .p_3()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .items_baseline()
                    .child(div().font_semibold().child(self.title()))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(t!("in {dir}", dir = self.dir.display())),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .items_end()
                    .child(Self::field(t!("Name"), &self.name, muted).flex_1())
                    .child(Self::field(t!("Extension"), &self.ext, muted).w(rems(10.)))
                    .child(Self::field(t!("Counter start"), &self.start, muted).w(rems(6.)))
                    .child(Self::field(t!("Step"), &self.step, muted).w(rems(4.)))
                    .child(Self::field(t!("Digits"), &self.digits, muted).w(rems(4.))),
            )
            .child(
                h_flex()
                    .gap_3()
                    .items_end()
                    .child(Self::field(t!("Find##rename"), &self.find, muted).flex_1())
                    .child(Self::field(t!("Replace##rename"), &self.replace, muted).flex_1())
                    .child(
                        h_flex()
                            .gap_3()
                            .h(rems(1.75))
                            .child(
                                Checkbox::new("rename-regex")
                                    .label(t!("Regex"))
                                    .checked(self.regex)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.regex = *checked;
                                        this.recompute(cx);
                                    })),
                            )
                            .child(
                                Checkbox::new("rename-ignore-case")
                                    .label(t!("Ignore case"))
                                    .checked(self.ignore_case)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.ignore_case = *checked;
                                        this.recompute(cx);
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(div().text_xs().text_color(muted).child(t!("Case")))
                    .child(h_flex().gap_1().children(case_buttons))
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(muted).child(t!(
                        "[N] name · [E] extension · [C] counter · $1 regex groups"
                    ))),
            )
            .when_some(error, |this, error| {
                this.child(div().text_sm().text_color(danger).child(error))
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .border_1()
                    .border_color(border)
                    .rounded_sm()
                    .child(
                        h_flex()
                            .h(rems(1.5))
                            .px_2()
                            .gap_2()
                            .text_xs()
                            .text_color(muted)
                            .bg(list_head)
                            .border_b_1()
                            .border_color(border)
                            .child(div().flex_1().child(t!("Old name")))
                            // Same width as the arrow in the rows.
                            .child(div().flex_none().text_sm().invisible().child("→"))
                            .child(div().flex_1().child(t!("New name"))),
                    )
                    .child(div().flex_1().min_h_0().child(list)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().text_sm().text_color(muted).child(summary))
                    .child(
                        Button::new("rename-cancel")
                            .small()
                            .label(t!("Cancel"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(RenameEvent::Cancelled))),
                    )
                    .child(
                        Button::new("rename-apply")
                            .small()
                            .primary()
                            .icon(IconName::Check)
                            .label(t!("Rename"))
                            .disabled(!can_apply)
                            .on_click(cx.listener(|this, _, _, cx| this.apply(cx))),
                    ),
            )
            .text_color(fg)
    }
}
