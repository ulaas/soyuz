//! The core's tool queue (`soyuz_appcore::models::tools::{queue, take_queue}`) in the
//! terminal UI, drained every frame like the GUI's `ui/tools/mod.rs`: the model commands
//! (tags, the shelf, Recent, …) apply in the core; Compare & sync opens the compare
//! screen (`crate::input::compare`); the other tool windows have no terminal view.

use soyuz_appcore::models::tools::{self, ToolCmd};
use soyuz_appcore::tr;

use crate::app::App;

/// Apply and clear the queued tool commands (`App::frame`, after the background pumps
/// and after the input).
pub fn drain(app: &mut App) {
    let queued = tools::take_queue();
    if queued.is_empty() {
        return;
    }
    let rest = tools::apply_model_cmds(&mut app.core, &app.host, queued);
    let mut windows = false;
    for c in rest {
        match c {
            ToolCmd::Compare(left, right) => crate::input::compare::open(app, left, right),
            ToolCmd::CompareProfile(name) => crate::input::compare::open_profile(app, name),
            ToolCmd::SyncProfiles => crate::input::compare::open_profiles(app),
            _ => windows = true,
        }
    }
    if windows {
        app.unavailable(tr!("tui-menu-tool-windows"));
    }
}
