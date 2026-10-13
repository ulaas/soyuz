//! Carries out the core's [`FrontendCmd`] requests in the GUI.

use crate::app::SoyuzApp;
use crate::host::EguiHost;
use crate::models::frontend::FrontendCmd;
use crate::models::properties::{LinkKind, MetaDialog};
use crate::ui::dialogs;

/// Apply and clear every queued request.
pub fn drain(app: &mut SoyuzApp, host: &EguiHost) {
    for c in std::mem::take(&mut app.core.frontend) {
        match c {
            FrontendCmd::OpenSettings(section) => crate::ui::settings::open(app, section),
            FrontendCmd::CloseSettings => crate::ui::settings::close(app),
            FrontendCmd::SaveWorkspace(name) => {
                app.gui.dialogs.push(crate::ui::workspaces::dialogs::SaveWorkspaceDialog::new(name))
            }
            FrontendCmd::ManageWorkspaces => {
                app.gui.dialogs.push(crate::ui::workspaces::dialogs::ManageWorkspacesDialog::default())
            }
            FrontendCmd::TabHistory { tab, pane } => {
                app.gui.dialogs.push(crate::ui::workspaces::history::HistoryDialog::new(tab, pane))
            }
            FrontendCmd::CloseWindowPrompt { id, tabs } => crate::ui::workspaces::windows::close_prompt(app, id, tabs),
            FrontendCmd::FolderProperties(dir) => crate::ui::tree::properties(app, &dir),
            FrontendCmd::UnlockDevice { object, name, kind, pane } => {
                crate::ui::sidebar::unlock_dialog(app, object, name, kind, pane)
            }
            FrontendCmd::Smart(p) => crate::ui::smart::prompt(app, p),
            FrontendCmd::Remote(p) => crate::ui::remote::prompt(app, p),
            FrontendCmd::Archive(p) => crate::ui::archive::prompt(app, p),
            FrontendCmd::ReleaseFocus => crate::ui::panels::terminal::release_focus(host.ctx()),
            FrontendCmd::About => app.gui.dialogs.push(dialogs::AboutDialog::new(app.core.renderer)),
            FrontendCmd::Confirm { title, message, confirm, on_confirm } => {
                app.gui.dialogs.push(dialogs::ConfirmDialog::new(&title, &message, &confirm, on_confirm))
            }
            FrontendCmd::SelectByPattern(pane) => app.gui.dialogs.push(dialogs::PatternDialog::new(pane)),
            FrontendCmd::ChooseColumns(pane) => app.gui.dialogs.push(dialogs::ColumnsDialog::new(pane)),
            FrontendCmd::OpenWith(path) => app.gui.dialogs.push(dialogs::open_with::OpenWithDialog::new(path)),
            FrontendCmd::Properties { pane, entries, tab, start } => {
                let d = dialogs::properties::PropertiesDialog::new(
                    pane,
                    entries,
                    app.core.config.general.size_style,
                    tab,
                    start,
                )
                .naming(app);
                app.gui.dialogs.push(d);
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
            FrontendCmd::CrashReports(reports) => {
                app.gui.dialogs.push(dialogs::recovery_crash::CrashDialog::new(reports))
            }
            FrontendCmd::RecoveredJobs(recovered) => {
                app.gui.dialogs.push(dialogs::recovery::RecoveryDialog::new(host.ctx(), recovered))
            }
            FrontendCmd::EditorUnsaved(p) => app.gui.dialogs.push(crate::ui::editors::UnsavedDialog::new(p)),
            // Terminal-UI-only actions (`actions::tui_only`): nothing to do in the GUI.
            FrontendCmd::View(_) => {}
            // Leaving the screen for a shell / editor is a terminal UI request.
            FrontendCmd::Suspend(_) => {}
        }
    }
}
