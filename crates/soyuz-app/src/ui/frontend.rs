//! Carries out the core's [`FrontendCmd`] requests in the GUI.

use crate::app::SoyuzApp;
use crate::host::Host;
use crate::models::frontend::FrontendCmd;
use crate::models::properties::{LinkKind, MetaDialog};
use crate::ui::dialogs;

/// Apply and clear every queued request.
pub fn drain(app: &mut SoyuzApp, host: &dyn Host) {
    for c in std::mem::take(&mut app.frontend) {
        match c {
            FrontendCmd::OpenSettings(section) => crate::ui::settings::open(app, section),
            FrontendCmd::CloseSettings => crate::ui::settings::close(app),
            FrontendCmd::ReleaseFocus => crate::ui::panels::terminal::release_focus(crate::host::egui_ctx(host)),
            FrontendCmd::About => app.ui.dialogs.push(dialogs::AboutDialog::new(app.renderer)),
            FrontendCmd::Confirm { title, message, confirm, on_confirm } => {
                app.ui.dialogs.push(dialogs::ConfirmDialog::new(&title, &message, &confirm, on_confirm))
            }
            FrontendCmd::SelectByPattern(pane) => app.ui.dialogs.push(dialogs::PatternDialog::new(pane)),
            FrontendCmd::ChooseColumns(pane) => app.ui.dialogs.push(dialogs::ColumnsDialog::new(pane)),
            FrontendCmd::OpenWith(path) => app.ui.dialogs.push(dialogs::open_with::OpenWithDialog::new(path)),
            FrontendCmd::Properties { pane, entries, tab, start } => {
                let d = dialogs::properties::PropertiesDialog::new(
                    pane,
                    entries,
                    app.config.general.size_style,
                    tab,
                    start,
                )
                .naming(app);
                app.ui.dialogs.push(d);
            }
            FrontendCmd::MetaDialog(d) => match d {
                MetaDialog::BatchRename => dialogs::batch_rename::open(app),
                MetaDialog::Attributes => dialogs::attributes::open(app),
                MetaDialog::Link(kind) => {
                    let choice = match kind {
                        LinkKind::Symlink => dialogs::new_item::LinkChoice::Symlink,
                        LinkKind::Hardlink => dialogs::new_item::LinkChoice::Hardlink,
                        LinkKind::Junction => dialogs::new_item::LinkChoice::Junction,
                    };
                    dialogs::links::open(app, choice)
                }
            },
            FrontendCmd::Transfer { paths, start, mv } => {
                app.ui.dialogs.push(dialogs::new_item::TransferDialog::new(paths, start, mv))
            }
            FrontendCmd::ConfirmQuit { pending } => {
                app.ui.dialogs.push(dialogs::quit::QuitDialog::new(pending, app.quit.prompting.clone()))
            }
            FrontendCmd::CrashReports(reports) => {
                app.ui.dialogs.push(dialogs::recovery_crash::CrashDialog::new(reports))
            }
            FrontendCmd::RecoveredJobs(recovered) => {
                app.ui.dialogs.push(dialogs::recovery::RecoveryDialog::new(crate::host::egui_ctx(host), recovered))
            }
            FrontendCmd::McpApproval(id) => {
                let approvals = app.services.mcp.approvals.clone();
                let flag = app.services.mcp.modal_open.clone();
                app.ui.dialogs.push(dialogs::mcp_approval::McpApprovalDialog::new(id, approvals, flag))
            }
        }
    }
}
