//! The Agents panel at the right edge of the window: sessions with coding
//! agents (or a plain shell) in an embedded terminal, one per tab.

pub mod detect;
pub mod launch;

use std::path::PathBuf;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::palette::{self, Palette, PaletteItem};
use crate::terminal::{TerminalEvent, TerminalView};
use crate::ui::{h_flex, v_flex};
use detect::{Agent, AgentKind};

pub enum AgentsEvent {
    /// The panel's close button.
    CloseRequested,
}

struct Session {
    kind: AgentKind,
    view: Entity<TerminalView>,
    _subscription: Subscription,
}

pub struct AgentsPanel {
    agents: Vec<Agent>,
    /// `VAIS_BRIDGE` and friends: how a session's agent reaches the app.
    bridge_env: Vec<(String, String)>,
    sessions: Vec<Session>,
    active: usize,
    /// Where new sessions start: the active file panel's directory.
    default_cwd: Option<PathBuf>,
    focus_handle: FocusHandle,
}

impl EventEmitter<AgentsEvent> for AgentsPanel {}

impl AgentsPanel {
    pub fn new(bridge_env: Vec<(String, String)>, cx: &mut Context<Self>) -> Self {
        Self {
            agents: detect::available(),
            bridge_env,
            sessions: Vec::new(),
            active: 0,
            default_cwd: None,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Look for installed agents again (the panel was opened).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.agents = detect::available();
        cx.notify();
    }

    /// The focus target: the active terminal, else the panel itself.
    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.sessions
            .get(self.active)
            .map(|s| s.view.read(cx).focus_handle().clone())
            .unwrap_or_else(|| self.focus_handle.clone())
    }

    /// The directory new sessions start in (set by the workspace each frame).
    pub fn set_default_cwd(&mut self, cwd: Option<PathBuf>) {
        self.default_cwd = cwd;
    }

    /// Start `agent` in a new session tab and focus it.
    pub fn start(&mut self, agent: &Agent, window: &mut Window, cx: &mut Context<Self>) {
        let launch = launch::plan(agent, self.default_cwd.clone(), &self.bridge_env);
        let view = cx.new(|cx| TerminalView::new(launch, cx));
        let subscription = cx.subscribe(
            &view,
            |_this, _view, event: &TerminalEvent, cx| match event {
                TerminalEvent::TitleChanged | TerminalEvent::Exited => cx.notify(),
            },
        );
        self.sessions.push(Session {
            kind: agent.kind,
            view: view.clone(),
            _subscription: subscription,
        });
        self.active = self.sessions.len() - 1;
        view.read(cx).focus_handle().clone().focus(window, cx);
        cx.notify();
    }

    /// The "+" button: pick an installed agent (or the shell).
    pub fn choose_and_start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let agents = self.agents.clone();
        if agents.is_empty() {
            return;
        }
        let items: Vec<PaletteItem> = agents
            .iter()
            .map(|a| {
                PaletteItem::new(a.kind.label())
                    .detail(a.path.to_string_lossy().into_owned())
                    .icon(IconName::Plus)
            })
            .collect();
        let this = cx.entity();
        let palette = cx.new(|cx| {
            Palette::pick(items, cx)
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    let Some(agent) = choice.item.and_then(|ix| agents.get(ix).cloned()) else {
                        return;
                    };
                    let this = this.clone();
                    window.defer(cx, move |window, cx| {
                        this.update(cx, |panel, cx| panel.start(&agent, window, cx));
                    });
                })
                .on_cancel(|window, cx| window.close_dialog(cx))
        });
        palette::open("New agent session".into(), palette, window, cx);
    }

    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix < self.sessions.len() {
            self.active = ix;
            self.sessions[ix]
                .view
                .read(cx)
                .focus_handle()
                .clone()
                .focus(window, cx);
            cx.notify();
        }
    }

    fn close_session(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.sessions.len() {
            return;
        }
        self.sessions.remove(ix);
        if self.active >= self.sessions.len() {
            self.active = self.sessions.len().saturating_sub(1);
        }
        if let Some(session) = self.sessions.get(self.active) {
            session
                .view
                .read(cx)
                .focus_handle()
                .clone()
                .focus(window, cx);
        } else {
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    fn session_title(&self, ix: usize, cx: &App) -> String {
        let session = &self.sessions[ix];
        let view = session.view.read(cx);
        let base = session.kind.label();
        let state = if view.is_running() { "" } else { " (exited)" };
        if view.title().is_empty() {
            format!("{base}{state}")
        } else {
            format!("{base}: {}{state}", view.title())
        }
    }
}

impl Render for AgentsPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (bg, fg, border, muted, accent, hover) = (
            theme.sidebar,
            theme.sidebar_foreground,
            theme.sidebar_border,
            theme.sidebar_foreground.alpha(0.7),
            theme.sidebar_accent,
            theme.list_hover,
        );
        let titles: Vec<String> = (0..self.sessions.len())
            .map(|ix| self.session_title(ix, cx))
            .collect();
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
                        Button::new("agents-new")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.choose_and_start(window, cx)
                                }),
                            ),
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
            // Session tabs.
            .when(!self.sessions.is_empty(), |d| {
                d.child(
                    h_flex()
                        .flex_none()
                        .w_full()
                        .h(rems(1.75))
                        .overflow_x_hidden()
                        .border_b_1()
                        .border_color(border)
                        .children(titles.into_iter().enumerate().map(|(ix, title)| {
                            let active = ix == self.active;
                            h_flex()
                                .id(("agent-tab", ix))
                                .h_full()
                                .px_2()
                                .gap_1()
                                .flex_none()
                                .max_w(rems(14.))
                                .cursor_pointer()
                                .text_xs()
                                .border_r_1()
                                .border_color(border)
                                .when(active, |d| d.bg(accent))
                                .when(!active, |d| d.text_color(muted).hover(|s| s.bg(hover)))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.activate(ix, window, cx)
                                }))
                                .child(div().min_w_0().truncate().child(title))
                                .child(
                                    div()
                                        .id(("agent-tab-close", ix))
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .child(
                                            Button::new(("agent-tab-close-btn", ix))
                                                .ghost()
                                                .xsmall()
                                                .icon(IconName::Close)
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.close_session(ix, window, cx)
                                                    },
                                                )),
                                        ),
                                )
                        })),
                )
            })
            .child(match self.sessions.get(self.active) {
                Some(session) => div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(session.view.clone())
                    .into_any_element(),
                None => self.render_welcome(muted, hover, cx).into_any_element(),
            })
    }
}

impl AgentsPanel {
    /// No session yet: the installed agents, one click away.
    fn render_welcome(&self, muted: Hsla, hover: Hsla, cx: &mut Context<Self>) -> Div {
        v_flex()
            .flex_1()
            .min_h_0()
            .p_3()
            .gap_1()
            .text_sm()
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("Start a session with"),
            )
            .children(self.agents.iter().cloned().enumerate().map(|(ix, agent)| {
                let (kind, path_text) = (agent.kind, agent.path.to_string_lossy().into_owned());
                h_flex()
                    .id(("agent-start", ix))
                    .gap_2()
                    .h(rems(1.75))
                    .px_2()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(hover))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.start(&agent, window, cx)),
                    )
                    .child(
                        Icon::new(if kind == AgentKind::Shell {
                            Lucide::SquareTerminal
                        } else {
                            Lucide::Bot
                        })
                        .size_4()
                        .flex_none()
                        .text_color(muted),
                    )
                    .child(div().flex_none().child(kind.label()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(muted)
                            .child(path_text),
                    )
            }))
    }
}
