//! The picker window: the Places sidebar, one file panel in pick mode and a
//! bottom bar with the file name (Save), the caller's filters and the
//! Cancel / accept buttons. Enter on a file (or a double click) chooses it,
//! Enter on a directory enters it, Ctrl+Enter or the button accepts the
//! marks, the cursor or the current directory, Escape cancels.

use std::path::{Path, PathBuf};

use gpui_kit::base::IndexPath;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::uri::unique_name;
use super::{PickKind, PickResult, PickSpec, Reply};
use crate::config::{Config, Favorite, PanelConfig, ThemeChoice};
use crate::keymap::{PICKER_CONTEXT, PickerAccept, PickerCancel};
use crate::palette::{self, Palette, PaletteItem};
use crate::panel::{FilePanel, PanelEvent, PickMode};
use crate::sidebar::{Sidebar, SidebarEvent};
use crate::ui::{h_flex, v_flex};
use crate::vfs::Location;

const SIDEBAR_WIDTH: f32 = 220.0;
const WINDOW_SIZE: (f32, f32) = (1000.0, 680.0);

/// What the picker takes from the user's config (never written back).
pub struct PickerConfig {
    theme: ThemeChoice,
    font_family: Option<String>,
    scale: f32,
    sidebar_collapsed: Vec<String>,
    system_places: bool,
    /// Only the local ones: a picker returns local paths.
    favorites: Vec<Favorite>,
    recent: Vec<Location>,
}

impl PickerConfig {
    /// The user's theme, font and UI scale on a picker window.
    pub fn apply_theme(&self, window: &mut Window, cx: &mut App) {
        let found = crate::theme::apply(&self.theme, Some(window), cx);
        crate::theme::apply_scale(self.scale, self.font_family.as_deref(), cx);
        log::debug!(
            "portal: theme {:?} applied={found}, now {:?} / {:?} {}px",
            self.theme,
            cx.theme().theme_name(),
            cx.theme().font_family,
            cx.theme().font_size.as_f32()
        );
    }
}

impl From<&Config> for PickerConfig {
    fn from(config: &Config) -> Self {
        Self {
            theme: config.theme.clone(),
            font_family: config.font_family.clone(),
            scale: config.ui_scale,
            sidebar_collapsed: config.sidebar_collapsed.clone(),
            system_places: config.system_places,
            favorites: config
                .favorites
                .iter()
                .filter(|f| f.path.is_local())
                .cloned()
                .collect(),
            recent: config
                .recent
                .iter()
                .filter(|l| l.is_local())
                .cloned()
                .collect(),
        }
    }
}

/// Open the picker window for a request; the reply is sent when the user is
/// done (or the window goes away).
pub fn open(
    spec: PickSpec,
    reply: Reply,
    cfg: &PickerConfig,
    cx: &mut App,
) -> anyhow::Result<AnyWindowHandle> {
    let title = if spec.title.is_empty() {
        match spec.kind {
            PickKind::Open {
                directory: true, ..
            } => "Select folder",
            PickKind::Open { .. } => "Open file",
            PickKind::Save { .. } => "Save file",
            PickKind::SaveFiles { .. } => "Save files",
        }
        .to_string()
    } else {
        spec.title.clone()
    };
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            ..Default::default()
        }),
        app_id: Some("vais-commander".into()),
        window_min_size: Some(size(px(640.), px(400.))),
        ..Default::default()
    };
    let (handle, picker) = gpui_kit::open_window(options, cx, |window, cx| {
        cfg.apply_theme(window, cx);
        cx.new(|cx| Picker::new(spec, reply, cfg, window, cx))
    })?;
    handle.update(cx, |_, window, cx| {
        // The Root resets the appearance while it is set up; the theme goes
        // on again now that the window is complete.
        cfg.apply_theme(window, cx);
        picker.update(cx, |picker, cx| picker.focus_start(window, cx));
        window.activate_window();
    })?;
    Ok(handle)
}

