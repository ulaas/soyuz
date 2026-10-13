//! The full-screen tools' view (Phase T7, agent C; see `crate::input::screens`): one
//! [`ScreensVm`] for the shown screen, drawn by [`draw_screens`] over the whole area like
//! the compare screen (an `Overlay` target first, so no click reaches the panes).
//!
//! Skeleton (Phase T7 wave 0): filled by agent C.

pub mod dupes;
pub mod treemap;

use ratatui::Frame;
use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;

use crate::input::InputState;
use crate::layout::{Hits, Regions};
use crate::theme::TuiTheme;

/// What the shown tool screen draws.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreensVm {
    /// A screen shows (open and not hidden).
    pub visible: bool,
    pub screen: Option<ScreenVm>,
}

/// The shown screen's model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScreenVm {
    Treemap(treemap::TreemapVm),
    Dupes(dupes::DupesVm),
}

impl ScreensVm {
    pub fn from_core(_core: &AppCore, input: &InputState, _regions: &Regions) -> ScreensVm {
        ScreensVm { visible: crate::input::screens::shown(&input.screens), screen: None }
    }
}

/// Step 1 (`view::prepare`): what the shown screen needs from `&mut AppCore` (worker
/// results, the cell layout for the area).
pub fn prepare(_core: &mut AppCore, _input: &mut InputState, _regions: &Regions) {}

/// Draw the shown screen over `area`, recording its click targets.
pub fn draw_screens(f: &mut Frame<'_>, area: Rect, m: &ScreensVm, theme: &TuiTheme, hits: &mut Hits) {
    let area = area.intersection(f.area());
    if area.is_empty() || !m.visible {
        return;
    }
    match &m.screen {
        Some(ScreenVm::Treemap(t)) => treemap::draw(f, area, t, theme, hits),
        Some(ScreenVm::Dupes(d)) => dupes::draw(f, area, d, theme, hits),
        None => {}
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Models for `view::tests::draw_every_view_at_tiny_sizes` (agent C adds filled ones).
    pub(crate) fn tiny_vms() -> Vec<ScreensVm> {
        vec![
            ScreensVm { visible: true, screen: Some(ScreenVm::Treemap(treemap::TreemapVm::default())) },
            ScreensVm { visible: true, screen: Some(ScreenVm::Dupes(dupes::DupesVm::default())) },
        ]
    }
}
