//! Colours a tab (and a saved layout) can be tagged with: a few presets and
//! any `#rrggbb` the user types.

use gpui_kit::*;

/// Name and hex of the presets offered in the colour palette.
pub const PRESETS: [(&str, &str); 9] = [
    ("Red", "#e5484d"),
    ("Orange", "#f76b15"),
    ("Yellow", "#f5d90a"),
    ("Green", "#30a46c"),
    ("Teal", "#12a594"),
    ("Blue", "#3e63dd"),
    ("Purple", "#8e4ec6"),
    ("Pink", "#e93d82"),
    ("Gray", "#8b8d98"),
];

/// A preset's name as shown (the table's names are the English keys).
pub fn label(name: &'static str) -> &'static str {
    match name {
        "Red" => t!("Red"),
        "Orange" => t!("Orange"),
        "Yellow" => t!("Yellow"),
        "Green" => t!("Green"),
        "Teal" => t!("Teal"),
        "Blue" => t!("Blue"),
        "Purple" => t!("Purple"),
        "Pink" => t!("Pink"),
        "Gray" => t!("Gray"),
        _ => name,
    }
}

/// `#rgb`, `#rrggbb` or `#rrggbbaa` (with or without the `#`) as a colour.
pub fn parse(text: &str) -> Option<Hsla> {
    let text = text.trim();
    let hex = text.strip_prefix('#').unwrap_or(text);
    if !matches!(hex.len(), 3 | 6 | 8) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Rgba::try_from(format!("#{hex}").as_str())
        .ok()
        .map(Hsla::from)
}

/// The stored form of a colour the user typed: lowercase with a `#`.
pub fn normalise(text: &str) -> Option<String> {
    parse(text).map(|_| {
        let hex = text.trim().trim_start_matches('#').to_lowercase();
        format!("#{hex}")
    })
}

/// The preset name for a stored colour, if it is one.
pub fn preset_name(hex: &str) -> Option<&'static str> {
    PRESETS
        .iter()
        .find(|(_, preset)| preset.eq_ignore_ascii_case(hex))
        .map(|(name, _)| *name)
}

#[cfg(test)]
mod tests {
    use super::{normalise, parse, preset_name};

    #[test]
    fn parses_hex_in_the_usual_forms() {
        assert!(parse("#e5484d").is_some());
        assert!(parse("E5484D").is_some());
        assert!(parse("#fff").is_some());
        assert!(parse("#12345").is_none());
        assert!(parse("red").is_none());
        assert_eq!(normalise(" E5484D ").as_deref(), Some("#e5484d"));
        assert_eq!(preset_name("#E5484D"), Some("Red"));
        assert_eq!(preset_name("#000000"), None);
    }
}