pub struct Picker {
    spec: PickSpec,
    sidebar: Entity<Sidebar>,
    panel: Entity<FilePanel>,
    /// The file name, in Save mode.
    name_input: Option<Entity<InputState>>,
    /// The caller's filters, when there are any.
    filter_select: Option<Entity<SelectState<Vec<SharedString>>>>,
    current_filter: Option<usize>,
    reply: Reply,
    /// The reply went out; the window is on its way out.
    done: bool,
    /// The UI scale and font, put back whenever a theme change resets them.
    scale: f32,
    font_family: Option<String>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Picker {
    fn new(
        spec: PickSpec,
        reply: Reply,
        cfg: &PickerConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let start = start_dir(&spec);
        let mode = match spec.kind {
            PickKind::Open {
                directory: false, ..
            } => PickMode::Files,
            PickKind::Open { .. } | PickKind::SaveFiles { .. } => PickMode::Directory,
            PickKind::Save { .. } => PickMode::Save,
        };
        let current_filter = if spec.filters.is_empty() {
            None
        } else {
            Some(spec.current_filter.unwrap_or(0).min(spec.filters.len() - 1))
        };
        let matchers = current_filter
            .map(|ix| spec.filters[ix].matchers(|mime| crate::icons::globs_for_mime(cx, mime)))
            .unwrap_or_default();
        let select = match &spec.kind {
            PickKind::Save {
                current_file: Some(file),
                ..
            } => Some(Location::Local(file.clone())),
            _ => None,
        };

        let panel = cx.new(|cx| {
            let mut panel = FilePanel::new(
                0,
                PanelConfig {
                    path: Location::Local(start.clone()),
                    ..Default::default()
                },
                window,
                cx,
            );
            panel.set_pick_mode(Some(mode));
            panel.set_highlighted(true, cx);
            if !matchers.is_empty() {
                panel.set_pick_filter(matchers, cx);
            }
            if select.is_some() {
                panel.navigate_to_select(Location::Local(start.clone()), select, cx);
            }
            panel
        });
        let sidebar = cx.new(|cx| {
            let mut sidebar = Sidebar::new(cfg.sidebar_collapsed.clone(), cfg.system_places, cx);
            sidebar.set_show_connections(false);
            sidebar.set_favorites(&cfg.favorites, cx);
            sidebar.set_recent(&cfg.recent, cx);
            sidebar.set_current(Some(Location::Local(start)), cx);
            sidebar
        });

        let mut subscriptions = Vec::new();
        // Theme reloads and mode switches reset the font size and family.
        subscriptions.push(
            cx.observe_global::<gpui_kit::component::theme::Theme>(|this, cx| {
                this.ensure_scale(cx)
            }),
        );
        subscriptions.push(cx.subscribe_in(
            &panel,
            window,
            |this, _panel, event: &PanelEvent, window, cx| match event {
                PanelEvent::Activated(location) => this.activated(location.clone(), window, cx),
                PanelEvent::CancelRequested => this.cancel(window, cx),
                PanelEvent::Changed => {
                    let path = this.panel.read(cx).path().clone();
                    this.sidebar
                        .update(cx, |sidebar, cx| sidebar.set_current(Some(path), cx));
                }
                _ => {}
            },
        ));
        subscriptions.push(
            cx.subscribe(&sidebar, |this, _sidebar, event: &SidebarEvent, cx| {
                if let SidebarEvent::Open(location) = event
                    && location.is_local()
                {
                    let location = location.clone();
                    this.panel
                        .update(cx, |panel, cx| panel.navigate_to(location, cx));
                }
            }),
        );

        let name_input = match &spec.kind {
            PickKind::Save { current_name, .. } => {
                let name = current_name.clone().unwrap_or_default();
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("File name")
                        .default_value(name)
                });
                subscriptions.push(cx.subscribe_in(
                    &input,
                    window,
                    |this, _input, event: &InputEvent, window, cx| {
                        if let InputEvent::PressEnter { .. } = event {
                            this.accept(window, cx);
                        }
                    },
                ));
                Some(input)
            }
            _ => None,
        };

        let filter_select = current_filter.map(|ix| {
            let names: Vec<SharedString> = spec
                .filters
                .iter()
                .map(|f| SharedString::from(f.label.clone()))
                .collect();
            let select = cx.new(|cx| SelectState::new(names, Some(IndexPath::new(ix)), window, cx));
            subscriptions.push(cx.subscribe_in(
                &select,
                window,
                |this, select, event: &SelectEvent<Vec<SharedString>>, _window, cx| {
                    let SelectEvent::Confirm(_) = event;
                    let ix = select.read(cx).selected_index(cx).map(|ix| ix.row);
                    this.set_filter(ix, cx);
                },
            ));
            select
        });

