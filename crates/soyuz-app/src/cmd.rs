//! Deferred UI commands. Widgets never mutate app-wide state directly while drawing;
//! they push a [`Cmd`] which `SoyuzApp` applies after the frame's UI pass (or at the
//! start of the next one). This keeps borrows simple and makes every interaction
//! funnel through the same code paths as key bindings and the (future) command palette.

use std::path::PathBuf;

use soyuz_core::Location;
use soyuz_core::config::Action;
use soyuz_fs::AppInfo;

use crate::models::toasts::Toast;
use crate::state::pane::PaneId;
use crate::state::selection::EntryKey;

#[derive(Clone, Debug)]
pub enum Cmd {
    /// Run an action against the focused pane.
    Action(Action),
    /// Focus `pane` first, then run the action.
    PaneAction(PaneId, Action),
    Navigate {
        pane: PaneId,
        location: Location,
        select: Option<String>,
    },
    /// Activate an entry: enter folders, launch files.
    Open {
        pane: PaneId,
        key: EntryKey,
    },
    OpenInNewTab {
        location: Location,
        activate: bool,
    },
    OpenInOtherPane(Location),
    OpenWith {
        path: PathBuf,
        app: AppInfo,
    },
    /// Back/forward `n` steps (history dropdown).
    HistoryJump {
        pane: PaneId,
        back: bool,
        n: usize,
    },
    FocusPane(PaneId),
    CopyText(String),
    Toast(Toast),
    /// The pane's view state changed (remember it for the folder).
    ViewChanged(PaneId),
    /// Placeholder for operations that arrive with the jobs integration.
    ComingSoon(Action),
    RevealInSystem(PathBuf),
    TerminalAt(PathBuf),
    AddFavorite(PathBuf),
    RemoveFavorite(PathBuf),
    MoveFavorite {
        from: usize,
        to: usize,
    },
    /// Sidebar Portal: pin / unpin / reorder / relabel folders (`[[sidebar.portal]]`).
    Portal(crate::models::portal::PortalCmd),
    /// Sidebar icon bar: show the places sections or the disk tree (`[sidebar] mode`).
    SetSidebarMode(soyuz_core::config::SidebarMode),
    SelectTab(usize),
    CloseTab(usize),
    CloseOtherTabs(usize),
    CloseTabsToRight(usize),
    DuplicateTab(usize),
    PinTab(usize, bool),
    MoveTab {
        from: usize,
        to: usize,
    },
    NewTabAfter(usize),
    SetTheme(crate::host::ThemePref),
    SetSplit(f32),
    /// Make the pane's view the default for every folder (after confirmation).
    ApplyViewToAll(PaneId),
    /// Reveal the given path's parent and select it (path typed into the address bar).
    RevealPath(PathBuf),
    /// Open a new integrated terminal session in this folder.
    OpenTerminalPanel(PathBuf),
    /// Submit a background job (extract / compress, …) to the jobs engine.
    SubmitJob(Box<soyuz_jobs::JobSpec>),
    /// Extract selected archive entries (`(inner path, is_dir)`) into `dest`.
    ExtractEntries {
        archive: PathBuf,
        items: Vec<(String, bool)>,
        strip: usize,
        dest: PathBuf,
    },
    /// Run / remove a saved search (index into the sidebar list).
    RunSavedSearch(usize),
    RemoveSavedSearch(usize),
    /// File operations (clipboard, drag and drop, rename, prompts, trash): `ops::fileops`.
    FileOp(crate::ops::fileops::FileOpCmd),
    /// Metadata dialogs (properties, attributes, batch rename, links): `ui::dialogs::properties`.
    Meta(crate::ui::dialogs::properties::MetaCmd),
    /// Folder tree effects (focus, new folder, properties): `models::tree`.
    Tree(crate::models::tree::TreeCmd),
    /// Close confirmed while jobs are pending: now, or once they have finished.
    Quit {
        when_idle: bool,
    },
    /// Open the "Open with" chooser for a file.
    ChooseOpenWith(PathBuf),
    /// Open a path in a new Soyuz window (in-process or a new process, see
    /// `[workspace] new_window` and `ops::window`).
    NewWindow(Location),
    /// Tab groups, saved workspaces, layouts, windows, history graph: `ui::workspaces`.
    Workspace(crate::ui::workspaces::WsCmd),
    /// The in-app assistant (folder access, prefilled prompts): `ai::state`.
    Ai(crate::ai::AiCmd),
    /// Smart folders and the background index: `ui::smart`.
    Smart(crate::ui::smart::SmartCmd),
    /// Scripts, automations and the CLI companion: `automation`.
    Automation(crate::automation::AutoCmd),
    /// Remote locations (connect dialog, saved connections, host keys): `ui::remote`.
    Remote(crate::ui::remote::RemoteCmd),
    /// Sidebar devices (unlock dialog, read-only mount offer, hide): `models::devices`.
    Device(crate::models::devices::DeviceCmd),
}
