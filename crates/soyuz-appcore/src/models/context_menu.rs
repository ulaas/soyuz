//! The file list's context menus as data: what the GUI's right-click menu and the
//! terminal UI's popup menu (Shift+F10, right click) show and run.
//!
//! [`pane_menu`] builds the entries for a pane: the items menu when the menu targets the
//! selection ([`ContextTarget::Items`] with a non-empty selection), else the background
//! menu of the folder. Each frontend draws the [`Entry`] tree its own way (the GUI with its
//! menu widgets and Phosphor glyphs, the terminal UI with `widgets::Menu`) and hands an
//! activated entry's [`Run`] to [`run`]. The GUI's toolbar reuses the View / Sort /
//! Group-by sections ([`view_mode_entries`], [`sort_entries`], [`group_entries`]).
//!
//! Two entries depend on the frontend and are left to it: [`Entry::OpenWith`] (the
//! "Open with" submenu: the app lookup is slow and cached per frontend) and
//! [`Entry::FolderColor`] (colour swatches and emblems).
//!
//! `[context_menu] hide` (action names) leaves the matching action entries out.

use std::path::PathBuf;

use soyuz_core::config::{Action, CommandContext, Config, Keymap};
use soyuz_core::{Column, HashAlgo, Location, SortDir, ViewMode};

use crate::actions::GROUPS;
use crate::cmd::Cmd;
use crate::icon::IconId;
use crate::keys::chord_label;
use crate::models::tools::ToolCmd;
use crate::ops::fileops::FileOps;
use crate::services::folder_colors::FolderColor;
use crate::state::pane::{ContextTarget, Pane, PaneId};

/// The icon of an entry: a core [`IconId`], a few glyphs only menus use, or a colour tag
/// dot (drawn by the frontend).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuIcon {
    None,
    Id(IconId),
    ChartPieSlice,
    Check,
    CheckCircle,
    DotsThree,
    Files,
    Hash,
    ArrowsMerge,
    ArrowsSplit,
    Package,
    Prohibit,
    SlidersHorizontal,
    TrayArrowDown,
    Plus,
    PushPinSlash,
    X,
    ArrowLineRight,
    /// Disconnect (a server).
    Plugs,
    /// Eject a drive (device menus).
    Eject,
    /// Unlock an encrypted volume (device menus).
    LockOpen,
    /// No glyph, but the icon column stays (unticked rows of a ticked list).
    Blank,
    /// A dot of tag colour `0..8` (tab groups).
    Dot(u8),
    /// Colour tag `tag` (0-based), with a check mark when every selected item has it.
    Tag {
        tag: u8,
        checked: bool,
    },
}

/// How an item is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemStyle {
    /// Icon, label, chord.
    Plain,
    /// A check box row (label, chord), no icon.
    Check(bool),
}

/// What activating an entry does ([`run`]).
#[derive(Clone, Debug)]
pub enum Run {
    /// Queue a core command.
    Cmd(Cmd),
    /// Queue a tool command (`models::tools::queue`).
    Tool(ToolCmd),
    /// Clear the recent-files list, then refresh `pane`.
    ClearRecent(PaneId),
    /// Set (or clear) the colour of folders.
    FolderColor(Vec<PathBuf>, Option<FolderColor>),
    /// A terminal session tab's command ([`terminal_tab`]): queued for the frontend that
    /// drew the strip ([`take_terminal_tab_cmds`], [`apply_terminal_tab`]).
    Terminal(TermTabCmd),
}

/// A clickable row.
#[derive(Clone, Debug)]
pub struct Item {
    pub icon: MenuIcon,
    pub label: String,
    /// The key that runs it anyway (or a script / command hotkey hint).
    pub chord: Option<String>,
    pub enabled: bool,
    pub style: ItemStyle,
    /// `None`: activating it only closes the menu.
    pub run: Option<Run>,
    /// Activating it keeps the menu open (column toggles).
    pub keep_open: bool,
    /// Why it is disabled, or what it does: the GUI's hover text (`None`: none).
    pub tip: Option<&'static str>,
}

/// Which submenu (frontends size them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmenuKind {
    View,
    SortBy,
    GroupBy,
    Extract,
    Tags,
    Ai,
    Checksum,
    /// "Add to group" (tab menu).
    TabGroups,
    /// "Move to window" (tab menu).
    Windows,
}

/// A submenu row.
#[derive(Clone, Debug)]
pub struct Submenu {
    pub kind: SubmenuKind,
    pub icon: MenuIcon,
    pub label: String,
    pub items: Vec<Entry>,
}

/// A menu row.
#[derive(Clone, Debug)]
pub enum Entry {
    Item(Item),
    Submenu(Submenu),
    Separator,
    /// A heading row (not clickable).
    Caption(String),
    /// "Open with" for this file: the frontend lists the apps it found.
    OpenWith(PathBuf),
    /// "Folder color" for these folders (swatches, emblems).
    FolderColor(Vec<PathBuf>),
}

/// What the menus depend on besides the pane.
pub struct MenuCtx<'a> {
    pub config: &'a Config,
    pub keymap: &'a Keymap,
    /// Something to paste (Soyuz's clipboard or files on the system clipboard).
    pub has_clip: bool,
    /// Dual pane is shown (copy / move to the other pane).
    pub dual: bool,
    pub show_hidden: bool,
    pub checkbox_mode: bool,
}

/// Whether Paste has something to paste: Soyuz's own clipboard or files on the system
/// clipboard.
pub fn has_clip(fileops: &FileOps) -> bool {
    !fileops.clipboard.is_empty() || crate::services::os_clipboard::has_files()
}

/// Carry out an activated entry.
pub fn run(r: Run, cmds: &mut Vec<Cmd>) {
    match r {
        Run::Cmd(c) => cmds.push(c),
        Run::Tool(t) => crate::models::tools::queue(t),
        Run::ClearRecent(pane) => {
            crate::services::recent::clear();
            cmds.push(Cmd::PaneAction(pane, Action::Refresh));
        }
        Run::FolderColor(dirs, color) => crate::services::folder_colors::set_color(&dirs, color),
        Run::Terminal(c) => TERM_TAB_QUEUE.with_borrow_mut(|q| q.push(c)),
    }
}

/// `[context_menu] hide`: whether `a` is left out.
fn is_hidden(config: &Config, a: &Action) -> bool {
    config.context_menu.hide.iter().any(|n| n.trim().parse::<Action>().is_ok_and(|h| &h == a))
}

/// Parse `[context_menu] hide`; returns warnings for unknown names.
pub fn hide_warnings(names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter(|n| n.trim().parse::<Action>().is_err())
        .map(|n| format!("[context_menu] hide: unknown action '{n}'"))
        .collect()
}

fn item(icon: MenuIcon, label: impl Into<String>, chord: Option<String>, enabled: bool, run: Option<Run>) -> Entry {
    Entry::Item(Item {
        icon,
        label: label.into(),
        chord,
        enabled,
        style: ItemStyle::Plain,
        run,
        keep_open: false,
        tip: None,
    })
}

