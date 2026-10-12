//! The sidebar (step T2.6). Not drawn yet: [`shown`] is false, so `Regions` gives it no
//! columns.

use ratatui::Frame;
use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;

use crate::layout::{Hits, Regions};
use crate::theme::TuiTheme;

/// The sidebar's rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SidebarVm {}

impl SidebarVm {
    pub fn from_core(_core: &AppCore) -> SidebarVm {
        SidebarVm {}
    }
}

/// Whether the sidebar takes columns (`layout::Facts::sidebar`; hidden anyway below
/// `layout::SIDEBAR_MIN_COLS`).
pub fn shown(_core: &AppCore) -> bool {
    false
}

/// Wanted sidebar width in columns (`layout::Facts::sidebar_width`).
pub fn width(_core: &AppCore) -> u16 {
    24
}

/// Before the model is built: data that needs `&mut AppCore`.
pub fn prepare(_core: &mut AppCore, _regions: &Regions) {}

/// Draw the sidebar into `rect`.
pub fn draw_sidebar(_f: &mut Frame<'_>, _rect: Rect, _m: &SidebarVm, _theme: &TuiTheme, _hits: &mut Hits) {}
