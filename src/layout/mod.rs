//! Uniform grid layout of panels. A split tree can be added later behind this module.

pub mod grid;

use serde::{Deserialize, Serialize};

pub const MAX_ROWS: u8 = 3;
pub const MAX_COLS: u8 = 3;
pub const MAX_PANELS: usize = (MAX_ROWS * MAX_COLS) as usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    pub rows: u8,
    pub cols: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Default for Layout {
    fn default() -> Self {
        Self { rows: 1, cols: 2 }
    }
}

impl Layout {
    /// Alt-1 … Alt-9 presets, in order.
    pub const PRESETS: [Layout; 9] = [
        Layout { rows: 1, cols: 2 },
        Layout { rows: 2, cols: 1 },
        Layout { rows: 1, cols: 3 },
        Layout { rows: 3, cols: 1 },
        Layout { rows: 2, cols: 2 },
        Layout { rows: 2, cols: 3 },
        Layout { rows: 3, cols: 2 },
        Layout { rows: 3, cols: 3 },
        Layout { rows: 1, cols: 1 },
    ];

    pub fn new(rows: u8, cols: u8) -> Self {
        Self {
            rows: rows.clamp(1, MAX_ROWS),
            cols: cols.clamp(1, MAX_COLS),
        }
    }

    pub fn count(self) -> usize {
        self.rows as usize * self.cols as usize
    }

    pub fn label(self) -> String {
        format!("{}x{}", self.rows, self.cols)
    }

    pub fn position(self, ix: usize) -> (usize, usize) {
        (ix / self.cols as usize, ix % self.cols as usize)
    }

    pub fn index(self, row: usize, col: usize) -> usize {
        row * self.cols as usize + col
    }

    pub fn next(self, ix: usize) -> usize {
        (ix + 1) % self.count()
    }

    pub fn prev(self, ix: usize) -> usize {
        (ix + self.count() - 1) % self.count()
    }

    /// Grid neighbour in `dir`, without wrapping.
    pub fn neighbor(self, ix: usize, dir: Dir) -> Option<usize> {
        let (row, col) = self.position(ix);
        let (rows, cols) = (self.rows as usize, self.cols as usize);
        let (r, c) = match dir {
            Dir::Up => (row.checked_sub(1)?, col),
            Dir::Down => (row + 1, col),
            Dir::Left => (row, col.checked_sub(1)?),
            Dir::Right => (row, col + 1),
        };
        (r < rows && c < cols).then(|| self.index(r, c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbors_in_2x3() {
        let l = Layout::new(2, 3);
        assert_eq!(l.count(), 6);
        assert_eq!(l.neighbor(0, Dir::Right), Some(1));
        assert_eq!(l.neighbor(0, Dir::Left), None);
        assert_eq!(l.neighbor(0, Dir::Down), Some(3));
        assert_eq!(l.neighbor(5, Dir::Down), None);
        assert_eq!(l.neighbor(5, Dir::Up), Some(2));
        assert_eq!(l.next(5), 0);
        assert_eq!(l.prev(0), 5);
    }

    #[test]
    fn clamps() {
        assert_eq!(Layout::new(0, 9), Layout { rows: 1, cols: 3 });
    }
}
