//! Deferred UI commands. Widgets never mutate app-wide state directly while drawing;
//! they push a [`Cmd`] which `SoyuzApp` applies after the frame's UI pass (or at the
//! start of the next one). This keeps borrows simple and makes every interaction
//! funnel through the same code paths as key bindings and the (future) command palette.

use std::path::PathBuf;

use egui::ThemePreference;
use soyuz_core::Location;
use soyuz_core::config::Action;
use soyuz_fs::AppInfo;

use crate::state::pane::PaneId;
use crate::state::selection::EntryKey;
use crate::ui::toasts::Toast;

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
    SetTheme(ThemePreference),
    SetSplit(f32),
    /// Make the pane's view the default for every folder (after confirmation).
    ApplyViewToAll(PaneId),
    /// Reveal the given path's parent and select it (path typed into the address bar).
    RevealPath(PathBuf),
    /// File operations (clipboard, drag and drop, rename, prompts, trash): `ops::fileops`.
    FileOp(crate::ops::fileops::FileOpCmd),
}
