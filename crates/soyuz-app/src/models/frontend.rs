//! Requests from app logic to whichever frontend is running (Phase T0): things only a
//! frontend can do, such as opening its settings window. Core code pushes onto
//! `SoyuzApp::frontend`; the frontend drains the queue every frame.

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
    /// Copy (or move, `mv`) `paths` to a folder picked starting at `start`.
    Transfer { paths: Vec<PathBuf>, start: PathBuf, mv: bool },
    /// Quitting would cancel `pending` jobs: ask first.
    ConfirmQuit { pending: usize },
    /// Crash reports from the last run.
    CrashReports(Vec<crate::services::crash::CrashReport>),
    /// Interrupted jobs found in the journal (resume / discard).
    RecoveredJobs(Vec<soyuz_jobs::journal::Recovered>),
    /// Approve the MCP client request `id`.
    McpApproval(u64),
}