fn check(on: bool, label: impl Into<String>, chord: Option<String>, run: Option<Run>) -> Entry {
    Entry::Item(Item {
        icon: MenuIcon::None,
        label: label.into(),
        chord,
        enabled: true,
        style: ItemStyle::Check(on),
        run,
        keep_open: false,
        tip: None,
    })
}

fn cmd_item(icon: impl Into<MenuIcon>, label: impl Into<String>, c: Cmd) -> Entry {
    item(icon.into(), label, None, true, Some(Run::Cmd(c)))
}

impl From<IconId> for MenuIcon {
    fn from(id: IconId) -> MenuIcon {
        MenuIcon::Id(id)
    }
}

/// Builds rows that run actions on one pane.
struct B<'a> {
    cx: &'a MenuCtx<'a>,
    pane: PaneId,
    out: Vec<Entry>,
}

impl<'a> B<'a> {
    fn new(cx: &'a MenuCtx<'a>, pane: PaneId) -> B<'a> {
        B { cx, pane, out: Vec::new() }
    }

    fn sub(&self) -> B<'a> {
        B { cx: self.cx, pane: self.pane, out: Vec::new() }
    }

    fn push(&mut self, e: Entry) {
        self.out.push(e);
    }

    fn sep(&mut self) {
        self.out.push(Entry::Separator);
    }

    fn run(&self, a: Action) -> Option<Run> {
        Some(Run::Cmd(Cmd::PaneAction(self.pane, a)))
    }

    /// An action row with its chord, unless `[context_menu] hide` lists the action.
    fn action(&mut self, icon: impl Into<MenuIcon>, label: impl Into<String>, a: Action) {
        if is_hidden(self.cx.config, &a) {
            return;
        }
        self.plain_action(icon, label, a);
    }

    /// An action row with its chord (never hidden).
    fn plain_action(&mut self, icon: impl Into<MenuIcon>, label: impl Into<String>, a: Action) {
        let chord = chord_label(self.cx.keymap, &a);
        let run = self.run(a);
        self.out.push(item(icon.into(), label, chord, true, run));
    }

    /// An action row without a chord (never hidden).
    fn bare_action(&mut self, icon: impl Into<MenuIcon>, label: impl Into<String>, a: Action) {
        let run = self.run(a);
        self.out.push(item(icon.into(), label, None, true, run));
    }

    fn submenu(&mut self, kind: SubmenuKind, icon: impl Into<MenuIcon>, label: impl Into<String>, items: Vec<Entry>) {
        self.out.push(Entry::Submenu(Submenu { kind, icon: icon.into(), label: label.into(), items }));
    }
}

/// The context menu of `pane`: items (when it targets a non-empty selection) or
/// background.
pub fn pane_menu(cx: &MenuCtx<'_>, pane: &Pane) -> Vec<Entry> {
    match pane.context {
        ContextTarget::Items if !pane.selection.is_empty() => items_menu(cx, pane),
        _ => background_menu(cx, pane),
    }
}

/// `[sync] previous_versions` has a backend on this OS.
fn versions_supported() -> bool {
    cfg!(any(windows, target_os = "linux", target_os = "macos"))
}

/// The menu of the selected items.
pub fn items_menu(cx: &MenuCtx<'_>, pane: &Pane) -> Vec<Entry> {
    use IconId as I;
    let mut b = B::new(cx, pane.id);
    let sel = pane.selected_entries();
    let single = sel.len() == 1;
    let all_dirs = sel.iter().all(|e| e.is_dir_like());
    let first = sel.first().map(|e| (e.path.clone(), e.is_dir_like()));

    b.action(if all_dirs { I::FolderOpen } else { I::ArrowSquareOut }, tr!("app-ctx-open"), Action::Open);
    if all_dirs {
        b.action(I::Browsers, tr!("app-ctx-open-in-new-tab"), Action::OpenInNewTab);
        if single && let Some((path, _)) = &first {
            b.push(cmd_item(
                I::SquareSplitHorizontal,
                tr!("app-ctx-open-in-other-pane"),
                Cmd::OpenInOtherPane(Location::Dir(path.clone())),
            ));
        }
    }
    if single
        && !is_hidden(cx.config, &Action::OpenWith)
        && let Some((path, false)) = &first
    {
        b.push(Entry::OpenWith(path.clone()));
    }
    if single && let Some((_, false)) = &first {
        b.action(I::NotePencil, tr!("app-ctx-edit-in-soyuz"), Action::OpenInEditor);
    }
    b.sep();
    b.action(I::Scissors, tr!("app-ctx-cut"), Action::Cut);
    b.action(I::Copy, tr!("app-ctx-copy"), Action::Copy);
    if single
        && cx.has_clip
        && let Some((path, true)) = &first
    {
        b.push(cmd_item(
            I::Clipboard,
            tr!("app-ctx-paste-into-folder"),
            Cmd::FileOp(crate::ops::fileops::FileOpCmd::PasteInto(path.clone())),
        ));
    }
    if single && cx.config.sync.previous_versions && versions_supported() && matches!(pane.location, Location::Dir(_)) {
        b.action(I::ClockCounterClockwise, tr!("app-ctx-previous-versions"), Action::PreviousVersions);
    }
    if cx.dual {
        b.action(I::CopySimple, tr!("app-ctx-copy-to-other-pane"), Action::CopyToOtherPane);
        b.action(I::ArrowBendDownRight, tr!("app-ctx-move-to-other-pane"), Action::MoveToOtherPane);
    }
    b.action(I::PencilSimple, tr!("app-ctx-rename"), Action::Rename);
    b.action(I::CopySimple, tr!("app-ctx-duplicate"), Action::Duplicate);
    b.action(I::Link, tr!("app-ctx-create-link"), Action::NewSymlink);
    b.action(I::Trash, tr!("app-ctx-move-to-trash"), Action::Delete);
    b.action(I::XCircle, tr!("app-ctx-delete-permanently"), Action::DeletePermanent);
    b.sep();
    b.action(
        I::ClipboardText,
        if single { tr!("app-ctx-copy-path") } else { tr!("app-ctx-copy-paths") },
        Action::CopyPath,
    );
    b.action(I::TextAa, if single { tr!("app-ctx-copy-name") } else { tr!("app-ctx-copy-names") }, Action::CopyName);
    if single {
        b.action(I::ArrowSquareUpRight, tr!("app-ctx-reveal-in-system"), Action::RevealInSystem);
    }
    if single && let Some((path, true)) = &first {
        b.action(I::TerminalWindow, tr!("app-ctx-open-terminal-here"), Action::OpenInTerminal);
        b.push(cmd_item(I::ArrowSquareOut, tr!("app-ctx-open-external-terminal"), Cmd::TerminalAt(path.clone())));
        b.push(cmd_item(I::Star, tr!("app-ctx-add-to-favorites"), Cmd::AddFavorite(path.clone())));
    }
    let any_archive = sel.iter().any(|e| !e.is_dir_like() && crate::services::archive::is_browsable(&e.path));
    archive_entries(&mut b, &pane.location, any_archive, !sel.is_empty());
    let sel_paths: Vec<(PathBuf, bool)> = sel.iter().map(|e| (e.path.clone(), e.is_dir_like())).collect();
    tool_item_entries(&mut b, &pane.location, &sel_paths);
    ai_entries(&mut b, true, single && all_dirs);
    if all_dirs && matches!(pane.location, Location::Dir(_)) {
        let dirs: Vec<PathBuf> = sel_paths.iter().map(|(p, _)| p.clone()).collect();
        if !dirs.is_empty() {
            b.push(Entry::FolderColor(dirs));
        }
    }
    if all_dirs {
        b.action(I::ChartPie, tr!("app-ctx-folder-sizes"), Action::FolderSize);
    }
    let kinds = crate::commands::kinds_for(&pane.location, !all_dirs, sel.iter().any(|e| e.is_dir_like()), false);
    command_entries(&mut b, &kinds);
    let paths: Vec<PathBuf> = sel_paths.iter().map(|(p, _)| p.clone()).collect();
    script_entries(&mut b, &kinds, pane.location.as_fs_path().map(|p| p.to_path_buf()), &paths);
    b.sep();
    let single_file = single && first.as_ref().is_some_and(|(_, dir)| !dir);
    properties_entries(&mut b, sel.len() > 1, single_file);
    b.action(I::Info, tr!("app-ctx-properties"), Action::Properties);
    b.out
}

