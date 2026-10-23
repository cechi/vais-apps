//! `vais-commander --mcp`: a Model Context Protocol server on stdio for a
//! coding agent, forwarding every tool call to the running app over the
//! bridge socket. JSON-RPC 2.0, one message per line; logs go to stderr.

use std::io::{self, BufRead, BufReader, Read, Write};

use serde_json::{Value, json};

use crate::bridge::{BridgeAddr, ToolError};

const SERVER_NAME: &str = "vais-commander";
/// Protocol versions we speak; a client asking for one of them gets it back.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];
const INSTRUCTIONS: &str = "VAIS Commander is a file manager with tabs; each tab holds a grid of file panels (rows x cols), one of them active. Paths are local, `conn:/path` on a saved connection, or `archive.zip!/dir` inside an archive. Start with get_state to see the tabs and panels.";

/// The process entry point behind `--mcp`.
pub fn run() -> i32 {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut session = Session::default();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle(&line, &mut session)
            && (writeln!(out, "{reply}").is_err() || out.flush().is_err())
        {
            break;
        }
    }
    0
}

#[derive(Default)]
pub struct Session {
    /// The tool catalogue, fetched from the app once.
    tools: Option<Value>,
}

/// One JSON-RPC message in, at most one out (notifications get none).
pub fn handle(line: &str, session: &mut Session) -> Option<String> {
    let message: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(err) => {
            return Some(error_response(
                Value::Null,
                -32700,
                &format!("parse error: {err}"),
            ));
        }
    };
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    let response = match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("");
            let version = PROTOCOL_VERSIONS
                .iter()
                .find(|v| **v == requested)
                .copied()
                .unwrap_or(PROTOCOL_VERSIONS[0]);
            Ok(server_info(version))
        }
        "server/discover" => Ok(server_info(PROTOCOL_VERSIONS[0])),
        "notifications/initialized" | "notifications/cancelled" => return None,
        "ping" => Ok(json!({})),
        "tools/list" => tools(session).map(|tools| json!({ "tools": tools })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if name.is_empty() {
                Err((-32602, "missing tool name".to_string()))
            } else {
                Ok(match call_bridge(name, arguments) {
                    Ok(result) => json!({
                        "content": [{ "type": "text", "text": pretty(&result) }],
                        "structuredContent": result,
                        "isError": false
                    }),
                    Err(err) => json!({
                        "content": [{ "type": "text", "text": format!("{}: {}", err.code, err.message) }],
                        "isError": true
                    }),
                })
            }
        }
        _ => {
            if id.is_none() {
                return None;
            }
            Err((-32601, format!("method not found: {method}")))
        }
    };
    let id = id.unwrap_or(Value::Null);
    Some(match response {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string(),
        Err((code, message)) => error_response(id, code, &message),
    })
}

fn server_info(version: &str) -> Value {
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    })
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn tools(session: &mut Session) -> Result<Value, (i64, String)> {
    if let Some(tools) = &session.tools {
        return Ok(tools.clone());
    }
    match call_bridge("__tools", json!({})) {
        Ok(tools) => {
            session.tools = Some(tools.clone());
            Ok(tools)
        }
        Err(err) => Err((-32603, format!("{}: {}", err.code, err.message))),
    }
}

/// One request to the app over the bridge, on a fresh connection.
fn call_bridge(tool: &str, args: Value) -> Result<Value, ToolError> {
    let Some(addr) = BridgeAddr::from_env() else {
        return Err(ToolError::new(
            "unavailable",
            "VAIS Commander is not running (VAIS_BRIDGE is not set)",
        ));
    };
    let stream: Box<dyn ReadWrite> = match &addr {
        #[cfg(unix)]
        BridgeAddr::Unix(path) => {
            Box::new(std::os::unix::net::UnixStream::connect(path).map_err(|e| {
                ToolError::new(
                    "unavailable",
                    format!("VAIS Commander is not reachable: {e}"),
                )
            })?)
        }
        #[cfg(not(unix))]
        BridgeAddr::Unix(_) => {
            return Err(ToolError::new(
                "unavailable",
                "unix sockets are not supported here",
            ));
        }
        BridgeAddr::Tcp { port, token } => {
            let mut stream = std::net::TcpStream::connect(("127.0.0.1", *port)).map_err(|e| {
                ToolError::new(
                    "unavailable",
                    format!("VAIS Commander is not reachable: {e}"),
                )
            })?;
            let auth = json!({ "id": 0, "auth": token }).to_string();
            writeln!(stream, "{auth}").map_err(|e| ToolError::new("internal", e.to_string()))?;
            let mut line = String::new();
            BufReader::new(&mut stream)
                .read_line(&mut line)
                .map_err(|e| ToolError::new("internal", e.to_string()))?;
            Box::new(stream)
        }
    };
    let mut reader = BufReader::new(stream);
    let request = json!({ "id": 1, "tool": tool, "args": args }).to_string();
    writeln!(reader.get_mut(), "{request}")
        .map_err(|e| ToolError::new("internal", e.to_string()))?;
    reader
        .get_mut()
        .flush()
        .map_err(|e| ToolError::new("internal", e.to_string()))?;
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| ToolError::new("internal", e.to_string()))?;
    let reply: Value = serde_json::from_str(line.trim())
        .map_err(|e| ToolError::new("internal", format!("bad reply from the app: {e}")))?;
    if let Some(error) = reply.get("error") {
        let code = match error.get("code").and_then(Value::as_str) {
            Some("invalid_args") => "invalid_args",
            Some("not_found") => "not_found",
            Some("rejected") => "rejected",
            Some("cancelled") => "cancelled",
            Some("busy") => "busy",
            Some("timeout") => "timeout",
            _ => "internal",
        };
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error")
            .to_string();
        return Err(ToolError::new(code, message));
    }
    Ok(reply.get("result").cloned().unwrap_or(Value::Null))
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

#[cfg(test)]
mod tests {
    use super::{Session, handle};
    use serde_json::Value;

    #[test]
    fn speaks_the_handshake() {
        let mut session = Session::default();
        let reply = handle(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#,
            &mut session,
        )
        .unwrap();
        let v: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(v["result"]["serverInfo"]["name"], "vais-commander");
        assert!(
            handle(
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                &mut session
            )
            .is_none()
        );
        let ping = handle(r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#, &mut session).unwrap();
        assert!(ping.contains(r#""result":{}"#));
        let unknown = handle(r#"{"jsonrpc":"2.0","id":3,"method":"nope"}"#, &mut session).unwrap();
        assert!(unknown.contains("-32601"));
        let old = handle(r#"{"jsonrpc":"2.0","id":4,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#, &mut session).unwrap();
        assert!(old.contains("2025-11-25"));
    }

    #[test]
    fn tool_calls_need_the_app() {
        // Without VAIS_BRIDGE the call fails as a tool error, not a crash.
        let mut session = Session::default();
        unsafe { std::env::remove_var("VAIS_BRIDGE") };
        let reply = handle(
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"get_state","arguments":{}}}"#,
            &mut session,
        )
        .unwrap();
        let v: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(v["result"]["isError"], true);
    }
}
