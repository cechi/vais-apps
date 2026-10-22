//! The connection manager tab: saved connections on the left, a form for
//! the selected one on the right. Saving hands the whole list back to the
//! workspace, which stores it in the config and updates the manager.

use std::collections::HashMap;
use std::path::PathBuf;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Icon, IconName, Sizable as _, StyledExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::connections::{AuthMethod, ConnectionConfig, Protocol, Status};
use crate::keymap::*;
use crate::ui::{h_flex, v_flex};
use crate::vfs::ConnId;

pub enum ConnectionsEvent {
    /// The list changed (added, edited, removed): persist and apply it.
    Save(Vec<ConnectionConfig>),
    /// Connect and open the connection's directory in the active panel.
    Connect(ConnId),
    Disconnect(ConnId),
    Close,
}

pub struct ConnectionsView {
    items: Vec<ConnectionConfig>,
    selected: Option<usize>,
    status: HashMap<ConnId, Status>,
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    user: Entity<InputState>,
    path: Entity<InputState>,
    key: Entity<InputState>,
    password: Entity<InputState>,
    protocol: Protocol,
    auth: AuthMethod,
    remember: bool,
    dirty: bool,
    error: Option<String>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ConnectionsEvent> for ConnectionsView {}

impl ConnectionsView {
    pub fn new(
        items: Vec<ConnectionConfig>,
        status: HashMap<ConnId, Status>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = Vec::new();
        let mut input = |placeholder: &str, masked: bool| {
            let state = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder.to_string())
                    .masked(masked)
            });
            subscriptions.push(cx.subscribe_in(
                &state,
                window,
                |this, _, event: &InputEvent, _, cx| {
                    if let InputEvent::Change = event
                        && !this.dirty
                    {
                        this.dirty = true;
                        cx.notify();
                    }
                },
            ));
            state
        };
        let name = input("Name", false);
        let host = input("host or address", false);
        let port = input("22", false);
        let user = input("user", false);
        let path = input("/", false);
        let key = input("~/.ssh/id_ed25519", false);
        let password = input("password", true);
        let mut this = Self {
            selected: (!items.is_empty()).then_some(0),
            items,
            status,
            name,
            host,
            port,
            user,
            path,
            key,
            password,
            protocol: Protocol::Sftp,
            auth: AuthMethod::Auto,
            remember: false,
            dirty: false,
            error: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        this.load_selected(window, cx);
        this
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    /// The manager's statuses changed.
    pub fn set_status(&mut self, status: HashMap<ConnId, Status>, cx: &mut Context<Self>) {
        self.status = status;
        cx.notify();
    }

    fn set_text(
        &self,
        input: &Entity<InputState>,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        input.update(cx, |input, cx| input.set_value(text, window, cx));
    }

    /// Show the selected connection in the form.
    fn load_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = self
            .selected
            .and_then(|ix| self.items.get(ix))
            .cloned()
            .unwrap_or_default();
        self.set_text(&self.name, config.name.clone(), window, cx);
        self.set_text(&self.host, config.host.clone(), window, cx);
        self.set_text(&self.port, config.port.to_string(), window, cx);
        self.set_text(&self.user, config.user.clone(), window, cx);
        self.set_text(&self.path, config.path.clone(), window, cx);
        self.set_text(
            &self.key,
            config
                .key
                .as_ref()
                .map(|k| k.to_string_lossy().into_owned())
                .unwrap_or_default(),
            window,
            cx,
        );
        self.set_text(&self.password, String::new(), window, cx);
        self.protocol = config.protocol;
        self.auth = config.auth;
        self.remember = false;
        self.dirty = false;
        self.error = None;
        cx.notify();
    }

    fn select(&mut self, ix: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = ix;
        self.load_selected(window, cx);
    }

    /// The form as a config, or why it cannot be saved.
    fn read_form(&self, cx: &App) -> Result<ConnectionConfig, String> {
        let text = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        let name = text(&self.name);
        let host = text(&self.host);
        if host.is_empty() {
            return Err("host is required".into());
        }
        let port = match text(&self.port).as_str() {
            "" => self.protocol.default_port(),
            p => p.parse().map_err(|_| "port must be a number".to_string())?,
        };
        // An empty SFTP user is fine when ~/.ssh/config knows the host.
        let user = text(&self.user);
        if user.is_empty()
            && self.protocol.uses_keys()
            && crate::ssh_config::SshConfig::load()
                .resolve(&host)
                .user
                .is_none()
        {
            return Err("user is required (or set it for this host in ~/.ssh/config)".into());
        }
        let path = match text(&self.path).as_str() {
            "" => "/".to_string(),
            p => p.to_string(),
        };
        let key_text = text(&self.key);
        let key = (!key_text.is_empty()).then(|| PathBuf::from(expand_home(&key_text)));
        let auth = if self.protocol.uses_keys() {
            self.auth
        } else {
            AuthMethod::Password
        };
        if auth == AuthMethod::Key && key.is_none() {
            return Err("a key file is required".into());
        }
        let existing = self.selected.and_then(|ix| self.items.get(ix));
        let id = match existing {
            Some(c) => c.id.clone(),
            None => {
                let taken: Vec<String> = self.items.iter().map(|c| c.id.clone()).collect();
                ConnectionConfig::slug(if name.is_empty() { &host } else { &name }, &taken)
            }
        };
        Ok(ConnectionConfig {
            id,
            name: if name.is_empty() { host.clone() } else { name },
            protocol: self.protocol,
            host,
            port,
            user,
            path,
            auth,
            key,
        })
    }

    /// Store the form in the list (adding a new entry when none is
    /// selected) and tell the workspace. Returns the saved config.
    fn save(&mut self, cx: &mut Context<Self>) -> Option<ConnectionConfig> {
        let config = match self.read_form(cx) {
            Ok(config) => config,
            Err(err) => {
                self.error = Some(err);
                cx.notify();
                return None;
            }
        };
        let password = self.password.read(cx).value().trim().to_string();
        if !password.is_empty() && self.remember {
            crate::connections::remember_password(&config, &password);
        }
        match self.selected {
            Some(ix) if ix < self.items.len() => self.items[ix] = config.clone(),
            _ => {
                self.items.push(config.clone());
                self.selected = Some(self.items.len() - 1);
            }
        }
        self.dirty = false;
        self.error = None;
        cx.emit(ConnectionsEvent::Save(self.items.clone()));
        cx.notify();
        Some(config)
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        if let Some(config) = self.save(cx) {
            cx.emit(ConnectionsEvent::Connect(config.conn_id()));
        }
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.selected.filter(|&ix| ix < self.items.len()) else {
            return;
        };
        let removed = self.items.remove(ix);
        crate::connections::forget_password(&removed);
        cx.emit(ConnectionsEvent::Disconnect(removed.conn_id()));
        cx.emit(ConnectionsEvent::Save(self.items.clone()));
        let next = if self.items.is_empty() {
            None
        } else {
            Some(ix.min(self.items.len() - 1))
        };
        self.select(next, window, cx);
    }

    fn on_close(&mut self, _: &CloseConnections, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(ConnectionsEvent::Close);
    }

    /// Pick a host from `~/.ssh/config` and start a new connection from it.
    fn import_from_ssh_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use crate::palette::{self, Palette, PaletteItem};
        let hosts = crate::ssh_config::SshConfig::load().aliases();
        let items: Vec<PaletteItem> = hosts
            .iter()
            .map(|h| {
                let detail = match (&h.user, h.port) {
                    (Some(user), Some(port)) => format!("{user}@{}:{port}", h.address()),
                    (Some(user), None) => format!("{user}@{}", h.address()),
                    (None, Some(port)) => format!("{}:{port}", h.address()),
                    (None, None) => h.address().to_string(),
                };
                PaletteItem::new(h.alias.clone())
                    .detail(detail)
                    .icon(IconName::Globe)
            })
            .collect();
        let view = cx.entity();
        let palette = cx.new(|cx| {
            Palette::pick_filtered(items, "Filter hosts", window, cx)
                .hint("No Host entries in ~/.ssh/config")
                .on_confirm(move |choice, window, cx| {
                    window.close_dialog(cx);
                    let Some(host) = choice.item.and_then(|ix| hosts.get(ix).cloned()) else {
                        return;
                    };
                    let view = view.clone();
                    window.defer(cx, move |window, cx| {
                        view.update(cx, |this, cx| this.fill_from_ssh_host(&host, window, cx));
                    });
                })
                .on_cancel(|window, cx| window.close_dialog(cx))
        });
        palette::open("Add from ssh config".to_string(), palette, window, cx);
    }

    /// A new, unsaved connection for an ssh config host: the alias as the
    /// host (so later changes to the ssh config still apply) with the known
    /// values shown.
    fn fill_from_ssh_host(
        &mut self,
        host: &crate::ssh_config::SshHost,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected = None;
        self.load_selected(window, cx);
        self.protocol = Protocol::Sftp;
        self.auth = AuthMethod::Auto;
        self.set_text(&self.name, host.alias.clone(), window, cx);
        self.set_text(&self.host, host.alias.clone(), window, cx);
        self.set_text(
            &self.port,
            host.port
                .unwrap_or(Protocol::Sftp.default_port())
                .to_string(),
            window,
            cx,
        );
        self.set_text(
            &self.user,
            host.user.clone().unwrap_or_default(),
            window,
            cx,
        );
        self.set_text(
            &self.key,
            host.identity_files
                .first()
                .map(|k| k.to_string_lossy().into_owned())
                .unwrap_or_default(),
            window,
            cx,
        );
        self.dirty = true;
        cx.notify();
    }

    fn field(label: &'static str, input: &Entity<InputState>, muted: Hsla) -> Div {
        v_flex()
            .gap_1()
            .child(div().text_xs().text_color(muted).child(label))
            .child(Input::new(input).small().focus_bordered(false))
    }
}

