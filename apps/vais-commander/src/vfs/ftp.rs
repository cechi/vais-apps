//! Plain FTP (suppaftp, passive mode) as a [`Backend`]. One control
//! connection serves listings and metadata behind a mutex; every file
//! transfer logs in again on its own connection, because FTP carries one
//! data stream per session.

use std::io::{self, Read, Write};
use std::net::ToSocketAddrs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use suppaftp::list::{File, ListParser};
use suppaftp::types::FileType;
use suppaftp::{FtpError, FtpStream, ImplFtpStream, Mode, TlsStream, TransferStream};

use super::{Backend, ConnId, Location, Meta};
use crate::connections::{ConnectError, ConnectionConfig};
use crate::fs::{Entry, EntryKind};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

pub struct FtpBackend {
    conn: ConnId,
    config: ConnectionConfig,
    password: String,
    control: Mutex<FtpStream>,
}

/// Log in; an empty user means anonymous. Without a password the caller
/// is told to ask for one.
pub fn connect(
    config: &ConnectionConfig,
    password: Option<&str>,
) -> Result<Arc<dyn Backend>, ConnectError> {
    let anonymous = config.user.is_empty() || config.user == "anonymous";
    let password = match password {
        Some(p) => p.to_string(),
        None if anonymous => "anonymous@".to_string(),
        None => return Err(ConnectError::NeedsPassword),
    };
    let control = open(config, &password).map_err(|e| ConnectError::Other(e.to_string()))?;
    Ok(Arc::new(FtpBackend {
        conn: config.conn_id(),
        config: config.clone(),
        password,
        control: Mutex::new(control),
    }))
}

/// A fresh, logged-in control connection in binary passive mode.
fn open(config: &ConnectionConfig, password: &str) -> io::Result<FtpStream> {
    let addr = (config.host.as_str(), config.port)
        .to_socket_addrs()
        .map_err(|e| io::Error::other(format!("{}: {e}", config.host)))?
        .next()
        .ok_or_else(|| io::Error::other(t!("{host}: no address", host = config.host)))?;
    let mut stream = FtpStream::connect_timeout(addr, CONNECT_TIMEOUT).map_err(ftp_err)?;
    stream.set_mode(Mode::Passive);
    let user = if config.user.is_empty() {
        "anonymous"
    } else {
        &config.user
    };
    stream.login(user, password).map_err(ftp_err)?;
    stream.transfer_type(FileType::Binary).map_err(ftp_err)?;
    Ok(stream)
}

fn ftp_err(err: FtpError) -> io::Error {
    io::Error::other(err.to_string())
}

fn remote_path<'a>(conn: &ConnId, loc: &'a Location) -> io::Result<&'a str> {
    match loc {
        Location::Remote { conn: c, path } if c == conn => Ok(path.as_str()),
        _ => Err(io::Error::other(t!("not on this connection"))),
    }
}

/// Kind and size from a parsed listing line.
fn kind_of(file: &File) -> (EntryKind, u64) {
    if file.is_symlink() {
        (EntryKind::Symlink { to_dir: false }, file.size() as u64)
    } else if file.is_directory() {
        (EntryKind::Dir, 0)
    } else {
        (EntryKind::File, file.size() as u64)
    }
}

/// Owner/group/other rwx bits from the listing, as a Unix mode.
fn mode_of(file: &File) -> u32 {
    use suppaftp::list::PosixPexQuery;
    let mut mode = 0;
    for (who, shift) in [
        (PosixPexQuery::Owner, 6),
        (PosixPexQuery::Group, 3),
        (PosixPexQuery::Others, 0),
    ] {
        let bits = (file.can_read(who) as u32) << 2
            | (file.can_write(who) as u32) << 1
            | (file.can_execute(who) as u32);
        mode |= bits << shift;
    }
    if file.is_directory() {
        mode |= 0o040000;
    } else if file.is_symlink() {
        mode |= 0o120000;
    } else {
        mode |= 0o100000;
    }
    mode
}

impl FtpBackend {
    /// Parsed listing of a directory: MLSD when the server has it, else LIST.
    fn listing(&self, path: &str) -> io::Result<Vec<File>> {
        let mut control = self
            .control
            .lock()
            .map_err(|_| io::Error::other("ftp lock"))?;
        let parsed = match control.mlsd(Some(path)) {
            Ok(lines) => lines
                .iter()
                .filter_map(|line| ListParser::parse_mlsd(line).ok())
                .collect(),
            Err(_) => control
                .list(Some(path))
                .map_err(ftp_err)?
                .iter()
                .filter_map(|line| ListParser::parse_posix(line).ok())
                .collect::<Vec<_>>(),
        };
        Ok(parsed
            .into_iter()
            .filter(|f| f.name() != "." && f.name() != "..")
            .collect())
    }

