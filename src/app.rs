//! The workspace: tabs (each with its own content), theme and persistence.

use std::path::PathBuf;
use std::time::Duration;

use crate::theme;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable::{ResizableState, h_resizable, resizable_panel, v_resizable};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::theme::Theme;
use gpui_kit::component::{IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::config::{CONFIG_VERSION, Config, TabConfig, ThemeChoice, WindowConfig};
use crate::fs::format::human_size;
use crate::grid_view::{GridEvent, GridView};
use crate::keymap::*;
use crate::layout::Layout;
use crate::ops::OpKind;
use crate::ops::manager::{ManagerEvent, OperationsManager, summary};
use crate::ops_panel::{OperationsPanel, PanelEvent as OpsPanelEvent};
use crate::sidebar::{Sidebar, SidebarEvent};
use crate::tabs::Tab;
use crate::ui::{h_flex, v_flex};

const SAVE_DEBOUNCE: Duration = Duration::from_secs(2);
/// Height of the tab bar, matching the sidebar header.
pub const HEADER_HEIGHT_REMS: f32 = 2.0;
const OPS_PANEL_HEIGHT: f32 = 220.0;
const SIDEBAR_WIDTH: f32 = 220.0;

/// A tab being dragged to a new position.
#[derive(Clone, Copy)]
struct DragTab {
    index: usize,
}

struct TabDragPreview {
    title: String,
}

impl Render for TabDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_3()
            .py_1()
            .rounded_md()
            .bg(theme.tab_active)
            .text_color(theme.tab_active_foreground)
            .border_1()
            .border_color(theme.border)
            .text_sm()
            .child(self.title.clone())
    }
}

/// What a button in the function key bar does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FKey {
    Rename,
    View,
    Edit,
    Copy,
    Move,
    MkDir,
    Delete,
    DeletePermanently,
    Terminal,
    Quit,
}