/// The menu of the folder background.
pub fn background_menu(cx: &MenuCtx<'_>, pane: &Pane) -> Vec<Entry> {
    use IconId as I;
    let mut b = B::new(cx, pane.id);
    b.submenu(SubmenuKind::View, I::SquaresFour, tr!("app-ctx-view"), view_mode_entries(cx.keymap, pane));
    b.submenu(SubmenuKind::SortBy, I::SortAscending, tr!("app-ctx-sort-by"), sort_entries(pane));
    b.submenu(SubmenuKind::GroupBy, I::Rows, tr!("app-ctx-group-by"), group_entries(pane));
    b.sep();
    b.action(I::ArrowClockwise, tr!("app-ctx-refresh"), Action::Refresh);
    if cx.has_clip {
        b.action(I::Clipboard, tr!("app-ctx-paste"), Action::Paste);
        b.action(I::Link, tr!("app-ctx-paste-as-link"), Action::PasteAsLink);
        b.action(I::SealCheck, tr!("app-ctx-paste-and-verify"), Action::PasteAndVerify);
    } else {
        b.push(item(I::Clipboard.into(), tr!("app-ctx-paste"), chord_label(cx.keymap, &Action::Paste), false, None));
    }
    b.action(I::FolderPlus, tr!("app-ctx-new-folder"), Action::NewFolder);
    b.action(I::FilePlus, tr!("app-ctx-new-file"), Action::NewFile);
    b.action(I::Link, tr!("app-ctx-new-link"), Action::NewSymlink);
    b.sep();
    if let Location::Dir(dir) = &pane.location {
        b.action(I::TerminalWindow, tr!("app-ctx-open-terminal-here"), Action::OpenTerminalHere);
        b.push(cmd_item(I::ArrowSquareOut, tr!("app-ctx-open-external-terminal"), Cmd::TerminalAt(dir.clone())));
        b.action(I::ArrowSquareUpRight, tr!("app-ctx-reveal-in-system"), Action::RevealInSystem);
        b.action(I::ClipboardText, tr!("app-ctx-copy-folder-path"), Action::CopyPath);
        b.push(cmd_item(I::Star, tr!("app-ctx-add-to-favorites"), Cmd::AddFavorite(dir.clone())));
        ai_entries(&mut b, false, true);
        b.sep();
    }
    if matches!(pane.location, Location::Archive { .. }) {
        archive_entries(&mut b, &pane.location, false, false);
        b.sep();
    }
    tool_background_entries(&mut b, &pane.location, cx.dual);
    let hidden = b.run(Action::ToggleHidden);
    b.push(check(cx.show_hidden, tr!("app-ctx-show-hidden"), chord_label(cx.keymap, &Action::ToggleHidden), hidden));
    let boxes = b.run(Action::ToggleCheckboxes);
    b.push(check(cx.checkbox_mode, tr!("app-ctx-item-checkboxes"), None, boxes));
    b.action(I::SelectionAll, tr!("app-ctx-select-all"), Action::SelectAll);
    b.action(I::ChartPie, tr!("app-ctx-folder-sizes"), Action::FolderSize);
    command_entries(&mut b, &[CommandContext::Background]);
    script_entries(&mut b, &[CommandContext::Background], pane.location.as_fs_path().map(|p| p.to_path_buf()), &[]);
    b.sep();
    b.action(I::Info, tr!("app-ctx-properties"), Action::Properties);
    b.out
}

/// The view modes: icon, action (menus, toolbar, status bar switcher).
pub const MODES: [(ViewMode, IconId, Action); 5] = [
    (ViewMode::Details, IconId::List, Action::ViewDetails),
    (ViewMode::List, IconId::ListDashes, Action::ViewList),
    (ViewMode::Grid, IconId::SquaresFour, Action::ViewGrid),
    (ViewMode::Gallery, IconId::Images, Action::ViewGallery),
    (ViewMode::Miller, IconId::Columns, Action::ViewMiller),
];

/// Translated name of a view mode (menus, status bar switcher).
pub fn mode_label(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::Details => tr!("app-view-details"),
        ViewMode::List => tr!("app-view-list"),
        ViewMode::Grid => tr!("app-view-icons"),
        ViewMode::Gallery => tr!("app-view-gallery"),
        ViewMode::Miller => tr!("app-view-columns"),
    }
}

/// The View submenu: one row per mode, the current one checked (its icon is a check).
pub fn view_mode_entries(km: &Keymap, pane: &Pane) -> Vec<Entry> {
    MODES
        .iter()
        .map(|(mode, icon, action)| {
            let checked = pane.view.mode == *mode;
            let icon = if checked { MenuIcon::Check } else { MenuIcon::Id(*icon) };
            let run = Some(Run::Cmd(Cmd::PaneAction(pane.id, action.clone())));
            item(icon, mode_label(*mode), chord_label(km, action), true, run)
        })
        .collect()
}

/// The columns the Sort submenu offers.
pub const SORT_COLUMNS: [Column; 7] =
    [Column::Name, Column::Modified, Column::Size, Column::Kind, Column::Extension, Column::Created, Column::Accessed];

/// The Sort submenu: columns, then the direction.
pub fn sort_entries(pane: &Pane) -> Vec<Entry> {
    let mut out = Vec::new();
    for col in SORT_COLUMNS {
        let on = pane.view.sort.column == col;
        let run = (!on).then_some(Run::Cmd(Cmd::PaneAction(pane.id, Action::SortBy(col))));
        out.push(check(on, crate::i18n::labels::column(col), None, run));
    }
    out.push(Entry::Separator);
    for (dir, label) in [(SortDir::Asc, tr!("app-sort-ascending")), (SortDir::Desc, tr!("app-sort-descending"))] {
        let on = pane.view.sort.dir == dir;
        let run = (!on).then_some(Run::Cmd(Cmd::PaneAction(pane.id, Action::ToggleSortDir)));
        out.push(check(on, label, None, run));
    }
    out
}

