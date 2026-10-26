//! The left sidebar: system places (home, user folders, root, mounted
//! volumes). Clicking a place navigates the active panel there. More kinds of
//! sidebar content can be added later.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::scroll::Scrollbar;
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
    /// Ask for a new name for a favorite.
    RenameFavorite(FavoriteId),
    /// Ask for a colour tag for a favorite.
    ColorFavorite(FavoriteId),
    /// Put the favorite at `from` at position `to` of its list (the
    /// directories or the saved layouts).
    MoveFavorite {
        layout: bool,
        from: usize,
        to: usize,
    },
    /// Open the connection manager.
    ManageConnections,
    /// A group was folded or unfolded (worth saving).
    Changed,
}

/// A favorite being dragged to another position of its list.
struct DragFavorite {
    layout: bool,
    index: usize,
}

struct FavoriteDragPreview {
    name: SharedString,
}

impl Render for FavoriteDragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_3()
            .py_1()
            .rounded_md()
            .bg(theme.sidebar_accent)
            .text_color(theme.sidebar_accent_foreground)
            .border_1()
            .border_color(theme.border)
            .text_sm()
            .child(self.name.clone())
    }
}

/// One of the user's own favorites: a bookmarked directory or a saved layout
/// (by its index in the workspace's list).
#[derive(Clone, Debug)]
pub enum FavoriteId {
    Path(Location),
    Layout(usize),
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
    /// A colour tag, drawn as a thick left border (saved layouts).
    pub color: Option<Hsla>,
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
    /// Scroll position of the places list, for its scrollbar.
    scroll: ScrollHandle,
    /// Titles of the folded groups.
    collapsed: Vec<String>,
    /// Whether the Favorites group lists the system places (home, Desktop…).
    system_places: bool,
    /// Whether the Connections group is shown (not in the portal picker).
    show_connections: bool,
}

impl EventEmitter<SidebarEvent> for Sidebar {}