impl FKey {
    /// (key label, caption, enabled) for the bar, depending on Shift.
    fn slots(shift: bool) -> [(&'static str, &'static str, FKey, bool); 9] {
        if shift {
            [
                ("F2", "Rename", FKey::Rename, true),
                ("F3", "View", FKey::View, false),
                ("F4", "Edit", FKey::Edit, false),
                ("F5", "Copy", FKey::Copy, true),
                ("F6", "Rename", FKey::Rename, true),
                ("F7", "MkDir", FKey::MkDir, true),
                ("F8", "Delete!", FKey::DeletePermanently, true),
                ("F9", "Terminal", FKey::Terminal, true),
                ("F10", "Quit", FKey::Quit, true),
            ]
        } else {
            [
                ("F2", "Rename", FKey::Rename, true),
                ("F3", "View", FKey::View, false),
                ("F4", "Edit", FKey::Edit, false),
                ("F5", "Copy", FKey::Copy, true),
                ("F6", "Move", FKey::Move, true),
                ("F7", "MkDir", FKey::MkDir, true),
                ("F8", "Trash", FKey::Delete, true),
                ("F9", "Terminal", FKey::Terminal, true),
                ("F10", "Quit", FKey::Quit, true),
            ]
        }
    }
}
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
    sidebar: Entity<Sidebar>,
    sidebar_open: bool,
    /// Divider between the sidebar and the main column.
    side_split: Entity<ResizableState>,
    /// Finished operations to announce on the next render (needs the window).
    pending_notifications: Vec<(String, bool)>,
    window_cfg: Option<WindowConfig>,
    theme: ThemeChoice,
    scale: f32,
    /// Theme font sizes at scale 1.0, captured before any scaling.
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

        // Theme reloads (file watcher, mode switches) reset the font size to the
        // theme's own value; put the scale back whenever that happens.
        subscriptions.push(cx.observe_global::<Theme>(|this, cx| this.ensure_scale(cx)));

        // Custom themes load asynchronously; re-apply the chosen one once they are in.
        let weak = cx.weak_entity();
        theme::watch(cx, move |cx| {
            weak.update(cx, |this, cx| this.apply_theme(cx)).ok();
        });

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

        let sidebar = cx.new(|cx| Sidebar::new(cx));
        subscriptions.push(
            cx.subscribe(&sidebar, |this, _sidebar, event, cx| match event {
                SidebarEvent::Open(path) => this.open_place(path.clone(), cx),
            }),
        );

        let mut this = Self {
            tabs: Vec::new(),
            active_tab: 0,
            sidebar,
            sidebar_open: config.sidebar_open,
            side_split: cx.new(|_| ResizableState::default()),
            ops,
            ops_panel,
            ops_panel_open: false,
            main_split: cx.new(|_| ResizableState::default()),
            pending_notifications: Vec::new(),
            window_cfg: config.window,
            theme: config.theme.clone(),
            scale: 1.0,
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

    // ----- themes --------------------------------------------------------

    /// Apply `self.theme`, then rebase the UI scale on the theme's font sizes.
    fn apply_theme(&mut self, cx: &mut Context<Self>) {
        if !theme::apply(&self.theme, None, cx) {
            log::warn!(
                "theme {:?} is not available, using the system appearance",
                self.theme
            );
        }
        self.apply_scale(cx);
        cx.notify();
    }

    /// Unscaled font sizes of the active theme: what its config declares, or
    /// the kit defaults (16 / 13 px).
    fn base_font_sizes(cx: &App) -> (Pixels, Pixels) {
        let theme = cx.theme();
        let config = if theme.is_dark() {
            &theme.dark_theme
        } else {
            &theme.light_theme
        };
        (
            config.font_size.map(px).unwrap_or(px(16.)),
            config.mono_font_size.map(px).unwrap_or(px(13.)),
        )
    }

    /// Re-apply the scale when something (a theme reload) reset the font size.
    fn ensure_scale(&self, cx: &mut App) {
        let (base, base_mono) = Self::base_font_sizes(cx);
        let (want, want_mono) = (base * self.scale, base_mono * self.scale);
        let theme = cx.theme();
        if theme.font_size != want || theme.mono_font_size != want_mono {
            self.apply_scale(cx);
        }
    }

    fn set_theme(&mut self, choice: ThemeChoice, cx: &mut Context<Self>) {
        self.theme = choice;
        self.apply_theme(cx);
        self.schedule_save(cx);
    }

    /// A dialog listing the built-in and user themes; clicking one applies it.
    fn choose_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let themes = theme::available(cx);
        let current = self.theme.clone();
        let view = cx.entity();
        let dir = theme::themes_dir()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        window.open_alert_dialog(cx, move |alert, _window, cx| {
            let (themes, current, view, dir) =
                (themes.clone(), current.clone(), view.clone(), dir.clone());
            let (fg, muted, hover, accent) = {
                let t = cx.theme();
                (
                    t.foreground,
                    t.muted_foreground,
                    t.list_hover,
                    t.list_active,
                )
            };
            // The registry already lists the built-in pair as "Default Light/Dark".
            let mut choices: Vec<(String, ThemeChoice, String)> = vec![(
                "System".into(),
                ThemeChoice::System,
                "built-in light or dark, following the desktop".into(),
            )];
            for (name, mode) in &themes {
                choices.push((
                    name.clone(),
                    ThemeChoice::Named(name.clone()),
                    if mode.is_dark() {
                        "dark".into()
                    } else {
                        "light".into()
                    },
                ));
            }
            alert
                .title("Theme")
                .content(move |content, _, _| {
                    let rows = choices
                        .iter()
                        .enumerate()
                        .map(|(ix, (label, choice, note))| {
                            let selected = *choice == current;
                            let choice = choice.clone();
                            let view = view.clone();
                            h_flex()
                                .id(("theme-row", ix))
                                .w_full()
                                .px_2()
                                .py_1()
                                .gap_2()
                                .rounded_sm()
                                .cursor_pointer()
                                .when(selected, |d| d.bg(accent))
                                .hover(|s| s.bg(hover))
                                .on_click(move |_, window, cx| {
                                    let choice = choice.clone();
                                    view.update(cx, |this, cx| this.set_theme(choice, cx));
                                    window.close_dialog(cx);
                                })
                                .child(div().flex_1().text_color(fg).child(label.clone()))
                                .child(div().text_xs().text_color(muted).child(note.clone()))
                        });
                    content
                        .child(
                            div()
                                .id("theme-rows")
                                .max_h(rems(18.))
                                .overflow_y_scroll()
                                .text_sm()
                                .children(rows),
                        )
                        .child(
                            div()
                                .pt_2()
                                .text_xs()
                                .text_color(muted)
                                .child(format!("User themes: {dir}")),
                        )
                })
                .ok_text("Close")
        });
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
        let (base, base_mono) = Self::base_font_sizes(cx);
        let (font, mono) = (base * self.scale, base_mono * self.scale);
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

    /// Move the tab at `from` so that it ends up at position `to`.
    fn move_tab(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let to = to.min(self.tabs.len().saturating_sub(1));
        if from >= self.tabs.len() || from == to {
            return;
        }
        let active_id = self.active_tab;
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        // Keep the same tab active after the shuffle.
        self.active_tab = if active_id == from {
            to
        } else if from < active_id && active_id <= to {
            active_id - 1
        } else if to <= active_id && active_id < from {
            active_id + 1
        } else {
            active_id
        };
        self.resubscribe(cx);
        self.schedule_save(cx);
        cx.notify();
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

    // ----- sidebar -------------------------------------------------------

    /// Navigate the active panel of the active tab to a sidebar place.
    fn open_place(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let Some(view) = self.active_tab().grid_view() {
            let panel = view.read(cx).active_panel().clone();
            panel.update(cx, |panel, cx| panel.navigate_to(path, cx));
        }
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        if self.sidebar_open {
            self.sidebar.update(cx, |s, cx| {
                s.refresh();
                cx.notify();
            });
        }
        self.schedule_save(cx);
        cx.notify();
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
            theme: self.theme.clone(),
            ui_scale: (self.scale * 100.0).round() / 100.0,
            sidebar_open: self.sidebar_open,
            drop_action: crate::settings::get(cx).drop_action,
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
    fn on_toggle_theme(&mut self, _: &ToggleTheme, _window: &mut Window, cx: &mut Context<Self>) {
        let choice = if cx.theme().is_dark() {
            ThemeChoice::Light
        } else {
            ThemeChoice::Dark
        };
        self.set_theme(choice, cx);
    }
    fn on_choose_theme(&mut self, _: &ChooseTheme, window: &mut Window, cx: &mut Context<Self>) {
        self.choose_theme(window, cx);
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
    fn on_toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_sidebar(cx);
    }
    fn on_quit(&mut self, _: &Quit, _: &mut Window, cx: &mut Context<Self>) {
        cx.quit();
    }

    /// A click in the function key bar: act on the active grid as if the key
    /// had been pressed, with the focus back on its panel first.
    fn on_fkey(&mut self, key: FKey, window: &mut Window, cx: &mut Context<Self>) {
        if key == FKey::Quit {
            cx.quit();
            return;
        }
        let Some(grid) = self.active_tab().grid_view().cloned() else {
            return;
        };
        self.focus_active(window, cx);
        grid.update(cx, |grid, cx| match key {
            FKey::Rename => grid.start_rename(window, cx),
            FKey::Copy => grid.start_transfer(OpKind::Copy, window, cx),
            FKey::Move => grid.start_transfer(OpKind::Move, window, cx),
            FKey::MkDir => grid.start_mkdir(window, cx),
            FKey::Delete => grid.start_delete(false, window, cx),
            FKey::DeletePermanently => grid.start_delete(true, window, cx),
            FKey::Terminal => grid.active_panel().update(cx, |p, cx| p.open_terminal(cx)),
            FKey::View | FKey::Edit | FKey::Quit => {}
        });
    }

    // ----- rendering -----------------------------------------------------

    fn render_key_bar(&self, shift: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (bg, border, fg, muted_fg, hover, key_color) = (
            theme.status_bar,
            theme.status_bar_border,
            theme.foreground,
            theme.muted_foreground,
            theme.list_hover,
            theme.primary,
        );
        h_flex()
            .h(rems(1.75))
            .flex_none()
            .w_full()
            .bg(bg)
            .border_t_1()
            .border_color(border)
            .text_xs()
            .children(FKey::slots(shift).into_iter().enumerate().map(
                |(ix, (label, caption, key, enabled))| {
                    h_flex()
                        .id(("fkey", ix))
                        .flex_1()
                        .h_full()
                        .min_w_0()
                        .gap_1()
                        .px_2()
                        .border_r_1()
                        .border_color(border)
                        .cursor_pointer()
                        .when(enabled, |d| {
                            d.hover(|s| s.bg(hover)).on_click(
                                cx.listener(move |this, _, window, cx| {
                                    this.on_fkey(key, window, cx)
                                }),
                            )
                        })
                        .when(!enabled, |d| d.opacity(0.4))
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(key_color)
                                .child(label),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(if enabled { fg } else { muted_fg })
                                .child(caption),
                        )
                },
            ))
            .into_any_element()
    }

    /// Our own tab strip: tabs fill the bar's full height, the active one shares
    /// the panel background and has no line under it, so it joins the content.
    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (tab_bg, tab_fg, active_bg, active_fg, border, hover) = (
            theme.tab,
            theme.tab_foreground,
            theme.tab_active,
            theme.tab_active_foreground,
            theme.border,
            theme.list_hover,
        );
        let closable = self.tabs.len() > 1;
        let active_ix = self.active_tab;
        let drop_bg = theme.primary.alpha(0.35);

        let tabs = self.tabs.iter().enumerate().map(|(ix, tab)| {
            let title = tab.title(cx);
            let active = ix == active_ix;
            h_flex()
                .id(("tab", ix))
                .h_full()
                .pl_3()
                .pr_2()
                .gap_2()
                .flex_none()
                .cursor_pointer()
                .text_sm()
                .bg(if active { active_bg } else { tab_bg })
                .text_color(if active { active_fg } else { tab_fg })
                .border_r_1()
                .border_b_1()
                // The bottom line stays only under inactive tabs.
                .border_color(if active { active_bg } else { border })
                .when(!active, |d| d.hover(|s| s.bg(hover)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.activate_tab(ix, window, cx);
                }))
                // Drag to reorder: drop on another tab to take its position.
                .on_drag(DragTab { index: ix }, {
                    let title = title.clone();
                    move |_drag, _offset, _window, cx| {
                        let title = title.clone();
                        cx.new(|_| TabDragPreview { title })
                    }
                })
                .drag_over::<DragTab>(move |style, _, _, _| style.bg(drop_bg))
                .on_drop(cx.listener(move |this, drag: &DragTab, _window, cx| {
                    this.move_tab(drag.index, ix, cx);
                }))
                .child(div().min_w_0().truncate().child(title))
                .when(closable, |d| {
                    d.child(
                        div()
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
                            ),
                    )
                })
        });

        let new_tab = Button::new("tab-new")
            .ghost()
            .small()
            .icon(IconName::Plus)
            .on_click(cx.listener(|this, _, window, cx| this.new_tab(window, cx)));

        // Layout picker for the active tab. The menu dispatches `SetLayout` to the
        // active panel, so the grid handles it exactly like Alt+1..9.
        let (current_layout, panel_focus) = match self.active_tab().grid_view() {
            Some(view) => {
                let grid = view.read(cx);
                (
                    Some(grid.layout()),
                    Some(grid.active_panel().focus_handle(cx)),
                )
            }
            None => (None, None),
        };
        let layout_button = Button::new("tab-layout")
            .ghost()
            .small()
            .icon(IconName::LayoutDashboard)
            .label(current_layout.map(|l| l.label()).unwrap_or_default())
            .dropdown_menu(move |menu, _window, _cx| {
                let mut menu = match &panel_focus {
                    Some(focus) => menu.action_context(focus.clone()),
                    None => menu,
                };
                for layout in Layout::PRESETS {
                    menu = menu.menu_with_check(
                        format!("{} × {}", layout.rows, layout.cols),
                        current_layout == Some(layout),
                        Box::new(SetLayout {
                            rows: layout.rows,
                            cols: layout.cols,
                        }),
                    );
                }
                menu
            });

        h_flex()
            .id("tabs")
            .size_full()
            .items_stretch()
            .overflow_x_hidden()
            .children(tabs)
            // Filler and controls carry the bottom line the active tab omits.
            // Dropping a tab on the filler moves it to the end.
            .child(
                div()
                    .id("tabs-filler")
                    .flex_1()
                    .h_full()
                    .border_b_1()
                    .border_color(border)
                    .drag_over::<DragTab>(move |style, _, _, _| style.bg(drop_bg))
                    .on_drop(cx.listener(|this, drag: &DragTab, _window, cx| {
                        let last = this.tabs.len().saturating_sub(1);
                        this.move_tab(drag.index, last, cx);
                    })),
            )
            .child(
                h_flex()
                    .h_full()
                    .flex_none()
                    .px_1()
                    .gap_1()
                    .border_b_1()
                    .border_color(border)
                    .child(layout_button)
                    .child(new_tab),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.capture_window(window);
        self.ensure_scale(cx);
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
            "Cmd+T new tab · Cmd+W close · Ctrl+Tab / Cmd+1..9 tabs · Tab panel · Alt+1..9 layout · Cmd+L path · Cmd+Shift+. hidden · Cmd+Shift+T theme · Cmd+±/0 zoom · Cmd+Q quit"
        } else {
            "Ctrl+T new tab · Ctrl+W close · Ctrl+Tab / Ctrl+1..9 tabs · Tab panel · Alt+1..9 layout · Ctrl+L path · Ctrl+H hidden · Ctrl+Shift+T theme · Ctrl+±/0 zoom · Ctrl+Q quit"
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
        if self.sidebar_open {
            let current = self
                .active_tab()
                .grid_view()
                .map(|v| v.read(cx).active_panel().read(cx).path().to_path_buf());
            self.sidebar.update(cx, |s, cx| s.set_current(current, cx));
        }
        let shift = window.modifiers().shift;
        let key_bar = self.render_key_bar(shift, cx);

        let main_column = v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                div()
                    .h(rems(HEADER_HEIGHT_REMS))
                    .flex_none()
                    .w_full()
                    .bg(tab_bar)
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
            .child(key_bar);

        let sidebar_toggle = Button::new("sidebar-toggle")
            .ghost()
            .xsmall()
            .icon(if self.sidebar_open {
                IconName::PanelLeftClose
            } else {
                IconName::PanelLeftOpen
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)));

        v_flex()
            .key_context(WORKSPACE_CONTEXT)
            .on_action(cx.listener(Self::on_new_tab))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_next_tab))
            .on_action(cx.listener(Self::on_prev_tab))
            .on_action(cx.listener(Self::on_activate_tab))
            .on_action(cx.listener(Self::on_toggle_theme))
            .on_action(cx.listener(Self::on_choose_theme))
            .on_action(cx.listener(Self::on_zoom_in))
            .on_action(cx.listener(Self::on_zoom_out))
            .on_action(cx.listener(Self::on_zoom_reset))
            .on_action(cx.listener(Self::on_toggle_ops_panel))
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_quit))
            // Repaint when Shift changes so the key bar shows the Shift captions.
            .on_modifiers_changed(cx.listener(|_this, _ev, _window, cx| cx.notify()))
            .size_full()
            .bg(background)
            .text_color(foreground)
            .child(
                div().flex_1().min_h_0().w_full().child(
                    h_resizable("side-split")
                        .with_state(&self.side_split)
                        .child(
                            resizable_panel()
                                .size(px(SIDEBAR_WIDTH))
                                .visible(self.sidebar_open)
                                .child(self.sidebar.clone()),
                        )
                        .child(resizable_panel().child(main_column)),
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
                    .child(sidebar_toggle)
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
