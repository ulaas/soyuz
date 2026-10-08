//! Palette metadata for every [`Action`]: icon, one-line description and extra search
//! keywords. `Action::all()` is covered exhaustively (unit-tested).

use soyuz_core::config::{Action, ActionCategory};
use soyuz_core::{Column, GroupBy};

use crate::theme::icons::ph;

/// Everything the palette shows for an action.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub title: String,
    pub icon: &'static str,
    pub description: String,
    pub keywords: &'static str,
    pub category: ActionCategory,
}

pub fn entry(a: &Action) -> Entry {
    let (icon, description, keywords) = meta(a);
    Entry { title: title(a), icon, description, keywords, category: a.category() }
}

/// Friendlier titles where the generated one (`Action::title`) reads poorly.
pub fn title(a: &Action) -> String {
    use Action as A;
    let s = match a {
        A::GoTo => "Go to path…",
        A::FocusAddressBar => "Focus address bar",
        A::JumpToFolderDialog => "Go to folder…",
        A::OpenRecent => "Recent folders",
        A::ConnectToServer => "Connect to server…",
        A::TabToNewWindow => "Move tab to new window",
        A::MirrorPaneToOther => "Mirror folder in other pane",
        A::SyncNavigation => "Toggle synchronized browsing",
        A::ToggleQuadPane => "Toggle quad layout (2×2)",
        A::NewTabGroup => "Add tab to new group",
        A::UngroupTab => "Remove tab from group",
        A::MergeWindows => "Merge all windows into this one",
        A::Workspaces => "Workspaces…",
        A::SaveWorkspace => "Save workspace as…",
        A::UpdateWorkspace => "Update current workspace",
        A::OpenTimeline => "Timeline: changed files",
        A::TabHistory => "Where was I? (tab history graph)",
        A::ViewDetails => "View: Details",
        A::ViewGrid => "View: Grid",
        A::ViewList => "View: List",
        A::ViewMiller => "View: Columns (Miller)",
        A::ViewGallery => "View: Gallery",
        A::ToggleHidden => "Toggle hidden files",
        A::ToggleFlatView => "Toggle flat view",
        A::ToggleSortDir => "Reverse sort order",
        A::ChooseColumns => "Choose columns…",
        A::ApplyViewToAll => "Apply view to all folders",
        A::ToggleTreeSidebar => "Toggle folder tree",
        A::ToggleCheckboxes => "Toggle item checkboxes",
        A::SelectByPattern => "Select by pattern…",
        A::SelectSameExtension => "Select same extension",
        A::SelectSameName => "Select same name",
        A::MarkToggle => "Toggle mark",
        A::MarkAndMoveDown => "Mark and move down",
        A::Filter => "Filter this folder",
        A::TypeAheadToggle => "Toggle type-ahead mode",
        A::OpenWith => "Open with…",
        A::OpenInEditor => "Edit in Soyuz",
        A::ToggleEditorPanel => "Toggle editor panel",
        A::OpenInTerminal => "Open in terminal",
        A::RevealInSystem => "Reveal in system file manager",
        A::PasteShortcut => "Paste shortcut",
        A::CopyTo => "Copy to…",
        A::MoveTo => "Move to…",
        A::BatchRename => "Batch rename…",
        A::NewSymlink => "New symbolic link…",
        A::NewHardlink => "New hard link…",
        A::NewJunction => "New junction…",
        A::Delete => "Move to trash",
        A::DeletePermanent => "Delete permanently",
        A::SecureWipe => "Secure wipe",
        A::Properties => "Properties",
        A::CopyPathPosix => "Copy path (forward slashes)",
        A::CopyPathQuoted => "Copy path (quoted)",
        A::CopyAsFileList => "Copy as file list",
        A::CreateArchive => "Create archive…",
        A::ExtractToFolder => "Extract to folder",
        A::ExtractTo => "Extract to…",
        A::CompareChecksum => "Compare checksum…",
        A::SetAttributes => "Set attributes…",
        A::TouchTimestamps => "Touch timestamps",
        A::CommandPalette => "Command palette",
        A::SearchContent => "Search file contents",
        A::FolderSize => "Calculate folder sizes",
        A::DiskUsageTreemap => "Disk usage treemap",
        A::FindDuplicates => "Find duplicate files",
        A::CompareFolders => "Compare folders",
        A::SyncFolders => "Synchronize folders",
        A::SyncProfiles => "Sync profiles & schedules",
        A::PasteAndVerify => "Paste and verify",
        A::PreviousVersions => "Restore previous version…",
        A::Settings => "Settings",
        A::EditConfigFile => "Edit config file",
        A::ReloadConfig => "Reload config",
        A::OpenTrash => "Open trash",
        A::RestoreFromTrash => "Restore from trash",
        A::ShowKeybindings => "Keyboard shortcuts",
        A::About => "About Soyuz",
        A::Welcome => "Welcome tips",
        A::UiZoomIn => "Zoom in (interface)",
        A::UiZoomOut => "Zoom out (interface)",
        A::UiZoomReset => "Reset interface zoom",
        A::ToggleHighContrast => "Toggle high contrast",
        A::ZoomIn => "Larger icons",
        A::ZoomOut => "Smaller icons",
        A::ZoomReset => "Reset icon size",
        A::QuickLook => "Quick Look",
        A::QuickLookNext => "Quick Look: next item",
        A::QuickLookPrev => "Quick Look: previous item",
        A::ToggleHexPreview => "Toggle hex preview",
        A::ToggleAiPanel => "Toggle AI assistant",
        A::AiOrganize => "AI: Organize this folder…",
        A::AiSuggestNames => "AI: Suggest names",
        A::AiSummarize => "AI: Summarize",
        A::AiExplainFolder => "AI: Explain this folder",
        A::AiFindRelated => "AI: Find related files",
        A::AiSmartSearch => "AI: Smart search",
        A::AiAuditLog => "AI: Audit log",
        A::AiRecipes => "AI: Recipes",
        A::AiStop => "AI: Stop the assistant",
        A::OpenScripts => "Scripts…",
        A::ReloadScripts => "Reload scripts",
        A::OpenAutomations => "Automations…",
        A::JobsPauseAll => "Pause all jobs",
        A::JobsResumeAll => "Resume all jobs",
        A::JobsCancelAll => "Cancel all jobs",
        A::JobsClearFinished => "Clear finished jobs",
        A::JobsShowHistory => "Show job history",
        A::OpenBookmark(n) => return format!("Open bookmark {n}"),
        A::SelectTab(9) => return "Go to last tab".into(),
        A::SelectTab(n) => return format!("Go to tab {n}"),
        A::OpenWorkspace(n) => return format!("Open workspace {n}"),
        A::AddTag(n) => return format!("Add tag: {}", tag_name(*n)),
        A::SetGroupBy(GroupBy::None) => return "Group by: None".into(),
        A::SetGroupBy(g) => return format!("Group by: {}", group_label(*g)),
        A::CustomCommand(name) => return name.clone(),
        A::RunScript(id) => return crate::automation::action_title(id),
        _ => return a.title(),
    };
    s.to_string()
}

