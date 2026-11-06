//! The system monitor (`monitor:` tabs): state and commands behind both frontends' pages.

use soyuz_core::Location;

use crate::core::AppCore;
use crate::ops;

/// Show the monitor: the tab that already has it, else a new tab.
pub fn open_tab(app: &mut AppCore) {
    let open = app.ws.tabs.iter().position(|t| t.panes.iter().any(|p| matches!(p.location, Location::Monitor)));
    match open {
        Some(i) => app.ws.active = i,
        None => ops::open_in_new_tab(app, Location::Monitor, true),
    }
}
