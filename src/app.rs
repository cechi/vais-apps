//! The workspace: tabs (each with its own content), theme and persistence.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use futures::StreamExt as _;
use std::time::Duration;

use crate::theme;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable::{ResizableState, h_resizable, resizable_panel, v_resizable};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::theme::Theme;
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

mod tools;

use crate::agents::{AgentsEvent, AgentsPanel};
use crate::bridge::BridgeAddr;
use crate::command_line::{CommandLine, CommandLineEvent};
use crate::config::{
    CONFIG_VERSION, Config, Favorite, FavoriteLayout, TabConfig, ThemeChoice, WindowConfig,
};
use crate::connections::{ConnectionEvent, ConnectionManager, Status};
use crate::connections_view::{ConnectionsEvent, ConnectionsView};
use crate::editor_view::{EditorEvent, EditorView};
use crate::fs::format::human_size;
use crate::grid_view::{GridEvent, GridView};
use crate::keymap::*;
use crate::layout::Layout;
use crate::ops::OpKind;
use crate::ops::manager::{ManagerEvent, OperationId, OperationsManager, summary};
use crate::ops_panel::{OperationsPanel, PanelEvent as OpsPanelEvent};
use crate::palette::{self, Palette, PaletteItem};
use crate::rename::view::{RenameEvent, RenameView};
use crate::settings_view::{SettingsEvent, SettingsValues, SettingsView};
use crate::sidebar::{FavoriteId, Sidebar, SidebarEvent};
use crate::tabs::Tab;
use crate::ui::{h_flex, v_flex};
use crate::vfs::{ConnId, Location};

const SAVE_DEBOUNCE: Duration = Duration::from_secs(2);
/// Height of the tab bar, matching the sidebar header.
pub const HEADER_HEIGHT_REMS: f32 = 2.0;
const OPS_PANEL_HEIGHT: f32 = 220.0;
const SIDEBAR_WIDTH: f32 = 220.0;

/// What a palette does with the user's choice once the dialog is closed.
type Apply<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