pub fn tag_name(n: u8) -> &'static str {
    ["Red", "Orange", "Yellow", "Green", "Cyan", "Blue", "Purple", "Gray"].get(n as usize - 1).copied().unwrap_or("Tag")
}

fn group_label(g: GroupBy) -> &'static str {
    match g {
        GroupBy::None => "None",
        GroupBy::Kind => "Type",
        GroupBy::Date => "Date modified",
        GroupBy::Size => "Size",
        GroupBy::Extension => "Extension",
        GroupBy::Tag => "Tag",
        GroupBy::FirstLetter => "First letter",
    }
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

fn meta(a: &Action) -> (&'static str, String, &'static str) {
    use Action as A;
    let (icon, desc, kw): (&'static str, &str, &'static str) = match a {
        // ---- navigation ----
        A::Back => (ph::ARROW_LEFT, "Go back in this pane's history", "previous history"),
        A::Forward => (ph::ARROW_RIGHT, "Go forward in this pane's history", "next history"),
        A::Up => (ph::ARROW_UP, "Go to the parent folder", "parent"),
        A::Home => (ph::HOUSE, "Go to your home folder", "user profile"),
        A::Root => (ph::HARD_DRIVE, "Go to the root of the current drive", "drive"),
        A::GoTo => (ph::ARROW_SQUARE_RIGHT, "Type a path into the address bar", "address location"),
        A::FocusAddressBar => (ph::TEXTBOX, "Edit the path in the address bar", "omnibar location"),
        A::EditPath => (ph::PENCIL_LINE, "Edit the current path as text", "address location"),
        A::Refresh => (ph::ARROW_CLOCKWISE, "Reload the folder listing", "reload rescan"),
        A::ToggleSidebar => (ph::SIDEBAR, "Show or hide the sidebar", "panel"),
        A::FocusSidebar => (ph::SIDEBAR, "Move keyboard focus to the sidebar", ""),
        A::FocusList => (ph::LIST, "Move keyboard focus to the file list", ""),
        A::FocusOtherPane => (ph::SQUARE_SPLIT_HORIZONTAL, "Switch focus to the other pane", "dual"),
        A::FocusTerminal => (ph::TERMINAL_WINDOW, "Move keyboard focus to the terminal", "shell console"),
        A::NextTab => (ph::ARROW_FAT_RIGHT, "Switch to the next tab", ""),
        A::PrevTab => (ph::ARROW_FAT_LEFT, "Switch to the previous tab", ""),
        A::JumpToFolderDialog => (ph::FOLDER_OPEN, "Fuzzy-jump to a recent or frequent folder", "frecency recent jump"),
        A::OpenRecent => (ph::CLOCK_COUNTER_CLOCKWISE, "Pick from recently visited folders", "history"),
        A::ConnectToServer => {
            (ph::GLOBE, "Browse an SFTP, WebDAV, FTP / FTPS or S3 server", "remote network ssh cloud bucket")
        }
        A::OpenBookmark(_) => (ph::BOOKMARK_SIMPLE, "Navigate to a configured bookmark", "favorite"),
        A::SelectTab(_) => (ph::BROWSERS, "Switch to a tab by position", ""),
        A::OpenTimeline => (ph::CALENDAR_DOTS, "Files changed today and this week", "recent modified changes week"),
        A::TabHistory => (ph::GIT_FORK, "Every place this tab visited, with branches", "history graph where was i"),
        // ---- tabs & panes ----
        A::NewTab => (ph::PLUS_SQUARE, "Open a new tab", ""),
        A::CloseTab => (ph::X_SQUARE, "Close the current tab", ""),
        A::ReopenClosedTab => (ph::ARROW_COUNTER_CLOCKWISE, "Reopen the most recently closed tab", "restore undo"),
        A::DuplicateTab => (ph::COPY, "Open the current folder in a new tab", "clone"),
        A::PinTab => (ph::PUSH_PIN, "Pin or unpin the current tab", ""),
        A::MoveTabLeft => (ph::CARET_DOUBLE_LEFT, "Move the tab one position left", "reorder"),
        A::MoveTabRight => (ph::CARET_DOUBLE_RIGHT, "Move the tab one position right", "reorder"),
        A::TabToNewWindow => (ph::APP_WINDOW, "Tear the tab off into its own window", ""),
        A::ToggleDualPane => (ph::SQUARE_SPLIT_HORIZONTAL, "Show or hide the second pane", "split commander"),
        A::SwapPanes => (ph::ARROWS_LEFT_RIGHT, "Swap the left and right panes", "exchange"),
        A::MirrorPaneToOther => (ph::COPY_SIMPLE, "Show this folder in the other pane too", "same"),
        A::OpenInOtherPane => (ph::ARROW_SQUARE_OUT, "Open the selected folder in the other pane", ""),
        A::SyncNavigation => (ph::LINK_SIMPLE, "Navigate both panes together", "lock"),
        A::NewWindow => (ph::APP_WINDOW, "Open a new Soyuz window", ""),
        A::CloseWindow => (ph::X, "Close this window", ""),
        A::ToggleQuadPane => (ph::SQUARES_FOUR, "Show four panes in a 2×2 grid", "layout grid split quad"),
        A::NewTabGroup => (ph::RECTANGLE_DASHED, "Put the current tab into a new tab group", "group color"),
        A::UngroupTab => (ph::RECTANGLE, "Take the current tab out of its group", "group"),
        A::MergeWindows => (ph::BROWSERS, "Move the tabs of every other window here", "windows combine"),
        A::Workspaces => (ph::STACK, "Open, save and manage named workspaces", "layout session"),
        A::SaveWorkspace => (ph::FLOPPY_DISK, "Save this window's tabs and layout under a name", "layout session"),
        A::UpdateWorkspace => (ph::ARROWS_CLOCKWISE, "Save this window over the workspace last opened", "layout"),
        A::OpenWorkspace(_) => (ph::STACK, "Restore a saved workspace", "layout"),
        // ---- view ----
        A::ViewDetails => (ph::LIST, "Show the folder as a detailed table", "table columns"),
        A::ViewGrid => (ph::SQUARES_FOUR, "Show icons or thumbnails in a grid", "icons thumbnails"),
        A::ViewList => (ph::LIST_DASHES, "Show a compact multi-column list", ""),
        A::ViewMiller => (ph::COLUMNS, "Browse with cascading columns", "finder miller"),
        A::ViewGallery => (ph::IMAGES, "Large preview with a filmstrip", "photos"),
        A::ZoomIn => (ph::MAGNIFYING_GLASS_PLUS, "Make icons and thumbnails larger", "bigger"),
        A::ZoomOut => (ph::MAGNIFYING_GLASS_MINUS, "Make icons and thumbnails smaller", "smaller"),
        A::ZoomReset => (ph::MAGNIFYING_GLASS, "Reset the icon size", "zoom"),
        A::UiZoomIn => (ph::MAGNIFYING_GLASS_PLUS, "Scale the whole interface up", "ui scale bigger accessibility"),
        A::UiZoomOut => (ph::MAGNIFYING_GLASS_MINUS, "Scale the whole interface down", "ui scale smaller"),
        A::UiZoomReset => (ph::MAGNIFYING_GLASS, "Back to 100 % interface scale", "ui scale"),
        A::ToggleHighContrast => (ph::CIRCLE_HALF, "Stronger text, borders and focus rings", "accessibility a11y"),
        A::ToggleHidden => (ph::EYE_SLASH, "Show or hide hidden files", "dotfiles"),
        A::ToggleFlatView => (ph::TREE_STRUCTURE, "List the contents of all subfolders", "recursive flatten"),
        A::ToggleSortDir => (ph::ARROWS_DOWN_UP, "Flip ascending / descending order", "sort"),
        A::ChooseColumns => (ph::COLUMNS, "Pick the columns shown in details view", ""),
        A::ResetFolderView => (ph::ARROW_U_UP_LEFT, "Forget the view settings of this folder", ""),
        A::ApplyViewToAll => (ph::STACK, "Use this layout for every folder", "default"),
        A::TogglePreviewPane => (ph::SIDEBAR_SIMPLE, "Show or hide the preview pane", "inspector"),
        A::ToggleShelf => (ph::TRAY, "Show or hide the drop shelf for collecting files", "stash basket collect"),
        A::ToggleInfoPane => (ph::INFO, "Show or hide the info pane", "metadata"),
        A::ToggleTerminal => (ph::TERMINAL_WINDOW, "Show or hide the integrated terminal", "shell console"),
        A::ToggleActivityPanel => (ph::PULSE, "Show or hide running jobs", "progress operations"),
        A::ToggleStatusBar => (ph::RECTANGLE, "Show or hide the status bar", ""),
        A::ToggleTreeSidebar => (ph::TREE_VIEW, "Show or hide the folder tree", ""),
        A::ToggleCheckboxes => (ph::CHECK_SQUARE, "Select items with checkboxes", "touch"),
        A::SortBy(c) => (column_icon(*c), "Sort the folder by this column", "order"),
        A::SetGroupBy(_) => (ph::ROWS, "Group items under headers", ""),
        // ---- selection ----
        A::SelectAll => (ph::SELECTION_ALL, "Select every item", ""),
        A::SelectNone => (ph::SELECTION_SLASH, "Clear the selection", "deselect"),
        A::InvertSelection => (ph::SELECTION_INVERSE, "Select everything that isn't selected", ""),
        A::SelectByPattern => (ph::SELECTION_PLUS, "Select items matching a glob or text", "wildcard"),
        A::SelectSameExtension => (ph::FILE_MAGNIFYING_GLASS, "Select files with the same extension", "type"),
        A::SelectSameName => (ph::TEXT_AA, "Select items with the same base name", ""),
        A::MarkToggle => (ph::CHECK_SQUARE_OFFSET, "Toggle selection of the item under the cursor", ""),
        A::MarkAndMoveDown => (ph::CHECK_SQUARE_OFFSET, "Toggle selection and move to the next item", ""),
        A::CursorUp => (ph::CARET_UP, "Move the cursor up", ""),
        A::CursorDown => (ph::CARET_DOWN, "Move the cursor down", ""),
        A::CursorPageUp => (ph::CARET_DOUBLE_UP, "Move the cursor one page up", ""),
        A::CursorPageDown => (ph::CARET_DOUBLE_DOWN, "Move the cursor one page down", ""),
        A::CursorHome => (ph::ARROW_LINE_UP, "Jump to the first item", "top"),
        A::CursorEnd => (ph::ARROW_LINE_DOWN, "Jump to the last item", "bottom"),
        A::ExtendUp => (ph::CARET_UP, "Extend the selection upwards", ""),
        A::ExtendDown => (ph::CARET_DOWN, "Extend the selection downwards", ""),
        A::Filter => (ph::FUNNEL, "Filter the list as you type", "find"),
        A::ClearFilter => (ph::FUNNEL_X, "Remove the current filter", ""),
        A::TypeAheadToggle => (ph::KEYBOARD, "Switch typing between jump and filter", ""),
        // ---- file ops ----
        A::Open => (ph::ARROW_SQUARE_OUT, "Open the selection", "launch enter"),
        A::OpenWith => (ph::APP_WINDOW, "Open with a chosen application", "program"),
        A::ToggleEditorPanel => (ph::NOTE_PENCIL, "Show or hide the built-in editor panel", "edit text hex image"),
        A::OpenInEditor => {
            (ph::NOTE_PENCIL, "Open the file in the built-in text, hex or image editor", "edit hex image notepad")
        }
        A::OpenInNewTab => (ph::BROWSERS, "Open the selected folders in new tabs", ""),
        A::OpenInTerminal => (ph::TERMINAL_WINDOW, "Open the selected folder in the terminal", "shell"),
        A::OpenTerminalHere => (ph::TERMINAL_WINDOW, "Open a terminal in the current folder", "shell console"),
        A::RunElevated => (ph::SHIELD_CHECK, "Run the selection as administrator", "admin sudo"),
        A::RevealInSystem => (ph::ARROW_SQUARE_UP_RIGHT, "Show the item in the system file manager", "explorer finder"),
        A::Copy => (ph::COPY, "Copy the selection to the clipboard", ""),
        A::Cut => (ph::SCISSORS, "Cut the selection to the clipboard", "move"),
        A::Paste => (ph::CLIPBOARD, "Paste the clipboard into this folder", ""),
        A::PasteAsLink => (ph::LINK, "Paste the clipboard as links", "symlink"),
        A::PasteShortcut => (ph::LINK_SIMPLE, "Paste the clipboard as shortcuts", "lnk"),
        A::PasteAndVerify => (ph::SEAL_CHECK, "Paste, then hash-compare every copy", "verify checksum copy"),
        A::CopyToOtherPane => (ph::COPY_SIMPLE, "Copy the selection to the other pane", "f5"),
        A::MoveToOtherPane => (ph::ARROW_BEND_DOWN_RIGHT, "Move the selection to the other pane", "f6"),
        A::CopyTo => (ph::COPY, "Copy the selection to a chosen folder", ""),
        A::MoveTo => (ph::ARROW_BEND_UP_RIGHT, "Move the selection to a chosen folder", ""),
        A::Duplicate => (ph::COPY_SIMPLE, "Make a copy next to the original", "clone"),
        A::Rename => (ph::PENCIL_SIMPLE, "Rename the item under the cursor", "f2"),
        A::BatchRename => (ph::TEXT_COLUMNS, "Rename many items with patterns", "bulk regex"),
        A::NewFolder => (ph::FOLDER_PLUS, "Create a new folder here", "mkdir directory"),
        A::NewFile => (ph::FILE_PLUS, "Create a new empty file here", "touch"),
        A::NewSymlink => (ph::LINK, "Create a symbolic link", "shortcut"),
        A::NewHardlink => (ph::LINK_BREAK, "Create a hard link", ""),
        A::NewJunction => (ph::GIT_FORK, "Create an NTFS junction", "mount"),
        A::Delete => (ph::TRASH, "Move the selection to the trash", "remove recycle"),
        A::DeletePermanent => (ph::X_CIRCLE, "Delete without the trash", "remove erase"),
        A::SecureWipe => (ph::FIRE, "Overwrite and delete the selection", "shred"),
        A::Properties => (ph::INFO, "Show size, dates and attributes", "info details"),
        A::CopyPath => (ph::CLIPBOARD_TEXT, "Copy the full path to the clipboard", ""),
        A::CopyPathPosix => (ph::CLIPBOARD_TEXT, "Copy the path with forward slashes", "unix"),
        A::CopyPathQuoted => (ph::CLIPBOARD_TEXT, "Copy the path in double quotes", ""),
        A::CopyName => (ph::TEXT_AA, "Copy the file name to the clipboard", ""),
        A::CopyAsFileList => (ph::LIST_BULLETS, "Copy all selected paths, one per line", ""),
        A::CreateArchive => (ph::FILE_ZIP, "Compress the selection into an archive", "zip 7z tar"),
        A::ExtractHere => (ph::ARCHIVE, "Extract the archive into this folder", "unzip"),
        A::ExtractToFolder => (ph::ARCHIVE, "Extract into a folder named after the archive", "unzip"),
        A::ExtractTo => (ph::ARCHIVE, "Extract the archive to a chosen folder", "unzip"),
        A::TestArchive => (ph::SEAL_CHECK, "Verify the archive's integrity", "check"),
        A::Checksum(_) => (ph::FINGERPRINT, "Compute a checksum of the selection", "hash"),
        A::CompareChecksum => (ph::FINGERPRINT, "Compare a file against a known checksum", "verify hash"),
        A::SetAttributes => (ph::SLIDERS, "Change read-only / hidden / system attributes", "chmod"),
        A::TouchTimestamps => (ph::CLOCK, "Set modification times to now", "touch"),
        A::SplitFile => (ph::SCISSORS, "Split a large file into parts", ""),
        A::JoinFiles => (ph::PUZZLE_PIECE, "Join split parts back together", "merge"),
        A::AddTag(_) => (ph::TAG, "Label the selection with a color tag", "label color"),
        A::ClearTags => (ph::TAG_SIMPLE, "Remove all tags from the selection", ""),
        A::Undo => (ph::ARROW_U_UP_LEFT, "Undo the last file operation", "revert"),
        A::Redo => (ph::ARROW_U_UP_RIGHT, "Redo the last undone operation", ""),
        A::PreviousVersions => (
            ph::CLOCK_COUNTER_CLOCKWISE,
            "Browse and restore earlier versions from shadow copies",
            "snapshot vss backup",
        ),
        // ---- tools ----
        A::CommandPalette => (ph::COMMAND, "Search and run any command", "actions"),
        A::Search => (ph::MAGNIFYING_GLASS, "Search this folder and its subfolders", "find"),
        A::SearchContent => (ph::FILE_SEARCH, "Search inside files", "grep text"),
        A::FolderSize => (ph::CHART_PIE, "Compute the size of folders", "disk usage"),
        A::DiskUsageTreemap => (ph::SQUARES_FOUR, "Visualize what takes up space", "space"),
        A::FindDuplicates => (ph::COPY, "Find files with identical content", "dupes"),
        A::CompareFolders => (ph::GIT_DIFF, "Compare the contents of two folders", "diff"),
        A::SyncFolders => (ph::ARROWS_CLOCKWISE, "Make two folders identical", "mirror backup"),
        A::SyncProfiles => {
            (ph::CALENDAR_CHECK, "Saved sync profiles, schedules and run reports", "backup scheduled two-way")
        }
        A::Settings => (ph::GEAR_SIX, "Open the settings", "preferences options"),
        A::EditConfigFile => (ph::FILE_CODE, "Open config.toml in your editor", "toml"),
        A::ReloadConfig => (ph::ARROW_CLOCKWISE, "Re-read config.toml from disk", ""),
        A::OpenTrash => (ph::TRASH, "Browse the trash", "recycle bin"),
        A::EmptyTrash => (ph::TRASH_SIMPLE, "Permanently delete everything in the trash", "recycle bin"),
        A::RestoreFromTrash => (ph::ARROW_COUNTER_CLOCKWISE, "Put trashed items back", "undelete"),
        A::Computer => (ph::DESKTOP_TOWER, "Show drives and devices", "this pc volumes"),
        A::ShowKeybindings => (ph::KEYBOARD, "Browse and edit keyboard shortcuts", "keys hotkeys"),
        A::About => (ph::PLANET, "Version and credits", ""),
        A::Welcome => (ph::HAND_WAVING, "Show the getting-started tips again", "onboarding help intro"),
        A::Quit => (ph::POWER, "Quit Soyuz", "exit close"),
        A::QuickLook => (ph::EYE, "Preview the item in a popup", "space"),
        A::QuickLookNext => (ph::CARET_RIGHT, "Preview the next item", ""),
        A::QuickLookPrev => (ph::CARET_LEFT, "Preview the previous item", ""),
        A::ToggleHexPreview => (ph::BINARY, "Show the preview as hex", "bytes"),
        A::ToggleAiPanel => (ph::SPARKLE, "Show or hide the AI assistant panel", "chat agent llm copilot"),
        A::AiOrganize => (ph::MAGIC_WAND, "Ask the assistant to plan a tidy-up of this folder", "sort clean tidy ai"),
        A::AiSuggestNames => (ph::TEXT_T, "Ask the assistant for better names (preview before renaming)", "rename ai"),
        A::AiSummarize => (ph::NOTEPAD, "Summarize the selected files (contents only where allowed)", "ai tldr"),
        A::AiExplainFolder => (ph::QUESTION, "Ask the assistant what this folder contains", "ai describe"),
        A::AiFindRelated => (ph::FLOW_ARROW, "Find files related to the selection", "ai similar"),
        A::AiSmartSearch => (
            ph::MAGNIFYING_GLASS,
            "Type a search in plain words; edit the query before it runs",
            "natural language ai find",
        ),
        A::AiAuditLog => {
            (ph::SCROLL, "Every prompt, tool call, plan, approval and job of the assistant", "ai history log privacy")
        }
        A::AiRecipes => (ph::BOOK_OPEN, "Saved assistant plans you can re-run without a model", "ai macro recipe"),
        A::AiStop => (ph::STOP_CIRCLE, "Cancel the running assistant task", "ai cancel escape"),
        A::CustomCommand(_) => (ph::LIGHTNING, "Custom command", "user"),
        A::RunScript(_) => (ph::CODE, "Script action", "rhai script user"),
        A::OpenScripts => (ph::CODE, "Your Rhai scripts: actions, columns, previewers; errors", "rhai plugin"),
        A::ReloadScripts => (ph::ARROWS_CLOCKWISE, "Load the scripts folder again", "rhai refresh"),
        A::OpenAutomations => (ph::ROBOT, "Rules that act on new files in watched folders", "rules hazel watch"),
        // ---- jobs ----
        A::JobsPauseAll => (ph::PAUSE, "Pause every running job", ""),
        A::JobsResumeAll => (ph::PLAY, "Resume every paused job", ""),
        A::JobsCancelAll => (ph::STOP, "Cancel every job", ""),
        A::JobsClearFinished => (ph::BROOM, "Remove finished jobs from the list", ""),
        A::JobsShowHistory => (ph::CLOCK_COUNTER_CLOCKWISE, "Browse completed operations and undo", ""),
    };
    (icon, desc.to_string(), kw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_action_has_palette_metadata() {
        let mut titles = HashSet::new();
        for a in Action::all() {
            let e = entry(&a);
            assert!(!e.title.trim().is_empty(), "{a}: title");
            assert!(!e.icon.is_empty(), "{a}: icon");
            assert!(!e.description.trim().is_empty(), "{a}: description");
            assert!(titles.insert(e.title.clone()), "duplicate palette title {}", e.title);
        }
        let cc = entry(&Action::CustomCommand("Open in VS Code".into()));
        assert_eq!(cc.title, "Open in VS Code");
    }
}
