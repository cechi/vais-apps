//! Saved connections (SFTP for now) and their live state.
//!
//! Three layers: the configs come from the config file; a process-wide
//! registry hands the open backends to code that has no app context
//! (`vfs::backend_for` on background threads); and the `ConnectionManager`
//! entity drives connecting on the background executor and tells the
//! workspace about changes and password prompts.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use gpui_kit::*;
use serde::{Deserialize, Serialize};

use crate::vfs::{Backend, ConnId, Location};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    #[default]
    Sftp,
    Ftp,
}

impl Protocol {
    pub const ALL: [Protocol; 2] = [Protocol::Sftp, Protocol::Ftp];

    pub fn label(self) -> &'static str {
        match self {
            Protocol::Sftp => "SFTP",
            Protocol::Ftp => "FTP",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Protocol::Sftp => 22,
            Protocol::Ftp => 21,
        }
    }

    /// FTP knows only passwords (or anonymous access).
    pub fn uses_keys(self) -> bool {
        matches!(self, Protocol::Sftp)
    }
}

/// How to authenticate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    /// The SSH agent, then the usual key files, then a password if asked.
    #[default]
    Auto,
    /// A specific private key file.
    Key,
    /// A password (kept in the system keyring or asked for).
    Password,
}

impl AuthMethod {
    pub const ALL: [AuthMethod; 3] = [AuthMethod::Auto, AuthMethod::Key, AuthMethod::Password];

    pub fn label(self) -> &'static str {
        match self {
            AuthMethod::Auto => "Agent / keys",
            AuthMethod::Key => "Key file",
            AuthMethod::Password => "Password",
        }
    }
}

/// One saved connection, as in the config file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectionConfig {
    /// Stable short id, also the first part of `id:/path` locations.
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub user: String,
    /// The directory opened on connect.
    pub path: String,
    pub auth: AuthMethod,
    /// Private key file for `AuthMethod::Key`.
    pub key: Option<PathBuf>,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            protocol: Protocol::Sftp,
            host: String::new(),
            port: 22,
            user: String::new(),
            path: "/".into(),
            auth: AuthMethod::Auto,
            key: None,
        }
    }
}

impl ConnectionConfig {
    pub fn conn_id(&self) -> ConnId {
        ConnId(self.id.clone())
    }

    /// The location a click on the connection opens.
    pub fn home(&self) -> Location {
        let path = if self.path.trim().is_empty() {
            "/"
        } else {
            self.path.trim()
        };
        Location::remote(self.id.clone(), path)
    }

    /// A usable id from the name (`My NAS` → `my-nas`), unique among `taken`.
    pub fn slug(name: &str, taken: &[String]) -> String {
        let mut base: String = name
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_string();
        while base.contains("--") {
            base = base.replace("--", "-");
        }
        if base.len() < 2 {
            base = format!("conn-{}", base);
        }
        let mut candidate = base.clone();
        let mut n = 2;
        while taken.iter().any(|t| *t == candidate) {
            candidate = format!("{base}-{n}");
            n += 1;
        }
        candidate
    }
}

// ----- the registry background code reads ---------------------------------

fn registry() -> &'static RwLock<HashMap<ConnId, Arc<dyn Backend>>> {
    static REGISTRY: OnceLock<RwLock<HashMap<ConnId, Arc<dyn Backend>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

/// The open backend of a connection, if it is connected.
pub fn backend(conn: &ConnId) -> Option<Arc<dyn Backend>> {
    registry().read().ok()?.get(conn).cloned()
}

fn register(conn: ConnId, backend: Arc<dyn Backend>) {
    if let Ok(mut map) = registry().write() {
        map.insert(conn, backend);
    }
}

fn unregister(conn: &ConnId) {
    if let Ok(mut map) = registry().write() {
        map.remove(conn);
    }
}

// ----- passwords -------------------------------------------------------------

