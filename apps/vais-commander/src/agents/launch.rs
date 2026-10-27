//! How each agent is started so that it finds our MCP server: the command
//! line (or environment, or a settings file) that registers
//! `vais-commander --mcp` for that one session, plus the directory it runs in.

use std::path::PathBuf;
use std::sync::OnceLock;

use serde_json::json;

use super::detect::{Agent, AgentKind};
use crate::terminal::Launch;

/// The MCP server name the agents see (tools become `mcp__vais__<tool>`,
/// `vais_<tool>` …).
pub const SERVER_NAME: &str = "vais";

/// The launch for `agent` in `cwd`; `bridge_env` is the app's bridge
/// address (empty when the bridge is not running, then no MCP is wired).
pub fn plan(agent: &Agent, cwd: Option<PathBuf>, bridge_env: &[(String, String)]) -> Launch {
    let cwd = cwd.unwrap_or_else(crate::config::home_dir);
    let program = agent.path.to_string_lossy().into_owned();
    let mut launch = Launch {
        program,
        args: Vec::new(),
        cwd: Some(cwd.clone()),
        env: bridge_env.to_vec(),
    };
    let exe = exe();
    let with_mcp = !bridge_env.is_empty() && agent.kind.supports_mcp();
    log::debug!("launch {agent:?}: exe {exe:?}, bridge {:?}", bridge_env);
    match agent.kind {
        AgentKind::Shell => {
            if cfg!(target_os = "macos") {
                launch.args.push("-l".into());
            }
        }
        AgentKind::Claude => {
            if let (true, Some(exe)) = (with_mcp, &exe) {
                let config = json!({
                    "mcpServers": {
                        SERVER_NAME: { "type": "stdio", "command": exe, "args": ["--mcp"] }
                    }
                });
                launch.args.extend([
                    "--mcp-config".to_string(),
                    config.to_string(),
                    "--allowedTools".to_string(),
                    format!("mcp__{SERVER_NAME}__*"),
                ]);
            }
        }
        AgentKind::OpenCode => {
            if let (true, Some(exe)) = (with_mcp, &exe) {
                let config = json!({
                    "mcp": {
                        SERVER_NAME: { "type": "local", "command": [exe, "--mcp"], "enabled": true }
                    }
                });
                launch
                    .env
                    .push(("OPENCODE_CONFIG_CONTENT".into(), config.to_string()));
            }
            launch.args.push(cwd.to_string_lossy().into_owned());
        }
        AgentKind::Codex => {
            launch
                .args
                .extend(["-C".to_string(), cwd.to_string_lossy().into_owned()]);
            if let (true, Some(exe)) = (with_mcp, &exe) {
                // `-c key=value` takes TOML values (not verified against a
                // local install: codex is not available here).
                launch.args.extend([
                    "-c".to_string(),
                    format!("mcp_servers.{SERVER_NAME}.command={}", toml_string(exe)),
                    "-c".to_string(),
                    format!("mcp_servers.{SERVER_NAME}.args=[\"--mcp\"]"),
                ]);
            }
        }
        AgentKind::Gemini => {
            if let (true, Some(exe)) = (with_mcp, &exe)
                && let Some(settings) = gemini_settings(exe)
            {
                // No per-session flag: a settings file of our own, pointed
                // at through the environment.
                launch.env.push((
                    "GEMINI_CLI_SYSTEM_SETTINGS_PATH".into(),
                    settings.to_string_lossy().into_owned(),
                ));
            }
        }
    }
    launch
}

/// This program's path, for the agents to start `--mcp` from.
static EXE: OnceLock<Option<String>> = OnceLock::new();

/// Take the program's path now, at startup, while the file is surely there.
pub fn remember_exe() {
    exe();
}

fn exe() -> Option<&'static str> {
    EXE.get_or_init(|| resolve_exe(std::env::current_exe().ok()?))
        .as_deref()
}

/// The canonical path of `exe`. A binary replaced while the app runs (an
/// update, a rebuild) is reported by Linux as "<path> (deleted)"; the new
/// file at the same path serves, as the MCP server only talks to the bridge.
fn resolve_exe(exe: PathBuf) -> Option<String> {
    let exe = match exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
        Some(path) => PathBuf::from(path),
        None => exe,
    };
    exe.canonicalize()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

fn toml_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A settings file registering our server for Gemini CLI, in the cache.
fn gemini_settings(exe: &str) -> Option<PathBuf> {
    let dir = directories::ProjectDirs::from("", "", "vais-commander")?
        .cache_dir()
        .join("gemini");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("settings-{}.json", std::process::id()));
    let settings = json!({
        "mcpServers": {
            SERVER_NAME: { "command": exe, "args": ["--mcp"], "trust": true }
        }
    });
    std::fs::write(&path, serde_json::to_string_pretty(&settings).ok()?).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::{plan, resolve_exe};
    use crate::agents::detect::{Agent, AgentKind};
    use std::path::PathBuf;

    fn agent(kind: AgentKind) -> Agent {
        Agent {
            kind,
            path: PathBuf::from("/usr/bin/x"),
        }
    }

    #[test]
    fn claude_gets_the_server_on_its_command_line() {
        let env = vec![("VAIS_BRIDGE".to_string(), "unix:/tmp/b.sock".to_string())];
        let launch = plan(&agent(AgentKind::Claude), Some(PathBuf::from("/tmp")), &env);
        assert_eq!(launch.args[0], "--mcp-config");
        assert!(launch.args[1].contains("\"--mcp\""));
        assert_eq!(launch.args[3], "mcp__vais__*");
        assert_eq!(launch.cwd.as_deref(), Some(std::path::Path::new("/tmp")));
        assert!(launch.env.iter().any(|(k, _)| k == "VAIS_BRIDGE"));
    }

    #[test]
    fn a_replaced_binary_resolves_to_the_new_file() {
        let file = std::env::temp_dir().join(format!("vais-exe-{}", std::process::id()));
        std::fs::write(&file, b"").unwrap();
        let deleted = PathBuf::from(format!("{} (deleted)", file.display()));
        let expected = file.canonicalize().unwrap().to_string_lossy().into_owned();
        assert_eq!(resolve_exe(deleted).as_deref(), Some(expected.as_str()));
        assert_eq!(
            resolve_exe(file.clone()).as_deref(),
            Some(expected.as_str())
        );
        std::fs::remove_file(&file).unwrap();
        assert_eq!(resolve_exe(file), None);
    }

    #[test]
    fn without_a_bridge_nothing_is_registered() {
        let launch = plan(&agent(AgentKind::Claude), None, &[]);
        assert!(launch.args.is_empty());
        let shell = plan(&agent(AgentKind::Shell), None, &[]);
        assert!(shell.env.is_empty());
    }

    #[test]
    fn opencode_takes_the_server_through_the_environment() {
        let env = vec![("VAIS_BRIDGE".to_string(), "unix:/tmp/b.sock".to_string())];
        let launch = plan(
            &agent(AgentKind::OpenCode),
            Some(PathBuf::from("/tmp")),
            &env,
        );
        let content = launch
            .env
            .iter()
            .find(|(k, _)| k == "OPENCODE_CONFIG_CONTENT")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert!(content.contains("\"type\":\"local\""));
        assert_eq!(launch.args, vec!["/tmp".to_string()]);
    }
}