/// The Group-by submenu.
pub fn group_entries(pane: &Pane) -> Vec<Entry> {
    GROUPS
        .iter()
        .map(|&(g, _)| {
            check(
                pane.view.group == g,
                crate::palette::catalog::group_label(g),
                None,
                Some(Run::Cmd(Cmd::PaneAction(pane.id, Action::SetGroupBy(g)))),
            )
        })
        .collect()
}

/// Archive rows: extract / compress (items), or extract / edit inside a browsed archive.
fn archive_entries(b: &mut B<'_>, location: &Location, any_archive: bool, has_selection: bool) {
    use IconId as I;
    match location {
        Location::Archive { archive, .. } => {
            b.sep();
            b.bare_action(
                MenuIcon::TrayArrowDown,
                if has_selection { tr!("views-archive-extract-selected") } else { tr!("views-archive-extract-all") },
                Action::ExtractTo,
            );
            b.bare_action(I::ArrowLineDown, tr!("views-archive-extract-here"), Action::ExtractHere);
            if crate::models::archive::is_editable_zip_name(archive) {
                b.sep();
                b.bare_action(I::FilePlus, tr!("views-archive-add-here"), Action::Paste);
                if has_selection {
                    b.bare_action(I::PencilSimple, tr!("views-archive-rename-menu"), Action::Rename);
                    b.bare_action(I::Trash, tr!("views-archive-delete-menu"), Action::DeletePermanent);
                }
            }
        }
        _ => {
            if any_archive {
                b.sep();
                let mut s = b.sub();
                s.bare_action(I::ArrowLineDown, tr!("views-archive-extract-here"), Action::ExtractHere);
                s.bare_action(I::FolderSimple, tr!("views-archive-extract-to-folder"), Action::ExtractToFolder);
                s.bare_action(MenuIcon::TrayArrowDown, tr!("views-archive-extract-to"), Action::ExtractTo);
                s.sep();
                s.bare_action(MenuIcon::CheckCircle, tr!("views-archive-test"), Action::TestArchive);
                b.submenu(SubmenuKind::Extract, I::FileZip, tr!("views-archive-extract"), s.out);
            }
            if has_selection && matches!(location, Location::Dir(_)) {
                b.bare_action(MenuIcon::Package, tr!("views-archive-compress-menu"), Action::CreateArchive);
            }
        }
    }
}

/// Tool rows of the items menu: tags, shelf, disk usage, duplicates, compare, split / join.
fn tool_item_entries(b: &mut B<'_>, location: &Location, sel: &[(PathBuf, bool)]) {
    use crate::services::tags::{self, TAG_COUNT};
    use IconId as I;
    if sel.is_empty() {
        return;
    }
    let paths: Vec<PathBuf> = sel.iter().map(|(p, _)| p.clone()).collect();
    b.sep();
    if matches!(location, Location::Recent) {
        b.push(item(
            I::ClockCounterClockwise.into(),
            tr!("tools-menu-remove-from-recent"),
            None,
            true,
            Some(Run::Tool(ToolCmd::ForgetRecent(paths.clone()))),
        ));
    }
    let masks: Vec<u8> = paths.iter().map(|p| tags::tags_of(p)).collect();
    let mut s = Vec::new();
    for t in 0..TAG_COUNT as u8 {
        let all = masks.iter().all(|m| m & (1 << t) != 0);
        s.push(item(
            MenuIcon::Tag { tag: t, checked: all },
            tags::name(t),
            chord_label(b.cx.keymap, &Action::AddTag(t + 1)),
            true,
            Some(Run::Tool(ToolCmd::ToggleTag { paths: paths.clone(), tag: t })),
        ));
    }
    s.push(Entry::Separator);
    let any = masks.iter().any(|m| *m != 0);
    s.push(item(
        I::XCircle.into(),
        tr!("tools-tags-clear"),
        None,
        any,
        Some(Run::Tool(ToolCmd::ClearTags(paths.clone()))),
    ));
    b.submenu(SubmenuKind::Tags, I::Tag, tr!("tools-tags-menu"), s);
    b.push(item(
        MenuIcon::TrayArrowDown,
        tr!("tools-menu-add-to-shelf"),
        None,
        true,
        Some(Run::Tool(ToolCmd::ShelfAdd(paths.clone()))),
    ));
    let dirs = sel.iter().filter(|(_, d)| *d).count();
    if dirs == 1 && sel.len() == 1 {
        b.plain_action(MenuIcon::ChartPieSlice, tr!("tools-menu-analyze-disk-usage"), Action::DiskUsageTreemap);
    }
    if dirs > 0 && dirs == sel.len() {
        b.plain_action(MenuIcon::Files, tr!("tools-menu-find-duplicates"), Action::FindDuplicates);
    }
    if dirs == 2 && sel.len() == 2 {
        b.plain_action(I::GitDiff, tr!("tools-menu-compare-folders"), Action::CompareFolders);
    }
    if let [(p, false)] = sel {
        if crate::models::tools::is_part(p) {
            b.plain_action(MenuIcon::ArrowsMerge, tr!("tools-menu-join-parts"), Action::JoinFiles);
        } else {
            b.plain_action(MenuIcon::ArrowsSplit, tr!("tools-menu-split-file"), Action::SplitFile);
        }
    }
}

/// Tool rows of the background menu.
fn tool_background_entries(b: &mut B<'_>, location: &Location, dual: bool) {
    use IconId as I;
    match location {
        Location::Dir(_) => {
            b.plain_action(MenuIcon::ChartPieSlice, tr!("tools-menu-analyze-disk-usage"), Action::DiskUsageTreemap);
            b.plain_action(MenuIcon::Files, tr!("tools-menu-find-duplicates"), Action::FindDuplicates);
            b.plain_action(
                I::GitDiff,
                if dual { tr!("tools-menu-compare-other-pane") } else { tr!("tools-menu-compare-folders-dialog") },
                Action::CompareFolders,
            );
            b.sep();
        }
        Location::Recent => {
            let pane = b.pane;
            b.push(item(I::Broom.into(), tr!("tools-menu-clear-recent"), None, true, Some(Run::ClearRecent(pane))));
            b.sep();
        }
        _ => {}
    }
}

/// The AI assistant submenu.
fn ai_entries(b: &mut B<'_>, items: bool, single_dir: bool) {
    use IconId as I;
    let mut s = b.sub();
    if items {
        s.plain_action(I::TextT, tr!("ai-menu-suggest-names"), Action::AiSuggestNames);
        s.plain_action(I::Notepad, tr!("ai-menu-summarize"), Action::AiSummarize);
        s.plain_action(I::FlowArrow, tr!("ai-menu-find-related"), Action::AiFindRelated);
    }
    if !items || single_dir {
        s.plain_action(I::Question, tr!("ai-menu-explain-folder"), Action::AiExplainFolder);
    }
    s.plain_action(I::MagicWand, tr!("ai-menu-organize"), Action::AiOrganize);
    s.sep();
    s.plain_action(I::MagnifyingGlass, tr!("ai-menu-smart-search"), Action::AiSmartSearch);
    b.submenu(SubmenuKind::Ai, I::Sparkle, tr!("ai-menu-assistant"), s.out);
}

