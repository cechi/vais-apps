//! Pure, testable state of one file panel: entries, cursor, marks, history.

use crate::vfs::Location;
use std::collections::{BTreeSet, HashMap};

use crate::fs::filter::Matcher;
use crate::fs::{Entry, Sort};

const HISTORY_LIMIT: usize = 64;

/// The panel's filter bar state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub query: String,
    /// Search subdirectories recursively instead of filtering the listing.
    pub recursive: bool,
    /// Look for the text inside files instead of matching names.
    pub content: bool,
}

impl Filter {
    pub fn matcher(&self) -> Matcher {
        Matcher::new(&self.query)
    }

    /// The listing comes from a search task (subfolders or contents), not
    /// from filtering the directory's own entries.
    pub fn is_search(&self) -> bool {
        self.recursive || self.content
    }
}

#[derive(Debug)]
pub struct PanelState {
    pub path: Location,
    /// Everything the loader (or a recursive search) produced.
    pub all_entries: Vec<Entry>,
    /// The displayed rows: `all_entries` after the filter. Cursor and marks index these.
    pub entries: Vec<Entry>,
    pub filter: Option<Filter>,
    /// The portal picker's file filter (any of these must match a file's
    /// name); directories always pass. Kept across navigation.
    pub pick_filter: Vec<Matcher>,
    /// A recursive search stopped at its result limit.
    pub search_truncated: bool,
    pub cursor: usize,
    /// Commander-style marks, independent of the cursor.
    pub marked: BTreeSet<usize>,
    /// Anchor for Shift+movement range marking.
    pub anchor: Option<usize>,
    pub sort: Sort,
    pub show_hidden: bool,
    /// Back stack: directories visited before the current one.
    pub history: Vec<Location>,
    /// Forward stack: directories left by going back.
    pub forward: Vec<Location>,
    pub error: Option<String>,
    pub loading: bool,
    /// Directories shown expanded inline (tree view).
    pub expanded: BTreeSet<Location>,
    /// Sizes of directories counted on request (Space), by location.
    pub dir_sizes: HashMap<Location, u64>,
}

impl PanelState {
    pub fn new(path: Location, show_hidden: bool, sort: Sort) -> Self {
        Self {
            path,
            all_entries: Vec::new(),
            entries: Vec::new(),
            filter: None,
            pick_filter: Vec::new(),
            search_truncated: false,
            cursor: 0,
            marked: BTreeSet::new(),
            anchor: None,
            sort,
            show_hidden,
            history: Vec::new(),
            forward: Vec::new(),
            error: None,
            loading: false,
            expanded: BTreeSet::new(),
            dir_sizes: HashMap::new(),
        }
    }

    pub fn is_expanded(&self, ix: usize) -> bool {
        self.entries
            .get(ix)
            .is_some_and(|e| self.expanded.contains(&e.location))
    }

