//! A reader for `~/.ssh/config`, enough to turn a host alias into what
//! libssh2 needs: `HostName`, `Port`, `User` and `IdentityFile`. `Include`
//! is followed; `Match` blocks, `ProxyJump` and friends are ignored.
//! Like OpenSSH, the first value found for an option wins, so specific
//! `Host` blocks should come before `Host *`.

use std::path::{Path, PathBuf};

/// What the config says about one alias (or any host, via `Host *`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SshHost {
    pub alias: String,
    pub host_name: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub identity_files: Vec<PathBuf>,
}

impl SshHost {
    /// The address to connect to: `HostName`, else the alias itself.
    pub fn address(&self) -> &str {
        self.host_name.as_deref().unwrap_or(&self.alias)
    }
}

/// One `Host` block: its patterns and options in file order.
#[derive(Debug, Clone)]
struct Block {
    patterns: Vec<String>,
    options: Vec<(String, String)>,
}

/// The user's ssh config, with includes resolved.
#[derive(Debug, Default)]
pub struct SshConfig {
    blocks: Vec<Block>,
}

impl SshConfig {
    pub fn load() -> Self {
        let Some(path) = default_path() else {
            return Self::default();
        };
        Self::from_file(&path)
    }

    pub fn from_file(path: &Path) -> Self {
        let mut config = Self::default();
        let mut seen = Vec::new();
        config.read(path, &mut seen);
        config
    }

    #[cfg(test)]
    pub fn from_str(text: &str) -> Self {
        let mut config = Self::default();
        config.parse(text, Path::new("/nonexistent/.ssh"), &mut Vec::new());
        config
    }

    fn read(&mut self, path: &Path, seen: &mut Vec<PathBuf>) {
        if seen.iter().any(|p| p == path) || seen.len() > 32 {
            return;
        }
        seen.push(path.to_path_buf());
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let dir = path.parent().unwrap_or(Path::new("/")).to_path_buf();
        self.parse(&text, &dir, seen);
    }

    fn parse(&mut self, text: &str, dir: &Path, seen: &mut Vec<PathBuf>) {
        // Options before the first `Host` apply everywhere.
        let mut current = Block {
            patterns: vec!["*".into()],
            options: Vec::new(),
        };
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = split_option(line) else {
                continue;
            };
            match key.to_ascii_lowercase().as_str() {
                "host" => {
                    self.blocks.push(std::mem::replace(
                        &mut current,
                        Block {
                            patterns: value.split_whitespace().map(str::to_string).collect(),
                            options: Vec::new(),
                        },
                    ));
                }
                "match" => {
                    // Conditions are not evaluated; keep the block only when
                    // it applies to every host.
                    let patterns = if value.trim().eq_ignore_ascii_case("all") {
                        vec!["*".into()]
                    } else {
                        Vec::new()
                    };
                    self.blocks.push(std::mem::replace(
                        &mut current,
                        Block {
                            patterns,
                            options: Vec::new(),
                        },
                    ));
                }
                "include" => {
                    for pattern in value.split_whitespace() {
                        for file in expand_include(pattern, dir) {
                            self.read(&file, seen);
                        }
                    }
                }
                _ => current.options.push((key.to_string(), value.to_string())),
            }
        }
        self.blocks.push(current);
    }

    /// Aliases a user would pick from: explicit `Host` names without wildcards.
    pub fn aliases(&self) -> Vec<SshHost> {
        let mut aliases: Vec<String> = Vec::new();
        for block in &self.blocks {
            for pattern in &block.patterns {
                if !pattern.contains(['*', '?', '!']) && !aliases.contains(pattern) {
                    aliases.push(pattern.clone());
                }
            }
        }
        aliases.into_iter().map(|a| self.resolve(&a)).collect()
    }

    /// Everything the config says about `alias` (first value wins).
    pub fn resolve(&self, alias: &str) -> SshHost {
        let mut host = SshHost {
            alias: alias.to_string(),
            ..Default::default()
        };
        for block in &self.blocks {
            if !block_matches(block, alias) {
                continue;
            }
            for (key, value) in &block.options {
                match key.to_ascii_lowercase().as_str() {
                    "hostname" if host.host_name.is_none() => {
                        host.host_name = Some(value.replace("%h", alias));
                    }
                    "port" if host.port.is_none() => host.port = value.parse().ok(),
                    "user" if host.user.is_none() => host.user = Some(value.clone()),
                    // Every identity file is kept, in order.
                    "identityfile" => {
                        let path = expand_tilde(value.trim_matches('"'));
                        if !host.identity_files.contains(&path) {
                            host.identity_files.push(path);
                        }
                    }
                    _ => {}
                }
            }
        }
        host
    }

    /// The config knows this alias (an explicit `Host` entry).
    #[cfg(test)]
    pub fn has_alias(&self, alias: &str) -> bool {
        self.blocks
            .iter()
            .any(|b| b.patterns.iter().any(|p| p == alias))
    }
}