/// User-defined commands (`[[custom_commands]]`) for `kinds`.
fn command_entries(b: &mut B<'_>, kinds: &[CommandContext]) {
    let list = crate::commands::context_entries(b.cx.config, kinds);
    if list.is_empty() {
        return;
    }
    b.sep();
    for c in list {
        let icon = if c.terminal { IconId::Terminal } else { IconId::Lightning };
        let hint = (!c.hotkey.trim().is_empty()).then(|| c.hotkey.clone());
        let run = b.run(Action::CustomCommand(c.name.clone()));
        b.push(item(icon.into(), c.name.clone(), hint, true, run));
    }
}

/// Script actions whose conditions hold for the selection.
fn script_entries(b: &mut B<'_>, kinds: &[CommandContext], dir: Option<PathBuf>, selected: &[PathBuf]) {
    let list = crate::automation::menu::context_entries(kinds, dir, selected);
    if list.is_empty() {
        return;
    }
    b.sep();
    for e in list {
        let hint = (!e.hotkey.trim().is_empty()).then(|| e.hotkey.clone());
        let run = b.run(Action::RunScript(e.id));
        b.push(item(IconId::Code.into(), e.title, hint, true, run));
    }
}

/// The checksum algorithms in menu order.
pub const CHECKSUM_ALGOS: [HashAlgo; 5] =
    [HashAlgo::Md5, HashAlgo::Sha1, HashAlgo::Sha256, HashAlgo::Blake3, HashAlgo::Crc32];

/// Batch rename, attributes, checksums.
fn properties_entries(b: &mut B<'_>, multi: bool, single_file: bool) {
    use IconId as I;
    if multi {
        b.plain_action(I::Textbox, tr!("dialogs-menu-batch-rename"), Action::BatchRename);
    }
    b.plain_action(MenuIcon::SlidersHorizontal, tr!("dialogs-menu-attributes"), Action::SetAttributes);
    if single_file {
        let mut s = b.sub();
        for a in CHECKSUM_ALGOS {
            s.plain_action(MenuIcon::Hash, a.label(), Action::Checksum(a));
        }
        s.plain_action(MenuIcon::CheckCircle, tr!("dialogs-menu-compare"), Action::CompareChecksum);
        b.submenu(SubmenuKind::Checksum, I::Fingerprint, tr!("dialogs-menu-checksum"), s.out);
    }
}

/// Translated name of colour tag `i` (`services::tags::TAG_NAMES` order).
pub fn color_name(i: usize) -> &'static str {
    match i {
        0 => tr!("app-color-red"),
        1 => tr!("app-color-orange"),
        2 => tr!("app-color-yellow"),
        3 => tr!("app-color-green"),
        4 => tr!("app-color-cyan"),
        5 => tr!("app-color-blue"),
        6 => tr!("app-color-purple"),
        _ => tr!("app-color-gray"),
    }
}

/// A plain "Folder color" submenu (frontends without swatches): one row per colour, the
/// current one checked, then "No color".
pub fn folder_color_entries(dirs: &[PathBuf]) -> Vec<Entry> {
    let current = dirs.first().and_then(|d| crate::services::folder_colors::style_of(d)).unwrap_or_default();
    let mut out: Vec<Entry> = (0..crate::services::tags::TAG_NAMES.len())
        .map(|i| {
            let color = FolderColor::Tag(i as u8);
            check(current.color == Some(color), color_name(i), None, Some(Run::FolderColor(dirs.to_vec(), Some(color))))
        })
        .collect();
    out.push(Entry::Separator);
    out.push(item(
        MenuIcon::Prohibit,
        tr!("app-looks-no-color"),
        None,
        current.color.is_some(),
        Some(Run::FolderColor(dirs.to_vec(), None)),
    ));
    out
}

// ---- sidebar places, tabs, the details header ----------------------------------------

/// The menu of a sidebar place at `path` (favourites, bookmarks, drives, cloud folders):
/// open in a new tab / the other pane, reveal, copy the path, and add to (or, for a
/// favourite, `removable`, remove from) the favourites.
pub fn place_menu(path: &std::path::Path, removable: bool) -> Vec<Entry> {
    use IconId as I;
    let loc = || Location::Dir(path.to_path_buf());
    let mut out = vec![
        cmd_item(I::Browsers, tr!("app-ctx-open-in-new-tab"), Cmd::OpenInNewTab { location: loc(), activate: true }),
        cmd_item(I::SquareSplitHorizontal, tr!("app-ctx-open-in-other-pane"), Cmd::OpenInOtherPane(loc())),
        cmd_item(I::ArrowSquareUpRight, tr!("app-ctx-reveal-in-system"), Cmd::RevealInSystem(path.to_path_buf())),
        cmd_item(
            I::ClipboardText,
            tr!("app-ctx-copy-path"),
            Cmd::CopyText(soyuz_fs::strip_verbatim(path).to_string_lossy().into_owned()),
        ),
        Entry::Separator,
    ];
    out.push(if removable {
        cmd_item(I::Star, tr!("app-sidebar-remove-favorite"), Cmd::RemoveFavorite(path.to_path_buf()))
    } else {
        cmd_item(I::Star, tr!("app-ctx-add-to-favorites"), Cmd::AddFavorite(path.to_path_buf()))
    });
    out
}

