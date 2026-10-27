//! SFTP over libssh2 (`ssh2` crate) as a [`Backend`]. The crate serialises
//! all session calls behind its own lock, so one `Sftp` handle is shared by
//! every panel and operation on the connection.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ssh2::{FileStat, RenameFlags, Session, Sftp};

use super::{Backend, ConnId, Location, Meta};
use crate::connections::{AuthMethod, ConnectError, ConnectionConfig, Protocol};
use crate::fs::{Entry, EntryKind};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// libssh2 read/write timeout for a stalled server.
const IO_TIMEOUT_MS: u32 = 60_000;

pub struct SftpBackend {
    conn: ConnId,
    /// Kept alive for the sftp channel.
    _session: Session,
    sftp: Sftp,
}

/// Open a connection and authenticate: the agent, then key files, then a
/// password when there is one. Without a password and nothing else working
/// the caller is told to ask for one.
pub fn connect(
    config: &ConnectionConfig,
    password: Option<&str>,
) -> Result<Arc<dyn Backend>, ConnectError> {
    let other = |e: String| ConnectError::Other(e);
    // `~/.ssh/config` fills in what the connection leaves at its defaults:
    // the address behind an alias, its port, user and identity files.
    let ssh = crate::ssh_config::SshConfig::load().resolve(&config.host);
    let address = ssh.address().to_string();
    let port = match ssh.port {
        Some(port) if config.port == Protocol::Sftp.default_port() => port,
        _ => config.port,
    };
    let user = if config.user.is_empty() {
        ssh.user.clone().ok_or_else(|| {
            other(t!(
                "{host}: no user (set one, or in ~/.ssh/config)",
                host = config.host
            ))
        })?
    } else {
        config.user.clone()
    };
    let addr = (address.as_str(), port)
        .to_socket_addrs()
        .map_err(|e| other(format!("{address}: {e}")))?
        .next()
        .ok_or_else(|| other(t!("{host}: no address", host = address)))?;
    let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .map_err(|e| other(format!("{}: {e}", config.host)))?;
    let mut session = Session::new().map_err(|e| other(e.message().to_string()))?;
    session.set_tcp_stream(stream);
    session.set_timeout(IO_TIMEOUT_MS);
    session
        .handshake()
        .map_err(|e| other(t!("handshake: {error}", error = e.message())))?;
    // The host key is not checked yet; log it so a change can at least be seen.
    if let Some((key, _)) = session.host_key() {
        log::info!("{}: host key {} bytes", config.host, key.len());
    }

    let user = user.as_str();
    match config.auth {
        AuthMethod::Password => {
            let Some(password) = password else {
                return Err(ConnectError::NeedsPassword);
            };
            session
                .userauth_password(user, password)
                .map_err(|e| other(t!("password: {error}", error = e.message())))?;
        }
        AuthMethod::Key => {
            let Some(key) = &config.key else {
                return Err(other(t!("no key file set").into()));
            };
            session
                .userauth_pubkey_file(user, None, key, password)
                .map_err(|e| other(format!("{}: {}", key.display(), e.message())))?;
        }
        AuthMethod::Auto => {
            let _ = session.userauth_agent(user);
            if !session.authenticated() {
                for key in default_keys(config, &ssh.identity_files) {
                    if session.userauth_pubkey_file(user, None, &key, None).is_ok() {
                        break;
                    }
                }
            }
            if !session.authenticated() {
                let Some(password) = password else {
                    return Err(ConnectError::NeedsPassword);
                };
                session
                    .userauth_password(user, password)
                    .map_err(|e| other(t!("password: {error}", error = e.message())))?;
            }
        }
    }
    if !session.authenticated() {
        return Err(other(t!("authentication failed").into()));
    }
    let sftp = session
        .sftp()
        .map_err(|e| other(format!("sftp: {}", e.message())))?;
    Ok(Arc::new(SftpBackend {
        conn: config.conn_id(),
        _session: session,
        sftp,
    }))
}

/// The configured key and the usual files in `~/.ssh`, those that exist.
fn default_keys(config: &ConnectionConfig, from_ssh_config: &[PathBuf]) -> Vec<PathBuf> {
    let mut keys = Vec::new();
    if let Some(key) = &config.key {
        keys.push(key.clone());
    }
    keys.extend(from_ssh_config.iter().cloned());
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let ssh = Path::new(&home).join(".ssh");
        for name in ["id_ed25519", "id_ecdsa", "id_rsa"] {
            keys.push(ssh.join(name));
        }
    }
    keys.into_iter().filter(|k| k.is_file()).collect()
}

