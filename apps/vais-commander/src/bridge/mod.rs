//! The bridge: a local socket on which another process (our own `--mcp`
//! server, started by a coding agent) asks the running app to do things.
//! Requests arrive on their own threads and are handed to the workspace on
//! the foreground; the reply travels back through [`Reply`].

pub mod server;
pub mod tools;

use std::io;
use std::path::PathBuf;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};

use futures::channel::mpsc::UnboundedReceiver;
use serde_json::{Value, json};

/// Where the bridge listens; exported to agents through the environment.
#[derive(Debug, Clone)]
pub enum BridgeAddr {
    Unix(PathBuf),
    /// Loopback TCP with a secret the first line of a connection must carry.
    Tcp {
        port: u16,
        token: String,
    },
}

impl BridgeAddr {
    /// `VAIS_BRIDGE` (and `VAIS_BRIDGE_TOKEN`) for a child process.
    pub fn env(&self) -> Vec<(String, String)> {
        match self {
            BridgeAddr::Unix(path) => {
                vec![("VAIS_BRIDGE".into(), format!("unix:{}", path.display()))]
            }
            BridgeAddr::Tcp { port, token } => vec![
                ("VAIS_BRIDGE".into(), format!("tcp://127.0.0.1:{port}")),
                ("VAIS_BRIDGE_TOKEN".into(), token.clone()),
            ],
        }
    }

    /// The address a child was given, from its environment; without one,
    /// the newest instance's socket (`bridge.sock`), so an agent started
    /// outside the app can still reach it.
    pub fn from_env() -> Option<Self> {
        let Ok(value) = std::env::var("VAIS_BRIDGE") else {
            return Self::latest();
        };
        if let Some(path) = value.strip_prefix("unix:") {
            return Some(BridgeAddr::Unix(PathBuf::from(path)));
        }
        let port = value.strip_prefix("tcp://127.0.0.1:")?.parse().ok()?;
        let token = std::env::var("VAIS_BRIDGE_TOKEN").unwrap_or_default();
        Some(BridgeAddr::Tcp { port, token })
    }
}

/// Why a tool call did not produce a result.
#[derive(Debug, Clone)]
pub struct ToolError {
    pub code: &'static str,
    pub message: String,
}

impl ToolError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid_args", message)
    }

    pub fn rejected(message: impl Into<String>) -> Self {
        Self::new("rejected", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message)
    }

    pub fn to_json(&self) -> Value {
        json!({ "code": self.code, "message": self.message })
    }
}

pub type ToolResult = Result<Value, ToolError>;

/// The answer slot of a request. Clone so palette closures can hold it;
/// answering twice does nothing.
#[derive(Clone)]
pub struct Reply(Arc<Mutex<Option<SyncSender<ToolResult>>>>);

impl Reply {
    pub fn new(sender: SyncSender<ToolResult>) -> Self {
        Self(Arc::new(Mutex::new(Some(sender))))
    }

    pub fn send(&self, result: ToolResult) {
        if let Ok(mut slot) = self.0.lock()
            && let Some(sender) = slot.take()
        {
            let _ = sender.send(result);
        }
    }
}

/// One tool call waiting for the workspace.
pub struct BridgeRequest {
    pub tool: String,
    pub args: Value,
    pub reply: Reply,
}

/// Start listening; the receiver delivers requests to the foreground.
pub fn start() -> io::Result<(BridgeAddr, UnboundedReceiver<BridgeRequest>)> {
    server::start()
}

impl BridgeAddr {
    #[cfg(unix)]
    fn latest() -> Option<Self> {
        let link = server::socket_dir()?.join("bridge.sock");
        let target = std::fs::read_link(&link).ok()?;
        target.exists().then_some(BridgeAddr::Unix(target))
    }

    #[cfg(not(unix))]
    fn latest() -> Option<Self> {
        None
    }
}

/// Remove what the listener left behind (the socket file and, when it
/// still points here, the `bridge.sock` link).
pub fn cleanup(addr: &BridgeAddr) {
    if let BridgeAddr::Unix(path) = addr {
        let _ = std::fs::remove_file(path);
        if let Some(link) = path.parent().map(|d| d.join("bridge.sock"))
            && std::fs::read_link(&link).is_ok_and(|t| t == *path)
        {
            let _ = std::fs::remove_file(link);
        }
    }
}
