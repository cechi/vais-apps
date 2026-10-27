//! The operations panel at the bottom of the window: one row per operation
//! with progress, a speed sparkline and a cancel button.

use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::fs::format::human_size;
use crate::ops::manager::{ManagerEvent, OpStatus, Operation, OperationsManager, SPEED_SAMPLES};
use crate::ui::{h_flex, v_flex};

pub enum PanelEvent {
    /// The close button was clicked.
    CloseRequested,
}

pub struct OperationsPanel {
    manager: Entity<OperationsManager>,
    _subscription: Subscription,
}

impl EventEmitter<PanelEvent> for OperationsPanel {}

/// Theme colours copied out so row rendering does not borrow `cx`.
#[derive(Clone, Copy)]
struct Colors {
    border: Hsla,
    muted: Hsla,
    muted_foreground: Hsla,
    primary: Hsla,
    success: Hsla,
    danger: Hsla,
    warning: Hsla,
    list_even: Hsla,
}

impl OperationsPanel {
    pub fn new(manager: Entity<OperationsManager>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&manager, |_this, _manager, event, cx| {
            if matches!(event, ManagerEvent::Changed) {
                cx.notify();
            }
        });
        Self {
            manager,
            _subscription: subscription,
        }
    }

    fn render_row(&self, op: &Operation, ix: usize, c: Colors, cx: &Context<Self>) -> AnyElement {
        let id = op.id;
        let progress = op.snapshot();
        let fraction = op.fraction();

        let (status_text, status_color) = match op.status {
            OpStatus::Running => (t!("Running"), c.primary),
            OpStatus::Cancelling => (t!("Cancelling…"), c.warning),
            OpStatus::Done => (t!("Done"), c.success),
            OpStatus::Failed => (t!("Failed"), c.danger),
            OpStatus::Cancelled => (t!("Cancelled"), c.warning),
        };
        let bar_color = match op.status {
            OpStatus::Failed => c.danger,
            OpStatus::Cancelled | OpStatus::Cancelling => c.warning,
            OpStatus::Done => c.success,
            OpStatus::Running => c.primary,
        };

        let progress_text = if progress.total_bytes > 0 {
            tn!(
                "{bytes} / {total_bytes}  ·  {done} / {n} item",
                "{bytes} / {total_bytes}  ·  {done} / {n} items",
                progress.total,
                bytes = human_size(progress.bytes),
                total_bytes = human_size(progress.total_bytes),
                done = progress.done
            )
        } else {
            tn!(
                "{done} / {n} item",
                "{done} / {n} items",
                progress.total,
                done = progress.done
            )
        };
        let detail = if op.is_running() {
            progress.current.clone()
        } else {
            op.report
                .as_ref()
                .and_then(|r| r.errors.first().cloned())
                .unwrap_or_default()
        };
        let speed_text = if op.is_running() && progress.total_bytes > 0 {
            format!("{}/s", human_size(op.speed as u64))
        } else if progress.total_bytes > 0 {
            let secs = op.elapsed().as_secs_f64().max(0.001);
            t!(
                "{speed}/s avg",
                speed = human_size((progress.bytes as f64 / secs) as u64)
            )
        } else {
            String::new()
        };
        let dest = op.dest.as_ref().map(|d| d.display()).unwrap_or_default();

        let max_speed = op.speed_samples.iter().cloned().fold(1.0_f64, f64::max);
        let sparkline = h_flex()
            .items_end()
            .gap(px(1.))
            .h(rems(1.25))
            .w(rems(6.))
            .flex_none()
            .children((0..SPEED_SAMPLES).map(|i| {
                let offset = SPEED_SAMPLES - op.speed_samples.len();
                let value = if i >= offset {
                    op.speed_samples[i - offset] / max_speed
                } else {
                    0.0
                };
                div()
                    .w(px(2.))
                    .h(DefiniteLength::Fraction((value as f32).clamp(0.04, 1.0)))
                    .bg(if value > 0.0 { bar_color } else { c.muted })
            }));

        let action: AnyElement = if op.is_running() {
            Button::new(("op-cancel", ix))
                .ghost()
                .xsmall()
                .icon(IconName::Close)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.manager.update(cx, |m, cx| m.cancel(id, cx));
                }))
                .into_any_element()
        } else {
            Button::new(("op-remove", ix))
                .ghost()
                .xsmall()
                .icon(IconName::Delete)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.manager.update(cx, |m, cx| m.remove(id, cx));
                }))
                .into_any_element()
        };

        h_flex()
            .id(("op-row", ix))
            .w_full()
            .h(rems(2.75))
            .px_2()
            .gap_3()
            .text_sm()
            .whitespace_nowrap()
            .when(ix % 2 == 1, |d| d.bg(c.list_even))
            .border_b_1()
            .border_color(c.border)
            .child(
                v_flex()
                    .w(rems(14.))
                    .min_w_0()
                    .flex_none()
                    .child(div().truncate().child(op.kind.title(&op.label)))
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(c.muted_foreground)
                            .child(dest),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div().w_full().h(px(6.)).rounded_sm().bg(c.muted).child(
                            div()
                                .h_full()
                                .rounded_sm()
                                .w(DefiniteLength::Fraction(fraction.clamp(0.0, 1.0)))
                                .bg(bar_color),
                        ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .text_color(c.muted_foreground)
                            .child(div().flex_none().child(progress_text))
                            .child(div().flex_1().min_w_0().truncate().child(detail)),
                    ),
            )
            .child(
                h_flex()
                    .w(rems(13.))
                    .flex_none()
                    .gap_2()
                    .child(sparkline)
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(c.muted_foreground)
                            .child(speed_text),
                    ),
            )
            .child(
                div()
                    .w(rems(4.))
                    .flex_none()
                    .text_xs()
                    .text_color(c.muted_foreground)
                    .child(format_duration(op.elapsed())),
            )
            .child(
                div()
                    .w(rems(5.5))
                    .flex_none()
                    .text_xs()
                    .text_color(status_color)
                    .child(status_text),
            )
            .child(div().flex_none().child(action))
            .into_any_element()
    }
}

fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs >= 60 {
        format!("{}:{:02}", secs / 60, secs % 60)
    } else {
        format!("{:.1} s", d.as_secs_f64())
    }
}

impl Render for OperationsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let c = Colors {
            border: theme.border,
            muted: theme.muted,
            muted_foreground: theme.muted_foreground,
            primary: theme.primary,
            success: theme.success,
            danger: theme.danger,
            warning: theme.warning,
            list_even: theme.list_even,
        };
        let (background, title_bar, foreground) =
            (theme.background, theme.title_bar, theme.foreground);

        let (running, total) = {
            let m = self.manager.read(cx);
            (m.running_count(), m.operations().len())
        };
        let title = if running > 0 {
            tn!(
                "Operations · {n} running",
                "Operations · {n} running",
                running
            )
        } else if total > 0 {
            tn!(
                "Operations · {n} finished",
                "Operations · {n} finished",
                total
            )
        } else {
            t!("Operations").to_string()
        };

        let rows: Vec<AnyElement> = {
            let cx: &Context<Self> = cx;
            self.manager
                .read(cx)
                .operations()
                .iter()
                .enumerate()
                .map(|(ix, op)| self.render_row(op, ix, c, cx))
                .collect()
        };
        let empty = rows.is_empty();

        v_flex()
            .size_full()
            .min_h_0()
            .bg(background)
            .text_color(foreground)
            .border_t_1()
            .border_color(c.border)
            .child(
                h_flex()
                    .h(rems(1.75))
                    .flex_none()
                    .px_2()
                    .gap_2()
                    .bg(title_bar)
                    .border_b_1()
                    .border_color(c.border)
                    .text_xs()
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        Button::new("ops-clear")
                            .ghost()
                            .xsmall()
                            .label(t!("Clear finished"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.manager.update(cx, |m, cx| m.clear_finished(cx));
                            })),
                    )
                    .child(
                        Button::new("ops-close")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .on_click(
                                cx.listener(|_this, _, _, cx| cx.emit(PanelEvent::CloseRequested)),
                            ),
                    ),
            )
            .child(
                div()
                    .id("ops-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(empty, |d| {
                        d.child(
                            div()
                                .p_3()
                                .text_sm()
                                .text_color(c.muted_foreground)
                                .child(t!("No operations yet. F5 copies, F6 moves, F8 deletes.")),
                        )
                    })
                    .children(rows),
            )
    }
}
