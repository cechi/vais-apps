//! The left sidebar: system places (home, user folders, root, mounted
//! volumes). Clicking a place navigates the active panel there. More kinds of
//! sidebar content can be added later.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::{Icon, IconName, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::platform;
use crate::ui::{h_flex, v_flex};
use crate::vfs::Location;

pub enum SidebarEvent {
    /// Navigate the active panel to this directory.
    Open(Location),
    /// Open a new tab from the saved layout with this index.
    OpenLayout(usize),
    /// Drop a bookmarked directory.
    RemoveFavorite(Location),
    /// Drop a saved layout.
    RemoveLayout(usize),
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
    /// A saved layout (index into the workspace's list) instead of a directory.
    pub layout: Option<usize>,
    /// A favorite the context menu can remove.
    pub removable: bool,
}

/// How many recently visited directories the sidebar lists.
pub const RECENT_LIMIT: usize = 10;

pub struct Sidebar {
    places: Vec<Place>,
    /// Bookmarked directories (Ctrl+D), listed after the user folders.
    favorites: Vec<Place>,
    /// Saved layouts, after the bookmarked directories.
    layouts: Vec<Place>,
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
            favorites: Vec::new(),
            layouts: Vec::new(),
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
                layout: None,
                removable: false,
            })
            .collect();
        self.volumes = platform::volumes()
            .into_iter()
            .map(|(name, path)| Place {
                name,
                path: Location::Local(path),
                icon: IconName::HardDrive,
                detail: None,
                layout: None,
                removable: false,
            })
            .collect();
    }

    /// The bookmarked directories, in the order they were added.
    pub fn set_favorites(&mut self, paths: &[Location], cx: &mut Context<Self>) {
        let home = crate::config::home_dir();
        self.favorites = paths
            .iter()
            .map(|path| {
                let parent = path.parent().map(|p| match p.as_local() {
                    Some(local) => abbreviate(local, &home),
                    None => p.display(),
                });
                Place {
                    name: path.display_name(),
                    path: path.clone(),
                    icon: IconName::Star,
                    detail: parent.filter(|p| !p.is_empty()),
                    layout: None,
                    removable: true,
                }
            })
            .collect();
        cx.notify();
    }

    /// The saved layouts (whole tabs), in the order they were added.
    pub fn set_layouts(&mut self, layouts: &[crate::config::FavoriteLayout], cx: &mut Context<Self>) {
        self.layouts = layouts
            .iter()
            .enumerate()
            .map(|(ix, saved)| Place {
                name: saved.name.clone(),
                path: Location::Local(std::path::PathBuf::new()),
                icon: IconName::LayoutDashboard,
                detail: Some(format!(
                    "{}x{} layout",
                    saved.tab.layout.rows, saved.tab.layout.cols
                )),
                layout: Some(ix),
                removable: true,
            })
            .collect();
        cx.notify();
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
                    layout: None,
                    removable: false,
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
                layout: None,
                removable: false,
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
                let selected =
                    place.layout.is_none() && self.current.as_ref() == Some(&place.path);
                let path = place.path.clone();
                let layout = place.layout;
                let row = h_flex()
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
                        cx.emit(match layout {
                            Some(ix) => SidebarEvent::OpenLayout(ix),
                            None => SidebarEvent::Open(path.clone()),
                        });
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
                    });
                if !place.removable {
                    return row.into_any_element();
                }
                // Favorites: a context menu to drop them again.
                let sidebar = cx.entity();
                let path = place.path.clone();
                row.context_menu(move |menu, _, _| {
                    let (sidebar, path) = (sidebar.clone(), path.clone());
                    menu.item(
                        PopupMenuItem::new("Remove from favorites").on_click(move |_, _, cx| {
                            sidebar.update(cx, |_, cx| {
                                cx.emit(match layout {
                                    Some(ix) => SidebarEvent::RemoveLayout(ix),
                                    None => SidebarEvent::RemoveFavorite(path.clone()),
                                });
                            });
                        }),
                    )
                })
                .into_any_element()
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
        let places: Vec<Place> = self
            .places
            .iter()
            .chain(self.favorites.iter())
            .chain(self.layouts.iter())
            .cloned()
            .collect();
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
