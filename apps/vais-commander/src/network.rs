//! Windows network paths: `\\server` lists the server's shares, a share's
//! parent is its server, and the path bar shows the server and the share as
//! crumbs of their own. The path helpers are plain string work, so they are
//! tested everywhere; only Windows treats such paths as network paths.

use std::path::{Path, PathBuf};

/// `\\server\share\a\b` (either slash) as its server and the segments after
/// it, the share first. Device and verbatim paths (`\\?\`, `\\.\`) are not
/// network paths.
fn split(path: &Path) -> Option<(&str, Vec<&str>)> {
    let text = path.to_str()?;
    let rest = text
        .strip_prefix(r"\\")
        .or_else(|| text.strip_prefix("//"))?;
    let mut parts = rest.split(['\\', '/']).filter(|s| !s.is_empty());
    let server = parts.next().filter(|s| !matches!(*s, "?" | "."))?;
    Some((server, parts.collect()))
}

/// The path of `server` and `segments` in the form the rest of the app
/// compares against: backslashes, and a share's root with a trailing one (as
/// `Path::parent` of a directory in the share gives it).
fn compose(server: &str, segments: &[&str]) -> PathBuf {
    let mut text = format!(r"\\{server}");
    for segment in segments {
        text.push('\\');
        text.push_str(segment);
    }
    if segments.len() == 1 {
        text.push('\\');
    }
    PathBuf::from(text)
}

/// A typed network path in that form: `//nas/data` → `\\nas\data\`.
pub fn normalize(text: &str) -> Option<PathBuf> {
    let (server, segments) = split(Path::new(text))?;
    Some(compose(server, &segments))
}

/// The server a network path is on.
pub fn server_of(path: &Path) -> Option<String> {
    split(path).map(|(server, _)| server.to_string())
}

/// `\\server` alone: its listing is the server's shares.
pub fn server_path(server: &str) -> PathBuf {
    compose(server, &[])
}

pub fn is_server(path: &Path) -> bool {
    split(path).is_some_and(|(_, segments)| segments.is_empty())
}

/// The directory above a network path: a share's root is below its server,
/// a server below the roots listing.
pub fn parent(path: &Path) -> Option<PathBuf> {
    let (server, segments) = split(path)?;
    Some(match segments.split_last() {
        Some((_, above)) => compose(server, above),
        None => crate::platform::roots_path(),
    })
}

/// The path bar's crumbs of a network path: the server, the share, then each
/// directory, each with where it leads.
pub fn crumbs(path: &Path) -> Option<Vec<(String, PathBuf)>> {
    let (server, segments) = split(path)?;
    let mut crumbs = vec![(format!(r"\\{server}"), server_path(server))];
    for (ix, segment) in segments.iter().enumerate() {
        crumbs.push((segment.to_string(), compose(server, &segments[..=ix])));
    }
    Some(crumbs)
}

/// What to log in to for a network path: its share, or for the server
/// itself (its share list) the server's `IPC$`.
pub fn login_target(path: &Path) -> Option<String> {
    let (server, segments) = split(path)?;
    Some(match segments.first() {
        Some(share) => format!(r"\\{server}\{share}"),
        None => format!(r"\\{server}\IPC$"),
    })
}

/// Whether reading a network path failed for want of other credentials
/// (access denied, a wrong password, no session).
pub fn is_login_error(err: &std::io::Error) -> bool {
    matches!(err.raw_os_error(), Some(5 | 86 | 1244 | 1326))
}

/// The user cancelled Windows' credential dialog (`ERROR_CANCELLED`).
#[cfg(windows)]
pub fn is_cancelled(err: &std::io::Error) -> bool {
    err.raw_os_error() == Some(1223)
}

/// Connect to `target` with credentials the user enters in Windows' own
/// dialog, which can also remember them in the Credential Manager, as
/// Explorer's does. The dialog runs a modal loop, so this must not be called
/// while the app is borrowed (inside an update).
#[cfg(windows)]
pub fn log_in(hwnd: isize, target: &str) -> std::io::Result<()> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::NetworkManagement::WNet::{
        CONNECT_INTERACTIVE, CONNECT_PROMPT, NETRESOURCEW, RESOURCETYPE_ANY, WNetAddConnection3W,
    };
    use windows::core::{PCWSTR, PWSTR};

    let mut remote: Vec<u16> = target.encode_utf16().chain(Some(0)).collect();
    let resource = NETRESOURCEW {
        dwType: RESOURCETYPE_ANY,
        lpRemoteName: PWSTR(remote.as_mut_ptr()),
        ..Default::default()
    };
    // SAFETY: the resource and its NUL-terminated name outlive the call; no
    // user name or password, so the dialog asks for them.
    let status = unsafe {
        WNetAddConnection3W(
            HWND(hwnd as *mut _),
            &resource,
            PCWSTR::null(),
            PCWSTR::null(),
            CONNECT_INTERACTIVE | CONNECT_PROMPT,
        )
    };
    if status.is_ok() {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(status.0 as i32))
    }
}

