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
            ViewMode::Full => t!("Full"),
            ViewMode::Brief => t!("Brief"),
            ViewMode::Thumbnails => t!("Thumbnails"),
        }
    }

    /// The label in the panel's context menu.
    pub fn menu_label(self) -> &'static str {
        match self {
            ViewMode::Full => t!("Full view"),
            ViewMode::Brief => t!("Brief view"),
            ViewMode::Thumbnails => t!("Thumbnails view"),
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

/// Where a panel shows the preview of the entry under the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewMode {
    #[default]
    Off,
    /// Under the list.
    Bottom,
    /// Beside the list.
    Right,
}

impl PreviewMode {
    pub const ALL: [PreviewMode; 3] = [PreviewMode::Off, PreviewMode::Bottom, PreviewMode::Right];

    /// The label in the view menus.
    pub fn menu_label(self) -> &'static str {
        match self {
            PreviewMode::Off => t!("No preview"),
            PreviewMode::Bottom => t!("Preview at the bottom"),
            PreviewMode::Right => t!("Preview on the right"),
        }
    }

    /// Off → bottom → right → off (Ctrl+Shift+Q).
    pub fn next(self) -> Self {
        match self {
            PreviewMode::Off => PreviewMode::Bottom,
            PreviewMode::Bottom => PreviewMode::Right,
            PreviewMode::Right => PreviewMode::Off,
        }
    }
}

/// Which columns the full view shows besides the name, and how wide.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Columns {
    pub size: bool,
    pub kind: bool,
    pub modified: bool,
    pub created: bool,
    pub permissions: bool,
    pub owner: bool,
    #[serde(skip_serializing_if = "ColumnWidths::is_auto")]
    pub widths: ColumnWidths,
}

/// Widths the user dragged the columns to, in rems; `None` is the default.
/// While the name has no width it fills the rest of the panel; once it has
/// one, the list keeps its width and scrolls sideways when it is too wide.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ColumnWidths {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<f32>,
}

impl ColumnWidths {
    pub fn is_auto(&self) -> bool {
        *self == Self::default()
    }

    /// The width of `column`; `None` is the name column.
    pub fn get(&self, column: Option<Column>) -> Option<f32> {
        *self.slot(column)
    }

    /// Set the width of `column` (`None` is the name column), within limits.
    pub fn set(&mut self, column: Option<Column>, width: Option<f32>) {
        let min = match column {
            None => NAME_WIDTH_MIN_REMS,
            Some(_) => COLUMN_WIDTH_MIN_REMS,
        };
        *self.slot_mut(column) = width
            .filter(|w| w.is_finite())
            .map(|w| w.clamp(min, WIDTH_MAX_REMS));
    }

    fn slot(&self, column: Option<Column>) -> &Option<f32> {
        match column {
            None => &self.name,
            Some(Column::Size) => &self.size,
            Some(Column::Kind) => &self.kind,
            Some(Column::Modified) => &self.modified,
            Some(Column::Created) => &self.created,
            Some(Column::Permissions) => &self.permissions,
            Some(Column::Owner) => &self.owner,
        }
    }

    fn slot_mut(&mut self, column: Option<Column>) -> &mut Option<f32> {
        match column {
            None => &mut self.name,
            Some(Column::Size) => &mut self.size,
            Some(Column::Kind) => &mut self.kind,
            Some(Column::Modified) => &mut self.modified,
            Some(Column::Created) => &mut self.created,
            Some(Column::Permissions) => &mut self.permissions,
            Some(Column::Owner) => &mut self.owner,
        }
    }
}

/// The narrowest a dragged column gets, in rems.
pub const COLUMN_WIDTH_MIN_REMS: f32 = 2.0;
/// The narrowest a dragged name column gets, with its icon, in rems.
pub const NAME_WIDTH_MIN_REMS: f32 = 6.0;
/// The widest any column gets, in rems.
pub const WIDTH_MAX_REMS: f32 = 200.0;

impl Default for Columns {
    fn default() -> Self {
        Self {
            size: true,
            kind: true,
            modified: true,
            created: false,
            permissions: false,
            owner: false,
            widths: ColumnWidths::default(),
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

    /// The width of `column`: the dragged one, or its default.
    pub fn width(&self, column: Column) -> f32 {
        self.widths
            .get(Some(column))
            .unwrap_or_else(|| column.width_rems())
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
            Column::Size => t!("Size"),
            Column::Kind => t!("Type"),
            Column::Modified => t!("Modified"),
            Column::Created => t!("Created"),
            Column::Permissions => t!("Permissions"),
            Column::Owner => t!("Owner"),
        }
    }

    /// The default width.
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
    use super::{Column, ColumnWidths, Columns, PreviewMode, ViewMode};

    #[test]
    fn preview_cycles_off_bottom_right() {
        let mut mode = PreviewMode::default();
        let mut seen = Vec::new();
        for _ in 0..3 {
            seen.push(mode);
            mode = mode.next();
        }
        assert_eq!(seen, PreviewMode::ALL);
        assert_eq!(mode, PreviewMode::Off);
    }

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

    #[test]
    fn widths_are_clamped_and_reset() {
        let mut c = Columns::default();
        assert_eq!(c.width(Column::Size), Column::Size.width_rems());
        c.widths.set(Some(Column::Size), Some(0.5));
        c.widths.set(None, Some(f32::NAN));
        assert_eq!(c.width(Column::Size), super::COLUMN_WIDTH_MIN_REMS);
        assert_eq!(c.widths.get(None), None);
        c.widths.set(None, Some(30.0));
        assert_eq!(c.widths.get(None), Some(30.0));
        assert!(!c.widths.is_auto());
        c.widths.set(Some(Column::Size), None);
        c.widths.set(None, None);
        assert!(c.widths.is_auto());
    }

    #[test]
    fn widths_are_saved_only_when_dragged() {
        let mut c = Columns::default();
        assert!(!toml::to_string(&c).unwrap().contains("widths"));
        c.widths.set(Some(Column::Kind), Some(7.0));
        let text = toml::to_string(&c).unwrap();
        assert!(text.contains("kind = 7.0"), "{text}");
        let back: Columns = toml::from_str(&text).unwrap();
        assert_eq!(back, c);
        // A config from before the widths.
        let old: Columns = toml::from_str("size = false").unwrap();
        assert!(old.widths.is_auto() && !old.size);
        assert_eq!(old.widths, ColumnWidths::default());
    }
}
