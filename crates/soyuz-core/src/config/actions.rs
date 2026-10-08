//! The [`Action`] enum: every palette command / bindable operation (PLAN §2.10).
//!
//! String form (used as `[keys.bindings]` keys and in `[toolbar]`): `snake_case`
//! variant name, payload after `:` — `"copy"`, `"sort_by:name"`, `"open_bookmark:1"`,
//! `"checksum:blake3"`, `"custom_command:Open in VS Code"`.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::columns::{Column, GroupBy};
use crate::progress::HashAlgo;

/// Palette grouping.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum ActionCategory {
    Navigation,
    Tabs,
    View,
    Selection,
    FileOps,
    Tools,
    Jobs,
}

impl ActionCategory {
    pub fn label(self) -> &'static str {
        match self {
            ActionCategory::Navigation => "Navigation",
            ActionCategory::Tabs => "Tabs & panes",
            ActionCategory::View => "View",
            ActionCategory::Selection => "Selection",
            ActionCategory::FileOps => "File",
            ActionCategory::Tools => "Tools",
            ActionCategory::Jobs => "Jobs",
        }
    }
}

macro_rules! define_actions {
    ($( $cat:ident : [ $($v:ident),* $(,)? ] ),* $(,)?) => {
        /// Every palette command / bindable operation.
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum Action {
            $($($v,)*)*
            OpenBookmark(u8),
            SelectTab(u8),
            SortBy(Column),
            SetGroupBy(GroupBy),
            AddTag(u8),
            Checksum(HashAlgo),
            CustomCommand(String),
        }

        /// `(action, CamelCase name, category)` for every unit variant.
        const UNIT_ACTIONS: &[(Action, &str, ActionCategory)] = &[
            $($((Action::$v, stringify!($v), ActionCategory::$cat),)*)*
        ];
    };
}

define_actions! {
    Navigation: [
        Back, Forward, Up, Home, Root, GoTo, FocusAddressBar, EditPath, Refresh, ToggleSidebar,
        FocusSidebar, FocusList, FocusOtherPane, FocusTerminal, NextTab, PrevTab,
        JumpToFolderDialog, OpenRecent,
    ],
    Tabs: [
        NewTab, CloseTab, ReopenClosedTab, DuplicateTab, PinTab, MoveTabLeft, MoveTabRight,
        TabToNewWindow, ToggleDualPane, SwapPanes, MirrorPaneToOther, OpenInOtherPane,
        SyncNavigation, NewWindow, CloseWindow,
    ],
    View: [
        ViewDetails, ViewGrid, ViewList, ViewMiller, ViewGallery, ZoomIn, ZoomOut, ZoomReset,
        UiZoomIn, UiZoomOut, UiZoomReset, ToggleHighContrast,
        ToggleHidden, ToggleFlatView, ToggleSortDir, ChooseColumns, ResetFolderView,
        ApplyViewToAll, TogglePreviewPane, ToggleInfoPane, ToggleTerminal, ToggleActivityPanel,
        ToggleAiPanel, ToggleStatusBar, ToggleTreeSidebar, ToggleCheckboxes,
        ToggleShelf,
    ],
    Selection: [
        SelectAll, SelectNone, InvertSelection, SelectByPattern, SelectSameExtension,
        SelectSameName, MarkToggle, MarkAndMoveDown, CursorUp, CursorDown, CursorPageUp,
        CursorPageDown, CursorHome, CursorEnd, ExtendUp, ExtendDown, Filter, ClearFilter,
        TypeAheadToggle,
    ],
    FileOps: [
        Open, OpenWith, OpenInNewTab, OpenInTerminal, OpenTerminalHere, RunElevated,
        RevealInSystem, Copy, Cut, Paste, PasteAsLink, PasteShortcut, CopyToOtherPane,
        MoveToOtherPane, CopyTo, MoveTo, Duplicate, Rename, BatchRename, NewFolder, NewFile,
        NewSymlink, NewHardlink, NewJunction, Delete, DeletePermanent, SecureWipe, Properties,
        CopyPath, CopyPathPosix, CopyPathQuoted, CopyName, CopyAsFileList, CreateArchive,
        ExtractHere, ExtractToFolder, ExtractTo, TestArchive, CompareChecksum, SetAttributes,
        TouchTimestamps, SplitFile, JoinFiles, ClearTags, Undo, Redo,
    ],
    Tools: [
        CommandPalette, Search, SearchContent, FolderSize, DiskUsageTreemap, FindDuplicates,
        CompareFolders, SyncFolders, Settings, EditConfigFile, ReloadConfig, OpenTrash,
        EmptyTrash, RestoreFromTrash, Computer, ShowKeybindings, About, Welcome, Quit, QuickLook,
        QuickLookNext, QuickLookPrev, ToggleHexPreview,
        AiOrganize, AiSuggestNames, AiSummarize, AiExplainFolder, AiFindRelated, AiSmartSearch,
        AiAuditLog, AiRecipes, AiStop,
    ],
    Jobs: [
        JobsPauseAll, JobsResumeAll, JobsCancelAll, JobsClearFinished, JobsShowHistory,
    ],
}

