//! The workspace: tabs (each with its own content), theme and persistence.

use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable::{ResizableState, resizable_panel, v_resizable};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::tab::{Tab as TabItem, TabBar};
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::component::{IconName, Sizable as _};
use gpui_kit::*;

use crate::config::{CONFIG_VERSION, Config, TabConfig, ThemeChoice, WindowConfig};
use crate::fs::format::human_size;
use crate::grid_view::{GridEvent, GridView};
use crate::keymap::*;
use crate::ops::manager::{ManagerEvent, OperationsManager, summary};
use crate::ops_panel::{OperationsPanel, PanelEvent as OpsPanelEvent};
use crate::tabs::Tab;
use crate::ui::{h_flex, v_flex};

const SAVE_DEBOUNCE: Duration = Duration::from_secs(2);
const TAB_BAR_HEIGHT_REMS: f32 = 2.0;
const OPS_PANEL_HEIGHT: f32 = 220.0;
pub const MIN_SCALE: f32 = 0.75;
pub const MAX_SCALE: f32 = 2.0;
const SCALE_STEP: f32 = 0.05;

pub struct Workspace {
    tabs: Vec<Tab>,
    active_tab: usize,
    ops: Entity<OperationsManager>,
    ops_panel: Entity<OperationsPanel>,
    ops_panel_open: bool,
    /// Divider between the tab content and the operations panel.
    main_split: Entity<ResizableState>,
    /// Finished operations to announce on the next render (needs the window).
    pending_notifications: Vec<(String, bool)>,
    window_cfg: Option<WindowConfig>,
    theme: ThemeChoice,
    scale: f32,
    /// Theme font sizes at scale 1.0, captured before any scaling.
    base_font_size: Pixels,
    base_mono_font_size: Pixels,
    scale_slider: Entity<SliderState>,
    save_task: Option<Task<()>>,
    tab_subscriptions: Vec<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(config: Config, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.on_app_quit(|this, cx| {
            this.save_now(cx);
            std::future::ready(())
        }));

        let scale = clamp_scale(config.ui_scale);
        let scale_slider = cx.new(|_| {
            SliderState::new()
                .min(MIN_SCALE)
                .max(MAX_SCALE)
                .step(SCALE_STEP)
                .default_value(scale)
        });
        subscriptions.push(cx.subscribe(&scale_slider, |this, _slider, event, cx| {
            if let SliderEvent::Change(value) = event {
                this.set_scale(value.start(), cx);
            }
        }));

        let ops = cx.new(|_| OperationsManager::new());
        let ops_panel = cx.new(|cx| OperationsPanel::new(ops.clone(), cx));
        subscriptions.push(cx.subscribe(&ops, |this, ops, event, cx| match event {
            ManagerEvent::Started => {
                this.ops_panel_open = true;
                cx.notify();
            }
            ManagerEvent::Finished(id) => {
                if let Some(op) = ops.read(cx).get(*id) {
                    let failed = op
                        .report
                        .as_ref()
                        .is_some_and(|r| !r.errors.is_empty() || r.cancelled);
                    this.pending_notifications.push((summary(op), failed));
                }
                cx.notify();
            }
            ManagerEvent::Changed => cx.notify(),
        }));
        subscriptions.push(
            cx.subscribe(&ops_panel, |this, _panel, event, cx| match event {
                OpsPanelEvent::CloseRequested => {
                    this.ops_panel_open = false;
                    cx.notify();
                }
            }),
        );

