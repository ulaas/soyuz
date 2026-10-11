//! Carries out the core's [`FrontendCmd`] requests in the GUI.

use crate::app::SoyuzApp;
use crate::models::frontend::FrontendCmd;

/// Apply and clear every queued request.
pub fn drain(app: &mut SoyuzApp) {
    for c in std::mem::take(&mut app.frontend) {
        match c {
            FrontendCmd::OpenSettings(section) => crate::ui::settings::open(app, section),
            FrontendCmd::CloseSettings => crate::ui::settings::close(app),
        }
    }
}