        Self {
            spec,
            sidebar,
            panel,
            name_input,
            filter_select,
            current_filter,
            reply,
            done: false,
            scale: cfg.scale,
            font_family: cfg.font_family.clone(),
            focus_handle,
            _subscriptions: subscriptions,
        }
    }

    fn ensure_scale(&self, cx: &mut Context<Self>) {
        let (base, base_mono) = crate::theme::base_font_sizes(cx);
        let (want, want_mono) = (base * self.scale, base_mono * self.scale);
        let theme = cx.theme();
        let family_ok = self
            .font_family
            .as_deref()
            .is_none_or(|f| theme.font_family.as_ref() == f);
        if theme.font_size != want || theme.mono_font_size != want_mono || !family_ok {
            crate::theme::apply_scale(self.scale, self.font_family.as_deref(), cx);
        }
    }

    /// The file name field in Save mode (the name without its extension
    /// selected), else the panel.
    fn focus_start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = &self.name_input {
            input.update(cx, |input, cx| {
                let name = input.value().to_string();
                let stem = match name.rfind('.') {
                    Some(ix) if ix > 0 => ix,
                    _ => name.len(),
                };
                input.focus(window, cx);
                input.set_selected_range(0..stem, cx);
            });
        } else {
            let focus = self.panel.read(cx).focus_handle(cx);
            focus.focus(window, cx);
        }
    }

    fn set_filter(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        if ix == self.current_filter {
            return;
        }
        self.current_filter = ix;
        let matchers = ix
            .and_then(|ix| self.spec.filters.get(ix))
            .map(|f| f.matchers(|mime| crate::icons::globs_for_mime(cx, mime)))
            .unwrap_or_default();
        self.panel
            .update(cx, |panel, cx| panel.set_pick_filter(matchers, cx));
    }

    /// Enter or a double click on a file in the panel.
    fn activated(&mut self, location: Location, window: &mut Window, cx: &mut Context<Self>) {
        match &self.spec.kind {
            PickKind::Open {
                directory: false, ..
            } => {
                if let Some(path) = location.as_local() {
                    let paths = vec![path.to_path_buf()];
                    self.finish(self.accepted(paths), window, cx);
                }
            }
            PickKind::Save { .. } => {
                // The file's name goes into the field; Save confirms (and
                // asks before replacing it).
                if let (Some(input), Some(name)) = (&self.name_input, location.file_name()) {
                    input.update(cx, |input, cx| {
                        input.set_value(name, window, cx);
                        input.focus(window, cx);
                    });
                }
            }
            // Only directories count here.
            PickKind::Open { .. } | PickKind::SaveFiles { .. } => {}
        }
    }

    fn accepted(&self, paths: Vec<PathBuf>) -> PickResult {
        PickResult::Accepted {
            paths,
            filter: self.current_filter,
        }
    }

    /// The accept button, Ctrl+Enter, Enter in the name field.
    fn accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.spec.kind.clone() {
            PickKind::Open {
                multiple,
                directory: false,
            } => {
                let panel = self.panel.read(cx);
                let chosen: Vec<Location> = if multiple {
                    panel.selected_paths()
                } else {
                    panel.current_path().into_iter().collect()
                };
                let files: Vec<PathBuf> = chosen
                    .iter()
                    .filter_map(|l| l.as_local())
                    .filter(|p| !p.is_dir())
                    .map(Path::to_path_buf)
                    .collect();
                if files.is_empty() {
                    // The cursor on a directory: enter it instead.
                    if let Some(current) = panel.current_path()
                        && current.as_local().is_some_and(|p| p.is_dir())
                    {
                        self.panel
                            .update(cx, |panel, cx| panel.navigate_to(current, cx));
                    }
                    return;
                }
                self.finish(self.accepted(files), window, cx);
            }
            PickKind::Open {
                multiple,
                directory: true,
            } => {
                let dirs = self.chosen_dirs(multiple, cx);
                self.finish(self.accepted(dirs), window, cx);
            }
            PickKind::SaveFiles { files } => {
                let Some(dir) = self.chosen_dirs(false, cx).into_iter().next() else {
                    return;
                };
                let paths = files
                    .iter()
                    .map(|name| dir.join(unique_name(&dir, name)))
                    .collect();
                self.finish(self.accepted(paths), window, cx);
            }
            PickKind::Save { .. } => {
                let Some(input) = &self.name_input else {
                    return;
                };
                let name = input.read(cx).value().trim().to_string();
                if name.is_empty() {
                    window.push_notification(Notification::warning("Type a file name"), cx);
                    return;
                }
                let Some(dir) = self.panel.read(cx).path().as_local().map(Path::to_path_buf) else {
                    return;
                };
                let target = dir.join(&name);
                if target.is_dir() {
                    self.panel.update(cx, |panel, cx| {
                        panel.navigate_to(Location::Local(target), cx)
                    });
                    return;
                }
                if target.exists() {
                    self.confirm_replace(target, window, cx);
                    return;
                }
                self.finish(self.accepted(vec![target]), window, cx);
            }
        }
    }

    /// The directories a directory pick means: the marked ones (when several
    /// may be chosen), else the one under the cursor, else the current one.
    fn chosen_dirs(&self, multiple: bool, cx: &App) -> Vec<PathBuf> {
        let panel = self.panel.read(cx);
        let local_dirs = |locations: Vec<Location>| -> Vec<PathBuf> {
            locations
                .iter()
                .filter_map(|l| l.as_local())
                .filter(|p| p.is_dir())
                .map(Path::to_path_buf)
                .collect()
        };
        if multiple {
            let marked = local_dirs(panel.selected_paths());
            if !marked.is_empty() {
                return marked;
            }
        }
        let under_cursor = local_dirs(panel.current_path().into_iter().collect());
        if !under_cursor.is_empty() {
            return under_cursor;
        }
        panel
            .path()
            .as_local()
            .map(|p| vec![p.to_path_buf()])
            .unwrap_or_default()
    }

    fn confirm_replace(&mut self, target: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let items = vec![
            PaletteItem::new("Replace")
                .icon(IconName::Check)
                .danger(true),
            PaletteItem::new("Cancel").icon(IconName::Close),
        ];
        let view = cx.entity();
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let palette = cx.new(|cx| {
            Palette::pick(items, cx)
                .description(format!("{name} already exists in this folder"))
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    if choice.item != Some(0) {
                        return;
                    }
                    let (view, target) = (view.clone(), target.clone());
                    window.defer(cx, move |window, cx| {
                        view.update(cx, |this, cx| {
                            let result = this.accepted(vec![target]);
                            this.finish(result, window, cx);
                        });
                    });
                })
                .on_cancel(|window, cx| window.close_dialog(cx))
        });
        palette::open("Replace file?".into(), palette, window, cx);
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish(PickResult::Cancelled, window, cx);
    }

    fn finish(&mut self, result: PickResult, window: &mut Window, cx: &mut Context<Self>) {
        if self.done {
            return;
        }
        self.done = true;
        if matches!(result, PickResult::Accepted { .. })
            && let Some(dir) = self.panel.read(cx).path().as_local()
        {
            remember_dir(dir);
        }
        self.reply.send(result);
        window.remove_window();
    }

    fn on_accept(&mut self, _: &PickerAccept, window: &mut Window, cx: &mut Context<Self>) {
        self.accept(window, cx);
    }

    fn on_cancel(&mut self, _: &PickerCancel, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel(window, cx);
    }
}