impl Sidebar {
    pub fn new(collapsed: Vec<String>, system_places: bool, _cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            places: Vec::new(),
            favorites: Vec::new(),
            layouts: Vec::new(),
            volumes: Vec::new(),
            recent: Vec::new(),
            connections: Vec::new(),
            current: None,
            scroll: ScrollHandle::new(),
            collapsed,
            system_places,
            show_connections: true,
        };
        this.refresh();
        this
    }

    /// Hide the Connections group (the portal picker returns local paths only).
    pub fn set_show_connections(&mut self, show: bool) {
        self.show_connections = show;
    }

    /// Show or hide the system places at the top of the Favorites group.
    pub fn set_system_places(&mut self, show: bool, cx: &mut Context<Self>) {
        if self.system_places != show {
            self.system_places = show;
            self.refresh();
            cx.notify();
        }
    }

    /// Re-read user folders and mounts (cheap; called when the sidebar opens).
    pub fn refresh(&mut self) {
        self.places = if self.system_places {
            platform::user_places()
                .into_iter()
                .map(|(name, path, icon)| Place {
                    name,
                    path: Location::Local(path),
                    icon,
                    detail: None,
                    layout: None,
                    removable: false,
                    color: None,
                })
                .collect()
        } else {
            Vec::new()
        };
        self.volumes = platform::volumes()
            .into_iter()
            .map(|(name, path)| Place {
                name,
                path: Location::Local(path),
                icon: IconName::HardDrive,
                detail: None,
                layout: None,
                removable: false,
                color: None,
            })
            .collect();
    }

    /// The bookmarked directories, in the order they were added.
    pub fn set_favorites(&mut self, favorites: &[crate::config::Favorite], cx: &mut Context<Self>) {
        let home = crate::config::home_dir();
        self.favorites = favorites
            .iter()
            .map(|fav| {
                let path = &fav.path;
                // A renamed favorite shows the whole path as its detail, the
                // others only the parent.
                let detail = if fav.name.is_some() {
                    Some(match path.as_local() {
                        Some(local) => abbreviate(local, &home),
                        None => path.display(),
                    })
                } else {
                    path.parent().map(|p| match p.as_local() {
                        Some(local) => abbreviate(local, &home),
                        None => p.display(),
                    })
                };
                Place {
                    name: fav.display_name(),
                    path: path.clone(),
                    icon: IconName::Star,
                    detail: detail.filter(|p| !p.is_empty()),
                    layout: None,
                    removable: true,
                    color: fav.color.as_deref().and_then(crate::tab_color::parse),
                }
            })
            .collect();
        cx.notify();
    }

    /// The saved layouts (whole tabs), in the order they were added.
    pub fn set_layouts(
        &mut self,
        layouts: &[crate::config::FavoriteLayout],
        cx: &mut Context<Self>,
    ) {
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
                color: saved.tab.color.as_deref().and_then(crate::tab_color::parse),
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
                    color: None,
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
                color: None,
            })
            .collect();
        cx.notify();
    }

    /// The folded groups, for the config.
    pub fn collapsed(&self) -> &[String] {
        &self.collapsed
    }

    fn is_collapsed(&self, title: &str) -> bool {
        self.collapsed.iter().any(|t| t == title)
    }

    fn toggle_group(&mut self, title: &str, cx: &mut Context<Self>) {
        match self.collapsed.iter().position(|t| t == title) {
            Some(ix) => {
                self.collapsed.remove(ix);
            }
            None => self.collapsed.push(title.to_string()),
        }
        cx.emit(SidebarEvent::Changed);
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
        let collapsed = self.is_collapsed(title);
        v_flex()
            .w_full()
            // The header folds the group; a chevron shows the state.
            .child(
                h_flex()
                    .id(title)
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .gap_1()
                    .text_xs()
                    .text_color(c.muted_foreground)
                    .cursor_pointer()
                    .hover(|s| s.text_color(c.accent_foreground))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_group(title, cx)))
                    .child(
                        Icon::new(if collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .size_3()
                        .flex_none(),
                    )
                    .child(title),
            )
            .when(!collapsed, |d| {
                d.children(places.iter().enumerate().map(|(i, place)| {
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
                        // A colour tag: a thick stripe at the left edge.
                        .when_some(place.color, |d, color| {
                            d.border_l_4().border_color(color).pl_2()
                        })
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
                    // Its position in its own list: drag it onto another
                    // favorite of the same kind, or use the menu, to move it.
                    let is_layout = layout.is_some();
                    let c_drop = c.drop;
                    let index = match layout {
                        Some(ix) => ix,
                        None => self
                            .favorites
                            .iter()
                            .position(|f| f.path == place.path)
                            .unwrap_or(0),
                    };
                    let count = if is_layout {
                        self.layouts.len()
                    } else {
                        self.favorites.len()
                    };
                    let name: SharedString = place.name.clone().into();
                    let row = row
                        .on_drag(
                            DragFavorite {
                                layout: is_layout,
                                index,
                            },
                            move |_drag, _offset, _window, cx| {
                                let name = name.clone();
                                cx.new(|_| FavoriteDragPreview { name })
                            },
                        )
                        // The line shows where the favorite lands: under this
                        // row when it comes from above, over it otherwise.
                        .drag_over::<DragFavorite>(move |style, drag, _, _| {
                            if drag.layout != is_layout || drag.index == index {
                                style
                            } else if drag.index < index {
                                style.border_b_2().border_color(c_drop)
                            } else {
                                style.border_t_2().border_color(c_drop)
                            }
                        })
                        .on_drop(cx.listener(move |_this, drag: &DragFavorite, _, cx| {
                            if drag.layout == is_layout && drag.index != index {
                                cx.emit(SidebarEvent::MoveFavorite {
                                    layout: is_layout,
                                    from: drag.index,
                                    to: index,
                                });
                            }
                        }));
                    // Favorites: a context menu to rename, colour, move or
                    // drop them.
                    let sidebar = cx.entity();
                    let id = match layout {
                        Some(ix) => FavoriteId::Layout(ix),
                        None => FavoriteId::Path(place.path.clone()),
                    };
                    row.context_menu(move |menu, _, _| {
                        let emit = |event: fn(FavoriteId) -> SidebarEvent| {
                            let (sidebar, id) = (sidebar.clone(), id.clone());
                            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                sidebar.update(cx, |_, cx| cx.emit(event(id.clone())));
                            }
                        };
                        let move_to = |to: usize| {
                            let sidebar = sidebar.clone();
                            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                sidebar.update(cx, |_, cx| {
                                    cx.emit(SidebarEvent::MoveFavorite {
                                        layout: is_layout,
                                        from: index,
                                        to,
                                    })
                                });
                            }
                        };
                        let mut menu = menu
                            .item(
                                PopupMenuItem::new("Rename…")
                                    .on_click(emit(SidebarEvent::RenameFavorite)),
                            )
                            .item(
                                PopupMenuItem::new("Color…")
                                    .on_click(emit(SidebarEvent::ColorFavorite)),
                            );
                        if count > 1 {
                            menu = menu.separator();
                            if index > 0 {
                                menu = menu.item(
                                    PopupMenuItem::new("Move up").on_click(move_to(index - 1)),
                                );
                            }
                            if index + 1 < count {
                                menu = menu.item(
                                    PopupMenuItem::new("Move down").on_click(move_to(index + 1)),
                                );
                            }
                        }
                        menu.separator()
                            .item(PopupMenuItem::new("Remove from favorites").on_click(emit(
                                |id| match id {
                                    FavoriteId::Layout(ix) => SidebarEvent::RemoveLayout(ix),
                                    FavoriteId::Path(path) => SidebarEvent::RemoveFavorite(path),
                                },
                            )))
                    })
                    .into_any_element()
                }))
            })
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
    /// The line where a dragged favorite would land.
    drop: Hsla,
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let c = Colors {
            muted_foreground: theme.sidebar_foreground.alpha(0.7),
            accent: theme.sidebar_accent,
            accent_foreground: theme.sidebar_accent_foreground,
            hover: theme.list_hover,
            drop: theme.primary,
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
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("sidebar-scroll")
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            // Without the system places the group exists only
                            // while the user has favorites of their own.
                            .when(!places.is_empty(), |d| {
                                d.child(self.render_section("Favorites", &places, 0, &c, cx))
                            })
                            .when(!volumes.is_empty(), |d| {
                                d.child(self.render_section(
                                    "Volumes",
                                    &volumes,
                                    places.len(),
                                    &c,
                                    cx,
                                ))
                            })
                            .when(self.show_connections, |d| {
                                let offset = places.len() + volumes.len();
                                d.child(self.render_section(
                                    "Connections",
                                    &connections,
                                    offset,
                                    &c,
                                    cx,
                                ))
                            })
                            .when(
                                self.show_connections && !self.is_collapsed("Connections"),
                                |d| {
                                    d.child(
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
                                            .on_click(cx.listener(|_, _, _, cx| {
                                                cx.emit(SidebarEvent::ManageConnections)
                                            }))
                                            .child(Icon::new(IconName::Plus).size_4().flex_none())
                                            .child("Manage connections…"),
                                    )
                                },
                            )
                            .when(!recent.is_empty(), |d| {
                                let offset = places.len() + volumes.len() + connections.len();
                                d.child(self.render_section("Recent", &recent, offset, &c, cx))
                            }),
                    )
                    .child(Scrollbar::vertical(&self.scroll).viewport_from_layout()),
            )
    }
}
