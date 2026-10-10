//! Palette metadata for every [`Action`]: icon, translated title and one-line
//! description, and extra (English) search keywords. `Action::all()` is covered
//! exhaustively (unit-tested, in both languages).
//!
//! Titles and descriptions live in `locales/<lang>/actions.ftl` as `action-<name>` /
//! `action-<name>-desc`, `<name>` being the action's config name with `-` for `_`.

use soyuz_core::config::{Action, ActionCategory};
use soyuz_core::{Column, GroupBy, Locale};

use crate::i18n::{FluentArgs, t_args_in, t_in};
use crate::theme::icons::ph;

/// Everything the palette shows for an action.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub title: String,
    /// The English title: the palette matches it too, so English muscle memory keeps
    /// working in another UI language.
    pub title_en: String,
    pub icon: &'static str,
    pub description: String,
    /// English search keywords.
    pub keywords: &'static str,
    pub category: ActionCategory,
}

/// [`entry_in`] the current UI language.
pub fn entry(a: &Action) -> Entry {
    entry_in(a, crate::i18n::current())
}

pub fn entry_in(a: &Action, l: Locale) -> Entry {
    let (icon, keywords) = meta(a);
    Entry {
        title: title_in(a, l),
        title_en: title_in(a, Locale::EnUs),
        icon,
        description: description_in(a, l),
        keywords,
        category: a.category(),
    }
}

/// Palette title in the current UI language.
pub fn title(a: &Action) -> String {
    title_in(a, crate::i18n::current())
}

/// `action-<name>`: the base key of an action's strings.
fn key_of(a: &Action) -> String {
    format!("action-{}", a.name().replace('_', "-"))
}

fn with_arg(key: &str, name: &'static str, value: String, l: Locale) -> String {
    let mut args = FluentArgs::new();
    args.set(name, value);
    t_args_in(key, &args, l)
}

pub fn title_in(a: &Action, l: Locale) -> String {
    use Action as A;
    match a {
        // Slot numbers are identifiers: passed as strings (no digit grouping).
        A::OpenBookmark(n) => with_arg("action-open-bookmark", "n", n.to_string(), l),
        A::SelectTab(9) => t_in("action-select-tab-last", l).to_string(),
        A::SelectTab(n) => with_arg("action-select-tab", "n", n.to_string(), l),
        A::OpenWorkspace(n) => with_arg("action-open-workspace", "n", n.to_string(), l),
        A::AddTag(n) => with_arg("action-add-tag", "tag", tag_name_in(*n, l), l),
        A::SetGroupBy(g) => with_arg("action-set-group-by", "group", group_label_in(*g, l).to_string(), l),
        A::SortBy(c) => with_arg("action-sort-by", "column", column_label_in(*c, l).to_string(), l),
        A::Checksum(h) => with_arg("action-checksum", "algo", h.label().to_string(), l),
        A::CustomCommand(name) => name.clone(),
        A::RunScript(id) => crate::automation::action_title(id),
        // i18n-dynamic: action-
        _ => t_in(&key_of(a), l).to_string(),
    }
}

/// One-line description in `l`.
pub fn description_in(a: &Action, l: Locale) -> String {
    // i18n-dynamic: action-
    t_in(&format!("{}-desc", key_of(a)), l).to_string()
}

/// Column name in `l`: the core labels are the English source; `labels::column` follows
/// the current language (Turkish in practice).
fn column_label_in(c: Column, l: Locale) -> &'static str {
    if l == Locale::EnUs { c.label() } else { crate::i18n::labels::column(c) }
}

/// Name of color tag `n` (1-based) in the current language (or the user's name for it).
pub fn tag_name(n: u8) -> String {
    tag_name_in(n, crate::i18n::current())
}

