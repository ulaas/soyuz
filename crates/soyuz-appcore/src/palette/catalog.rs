//! Palette metadata for every [`Action`]: icon, translated title and one-line
//! description, and extra (English) search keywords. `Action::all()` is covered
//! exhaustively (unit-tested, in both languages).
//!
//! Titles and descriptions live in `locales/<lang>/actions.ftl` as `action-<name>` /
//! `action-<name>-desc`, `<name>` being the action's config name with `-` for `_`.

use soyuz_core::config::{Action, ActionCategory};
use soyuz_core::{Column, GroupBy, Locale};

use crate::i18n::{FluentArgs, t_args_in, t_in};
use crate::icon::IconId;

/// Everything the palette shows for an action.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub title: String,
    /// The English title: the palette matches it too, so English muscle memory keeps
    /// working in another UI language.
    pub title_en: String,
    pub icon: IconId,
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
        description: shown_description_in(a, l),
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

/// One-line description in `l` (the generated keyboard reference uses this
/// platform-independent text; the palette shows [`shown_description_in`]).
pub fn description_in(a: &Action, l: Locale) -> String {
    // i18n-dynamic: action-
    t_in(&format!("{}-desc", key_of(a)), l).to_string()
}

/// [`description_in`] as shown on this platform: on Linux an
/// `action-<name>-desc-linux` variant, when present, replaces wording that only fits
/// Windows (shadow copies, administrator, …).
pub fn shown_description_in(a: &Action, l: Locale) -> String {
    if cfg!(target_os = "linux") {
        let linux = format!("{}-desc-linux", key_of(a));
        if crate::i18n::has_in(&linux, l) {
            // i18n-dynamic: action-
            return t_in(&linux, l).to_string();
        }
    }
    description_in(a, l)
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

fn column_icon(c: Column) -> IconId {
    match c {
        Column::Name => IconId::TextAa,
        Column::Size => IconId::HardDrives,
        Column::Modified | Column::Created | Column::Accessed => IconId::Calendar,
        Column::Kind | Column::Extension => IconId::File,
        Column::Tags => IconId::Tag,
        Column::GitStatus => IconId::GitBranch,
        _ => IconId::SortAscending,
    }
}

/// Icon and English search keywords.
fn meta(a: &Action) -> (IconId, &'static str) {
    use Action as A;
    match a {
        // ---- navigation ----
        A::Back => (IconId::ArrowLeft, "previous history"),
        A::Forward => (IconId::ArrowRight, "next history"),
        A::Up => (IconId::ArrowUp, "parent"),
        A::Home => (IconId::House, "user profile"),
        A::Root => (IconId::HardDrive, "drive"),
        A::GoTo => (IconId::ArrowSquareRight, "address location"),
        A::FocusAddressBar => (IconId::Textbox, "omnibar location"),
        A::EditPath => (IconId::PencilLine, "address location"),
        A::Refresh => (IconId::ArrowClockwise, "reload rescan"),
        A::ToggleSidebar => (IconId::Sidebar, "panel"),
        A::FocusSidebar => (IconId::Sidebar, ""),
        A::FocusActivityPanel => (IconId::Pulse, "jobs progress operations"),
        A::FocusList => (IconId::List, ""),
        A::FocusOtherPane => (IconId::SquareSplitHorizontal, "dual"),
        A::FocusTerminal => (IconId::TerminalWindow, "shell console"),
        A::NextTab => (IconId::ArrowFatRight, ""),
        A::PrevTab => (IconId::ArrowFatLeft, ""),
        A::JumpToFolderDialog => (IconId::FolderOpen, "frecency recent jump"),
        A::OpenRecent => (IconId::ClockCounterClockwise, "history"),
        A::ConnectToServer => (IconId::Globe, "remote network ssh cloud bucket"),
        A::OpenBookmark(_) => (IconId::BookmarkSimple, "favorite"),
        A::SelectTab(_) => (IconId::Browsers, ""),
        A::OpenTimeline => (IconId::CalendarDots, "recent modified changes week"),
        A::TabHistory => (IconId::GitFork, "history graph where was i"),
        // ---- tabs & panes ----
        A::NewTab => (IconId::PlusSquare, ""),
        A::CloseTab => (IconId::XSquare, ""),
        A::ReopenClosedTab => (IconId::ArrowCounterClockwise, "restore undo"),
        A::DuplicateTab => (IconId::Copy, "clone"),
        A::PinTab => (IconId::PushPin, ""),
        A::MoveTabLeft => (IconId::CaretDoubleLeft, "reorder"),
        A::MoveTabRight => (IconId::CaretDoubleRight, "reorder"),
        A::TabToNewWindow => (IconId::AppWindow, ""),
        A::ToggleDualPane => (IconId::SquareSplitHorizontal, "split commander"),
        A::SwapPanes => (IconId::ArrowsLeftRight, "exchange"),
        A::MirrorPaneToOther => (IconId::CopySimple, "same"),
        A::OpenInOtherPane => (IconId::ArrowSquareOut, ""),
        A::SyncNavigation => (IconId::LinkSimple, "lock"),
        A::NewWindow => (IconId::AppWindow, ""),
        A::CloseWindow => (IconId::X, ""),
        A::ToggleQuadPane => (IconId::SquaresFour, "layout grid split quad"),
        A::NewTabGroup => (IconId::RectangleDashed, "group color"),
        A::UngroupTab => (IconId::Rectangle, "group"),
        A::MergeWindows => (IconId::Browsers, "windows combine"),
        A::Workspaces => (IconId::Stack, "layout session"),
        A::SaveWorkspace => (IconId::FloppyDisk, "layout session"),
        A::UpdateWorkspace => (IconId::ArrowsClockwise, "layout"),
        A::OpenWorkspace(_) => (IconId::Stack, "layout"),
        // ---- view ----
        A::ViewDetails => (IconId::List, "table columns"),
        A::ViewGrid => (IconId::SquaresFour, "icons thumbnails"),
        A::ViewList => (IconId::ListDashes, ""),
        A::ViewMiller => (IconId::Columns, "finder miller"),
        A::ViewGallery => (IconId::Images, "photos"),
        A::ZoomIn => (IconId::MagnifyingGlassPlus, "bigger"),
        A::ZoomOut => (IconId::MagnifyingGlassMinus, "smaller"),
        A::ZoomReset => (IconId::MagnifyingGlass, "zoom"),
        A::UiZoomIn => (IconId::MagnifyingGlassPlus, "ui scale bigger accessibility"),
        A::UiZoomOut => (IconId::MagnifyingGlassMinus, "ui scale smaller"),
        A::UiZoomReset => (IconId::MagnifyingGlass, "ui scale"),
        A::ToggleHighContrast => (IconId::CircleHalf, "accessibility a11y"),
        A::ToggleHidden => (IconId::EyeSlash, "dotfiles"),
        A::ToggleFlatView => (IconId::TreeStructure, "recursive flatten"),
        A::ToggleSortDir => (IconId::ArrowsDownUp, "sort"),
        A::ChooseColumns => (IconId::Columns, ""),
        A::ResetFolderView => (IconId::ArrowUUpLeft, ""),
        A::ApplyViewToAll => (IconId::Stack, "default"),
        A::TogglePreviewPane => (IconId::SidebarSimple, "inspector"),
        A::ToggleShelf => (IconId::Tray, "stash basket collect"),
        A::ToggleInfoPane => (IconId::Info, "metadata"),
        A::ToggleTerminal => (IconId::TerminalWindow, "shell console"),
        A::ToggleActivityPanel => (IconId::Pulse, "progress operations"),
        A::ToggleStatusBar => (IconId::Rectangle, ""),
        A::ToggleTreeSidebar => (IconId::TreeView, ""),
        A::ToggleCheckboxes => (IconId::CheckSquare, "touch"),
        A::SortBy(c) => (column_icon(*c), "order"),
        A::SetGroupBy(_) => (IconId::Rows, ""),
        // ---- selection ----
        A::SelectAll => (IconId::SelectionAll, ""),
        A::SelectNone => (IconId::SelectionSlash, "deselect"),
        A::InvertSelection => (IconId::SelectionInverse, ""),
        A::SelectByPattern => (IconId::SelectionPlus, "wildcard"),
        A::SelectSameExtension => (IconId::FileMagnifyingGlass, "type"),
        A::SelectSameName => (IconId::TextAa, ""),
        A::MarkToggle => (IconId::CheckSquareOffset, ""),
        A::MarkAndMoveDown => (IconId::CheckSquareOffset, ""),
        A::CursorUp => (IconId::CaretUp, ""),
        A::CursorDown => (IconId::CaretDown, ""),
        A::CursorPageUp => (IconId::CaretDoubleUp, ""),
        A::CursorPageDown => (IconId::CaretDoubleDown, ""),
        A::CursorHome => (IconId::ArrowLineUp, "top"),
        A::CursorEnd => (IconId::ArrowLineDown, "bottom"),
        A::ExtendUp => (IconId::CaretUp, ""),
        A::ExtendDown => (IconId::CaretDown, ""),
        A::Filter => (IconId::Funnel, "find"),
        A::ClearFilter => (IconId::FunnelX, ""),
        A::TypeAheadToggle => (IconId::Keyboard, ""),
        // ---- file ops ----
        A::Open => (IconId::ArrowSquareOut, "launch enter"),
        A::OpenWith => (IconId::AppWindow, "program"),
        A::ToggleEditorPanel => (IconId::NotePencil, "edit text hex image"),
        A::OpenInEditor => (IconId::NotePencil, "edit hex image notepad"),
        A::OpenInNewTab => (IconId::Browsers, ""),
        A::OpenInTerminal => (IconId::TerminalWindow, "shell"),
        A::OpenTerminalHere => (IconId::TerminalWindow, "shell console"),
        A::RunElevated => (IconId::ShieldCheck, "admin sudo"),
        A::RevealInSystem => (IconId::ArrowSquareUpRight, "explorer finder"),
        A::Copy => (IconId::Copy, ""),
        A::Cut => (IconId::Scissors, "move"),
        A::Paste => (IconId::Clipboard, ""),
        A::PasteAsLink => (IconId::Link, "symlink"),
        A::PasteShortcut => (IconId::LinkSimple, "lnk"),
        A::PasteAndVerify => (IconId::SealCheck, "verify checksum copy"),
        A::CopyToOtherPane => (IconId::CopySimple, "f5"),
        A::MoveToOtherPane => (IconId::ArrowBendDownRight, "f6"),
        A::CopyTo => (IconId::Copy, ""),
        A::MoveTo => (IconId::ArrowBendUpRight, ""),
        A::Duplicate => (IconId::CopySimple, "clone"),
        A::Rename => (IconId::PencilSimple, "f2"),
        A::BatchRename => (IconId::TextColumns, "bulk regex"),
        A::NewFolder => (IconId::FolderPlus, "mkdir directory"),
        A::NewFile => (IconId::FilePlus, "touch"),
        A::NewSymlink => (IconId::Link, "shortcut"),
        A::NewHardlink => (IconId::LinkBreak, ""),
        A::NewJunction => (IconId::GitFork, "mount"),
        A::Delete => (IconId::Trash, "remove recycle"),
        A::DeletePermanent => (IconId::XCircle, "remove erase"),
        A::SecureWipe => (IconId::Fire, "shred"),
        A::Properties => (IconId::Info, "info details"),
        A::CopyPath => (IconId::ClipboardText, ""),
        A::CopyPathPosix => (IconId::ClipboardText, "unix"),
        A::CopyPathQuoted => (IconId::ClipboardText, ""),
        A::CopyName => (IconId::TextAa, ""),
        A::CopyAsFileList => (IconId::ListBullets, ""),
        A::CreateArchive => (IconId::FileZip, "zip 7z tar"),
        A::ExtractHere => (IconId::Archive, "unzip"),
        A::ExtractToFolder => (IconId::Archive, "unzip"),
        A::ExtractTo => (IconId::Archive, "unzip"),
        A::TestArchive => (IconId::SealCheck, "check"),
        A::Checksum(_) => (IconId::Fingerprint, "hash"),
        A::CompareChecksum => (IconId::Fingerprint, "verify hash"),
        A::SetAttributes => (IconId::Sliders, "chmod"),
        A::TouchTimestamps => (IconId::Clock, "touch"),
        A::SplitFile => (IconId::Scissors, ""),
        A::JoinFiles => (IconId::PuzzlePiece, "merge"),
        A::AddTag(_) => (IconId::Tag, "label color"),
        A::ClearTags => (IconId::TagSimple, ""),
        A::Undo => (IconId::ArrowUUpLeft, "revert"),
        A::Redo => (IconId::ArrowUUpRight, ""),
        A::PreviousVersions => (IconId::ClockCounterClockwise, "snapshot vss backup"),
        // ---- tools ----
        A::CommandPalette => (IconId::Command, "actions"),
        A::Search => (IconId::MagnifyingGlass, "find"),
        A::SearchContent => (IconId::FileSearch, "grep text"),
        A::FolderSize => (IconId::ChartPie, "disk usage"),
        A::DiskUsageTreemap => (IconId::SquaresFour, "space"),
        A::FindDuplicates => (IconId::Copy, "dupes"),
        A::CompareFolders => (IconId::GitDiff, "diff"),
        A::SyncFolders => (IconId::ArrowsClockwise, "mirror backup"),
        A::SyncProfiles => (IconId::CalendarCheck, "backup scheduled two-way"),
        A::Settings => (IconId::GearSix, "preferences options"),
        A::EditConfigFile => (IconId::FileCode, "toml"),
        A::ReloadConfig => (IconId::ArrowClockwise, ""),
        A::OpenTrash => (IconId::Trash, "recycle bin"),
        A::EmptyTrash => (IconId::TrashSimple, "recycle bin"),
        A::RestoreFromTrash => (IconId::ArrowCounterClockwise, "undelete"),
        A::Computer => (IconId::DesktopTower, "this pc volumes"),
        A::ShowKeybindings => (IconId::Keyboard, "keys hotkeys"),
        A::About => (IconId::Planet, ""),
        A::Welcome => (IconId::HandWaving, "onboarding help intro"),
        A::Quit => (IconId::Power, "exit close"),
        A::QuickLook => (IconId::Eye, "space"),
        A::QuickLookNext => (IconId::CaretRight, ""),
        A::QuickLookPrev => (IconId::CaretLeft, ""),
        A::ToggleHexPreview => (IconId::Binary, "bytes"),
        A::PreviewScrollUp => (IconId::CaretUp, "preview scroll"),
        A::PreviewScrollDown => (IconId::CaretDown, "preview scroll"),
        A::PreviewPageUp => (IconId::ArrowLineUp, "preview scroll page"),
        A::PreviewPageDown => (IconId::ArrowLineDown, "preview scroll page"),
        A::ContextMenu => (IconId::List, "right click menu shift+f10"),
        A::ToggleAiPanel => (IconId::Sparkle, "chat agent llm copilot"),
        A::AiOrganize => (IconId::MagicWand, "sort clean tidy ai"),
        A::AiSuggestNames => (IconId::TextT, "rename ai"),
        A::AiSummarize => (IconId::Notepad, "ai tldr"),
        A::AiExplainFolder => (IconId::Question, "ai describe"),
        A::AiFindRelated => (IconId::FlowArrow, "ai similar"),
        A::AiSmartSearch => (IconId::MagnifyingGlass, "natural language ai find"),
        A::AiAuditLog => (IconId::Scroll, "ai history log privacy"),
        A::AiRecipes => (IconId::BookOpen, "ai macro recipe"),
        A::AiStop => (IconId::StopCircle, "ai cancel escape"),
        A::CustomCommand(_) => (IconId::Lightning, "user"),
        A::RunScript(_) => (IconId::Code, "rhai script user"),
        A::OpenScripts => (IconId::Code, "rhai plugin"),
        A::ReloadScripts => (IconId::ArrowsClockwise, "rhai refresh"),
        A::OpenAutomations => (IconId::Robot, "rules hazel watch"),
        // ---- jobs ----
        A::JobsPauseAll => (IconId::Pause, ""),
        A::JobsResumeAll => (IconId::Play, ""),
        A::JobsCancelAll => (IconId::Stop, ""),
        A::JobsClearFinished => (IconId::Broom, ""),
        A::JobsShowHistory => (IconId::ClockCounterClockwise, ""),
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
    fn linux_descriptions_replace_windows_wording() {
        for l in [Locale::EnUs, Locale::TrTr] {
            let shown = shown_description_in(&Action::PreviousVersions, l);
            assert_eq!(shown.contains("Snapper"), cfg!(target_os = "linux"), "{shown}");
            assert!(!description_in(&Action::PreviousVersions, l).contains("Snapper"), "docs keep the base text");
        }
        assert_eq!(crate::actions::unavailable(&Action::NewJunction), !cfg!(windows));
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