/// The menu of tab `i`: new / duplicate / pin, close (others, to the right), reopen
/// closed; tab groups, move to a window, the tab's history.
pub fn tab_menu(core: &crate::core::AppCore, i: usize) -> Vec<Entry> {
    use crate::models::workspaces::WsCmd;
    use IconId as I;
    let Some(tab) = core.ws.tabs.get(i) else { return Vec::new() };
    let km = &core.keymap;
    let n = core.ws.tabs.len();
    let pinned = tab.pinned;
    let sc = |a: &Action| chord_label(km, a);
    let run = |c: Cmd| Some(Run::Cmd(c));
    let mut out = vec![
        item(MenuIcon::Plus, tr!("app-tabs-new"), sc(&Action::NewTab), true, run(Cmd::NewTabAfter(i))),
        item(
            I::CopySimple.into(),
            tr!("app-tabs-duplicate"),
            sc(&Action::DuplicateTab),
            true,
            run(Cmd::DuplicateTab(i)),
        ),
    ];
    let (pin_icon, pin_label) =
        if pinned { (MenuIcon::PushPinSlash, tr!("app-tabs-unpin")) } else { (I::PushPin.into(), tr!("app-tabs-pin")) };
    out.push(item(pin_icon, pin_label, sc(&Action::PinTab), true, run(Cmd::PinTab(i, !pinned))));
    out.push(Entry::Separator);
    out.push(item(MenuIcon::X, tr!("app-tabs-close"), sc(&Action::CloseTab), n > 1, run(Cmd::CloseTab(i))));
    out.push(item(I::XCircle.into(), tr!("app-tabs-close-others"), None, n > 1, run(Cmd::CloseOtherTabs(i))));
    out.push(item(
        MenuIcon::ArrowLineRight,
        tr!("app-tabs-close-right"),
        None,
        i + 1 < n,
        run(Cmd::CloseTabsToRight(i)),
    ));
    out.push(Entry::Separator);
    out.push(item(
        I::ArrowCounterClockwise.into(),
        tr!("app-tabs-reopen-closed"),
        sc(&Action::ReopenClosedTab),
        true,
        run(Cmd::Action(Action::ReopenClosedTab)),
    ));
    // Groups, windows, history (the GUI's `workspaces::tab_menu_extra`).
    out.push(Entry::Separator);
    if !pinned {
        out.push(cmd_item(
            I::RectangleDashed,
            tr!("workspaces-menu-add-new-group"),
            Cmd::Workspace(WsCmd::NewGroup(i)),
        ));
        let others: Vec<Entry> = core
            .ws
            .groups
            .iter()
            .filter(|g| Some(g.id) != tab.group)
            .map(|g| {
                let label =
                    if g.name.is_empty() { tr!("workspaces-unnamed-group").to_string() } else { g.name.clone() };
                cmd_item(MenuIcon::Dot(g.color), label, Cmd::Workspace(WsCmd::AddToGroup(i, g.id)))
            })
            .collect();
        if !others.is_empty() {
            out.push(Entry::Submenu(Submenu {
                kind: SubmenuKind::TabGroups,
                icon: I::Stack.into(),
                label: tr!("workspaces-menu-add-to-group").into(),
                items: others,
            }));
        }
        if tab.group.is_some() {
            out.push(cmd_item(
                I::Rectangle,
                tr!("workspaces-menu-remove-from-group"),
                Cmd::Workspace(WsCmd::RemoveFromGroup(i)),
            ));
        }
    }
    out.push(Entry::Separator);
    let can_move = n > 1;
    out.push(item(
        I::AppWindow.into(),
        tr!("workspaces-menu-move-new-window"),
        None,
        can_move,
        run(Cmd::Workspace(WsCmd::TabToNewWindow(i))),
    ));
    let here = core.workspaces.current;
    let others: Vec<Entry> = crate::models::windows::window_list(core)
        .into_iter()
        .filter(|(id, _)| *id != here)
        .map(|(id, title)| {
            item(I::AppWindow.into(), title, None, can_move, run(Cmd::Workspace(WsCmd::TabToWindow(i, id))))
        })
        .collect();
    if !others.is_empty() {
        out.push(Entry::Submenu(Submenu {
            kind: SubmenuKind::Windows,
            icon: I::Browsers.into(),
            label: tr!("workspaces-menu-move-to-window").into(),
            items: others,
        }));
    }
    out.push(item(
        I::GitFork.into(),
        tr!("workspaces-history-title"),
        sc(&Action::TabHistory),
        true,
        run(Cmd::Action(Action::TabHistory)),
    ));
    out
}

/// Show / hide a details column of a pane, or reset the columns (the header menu).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColumnCmd {
    Show { pane: PaneId, column: Column, visible: bool },
    Reset(PaneId),
}

/// The column chooser of `pane`'s details header: "Columns", a tick per column (Name
/// stays; activating a row keeps the menu open), "Reset columns".
pub fn column_menu(pane: &Pane) -> Vec<Entry> {
    use crate::models::requests::columns;
    let mut out = vec![Entry::Caption(tr!("app-ctx-columns").into())];
    for col in columns::choosable() {
        let on = columns::is_visible(pane, col);
        out.push(Entry::Item(Item {
            icon: if on { MenuIcon::Check } else { MenuIcon::Blank },
            label: columns::label(col),
            chord: None,
            enabled: columns::can_toggle(col),
            style: ItemStyle::Plain,
            run: Some(Run::Cmd(Cmd::Column(ColumnCmd::Show { pane: pane.id, column: col, visible: !on }))),
            keep_open: true,
            tip: None,
        }));
    }
    out.push(Entry::Separator);
    out.push(cmd_item(
        IconId::ArrowCounterClockwise,
        tr!("app-ctx-reset-columns"),
        Cmd::Column(ColumnCmd::Reset(pane.id)),
    ));
    out
}

/// Carry out a [`ColumnCmd`] (`Cmd::Column`); the view is remembered (`Cmd::ViewChanged`).
pub fn apply_column(app: &mut crate::core::AppCore, c: ColumnCmd) {
    let id = match c {
        ColumnCmd::Show { pane, column, visible } => {
            let Some(p) = app.ws.find_pane_mut(pane) else { return };
            if !crate::models::requests::columns::set_visible(p, column, visible) {
                return;
            }
            pane
        }
        ColumnCmd::Reset(pane) => {
            let columns = crate::state::view_memory::ViewState::from_config(&app.config).columns;
            let Some(p) = app.ws.find_pane_mut(pane) else { return };
            p.view.columns = columns;
            p.layout_sig = None;
            pane
        }
    };
    app.cmds.push(Cmd::ViewChanged(id));
}

/// What a terminal session tab's menu ([`terminal_tab`]) runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermTabCmd {
    /// Start renaming session `.0` (its label as the text).
    Rename(u64),
    /// A new session in the focused pane's folder.
    New,
    /// An external terminal in session `.0`'s folder.
    External(u64),
    /// Close (and kill) session `.0`.
    Close(u64),
}