impl Drop for Picker {
    fn drop(&mut self) {
        // Closed by the compositor or the portal: no choice was made.
        if !self.done {
            self.reply.send(PickResult::Closed);
        }
    }
}

impl Render for Picker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (bg, fg, border, muted) = (
            theme.background,
            theme.foreground,
            theme.border,
            theme.muted_foreground,
        );
        let accept_label: SharedString = self
            .spec
            .accept_label
            .clone()
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| {
                match self.spec.kind {
                    PickKind::Open {
                        directory: true, ..
                    }
                    | PickKind::SaveFiles { .. } => "Select",
                    PickKind::Open { .. } => "Open",
                    PickKind::Save { .. } => "Save",
                }
                .to_string()
            })
            .into();

        v_flex()
            .key_context(PICKER_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_accept))
            .on_action(cx.listener(Self::on_cancel))
            .size_full()
            .bg(bg)
            .text_color(fg)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        div()
                            .w(px(SIDEBAR_WIDTH))
                            .flex_none()
                            .h_full()
                            .border_r_1()
                            .border_color(border)
                            .child(self.sidebar.clone()),
                    )
                    .child(div().flex_1().min_w_0().h_full().child(self.panel.clone())),
            )
            .child(
                h_flex()
                    .flex_none()
                    .h(rems(2.75))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .border_t_1()
                    .border_color(border)
                    .when_some(self.name_input.clone(), |bar, input| {
                        bar.child(div().text_sm().text_color(muted).child("Name"))
                            .child(div().flex_1().min_w_0().child(Input::new(&input).small()))
                    })
                    .when(self.name_input.is_none(), |bar| bar.child(div().flex_1()))
                    .when_some(self.filter_select.clone(), |bar, select| {
                        bar.child(
                            div()
                                .w(px(220.))
                                .flex_none()
                                .child(Select::new(&select).small()),
                        )
                    })
                    .child(
                        Button::new("picker-cancel")
                            .small()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
                    )
                    .child(
                        Button::new("picker-accept")
                            .small()
                            .primary()
                            .label(accept_label)
                            .on_click(cx.listener(|this, _, window, cx| this.accept(window, cx))),
                    ),
            )
    }
}

/// The caller's directory, else the one the user picked in last time, else
/// home.
fn start_dir(spec: &PickSpec) -> PathBuf {
    spec.start_dir
        .clone()
        .filter(|d| d.is_dir())
        .or_else(|| remembered_dir().filter(|d| d.is_dir()))
        .unwrap_or_else(crate::config::home_dir)
}

fn memory_file() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "vais-commander")
        .map(|d| d.cache_dir().join("picker-dir"))
}

fn remembered_dir() -> Option<PathBuf> {
    let text = std::fs::read_to_string(memory_file()?).ok()?;
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| PathBuf::from(line))
}

fn remember_dir(dir: &Path) {
    let Some(file) = memory_file() else {
        return;
    };
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(file, dir.to_string_lossy().as_bytes());
}