/// The user's name for color tag `n` (1-based) from `[tags] names`, else its color
/// name in `l`.
pub fn tag_name_in(n: u8, l: Locale) -> String {
    if let Some(custom) = n.checked_sub(1).and_then(crate::services::tags::custom_name) {
        return custom;
    }
    let key = match n {
        1 => "actions-tag-red",
        2 => "actions-tag-orange",
        3 => "actions-tag-yellow",
        4 => "actions-tag-green",
        5 => "actions-tag-cyan",
        6 => "actions-tag-blue",
        7 => "actions-tag-purple",
        8 => "actions-tag-gray",
        _ => "actions-tag-other",
    };
    t_in(key, l).to_string()
}

/// Group-by choice in the current language.
pub fn group_label(g: GroupBy) -> &'static str {
    group_label_in(g, crate::i18n::current())
}

pub fn group_label_in(g: GroupBy, l: Locale) -> &'static str {
    let key = match g {
        GroupBy::None => "actions-group-none",
        GroupBy::Kind => "actions-group-kind",
        GroupBy::Date => "actions-group-date",
        GroupBy::Size => "actions-group-size",
        GroupBy::Extension => "actions-group-extension",
        GroupBy::Tag => "actions-group-tag",
        GroupBy::FirstLetter => "actions-group-first-letter",
    };
    t_in(key, l)
}

fn column_icon(c: Column) -> &'static str {
    match c {
        Column::Name => ph::TEXT_AA,
        Column::Size => ph::HARD_DRIVES,
        Column::Modified | Column::Created | Column::Accessed => ph::CALENDAR,
        Column::Kind | Column::Extension => ph::FILE,
        Column::Tags => ph::TAG,
        Column::GitStatus => ph::GIT_BRANCH,
        _ => ph::SORT_ASCENDING,
    }
}

