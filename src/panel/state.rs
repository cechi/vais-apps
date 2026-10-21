//! Pure, testable state of one file panel: entries, cursor, marks, history.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::fs::{Entry, Sort};

const HISTORY_LIMIT: usize = 64;

#[derive(Debug)]
pub struct PanelState {
    pub path: PathBuf,
    pub entries: Vec<Entry>,
    pub cursor: usize,
    /// Commander-style marks, independent of the cursor.
    pub marked: BTreeSet<usize>,
    /// Anchor for Shift+movement range marking.
    pub anchor: Option<usize>,
    pub sort: Sort,
    pub show_hidden: bool,
    pub history: Vec<PathBuf>,
    pub error: Option<String>,
    pub loading: bool,
}

impl PanelState {
    pub fn new(path: PathBuf, show_hidden: bool, sort: Sort) -> Self {
        Self {
            path,
            entries: Vec::new(),
            cursor: 0,
            marked: BTreeSet::new(),
            anchor: None,
            sort,
            show_hidden,
            history: Vec::new(),
            error: None,
            loading: false,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Number of real entries (without "..").
    pub fn item_count(&self) -> usize {
        self.entries.iter().filter(|e| !e.is_parent).count()
    }

    pub fn current(&self) -> Option<&Entry> {
        self.entries.get(self.cursor)
    }

    pub fn markable(&self, ix: usize) -> bool {
        self.entries.get(ix).is_some_and(|e| !e.is_parent)
    }

    fn clamp(&self, ix: isize) -> usize {
        if self.entries.is_empty() {
            0
        } else {
            ix.clamp(0, self.entries.len() as isize - 1) as usize
        }
    }

    pub fn move_cursor_to(&mut self, ix: usize) {
        self.cursor = self.clamp(ix.min(isize::MAX as usize) as isize);
        self.anchor = None;
    }

    pub fn last_index(&self) -> usize {
        self.entries.len().saturating_sub(1)
    }

    pub fn move_cursor_by(&mut self, delta: isize) {
        self.move_cursor_to(self.clamp(self.cursor as isize + delta));
    }

    /// Move the cursor and mark everything between the anchor and the new position.
    pub fn extend_to(&mut self, ix: usize) {
        let anchor = *self.anchor.get_or_insert(self.cursor);
        let target = self.clamp(ix.min(isize::MAX as usize) as isize);
        let (lo, hi) = if anchor <= target {
            (anchor, target)
        } else {
            (target, anchor)
        };
        for i in lo..=hi {
            if self.markable(i) {
                self.marked.insert(i);
            }
        }
        self.cursor = target;
    }

    pub fn extend_by(&mut self, delta: isize) {
        self.extend_to(self.clamp(self.cursor as isize + delta));
    }

    pub fn toggle_mark(&mut self, ix: usize) {
        if !self.markable(ix) {
            return;
        }
        if !self.marked.remove(&ix) {
            self.marked.insert(ix);
        }
        self.anchor = None;
    }

    pub fn mark_all(&mut self) {
        self.marked = (0..self.len()).filter(|&i| self.markable(i)).collect();
    }

    pub fn invert_marks(&mut self) {
        self.marked = (0..self.len())
            .filter(|&i| self.markable(i) && !self.marked.contains(&i))
            .collect();
    }

    pub fn clear_marks(&mut self) {
        self.marked.clear();
        self.anchor = None;
    }

    /// What an operation acts on: the marked entries, else the entry under the cursor.
    pub fn selected_entries(&self) -> Vec<&Entry> {
        if self.marked.is_empty() {
            self.current()
                .filter(|e| !e.is_parent)
                .into_iter()
                .collect()
        } else {
            self.marked
                .iter()
                .filter_map(|&i| self.entries.get(i))
                .filter(|e| !e.is_parent)
                .collect()
        }
    }

    pub fn marked_bytes(&self) -> u64 {
        self.marked
            .iter()
            .filter_map(|&i| self.entries.get(i))
            .filter(|e| !e.is_dir())
            .map(|e| e.size)
            .sum()
    }

    /// Replace the listing. The cursor goes to `select` when found; otherwise a
    /// refresh of the same directory keeps its position (so deleting an entry
    /// lands on the next one) and a new directory starts at the top.
    pub fn set_entries(&mut self, path: PathBuf, entries: Vec<Entry>, select: Option<&str>) {
        let same_dir = path == self.path;
        self.path = path;
        self.entries = entries;
        self.marked.clear();
        self.anchor = None;
        self.error = None;
        let by_name = select.and_then(|name| self.entries.iter().position(|e| e.name == name));
        self.cursor = match by_name {
            Some(ix) => ix,
            None if same_dir => self.cursor.min(self.entries.len().saturating_sub(1)),
            None => 0,
        };
    }

    pub fn push_history(&mut self, path: PathBuf) {
        self.history.push(path);
        if self.history.len() > HISTORY_LIMIT {
            self.history.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::EntryKind;

    fn state(n: usize) -> PanelState {
        let mut entries = vec![Entry {
            name: "..".into(),
            path: "/".into(),
            kind: EntryKind::Dir,
            size: 0,
            modified: None,
            is_parent: true,
        }];
        for i in 0..n {
            entries.push(Entry {
                name: format!("f{i}"),
                path: format!("/d/f{i}").into(),
                kind: EntryKind::File,
                size: 10,
                modified: None,
                is_parent: false,
            });
        }
        let mut s = PanelState::new("/d".into(), false, Sort::default());
        s.set_entries("/d".into(), entries, None);
        s
    }

    #[test]
    fn cursor_clamps_and_clears_anchor() {
        let mut s = state(3);
        s.move_cursor_by(10);
        assert_eq!(s.cursor, 3);
        s.move_cursor_by(-10);
        assert_eq!(s.cursor, 0);
        s.extend_by(1);
        assert_eq!(s.anchor, Some(0));
        s.move_cursor_by(1);
        assert_eq!(s.anchor, None);
    }

    #[test]
    fn end_moves_to_last_entry() {
        let mut s = state(3);
        s.move_cursor_to(usize::MAX);
        assert_eq!(s.cursor, 3);
        s.move_cursor_to(0);
        s.extend_to(usize::MAX);
        assert_eq!(s.cursor, 3);
        assert_eq!(s.marked.len(), 3);
    }

    #[test]
    fn parent_entry_is_never_marked() {
        let mut s = state(3);
        s.toggle_mark(0);
        assert!(s.marked.is_empty());
        s.mark_all();
        assert_eq!(s.marked.len(), 3);
        assert_eq!(s.marked_bytes(), 30);
        s.invert_marks();
        assert!(s.marked.is_empty());
    }

    #[test]
    fn shift_range_marks_inclusive() {
        let mut s = state(5);
        s.move_cursor_to(2);
        s.extend_by(1);
        s.extend_by(1);
        assert_eq!(s.cursor, 4);
        assert_eq!(s.marked.iter().copied().collect::<Vec<_>>(), vec![2, 3, 4]);
        s.extend_by(-3);
        assert_eq!(s.cursor, 1);
        assert_eq!(
            s.marked.iter().copied().collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
    }

    #[test]
    fn selection_falls_back_to_cursor() {
        let mut s = state(3);
        assert!(
            s.selected_entries().is_empty(),
            "cursor on .. selects nothing"
        );
        s.move_cursor_to(2);
        assert_eq!(s.selected_entries().len(), 1);
        s.toggle_mark(1);
        s.toggle_mark(3);
        let names: Vec<_> = s
            .selected_entries()
            .iter()
            .map(|e| e.name.clone())
            .collect();
        assert_eq!(names, ["f0", "f2"]);
    }

    #[test]
    fn refresh_keeps_position_when_entry_vanished() {
        let mut s = state(3);
        s.move_cursor_to(2); // f1
        let mut entries = s.entries.clone();
        entries.remove(2);
        s.set_entries("/d".into(), entries, Some("f1"));
        assert_eq!(s.cursor, 2, "lands on the entry that followed f1");
        let entries = s.entries.clone();
        s.set_entries("/other".into(), entries, Some("f1"));
        assert_eq!(s.cursor, 0, "a different directory starts at the top");
    }

    #[test]
    fn set_entries_selects_by_name() {
        let mut s = state(3);
        let entries = s.entries.clone();
        s.set_entries("/d".into(), entries, Some("f2"));
        assert_eq!(s.cursor, 3);
    }
}
