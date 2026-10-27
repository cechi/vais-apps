use std::time::SystemTime;

use super::EntryKind;

use chrono::{DateTime, Local};

/// Human readable size in binary units: `512 B`, `1.5 KiB`, `120 MiB`.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn format_time(time: SystemTime) -> String {
    let dt: DateTime<Local> = time.into();
    dt.format("%Y-%m-%d %H:%M").to_string()
}

/// `ls`-style permissions: `drwxr-xr-x`, with setuid/setgid/sticky bits.
pub fn format_mode(mode: u32, kind: &EntryKind) -> String {
    let mut out = String::with_capacity(10);
    out.push(match kind {
        EntryKind::Dir => 'd',
        EntryKind::Symlink { .. } => 'l',
        EntryKind::File => '-',
        EntryKind::Other => match mode & 0o170000 {
            0o060000 => 'b',
            0o020000 => 'c',
            0o010000 => 'p',
            0o140000 => 's',
            _ => '?',
        },
    });
    let bits = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ];
    for (i, (bit, ch)) in bits.iter().enumerate() {
        let set = mode & bit != 0;
        let special = match i {
            2 => mode & 0o4000 != 0,
            5 => mode & 0o2000 != 0,
            8 => mode & 0o1000 != 0,
            _ => false,
        };
        out.push(match (set, special, i) {
            (true, true, 8) => 't',
            (false, true, 8) => 'T',
            (true, true, _) => 's',
            (false, true, _) => 'S',
            (true, false, _) => *ch,
            (false, false, _) => '-',
        });
    }
    out
}

/// The user name for `uid`, from the password database (cached), or the
/// number when unknown.
#[cfg(unix)]
pub fn owner_name(uid: u32) -> String {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static USERS: OnceLock<HashMap<u32, String>> = OnceLock::new();
    let users = USERS.get_or_init(|| {
        std::fs::read_to_string("/etc/passwd")
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let mut parts = line.split(':');
                let name = parts.next()?;
                let uid: u32 = parts.nth(1)?.parse().ok()?;
                Some((uid, name.to_string()))
            })
            .collect()
    });
    users.get(&uid).cloned().unwrap_or_else(|| uid.to_string())
}

#[cfg(not(unix))]
pub fn owner_name(uid: u32) -> String {
    uid.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_look_like_ls() {
        assert_eq!(format_mode(0o100644, &EntryKind::File), "-rw-r--r--");
        assert_eq!(format_mode(0o040755, &EntryKind::Dir), "drwxr-xr-x");
        assert_eq!(
            format_mode(0o120777, &EntryKind::Symlink { to_dir: false }),
            "lrwxrwxrwx"
        );
        assert_eq!(format_mode(0o104755, &EntryKind::File), "-rwsr-xr-x");
        assert_eq!(format_mode(0o041777, &EntryKind::Dir), "drwxrwxrwt");
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(1536), "1.5 KiB");
        assert_eq!(human_size(120 * 1024 * 1024), "120 MiB");
        assert_eq!(human_size(5 * 1024 * 1024 * 1024), "5.0 GiB");
    }
}