fn remote_path<'a>(conn: &ConnId, loc: &'a Location) -> io::Result<&'a Path> {
    match loc {
        Location::Remote { conn: c, path } if c == conn => Ok(Path::new(path)),
        _ => Err(io::Error::other(t!("not on this connection"))),
    }
}

fn io_err(err: ssh2::Error) -> io::Error {
    io::Error::other(err.message().to_string())
}

fn time_of(secs: Option<u64>) -> Option<SystemTime> {
    secs.map(|s| UNIX_EPOCH + Duration::from_secs(s))
}

impl SftpBackend {
    /// Kind and size of an entry from its lstat; symlinks are resolved once
    /// more to know whether they point at a directory.
    fn kind_of(&self, path: &Path, stat: &FileStat) -> (EntryKind, u64) {
        let size = stat.size.unwrap_or(0);
        let file_type = stat.file_type();
        if file_type.is_symlink() {
            let to_dir = self.sftp.stat(path).map(|s| s.is_dir()).unwrap_or(false);
            (EntryKind::Symlink { to_dir }, size)
        } else if file_type.is_dir() {
            (EntryKind::Dir, 0)
        } else if file_type.is_file() {
            (EntryKind::File, size)
        } else {
            (EntryKind::Other, size)
        }
    }

    fn remove_tree(&self, path: &Path) -> io::Result<()> {
        let stat = self.sftp.lstat(path).map_err(io_err)?;
        if stat.is_dir() && !stat.file_type().is_symlink() {
            for (child, _) in self.sftp.readdir(path).map_err(io_err)? {
                self.remove_tree(&child)?;
            }
            self.sftp.rmdir(path).map_err(io_err)
        } else {
            self.sftp.unlink(path).map_err(io_err)
        }
    }
}

impl Backend for SftpBackend {
    fn read_dir(&self, dir: &Location, show_hidden: bool) -> io::Result<Vec<Entry>> {
        let path = remote_path(&self.conn, dir)?;
        let listing = self.sftp.readdir(path).map_err(io_err)?;
        let mut entries = Vec::with_capacity(listing.len());
        for (child, stat) in listing {
            let name = child
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            let (kind, size) = self.kind_of(&child, &stat);
            entries.push(Entry {
                name,
                location: dir.join(
                    &child
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                ),
                kind,
                size,
                modified: time_of(stat.mtime),
                created: None,
                mode: stat.perm,
                uid: stat.uid,
                is_parent: false,
                depth: 0,
            });
        }
        Ok(entries)
    }

    fn stat(&self, loc: &Location) -> io::Result<Meta> {
        let path = remote_path(&self.conn, loc)?;
        let stat = self.sftp.lstat(path).map_err(io_err)?;
        let (kind, size) = self.kind_of(path, &stat);
        Ok(Meta {
            kind,
            size,
            modified: time_of(stat.mtime),
        })
    }

    fn mkdir_all(&self, dir: &Location) -> io::Result<()> {
        let path = remote_path(&self.conn, dir)?;
        let mut acc = PathBuf::from("/");
        for component in path.components().skip(1) {
            acc.push(component);
            if self.sftp.stat(&acc).is_ok() {
                continue;
            }
            self.sftp.mkdir(&acc, 0o755).map_err(io_err)?;
        }
        Ok(())
    }

    fn rename(&self, from: &Location, to: &Location) -> io::Result<()> {
        let (from, to) = (remote_path(&self.conn, from)?, remote_path(&self.conn, to)?);
        let flags = RenameFlags::OVERWRITE | RenameFlags::ATOMIC | RenameFlags::NATIVE;
        match self.sftp.rename(from, to, Some(flags)) {
            Ok(()) => Ok(()),
            Err(err) => {
                // SFTP v3 servers (OpenSSH) refuse to rename over an existing
                // file whatever the flags say: clear the target, then retry.
                if let Ok(existing) = self.sftp.lstat(to)
                    && !existing.is_dir()
                {
                    self.sftp.unlink(to).map_err(io_err)?;
                    return self.sftp.rename(from, to, Some(flags)).map_err(io_err);
                }
                Err(io_err(err))
            }
        }
    }

    fn remove(&self, loc: &Location) -> io::Result<()> {
        self.remove_tree(remote_path(&self.conn, loc)?)
    }

    fn open_read(&self, loc: &Location) -> io::Result<Box<dyn Read + Send>> {
        let file = self
            .sftp
            .open(remote_path(&self.conn, loc)?)
            .map_err(io_err)?;
        Ok(Box::new(file))
    }

    fn open_write(&self, loc: &Location) -> io::Result<Box<dyn Write + Send>> {
        let file = self
            .sftp
            .create(remote_path(&self.conn, loc)?)
            .map_err(io_err)?;
        Ok(Box::new(file))
    }
}