        let mut this = Self {
            tabs: Vec::new(),
            active_tab: 0,
            ops,
            ops_panel,
            ops_panel_open: false,
            main_split: cx.new(|_| ResizableState::default()),
            pending_notifications: Vec::new(),
            window_cfg: config.window,
            theme: config.theme,
            scale: 1.0,
            base_font_size: cx.theme().font_size,
            base_mono_font_size: cx.theme().mono_font_size,
            scale_slider,
            save_task: None,
            tab_subscriptions: Vec::new(),
            _subscriptions: subscriptions,
        };
        this.set_scale(scale, cx);
        for tab_cfg in config.tabs() {
            this.push_grid_tab(&tab_cfg, window, cx);
        }
        this.active_tab = config.active_tab.min(this.tabs.len() - 1);
        this
    }

    // ----- UI scale ------------------------------------------------------

    /// Scale the whole UI: gpui-kit's `Root` sets the window rem size from the
    /// theme font size every frame, and all our sizes are in rems.
    fn set_scale(&mut self, scale: f32, cx: &mut Context<Self>) {
        let scale = clamp_scale(scale);
        self.scale = scale;
        self.apply_scale(cx);
        self.schedule_save(cx);
        cx.notify();
    }

    fn apply_scale(&self, cx: &mut App) {
        let (font, mono) = (
            self.base_font_size * self.scale,
            self.base_mono_font_size * self.scale,
        );
        Theme::update(cx, |theme| {
            theme.font_size = font;
            theme.mono_font_size = mono;
        });
    }

    fn step_scale(&mut self, delta: f32, window: &mut Window, cx: &mut Context<Self>) {
        let target = if delta == 0.0 {
            1.0
        } else {
            self.scale + delta
        };
        let scale = clamp_scale((target / SCALE_STEP).round() * SCALE_STEP);
        self.scale_slider
            .update(cx, |slider, cx| slider.set_value(scale, window, cx));
        self.set_scale(scale, cx);
    }

    pub fn active_tab(&self) -> &Tab {
        &self.tabs[self.active_tab]
    }

    pub fn focus_active(&self, window: &mut Window, cx: &mut App) {
        self.active_tab().focus(window, cx);
    }

    fn push_grid_tab(&mut self, cfg: &TabConfig, window: &mut Window, cx: &mut Context<Self>) {
        let ops = self.ops.clone();
        let view = cx.new(|cx| GridView::new(cfg, ops, window, cx));
        self.tabs.push(Tab::grid(view));
        self.resubscribe(cx);
    }

    /// Tabs come and go, so the subscriptions are rebuilt from the list.
    fn resubscribe(&mut self, cx: &mut Context<Self>) {
        self.tab_subscriptions = self
            .tabs
            .iter()
            .filter_map(|tab| tab.grid_view())
            .map(|view| {
                cx.subscribe(view, |this, _view, event, cx| match event {
                    GridEvent::Changed => {
                        this.schedule_save(cx);
                        cx.notify();
                    }
                })
            })
            .collect();
    }

    fn activate_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.active_tab = ix;
        self.focus_active(window, cx);
        self.schedule_save(cx);
        cx.notify();
    }

    /// Open a new tab next to the current one, as a copy of its layout and paths.
    fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cfg = self.active_tab().config(cx);
        let ops = self.ops.clone();
        let view = cx.new(|cx| GridView::new(&cfg, ops, window, cx));
        let ix = self.active_tab + 1;
        self.tabs.insert(ix, Tab::grid(view));
        self.resubscribe(cx);
        self.activate_tab(ix, window, cx);
    }

    fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() <= 1 || ix >= self.tabs.len() {
            return;
        }
        self.tabs.remove(ix);
        self.resubscribe(cx);
        let next = if ix < self.active_tab || self.active_tab >= self.tabs.len() {
            self.active_tab.saturating_sub(1)
        } else {
            self.active_tab
        };
        self.activate_tab(next, window, cx);
    }

    // ----- persistence ---------------------------------------------------

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DEBOUNCE).await;
            this.update(cx, |ws, cx| ws.save_now(cx)).ok();
        }));
    }

    fn save_now(&mut self, cx: &mut Context<Self>) {
        let config = Config {
            version: CONFIG_VERSION,
            theme: self.theme,
            ui_scale: (self.scale * 100.0).round() / 100.0,
            window: self.window_cfg,
            active_tab: self.active_tab,
            tabs: self.tabs.iter().map(|t| t.config(cx)).collect(),
            ..Default::default()
        };
        if let Err(err) = config.save() {
            log::error!("failed to save config: {err:#}");
        }
    }

    fn capture_window(&mut self, window: &Window) {
        let (bounds, maximized) = match window.window_bounds() {
            WindowBounds::Windowed(b) => (b, false),
            WindowBounds::Maximized(b) => (b, true),
            WindowBounds::Fullscreen(b) => (b, false),
        };
        self.window_cfg = Some(WindowConfig {
            x: bounds.origin.x.as_f32(),
            y: bounds.origin.y.as_f32(),
            w: bounds.size.width.as_f32(),
            h: bounds.size.height.as_f32(),
            maximized,
        });
    }

    // ----- action handlers ----------------------------------------------

    fn on_new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        self.new_tab(window, cx);
    }
    fn on_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tab(self.active_tab, window, cx);
    }
    fn on_next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        let ix = (self.active_tab + 1) % self.tabs.len();
        self.activate_tab(ix, window, cx);
    }
    fn on_prev_tab(&mut self, _: &PrevTab, window: &mut Window, cx: &mut Context<Self>) {
        let ix = (self.active_tab + self.tabs.len() - 1) % self.tabs.len();
        self.activate_tab(ix, window, cx);
    }
    fn on_activate_tab(
        &mut self,
        action: &ActivateTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activate_tab(action.index, window, cx);
    }
    fn on_toggle_theme(&mut self, _: &ToggleTheme, window: &mut Window, cx: &mut Context<Self>) {
        let (mode, choice) = if cx.theme().is_dark() {
            (ThemeMode::Light, ThemeChoice::Light)
        } else {
            (ThemeMode::Dark, ThemeChoice::Dark)
        };
        Theme::change(mode, Some(window), cx);
        // A theme change resets the typography tokens, so re-apply the scale.
        self.apply_scale(cx);
        self.theme = choice;
        self.schedule_save(cx);
        cx.notify();
    }
    fn on_zoom_in(&mut self, _: &ZoomIn, window: &mut Window, cx: &mut Context<Self>) {
        self.step_scale(SCALE_STEP, window, cx);
    }
    fn on_zoom_out(&mut self, _: &ZoomOut, window: &mut Window, cx: &mut Context<Self>) {
        self.step_scale(-SCALE_STEP, window, cx);
    }
    fn on_zoom_reset(&mut self, _: &ZoomReset, window: &mut Window, cx: &mut Context<Self>) {
        self.step_scale(0.0, window, cx);
    }
    fn on_toggle_ops_panel(
        &mut self,
        _: &ToggleOperationsPanel,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ops_panel_open = !self.ops_panel_open;
        cx.notify();
    }
    fn on_quit(&mut self, _: &Quit, _: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }

    // ----- rendering -----------------------------------------------------

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let closable = self.tabs.len() > 1;
        let items = self.tabs.iter().enumerate().map(|(ix, tab)| {
            let title = tab.title(cx);
            let close = div()
                .id(("tab-close", ix))
                // Keep the click from also activating the tab.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    Button::new(("tab-close-btn", ix))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.close_tab(ix, window, cx);
                        })),
                );
            let mut item = TabItem::new().label(title);
            if closable {
                item = item.suffix(close);
            }
            item
        });

        let new_tab = Button::new("tab-new")
            .ghost()
            .small()
            .icon(IconName::Plus)
            .on_click(cx.listener(|this, _, window, cx| this.new_tab(window, cx)));

        TabBar::new("tabs")
            .small()
            .selected_index(self.active_tab)
            .on_click(cx.listener(|this, ix: &usize, window, cx| {
                this.activate_tab(*ix, window, cx);
            }))
            .children(items)
            .suffix(new_tab)
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.capture_window(window);
        for (message, failed) in std::mem::take(&mut self.pending_notifications) {
            let note = if failed {
                Notification::error(message)
            } else {
                Notification::success(message)
            };
            window.push_notification(note, cx);
        }

        let theme = cx.theme();
        let (background, foreground, tab_bar, status_bar, border, muted_fg) = (
            theme.background,
            theme.foreground,
            theme.tab_bar,
            theme.status_bar,
            theme.status_bar_border,
            theme.muted_foreground,
        );

        let ops_status = {
            let m = self.ops.read(cx);
            match m.running_count() {
                0 => None,
                n => Some(format!(
                    "{n} running · {}/s",
                    human_size(m.total_speed() as u64)
                )),
            }
        };
        let status_left = match self.active_tab().grid_view() {
            Some(_) if ops_status.is_some() => ops_status.unwrap_or_default(),
            Some(view) => {
                let grid = view.read(cx);
                format!(
                    "Tab {}/{}  ·  Layout {}  ·  Panel {}/{}",
                    self.active_tab + 1,
                    self.tabs.len(),
                    grid.layout().label(),
                    grid.active_index() + 1,
                    grid.layout().count()
                )
            }
            None => format!("Tab {}/{}", self.active_tab + 1, self.tabs.len()),
        };
        let status_right = if cfg!(target_os = "macos") {
            "Cmd+T new tab · Cmd+W close · Ctrl+Tab / Cmd+1..9 tabs · Tab panel · Alt+1..9 layout · Cmd+L path · Cmd+Shift+. hidden · Cmd+Shift+D theme · Cmd+±/0 zoom · Cmd+Q quit"
        } else {
            "Ctrl+T new tab · Ctrl+W close · Ctrl+Tab / Ctrl+1..9 tabs · Tab panel · Alt+1..9 layout · Ctrl+L path · Ctrl+H hidden · Ctrl+Shift+D theme · Ctrl+±/0 zoom · Ctrl+Q quit"
        };
        let scale_label = format!("{:.0} %", self.scale * 100.0);
        let scale_control = h_flex()
            .flex_none()
            .gap_2()
            .child(div().flex_none().child("UI"))
            .child(Slider::new(&self.scale_slider).w(rems(7.)))
            .child(
                div()
                    .flex_none()
                    .w(rems(2.5))
                    .text_right()
                    .child(scale_label),
            );

        let content = self.active_tab().render();

        v_flex()
            .key_context(WORKSPACE_CONTEXT)
            .on_action(cx.listener(Self::on_new_tab))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_prev_tab))
            .on_action(cx.listener(Self::on_activate_tab))
            .on_action(cx.listener(Self::on_toggle_theme))
            .on_action(cx.listener(Self::on_zoom_in))
            .on_action(cx.listener(Self::on_zoom_out))
            .on_action(cx.listener(Self::on_zoom_reset))
            .on_action(cx.listener(Self::on_toggle_ops_panel))
            .on_action(cx.listener(Self::on_quit))
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child(
                div()
                    .h(rems(TAB_BAR_HEIGHT_REMS))
                    .flex_none()
                    .w_full()
                    .bg(tab_bar)
                    .border_b_1()
                    .border_color(border)
                    .child(self.render_tab_bar(cx)),
            )
            .child(
                div().flex_1().min_h_0().w_full().child(
                    v_resizable("main-split")
                        .with_state(&self.main_split)
                        .child(resizable_panel().child(content))
                        .child(
                            resizable_panel()
                                .size(px(OPS_PANEL_HEIGHT))
                                .visible(self.ops_panel_open)
                                .child(self.ops_panel.clone()),
                        ),
                ),
            )
            .child(
                h_flex()
                    .h(rems(1.5))
                    .flex_none()
                    .px_3()
                    .gap_4()
                    .bg(status_bar)
                    .border_t_1()
                    .border_color(border)
                    .text_xs()
                    .text_color(muted_fg)
                    .child(div().flex_none().child(status_left))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_right()
                            .child(status_right),
                    )
                    .child(scale_control),
            )
    }
}

fn clamp_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(MIN_SCALE, MAX_SCALE)
    } else {
        1.0
    }
}
