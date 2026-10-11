//! Carries out the core's [`FrontendCmd`] requests in the GUI.

use crate::app::SoyuzApp;
use crate::models::frontend::FrontendCmd;

/// Apply and clear every queued request.
pub fn drain(app: &mut SoyuzApp) {
    for c in std::mem::take(&mut app.frontend) {
        match c {
            FrontendCmd::OpenSettings(section) => crate::ui::settings::open(app, section),
            FrontendCmd::CloseSettings => crate::ui::settings::close(app),
            FrontendCmd::Properties(dir) => crate::ui::tree::properties(app, &dir),
            FrontendCmd::UnlockDevice { object, name, kind, pane } => {
                crate::ui::sidebar::unlock_dialog(app, object, name, kind, pane)
            }
            FrontendCmd::OfferReadOnlyMount { dev, name } => crate::ui::sidebar::offer_read_only(app, dev, name),
        }
    }
}