const KEYRING_SERVICE: &str = "vais-commander";

fn keyring_entry(config: &ConnectionConfig) -> Option<keyring::Entry> {
    keyring::Entry::new(
        KEYRING_SERVICE,
        &format!("{}@{}:{}", config.user, config.host, config.port),
    )
    .ok()
}

pub fn stored_password(config: &ConnectionConfig) -> Option<String> {
    keyring_entry(config)?.get_password().ok()
}

pub fn remember_password(config: &ConnectionConfig, password: &str) {
    if let Some(entry) = keyring_entry(config)
        && let Err(err) = entry.set_password(password)
    {
        log::warn!("keyring: {err}");
    }
}

pub fn forget_password(config: &ConnectionConfig) {
    if let Some(entry) = keyring_entry(config) {
        let _ = entry.delete_credential();
    }
}

// ----- the manager -----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Failed(String),
}

pub enum ConnectionEvent {
    /// A connection's status changed (panels on it should reload).
    Changed(ConnId),
    /// Connecting needs a password the keyring does not have.
    NeedsPassword(ConnId),
}

/// What connecting can come back with.
pub enum ConnectError {
    NeedsPassword,
    Other(String),
}

pub struct ConnectionManager {
    configs: Vec<ConnectionConfig>,
    status: HashMap<ConnId, Status>,
    /// When each connection was last tried, so a failing one is not retried
    /// in a loop by every panel reload.
    last_attempt: HashMap<ConnId, std::time::Instant>,
}

/// A failed connection is tried again on demand at most this often.
const RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(10);

impl EventEmitter<ConnectionEvent> for ConnectionManager {}

struct Handle(Entity<ConnectionManager>);
impl Global for Handle {}

pub fn init(configs: Vec<ConnectionConfig>, cx: &mut App) -> Entity<ConnectionManager> {
    let manager = cx.new(|_| ConnectionManager {
        configs,
        status: HashMap::new(),
        last_attempt: HashMap::new(),
    });
    cx.set_global(Handle(manager.clone()));
    manager
}

pub fn manager(cx: &App) -> Entity<ConnectionManager> {
    cx.global::<Handle>().0.clone()
}

/// Make sure `conn` is connected or connecting; a panel calls this before
/// listing a remote directory. Returns the current status.
pub fn ensure(conn: &ConnId, cx: &mut App) -> Status {
    let manager = manager(cx);
    manager.update(cx, |this, cx| {
        let status = this.status(conn);
        let retry_due = this
            .last_attempt
            .get(conn)
            .is_none_or(|at| at.elapsed() >= RETRY_AFTER);
        if status == Status::Disconnected || (matches!(status, Status::Failed(_)) && retry_due) {
            this.connect(conn, None, false, cx);
        }
        this.status(conn)
    })
}

impl ConnectionManager {
    pub fn configs(&self) -> &[ConnectionConfig] {
        &self.configs
    }

    pub fn config(&self, conn: &ConnId) -> Option<&ConnectionConfig> {
        self.configs.iter().find(|c| c.id == conn.0)
    }

    pub fn status(&self, conn: &ConnId) -> Status {
        self.status.get(conn).cloned().unwrap_or_default()
    }

    /// Replace the saved connections (from the connection manager tab).
    /// Connections that disappeared are closed.
    pub fn set_configs(&mut self, configs: Vec<ConnectionConfig>, cx: &mut Context<Self>) {
        for old in &self.configs {
            let changed = configs
                .iter()
                .find(|c| c.id == old.id)
                .is_none_or(|c| c != old);
            if changed {
                let conn = old.conn_id();
                unregister(&conn);
                self.status.remove(&conn);
                cx.emit(ConnectionEvent::Changed(conn));
            }
        }
        self.configs = configs;
        cx.notify();
    }