thread_local! {
    static TERM_TAB_QUEUE: std::cell::RefCell<Vec<TermTabCmd>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The menu of terminal session `id` (the session strip's right click): rename, new
/// session, external terminal, close. Activated entries queue a [`TermTabCmd`]
/// ([`run`]); the frontend takes them with [`take_terminal_tab_cmds`].
pub fn terminal_tab(id: u64) -> Vec<Entry> {
    let run = |c: TermTabCmd| Some(Run::Terminal(c));
    vec![
        item(IconId::PencilSimple.into(), tr!("common-rename"), None, true, run(TermTabCmd::Rename(id))),
        item(MenuIcon::Plus, tr!("panels-terminal-new"), None, true, run(TermTabCmd::New)),
        item(
            IconId::ArrowSquareOut.into(),
            tr!("panels-terminal-open-external"),
            None,
            true,
            run(TermTabCmd::External(id)),
        ),
        Entry::Separator,
        item(MenuIcon::X, tr!("common-close"), None, true, run(TermTabCmd::Close(id))),
    ]
}

/// The terminal tab commands activated since the last call (this thread's).
pub fn take_terminal_tab_cmds() -> Vec<TermTabCmd> {
    TERM_TAB_QUEUE.with_borrow_mut(std::mem::take)
}

/// Carry out a [`TermTabCmd`].
pub fn apply_terminal_tab(app: &mut crate::core::AppCore, host: &dyn crate::host::Host, c: TermTabCmd) {
    use crate::models::terminal as term;
    match c {
        TermTabCmd::Rename(id) => {
            if let Some(label) = app.ui.terminal.session(id).map(term::Session::label) {
                app.ui.terminal.renaming = Some((id, label));
                app.ui.terminal.focused = true;
            }
        }
        TermTabCmd::New => term::new_session(app, host),
        TermTabCmd::External(id) => term::open_external(app, id),
        TermTabCmd::Close(id) => term::close_session(app, id),
    }
}

// ---- sidebar locations: smart folders, saved searches, servers (Phase T5) -------------------

/// The menu of smart folder `i` (a sidebar row): open, open in a new tab, edit, delete
/// (the GUI's `ui::smart::sidebar_section`), then "New smart folder".
pub fn smart_folder(i: usize) -> Vec<Entry> {
    use crate::models::smart::SmartCmd;
    let mut out = smart_folder_row(i);
    out.push(Entry::Separator);
    out.push(cmd_item(MenuIcon::Plus, tr!("views-smart-new"), Cmd::Smart(SmartCmd::Edit(None))));
    out
}

/// The GUI's smart folder row menu: [`smart_folder`] without "New smart folder" (the GUI
/// has a "New smart folder" row right under the list).
pub fn smart_folder_row(i: usize) -> Vec<Entry> {
    use crate::models::smart::SmartCmd;
    use IconId as I;
    vec![
        cmd_item(I::FolderOpen, tr!("views-smart-open"), Cmd::Smart(SmartCmd::Open { index: i, new_tab: false })),
        cmd_item(
            MenuIcon::Plus,
            tr!("views-smart-open-new-tab"),
            Cmd::Smart(SmartCmd::Open { index: i, new_tab: true }),
        ),
        Entry::Separator,
        cmd_item(I::PencilSimple, tr!("views-smart-edit"), Cmd::Smart(SmartCmd::Edit(Some(i)))),
        cmd_item(I::Trash, tr!("views-smart-delete"), Cmd::Smart(SmartCmd::Delete(i))),
    ]
}

/// The menu of the smart folders section (its header, the "New smart folder" row): new
/// smart folder, the index settings, rebuild the index (not when the index failed) —
/// the GUI's index status chip menu.
pub fn smart_section(index_failed: bool) -> Vec<Entry> {
    use crate::models::smart::SmartCmd;
    use IconId as I;
    vec![
        cmd_item(MenuIcon::Plus, tr!("views-smart-new"), Cmd::Smart(SmartCmd::Edit(None))),
        Entry::Separator,
        cmd_item(I::GearSix, tr!("views-index-settings"), Cmd::Smart(SmartCmd::OpenSettings)),
        item(
            I::ArrowsClockwise.into(),
            tr!("views-index-rebuild"),
            None,
            !index_failed,
            (!index_failed).then_some(Run::Cmd(Cmd::Smart(SmartCmd::RebuildIndex))),
        ),
    ]
}

/// The menu of saved search `i` (a sidebar row): run, remove. The first `n_config`
/// searches come from `config.toml` and are removed there, not here.
pub fn saved_search(i: usize, n_config: usize) -> Vec<Entry> {
    let removable = i >= n_config;
    let mut remove = item(
        MenuIcon::X,
        tr!("views-search-remove"),
        None,
        removable,
        removable.then_some(Run::Cmd(Cmd::RemoveSavedSearch(i))),
    );
    if let Entry::Item(it) = &mut remove
        && !removable
    {
        it.tip = Some(tr!("views-search-defined-in-config"));
    }
    vec![cmd_item(IconId::MagnifyingGlass, tr!("views-search-run-saved"), Cmd::RunSavedSearch(i)), remove]
}

/// The menu of a saved connection `conn` (a sidebar "Network" row) at `uri` in `state`
/// (`saved`: stored, not a debug server): open in a new tab, edit, disconnect, copy the
/// address, remove from the sidebar (the GUI's `ui::remote::sidebar_section`).
pub fn remote_connection(
    conn: &crate::services::remote::SavedConn,
    uri: &soyuz_vfs::RemoteUri,
    state: &soyuz_vfs::ConnState,
    saved: bool,
) -> Vec<Entry> {
    use crate::models::remote::RemoteCmd;
    use IconId as I;
    let address = uri.display();
    let connected = crate::models::remote::can_disconnect(state);
    let edit = Cmd::Remote(RemoteCmd::ConnectDialog { uri: None, edit: Some(conn.id.clone()), note: None });
    let mut out = vec![
        cmd_item(
            I::Browsers,
            tr!("remote-open-new-tab"),
            Cmd::Remote(RemoteCmd::Open { uri: address.clone(), new_tab: true }),
        ),
        item(I::PencilSimple.into(), tr!("remote-edit-connection"), None, saved, saved.then_some(Run::Cmd(edit))),
        item(
            MenuIcon::Plugs,
            tr!("remote-disconnect"),
            None,
            connected,
            connected.then(|| Run::Cmd(Cmd::Remote(RemoteCmd::Disconnect(address.clone())))),
        ),
        cmd_item(I::ClipboardText, tr!("remote-copy-address"), Cmd::CopyText(address)),
    ];
    if saved {
        out.push(Entry::Separator);
        out.push(cmd_item(I::Trash, tr!("remote-remove"), Cmd::Remote(RemoteCmd::Remove(conn.id.clone()))));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::view_memory::ViewState;

    fn labels(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| match e {
                Entry::Item(i) => i.label.clone(),
                Entry::Submenu(s) => format!("{} >", s.label),
                Entry::Separator => "-".into(),
                Entry::Caption(c) => format!("# {c}"),
                Entry::OpenWith(_) => "open with".into(),
                Entry::FolderColor(_) => "folder color".into(),
            })
            .collect()
    }

    fn ctx<'a>(config: &'a Config, km: &'a Keymap, has_clip: bool) -> MenuCtx<'a> {
        MenuCtx { config, keymap: km, has_clip, dual: false, show_hidden: false, checkbox_mode: false }
    }

    #[test]
    fn background_menu_follows_the_clipboard_and_hide() {
        let mut config = Config::default();
        let km = Keymap::default();
        let pane = Pane::new(Location::Dir(std::env::temp_dir()), ViewState::from_config(&config));
        let without = background_menu(&ctx(&config, &km, false), &pane);
        let l = labels(&without);
        assert!(l[0].ends_with('>') && l[1].ends_with('>') && l[2].ends_with('>'), "{l:?}");
        let paste = without
            .iter()
            .find_map(|e| match e {
                Entry::Item(i) if i.label == tr!("app-ctx-paste") => Some(i),
                _ => None,
            })
            .expect("paste row");
        assert!(!paste.enabled && paste.run.is_none());
        let with = labels(&background_menu(&ctx(&config, &km, true), &pane));
        assert!(with.iter().any(|s| s == tr!("app-ctx-paste-as-link")));
        // `[context_menu] hide` drops the action rows.
        config.context_menu.hide = vec!["refresh".into()];
        let hidden = labels(&background_menu(&ctx(&config, &km, false), &pane));
        assert!(!hidden.iter().any(|s| s == tr!("app-ctx-refresh")));
        assert!(l.iter().any(|s| s == tr!("app-ctx-refresh")));
        assert_eq!(hide_warnings(&["refresh".into(), "nope".into()]).len(), 1);
    }

    #[test]
    fn sort_and_view_sections() {
        let config = Config::default();
        let km = Keymap::default();
        let pane = Pane::new(Location::Dir(std::env::temp_dir()), ViewState::from_config(&config));
        let view = view_mode_entries(&km, &pane);
        assert_eq!(view.len(), MODES.len());
        let checked = view.iter().filter(|e| matches!(e, Entry::Item(i) if i.icon == MenuIcon::Check)).count();
        assert_eq!(checked, 1);
        let sort = sort_entries(&pane);
        // The current column and direction are checked and run nothing.
        let on: Vec<&Item> = sort
            .iter()
            .filter_map(|e| match e {
                Entry::Item(i) if i.style == ItemStyle::Check(true) => Some(i),
                _ => None,
            })
            .collect();
        assert_eq!(on.len(), 2);
        assert!(on.iter().all(|i| i.run.is_none()));
        assert_eq!(group_entries(&pane).len(), GROUPS.len());
    }

    #[test]
    fn place_and_column_menus() {
        let p = std::env::temp_dir();
        let fav = labels(&place_menu(&p, true));
        assert_eq!(fav.last().map(String::as_str), Some(tr!("app-sidebar-remove-favorite")));
        assert_eq!(fav[0], tr!("app-ctx-open-in-new-tab"));
        let other = labels(&place_menu(&p, false));
        assert_eq!(other.last().map(String::as_str), Some(tr!("app-ctx-add-to-favorites")));
        let config = Config::default();
        let pane = Pane::new(Location::Dir(p), ViewState::from_config(&config));
        let cols = column_menu(&pane);
        assert!(matches!(&cols[0], Entry::Caption(c) if c == tr!("app-ctx-columns")));
        let name = crate::models::requests::columns::label(Column::Name);
        let row = cols
            .iter()
            .find_map(|e| match e {
                Entry::Item(i) if i.label == name => Some(i),
                _ => None,
            })
            .expect("Name");
        assert!(!row.enabled && row.keep_open && row.icon == MenuIcon::Check, "Name stays");
        let ticks = cols.iter().filter(|e| matches!(e, Entry::Item(i) if i.keep_open)).count();
        assert_eq!(ticks, crate::models::requests::columns::choosable().len());
        assert!(
            matches!(cols.last(), Some(Entry::Item(i)) if !i.keep_open && matches!(i.run, Some(Run::Cmd(Cmd::Column(ColumnCmd::Reset(_))))))
        );
    }

    #[test]
    fn run_queues_commands() {
        let mut cmds = Vec::new();
        run(Run::Cmd(Cmd::PaneAction(PaneId::next(), Action::Refresh)), &mut cmds);
        assert_eq!(cmds.len(), 1);
    }

    #[test]
    fn terminal_tab_menu_queues_its_commands() {
        let entries = terminal_tab(7);
        let runs: Vec<Run> = entries
            .into_iter()
            .filter_map(|e| match e {
                Entry::Item(it) => it.run,
                _ => None,
            })
            .collect();
        assert_eq!(runs.len(), 4);
        let mut cmds = Vec::new();
        for r in runs {
            run(r, &mut cmds);
        }
        assert!(cmds.is_empty());
        assert_eq!(
            take_terminal_tab_cmds(),
            vec![TermTabCmd::Rename(7), TermTabCmd::New, TermTabCmd::External(7), TermTabCmd::Close(7)]
        );
        assert!(take_terminal_tab_cmds().is_empty());
    }

    fn runs(entries: Vec<Entry>) -> Vec<Option<Run>> {
        entries
            .into_iter()
            .filter_map(|e| match e {
                Entry::Item(it) => Some(it.run),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn smart_and_saved_search_menus() {
        use crate::models::smart::SmartCmd;
        let l = labels(&smart_folder(2));
        assert_eq!(l[0], tr!("views-smart-open"));
        assert!(l.iter().any(|s| s == tr!("views-smart-delete")));
        let r = runs(smart_folder(2));
        assert!(matches!(&r[2], Some(Run::Cmd(Cmd::Smart(SmartCmd::Edit(Some(2)))))));
        assert!(matches!(&r[3], Some(Run::Cmd(Cmd::Smart(SmartCmd::Delete(2))))));
        let failed = runs(smart_section(true));
        assert!(failed.last().is_some_and(Option::is_none), "no rebuild while the index failed");
        assert!(runs(smart_section(false)).iter().all(Option::is_some));
        // Saved searches from config.toml cannot be removed here.
        let cfg = runs(saved_search(0, 1));
        assert!(matches!(&cfg[0], Some(Run::Cmd(Cmd::RunSavedSearch(0)))) && cfg[1].is_none());
        let own = runs(saved_search(1, 1));
        assert!(matches!(&own[1], Some(Run::Cmd(Cmd::RemoveSavedSearch(1)))));
        // The disabled remove row says why (the GUI's hover text).
        let tip = |e: &[Entry]| match &e[1] {
            Entry::Item(it) => it.tip,
            _ => None,
        };
        assert_eq!(tip(&saved_search(0, 1)), Some(tr!("views-search-defined-in-config")));
        assert_eq!(tip(&saved_search(1, 1)), None);
        // The GUI's row menu: the same rows without "New smart folder".
        let row = labels(&smart_folder_row(2));
        assert_eq!(row[..], labels(&smart_folder(2))[..row.len()]);
        assert!(!row.iter().any(|s| s == tr!("views-smart-new")));
    }

    #[test]
    fn remote_connection_menu() {
        use crate::models::remote::RemoteCmd;
        use crate::services::remote::{AuthKind, SavedConn};
        let uri = soyuz_vfs::RemoteUri::parse("sftp://me@example.org/home").unwrap();
        let conn = SavedConn {
            id: "c1".into(),
            name: "Box".into(),
            uri: uri.display(),
            auth: AuthKind::Password,
            key_file: None,
            has_secret: false,
        };
        let idle = remote_connection(&conn, &uri, &soyuz_vfs::ConnState::Idle, true);
        let l = labels(&idle);
        assert_eq!(l.last().map(String::as_str), Some(tr!("remote-remove")));
        let r = runs(idle);
        assert!(matches!(&r[0], Some(Run::Cmd(Cmd::Remote(RemoteCmd::Open { new_tab: true, .. })))));
        assert!(r[2].is_none(), "nothing to disconnect");
        assert!(matches!(&r[3], Some(Run::Cmd(Cmd::CopyText(a))) if a.starts_with("sftp://")));
        let live = runs(remote_connection(&conn, &uri, &soyuz_vfs::ConnState::Connected, false));
        assert!(r.len() > live.len(), "a debug server cannot be removed");
        assert!(live[1].is_none(), "nor edited");
        assert!(matches!(&live[2], Some(Run::Cmd(Cmd::Remote(RemoteCmd::Disconnect(_))))));
    }
}
