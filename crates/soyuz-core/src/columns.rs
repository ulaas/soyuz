//! Columns, grouping and view modes.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A details-view column (also the sort key).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Column {
    Name,
    Size,
    Kind,
    Modified,
    Created,
    Accessed,
    Attributes,
    Mode,
    Owner,
    Extension,
    Path,
    Tags,
    GitStatus,
    Dimensions,
    Duration,
    LinkTarget,
    Checksum,
    ItemCount,
    /// Values computed by script column providers, in provider order.
    Script1,
    Script2,
    Script3,
    Script4,
}

impl Column {
    pub const ALL: [Column; 22] = [
        Column::Name,
        Column::Size,
        Column::Kind,
        Column::Modified,
        Column::Created,
        Column::Accessed,
        Column::Attributes,
        Column::Mode,
        Column::Owner,
        Column::Extension,
        Column::Path,
        Column::Tags,
        Column::GitStatus,
        Column::Dimensions,
        Column::Duration,
        Column::LinkTarget,
        Column::Checksum,
        Column::ItemCount,
        Column::Script1,
        Column::Script2,
        Column::Script3,
        Column::Script4,
    ];

    /// The script-provided columns (labels come from the provider in the app).
    pub const SCRIPT: [Column; 4] = [Column::Script1, Column::Script2, Column::Script3, Column::Script4];

    /// Slot of a script column (0-based).
    pub fn script_slot(self) -> Option<usize> {
        Column::SCRIPT.iter().position(|c| *c == self)
    }

    /// Header label.
    pub fn label(self) -> &'static str {
        match self {
            Column::Name => "Name",
            Column::Size => "Size",
            Column::Kind => "Kind",
            Column::Modified => "Modified",
            Column::Created => "Created",
            Column::Accessed => "Accessed",
            Column::Attributes => "Attributes",
            Column::Mode => "Permissions",
            Column::Owner => "Owner",
            Column::Extension => "Extension",
            Column::Path => "Path",
            Column::Tags => "Tags",
            Column::GitStatus => "Git",
            Column::Dimensions => "Dimensions",
            Column::Duration => "Duration",
            Column::LinkTarget => "Link target",
            Column::Checksum => "Checksum",
            Column::ItemCount => "Items",
            Column::Script1 => "Script 1",
            Column::Script2 => "Script 2",
            Column::Script3 => "Script 3",
            Column::Script4 => "Script 4",
        }
    }

    /// Default width in points.
    pub fn default_width(self) -> f32 {
        match self {
            Column::Name => 320.0,
            Column::Size | Column::ItemCount | Column::GitStatus => 90.0,
            Column::Kind | Column::Mode | Column::Owner | Column::Dimensions => 120.0,
            Column::Modified | Column::Created | Column::Accessed => 150.0,
            Column::Path | Column::LinkTarget | Column::Checksum => 260.0,
            _ => 100.0,
        }
    }
}

/// Width/visibility of one column.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColumnSpec {
    pub column: Column,
    pub width: f32,
    pub visible: bool,
}

impl ColumnSpec {
    pub fn new(column: Column) -> Self {
        ColumnSpec { column, width: column.default_width(), visible: true }
    }
}

/// Grouping of list entries.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, PartialOrd, Ord, Default)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    #[default]
    None,
    Kind,
    Date,
    Size,
    Extension,
    Tag,
    FirstLetter,
}

impl GroupBy {
    pub const ALL: [GroupBy; 7] = [
        GroupBy::None,
        GroupBy::Kind,
        GroupBy::Date,
        GroupBy::Size,
        GroupBy::Extension,
        GroupBy::Tag,
        GroupBy::FirstLetter,
    ];
}

/// Pane view mode (moved to core so `[general] default_view` can be typed; the app
/// re-exports it).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ViewMode {
    #[default]
    Details,
    Grid,
    List,
    Miller,
    Gallery,
}

macro_rules! enum_string_impls {
    ($($t:ty => $what:literal),* $(,)?) => {$(
        impl fmt::Display for $t {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&crate::enum_str::to_str(self))
            }
        }
        impl FromStr for $t {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                crate::enum_str::from_str(s).ok_or_else(|| format!(concat!("unknown ", $what, ": {}"), s))
            }
        }
    )*};
}

enum_string_impls!(Column => "column", GroupBy => "group", ViewMode => "view mode");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_forms() {
        for c in Column::ALL {
            assert_eq!(c.to_string().parse::<Column>().unwrap(), c);
        }
        assert_eq!(Column::GitStatus.to_string(), "git_status");
        assert_eq!("item-count".parse::<Column>().unwrap(), Column::ItemCount);
        for g in GroupBy::ALL {
            assert_eq!(g.to_string().parse::<GroupBy>().unwrap(), g);
        }
        assert_eq!(GroupBy::FirstLetter.to_string(), "first_letter");
        assert_eq!("details".parse::<ViewMode>().unwrap(), ViewMode::Details);
        assert!("nope".parse::<Column>().is_err());
    }
}