/// Number of bookmark / tab / tag slots enumerated by [`Action::all`].
pub const BOOKMARK_SLOTS: u8 = 9;
pub const TAB_SLOTS: u8 = 9;
pub const TAG_SLOTS: u8 = 8;

/// Same conversion as serde's `rename_all = "snake_case"`.
fn snake_case(camel: &str) -> String {
    let mut out = String::with_capacity(camel.len() + 4);
    for (i, ch) in camel.char_indices() {
        if ch.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(ch.to_lowercase());
    }
    out
}

struct Names {
    by_action: HashMap<Action, (String, ActionCategory)>,
    by_name: HashMap<String, Action>,
}

fn names() -> &'static Names {
    static NAMES: OnceLock<Names> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut by_action = HashMap::new();
        let mut by_name = HashMap::new();
        for (a, camel, cat) in UNIT_ACTIONS {
            let s = snake_case(camel);
            by_name.insert(s.clone(), a.clone());
            by_action.insert(a.clone(), (s, *cat));
        }
        Names { by_action, by_name }
    })
}

impl Action {
    /// Every unit variant plus payload variants over their finite domains
    /// (bookmarks 1..=9, tabs 1..=9, tags 1..=8, all columns / groupings / hashes).
    /// `CustomCommand` is not included (its domain comes from the config).
    pub fn all() -> Vec<Action> {
        let mut v: Vec<Action> = UNIT_ACTIONS.iter().map(|(a, _, _)| a.clone()).collect();
        v.extend((1..=BOOKMARK_SLOTS).map(Action::OpenBookmark));
        v.extend((1..=TAB_SLOTS).map(Action::SelectTab));
        v.extend(Column::ALL.into_iter().map(Action::SortBy));
        v.extend(GroupBy::ALL.into_iter().map(Action::SetGroupBy));
        v.extend((1..=TAG_SLOTS).map(Action::AddTag));
        v.extend(HashAlgo::ALL.into_iter().map(Action::Checksum));
        v
    }

    /// The `snake_case` variant name without payload (`"sort_by"`).
    pub fn name(&self) -> &str {
        match self {
            Action::OpenBookmark(_) => "open_bookmark",
            Action::SelectTab(_) => "select_tab",
            Action::SortBy(_) => "sort_by",
            Action::SetGroupBy(_) => "set_group_by",
            Action::AddTag(_) => "add_tag",
            Action::Checksum(_) => "checksum",
            Action::CustomCommand(_) => "custom_command",
            unit => names().by_action.get(unit).map(|(s, _)| s.as_str()).unwrap_or(""),
        }
    }

    pub fn category(&self) -> ActionCategory {
        match self {
            Action::OpenBookmark(_) | Action::SelectTab(_) => ActionCategory::Navigation,
            Action::SortBy(_) | Action::SetGroupBy(_) => ActionCategory::View,
            Action::AddTag(_) | Action::Checksum(_) => ActionCategory::FileOps,
            Action::CustomCommand(_) => ActionCategory::Tools,
            unit => names().by_action.get(unit).map(|(_, c)| *c).unwrap_or(ActionCategory::Tools),
        }
    }