    pub fn disconnect(&mut self, conn: &ConnId, cx: &mut Context<Self>) {
        unregister(conn);
        self.status.insert(conn.clone(), Status::Disconnected);
        cx.emit(ConnectionEvent::Changed(conn.clone()));
        cx.notify();
    }

    /// Open `conn` on the background executor. Without a password the
    /// keyring is tried (for password connections right away, otherwise only
    /// once keys failed); when that is not enough the workspace is asked.
    pub fn connect(
        &mut self,
        conn: &ConnId,
        password: Option<String>,
        remember: bool,
        cx: &mut Context<Self>,
    ) {
        self.connect_inner(conn, password, remember, false, cx);
    }

    fn connect_inner(
        &mut self,
        conn: &ConnId,
        password: Option<String>,
        remember: bool,
        keyring_tried: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(config) = self.config(conn).cloned() else {
            self.status
                .insert(conn.clone(), Status::Failed("unknown connection".into()));
            cx.emit(ConnectionEvent::Changed(conn.clone()));
            return;
        };
        if self.status(conn) == Status::Connecting {
            return;
        }
        self.status.insert(conn.clone(), Status::Connecting);
        self.last_attempt
            .insert(conn.clone(), std::time::Instant::now());
        cx.emit(ConnectionEvent::Changed(conn.clone()));
        cx.notify();

        let conn = conn.clone();
        let (password, keyring_tried) = match password {
            Some(p) => (Some(p), keyring_tried),
            None if config.auth == AuthMethod::Password
                || !config.protocol.uses_keys()
                || keyring_tried =>
            {
                (stored_password(&config), true)
            }
            None => (None, false),
        };
        log::debug!(
            "connecting {}: password {}, keyring tried {keyring_tried}",
            conn,
            if password.is_some() { "given" } else { "none" }
        );
        let task = cx.background_spawn({
            let config = config.clone();
            let password = password.clone();
            async move {
                match config.protocol {
                    Protocol::Sftp => crate::vfs::sftp::connect(&config, password.as_deref()),
                    Protocol::Ftp => crate::vfs::ftp::connect(&config, password.as_deref()),
                }
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(backend) => {
                        if remember && let Some(password) = &password {
                            remember_password(&config, password);
                        }
                        register(conn.clone(), backend);
                        this.status.insert(conn.clone(), Status::Connected);
                        cx.emit(ConnectionEvent::Changed(conn));
                    }
                    Err(ConnectError::NeedsPassword) => {
                        this.status.insert(conn.clone(), Status::Disconnected);
                        if !keyring_tried && stored_password(&config).is_some() {
                            // Keys did not work; the keyring may have a password.
                            this.connect_inner(&conn, None, false, true, cx);
                        } else {
                            cx.emit(ConnectionEvent::NeedsPassword(conn));
                        }
                    }
                    Err(ConnectError::Other(err)) => {
                        log::warn!("{conn}: {err}");
                        this.status.insert(conn.clone(), Status::Failed(err));
                        cx.emit(ConnectionEvent::Changed(conn));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::ConnectionConfig;

    /// Touches the real keyring: `cargo test -- --ignored keyring_roundtrip`.
    #[test]
    #[ignore]
    fn keyring_roundtrip() {
        let entry = keyring::Entry::new("vais-commander-test", "roundtrip").unwrap();
        entry.set_password("s3cret").unwrap();
        assert_eq!(entry.get_password().unwrap(), "s3cret");
        entry.delete_credential().unwrap();
        assert!(entry.get_password().is_err());
    }

    #[test]
    fn slugs_are_short_unique_and_safe() {
        assert_eq!(ConnectionConfig::slug("My NAS", &[]), "my-nas");
        assert_eq!(ConnectionConfig::slug("  web (prod) ", &[]), "web-prod");
        assert_eq!(
            ConnectionConfig::slug("nas", &["nas".into(), "nas-2".into()]),
            "nas-3"
        );
        assert_eq!(ConnectionConfig::slug("x", &[]), "conn-x");
    }
}
