//! How a panel shows its entries: the full list with columns, a brief
//! multi-column list of names, or a grid of thumbnails. Also which optional
//! columns the full view shows.

use gpui_kit::component::IconName;
use serde::{Deserialize, Serialize};

use crate::fs::SortKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    /// One entry per row with columns.
    #[default]
    Full,
    /// Names only, several per row.
    Brief,
    /// A grid of image thumbnails (icons for everything else).
    Thumbnails,
}

impl ViewMode {
    pub const ALL: [ViewMode; 3] = [ViewMode::Full, ViewMode::Brief, ViewMode::Thumbnails];

    pub fn label(self) -> &'static str {
        match self {
            ViewMode::Full => "Full",
            ViewMode::Brief => "Brief",
            ViewMode::Thumbnails => "Thumbnails",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            ViewMode::Full => IconName::Menu,
            ViewMode::Brief => IconName::GalleryVerticalEnd,
            ViewMode::Thumbnails => IconName::LayoutDashboard,
        }
    }

    /// Entries are laid out in a grid (several per row).
    pub fn is_grid(self) -> bool {
        self != ViewMode::Full
    }
}

/// Which columns the full view shows besides the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Columns {
    pub size: bool,
    pub kind: bool,
    pub modified: bool,
    pub created: bool,
    pub permissions: bool,
    pub owner: bool,
}

impl Default for Columns {
    fn default() -> Self {
        Self {
            size: true,
            kind: true,
            modified: true,
            created: false,
            permissions: false,
            owner: false,
        }
    }
}

impl Columns {
    pub fn get(&self, column: Column) -> bool {
        match column {
            Column::Size => self.size,
            Column::Kind => self.kind,
            Column::Modified => self.modified,
            Column::Created => self.created,
            Column::Permissions => self.permissions,
            Column::Owner => self.owner,
        }
    }

    pub fn toggle(&mut self, column: Column) {
        let flag = match column {
            Column::Size => &mut self.size,
            Column::Kind => &mut self.kind,
            Column::Modified => &mut self.modified,
            Column::Created => &mut self.created,
            Column::Permissions => &mut self.permissions,
            Column::Owner => &mut self.owner,
        };
        *flag = !*flag;
    }

    /// The columns shown, in display order.
    pub fn shown(&self) -> impl Iterator<Item = Column> + '_ {
        Column::ALL
            .into_iter()
            .filter(|c| c.available() && self.get(*c))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Column {
    Size,
    Kind,
    Modified,
    Created,
    Permissions,
    Owner,
}

impl Column {
    pub const ALL: [Column; 6] = [
        Column::Size,
        Column::Kind,
        Column::Modified,
        Column::Created,
        Column::Permissions,
        Column::Owner,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Column::Size => "Size",
            Column::Kind => "Type",
            Column::Modified => "Modified",
            Column::Created => "Created",
            Column::Permissions => "Permissions",
            Column::Owner => "Owner",
        }
    }

    pub fn width_rems(self) -> f32 {
        match self {
            Column::Size => 5.0,
            Column::Kind => 4.0,
            Column::Modified | Column::Created => 8.5,
            Column::Permissions => 6.5,
            Column::Owner => 6.0,
        }
    }

    pub fn right_aligned(self) -> bool {
        self == Column::Size
    }

    /// Clicking the header sorts by this key, when there is one.
    pub fn sort_key(self) -> Option<SortKey> {
        match self {
            Column::Size => Some(SortKey::Size),
            Column::Kind => Some(SortKey::Type),
            Column::Modified => Some(SortKey::Modified),
            Column::Created => Some(SortKey::Created),
            Column::Permissions | Column::Owner => None,
        }
    }

    /// Permissions and owners are Unix things.
    pub fn available(self) -> bool {
        match self {
            Column::Permissions | Column::Owner => cfg!(unix),
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Column, Columns, ViewMode};

    #[test]
    fn default_columns_and_toggle() {
        let mut c = Columns::default();
        assert!(c.get(Column::Size) && c.get(Column::Modified) && !c.get(Column::Created));
        c.toggle(Column::Created);
        c.toggle(Column::Size);
        let shown: Vec<_> = c.shown().collect();
        assert_eq!(shown, [Column::Kind, Column::Modified, Column::Created]);
        assert!(ViewMode::Brief.is_grid() && !ViewMode::Full.is_grid());
    }
}
