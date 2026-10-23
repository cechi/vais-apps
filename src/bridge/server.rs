//! The listener: a Unix socket under the runtime directory (loopback TCP
//! with a token elsewhere), one line of JSON per request and per reply.
//!
//! Request: `{"id": 1, "tool": "navigate", "args": {...}}`
//! Reply:   `{"id": 1, "result": ...}` or `{"id": 1, "error": {"code", "message"}}`
//! `{"id": 0, "tool": "__tools"}` answers with the tool catalogue without
//! involving the UI.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::mpsc::sync_channel;
use std::time::Duration;

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use serde_json::{Value, json};

use super::{BridgeAddr, BridgeRequest, Reply, ToolError};

/// How long a request may wait for the workspace (confirmations wait for
/// the user).
const REPLY_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub fn start() -> io::Result<(BridgeAddr, UnboundedReceiver<BridgeRequest>)> {
    let (tx, rx) = unbounded();
    let addr = listen(tx)?;
    Ok((addr, rx))
}

#[cfg(unix)]
fn listen(tx: UnboundedSender<BridgeRequest>) -> io::Result<BridgeAddr> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    let dir = directories::BaseDirs::new()
        .and_then(|d| d.runtime_dir().map(|r| r.to_path_buf()))
        .or_else(|| {
            directories::ProjectDirs::from("", "", "vais-commander")
                .map(|d| d.cache_dir().to_path_buf())
        })
        .ok_or_else(|| io::Error::other("no runtime directory"))?
        .join("vais-commander");
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    let path = dir.join(format!("bridge-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    std::thread::Builder::new()
        .name("bridge".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    continue;
                };
                let tx = tx.clone();
                let _ = std::thread::Builder::new()
                    .name("bridge-conn".into())
                    .spawn(move || serve(stream, tx, None));
            }
        })?;
    Ok(BridgeAddr::Unix(path))
}

#[cfg(not(unix))]
fn listen(tx: UnboundedSender<BridgeRequest>) -> io::Result<BridgeAddr> {
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let token = random_token();
    let expected = token.clone();
    std::thread::Builder::new()
        .name("bridge".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    continue;
                };
                let (tx, expected) = (tx.clone(), expected.clone());
                let _ = std::thread::Builder::new()
                    .name("bridge-conn".into())
                    .spawn(move || serve(stream, tx, Some(expected)));
            }
        })?;
    Ok(BridgeAddr::Tcp { port, token })
}

#[cfg(not(unix))]
fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    (0..4)
        .map(|i| {
            let mut h = RandomState::new().build_hasher();
            h.write_u64(i);
            format!("{:016x}", h.finish())
        })
        .collect()
}

/// One connection: a line in, a line out, until it closes.
fn serve<S: Read + Write>(stream: S, tx: UnboundedSender<BridgeRequest>, token: Option<String>) {
    let mut reader = BufReader::new(stream);
    let mut authenticated = token.is_none();
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(err) => {
                let _ = write_line(
                    reader.get_mut(),
                    &json!({ "id": null, "error": ToolError::invalid(format!("bad JSON: {err}")).to_json() }),
                );
                continue;
            }
        };
        if !authenticated {
            if request.get("auth").and_then(Value::as_str) == token.as_deref() {
                authenticated = true;
                let _ = write_line(
                    reader.get_mut(),
                    &json!({ "id": request.get("id"), "result": "ok" }),
                );
            } else {
                let _ = write_line(
                    reader.get_mut(),
                    &json!({ "id": request.get("id"), "error": ToolError::rejected("bad token").to_json() }),
                );
                return;
            }
            continue;
        }
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let tool = request
            .get("tool")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let args = request.get("args").cloned().unwrap_or_else(|| json!({}));
        let result = if tool == "__tools" {
            Ok(super::tools::catalog())
        } else {
            let (reply_tx, reply_rx) = sync_channel(1);
            let sent = tx
                .unbounded_send(BridgeRequest {
                    tool,
                    args,
                    reply: Reply::new(reply_tx),
                })
                .is_ok();
            if !sent {
                Err(ToolError::new("internal", "the app is shutting down"))
            } else {
                match reply_rx.recv_timeout(REPLY_TIMEOUT) {
                    Ok(result) => result,
                    Err(_) => Err(ToolError::new("timeout", "no answer from the app")),
                }
            }
        };
        let message = match result {
            Ok(value) => json!({ "id": id, "result": value }),
            Err(err) => json!({ "id": id, "error": err.to_json() }),
        };
        if write_line(reader.get_mut(), &message).is_err() {
            return;
        }
    }
}

fn write_line<W: Write>(out: &mut W, value: &Value) -> io::Result<()> {
    let mut text = serde_json::to_string(value).map_err(io::Error::other)?;
    text.push('\n');
    out.write_all(text.as_bytes())?;
    out.flush()
}
