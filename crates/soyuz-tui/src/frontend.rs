//! Carries out the core's requests for the frontend (`FrontendCmd`) in the terminal UI,
//! like the GUI's `ui/frontend.rs`.
//!
//! [`drain`] runs after every command (`App::apply_cmds`). Each request goes to the
//! module that owns it, so the phases fill their own files, not this one:
//!
//! | Request | Handler | Phase |
//! |---|---|---|
//! | `Confirm` | `input::dialogs::confirm::open` | T3 |
//! | `SelectByPattern` | `input::dialogs::pattern::open` | T3 |
//! | `ChooseColumns` | `input::dialogs::columns::open` | T3 |
//! | `OpenWith` | `input::dialogs::open_with::open` | T3 |
//! | `MetaDialog(BatchRename / Attributes / Link)` | `input::dialogs::{batch_rename, attributes, link}::open` | T3 |
//! | `Properties` / `FolderProperties` | `input::dialogs::properties::{open, open_folder}` | T3 |
//! | `RecoveredJobs` | `input::dialogs::recovery::open` | T3 |
//! | `CrashReports` | `input::dialogs::crash_reports::open` | T3 |
//! | `EditorUnsaved` | `input::dialogs::unsaved::open` | T4 |
//! | `View(ContextMenu)` | `input::menu::open_context` | T3 |
//! | `View(FocusActivity)` | `input::jobs::focus` | T3 |
//! | `View(PreviewScroll)` | `input::preview::scroll` | T4 |
//! | `ReleaseFocus`, `CloseSettings` | nothing to do | |
//! | the rest | "not available in the terminal UI yet" ([`unavailable`]) | T5 / T6 |
//!
//! Every handler starts as a stub with the Phase T1 / T2 behaviour (mostly that toast).

use soyuz_appcore::models::frontend::{FrontendCmd, ViewRequest};
use soyuz_appcore::models::properties::MetaDialog;
use soyuz_appcore::tr;

use crate::app::App;
use crate::input::{dialogs, jobs, menu, preview};

/// Apply and clear every queued request.
pub fn drain(app: &mut App) {
    for c in std::mem::take(&mut app.core.frontend) {
        handle(app, c);
    }
}

/// One request.
pub fn handle(app: &mut App, c: FrontendCmd) {
    match c {
        FrontendCmd::ReleaseFocus | FrontendCmd::CloseSettings => {}
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
        FrontendCmd::View(ViewRequest::PreviewScroll(step)) => preview::scroll(app, step),
        other => unavailable(app, other),
    }
}

/// Requests of later phases (T5: devices, smart folders, remotes, archives; T6: settings,
/// workspaces, …): say the feature is not available yet.
///
/// While the T3 / T4 parts replace their stubs, `tui-feature-*` keys may stay in `tui.ftl`
/// without a user; the marker below keeps the unused-key test quiet until the phase that
/// removes the last stub deletes the leftovers (and this marker).
// i18n-dynamic: tui-feature-
fn unavailable(app: &mut App, c: FrontendCmd) {
    let feature = match c {
        FrontendCmd::OpenSettings(_) => tr!("tui-feature-settings"),
        FrontendCmd::SaveWorkspace(_) | FrontendCmd::ManageWorkspaces => tr!("tui-feature-workspaces"),
        FrontendCmd::TabHistory { .. } => tr!("tui-feature-history"),
        FrontendCmd::CloseWindowPrompt { .. } => tr!("tui-feature-windows"),
        FrontendCmd::UnlockDevice { .. } | FrontendCmd::OfferReadOnlyMount { .. } => tr!("tui-feature-devices"),
        FrontendCmd::Smart(_) => tr!("tui-feature-smart"),
        FrontendCmd::Remote(_) => tr!("tui-feature-remote"),
        FrontendCmd::Archive(_) => tr!("tui-feature-archive"),
        FrontendCmd::About => tr!("tui-feature-about"),
        // Handled in `handle`.
        _ => return,
    };
    app.unavailable(feature);
}
