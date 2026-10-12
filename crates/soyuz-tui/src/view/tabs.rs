//! The tab strip (step T2.4). Not drawn yet: [`shown`] is false, so `Regions` gives it no
//! row and the location line stays on top as in Phase T1.

use ratatui::Frame;
use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;

use crate::dialog::safe;
use crate::layout::Hits;
use crate::theme::TuiTheme;

/// The tabs of the window.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabsVm {
    /// Title per tab (its focused pane's location).
    pub titles: Vec<String>,
    pub active: usize,
}

impl TabsVm {
    pub fn from_core(core: &AppCore) -> TabsVm {
        let titles = core
            .ws
            .tabs
            .iter()
            .map(|t| safe(&soyuz_appcore::state::crumbs::location_title(&t.active().location)))
            .collect();
        TabsVm { titles, active: core.ws.active }
    }
}

/// Whether the tab strip takes a row (`layout::Facts::tab_strip`).
pub fn shown(_core: &AppCore) -> bool {
    false
}

/// Draw the tab strip into `rect` (one row; empty when hidden).
pub fn draw_tabs(_f: &mut Frame<'_>, _rect: Rect, _m: &TabsVm, _theme: &TuiTheme, _hits: &mut Hits) {}
