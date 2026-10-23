//! The Agents panel at the right edge of the window: sessions with coding
//! agents (or a plain shell) in an embedded terminal. This first step only
//! lists what is installed; the terminal and sessions follow.

pub mod detect;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::*;

use crate::ui::{h_flex, v_flex};
use detect::{Agent, AgentKind};

pub enum AgentsEvent {
    /// The panel's close button.
    CloseRequested,
}

pub struct AgentsPanel {
    agents: Vec<Agent>,
    focus_handle: FocusHandle,
}

impl EventEmitter<AgentsEvent> for AgentsPanel {}

impl AgentsPanel {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            agents: detect::available(),
            focus_handle: cx.focus_handle(),
        }
    }

    /// Look for installed agents again (the panel was opened).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.agents = detect::available();
        cx.notify();
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }
}

impl Render for AgentsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (bg, fg, border, muted) = (
            theme.sidebar,
            theme.sidebar_foreground,
            theme.sidebar_border,
            theme.sidebar_foreground.alpha(0.7),
        );
        v_flex()
            .size_full()
            .min_w_0()
            .bg(bg)
            .text_color(fg)
            .border_l_1()
            .border_color(border)
            .track_focus(&self.focus_handle)
            .child(
                h_flex()
                    .h(rems(crate::app::HEADER_HEIGHT_REMS))
                    .flex_none()
                    .px_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Agents"),
                    )
                    .child(
                        Button::new("agents-close")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.emit(AgentsEvent::CloseRequested)),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .p_3()
                    .gap_1()
                    .text_sm()
                    .child(div().text_xs().text_color(muted).child("Installed"))
                    .children(self.agents.iter().map(|agent| {
                        h_flex()
                            .gap_2()
                            .h(rems(1.75))
                            .child(
                                Icon::new(if agent.kind == AgentKind::Shell {
                                    Lucide::SquareTerminal
                                } else {
                                    Lucide::Bot
                                })
                                .size_4()
                                .flex_none()
                                .text_color(muted),
                            )
                            .child(div().flex_none().child(agent.kind.label()))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(agent.path.to_string_lossy().into_owned()),
                            )
                    })),
            )
    }
}