    /// Human title for the palette, e.g. `"Copy path posix"`, `"Sort by: Size"`.
    pub fn title(&self) -> String {
        let base = {
            let n = self.name().replace('_', " ");
            let mut c = n.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        };
        match self {
            Action::OpenBookmark(n) | Action::SelectTab(n) | Action::AddTag(n) => format!("{base} {n}"),
            Action::SortBy(c) => format!("{base}: {}", c.label()),
            Action::SetGroupBy(g) => format!("{base}: {g}"),
            Action::Checksum(h) => format!("{base}: {}", h.label()),
            Action::CustomCommand(s) => s.clone(),
            _ => base,
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())?;
        match self {
            Action::OpenBookmark(n) | Action::SelectTab(n) | Action::AddTag(n) => write!(f, ":{n}"),
            Action::SortBy(c) => write!(f, ":{c}"),
            Action::SetGroupBy(g) => write!(f, ":{g}"),
            Action::Checksum(h) => write!(f, ":{h}"),
            Action::CustomCommand(s) => write!(f, ":{s}"),
            _ => Ok(()),
        }
    }
}

/// Error parsing an action string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown action \"{0}\"")]
pub struct ParseActionError(pub String);

impl FromStr for Action {
    type Err = ParseActionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseActionError(s.to_string());
        let (name, payload) = match s.split_once(':') {
            Some((n, p)) => (n.trim().to_ascii_lowercase(), Some(p)),
            None => (s.trim().to_ascii_lowercase(), None),
        };
        let name = name.replace('-', "_");
        let slot = |p: Option<&str>, max: u8| -> Result<u8, ParseActionError> {
            let n: u8 = p.ok_or_else(err)?.trim().parse().map_err(|_| err())?;
            if (1..=max).contains(&n) { Ok(n) } else { Err(err()) }
        };
        match name.as_str() {
            "open_bookmark" => Ok(Action::OpenBookmark(slot(payload, BOOKMARK_SLOTS)?)),
            "select_tab" => Ok(Action::SelectTab(slot(payload, TAB_SLOTS)?)),
            "add_tag" => Ok(Action::AddTag(slot(payload, TAG_SLOTS)?)),
            "sort_by" => payload.ok_or_else(err)?.parse().map(Action::SortBy).map_err(|_| err()),
            "set_group_by" => payload.ok_or_else(err)?.parse().map(Action::SetGroupBy).map_err(|_| err()),
            "checksum" => payload.ok_or_else(err)?.parse().map(Action::Checksum).map_err(|_| err()),
            "custom_command" => match payload {
                Some(p) if !p.trim().is_empty() => Ok(Action::CustomCommand(p.trim().to_string())),
                _ => Err(err()),
            },
            unit if payload.is_none() => names().by_name.get(unit).cloned().ok_or_else(err),
            _ => Err(err()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn string_round_trip_all() {
        let all = Action::all();
        assert!(all.len() > 200, "{}", all.len());
        let mut seen = HashSet::new();
        for a in &all {
            let s = a.to_string();
            assert!(seen.insert(s.clone()), "duplicate string {s}");
            assert_eq!(s.parse::<Action>().unwrap(), *a, "{s}");
            assert!(!a.title().is_empty());
        }
        let cc = Action::CustomCommand("Open in VS Code".into());
        assert_eq!(cc.to_string(), "custom_command:Open in VS Code");
        assert_eq!("custom_command:Open in VS Code".parse::<Action>().unwrap(), cc);
    }

    #[test]
    fn specific_strings() {
        assert_eq!(Action::CopyPathPosix.to_string(), "copy_path_posix");
        assert_eq!(Action::SortBy(Column::Name).to_string(), "sort_by:name");
        assert_eq!(Action::OpenBookmark(1).to_string(), "open_bookmark:1");
        assert_eq!(Action::Checksum(HashAlgo::Blake3).to_string(), "checksum:blake3");
        assert_eq!("Copy-To-Other-Pane".parse::<Action>().unwrap(), Action::CopyToOtherPane);
        assert!("open_bookmark:0".parse::<Action>().is_err());
        assert!("sort_by".parse::<Action>().is_err());
        assert!("copy:1".parse::<Action>().is_err());
        assert!("nope".parse::<Action>().is_err());
        assert_eq!(Action::Copy.category(), ActionCategory::FileOps);
        assert_eq!(Action::SortBy(Column::Size).title(), "Sort by: Size");
    }

    #[test]
    fn serde_names_match_string_form() {
        // Unit variants: serde name == string form.
        for (a, _, _) in UNIT_ACTIONS {
            let v = toml::Value::try_from(a).unwrap();
            assert_eq!(v.as_str().unwrap(), a.to_string());
        }
    }
}