/// Icon and English search keywords.
fn meta(a: &Action) -> (&'static str, &'static str) {
    use Action as A;
    match a {
        // ---- navigation ----
        A::Back => (ph::ARROW_LEFT, "previous history"),
        A::Forward => (ph::ARROW_RIGHT, "next history"),
        A::Up => (ph::ARROW_UP, "parent"),
        A::Home => (ph::HOUSE, "user profile"),
        A::Root => (ph::HARD_DRIVE, "drive"),
        A::GoTo => (ph::ARROW_SQUARE_RIGHT, "address location"),
        A::FocusAddressBar => (ph::TEXTBOX, "omnibar location"),
        A::EditPath => (ph::PENCIL_LINE, "address location"),
        A::Refresh => (ph::ARROW_CLOCKWISE, "reload rescan"),
        A::ToggleSidebar => (ph::SIDEBAR, "panel"),
        A::FocusSidebar => (ph::SIDEBAR, ""),
        A::FocusList => (ph::LIST, ""),
        A::FocusOtherPane => (ph::SQUARE_SPLIT_HORIZONTAL, "dual"),
        A::FocusTerminal => (ph::TERMINAL_WINDOW, "shell console"),
        A::NextTab => (ph::ARROW_FAT_RIGHT, ""),
        A::PrevTab => (ph::ARROW_FAT_LEFT, ""),
        A::JumpToFolderDialog => (ph::FOLDER_OPEN, "frecency recent jump"),
        A::OpenRecent => (ph::CLOCK_COUNTER_CLOCKWISE, "history"),
        A::ConnectToServer => (ph::GLOBE, "remote network ssh cloud bucket"),
        A::OpenBookmark(_) => (ph::BOOKMARK_SIMPLE, "favorite"),
        A::SelectTab(_) => (ph::BROWSERS, ""),
        A::OpenTimeline => (ph::CALENDAR_DOTS, "recent modified changes week"),
        A::TabHistory => (ph::GIT_FORK, "history graph where was i"),
        // ---- tabs & panes ----
        A::NewTab => (ph::PLUS_SQUARE, ""),
        A::CloseTab => (ph::X_SQUARE, ""),
        A::ReopenClosedTab => (ph::ARROW_COUNTER_CLOCKWISE, "restore undo"),
        A::DuplicateTab => (ph::COPY, "clone"),
        A::PinTab => (ph::PUSH_PIN, ""),
        A::MoveTabLeft => (ph::CARET_DOUBLE_LEFT, "reorder"),
        A::MoveTabRight => (ph::CARET_DOUBLE_RIGHT, "reorder"),
        A::TabToNewWindow => (ph::APP_WINDOW, ""),
        A::ToggleDualPane => (ph::SQUARE_SPLIT_HORIZONTAL, "split commander"),
        A::SwapPanes => (ph::ARROWS_LEFT_RIGHT, "exchange"),
        A::MirrorPaneToOther => (ph::COPY_SIMPLE, "same"),
        A::OpenInOtherPane => (ph::ARROW_SQUARE_OUT, ""),
        A::SyncNavigation => (ph::LINK_SIMPLE, "lock"),
        A::NewWindow => (ph::APP_WINDOW, ""),
        A::CloseWindow => (ph::X, ""),
        A::ToggleQuadPane => (ph::SQUARES_FOUR, "layout grid split quad"),
        A::NewTabGroup => (ph::RECTANGLE_DASHED, "group color"),
        A::UngroupTab => (ph::RECTANGLE, "group"),
        A::MergeWindows => (ph::BROWSERS, "windows combine"),
        A::Workspaces => (ph::STACK, "layout session"),
        A::SaveWorkspace => (ph::FLOPPY_DISK, "layout session"),
        A::UpdateWorkspace => (ph::ARROWS_CLOCKWISE, "layout"),
        A::OpenWorkspace(_) => (ph::STACK, "layout"),
        // ---- view ----
        A::ViewDetails => (ph::LIST, "table columns"),
        A::ViewGrid => (ph::SQUARES_FOUR, "icons thumbnails"),
        A::ViewList => (ph::LIST_DASHES, ""),
        A::ViewMiller => (ph::COLUMNS, "finder miller"),
        A::ViewGallery => (ph::IMAGES, "photos"),
        A::ZoomIn => (ph::MAGNIFYING_GLASS_PLUS, "bigger"),
        A::ZoomOut => (ph::MAGNIFYING_GLASS_MINUS, "smaller"),
        A::ZoomReset => (ph::MAGNIFYING_GLASS, "zoom"),
        A::UiZoomIn => (ph::MAGNIFYING_GLASS_PLUS, "ui scale bigger accessibility"),
        A::UiZoomOut => (ph::MAGNIFYING_GLASS_MINUS, "ui scale smaller"),
        A::UiZoomReset => (ph::MAGNIFYING_GLASS, "ui scale"),
        A::ToggleHighContrast => (ph::CIRCLE_HALF, "accessibility a11y"),
        A::ToggleHidden => (ph::EYE_SLASH, "dotfiles"),
        A::ToggleFlatView => (ph::TREE_STRUCTURE, "recursive flatten"),
        A::ToggleSortDir => (ph::ARROWS_DOWN_UP, "sort"),
        A::ChooseColumns => (ph::COLUMNS, ""),
        A::ResetFolderView => (ph::ARROW_U_UP_LEFT, ""),
        A::ApplyViewToAll => (ph::STACK, "default"),
        A::TogglePreviewPane => (ph::SIDEBAR_SIMPLE, "inspector"),
        A::ToggleShelf => (ph::TRAY, "stash basket collect"),
        A::ToggleInfoPane => (ph::INFO, "metadata"),
        A::ToggleTerminal => (ph::TERMINAL_WINDOW, "shell console"),
        A::ToggleActivityPanel => (ph::PULSE, "progress operations"),
        A::ToggleStatusBar => (ph::RECTANGLE, ""),
        A::ToggleTreeSidebar => (ph::TREE_VIEW, ""),
        A::ToggleCheckboxes => (ph::CHECK_SQUARE, "touch"),
        A::SortBy(c) => (column_icon(*c), "order"),
        A::SetGroupBy(_) => (ph::ROWS, ""),
        // ---- selection ----
        A::SelectAll => (ph::SELECTION_ALL, ""),
        A::SelectNone => (ph::SELECTION_SLASH, "deselect"),
        A::InvertSelection => (ph::SELECTION_INVERSE, ""),
        A::SelectByPattern => (ph::SELECTION_PLUS, "wildcard"),
        A::SelectSameExtension => (ph::FILE_MAGNIFYING_GLASS, "type"),
        A::SelectSameName => (ph::TEXT_AA, ""),
        A::MarkToggle => (ph::CHECK_SQUARE_OFFSET, ""),
        A::MarkAndMoveDown => (ph::CHECK_SQUARE_OFFSET, ""),
        A::CursorUp => (ph::CARET_UP, ""),
        A::CursorDown => (ph::CARET_DOWN, ""),
        A::CursorPageUp => (ph::CARET_DOUBLE_UP, ""),
        A::CursorPageDown => (ph::CARET_DOUBLE_DOWN, ""),
        A::CursorHome => (ph::ARROW_LINE_UP, "top"),
        A::CursorEnd => (ph::ARROW_LINE_DOWN, "bottom"),
        A::ExtendUp => (ph::CARET_UP, ""),
        A::ExtendDown => (ph::CARET_DOWN, ""),
        A::Filter => (ph::FUNNEL, "find"),
        A::ClearFilter => (ph::FUNNEL_X, ""),
        A::TypeAheadToggle => (ph::KEYBOARD, ""),
        // ---- file ops ----
        A::Open => (ph::ARROW_SQUARE_OUT, "launch enter"),
        A::OpenWith => (ph::APP_WINDOW, "program"),
        A::ToggleEditorPanel => (ph::NOTE_PENCIL, "edit text hex image"),
        A::OpenInEditor => (ph::NOTE_PENCIL, "edit hex image notepad"),
        A::OpenInNewTab => (ph::BROWSERS, ""),
        A::OpenInTerminal => (ph::TERMINAL_WINDOW, "shell"),
        A::OpenTerminalHere => (ph::TERMINAL_WINDOW, "shell console"),
        A::RunElevated => (ph::SHIELD_CHECK, "admin sudo"),
        A::RevealInSystem => (ph::ARROW_SQUARE_UP_RIGHT, "explorer finder"),
        A::Copy => (ph::COPY, ""),
        A::Cut => (ph::SCISSORS, "move"),
        A::Paste => (ph::CLIPBOARD, ""),
        A::PasteAsLink => (ph::LINK, "symlink"),
        A::PasteShortcut => (ph::LINK_SIMPLE, "lnk"),
        A::PasteAndVerify => (ph::SEAL_CHECK, "verify checksum copy"),
        A::CopyToOtherPane => (ph::COPY_SIMPLE, "f5"),
        A::MoveToOtherPane => (ph::ARROW_BEND_DOWN_RIGHT, "f6"),
        A::CopyTo => (ph::COPY, ""),
        A::MoveTo => (ph::ARROW_BEND_UP_RIGHT, ""),
        A::Duplicate => (ph::COPY_SIMPLE, "clone"),
        A::Rename => (ph::PENCIL_SIMPLE, "f2"),
        A::BatchRename => (ph::TEXT_COLUMNS, "bulk regex"),
        A::NewFolder => (ph::FOLDER_PLUS, "mkdir directory"),
        A::NewFile => (ph::FILE_PLUS, "touch"),
        A::NewSymlink => (ph::LINK, "shortcut"),
        A::NewHardlink => (ph::LINK_BREAK, ""),
        A::NewJunction => (ph::GIT_FORK, "mount"),
        A::Delete => (ph::TRASH, "remove recycle"),
        A::DeletePermanent => (ph::X_CIRCLE, "remove erase"),
        A::SecureWipe => (ph::FIRE, "shred"),
        A::Properties => (ph::INFO, "info details"),
        A::CopyPath => (ph::CLIPBOARD_TEXT, ""),
        A::CopyPathPosix => (ph::CLIPBOARD_TEXT, "unix"),
        A::CopyPathQuoted => (ph::CLIPBOARD_TEXT, ""),
        A::CopyName => (ph::TEXT_AA, ""),
        A::CopyAsFileList => (ph::LIST_BULLETS, ""),
        A::CreateArchive => (ph::FILE_ZIP, "zip 7z tar"),
        A::ExtractHere => (ph::ARCHIVE, "unzip"),
        A::ExtractToFolder => (ph::ARCHIVE, "unzip"),
        A::ExtractTo => (ph::ARCHIVE, "unzip"),
        A::TestArchive => (ph::SEAL_CHECK, "check"),
        A::Checksum(_) => (ph::FINGERPRINT, "hash"),
        A::CompareChecksum => (ph::FINGERPRINT, "verify hash"),
        A::SetAttributes => (ph::SLIDERS, "chmod"),
        A::TouchTimestamps => (ph::CLOCK, "touch"),
        A::SplitFile => (ph::SCISSORS, ""),
        A::JoinFiles => (ph::PUZZLE_PIECE, "merge"),
        A::AddTag(_) => (ph::TAG, "label color"),
        A::ClearTags => (ph::TAG_SIMPLE, ""),
        A::Undo => (ph::ARROW_U_UP_LEFT, "revert"),
        A::Redo => (ph::ARROW_U_UP_RIGHT, ""),
        A::PreviousVersions => (ph::CLOCK_COUNTER_CLOCKWISE, "snapshot vss backup"),
        // ---- tools ----
        A::CommandPalette => (ph::COMMAND, "actions"),
        A::Search => (ph::MAGNIFYING_GLASS, "find"),
        A::SearchContent => (ph::FILE_SEARCH, "grep text"),
        A::FolderSize => (ph::CHART_PIE, "disk usage"),
        A::DiskUsageTreemap => (ph::SQUARES_FOUR, "space"),
        A::FindDuplicates => (ph::COPY, "dupes"),
        A::CompareFolders => (ph::GIT_DIFF, "diff"),
        A::SyncFolders => (ph::ARROWS_CLOCKWISE, "mirror backup"),
        A::SyncProfiles => (ph::CALENDAR_CHECK, "backup scheduled two-way"),
        A::Settings => (ph::GEAR_SIX, "preferences options"),
        A::EditConfigFile => (ph::FILE_CODE, "toml"),
        A::ReloadConfig => (ph::ARROW_CLOCKWISE, ""),
        A::OpenTrash => (ph::TRASH, "recycle bin"),
        A::EmptyTrash => (ph::TRASH_SIMPLE, "recycle bin"),
        A::RestoreFromTrash => (ph::ARROW_COUNTER_CLOCKWISE, "undelete"),
        A::Computer => (ph::DESKTOP_TOWER, "this pc volumes"),
        A::ShowKeybindings => (ph::KEYBOARD, "keys hotkeys"),
        A::About => (ph::PLANET, ""),
        A::Welcome => (ph::HAND_WAVING, "onboarding help intro"),
        A::Quit => (ph::POWER, "exit close"),
        A::QuickLook => (ph::EYE, "space"),
        A::QuickLookNext => (ph::CARET_RIGHT, ""),
        A::QuickLookPrev => (ph::CARET_LEFT, ""),
        A::ToggleHexPreview => (ph::BINARY, "bytes"),
        A::ToggleAiPanel => (ph::SPARKLE, "chat agent llm copilot"),
        A::AiOrganize => (ph::MAGIC_WAND, "sort clean tidy ai"),
        A::AiSuggestNames => (ph::TEXT_T, "rename ai"),
        A::AiSummarize => (ph::NOTEPAD, "ai tldr"),
        A::AiExplainFolder => (ph::QUESTION, "ai describe"),
        A::AiFindRelated => (ph::FLOW_ARROW, "ai similar"),
        A::AiSmartSearch => (ph::MAGNIFYING_GLASS, "natural language ai find"),
        A::AiAuditLog => (ph::SCROLL, "ai history log privacy"),
        A::AiRecipes => (ph::BOOK_OPEN, "ai macro recipe"),
        A::AiStop => (ph::STOP_CIRCLE, "ai cancel escape"),
        A::CustomCommand(_) => (ph::LIGHTNING, "user"),
        A::RunScript(_) => (ph::CODE, "rhai script user"),
        A::OpenScripts => (ph::CODE, "rhai plugin"),
        A::ReloadScripts => (ph::ARROWS_CLOCKWISE, "rhai refresh"),
        A::OpenAutomations => (ph::ROBOT, "rules hazel watch"),
        // ---- jobs ----
        A::JobsPauseAll => (ph::PAUSE, ""),
        A::JobsResumeAll => (ph::PLAY, ""),
        A::JobsCancelAll => (ph::STOP, ""),
        A::JobsClearFinished => (ph::BROOM, ""),
        A::JobsShowHistory => (ph::CLOCK_COUNTER_CLOCKWISE, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn every_action() -> Vec<Action> {
        let mut v = Action::all();
        v.push(Action::CustomCommand("Open in VS Code".into()));
        v.push(Action::RunScript("tidy/copy-names".into()));
        v
    }

    #[test]
    fn every_action_has_palette_metadata() {
        let mut titles = HashSet::new();
        for a in Action::all() {
            let e = entry_in(&a, Locale::EnUs);
            assert!(!e.title.trim().is_empty(), "{a}: title");
            assert!(!e.icon.is_empty(), "{a}: icon");
            assert!(!e.description.trim().is_empty(), "{a}: description");
            assert!(titles.insert(e.title.clone()), "duplicate palette title {}", e.title);
        }
        let cc = entry(&Action::CustomCommand("Open in VS Code".into()));
        assert_eq!(cc.title, "Open in VS Code");
    }

    /// The dynamic `action-<name>` / `action-<name>-desc` keys exist in both languages
    /// for every action, and formatting leaves no unresolved placeholders.
    #[test]
    fn every_action_is_translated() {
        for l in [Locale::EnUs, Locale::TrTr] {
            let mut titles = HashSet::new();
            for a in every_action() {
                let desc = format!("{}-desc", key_of(&a));
                assert!(crate::i18n::has_in(&desc, l), "{a}: {desc} missing in {}", l.tag());
                let is_unit = a.to_string() == a.name();
                if is_unit {
                    assert!(crate::i18n::has_in(&key_of(&a), l), "{a}: {} missing in {}", key_of(&a), l.tag());
                }
                let t = title_in(&a, l);
                assert!(!t.contains('{') && !t.starts_with("action-"), "{a}: {t}");
                assert!(!description_in(&a, l).starts_with("action-"), "{a}");
                if l == Locale::TrTr {
                    assert!(titles.insert(t.clone()), "duplicate Turkish palette title {t}");
                }
            }
        }
    }

    #[test]
    fn titles_keep_their_english_text() {
        assert_eq!(title_in(&Action::GoTo, Locale::EnUs), "Go to path…");
        assert_eq!(title_in(&Action::CopyPathPosix, Locale::EnUs), "Copy path (forward slashes)");
        assert_eq!(title_in(&Action::PrevTab, Locale::EnUs), "Prev tab");
        assert_eq!(title_in(&Action::SortBy(Column::Size), Locale::EnUs), "Sort by: Size");
        assert_eq!(title_in(&Action::SelectTab(9), Locale::EnUs), "Go to last tab");
        assert_eq!(title_in(&Action::SelectTab(2), Locale::EnUs), "Go to tab 2");
        assert_eq!(title_in(&Action::AddTag(1), Locale::EnUs), "Add tag: Red");
        assert_eq!(title_in(&Action::SetGroupBy(GroupBy::None), Locale::EnUs), "Group by: None");
        assert_eq!(title_in(&Action::SetGroupBy(GroupBy::Date), Locale::EnUs), "Group by: Date modified");
        assert_eq!(description_in(&Action::Back, Locale::EnUs), "Go back in this pane's history");
        assert_eq!(title_in(&Action::Delete, Locale::TrTr), "Çöp Kutusu'na taşı");
        assert_eq!(title_in(&Action::OpenBookmark(3), Locale::TrTr), "3. yer işaretini aç");
        let e = entry_in(&Action::NewFolder, Locale::TrTr);
        assert_eq!((e.title.as_str(), e.title_en.as_str()), ("Yeni klasör", "New folder"));
    }
}