    /// Expand or collapse the directory at `ix`; returns its path when it is
    /// a directory (the caller then reloads the listing).
    pub fn toggle_expanded(&mut self, ix: usize) -> Option<Location> {
        let entry = self.entries.get(ix)?;
        if entry.is_parent || !entry.is_dir() {
            return None;
        }
        let path = entry.location.clone();
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

    /// Names of the marked entries, in listing order.
    pub fn selected_marked_names(&self) -> Vec<String> {
        self.marked
            .iter()
            .filter_map(|&i| self.entries.get(i))
            .map(|e| e.name.clone())
            .collect()
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

    /// Bytes in the marked entries; a directory counts once its size was
    /// asked for (Space).
    pub fn marked_bytes(&self) -> u64 {
        self.marked
            .iter()
            .filter_map(|&i| self.entries.get(i))
            .map(|e| self.entry_bytes(e))
            .sum()
    }

    pub fn entry_bytes(&self, entry: &Entry) -> u64 {
        if entry.is_dir() {
            self.dir_sizes.get(&entry.location).copied().unwrap_or(0)
        } else {
            entry.size
        }
    }

    /// Mark (or unmark) every entry whose name matches `matcher`.
    pub fn mark_matching(&mut self, matcher: &Matcher, on: bool) {
        for (ix, entry) in self.entries.iter().enumerate() {
            if entry.is_parent || !matcher.matches(&entry.name) {
                continue;
            }
            if on {
                self.marked.insert(ix);
            } else {
                self.marked.remove(&ix);
            }
        }
        self.anchor = None;
    }

    /// Replace the full listing and rebuild the displayed rows through the filter.
    pub fn set_entries(&mut self, path: Location, entries: Vec<Entry>, select: Option<&Location>) {
        self.all_entries = entries;
        self.apply_view(path, select);
    }

    /// Re-run the filter over the current full listing (same directory).
    pub fn refilter(&mut self, select: Option<&Location>) {
        let path = self.path.clone();
        self.apply_view(path, select);
    }

    fn filtered_rows(&self) -> Vec<Entry> {
        let rows = match &self.filter {
            // Recursive results arrive already matched; the live filter matches names.
            Some(filter) if !filter.is_search() && !filter.matcher().is_all() => {
                let matcher = filter.matcher();
                self.all_entries
                    .iter()
                    .filter(|e| e.is_parent || matcher.matches(&e.name))
                    .cloned()
                    .collect()
            }
            _ => self.all_entries.clone(),
        };
        if self.pick_filter.is_empty() {
            return rows;
        }
        rows.into_iter()
            .filter(|e| {
                e.is_parent || e.is_dir() || self.pick_filter.iter().any(|m| m.matches(&e.name))
            })
            .collect()
    }

    /// Rebuild `entries`. The cursor goes to `select` when found; otherwise a
    /// refresh of the same directory keeps its position (so deleting an entry
    /// lands on the next one) and a new directory starts at the top. Marks
    /// survive a refresh of the same directory, matched by path.
    fn apply_view(&mut self, path: Location, select: Option<&Location>) {
        let same_dir = path == self.path;
        let marked_paths: Vec<Location> = if same_dir {
            self.marked
                .iter()
                .filter_map(|&i| self.entries.get(i))
                .map(|e| e.location.clone())
                .collect()
        } else {
            Vec::new()
        };
        if !same_dir {
            self.dir_sizes.clear();
        }
        self.path = path;
        self.entries = self.filtered_rows();
        self.anchor = None;
        self.error = None;
        self.marked = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.is_parent && marked_paths.contains(&e.location))
            .map(|(i, _)| i)
            .collect();
        let by_path = select.and_then(|p| self.entries.iter().position(|e| e.location == *p));
        self.cursor = match by_path {
            Some(ix) => ix,
            None if same_dir => self.cursor.min(self.entries.len().saturating_sub(1)),
            None => 0,
        };
    }

    /// A new navigation: remember where we were and drop the forward stack.
    pub fn push_history(&mut self, path: Location) {
        self.history.push(path);
        if self.history.len() > HISTORY_LIMIT {
            self.history.remove(0);
        }
        self.forward.clear();
    }

    /// Where "Back" goes; the current path moves to the forward stack.
    pub fn pop_back(&mut self) -> Option<Location> {
        let target = self.history.pop()?;
        self.forward.push(self.path.clone());
        Some(target)
    }

    /// Where "Forward" goes; the current path moves to the back stack.
    pub fn pop_forward(&mut self) -> Option<Location> {
        let target = self.forward.pop()?;
        self.history.push(self.path.clone());
        Some(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::EntryKind;

    fn state(n: usize) -> PanelState {
        let mut entries = vec![Entry {
            name: "..".into(),
            location: "/".into(),
            kind: EntryKind::Dir,
            size: 0,
            modified: None,
            created: None,
            mode: None,
            uid: None,
            is_parent: true,
            depth: 0,
        }];
        for i in 0..n {
            entries.push(Entry {
                name: format!("f{i}"),
                location: Location::local(format!("/d/f{i}")),
                kind: EntryKind::File,
                size: 10,
                modified: None,
                created: None,
                mode: None,
                uid: None,
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
    fn back_and_forward_stacks() {
        let mut s = state(1);
        s.push_history("/a".into());
        s.path = "/b".into();
        s.push_history("/b".into());
        s.path = "/c".into();
        assert_eq!(s.pop_back(), Some(Location::local("/b")));
        s.path = "/b".into();
        assert_eq!(s.forward, vec![Location::local("/c")]);
        assert_eq!(s.pop_forward(), Some(Location::local("/c")));
        s.path = "/c".into();
        assert!(s.forward.is_empty());
        assert_eq!(s.pop_back(), Some(Location::local("/b")));
        s.path = "/b".into();
        s.push_history("/b".into());
        assert!(
            s.forward.is_empty(),
            "a new navigation drops the forward stack"
        );
    }

    #[test]
    fn filter_narrows_rows_and_keeps_marks_by_path() {
        let mut s = state(4);
        s.toggle_mark(3); // f2
        s.filter = Some(Filter {
            query: "f2".into(),
            recursive: false,
            content: false,
        });
        s.refilter(Some(&Location::local("/d/f2")));
        let names: Vec<_> = s.entries.iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, ["..", "f2"]);
        assert_eq!(s.cursor, 1);
        assert_eq!(s.marked.iter().copied().collect::<Vec<_>>(), vec![1]);
        s.filter = None;
        s.refilter(Some(&Location::local("/d/f2")));
        assert_eq!(s.entries.len(), 5);
        assert_eq!(s.cursor, 3);
        assert_eq!(s.marked.iter().copied().collect::<Vec<_>>(), vec![3]);
    }

    #[test]
    fn pick_filter_hides_files_but_not_directories() {
        let mut s = state(4);
        s.all_entries[1].kind = crate::fs::EntryKind::Dir; // f0 is a directory
        s.pick_filter = vec![Matcher::Glob("f2".chars().collect())];
        s.refilter(None);
        let names: Vec<_> = s.entries.iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, ["..", "f0", "f2"]);
        // The live filter narrows further; the pick filter stays.
        s.filter = Some(Filter {
            query: "f0".into(),
            recursive: false,
            content: false,
        });
        s.refilter(None);
        let names: Vec<_> = s.entries.iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, ["..", "f0"]);
        s.pick_filter.clear();
        s.filter = None;
        s.refilter(None);
        assert_eq!(s.entries.len(), 5);
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
        s.set_entries("/d".into(), entries, Some(&Location::local("/d/f1")));
        assert_eq!(s.cursor, 2, "lands on the entry that followed f1");
        let entries = s.entries.clone();
        s.set_entries("/other".into(), entries, Some(&Location::local("/d/f1")));
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
                location: "/d/new".into(),
                kind: EntryKind::Dir,
                size: 0,
                modified: None,
                created: None,
                mode: None,
                uid: None,
                is_parent: false,
                depth: 0,
            },
        );
        s.set_entries("/d".into(), entries, Some(&Location::local("/d/f2")));
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
            location: "/d/dir".into(),
            kind: EntryKind::Dir,
            size: 0,
            modified: None,
            created: None,
            mode: None,
            uid: None,
            is_parent: false,
            depth: 0,
        };
        let child = Entry {
            name: "c".into(),
            location: "/d/dir/c".into(),
            kind: EntryKind::File,
            size: 1,
            modified: None,
            created: None,
            mode: None,
            uid: None,
            is_parent: false,
            depth: 1,
        };
        let entries = vec![s.entries[0].clone(), dir, child, s.entries[1].clone()];
        s.set_entries("/d".into(), entries, None);
        assert_eq!(s.toggle_expanded(0), None, ".. is not expandable");
        assert_eq!(s.toggle_expanded(3), None, "files are not expandable");
        assert_eq!(s.toggle_expanded(1), Some(Location::local("/d/dir")));
        assert!(s.is_expanded(1));
        assert_eq!(s.parent_index(2), Some(1));
        assert_eq!(s.parent_index(1), None);
        s.toggle_expanded(1);
        assert!(!s.is_expanded(1));
    }
}
