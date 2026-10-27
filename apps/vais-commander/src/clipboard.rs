//! The file clipboard behind Ctrl+C / Ctrl+X / Ctrl+V: a list of paths and
//! whether they were cut. It lives inside the app (the system clipboard
//! keeps whatever text the user put there); panels dim the cut entries.

use gpui_kit::*;

use crate::vfs::Location;

#[derive(Debug, Clone, Default)]
pub struct FileClipboard {
    pub paths: Vec<Location>,
    pub cut: bool,
}

impl Global for FileClipboard {}

impl FileClipboard {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// Whether `path` is waiting to be moved (shown dimmed).
    pub fn is_cut(&self, path: &Location) -> bool {
        self.cut && self.paths.iter().any(|p| p == path)
    }

    /// "3 items (cut)" for the status bar, or `None` when empty.
    pub fn summary(&self) -> Option<String> {
        if self.paths.is_empty() {
            return None;
        }
        let n = self.paths.len();
        let name = if n == 1 {
            self.paths[0].file_name()
        } else {
            None
        };
        Some(match (name, self.cut) {
            (Some(name), false) => name,
            (Some(name), true) => t!("{name} (cut)", name = name),
            (None, false) => tn!("{n} item", "{n} items", n),
            (None, true) => tn!("{n} item (cut)", "{n} items (cut)", n),
        })
    }
}

pub fn init(cx: &mut App) {
    cx.set_global(FileClipboard::default());
}

pub fn set(cx: &mut App, paths: Vec<Location>, cut: bool) {
    *cx.global_mut::<FileClipboard>() = FileClipboard { paths, cut };
}

pub fn clear(cx: &mut App) {
    *cx.global_mut::<FileClipboard>() = FileClipboard::default();
}

pub fn get(cx: &App) -> FileClipboard {
    cx.global::<FileClipboard>().clone()
}

#[cfg(test)]
mod tests {
    use super::FileClipboard;
    use crate::vfs::Location;

    #[test]
    fn summary_names_a_single_item_and_counts_more() {
        let one = FileClipboard {
            paths: vec![Location::local("/x/report.pdf")],
            cut: false,
        };
        assert_eq!(one.summary().as_deref(), Some("report.pdf"));
        let two = FileClipboard {
            paths: vec![Location::local("/x/a"), Location::local("/x/b")],
            cut: true,
        };
        assert_eq!(two.summary().as_deref(), Some("2 items (cut)"));
        assert!(two.is_cut(&Location::local("/x/a")));
        assert!(!one.is_cut(&Location::local("/x/report.pdf")));
        assert_eq!(FileClipboard::default().summary(), None);
    }
}
