//! Requests from app logic to whichever frontend is running (Phase T0): things only a
//! frontend can do, such as opening its settings window. Core code pushes onto
//! `AppCore::frontend`; the frontend drains the queue every frame.

use std::path::PathBuf;

use soyuz_core::EntryInfo;

use super::settings::Section;
use crate::cmd::Cmd;
use crate::state::pane::PaneId;

#[derive(Clone, Debug)]
pub enum FrontendCmd {
    /// Open the settings, optionally on a page.
    OpenSettings(Option<Section>),
    /// Close the settings (writes a pending `config.toml` save).
    CloseSettings,
    /// "Save workspace as…", with a suggested name.
    SaveWorkspace(String),
    /// "Manage workspaces".
    ManageWorkspaces,
    /// The history graph of `tab`, starting at `pane`.
    TabHistory { tab: crate::state::workspace::TabId, pane: crate::state::pane::PaneId },
    /// Window `id` (with `tabs` tabs) is closing: ask whether to move or close its tabs.
    CloseWindowPrompt { id: crate::state::workspace_windows::WindowId, tabs: usize },
    /// Show the properties of a folder (from the folder tree).
    FolderProperties(std::path::PathBuf),
    /// Ask for the passphrase / recovery key of a locked device container, then mount it
    /// into `pane`.
    UnlockDevice { object: String, name: String, kind: String, pane: crate::state::pane::PaneId },
    /// A smart-folder dialog (`models::smart`).
    Smart(super::smart::SmartPrompt),
    /// A remote-location dialog (`models::remote`).
    Remote(super::remote::RemotePrompt),
    /// An archive dialog (`models::archive`).
    Archive(super::archive::ArchivePrompt),
    /// No widget keeps the keyboard (the terminal gave it back to the file list).
    ReleaseFocus,
    /// The About window.
    About,
    /// Ask before running `on_confirm`.
    Confirm { title: String, message: String, confirm: String, on_confirm: Cmd },
    /// Select by pattern in a pane.
    SelectByPattern(PaneId),
    /// Choose the columns of a pane's details view.
    ChooseColumns(PaneId),
    /// Pick the app a file opens with.
    OpenWith(PathBuf),
    /// The Properties dialog for `entries` of `pane`.
    Properties {
        pane: PaneId,
        entries: Vec<EntryInfo>,
        tab: super::properties::Tab,
        start: Option<super::properties::Start>,
    },
    /// Batch rename / bulk attributes / new link, on the focused pane's selection.
    MetaDialog(super::properties::MetaDialog),
    /// Crash reports from the last run.
    CrashReports(Vec<crate::services::crash::CrashReport>),
    /// Interrupted jobs found in the journal (resume / discard).
    RecoveredJobs(Vec<soyuz_jobs::journal::Recovered>),
    /// "Save changes?" for an editor document or before quitting.
    EditorUnsaved(crate::editors::UnsavedPrompt),
    /// Actions only the terminal UI has a view for so far (see
    /// [`crate::actions::tui_only`]); the GUI ignores them.
    View(ViewRequest),
    /// Terminal UI only: leave the screen to run a shell, a command or a text editor in
    /// the real terminal, then come back (`[tui] shell_mode = "suspend"`, "Edit config
    /// file"). The GUI ignores it.
    Suspend(SuspendRequest),
}

/// [`FrontendCmd::Suspend`]: what runs while the terminal UI steps aside.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuspendRequest {
    /// An interactive shell in `dir`; `exit` returns.
    Shell { dir: PathBuf },
    /// One shell command line in `dir`, then "press Enter to return".
    Command { dir: PathBuf, line: String },
    /// The text editor (`[tui] editor`, `$VISUAL`, `$EDITOR`, …) on `path`.
    Editor { path: PathBuf, purpose: EditPurpose },
}

/// Why [`SuspendRequest::Editor`] edits a file: what happens after the editor exits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditPurpose {
    /// `config.toml`: reloaded afterwards.
    Config,
    /// A scratch file of a frontend feature (e.g. the AI panel's JSON arguments), handed
    /// back to its owner by this token.
    Scratch(u64),
}

/// [`FrontendCmd::View`]: what a terminal-UI-only action asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewRequest {
    /// `ContextMenu` (Shift+F10): the menu of the focused item (both frontends).
    ContextMenu,
    /// `PreviewScroll*` / `PreviewPage*`: scroll the preview pane.
    PreviewScroll(ScrollStep),
    /// `FocusActivityPanel`: move the keyboard to the jobs panel (opening it).
    FocusActivity,
}

/// How far [`ViewRequest::PreviewScroll`] scrolls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollStep {
    LineUp,
    LineDown,
    PageUp,
    PageDown,
}
