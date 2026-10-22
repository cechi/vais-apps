//! The left sidebar: system places (home, user folders, root, mounted
//! volumes). Clicking a place navigates the active panel there. More kinds of
//! sidebar content can be added later.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::platform;
use crate::ui::{h_flex, v_flex};
use crate::vfs::Location;

pub enum SidebarEvent {
    /// Navigate the active panel to this directory.
    Open(Location),
    /// Open the connection manager.
    ManageConnections,
}

#[derive(Clone)]
pub struct Place {
    pub name: String,
    pub path: Location,
    pub icon: IconName,
    /// Muted text after the name (the parent of a recent directory).
    pub detail: Option<String>,
}

/// How many recently visited directories the sidebar lists.
pub const RECENT_LIMIT: usize = 10;

pub struct Sidebar {
    places: Vec<Place>,
    volumes: Vec<Place>,
    recent: Vec<Place>,
    connections: Vec<Place>,
    /// Directory of the active panel, highlighted when it is a listed place.
    current: Option<Location>,
}

impl EventEmitter<SidebarEvent> for Sidebar {}

impl Sidebar {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            places: Vec::new(),
            volumes: Vec::new(),
            recent: Vec::new(),
            connections: Vec::new(),
            current: None,
        };
        this.refresh();
        this
    }

    /// Re-read user folders and mounts (cheap; called when the sidebar opens).
    pub fn refresh(&mut self) {
        self.places = platform::user_places()
            .into_iter()
            .map(|(name, path, icon)| Place {
                name,
                path: Location::Local(path),
                icon,
                detail: None,
            })
            .collect();
        self.volumes = platform::volumes()
            .into_iter()
            .map(|(name, path)| Place {
                name,
                path: Location::Local(path),
                icon: IconName::HardDrive,
                detail: None,
            })
            .collect();
    }

    /// The recently visited directories, newest first.
    pub fn set_recent(&mut self, paths: &[Location], cx: &mut Context<Self>) {
        let home = crate::config::home_dir();
        self.recent = paths
            .iter()
            .map(|path| {
                let parent = path.parent().map(|p| match p.as_local() {
                    Some(local) => abbreviate(local, &home),
                    None => p.display(),
                });
                Place {
                    name: path.display_name(),
                    path: path.clone(),
                    icon: IconName::Folder,
                    detail: parent.filter(|p| !p.is_empty()),
                }
            })
            .collect();
        cx.notify();
    }

    /// The saved connections with their state (shown as the detail).
    pub fn set_connections(
        &mut self,
        connections: &[(
            crate::connections::ConnectionConfig,
            crate::connections::Status,
        )],
        cx: &mut Context<Self>,
    ) {
        use crate::connections::Status;
        self.connections = connections
            .iter()
            .map(|(config, status)| Place {
                name: if config.name.is_empty() {
                    config.id.clone()
                } else {
                    config.name.clone()
                },
                path: config.home(),
                icon: IconName::Globe,
                detail: Some(match status {
                    Status::Disconnected => format!("{}@{}", config.user, config.host),
                    Status::Connecting => "connecting…".to_string(),
                    Status::Connected => "connected".to_string(),
                    Status::Failed(err) => format!("failed: {err}"),
                }),
            })
            .collect();
        cx.notify();
    }

    pub fn set_current(&mut self, path: Option<Location>, cx: &mut Context<Self>) {
        if self.current != path {
            self.current = path;
            cx.notify();
        }
    }

    fn render_section(
        &self,
        title: &'static str,
        places: &[Place],
        offset: usize,
        c: &Colors,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        v_flex()
            .w_full()
            .child(
                div()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_xs()
                    .text_color(c.muted_foreground)
                    .child(title),
            )
            .children(places.iter().enumerate().map(|(i, place)| {
                let ix = offset + i;
                let selected = self.current.as_ref() == Some(&place.path);
                let path = place.path.clone();
                h_flex()
                    .id(("place", ix))
                    .w_full()
                    .h(rems(1.75))
                    .px_3()
                    .gap_2()
                    .cursor_pointer()
                    .text_sm()
                    .when(selected, |d| d.bg(c.accent).text_color(c.accent_foreground))
                    .hover(|s| s.bg(c.hover))
                    .on_click(cx.listener(move |_this, _, _window, cx| {
                        cx.emit(SidebarEvent::Open(path.clone()));
                    }))
                    .child(
                        Icon::new(place.icon.clone())
                            .size_4()
                            .flex_none()
                            .text_color(if selected {
                                c.accent_foreground
                            } else {
                                c.muted_foreground
                            }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .max_w_full()
                            .truncate()
                            .child(place.name.clone()),
                    )
                    .when_some(place.detail.clone(), |row, detail| {
                        row.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(if selected {
                                    c.accent_foreground
                                } else {
                                    c.muted_foreground
                                })
                                .child(detail),
                        )
                    })
            }))
    }
}

/// `path` with the home directory shortened to `~`.
fn abbreviate(path: &std::path::Path, home: &std::path::Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

struct Colors {
    muted_foreground: Hsla,
    accent: Hsla,
    accent_foreground: Hsla,
    hover: Hsla,
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let c = Colors {
            muted_foreground: theme.sidebar_foreground.alpha(0.7),
            accent: theme.sidebar_accent,
            accent_foreground: theme.sidebar_accent_foreground,
            hover: theme.list_hover,
        };
        let (bg, fg, border) = (
            theme.sidebar,
            theme.sidebar_foreground,
            theme.sidebar_border,
        );
        let places = self.places.clone();
        let volumes = self.volumes.clone();
        let recent = self.recent.clone();
        let connections = self.connections.clone();

        v_flex()
            .size_full()
            .min_w_0()
            .bg(bg)
            .text_color(fg)
            .border_r_1()
            .border_color(border)
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
                            .child("Places"),
                    )
                    .child(
                        Button::new("sidebar-refresh")
                            .ghost()
                            .xsmall()
                            .icon(IconName::RefreshCw)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.refresh();
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("sidebar-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(self.render_section("Favorites", &places, 0, &c, cx))
                    .when(!volumes.is_empty(), |d| {
                        d.child(self.render_section("Volumes", &volumes, places.len(), &c, cx))
                    })
                    .child({
                        let offset = places.len() + volumes.len();
                        self.render_section("Connections", &connections, offset, &c, cx)
                    })
                    .child(
                        h_flex()
                            .id("sidebar-manage-connections")
                            .w_full()
                            .h(rems(1.75))
                            .px_3()
                            .gap_2()
                            .cursor_pointer()
                            .text_sm()
                            .text_color(c.muted_foreground)
                            .hover(|s| s.bg(c.hover))
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::ManageConnections)),
                            )
                            .child(Icon::new(IconName::Plus).size_4().flex_none())
                            .child("Manage connections…"),
                    )
                    .when(!recent.is_empty(), |d| {
                        let offset = places.len() + volumes.len() + connections.len();
                        d.child(self.render_section("Recent", &recent, offset, &c, cx))
                    }),
            )
    }
}
