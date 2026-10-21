//! Pure, testable state of one file panel: entries, cursor, marks, history.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

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
    /// Directories shown expanded inline (tree view).
    pub expanded: BTreeSet<PathBuf>,
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
            expanded: BTreeSet::new(),
        }
    }

    pub fn is_expanded(&self, ix: usize) -> bool {
        self.entries
            .get(ix)
            .is_some_and(|e| self.expanded.contains(&e.path))
    }

    /// Expand or collapse the directory at `ix`; returns its path when it is
    /// a directory (the caller then reloads the listing).
    pub fn toggle_expanded(&mut self, ix: usize) -> Option<PathBuf> {
        let entry = self.entries.get(ix)?;
        if entry.is_parent || !entry.is_dir() {
            return None;
        }
        let path = entry.path.clone();
        if !self.expanded.remove(&path) {
            self.expanded.insert(path.clone());
        } else {
            // Collapsing a directory also forgets what was expanded inside it.
            self.expanded.retain(|p| !p.starts_with(&path));
        }
        Some(path)
    }

    /// Index of the row the entry at `ix` is nested under, if any.
    pub fn parent_index(&self, ix: usize) -> Option<usize> {
        let depth = self.entries.get(ix)?.depth;
        if depth == 0 {
            return None;
        }
        (0..ix).rev().find(|&i| self.entries[i].depth < depth)
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

    /// Set the marks to `base` plus `rows` (the rubber band selection).
    pub fn mark_rows(&mut self, rows: impl IntoIterator<Item = usize>, base: &BTreeSet<usize>) {
        let mut marked = base.clone();
        marked.extend(rows);
        self.marked = marked.into_iter().filter(|&i| self.markable(i)).collect();
        self.anchor = None;
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
    /// lands on the next one) and a new directory starts at the top. Marks
    /// survive a refresh of the same directory, matched by path.
    pub fn set_entries(&mut self, path: PathBuf, entries: Vec<Entry>, select: Option<&Path>) {
        let same_dir = path == self.path;
        let marked_paths: Vec<PathBuf> = if same_dir {
            self.marked
                .iter()
                .filter_map(|&i| self.entries.get(i))
                .map(|e| e.path.clone())
                .collect()
        } else {
            Vec::new()
        };
        self.path = path;
        self.entries = entries;
        self.anchor = None;
        self.error = None;
        self.marked = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.is_parent && marked_paths.contains(&e.path))
            .map(|(i, _)| i)
            .collect();
        let by_path = select.and_then(|p| self.entries.iter().position(|e| e.path == p));
        self.cursor = match by_path {
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
            depth: 0,
        }];
        for i in 0..n {
            entries.push(Entry {
                name: format!("f{i}"),
                path: format!("/d/f{i}").into(),
                kind: EntryKind::File,
                size: 10,
                modified: None,
                is_parent: false,
                depth: 0,
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
    fn mark_rows_unions_with_base_and_skips_parent() {
        let mut s = state(4);
        s.toggle_mark(4);
        let base = s.marked.clone();
        s.mark_rows(0..=2, &base);
        assert_eq!(s.marked.iter().copied().collect::<Vec<_>>(), vec![1, 2, 4]);
        s.mark_rows(3..=3, &BTreeSet::new());
        assert_eq!(s.marked.iter().copied().collect::<Vec<_>>(), vec![3]);
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
        s.set_entries("/d".into(), entries, Some(Path::new("/d/f1")));
        assert_eq!(s.cursor, 2, "lands on the entry that followed f1");
        let entries = s.entries.clone();
        s.set_entries("/other".into(), entries, Some(Path::new("/d/f1")));
        assert_eq!(s.cursor, 0, "a different directory starts at the top");
    }

    #[test]
    fn set_entries_selects_by_path_and_keeps_marks() {
        let mut s = state(3);
        s.toggle_mark(1);
        s.toggle_mark(3);
        let mut entries = s.entries.clone();
        entries.insert(
            1,
            Entry {
                name: "new".into(),
                path: "/d/new".into(),
                kind: EntryKind::Dir,
                size: 0,
                modified: None,
                is_parent: false,
                depth: 0,
            },
        );
        s.set_entries("/d".into(), entries, Some(Path::new("/d/f2")));
        assert_eq!(s.cursor, 4);
        let marked: Vec<_> = s
            .marked
            .iter()
            .map(|&i| s.entries[i].name.clone())
            .collect();
        assert_eq!(marked, ["f0", "f2"], "marks follow their paths");
    }

    #[test]
    fn expand_and_parent_index() {
        let mut s = state(2);
        let dir = Entry {
            name: "dir".into(),
            path: "/d/dir".into(),
            kind: EntryKind::Dir,
            size: 0,
            modified: None,
            is_parent: false,
            depth: 0,
        };
        let child = Entry {
            name: "c".into(),
            path: "/d/dir/c".into(),
            kind: EntryKind::File,
            size: 1,
            modified: None,
            is_parent: false,
            depth: 1,
        };
        let entries = vec![s.entries[0].clone(), dir, child, s.entries[1].clone()];
        s.set_entries("/d".into(), entries, None);
        assert_eq!(s.toggle_expanded(0), None, ".. is not expandable");
        assert_eq!(s.toggle_expanded(3), None, "files are not expandable");
        assert_eq!(s.toggle_expanded(1), Some(PathBuf::from("/d/dir")));
        assert!(s.is_expanded(1));
        assert_eq!(s.parent_index(2), Some(1));
        assert_eq!(s.parent_index(1), None);
        s.toggle_expanded(1);
        assert!(!s.is_expanded(1));
    }
}