    /// Metadata of one entry, found in its parent's listing.
    fn find(&self, loc: &Location) -> io::Result<File> {
        let name = loc
            .file_name()
            .ok_or_else(|| io::Error::other(t!("the root has no listing entry")))?;
        let parent = loc
            .parent()
            .ok_or_else(|| io::Error::other(t!("no parent directory")))?;
        let parent_path = remote_path(&self.conn, &parent)?;
        self.listing(parent_path)?
            .into_iter()
            .find(|f| f.name() == name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, t!("not found")))
    }

    fn remove_tree(&self, loc: &Location) -> io::Result<()> {
        let path = remote_path(&self.conn, loc)?;
        let file = self.find(loc)?;
        if file.is_directory() && !file.is_symlink() {
            for child in self.listing(path)? {
                self.remove_tree(&loc.join(child.name()))?;
            }
            let mut control = self
                .control
                .lock()
                .map_err(|_| io::Error::other("ftp lock"))?;
            control.rmdir(path).map_err(ftp_err)
        } else {
            let mut control = self
                .control
                .lock()
                .map_err(|_| io::Error::other("ftp lock"))?;
            control.rm(path).map_err(ftp_err)
        }
    }
}

impl Backend for FtpBackend {
    fn read_dir(&self, dir: &Location, show_hidden: bool) -> io::Result<Vec<Entry>> {
        let path = remote_path(&self.conn, dir)?;
        let mut entries = Vec::new();
        for file in self.listing(path)? {
            let name = file.name().to_string();
            if !show_hidden && name.starts_with('.') {
                continue;
            }
            let (kind, size) = kind_of(&file);
            entries.push(Entry {
                location: dir.join(&name),
                name,
                kind,
                size,
                modified: Some(file.modified()),
                created: None,
                mode: Some(mode_of(&file)),
                uid: file.uid(),
                is_parent: false,
                depth: 0,
            });
        }
        Ok(entries)
    }

    fn stat(&self, loc: &Location) -> io::Result<Meta> {
        if loc.parent().is_none() {
            return Ok(Meta {
                kind: EntryKind::Dir,
                size: 0,
                modified: None,
            });
        }
        let file = self.find(loc)?;
        let (kind, size) = kind_of(&file);
        Ok(Meta {
            kind,
            size,
            modified: Some(file.modified()),
        })
    }

    fn mkdir_all(&self, dir: &Location) -> io::Result<()> {
        let path = remote_path(&self.conn, dir)?;
        let mut acc = String::new();
        for segment in path.split('/').filter(|s| !s.is_empty()) {
            acc.push('/');
            acc.push_str(segment);
            let exists = self
                .find(&Location::remote(self.conn.0.clone(), acc.clone()))
                .is_ok();
            if exists {
                continue;
            }
            let mut control = self
                .control
                .lock()
                .map_err(|_| io::Error::other("ftp lock"))?;
            control.mkdir(&acc).map_err(ftp_err)?;
        }
        Ok(())
    }

    fn rename(&self, from: &Location, to: &Location) -> io::Result<()> {
        let (from, to) = (remote_path(&self.conn, from)?, remote_path(&self.conn, to)?);
        let mut control = self
            .control
            .lock()
            .map_err(|_| io::Error::other("ftp lock"))?;
        control.rename(from, to).map_err(ftp_err)
    }

    fn remove(&self, loc: &Location) -> io::Result<()> {
        self.remove_tree(loc)
    }

    fn open_read(&self, loc: &Location) -> io::Result<Box<dyn Read + Send>> {
        let path = remote_path(&self.conn, loc)?.to_string();
        let mut session = open(&self.config, &self.password)?;
        let stream = session.retr_as_stream(&path).map_err(ftp_err)?;
        Ok(Box::new(Transfer {
            stream,
            _session: session,
        }))
    }

    fn open_write(&self, loc: &Location) -> io::Result<Box<dyn Write + Send>> {
        let path = remote_path(&self.conn, loc)?.to_string();
        let mut session = open(&self.config, &self.password)?;
        let stream = session.put_with_stream(&path).map_err(ftp_err)?;
        Ok(Box::new(Transfer {
            stream,
            _session: session,
        }))
    }
}

/// A data stream with the session that owns it. Dropping the stream
/// finalises the transfer (reads the server's reply) before the session
/// goes away.
struct Transfer<T: TlsStream> {
    stream: TransferStream<T>,
    _session: ImplFtpStream<T>,
}

impl<T: TlsStream> Read for Transfer<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buf)
    }
}

impl<T: TlsStream> Write for Transfer<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}
