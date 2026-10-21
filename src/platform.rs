//! The few places where the file system differs between operating systems.
//!
//! Linux is the only tested platform so far; the macOS and Windows branches
//! follow the platform conventions but are untested.

use std::path::{Path, PathBuf};

/// Whether an entry is hidden by the platform's convention.
#[cfg(not(windows))]
pub fn is_hidden(name: &str, _meta: Option<&std::fs::Metadata>) -> bool {
    name.starts_with('.')
}

#[cfg(windows)]
pub fn is_hidden(name: &str, meta: Option<&std::fs::Metadata>) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    name.starts_with('.') || meta.is_some_and(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
}

/// Top-level roots: `/` on Unix, the mounted drive letters on Windows.
#[cfg(not(windows))]
pub fn roots() -> Vec<PathBuf> {
    vec![PathBuf::from("/")]
}

#[cfg(windows)]
pub fn roots() -> Vec<PathBuf> {
    let drives: Vec<PathBuf> = (b'A'..=b'Z')
        .map(|letter| PathBuf::from(format!("{}:\\", letter as char)))
        .filter(|p| p.exists())
        .collect();
    if drives.is_empty() {
        vec![PathBuf::from("C:\\")]
    } else {
        drives
    }
}

/// Where to go when nothing better is known (empty path input, no home dir).
pub fn default_root() -> PathBuf {
    roots()
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// The synthetic "list of roots" location, used above drive roots on Windows.
/// An empty path is the sentinel; it never names a real directory.
pub fn roots_path() -> PathBuf {
    PathBuf::new()
}

pub fn is_roots_path(path: &Path) -> bool {
    path.as_os_str().is_empty()
}

/// Whether a `..` entry above `path` makes sense: a parent directory, or the
/// roots listing when there is more than one root.
pub fn parent_of(path: &Path) -> Option<PathBuf> {
    if is_roots_path(path) {
        return None;
    }
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Some(parent.to_path_buf()),
        _ if roots().len() > 1 => Some(roots_path()),
        _ => None,
    }
}

/// Home and the standard user folders that exist, with an icon for each.
pub fn user_places() -> Vec<(String, PathBuf, gpui_kit::component::IconName)> {
    use gpui_kit::component::IconName;
    let mut places = Vec::new();
    let Some(dirs) = directories::UserDirs::new() else {
        return places;
    };
    places.push((
        "Home".to_string(),
        dirs.home_dir().to_path_buf(),
        IconName::User,
    ));
    let home = dirs.home_dir();
    // XDG user dirs when configured, else the conventional folder names.
    let known: [(&str, Option<&Path>, IconName); 6] = [
        ("Desktop", dirs.desktop_dir(), IconName::Frame),
        ("Documents", dirs.document_dir(), IconName::FileText),
        ("Downloads", dirs.download_dir(), IconName::Inbox),
        ("Pictures", dirs.picture_dir(), IconName::GalleryVerticalEnd),
        ("Music", dirs.audio_dir(), IconName::Mic),
        ("Videos", dirs.video_dir(), IconName::Play),
    ];
    for (name, dir, icon) in known {
        let dir = dir
            .map(Path::to_path_buf)
            .filter(|d| d.is_dir())
            .or_else(|| Some(home.join(name)).filter(|d| d.is_dir()));
        if let Some(dir) = dir {
            places.push((name.to_string(), dir, icon));
        }
    }
    places
}

/// Mounted volumes worth listing: the roots plus real file systems mounted
/// under the usual media/mount locations.
pub fn volumes() -> Vec<(String, PathBuf)> {
    let mut volumes: Vec<(String, PathBuf)> = roots()
        .into_iter()
        .map(|r| (r.to_string_lossy().into_owned(), r))
        .collect();
    #[cfg(target_os = "linux")]
    {
        const REAL_FS: [&str; 22] = [
            "ext2",
            "ext3",
            "ext4",
            "btrfs",
            "xfs",
            "zfs",
            "f2fs",
            "vfat",
            "exfat",
            "ntfs",
            "ntfs3",
            "fuseblk",
            "nfs",
            "nfs4",
            "cifs",
            "smb3",
            "autofs",
            "fuse.sshfs",
            "iso9660",
            "udf",
            "hfsplus",
            "apfs",
        ];
        if let Ok(mounts) = std::fs::read_to_string("/proc/self/mounts") {
            for line in mounts.lines() {
                let mut fields = line.split_whitespace();
                let (Some(_dev), Some(target), Some(fstype)) =
                    (fields.next(), fields.next(), fields.next())
                else {
                    continue;
                };
                if !REAL_FS.contains(&fstype) || target == "/" {
                    continue;
                }
                // Keep user-facing mount points, skip system subvolumes and internals.
                let user_facing = target.starts_with("/mnt/")
                    || target.starts_with("/media/")
                    || target.starts_with("/run/media/")
                    || target == "/home"
                    || target.starts_with("/home/")
                    || target.starts_with("/Volumes/");
                if !user_facing || target.contains("/docker/") {
                    continue;
                }
                let target = target.replace("\\040", " ");
                let path = PathBuf::from(&target);
                if volumes.iter().any(|(_, p)| *p == path) {
                    continue;
                }
                // The full mount point: names like "home" repeat across mounts.
                volumes.push((target.clone(), path));
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(rd) = std::fs::read_dir("/Volumes") {
            for entry in rd.flatten() {
                let path = entry.path();
                if path.is_dir() && !volumes.iter().any(|(_, p)| *p == path) {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    volumes.push((name, path));
                }
            }
        }
    }
    volumes
}

/// Open the user's terminal emulator in `dir`.
pub fn open_terminal(dir: &Path) -> std::io::Result<()> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    {
        return Command::new("open")
            .args(["-a", "Terminal"])
            .arg(dir)
            .spawn()
            .map(|_| ());
    }
    #[cfg(windows)]
    {
        return Command::new("cmd")
            .args(["/C", "start", "cmd"])
            .current_dir(dir)
            .spawn()
            .map(|_| ());
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let mut candidates: Vec<String> = Vec::new();
        if let Ok(term) = std::env::var("TERMINAL") {
            if !term.is_empty() {
                candidates.push(term);
            }
        }
        for name in [
            "xdg-terminal-exec",
            "ghostty",
            "alacritty",
            "kitty",
            "foot",
            "wezterm",
            "gnome-terminal",
            "konsole",
            "xterm",
        ] {
            candidates.push(name.to_string());
        }
        let mut last_err = std::io::Error::other("no terminal emulator found");
        for term in candidates {
            match Command::new(&term).current_dir(dir).spawn() {
                Ok(_) => return Ok(()),
                Err(err) => last_err = err,
            }
        }
        Err(last_err)
    }
}

/// Human readable label for a location, for the path bar and tab titles.
pub fn display_name(path: &Path) -> String {
    if is_roots_path(path) {
        return "Computer".to_string();
    }
    match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.to_string_lossy().into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_are_never_empty() {
        assert!(!roots().is_empty());
        assert!(roots().iter().all(|r| r.is_absolute()));
    }

    #[test]
    fn parent_chain_ends() {
        let mut p = default_root().join("a").join("b");
        let mut steps = 0;
        while let Some(parent) = parent_of(&p) {
            p = parent;
            steps += 1;
            assert!(steps < 10, "parent chain does not terminate");
        }
        assert!(steps >= 2);
    }

    #[test]
    fn roots_sentinel() {
        assert!(is_roots_path(&roots_path()));
        assert_eq!(display_name(&roots_path()), "Computer");
        assert_eq!(display_name(Path::new("/usr/lib")), "lib");
    }
}
