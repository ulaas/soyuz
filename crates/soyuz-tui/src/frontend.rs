//! Carries out the core's requests for the frontend (`FrontendCmd`) in the terminal UI,
//! like the GUI's `ui/frontend.rs`.
//!
//! [`drain`] runs after every command (`App::apply_cmds`). Each request goes to the
//! module that owns it:
//!
//! | Request | Handler |
//! |---|---|
//! | `Confirm` | `input::dialogs::confirm::open` |
//! | `SelectByPattern` | `input::dialogs::pattern::open` |
//! | `ChooseColumns` | `input::dialogs::columns::open` |
//! | `OpenWith` | `input::dialogs::open_with::open` |
//! | `MetaDialog(BatchRename / Attributes / Link)` | `input::dialogs::{batch_rename, attributes, link}::open` |
//! | `Properties` / `FolderProperties` | `input::dialogs::properties::{open, open_folder}` |
//! | `RecoveredJobs` | `input::dialogs::recovery::open` |
//! | `CrashReports` | `input::dialogs::crash_reports::open` |
//! | `EditorUnsaved` | `input::dialogs::unsaved::open` |
//! | `View(ContextMenu)` | `input::menu::open_context` |
//! | `View(FocusActivity)` | `input::jobs::focus` |
//! | `View(PreviewScroll)` | `input::quicklook::scroll` while Quick Look is open, else `input::preview::scroll` |
//! | `OpenSettings` / `CloseSettings` | `input::settings::{open, close}` |
//! | `Smart` / `Remote` / `Archive` | `input::dialogs::{smart, remote, archive}::open` |
//! | `Suspend` | `App::request_suspend` (run between frames by `crate::suspend`) |
//! | `SaveWorkspace` / `ManageWorkspaces` | `input::dialogs::{save_workspace, manage_workspaces}::open` |
//! | `TabHistory` | `input::dialogs::tab_history::open` |
//! | `About` | `input::dialogs::about::open` |
//! | `UnlockDevice` | `input::dialogs::unlock_device::open` |
//! | `CloseWindowPrompt` | nothing (logged): it cannot happen in the terminal UI |
//! | `ReleaseFocus` | nothing to do |
//!
//! The hibernated-NTFS offer of a device mount is a `Confirm` (`models::devices::offer_read_only`).
//!
//! `CloseWindowPrompt` is pushed only while secondary GUI windows exist
//! (`models::windows::request_close` / `intercept_main_close`); the terminal UI never
//! opens one (the window operations are `AppCore::tui_unavailable`). Quitting goes through
//! `AppCore::handle_close_request`: unsaved editors ask (`EditorUnsaved`), running jobs
//! ask (the core's `QuitModel`), like the GUI.

use soyuz_appcore::models::frontend::{FrontendCmd, ViewRequest};
use soyuz_appcore::models::properties::MetaDialog;

use crate::app::App;
use crate::input::{dialogs, jobs, menu, preview, quicklook, settings};

/// Apply and clear every queued request.
pub fn drain(app: &mut App) {
    for c in std::mem::take(&mut app.core.frontend) {
        handle(app, c);
    }
}

/// One request.
pub fn handle(app: &mut App, c: FrontendCmd) {
    match c {
        FrontendCmd::ReleaseFocus => {}
        FrontendCmd::OpenSettings(section) => settings::open(app, section),
        FrontendCmd::CloseSettings => settings::close(app),
        FrontendCmd::Smart(p) => dialogs::smart::open(app, p),
        FrontendCmd::Remote(p) => dialogs::remote::open(app, p),
        FrontendCmd::Archive(p) => dialogs::archive::open(app, p),
        FrontendCmd::Suspend(req) => app.request_suspend(req),
        FrontendCmd::Confirm { title, message, confirm, on_confirm } => {
            dialogs::confirm::open(app, title, message, confirm, on_confirm)
        }
        FrontendCmd::SelectByPattern(pane) => dialogs::pattern::open(app, pane),
        FrontendCmd::ChooseColumns(pane) => dialogs::columns::open(app, pane),
        FrontendCmd::OpenWith(path) => dialogs::open_with::open(app, path),
        FrontendCmd::MetaDialog(MetaDialog::BatchRename) => dialogs::batch_rename::open(app),
        FrontendCmd::MetaDialog(MetaDialog::Attributes) => dialogs::attributes::open(app),
        FrontendCmd::MetaDialog(MetaDialog::Link(kind)) => dialogs::link::open(app, kind),
        FrontendCmd::Properties { pane, entries, tab, start } => {
            dialogs::properties::open(app, pane, entries, tab, start)
        }
        FrontendCmd::FolderProperties(dir) => dialogs::properties::open_folder(app, dir),
        FrontendCmd::RecoveredJobs(recovered) => dialogs::recovery::open(app, recovered),
        FrontendCmd::CrashReports(reports) => dialogs::crash_reports::open(app, reports),
        FrontendCmd::EditorUnsaved(p) => dialogs::unsaved::open(app, p),
        FrontendCmd::View(ViewRequest::ContextMenu) => menu::open_context(app),
        FrontendCmd::View(ViewRequest::FocusActivity) => jobs::focus(app),
        FrontendCmd::View(ViewRequest::PreviewScroll(step)) => {
            if app.core.ui.quicklook.open {
                quicklook::scroll(app, step)
            } else {
                preview::scroll(app, step)
            }
        }
        // ---- Phase T7 ----
        FrontendCmd::SaveWorkspace(name) => dialogs::save_workspace::open(app, name),
        FrontendCmd::ManageWorkspaces => dialogs::manage_workspaces::open(app),
        FrontendCmd::TabHistory { tab, pane } => dialogs::tab_history::open(app, tab, pane),
        FrontendCmd::About => dialogs::about::open(app),
        FrontendCmd::UnlockDevice { object, name, kind, pane } => {
            dialogs::unlock_device::open(app, object, name, kind, pane)
        }
        // Unreachable here: no secondary windows in the terminal UI (module docs).
        FrontendCmd::CloseWindowPrompt { id, tabs } => {
            log::debug!("tui: close-window prompt for window {id:?} ({tabs} tabs) ignored: no secondary windows")
        }
    }
}