/// The texts of a palette asking for a name (see `Workspace::ask_name`).
struct NameAsk {
    title: &'static str,
    placeholder: &'static str,
    hint: &'static str,
    /// The second row while a name is fixed now (else it is "Cancel").
    reset_label: &'static str,
    current: String,
    /// Whether a user-given name is set now, so it can be reset.
    fixed: bool,
}

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
                ("F3", "View", FKey::View, true),
                ("F4", "Edit", FKey::Edit, true),
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
                ("F3", "View", FKey::View, true),
                ("F4", "Edit", FKey::Edit, true),
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
    /// The sidebar's width as last dragged; it is laid out from this again
    /// after being hidden.
    sidebar_width: f32,
    /// Whether the sidebar's Favorites group starts with the system places.
    system_places: bool,
    command_line: Entity<CommandLine>,
    command_line_open: bool,
    /// Where the bridge for agents listens (`None` when it could not start).
    bridge_addr: Option<BridgeAddr>,
    /// Tool calls waiting for an operation to finish.
    bridge_waits: HashMap<OperationId, crate::bridge::Reply>,
    /// The Agents panel at the right edge (terminals with coding agents).
    agents: Entity<AgentsPanel>,
    agents_open: bool,
    agents_width: f32,
    connections: Entity<ConnectionManager>,
    /// Directories the focused panel showed, newest first (the sidebar's
    /// Recent group); `last_visited` is what was noted last.
    recent: Vec<Location>,
    last_visited: Option<Location>,
    /// Bookmarked directories (Ctrl+D).
    favorites: Vec<Favorite>,
    /// Whole tabs saved from a tab's context menu.
    favorite_layouts: Vec<FavoriteLayout>,
    /// Personal font family from the config, applied over every theme.
    font_family: Option<String>,
    /// Divider between the sidebar and the main column.
    side_split: Entity<ResizableState>,
    /// Finished operations to announce on the next render (needs the window).
    pending_notifications: Vec<(String, bool)>,
    /// Ctrl+wheel distance not yet turned into a zoom step (touchpads send
    /// many small deltas).
    wheel_zoom: f32,
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
            if let Some(addr) = &this.bridge_addr {
                crate::bridge::cleanup(addr);
            }
            std::future::ready(())
        }));

        // The bridge: tool calls from agents, carried out on the foreground.
        let bridge_addr = match crate::bridge::start() {
            Ok((addr, mut requests)) => {
                cx.spawn_in(window, async move |this, cx| {
                    while let Some(request) = requests.next().await {
                        let handled = this.update_in(cx, |this, window, cx| {
                            this.handle_tool(request, window, cx)
                        });
                        if handled.is_err() {
                            break;
                        }
                    }
                })
                .detach();
                log::info!("bridge listening: {addr:?}");
                Some(addr)
            }
            Err(err) => {
                log::warn!("bridge not started: {err}");
                None
            }
        };

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
                    this.tool_operation_finished(op);
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

        let sidebar =
            cx.new(|cx| Sidebar::new(config.sidebar_collapsed.clone(), config.system_places, cx));
        sidebar.update(cx, |s, cx| {
            s.set_recent(&config.recent, cx);
            s.set_favorites(&config.favorites, cx);
            s.set_layouts(&config.favorite_layouts, cx);
        });
        subscriptions.push(cx.subscribe_in(
            &sidebar,
            window,
            |this, _sidebar, event, window, cx| match event {
                SidebarEvent::Open(path) => this.open_place(path.clone(), cx),
                SidebarEvent::OpenLayout(ix) => this.open_layout(*ix, window, cx),
                SidebarEvent::RemoveFavorite(path) => {
                    this.toggle_favorite(path.clone(), window, cx)
                }
                SidebarEvent::RemoveLayout(ix) => this.remove_layout(*ix, window, cx),
                SidebarEvent::RenameFavorite(id) => this.rename_favorite(id.clone(), window, cx),
                SidebarEvent::ColorFavorite(id) => this.pick_favorite_color(id.clone(), window, cx),
                SidebarEvent::ManageConnections => this.open_connections(window, cx),
                SidebarEvent::Changed => this.schedule_save(cx),
            },
        ));
        let connections = crate::connections::manager(cx);
        subscriptions.push(cx.subscribe_in(
            &connections,
            window,
            |this, _manager, event, window, cx| match event {
                ConnectionEvent::Changed(conn) => {
                    for grid in this.tabs.iter().filter_map(|t| t.grid_view()) {
                        grid.update(cx, |grid, cx| grid.refresh_conn(conn, cx));
                    }
                    this.sync_connections(cx);
                }
                ConnectionEvent::NeedsPassword(conn) => this.ask_password(conn.clone(), window, cx),
            },
        ));
        let watcher = crate::watch::watcher(cx);
        subscriptions.push(cx.subscribe(&watcher, |this, _watcher, event, cx| {
            let crate::watch::WatchEvent::Changed(dirs) = event;
            for grid in this.tabs.iter().filter_map(|t| t.grid_view()) {
                grid.update(cx, |grid, cx| grid.refresh_changed(dirs, cx));
            }
        }));

        let command_line =
            cx.new(|cx| CommandLine::new(config.command_history.clone(), window, cx));
        subscriptions.push(cx.subscribe_in(
            &command_line,
            window,
            |this, _line, event, window, cx| match event {
                CommandLineEvent::Run(command) => this.run_command(command.clone(), window, cx),
                CommandLineEvent::Cancelled => this.focus_active_panel(window, cx),
            },
        ));

        let bridge_env = bridge_addr
            .as_ref()
            .map(BridgeAddr::env)
            .unwrap_or_default();
        let agents = cx.new(|cx| AgentsPanel::new(bridge_env, cx));
        // While the focus is in the Agents panel, the active file panel
        // keeps its active look: the user and the agent see the same thing.
        let agents_root = agents.read(cx).root_focus_handle().clone();
        subscriptions.push(cx.on_focus_in(&agents_root, window, |this, _, cx| {
            this.lend_focus(true, cx)
        }));
        subscriptions.push(cx.on_focus_out(&agents_root, window, |this, _, _, cx| {
            this.lend_focus(false, cx)
        }));
        subscriptions.push(
            cx.subscribe_in(
                &agents,
                window,
                |this, _panel, event, window, cx| match event {
                    AgentsEvent::CloseRequested => this.toggle_agents(window, cx),
                },
            ),
        );

        let mut this = Self {
            tabs: Vec::new(),
            active_tab: 0,
            sidebar,
            sidebar_open: config.sidebar_open,
            sidebar_width: SIDEBAR_WIDTH,
            system_places: config.system_places,
            command_line,
            command_line_open: config.command_line,
            bridge_addr,
            bridge_waits: HashMap::new(),
            agents,
            agents_open: config.agents_open,
            agents_width: if config.agents_width.is_finite() && config.agents_width >= 200.0 {
                config.agents_width
            } else {
                520.0
            },
            connections,
            recent: config.recent.clone(),
            last_visited: None,
            favorites: config.favorites.clone(),
            favorite_layouts: config.favorite_layouts.clone(),
            font_family: config.font_family.clone(),
            side_split: cx.new(|_| ResizableState::default()),
            ops,
            ops_panel,
            ops_panel_open: false,
            main_split: cx.new(|_| ResizableState::default()),
            pending_notifications: Vec::new(),
            wheel_zoom: 0.0,
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
        this.sync_connections(cx);
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

    fn base_font_sizes(cx: &App) -> (Pixels, Pixels) {
        theme::base_font_sizes(cx)
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
        self.sync_settings(cx);
    }

    /// A dialog listing the built-in and user themes; clicking one applies it.
    fn choose_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let themes = theme::available(cx);
        let current = self.theme.clone();
        let dir = theme::themes_dir()
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        // The registry already lists the built-in pair as "Default Light/Dark".
        let mut choices: Vec<ThemeChoice> = vec![ThemeChoice::System];
        let mut items = vec![
            PaletteItem::new("System")
                .detail("built-in light or dark, following the desktop")
                .icon(IconName::Palette)
                .checked(current == ThemeChoice::System),
        ];
        for (name, mode) in themes {
            let choice = ThemeChoice::Named(name.clone());
            items.push(
                PaletteItem::new(name)
                    .detail(if mode.is_dark() { "dark" } else { "light" })
                    .icon(if mode.is_dark() {
                        IconName::Moon
                    } else {
                        IconName::Sun
                    })
                    .checked(choice == current),
            );
            choices.push(choice);
        }
        let view = cx.entity();
        let palette = cx.new(|cx| {
            Palette::pick_filtered(items, "Filter themes", window, cx)
                .footer(format!("User themes: {dir}"))
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    if let Some(theme) = choice.item.and_then(|ix| choices.get(ix).cloned()) {
                        view.update(cx, |this, cx| this.set_theme(theme, cx));
                    }
                })
                .on_cancel(|window, cx| window.close_dialog(cx))
        });
        palette::open("Theme".to_string(), palette, window, cx);
    }

    /// A palette of the installed font families, "Theme default" first.
    fn choose_font(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut fonts = cx.text_system().all_font_names();
        fonts.sort_by_key(|f| f.to_lowercase());
        fonts.dedup();
        let current = self.font_family.clone();
        let mut items = vec![
            PaletteItem::new("Theme default")
                .detail("the font the theme (or the toolkit) picks")
                .icon(IconName::Palette)
                .checked(current.is_none()),
        ];
        items.extend(fonts.iter().map(|name| {
            PaletteItem::new(name.clone()).checked(current.as_deref() == Some(name.as_str()))
        }));
        let view = cx.entity();
        let palette = cx.new(|cx| {
            Palette::pick_filtered(items, "Filter fonts", window, cx)
                .footer("Installed font families; the choice applies to the UI and the editor")
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    let Some(ix) = choice.item else {
                        return;
                    };
                    let family = (ix > 0).then(|| fonts[ix - 1].clone());
                    let view = view.clone();
                    window.defer(cx, move |window, cx| {
                        view.update(cx, |this, cx| this.set_font(family, window, cx));
                    });
                })
                .on_cancel(|window, cx| window.close_dialog(cx))
        });
        palette::open("Font".to_string(), palette, window, cx);
    }

    /// Apply a font family over the theme; `None` goes back to the theme's.
    fn set_font(&mut self, family: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if self.font_family == family {
            return;
        }
        self.font_family = family;
        if self.font_family.is_none() {
            // The theme brings its own fonts back.
            theme::apply(&self.theme, Some(window), cx);
        }
        self.apply_scale(cx);
        self.sync_settings(cx);
        self.schedule_save(cx);
        cx.notify();
    }

    /// Ctrl+wheel: one zoom step per wheel notch (a `Lines` delta, or a
    /// pixel jump of a notch's size), or per 40 px of touchpad travel
    /// (small `Pixels` deltas, accumulated).
    fn wheel_zoom(&mut self, delta: &ScrollDelta, window: &mut Window, cx: &mut Context<Self>) {
        const NOTCH: f32 = 40.0;
        const WHEEL_JUMP: f32 = 60.0;
        let dy = match delta {
            ScrollDelta::Lines(lines) => lines.y.signum() * NOTCH,
            ScrollDelta::Pixels(pixels) if pixels.y.as_f32().abs() >= WHEEL_JUMP => {
                self.wheel_zoom = 0.0;
                pixels.y.as_f32().signum() * NOTCH
            }
            ScrollDelta::Pixels(pixels) => pixels.y.as_f32(),
        };
        self.wheel_zoom += dy;
        while self.wheel_zoom >= NOTCH {
            self.wheel_zoom -= NOTCH;
            self.step_scale(SCALE_STEP, window, cx);
        }
        while self.wheel_zoom <= -NOTCH {
            self.wheel_zoom += NOTCH;
            self.step_scale(-SCALE_STEP, window, cx);
        }
    }

    /// An invisible element whose paint registers a capture-phase wheel
    /// listener: Ctrl+wheel zooms and never reaches the lists below.
    fn wheel_zoom_catcher(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace = cx.entity();
        canvas(
            |_, _, _| {},
            move |_, _, window, _cx| {
                let workspace = workspace.clone();
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture || !event.modifiers.secondary() {
                        return;
                    }
                    let delta = event.delta;
                    log::debug!("ctrl+wheel {delta:?}");
                    workspace.update(cx, |this, cx| this.wheel_zoom(&delta, window, cx));
                    cx.stop_propagation();
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_0()
    }

    // ----- UI scale ------------------------------------------------------

    /// Scale the whole UI: gpui-kit's `Root` sets the window rem size from the
    /// theme font size every frame, and all our sizes are in rems.
    fn set_scale(&mut self, scale: f32, cx: &mut Context<Self>) {
        let scale = clamp_scale(scale);
        self.scale = scale;
        self.apply_scale(cx);
        self.schedule_save(cx);
        self.sync_settings(cx);
        cx.notify();
    }

    fn apply_scale(&self, cx: &mut App) {
        theme::apply_scale(self.scale, self.font_family.as_deref(), cx);
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
        self.resubscribe(window, cx);
    }

    /// Tabs come and go, so the subscriptions are rebuilt from the list.
    fn resubscribe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut subscriptions = Vec::new();
        for tab in &self.tabs {
            if let Some(view) = tab.grid_view() {
                subscriptions.push(cx.subscribe_in(
                    view,
                    window,
                    |this, _view, event, window, cx| match event {
                        GridEvent::Changed => {
                            this.schedule_save(cx);
                            cx.notify();
                        }
                        GridEvent::ClipboardChanged => cx.notify(),
                        GridEvent::ToggleFavorite(path) => {
                            this.toggle_favorite(path.clone(), window, cx)
                        }
                        GridEvent::BatchRename(paths) => {
                            this.open_rename(paths.clone(), window, cx)
                        }
                        GridEvent::OpenFile { location, readonly } => {
                            // Archives are read-only, whatever key asked.
                            let readonly = *readonly || location.is_archive();
                            this.open_file(location.clone(), readonly, window, cx)
                        }
                    },
                ));
            }
            if let Some(view) = tab.rename_view() {
                subscriptions.push(cx.subscribe_in(
                    view,
                    window,
                    |this, view, event, window, cx| {
                        this.finish_rename(view, event, window, cx);
                    },
                ));
            }
            if let Some(view) = tab.settings_view() {
                subscriptions.push(cx.subscribe_in(
                    view,
                    window,
                    |this, view, event, window, cx| {
                        this.on_settings_event(view, event, window, cx);
                    },
                ));
            }
            if let Some(view) = tab.connections_view() {
                subscriptions.push(cx.subscribe_in(
                    view,
                    window,
                    |this, view, event, window, cx| {
                        this.on_connections_event(view, event, window, cx);
                    },
                ));
            }
            if let Some(view) = tab.editor_view() {
                subscriptions.push(cx.subscribe_in(
                    view,
                    window,
                    |this, view, event, window, cx| {
                        match event {
                            EditorEvent::Close => {
                                if let Some(ix) = this
                                    .tabs
                                    .iter()
                                    .position(|tab| tab.editor_view() == Some(view))
                                {
                                    this.close_tab(ix, window, cx);
                                }
                            }
                            EditorEvent::Saved(location) => {
                                if let Some(dir) = location.parent() {
                                    for grid in this.tabs.iter().filter_map(|t| t.grid_view()) {
                                        grid.update(cx, |grid, cx| grid.refresh_dir(&dir, cx));
                                    }
                                }
                                cx.notify();
                            }
                        }
                    },
                ));
            }
        }
        self.tab_subscriptions = subscriptions;
    }

    // ----- settings tab --------------------------------------------------

    fn settings_values(&self, cx: &App) -> SettingsValues {
        SettingsValues {
            theme: self.theme.clone(),
            font_family: self.font_family.clone(),
            scale: self.scale,
            sidebar_open: self.sidebar_open,
            system_places: self.system_places,
            command_line: self.command_line_open,
            drop_action: crate::settings::get(cx).drop_action,
            file_icons: crate::settings::get(cx).file_icons,
        }
    }

    /// Show the current values in the settings tab, if one is open.
    fn sync_settings(&mut self, cx: &mut Context<Self>) {
        let values = self.settings_values(cx);
        for view in self.tabs.iter().filter_map(|t| t.settings_view()) {
            view.update(cx, |view, cx| view.sync(values.clone(), cx));
        }
    }

    // ----- connections -----------------------------------------------------

    /// Configs with their current status, for the sidebar and the manager tab.
    fn connection_rows(&self, cx: &App) -> Vec<(crate::connections::ConnectionConfig, Status)> {
        let manager = self.connections.read(cx);
        manager
            .configs()
            .iter()
            .map(|c| (c.clone(), manager.status(&c.conn_id())))
            .collect()
    }

    /// Push the connection list and statuses to the sidebar and an open
    /// manager tab.
    fn sync_connections(&mut self, cx: &mut Context<Self>) {
        let rows = self.connection_rows(cx);
        self.sidebar
            .update(cx, |s, cx| s.set_connections(&rows, cx));
        let status: std::collections::HashMap<ConnId, Status> =
            rows.iter().map(|(c, s)| (c.conn_id(), s.clone())).collect();
        for view in self.tabs.iter().filter_map(|t| t.connections_view()) {
            view.update(cx, |view, cx| view.set_status(status.clone(), cx));
        }
        cx.notify();
    }

    /// Open the connection manager tab (or switch to it).
    fn open_connections(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self
            .tabs
            .iter()
            .position(|t| t.connections_view().is_some())
        {
            self.activate_tab(ix, window, cx);
            return;
        }
        let rows = self.connection_rows(cx);
        let items = rows.iter().map(|(c, _)| c.clone()).collect();
        let status = rows.iter().map(|(c, s)| (c.conn_id(), s.clone())).collect();
        let view = cx.new(|cx| ConnectionsView::new(items, status, window, cx));
        let ix = self.active_tab + 1;
        self.tabs.insert(ix, Tab::connections(view));
        self.resubscribe(window, cx);
        self.activate_tab(ix, window, cx);
    }

    fn on_connections_event(
        &mut self,
        view: &Entity<ConnectionsView>,
        event: &ConnectionsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ConnectionsEvent::Save(items) => {
                let items = items.clone();
                self.connections
                    .update(cx, |m, cx| m.set_configs(items, cx));
                self.schedule_save(cx);
                self.sync_connections(cx);
            }
            ConnectionsEvent::Connect(conn) => {
                let home = self.connections.read(cx).config(conn).map(|c| c.home());
                if let Some(home) = home {
                    // Listing the directory connects on demand.
                    self.open_place(home, cx);
                }
            }
            ConnectionsEvent::Disconnect(conn) => {
                self.connections.update(cx, |m, cx| m.disconnect(conn, cx));
            }
            ConnectionsEvent::Close => {
                if let Some(ix) = self
                    .tabs
                    .iter()
                    .position(|t| t.connections_view() == Some(view))
                {
                    self.close_tab(ix, window, cx);
                }
            }
        }
    }

    /// A connection needs a password: ask with a masked palette, then retry.
    /// The password is remembered in the keyring when the connection is set
    /// to password authentication.
    fn ask_password(&mut self, conn: ConnId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.connections.read(cx).config(&conn).cloned() else {
            return;
        };
        let manager = self.connections.clone();
        let remember =
            config.auth == crate::connections::AuthMethod::Password || !config.protocol.uses_keys();
        let palette = cx.new(|cx| {
            let (manager_ok, manager_cancel, conn_ok, conn_cancel) =
                (manager.clone(), manager, conn.clone(), conn);
            Palette::new(Vec::new(), "", "Password", window, cx)
                .masked(true)
                .input_icon(Some(IconName::Globe))
                .hint(if remember {
                    "Stored in the system keyring after a successful login"
                } else {
                    "Asked again next time (set the connection to Password to store it)"
                })
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    if choice.value.is_empty() {
                        return;
                    }
                    manager_ok.update(cx, |m, cx| {
                        m.connect(&conn_ok, Some(choice.value.clone()), remember, cx)
                    });
                })
                .on_cancel(move |window, cx| {
                    window.close_dialog(cx);
                    manager_cancel.update(cx, |m, cx| m.disconnect(&conn_cancel, cx));
                })
        });
        palette::open(
            format!("Password for {}@{}", config.user, config.host),
            palette,
            window,
            cx,
        );
    }

    fn on_open_connections(
        &mut self,
        _: &OpenConnections,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_connections(window, cx);
    }

    /// Open the settings tab (or switch to it when it is already open).
    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.tabs.iter().position(|t| t.settings_view().is_some()) {
            self.activate_tab(ix, window, cx);
            return;
        }
        let values = self.settings_values(cx);
        let view = cx.new(|cx| SettingsView::new(values, (MIN_SCALE, MAX_SCALE, SCALE_STEP), cx));
        let ix = self.active_tab + 1;
        self.tabs.insert(ix, Tab::settings(view));
        self.resubscribe(window, cx);
        self.activate_tab(ix, window, cx);
    }

    fn on_settings_event(
        &mut self,
        view: &Entity<SettingsView>,
        event: &SettingsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SettingsEvent::ChooseTheme => self.choose_theme(window, cx),
            SettingsEvent::ChooseFont => self.choose_font(window, cx),
            SettingsEvent::Scale(scale) => {
                let scale = clamp_scale(*scale);
                self.scale_slider
                    .update(cx, |slider, cx| slider.set_value(scale, window, cx));
                self.set_scale(scale, cx);
            }
            SettingsEvent::Sidebar(open) => {
                if self.sidebar_open != *open {
                    self.toggle_sidebar(cx);
                }
            }
            SettingsEvent::SystemPlaces(show) => {
                if self.system_places != *show {
                    self.system_places = *show;
                    self.sidebar
                        .update(cx, |s, cx| s.set_system_places(*show, cx));
                    self.schedule_save(cx);
                    cx.notify();
                }
            }
            SettingsEvent::CommandLine(open) => {
                if self.command_line_open != *open {
                    self.command_line_open = *open;
                    self.schedule_save(cx);
                    cx.notify();
                }
            }
            SettingsEvent::DropAction(action) => {
                crate::settings::set_drop_action(cx, *action);
                self.schedule_save(cx);
            }
            SettingsEvent::FileIcons(icons) => {
                crate::settings::set_file_icons(cx, *icons);
                self.schedule_save(cx);
                for grid in self.tabs.iter().filter_map(|tab| tab.grid_view()) {
                    grid.update(cx, |grid, cx| grid.redraw_panels(cx));
                }
            }
            SettingsEvent::Close => {
                if let Some(ix) = self
                    .tabs
                    .iter()
                    .position(|tab| tab.settings_view() == Some(view))
                {
                    self.close_tab(ix, window, cx);
                }
            }
        }
    }

    /// Ctrl+D: bookmark the active panel's directory, or drop the bookmark.
    fn on_toggle_favorite(
        &mut self,
        _: &ToggleFavorite,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(grid) = self.active_tab().grid_view() else {
            return;
        };
        let path = grid.read(cx).active_panel().read(cx).path().clone();
        self.toggle_favorite(path, window, cx);
    }

    /// Add `path` to the favorites, or remove it when it is there already.
    fn toggle_favorite(&mut self, path: Location, window: &mut Window, cx: &mut Context<Self>) {
        if path.is_roots() {
            return;
        }
        let note = if let Some(ix) = self.favorites.iter().position(|f| f.path == path) {
            let removed = self.favorites.remove(ix);
            format!("Removed {} from favorites", removed.display_name())
        } else {
            self.favorites.push(Favorite::new(path.clone()));
            format!("Added {} to favorites", path.display_name())
        };
        self.sync_favorites(cx);
        self.notify(Notification::info(note), window, cx);
        self.schedule_save(cx);
    }

    /// Show the bookmarked directories in the sidebar and the panels' stars.
    fn sync_favorites(&mut self, cx: &mut Context<Self>) {
        let paths = self.favorites.iter().map(|f| f.path.clone()).collect();
        crate::settings::set_favorites(cx, paths);
        self.sidebar
            .update(cx, |s, cx| s.set_favorites(&self.favorites, cx));
        // The stars next to the paths follow the list.
        for grid in self.tabs.iter().filter_map(|t| t.grid_view()) {
            grid.update(cx, |grid, cx| grid.redraw_panels(cx));
        }
    }

    /// The sidebar's context menu: a name for a favorite (a directory gets a
    /// fixed name instead of its own, or that back; a layout is renamed).
    fn rename_favorite(&mut self, id: FavoriteId, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        match id {
            FavoriteId::Path(path) => {
                let Some(fav) = self.favorites.iter().find(|f| f.path == path) else {
                    return;
                };
                let (current, fixed) = (fav.display_name(), fav.name.is_some());
                self.ask_name(
                    NameAsk {
                        title: "Rename favorite",
                        placeholder: "Name",
                        hint: "A fixed name; empty to show the directory's own name again",
                        reset_label: "Back to the directory name",
                        current,
                        fixed,
                    },
                    Rc::new(move |name, _, cx| {
                        let id = FavoriteId::Path(path.clone());
                        view.update(cx, |this, cx| this.set_favorite_name(id, name, cx));
                    }),
                    window,
                    cx,
                );
            }
            FavoriteId::Layout(ix) => {
                let Some(saved) = self.favorite_layouts.get(ix) else {
                    return;
                };
                self.ask_name(
                    NameAsk {
                        title: "Rename layout",
                        placeholder: "Name",
                        hint: "Tabs opened from the layout take this name as their title",
                        reset_label: "Cancel",
                        current: saved.name.clone(),
                        fixed: false,
                    },
                    Rc::new(move |name, _, cx| {
                        if name.is_some() {
                            let id = FavoriteId::Layout(ix);
                            view.update(cx, |this, cx| this.set_favorite_name(id, name, cx));
                        }
                    }),
                    window,
                    cx,
                );
            }
        }
    }

    fn set_favorite_name(&mut self, id: FavoriteId, name: Option<String>, cx: &mut Context<Self>) {
        match id {
            FavoriteId::Path(path) => {
                let Some(fav) = self.favorites.iter_mut().find(|f| f.path == path) else {
                    return;
                };
                fav.name = name;
                self.sync_favorites(cx);
            }
            FavoriteId::Layout(ix) => {
                let (Some(saved), Some(name)) = (self.favorite_layouts.get_mut(ix), name) else {
                    return;
                };
                saved.name = name;
                self.sync_layouts(cx);
            }
        }
        self.schedule_save(cx);
    }

    /// The sidebar's context menu: a colour tag for a favorite, with the
    /// same choices as a tab.
    fn pick_favorite_color(&mut self, id: FavoriteId, window: &mut Window, cx: &mut Context<Self>) {
        let current = match &id {
            FavoriteId::Path(path) => self
                .favorites
                .iter()
                .find(|f| f.path == *path)
                .map(|f| f.color.clone()),
            FavoriteId::Layout(ix) => self.favorite_layouts.get(*ix).map(|l| l.tab.color.clone()),
        };
        let Some(current) = current else {
            return;
        };
        let view = cx.entity();
        self.pick_color(
            "Favorite color",
            current,
            Rc::new(move |color, _, cx| {
                let id = id.clone();
                view.update(cx, |this, cx| this.set_favorite_color(id, color, cx));
            }),
            window,
            cx,
        );
    }

    fn set_favorite_color(
        &mut self,
        id: FavoriteId,
        color: Option<String>,
        cx: &mut Context<Self>,
    ) {
        match id {
            FavoriteId::Path(path) => {
                let Some(fav) = self.favorites.iter_mut().find(|f| f.path == path) else {
                    return;
                };
                fav.color = color;
                self.sync_favorites(cx);
            }
            FavoriteId::Layout(ix) => {
                let Some(saved) = self.favorite_layouts.get_mut(ix) else {
                    return;
                };
                saved.tab.color = color;
                self.sync_layouts(cx);
            }
        }
        self.schedule_save(cx);
    }

    /// The tab's context menu: save the whole tab (layout + panels) under a
    /// name asked for in a palette.
    fn save_layout_favorite(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else {
            return;
        };
        let Some(config) = tab.config(cx) else {
            return;
        };
        let default = tab.title(cx);
        // The config keeps every slot; only rows x cols of them are shown.
        let shown = usize::from(config.layout.rows) * usize::from(config.layout.cols);
        let description = format!(
            "{}x{} layout, {} panel{}",
            config.layout.rows,
            config.layout.cols,
            shown,
            if shown == 1 { "" } else { "s" }
        );
        let rows = vec![
            PaletteItem::new("Save").value("").icon(IconName::Check),
            PaletteItem::new("Cancel").value("").icon(IconName::Close),
        ];
        const CANCEL_ROW: usize = 1;
        let view = cx.entity();
        let len = default.len();
        let palette = cx.new(|cx| {
            Palette::new(rows, &default, "Name", window, cx)
                .input_icon(Some(IconName::LayoutDashboard))
                .description(description)
                .select_range(0..len)
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    if choice.item == Some(CANCEL_ROW) {
                        return;
                    }
                    let name = choice.value.trim().to_string();
                    if name.is_empty() {
                        return;
                    }
                    let (view, config) = (view.clone(), config.clone());
                    window.defer(cx, move |window, cx| {
                        view.update(cx, |this, cx| {
                            this.favorite_layouts.push(FavoriteLayout {
                                name: name.clone(),
                                tab: config,
                            });
                            this.sync_layouts(cx);
                            this.notify(
                                Notification::info(format!("Saved layout {name}")),
                                window,
                                cx,
                            );
                            this.schedule_save(cx);
                        });
                    });
                })
                .on_cancel(|window, cx| window.close_dialog(cx))
        });
        palette::open("Add layout to favorites".into(), palette, window, cx);
    }

    /// The tab's context menu: a fixed title for the tab, or none again.
    fn rename_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(grid) = self.tabs.get(ix).and_then(|t| t.grid_view()).cloned() else {
            return;
        };
        let current = grid.read(cx).title(cx);
        let fixed = grid.read(cx).custom_title().is_some();
        self.ask_name(
            NameAsk {
                title: "Rename tab",
                placeholder: "Tab title",
                hint: "A fixed title; empty to follow the active panel's directory again",
                reset_label: "Back to the directory name",
                current,
                fixed,
            },
            Rc::new(move |title, _, cx| {
                grid.update(cx, |grid, cx| grid.set_title(title, cx));
            }),
            window,
            cx,
        );
    }

    /// A palette asking for a name. The first row applies the typed text
    /// (`None` when empty); the second resets to `None` when a name is fixed
    /// now, else cancels.
    fn ask_name(
        &mut self,
        ask: NameAsk,
        apply: Apply<Option<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rows = vec![
            PaletteItem::new("Rename").value("").icon(IconName::Check),
            PaletteItem::new(if ask.fixed { ask.reset_label } else { "Cancel" })
                .value("")
                .icon(IconName::Close),
        ];
        const SECOND_ROW: usize = 1;
        let len = ask.current.len();
        let fixed = ask.fixed;
        let palette = cx.new(|cx| {
            Palette::new(rows, &ask.current, ask.placeholder, window, cx)
                .input_icon(None)
                .hint(ask.hint)
                .select_range(0..len)
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    let name = if choice.item == Some(SECOND_ROW) {
                        if fixed {
                            None
                        } else {
                            return;
                        }
                    } else {
                        Some(choice.value.trim().to_string()).filter(|t| !t.is_empty())
                    };
                    let apply = apply.clone();
                    window.defer(cx, move |window, cx| apply(name, window, cx));
                })
                .on_cancel(|window, cx| window.close_dialog(cx))
        });
        palette::open(ask.title.into(), palette, window, cx);
    }

    /// The tab's context menu: a colour tag from the presets, a typed hex,
    /// or none.
    fn pick_tab_color(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(grid) = self.tabs.get(ix).and_then(|t| t.grid_view()).cloned() else {
            return;
        };
        let current = grid.read(cx).color().map(str::to_string);
        self.pick_color(
            "Tab color",
            current,
            Rc::new(move |color, _, cx| {
                grid.update(cx, |grid, cx| grid.set_color(color, cx));
            }),
            window,
            cx,
        );
    }

    /// A palette with the preset colours, "Custom…" (a hex asked for next)
    /// and "No color"; `current` is shown checked.
    fn pick_color(
        &mut self,
        title: &'static str,
        current: Option<String>,
        apply: Apply<Option<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut items: Vec<PaletteItem> = crate::tab_color::PRESETS
            .iter()
            .map(|(name, hex)| {
                PaletteItem::new(*name)
                    .color(crate::tab_color::parse(hex))
                    .checked(
                        current
                            .as_deref()
                            .is_some_and(|c| c.eq_ignore_ascii_case(hex)),
                    )
            })
            .collect();
        let custom_ix = items.len();
        let custom_now = current
            .as_deref()
            .filter(|c| crate::tab_color::preset_name(c).is_none())
            .map(str::to_string);
        items.push(
            PaletteItem::new("Custom…")
                .detail(custom_now.clone().unwrap_or_else(|| "#rrggbb".into()))
                .color(custom_now.as_deref().and_then(crate::tab_color::parse))
                .checked(custom_now.is_some()),
        );
        let none_ix = items.len();
        items.push(PaletteItem::new("No color").checked(current.is_none()));

        let view = cx.entity();
        let palette = cx.new(|cx| {
            Palette::pick(items, cx)
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    let Some(item) = choice.item else {
                        return;
                    };
                    let (view, apply, current) = (view.clone(), apply.clone(), current.clone());
                    window.defer(cx, move |window, cx| {
                        if item == custom_ix {
                            view.update(cx, |this, cx| {
                                this.ask_color(title, current, apply, window, cx)
                            });
                            return;
                        }
                        let color = if item == none_ix {
                            None
                        } else {
                            crate::tab_color::PRESETS
                                .get(item)
                                .map(|(_, hex)| hex.to_string())
                        };
                        apply(color, window, cx);
                    });
                })
                .on_cancel(|window, cx| window.close_dialog(cx))
        });
        palette::open(title.into(), palette, window, cx);
    }

    /// The hex input behind "Custom…".
    fn ask_color(
        &mut self,
        title: &'static str,
        current: Option<String>,
        apply: Apply<Option<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = current.unwrap_or_else(|| "#".into());
        let len = current.len();
        let palette = cx.new(|cx| {
            Palette::new(Vec::new(), &current, "#rrggbb", window, cx)
                .input_icon(None)
                .hint("Three or six hex digits, with or without the #")
                .select_range(1.min(len)..len)
                .on_confirm(move |choice, window, cx| {
                    match crate::tab_color::normalise(&choice.value) {
                        Some(hex) => {
                            window.close_dialog(cx);
                            let apply = apply.clone();
                            window.defer(cx, move |window, cx| apply(Some(hex), window, cx));
                        }
                        None => window.push_notification(
                            Notification::error(format!("{}: not a hex color", choice.value)),
                            cx,
                        ),
                    }
                })
                .on_cancel(|window, cx| window.close_dialog(cx))
        });
        palette::open(
            format!("Custom {}", title.to_lowercase()).into(),
            palette,
            window,
            cx,
        );
    }

    fn sync_layouts(&mut self, cx: &mut Context<Self>) {
        self.sidebar
            .update(cx, |s, cx| s.set_layouts(&self.favorite_layouts, cx));
    }

    /// A saved layout from the sidebar: a new tab with it, activated.
    fn open_layout(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(saved) = self.favorite_layouts.get(ix) else {
            return;
        };
        // The new tab carries the layout's name as its title.
        let mut config = saved.tab.clone();
        config.title = Some(saved.name.clone());
        self.push_grid_tab(&config, window, cx);
        let last = self.tabs.len() - 1;
        self.activate_tab(last, window, cx);
        self.schedule_save(cx);
    }

    fn remove_layout(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.favorite_layouts.len() {
            return;
        }
        let removed = self.favorite_layouts.remove(ix);
        self.sync_layouts(cx);
        self.notify(
            Notification::info(format!("Removed layout {}", removed.name)),
            window,
            cx,
        );
        self.schedule_save(cx);
    }

    /// Ctrl+Shift+P: every panel action by name, with its key.
    fn on_command_palette(
        &mut self,
        _: &CommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let commands = crate::keymap::commands();
        let items: Vec<PaletteItem> = commands
            .iter()
            .map(|c| PaletteItem::new(c.label.clone()).detail(c.keys.clone()))
            .collect();
        let actions: std::rc::Rc<Vec<Box<dyn Action>>> =
            std::rc::Rc::new(commands.into_iter().map(|c| c.action).collect());
        let palette = cx.new(|cx| {
            Palette::pick_filtered(items, "Command…", window, cx)
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    let Some(ix) = choice.item else {
                        return;
                    };
                    let action = actions[ix].boxed_clone();
                    window.defer(cx, move |window, cx| window.dispatch_action(action, cx));
                })
                .on_cancel(|window, cx| window.close_dialog(cx))
        });
        palette::open("Commands".into(), palette, window, cx);
    }

    fn on_open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings(window, cx);
    }

    /// Open `path` in an editor tab (read-only for F3). A file already open
    /// gets its tab activated instead; F4 on it makes it editable.
    fn open_file(
        &mut self,
        location: Location,
        readonly: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self.tabs.iter().position(|t| {
            t.editor_view()
                .is_some_and(|v| *v.read(cx).location() == location)
        }) {
            if !readonly && let Some(view) = self.tabs[ix].editor_view() {
                view.update(cx, |editor, cx| editor.set_readonly(false, cx));
            }
            self.activate_tab(ix, window, cx);
            return;
        }
        // Reading may go over the network: do it on the background executor.
        let task = cx.background_spawn({
            let location = location.clone();
            async move { crate::editor_view::load(&location) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.file_loaded(location, readonly, result, window, cx)
            })
            .ok();
        })
        .detach();
    }

    /// The editor load finished: open the tab, or fall back to the default
    /// application (through a local copy for remote files).
    fn file_loaded(
        &mut self,
        location: Location,
        readonly: bool,
        result: Result<crate::editor_view::Loaded, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(loaded) => {
                let view = cx.new(|cx| EditorView::new(loaded, readonly, window, cx));
                let ix = self.active_tab + 1;
                self.tabs.insert(ix, Tab::editor(view));
                self.resubscribe(window, cx);
                self.activate_tab(ix, window, cx);
            }
            Err(reason) => {
                window.push_notification(
                    Notification::info(format!(
                        "{}: {reason}",
                        location.file_name().unwrap_or_default()
                    )),
                    cx,
                );
                cx.background_spawn(async move {
                    match crate::vfs::local_copy(&location) {
                        Ok(path) => {
                            if let Err(err) = open::that_detached(&path) {
                                log::warn!("open {}: {err}", path.display());
                            }
                        }
                        Err(err) => log::warn!("open {}: {err}", location.display()),
                    }
                })
                .detach();
            }
        }
    }

    /// Open the batch rename form for `paths` in a new tab next to the current one.
    fn open_rename(&mut self, paths: Vec<Location>, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.new(|cx| RenameView::new(paths, window, cx));
        let ix = self.active_tab + 1;
        self.tabs.insert(ix, Tab::rename(view));
        self.resubscribe(window, cx);
        self.activate_tab(ix, window, cx);
    }

    /// The rename form is done: close its tab, reload the panels showing
    /// that directory and say what happened.
    fn finish_rename(
        &mut self,
        view: &Entity<RenameView>,
        event: &RenameEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self
            .tabs
            .iter()
            .position(|tab| tab.rename_view() == Some(view))
        {
            self.tabs.remove(ix);
            self.resubscribe(window, cx);
            let next = if ix < self.active_tab || self.active_tab >= self.tabs.len() {
                self.active_tab.saturating_sub(1)
            } else {
                self.active_tab
            };
            self.activate_tab(next, window, cx);
        }
        if let RenameEvent::Done { dir, renamed } = event {
            for grid in self.tabs.iter().filter_map(|tab| tab.grid_view()) {
                grid.update(cx, |grid, cx| grid.refresh_dir(dir, cx));
            }
            window.push_notification(
                Notification::success(format!("Renamed {renamed} items")),
                cx,
            );
        }
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
        let cfg = self.active_tab().config(cx).unwrap_or_default();
        let ops = self.ops.clone();
        let view = cx.new(|cx| GridView::new(&cfg, ops, window, cx));
        let ix = self.active_tab + 1;
        self.tabs.insert(ix, Tab::grid(view));
        self.resubscribe(window, cx);
        self.activate_tab(ix, window, cx);
    }

    /// Move the tab at `from` so that it ends up at position `to`.
    fn move_tab(&mut self, from: usize, to: usize, window: &mut Window, cx: &mut Context<Self>) {
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
        self.resubscribe(window, cx);
        self.schedule_save(cx);
        cx.notify();
    }

    fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() <= 1 || ix >= self.tabs.len() {
            return;
        }
        // An editor with unsaved changes asks first.
        if let Some(editor) = self.tabs[ix].editor_view().cloned()
            && editor.read(cx).is_dirty()
        {
            self.confirm_close_editor(editor, window, cx);
            return;
        }
        self.remove_tab(ix, window, cx);
    }

    fn remove_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.len() <= 1 || ix >= self.tabs.len() {
            return;
        }
        self.tabs.remove(ix);
        self.resubscribe(window, cx);
        // Closing the active tab goes to the one on its left: an editor or
        // form opened from a grid sits right after it, so this returns there.
        let next = if ix <= self.active_tab {
            self.active_tab.saturating_sub(1)
        } else {
            self.active_tab
        };
        self.activate_tab(next, window, cx);
    }

    /// Save / Discard / Cancel for an editor tab with unsaved changes.
    fn confirm_close_editor(
        &mut self,
        editor: Entity<EditorView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = editor.read(cx).title();
        let view = cx.entity();
        let palette = cx.new(|cx| {
            let items = vec![
                PaletteItem::new("Save and close"),
                PaletteItem::new("Discard changes").danger(true),
                PaletteItem::new("Cancel"),
            ];
            Palette::pick(items, cx)
                .description(format!("{title} has unsaved changes."))
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    let (view, editor) = (view.clone(), editor.clone());
                    let save = match choice.item {
                        Some(0) => true,
                        Some(1) => false,
                        _ => return,
                    };
                    window.defer(cx, move |window, cx| {
                        if save && editor.update(cx, |e, cx| e.save(cx)).is_err() {
                            return;
                        }
                        view.update(cx, |this, cx| {
                            if let Some(ix) = this
                                .tabs
                                .iter()
                                .position(|t| t.editor_view() == Some(&editor))
                            {
                                this.remove_tab(ix, window, cx);
                            }
                        });
                    });
                })
                .on_cancel(|window, cx| window.close_dialog(cx))
        });
        palette::open("Unsaved changes".to_string(), palette, window, cx);
    }

    // ----- sidebar -------------------------------------------------------

    /// Navigate the active panel of the active tab to a sidebar place.
    fn open_place(&mut self, path: Location, cx: &mut Context<Self>) {
        if let Some(view) = self.active_tab().grid_view() {
            let panel = view.read(cx).active_panel().clone();
            panel.update(cx, |panel, cx| panel.navigate_to(path, cx));
        }
    }

    /// The focused panel shows `path`: move it to the front of the recent
    /// list (newest first, bounded) when it is a different directory.
    fn note_visit(&mut self, path: Option<Location>, cx: &mut Context<Self>) {
        let Some(path) = path else {
            return;
        };
        if self.last_visited.as_ref() == Some(&path) {
            return;
        }
        self.last_visited = Some(path.clone());
        self.recent.retain(|p| *p != path);
        self.recent.insert(0, path);
        self.recent.truncate(crate::sidebar::RECENT_LIMIT);
        let recent = self.recent.clone();
        self.sidebar.update(cx, |s, cx| s.set_recent(&recent, cx));
        self.schedule_save(cx);
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.remember_side_widths(cx);
        self.sidebar_open = !self.sidebar_open;
        self.reset_side_split(cx);
        if self.sidebar_open {
            self.sidebar.update(cx, |s, cx| {
                s.refresh();
                cx.notify();
            });
        }
        self.schedule_save(cx);
        self.sync_settings(cx);
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
            sidebar_collapsed: self.sidebar.read(cx).collapsed().to_vec(),
            system_places: self.system_places,
            drop_action: crate::settings::get(cx).drop_action,
            file_icons: crate::settings::get(cx).file_icons,
            window: self.window_cfg,
            // Transient tabs are not saved; the active index counts saved tabs only.
            active_tab: self.tabs[..self.active_tab.min(self.tabs.len())]
                .iter()
                .filter(|t| t.config(cx).is_some())
                .count(),
            tabs: self.tabs.iter().filter_map(|t| t.config(cx)).collect(),
            recent: self.recent.clone(),
            favorites: self.favorites.clone(),
            favorite_layouts: self.favorite_layouts.clone(),
            font_family: self.font_family.clone(),
            connections: self.connections.read(cx).configs().to_vec(),
            command_line: self.command_line_open,
            command_history: self.command_line.read(cx).history().to_vec(),
            agents_open: self.agents_open,
            agents_width: self.current_agents_width(cx),
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

    // ----- agents panel ----------------------------------------------------

    /// The panel's width as last laid out (the last resizable panel), else
    /// the remembered one.
    fn current_agents_width(&self, cx: &App) -> f32 {
        if !self.agents_open {
            return self.agents_width;
        }
        let ix = if self.sidebar_open { 2 } else { 1 };
        self.side_split
            .read(cx)
            .sizes()
            .get(ix)
            .map(|w| w.as_f32())
            .filter(|w| *w >= 200.0)
            .unwrap_or(self.agents_width)
    }

    /// Keep the widths of the side panels as laid out, so they come back the
    /// same after being hidden.
    fn remember_side_widths(&mut self, cx: &App) {
        self.agents_width = self.current_agents_width(cx);
        if self.sidebar_open {
            if let Some(w) = self.side_split.read(cx).sizes().first() {
                if w.as_f32() >= 100.0 {
                    self.sidebar_width = w.as_f32();
                }
            }
        }
    }

    /// Start the side split from a fresh state. The resizable state keeps a
    /// size per child, and a child that is hidden or added leaves the others'
    /// stale (the kit never re-measures a panel it has measured once); a drag
    /// then works from sizes that no longer add up to the window and the
    /// divider runs away from the pointer. The hidden panels are left out of
    /// the group and the state is rebuilt whenever their set changes.
    fn reset_side_split(&mut self, cx: &mut Context<Self>) {
        self.side_split = cx.new(|_| ResizableState::default());
    }

    fn lend_focus(&mut self, lent: bool, cx: &mut Context<Self>) {
        if let Some(grid) = self.active_tab().grid_view().cloned() {
            grid.update(cx, |grid, cx| grid.set_focus_lent(lent, cx));
        }
    }

    /// Show the Agents panel (with the installed CLIs re-detected).
    fn open_agents(&mut self, cx: &mut Context<Self>) {
        self.remember_side_widths(cx);
        self.agents_open = true;
        self.reset_side_split(cx);
        self.agents.update(cx, |panel, cx| panel.refresh(cx));
        self.schedule_save(cx);
        cx.notify();
    }

    fn toggle_agents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.agents_open {
            self.remember_side_widths(cx);
            self.agents_open = false;
            self.reset_side_split(cx);
            self.focus_active_panel(window, cx);
            self.schedule_save(cx);
            cx.notify();
        } else {
            self.open_agents(cx);
            let focus = self.agents.read(cx).focus_handle(cx);
            focus.focus(window, cx);
        }
    }

    fn on_toggle_agents(&mut self, _: &ToggleAgents, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_agents(window, cx);
    }

    /// Ctrl+`: into the Agents panel (opening it), or back to the files.
    fn on_focus_agents(&mut self, _: &FocusAgents, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.agents.read(cx).focus_handle(cx);
        if self.agents_open && focus.contains_focused(window, cx) {
            self.focus_active_panel(window, cx);
            return;
        }
        if !self.agents_open {
            self.open_agents(cx);
        }
        focus.focus(window, cx);
        cx.notify();
    }

    fn on_new_agent_session(
        &mut self,
        _: &NewAgentSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.agents_open {
            self.open_agents(cx);
        }
        self.agents
            .update(cx, |panel, cx| panel.choose_and_start(window, cx));
    }

    // ----- command line --------------------------------------------------

    fn notify(&self, note: Notification, window: &mut Window, cx: &mut App) {
        window.push_notification(note, cx);
    }

    fn focus_active_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(grid) = self.active_tab().grid_view().cloned() {
            grid.update(cx, |grid, cx| grid.focus_active(window, cx));
        }
    }

    fn on_focus_command_line(
        &mut self,
        _: &FocusCommandLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_tab().grid_view().is_none() {
            return;
        }
        if !self.command_line_open {
            self.command_line_open = true;
            self.schedule_save(cx);
            cx.notify();
        }
        self.command_line
            .update(cx, |line, cx| line.focus(window, cx));
    }

    /// The entry under the cursor of the active panel, as its name or path.
    fn insert_into_command_line(
        &mut self,
        full_path: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(grid) = self.active_tab().grid_view() else {
            return;
        };
        let panel = grid.read(cx).active_panel().read(cx);
        let Some(location) = panel.current_path() else {
            return;
        };
        let text = if full_path {
            match location.as_local() {
                Some(local) => local.to_string_lossy().into_owned(),
                None => location.display(),
            }
        } else {
            location.file_name().unwrap_or_else(|| location.display())
        };
        if !self.command_line_open {
            self.command_line_open = true;
            self.schedule_save(cx);
            cx.notify();
        }
        self.command_line
            .update(cx, |line, cx| line.insert(&text, window, cx));
    }

    fn on_insert_name(
        &mut self,
        _: &InsertNameToCommandLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.insert_into_command_line(false, window, cx);
    }

    fn on_insert_path(
        &mut self,
        _: &InsertPathToCommandLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.insert_into_command_line(true, window, cx);
    }

    /// Run `command` in the active panel's directory. `cd` moves the panel;
    /// anything else goes to the shell, and its output (if any) opens in a
    /// read-only tab.
    fn run_command(&mut self, command: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(grid) = self.active_tab().grid_view().cloned() else {
            return;
        };
        let current = grid.read(cx).active_panel().read(cx).path().clone();
        self.focus_active_panel(window, cx);

        if command == "cd" || command.starts_with("cd ") {
            let arg = command[2..].trim().trim_matches('"');
            let target = if arg.is_empty() {
                Location::Local(crate::config::home_dir())
            } else if arg.starts_with('~') || arg.starts_with('/') || arg.contains(":/") {
                crate::panel::expand_path(arg)
            } else {
                current.join(arg)
            };
            let target = match target.as_local() {
                Some(local) => match std::fs::canonicalize(local) {
                    Ok(path) if path.is_dir() => Location::Local(path),
                    _ => {
                        self.notify(
                            Notification::error(format!("cd: {}: not a directory", arg)),
                            window,
                            cx,
                        );
                        return;
                    }
                },
                None => target,
            };
            grid.update(cx, |grid, cx| {
                grid.active_panel()
                    .update(cx, |panel, cx| panel.navigate_to(target, cx));
            });
            return;
        }

        let Some(dir) = current.as_local().map(Path::to_path_buf) else {
            self.notify(
                Notification::error("The command line needs a local directory"),
                window,
                cx,
            );
            return;
        };
        let task = cx.background_spawn({
            let command = command.clone();
            async move { run_shell(&command, &dir) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.command_finished(command, result, window, cx)
            })
            .ok();
        })
        .detach();
    }

    fn command_finished(
        &mut self,
        command: String,
        result: Result<(i32, String), String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Err(err) => self.notify(Notification::error(format!("{command}: {err}")), window, cx),
            Ok((code, output)) if output.trim().is_empty() => {
                let text = format!("{command}: exit {code}");
                let note = if code == 0 {
                    Notification::success(text)
                } else {
                    Notification::error(text)
                };
                self.notify(note, window, cx);
            }
            Ok((code, output)) => {
                // The output goes to a file in the cache so the editor tab
                // can show (and reload) it.
                let word: String = command
                    .split_whitespace()
                    .next()
                    .unwrap_or("command")
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                    .take(16)
                    .collect();
                let name = format!("{word}-{}", chrono::Local::now().format("%H%M%S"));
                let dir = directories::ProjectDirs::from("", "", "vais-commander")
                    .map(|d| d.cache_dir().join("output"));
                let Some(dir) = dir else {
                    return;
                };
                let text = format!("$ {command}\n{output}\n[exit {code}]\n");
                let path = dir.join(format!("{name}.txt"));
                if let Err(err) =
                    std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, text))
                {
                    self.notify(Notification::error(format!("{command}: {err}")), window, cx);
                    return;
                }
                self.open_file(Location::Local(path), true, window, cx);
            }
        }
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
            FKey::View => grid.start_edit(true, window, cx),
            FKey::Edit => grid.start_edit(false, window, cx),
            FKey::Quit => {}
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
            let color = tab
                .grid_view()
                .and_then(|g| g.read(cx).color().and_then(crate::tab_color::parse));
            h_flex()
                .id(("tab", ix))
                .h_full()
                .relative()
                // The colour tag: a stripe along the top edge.
                .when_some(color, |d, color| {
                    d.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right_0()
                            .h(px(3.))
                            .bg(color),
                    )
                })
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
                .on_drop(cx.listener(move |this, drag: &DragTab, window, cx| {
                    this.move_tab(drag.index, ix, window, cx);
                }))
                // Right click: save the tab as a layout favorite, close it.
                .context_menu({
                    let workspace = cx.entity();
                    let is_grid = tab.grid_view().is_some();
                    move |menu, _, _| {
                        let menu = if is_grid {
                            let (ws_rename, ws_color, ws_save) =
                                (workspace.clone(), workspace.clone(), workspace.clone());
                            menu.item(PopupMenuItem::new("Rename tab…").on_click(
                                move |_, window, cx| {
                                    ws_rename
                                        .update(cx, |this, cx| this.rename_tab(ix, window, cx));
                                },
                            ))
                            .item(PopupMenuItem::new("Tab color…").on_click(
                                move |_, window, cx| {
                                    ws_color
                                        .update(cx, |this, cx| this.pick_tab_color(ix, window, cx));
                                },
                            ))
                            .item(
                                PopupMenuItem::new("Add layout to favorites…").on_click(
                                    move |_, window, cx| {
                                        ws_save.update(cx, |this, cx| {
                                            this.save_layout_favorite(ix, window, cx)
                                        });
                                    },
                                ),
                            )
                        } else {
                            menu
                        };
                        if closable {
                            let workspace = workspace.clone();
                            menu.separator()
                                .item(PopupMenuItem::new("Close tab").on_click(
                                    move |_, window, cx| {
                                        workspace
                                            .update(cx, |this, cx| this.close_tab(ix, window, cx));
                                    },
                                ))
                        } else {
                            menu
                        }
                    }
                })
                // Fonts reserve more room above capitals (accents) than below
                // the baseline, so a centred line box leaves the title looking
                // high. Nudge it down a little; the offset scales with the font.
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .relative()
                        .top(rems(0.05))
                        .child(title),
                )
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
        let tab_focus = self.active_tab().focus_handle(cx);
        let (current_layout, panel_focus) = match self.active_tab().grid_view() {
            Some(view) => (Some(view.read(cx).layout()), Some(tab_focus.clone())),
            None => (None, None),
        };
        // The application menu: actions go to the active tab, which bubbles
        // them up to the workspace.
        let sidebar_open = self.sidebar_open;
        let ops_open = self.ops_panel_open;
        let agents_open = self.agents_open;
        let app_menu = Button::new("app-menu")
            .ghost()
            .small()
            .icon(IconName::Menu)
            .dropdown_menu(move |menu, _window, _cx| {
                menu.action_context(tab_focus.clone())
                    .menu("Commands…", Box::new(CommandPalette))
                    .menu("Settings…", Box::new(OpenSettings))
                    .menu("Connections…", Box::new(OpenConnections))
                    .menu("Theme…", Box::new(ChooseTheme))
                    .separator()
                    .menu("Add / remove favorite", Box::new(ToggleFavorite))
                    .menu_with_check("Places sidebar", sidebar_open, Box::new(ToggleSidebar))
                    .menu_with_check(
                        "Operations panel",
                        ops_open,
                        Box::new(ToggleOperationsPanel),
                    )
                    .menu_with_check("Agents panel", agents_open, Box::new(ToggleAgents))
                    .separator()
                    .menu("New tab", Box::new(NewTab))
                    .menu("Close tab", Box::new(CloseTab))
                    .separator()
                    .menu("Quit", Box::new(Quit))
            });
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
            // "+" sits right after the last tab, like a browser.
            .child(
                h_flex()
                    .h_full()
                    .flex_none()
                    .px_1()
                    .border_b_1()
                    .border_color(border)
                    .child(new_tab),
            )
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
                    .on_drop(cx.listener(|this, drag: &DragTab, window, cx| {
                        let last = this.tabs.len().saturating_sub(1);
                        this.move_tab(drag.index, last, window, cx);
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
                    .child(app_menu),
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
        let status_left = match crate::clipboard::get(cx).summary() {
            Some(clip) => format!("{status_left}  ·  Clipboard: {clip}"),
            None => status_left,
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
        let current = self
            .active_tab()
            .grid_view()
            .map(|v| v.read(cx).active_panel().read(cx).path().clone());
        self.note_visit(current.clone(), cx);
        if self.sidebar_open {
            self.sidebar
                .update(cx, |s, cx| s.set_current(current.clone(), cx));
        }
        let shift = window.modifiers().shift;
        let key_bar = self.render_key_bar(shift, cx);
        if self.agents_open {
            let cwd = current
                .as_ref()
                .and_then(|loc| loc.as_local())
                .map(Path::to_path_buf);
            self.agents
                .update(cx, |panel, _| panel.set_default_cwd(cwd));
        }
        let show_command_line = self.command_line_open && self.active_tab().grid_view().is_some();
        if show_command_line {
            let prompt = current
                .as_ref()
                .map(|loc| match loc.as_local() {
                    Some(local) => abbreviate_home(local),
                    None => loc.display(),
                })
                .unwrap_or_default();
            self.command_line
                .update(cx, |line, cx| line.set_prompt(prompt, cx));
        }

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
            .when(show_command_line, |d| d.child(self.command_line.clone()))
            .child(key_bar);

        // Hidden side panels are left out (see `reset_side_split`); the
        // side panels keep their width while the window and the main column
        // share the rest.
        let mut side_panels: Vec<_> = Vec::with_capacity(3);
        if self.sidebar_open {
            side_panels.push(
                resizable_panel()
                    .size(px(self.sidebar_width))
                    .fixed()
                    .child(self.sidebar.clone()),
            );
        }
        side_panels.push(resizable_panel().child(main_column));
        if self.agents_open {
            side_panels.push(
                resizable_panel()
                    .size(px(self.agents_width))
                    .fixed()
                    .child(self.agents.clone()),
            );
        }
        let sidebar_toggle = Button::new("sidebar-toggle")
            .ghost()
            .xsmall()
            .icon(if self.sidebar_open {
                IconName::PanelLeftClose
            } else {
                IconName::PanelLeftOpen
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx)));
        let agents_toggle = Button::new("agents-toggle")
            .ghost()
            .xsmall()
            .icon(Icon::new(if self.agents_open {
                Lucide::PanelRightClose
            } else {
                Lucide::PanelRightOpen
            }))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_agents(window, cx)));

        v_flex()
            .key_context(WORKSPACE_CONTEXT)
            .child(self.wheel_zoom_catcher(cx))
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
            .on_action(cx.listener(Self::on_focus_command_line))
            .on_action(cx.listener(Self::on_command_palette))
            .on_action(cx.listener(Self::on_toggle_agents))
            .on_action(cx.listener(Self::on_focus_agents))
            .on_action(cx.listener(Self::on_new_agent_session))
            .on_action(cx.listener(Self::on_toggle_favorite))
            .on_action(cx.listener(Self::on_insert_name))
            .on_action(cx.listener(Self::on_insert_path))
            .on_action(cx.listener(Self::on_open_settings))
            .on_action(cx.listener(Self::on_open_connections))
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
                        .children(side_panels),
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
                    .child(scale_control)
                    .child(agents_toggle),
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

/// `~` for the home directory, as the sidebar shows it.
fn abbreviate_home(path: &Path) -> String {
    let home = crate::config::home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Run `command` through the user's shell in `dir`; the exit code and the
/// combined output.
fn run_shell(command: &str, dir: &Path) -> Result<(i32, String), String> {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", command]);
        c
    } else {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
        let mut c = std::process::Command::new(shell);
        c.args(["-c", command]);
        c
    };
    let output = cmd
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    let err = String::from_utf8_lossy(&output.stderr);
    if !err.trim().is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&err);
    }
    Ok((output.status.code().unwrap_or(-1), text))
}
