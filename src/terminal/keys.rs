//! Key presses as the bytes a terminal program expects: the xterm
//! sequences, with the DECCKM application cursor mode taken into account.
//! No kitty keyboard protocol yet.

use alacritty_terminal::term::TermMode;
use gpui_kit::*;

/// The bytes for `keystroke`, or `None` when it is not a terminal key.
pub fn encode(keystroke: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let m = &keystroke.modifiers;
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    // xterm modifier parameter: 1 + shift(1) + alt(2) + ctrl(4).
    let param = 1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.control as u8;
    let with_param = |prefix: &str, suffix: &str| -> Vec<u8> {
        if param > 1 {
            format!("\x1b[{prefix};{param}{suffix}").into_bytes()
        } else if app_cursor && suffix.len() == 1 && suffix.as_bytes()[0].is_ascii_alphabetic() {
            format!("\x1bO{suffix}").into_bytes()
        } else {
            format!("\x1b[{suffix}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if param > 1 {
            format!("\x1b[{n};{param}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    let key = keystroke.key.as_str();
    let bytes = match key {
        "enter" => {
            if m.alt {
                b"\x1b\r".to_vec()
            } else {
                b"\r".to_vec()
            }
        }
        "escape" => b"\x1b".to_vec(),
        "tab" => {
            if m.shift {
                b"\x1b[Z".to_vec()
            } else {
                b"\t".to_vec()
            }
        }
        "backspace" => match (m.control, m.alt) {
            (true, _) => b"\x08".to_vec(),
            (false, true) => b"\x1b\x7f".to_vec(),
            _ => b"\x7f".to_vec(),
        },
        "space" => {
            if m.control {
                b"\0".to_vec()
            } else if m.alt {
                b"\x1b ".to_vec()
            } else {
                b" ".to_vec()
            }
        }
        "up" => with_param("1", "A"),
        "down" => with_param("1", "B"),
        "right" => with_param("1", "C"),
        "left" => with_param("1", "D"),
        "home" => with_param("1", "H"),
        "end" => with_param("1", "F"),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => with_param("1", "P"),
        "f2" => with_param("1", "Q"),
        "f3" => with_param("1", "R"),
        "f4" => with_param("1", "S"),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        _ => {
            if m.control && !m.alt {
                return control_byte(key).map(|b| vec![b]);
            }
            let text = keystroke.key_char.as_deref().unwrap_or(key);
            if text.chars().count() != 1 {
                return None;
            }
            let mut bytes = Vec::new();
            if m.alt {
                bytes.push(0x1b);
            }
            bytes.extend_from_slice(text.as_bytes());
            bytes
        }
    };
    Some(bytes)
}

/// The byte Ctrl+`key` produces (letters, and the xterm punctuation set).
fn control_byte(key: &str) -> Option<u8> {
    let c = key.chars().next()?;
    if key.chars().count() != 1 {
        return None;
    }
    Some(match c.to_ascii_lowercase() {
        'a'..='z' => (c.to_ascii_lowercase() as u8) & 0x1f,
        '@' | '2' => 0x00,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '7' | '-' => 0x1f,
        '8' => 0x7f,
        _ => return None,
    })
}

/// Pasted text as the program should receive it: bracketed when it asked
/// for that, else with newlines turned into carriage returns.
pub fn paste(text: &str, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let cleaned = text.replace("\x1b[201~", "");
        let mut bytes = b"\x1b[200~".to_vec();
        bytes.extend_from_slice(cleaned.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
        bytes
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::{encode, paste};
    use alacritty_terminal::term::TermMode;
    use gpui_kit::{Keystroke, Modifiers};

    fn ks(key: &str, modifiers: Modifiers, key_char: Option<&str>) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_string(),
            key_char: key_char.map(str::to_string),
        }
    }

    #[test]
    fn encodes_xterm_sequences() {
        let none = TermMode::empty();
        assert_eq!(
            encode(&ks("enter", Modifiers::none(), None), none),
            Some(b"\r".to_vec())
        );
        assert_eq!(
            encode(&ks("up", Modifiers::none(), None), none),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            encode(&ks("up", Modifiers::none(), None), TermMode::APP_CURSOR),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            encode(&ks("up", Modifiers::control(), None), none),
            Some(b"\x1b[1;5A".to_vec())
        );
        assert_eq!(
            encode(&ks("f5", Modifiers::none(), None), none),
            Some(b"\x1b[15~".to_vec())
        );
        assert_eq!(
            encode(&ks("f1", Modifiers::none(), None), none),
            Some(b"\x1b[P".to_vec())
        );
        assert_eq!(
            encode(&ks("delete", Modifiers::shift(), None), none),
            Some(b"\x1b[3;2~".to_vec())
        );
        assert_eq!(
            encode(&ks("tab", Modifiers::shift(), None), none),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(
            encode(&ks("c", Modifiers::control(), None), none),
            Some(vec![0x03])
        );
        assert_eq!(
            encode(&ks("a", Modifiers::none(), Some("a")), none),
            Some(b"a".to_vec())
        );
        assert_eq!(
            encode(&ks("x", Modifiers::alt(), Some("x")), none),
            Some(b"\x1bx".to_vec())
        );
        assert_eq!(
            encode(&ks("ě", Modifiers::none(), Some("ě")), none),
            Some("ě".as_bytes().to_vec())
        );
        assert_eq!(encode(&ks("f13", Modifiers::none(), None), none), None);
    }

    #[test]
    fn paste_is_bracketed_on_request() {
        assert_eq!(paste("a\nb", TermMode::empty()), b"a\rb".to_vec());
        assert_eq!(
            paste("a\x1b[201~b", TermMode::BRACKETED_PASTE),
            b"\x1b[200~ab\x1b[201~".to_vec()
        );
    }
}
