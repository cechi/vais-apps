//! Which coding-agent CLIs are installed: a scan of `PATH` for the known
//! programs, plus the user's shell, which is always offered.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Claude,
    OpenCode,
    Codex,
    Gemini,
    Shell,
}

impl AgentKind {
    pub const ALL: [AgentKind; 5] = [
        AgentKind::Claude,
        AgentKind::OpenCode,
        AgentKind::Codex,
        AgentKind::Gemini,
        AgentKind::Shell,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude Code",
            AgentKind::OpenCode => "OpenCode",
            AgentKind::Codex => "Codex",
            AgentKind::Gemini => "Gemini CLI",
            AgentKind::Shell => t!("Shell"),
        }
    }

    /// The executable looked up in `PATH`; the shell comes from `$SHELL`.
    pub fn program(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::OpenCode => "opencode",
            AgentKind::Codex => "codex",
            AgentKind::Gemini => "gemini",
            AgentKind::Shell => "",
        }
    }

    /// Whether the CLI can be given our MCP server for a session.
    pub fn supports_mcp(self) -> bool {
        !matches!(self, AgentKind::Shell)
    }
}

/// One installed agent: the kind and where its executable is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub kind: AgentKind,
    pub path: PathBuf,
}

/// The agents found on this machine, in the order of `AgentKind::ALL`; the
/// shell is always last.
pub fn available() -> Vec<Agent> {
    AgentKind::ALL
        .iter()
        .filter_map(|&kind| {
            let path = if kind == AgentKind::Shell {
                Some(default_shell())
            } else {
                find_in_path(kind.program())
            };
            path.map(|path| Agent { kind, path })
        })
        .collect()
}

/// The user's shell: `$SHELL`, else `/bin/sh` (Windows: `%COMSPEC%`, else `cmd.exe`).
pub fn default_shell() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("COMSPEC")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("cmd.exe"))
    } else {
        std::env::var_os("SHELL")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/bin/sh"))
    }
}

/// The first executable called `program` on `PATH` (Windows also tries the
/// `PATHEXT` extensions).
pub fn find_in_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let candidates: Vec<String> = if cfg!(windows) {
        let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into());
        std::iter::once(program.to_string())
            .chain(
                exts.split(';')
                    .filter(|e| !e.is_empty())
                    .map(|e| format!("{program}{}", e.to_lowercase())),
            )
            .collect()
    } else {
        vec![program.to_string()]
    };
    std::env::split_paths(&path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| candidates.iter().map(move |c| dir.join(c)))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::{AgentKind, available, find_in_path};

    #[test]
    fn shell_is_always_offered_last() {
        let agents = available();
        assert_eq!(agents.last().map(|a| a.kind), Some(AgentKind::Shell));
        assert!(
            agents
                .iter()
                .all(|a| a.kind != AgentKind::Shell || a.path.as_os_str().len() > 0)
        );
    }

    #[test]
    fn finds_common_programs() {
        assert!(find_in_path("sh").is_some() || cfg!(windows));
        assert!(find_in_path("definitely-not-a-program-xyz").is_none());
    }
}