fn default_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(Path::new(&home).join(".ssh").join("config"))
}

/// `Key value`, `Key=value` or `Key = value`.
fn split_option(line: &str) -> Option<(&str, &str)> {
    let (key, rest) = line
        .split_once(|c: char| c == '=' || c.is_whitespace())
        .map(|(k, v)| (k.trim(), v.trim().trim_start_matches('=').trim()))?;
    (!key.is_empty() && !rest.is_empty()).then_some((key, rest))
}

/// Negated patterns exclude; otherwise any positive pattern has to match.
fn block_matches(block: &Block, alias: &str) -> bool {
    let mut matched = false;
    for pattern in &block.patterns {
        if let Some(negated) = pattern.strip_prefix('!') {
            if glob_match(negated, alias) {
                return false;
            }
        } else if glob_match(pattern, alias) {
            matched = true;
        }
    }
    matched
}

/// `*` and `?` as in ssh_config.
fn glob_match(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    fn go(p: &[char], t: &[char]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], t) || (!t.is_empty() && go(p, &t[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => go(&p[1..], &t[1..]),
            _ => false,
        }
    }
    go(&p, &t)
}

fn expand_tilde(value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        return Path::new(&home).join(rest);
    }
    PathBuf::from(value)
}

/// Files an `Include` names: relative to `~/.ssh`, with a simple `*` glob
/// in the file name.
fn expand_include(pattern: &str, dir: &Path) -> Vec<PathBuf> {
    let pattern = expand_tilde(pattern);
    let pattern = if pattern.is_absolute() {
        pattern
    } else {
        dir.join(pattern)
    };
    let Some(name) = pattern
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
    else {
        return Vec::new();
    };
    if !name.contains(['*', '?']) {
        return vec![pattern];
    }
    let parent = pattern.parent().unwrap_or(dir);
    let mut files: Vec<PathBuf> = std::fs::read_dir(parent)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .is_some_and(|n| glob_match(&name, &n.to_string_lossy()))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::{SshConfig, glob_match};
    use std::path::PathBuf;

    const SAMPLE: &str = "
# comment
Host nas
    HostName 192.168.1.5
    Port 2222
    User admin
    IdentityFile ~/.ssh/nas_key

Host web*
    User deploy

Host *.example.com !skip.example.com
    Port 2200

Host *
    User fallback
    IdentityFile ~/.ssh/id_ed25519
";

    #[test]
    fn resolves_first_value_wins() {
        let cfg = SshConfig::from_str(SAMPLE);
        let nas = cfg.resolve("nas");
        assert_eq!(nas.address(), "192.168.1.5");
        assert_eq!(nas.port, Some(2222));
        assert_eq!(nas.user.as_deref(), Some("admin"));
        assert_eq!(nas.identity_files.len(), 2);
        assert!(nas.identity_files[0].ends_with("nas_key"));

        let web = cfg.resolve("web1");
        assert_eq!(web.address(), "web1");
        assert_eq!(web.user.as_deref(), Some("deploy"));
        assert_eq!(web.port, None);

        assert_eq!(cfg.resolve("a.example.com").port, Some(2200));
        assert_eq!(cfg.resolve("skip.example.com").port, None);
        assert_eq!(cfg.resolve("other").user.as_deref(), Some("fallback"));
    }

    #[test]
    fn lists_explicit_aliases_only() {
        let cfg = SshConfig::from_str(SAMPLE);
        let aliases: Vec<String> = cfg.aliases().into_iter().map(|h| h.alias).collect();
        assert_eq!(aliases, ["nas"]);
        assert!(cfg.has_alias("nas"));
        assert!(!cfg.has_alias("web1"));
    }

    #[test]
    fn globs() {
        assert!(glob_match("*.example.com", "a.example.com"));
        assert!(glob_match("web?", "web1"));
        assert!(!glob_match("web?", "web10"));
        assert!(glob_match("NAS", "nas"));
        assert_eq!(PathBuf::from("x"), PathBuf::from("x"));
    }
}
