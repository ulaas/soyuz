//! Carries out the core's [`FrontendCmd`] requests in the GUI.

use crate::app::SoyuzApp;
use crate::models::frontend::FrontendCmd;

/// Apply and clear every queued request.
pub fn drain(app: &mut SoyuzApp) {
    for c in std::mem::take(&mut app.frontend) {
        match c {
            FrontendCmd::OpenSettings(section) => crate::ui::settings::open(app, section),
            FrontendCmd::CloseSettings => crate::ui::settings::close(app),
            FrontendCmd::SaveWorkspace(name) => {
                app.ui.dialogs.push(crate::ui::workspaces::dialogs::SaveWorkspaceDialog::new(name))
            }
            FrontendCmd::ManageWorkspaces => {
                app.ui.dialogs.push(crate::ui::workspaces::dialogs::ManageWorkspacesDialog::default())
            }
            FrontendCmd::TabHistory { tab, pane } => {
                app.ui.dialogs.push(crate::ui::workspaces::history::HistoryDialog::new(tab, pane))
            }
            FrontendCmd::CloseWindowPrompt { id, tabs } => crate::ui::workspaces::windows::close_prompt(app, id, tabs),
        }
    }
}