/// The disk shares of `server`; the administrative ones (`C$`, `ADMIN$`)
/// and other names ending in `$` only with hidden files shown.
#[cfg(windows)]
pub fn read_server(server: &str, show_hidden: bool) -> std::io::Result<Vec<crate::fs::Entry>> {
    use windows::Win32::NetworkManagement::NetManagement::NetApiBufferFree;
    use windows::Win32::Storage::FileSystem::{
        NetShareEnum, SHARE_INFO_1, STYPE_DISKTREE, STYPE_SPECIAL,
    };
    use windows::core::PCWSTR;

    use crate::fs::{Entry, EntryKind};
    use crate::vfs::Location;

    let wide: Vec<u16> = format!(r"\\{server}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut buffer: *mut u8 = std::ptr::null_mut();
    let (mut read, mut total) = (0u32, 0u32);
    // SAFETY: a NUL-terminated wide string and valid out-pointers; the
    // buffer the call allocates is freed below.
    let status = unsafe {
        NetShareEnum(
            PCWSTR(wide.as_ptr()),
            1,
            &mut buffer,
            u32::MAX,
            &mut read,
            &mut total,
            None,
        )
    };
    let free = |buffer: *mut u8| {
        if !buffer.is_null() {
            // SAFETY: allocated by NetShareEnum and not used afterwards.
            unsafe { NetApiBufferFree(Some(buffer as *const _)) };
        }
    };
    if status != 0 {
        free(buffer);
        return Err(std::io::Error::from_raw_os_error(status as i32));
    }
    // SAFETY: on success the buffer holds `read` SHARE_INFO_1 records.
    let infos = unsafe { std::slice::from_raw_parts(buffer as *const SHARE_INFO_1, read as usize) };
    let mut entries = Vec::new();
    for info in infos {
        // Printers, devices and IPC$ are not folders.
        if info.shi1_type.0 & 0xff != STYPE_DISKTREE.0 {
            continue;
        }
        // SAFETY: a NUL-terminated string inside the buffer.
        let Ok(name) = (unsafe { info.shi1_netname.to_string() }) else {
            continue;
        };
        let hidden = info.shi1_type.0 & STYPE_SPECIAL.0 != 0 || name.ends_with('$');
        if name.is_empty() || (hidden && !show_hidden) {
            continue;
        }
        entries.push(Entry {
            location: Location::Local(compose(server, &[&name])),
            name,
            kind: EntryKind::Dir,
            size: 0,
            modified: None,
            created: None,
            mode: None,
            uid: None,
            is_parent: false,
            depth: 0,
        });
    }
    free(buffer);
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_paths_take_one_form() {
        assert_eq!(normalize(r"\\nas"), Some(PathBuf::from(r"\\nas")));
        assert_eq!(normalize(r"\\nas\"), Some(PathBuf::from(r"\\nas")));
        assert_eq!(normalize("//nas/data"), Some(PathBuf::from(r"\\nas\data\")));
        assert_eq!(
            normalize(r"\\10.0.0.5\data\a/b\"),
            Some(PathBuf::from(r"\\10.0.0.5\data\a\b"))
        );
        assert_eq!(normalize(r"\\?\C:\x"), None);
        assert_eq!(normalize(r"\\.\pipe\x"), None);
        assert_eq!(normalize(r"C:\x"), None);
        assert_eq!(normalize("/home"), None);
    }

    #[test]
    fn a_share_is_below_its_server() {
        let up = |p: &str| parent(Path::new(p));
        assert_eq!(up(r"\\nas\data\a\b"), Some(PathBuf::from(r"\\nas\data\a")));
        assert_eq!(up(r"\\nas\data\a"), Some(PathBuf::from(r"\\nas\data\")));
        assert_eq!(up(r"\\nas\data\"), Some(PathBuf::from(r"\\nas")));
        assert_eq!(up(r"\\nas"), Some(crate::platform::roots_path()));
        assert_eq!(up(r"C:\x"), None);
        assert!(is_server(Path::new(r"\\nas")));
        assert!(!is_server(Path::new(r"\\nas\data")));
        assert_eq!(
            server_of(Path::new(r"\\nas\data\a")).as_deref(),
            Some("nas")
        );
        let target = |p: &str| login_target(Path::new(p));
        assert_eq!(target(r"\\nas\data\a").as_deref(), Some(r"\\nas\data"));
        assert_eq!(target(r"\\nas").as_deref(), Some(r"\\nas\IPC$"));
        assert_eq!(target(r"C:\x"), None);
    }

    #[test]
    fn the_server_and_the_share_are_crumbs() {
        let crumbs = crumbs(Path::new(r"\\nas\data\a")).unwrap();
        let expected = [
            (r"\\nas", r"\\nas"),
            ("data", r"\\nas\data\"),
            ("a", r"\\nas\data\a"),
        ];
        assert_eq!(crumbs.len(), expected.len());
        for ((label, path), (want_label, want_path)) in crumbs.iter().zip(expected) {
            assert_eq!(label, want_label);
            assert_eq!(path, &PathBuf::from(want_path));
        }
    }
}
