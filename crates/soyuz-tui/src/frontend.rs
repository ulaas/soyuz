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
//! | `ReleaseFocus`, `CloseSettings` | nothing to do |
//! | the rest | a "not available in the terminal UI yet" toast ([`unavailable`]) |
//!
//! "The rest" is settings, workspaces (save / manage), tab history, the close-window
//! prompt, devices (unlock, read-only mount), smart folders, remote locations, the
//! archive forms (extract, create, password, rename inside an archive) and About;
//! `crate::unsupported` does the same for the core states the terminal UI has no view
//! for (the settings flag, the terminal panel's focus).

use soyuz_appcore::models::frontend::{FrontendCmd, ViewRequest};
use soyuz_appcore::models::properties::MetaDialog;
use soyuz_appcore::tr;

use crate::app::App;
use crate::input::{dialogs, jobs, menu, preview, quicklook};

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
        FrontendCmd::View(ViewRequest::PreviewScroll(step)) => {
            if app.core.ui.quicklook.open {
                quicklook::scroll(app, step)
            } else {
                preview::scroll(app, step)
            }
        }
        other => unavailable(app, other),
    }
}

/// Requests the terminal UI has no view for yet: say the feature is not available.
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