/// `~/x` relative to the home directory.
fn expand_home(text: &str) -> String {
    if let Some(rest) = text.strip_prefix("~/") {
        return crate::config::home_dir()
            .join(rest)
            .to_string_lossy()
            .into_owned();
    }
    text.to_string()
}

impl Render for ConnectionsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (bg, fg, muted, border, accent, accent_fg, hover, danger, list_head) = (
            theme.background,
            theme.foreground,
            theme.muted_foreground,
            theme.border,
            theme.sidebar_accent,
            theme.sidebar_accent_foreground,
            theme.list_hover,
            theme.danger,
            theme.list_head,
        );
        let selected = self.selected;
        let selected_status = selected
            .and_then(|ix| self.items.get(ix))
            .map(|c| self.status.get(&c.conn_id()).cloned().unwrap_or_default());
        let is_connected = matches!(selected_status, Some(Status::Connected));
        let has_selection = selected.is_some_and(|ix| ix < self.items.len());

        let rows: Vec<AnyElement> = self
            .items
            .iter()
            .enumerate()
            .map(|(ix, config)| {
                let status = self
                    .status
                    .get(&config.conn_id())
                    .cloned()
                    .unwrap_or_default();
                let is_selected = selected == Some(ix);
                let dot = match status {
                    Status::Connected => theme.green,
                    Status::Connecting => theme.yellow,
                    Status::Failed(_) => theme.danger,
                    Status::Disconnected => muted.alpha(0.4),
                };
                h_flex()
                    .id(("connection", ix))
                    .w_full()
                    .h(rems(1.75))
                    .px_3()
                    .gap_2()
                    .cursor_pointer()
                    .text_sm()
                    .when(is_selected, |d| d.bg(accent).text_color(accent_fg))
                    .when(!is_selected, |d| d.hover(move |s| s.bg(hover)))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.select(Some(ix), window, cx)),
                    )
                    .child(div().size_2().rounded_full().bg(dot).flex_none())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(config.name.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(if is_selected { accent_fg } else { muted })
                            .child(format!("{}@{}", config.user, config.host)),
                    )
                    .into_any_element()
            })
            .collect();

        let protocol_buttons = Protocol::ALL.iter().map(|&protocol| {
            let button = Button::new(("protocol", protocol as usize))
                .small()
                .label(protocol.label());
            let button = if protocol == self.protocol {
                button.primary()
            } else {
                button.ghost()
            };
            button.on_click(cx.listener(move |this, _, window, cx| {
                if this.protocol != protocol {
                    this.protocol = protocol;
                    this.dirty = true;
                    // A fresh form keeps the protocol's usual port.
                    let port = this.port.read(cx).value().trim().to_string();
                    if port.is_empty() || port == "21" || port == "22" {
                        this.set_text(&this.port, protocol.default_port().to_string(), window, cx);
                    }
                    cx.notify();
                }
            }))
        });
        let uses_keys = self.protocol.uses_keys();
        let auth_buttons = AuthMethod::ALL.iter().map(|&method| {
            let button = Button::new(("auth", method as usize))
                .small()
                .label(method.label());
            let button = if method == self.auth {
                button.primary()
            } else {
                button.ghost()
            };
            button.on_click(cx.listener(move |this, _, _, cx| {
                this.auth = method;
                this.dirty = true;
                cx.notify();
            }))
        });

        let status_text = match &selected_status {
            Some(Status::Connected) => "Connected".to_string(),
            Some(Status::Connecting) => "Connecting…".to_string(),
            Some(Status::Failed(err)) => format!("Failed: {err}"),
            _ => String::new(),
        };

        h_flex()
            .id("connections")
            .key_context(CONNECTIONS_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_close))
            .size_full()
            .min_h_0()
            .items_start()
            .bg(bg)
            .text_color(fg)
            .child(
                v_flex()
                    .w(rems(18.))
                    .h_full()
                    .flex_none()
                    .border_r_1()
                    .border_color(border)
                    .child(
                        h_flex()
                            .h(rems(crate::app::HEADER_HEIGHT_REMS))
                            .flex_none()
                            .px_3()
                            .gap_2()
                            .bg(list_head)
                            .border_b_1()
                            .border_color(border)
                            .child(div().flex_1().font_semibold().child("Connections"))
                            .child(
                                Button::new("connections-import")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Globe)
                                    .label("ssh config")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.import_from_ssh_config(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("connections-new")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Plus)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.select(None, window, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("connections-list")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .children(rows)
                            .when(self.items.is_empty(), |d| {
                                d.child(
                                    div()
                                        .px_3()
                                        .py_2()
                                        .text_sm()
                                        .text_color(muted)
                                        .child("No connections yet. Fill in the form and save."),
                                )
                            }),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .p_4()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().font_semibold().text_lg().child(if has_selection {
                                "Edit connection"
                            } else {
                                "New connection"
                            }))
                            .child(div().flex_1())
                            .child(
                                Button::new("connections-close")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Close)
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(ConnectionsEvent::Close)
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .items_end()
                            .child(Self::field("Name", &self.name, muted).flex_1())
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().text_xs().text_color(muted).child("Protocol"))
                                    .child(h_flex().gap_1().children(protocol_buttons)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .items_end()
                            .child(Self::field("Host", &self.host, muted).flex_1())
                            .child(Self::field("Port", &self.port, muted).w(rems(5.)))
                            .child(Self::field("User", &self.user, muted).w(rems(10.))),
                    )
                    .child(Self::field("Directory", &self.path, muted))
                    .when(uses_keys, |form| {
                        form.child(
                            v_flex()
                                .gap_1()
                                .child(div().text_xs().text_color(muted).child("Authentication"))
                                .child(h_flex().gap_1().children(auth_buttons)),
                        )
                    })
                    .when(uses_keys && self.auth != AuthMethod::Password, |form| {
                        form.child(Self::field("Private key (optional)", &self.key, muted))
                    })
                    .child(
                        h_flex()
                            .gap_3()
                            .items_end()
                            .child(Self::field("Password", &self.password, muted).flex_1())
                            .child(
                                h_flex().h(rems(1.75)).child(
                                    Checkbox::new("connections-remember")
                                        .label("Remember in the system keyring")
                                        .checked(self.remember)
                                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                            this.remember = *checked;
                                            cx.notify();
                                        })),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(if uses_keys {
                                "Host may be an alias from ~/.ssh/config: its HostName, Port, User and IdentityFile apply where the fields are empty or at their defaults. Without a stored password you are asked when connecting; Agent / keys tries the SSH agent and ~/.ssh keys first."
                            } else {
                                "Plain FTP, passive mode. Leave the user empty for anonymous access; otherwise you are asked for the password unless it is stored."
                            }),
                    )
                    .when_some(self.error.clone(), |form, error| {
                        form.child(div().text_sm().text_color(danger).child(error))
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .pt_2()
                            .child(
                                div()
                                    .flex_1()
                                    .text_sm()
                                    .text_color(muted)
                                    .child(status_text),
                            )
                            .when(has_selection, |bar| {
                                bar.child(
                                    Button::new("connections-delete")
                                        .small()
                                        .danger()
                                        .label("Remove")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.delete(window, cx)
                                        })),
                                )
                            })
                            .when(is_connected, |bar| {
                                let conn = selected
                                    .and_then(|ix| self.items.get(ix))
                                    .map(|c| c.conn_id());
                                bar.child(
                                    Button::new("connections-disconnect")
                                        .small()
                                        .label("Disconnect")
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            if let Some(conn) = conn.clone() {
                                                cx.emit(ConnectionsEvent::Disconnect(conn));
                                            }
                                        })),
                                )
                            })
                            .child(
                                Button::new("connections-save")
                                    .small()
                                    .label("Save")
                                    .disabled(!self.dirty && has_selection)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.save(cx);
                                    })),
                            )
                            .child(
                                Button::new("connections-connect")
                                    .small()
                                    .primary()
                                    .icon(IconName::Globe)
                                    .label("Connect")
                                    .on_click(cx.listener(|this, _, _, cx| this.connect(cx))),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("Connections are saved in the config file (without passwords). The remote directory opens in the active panel; its address is written as id:/path."),
                    ),
            )
            .child(div().flex_none().child(Icon::new(IconName::Globe).size_4().opacity(0.)))
    }
}
