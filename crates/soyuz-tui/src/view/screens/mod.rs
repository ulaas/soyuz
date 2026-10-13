//! The full-screen tools' view (Phase T7, agent C; see `crate::input::screens`): one
//! [`ScreensVm`] for the shown screen, drawn by [`draw_screens`] over the whole area like
//! the compare screen (an `Overlay` target first, so no click reaches the panes).
//!
//! [`prepare`] pumps the shown screen's model and builds its view model into the input
//! state (`treemap::prepare`, `dupes::prepare`); [`ScreensVm::from_core`] copies it into
//! the frame's `ViewModel`. [`chrome`] holds what both screens draw alike.

pub mod chrome;
pub mod dupes;
pub mod treemap;

use ratatui::Frame;
use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;

use crate::input::InputState;
use crate::input::screens::Screen;
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
    Treemap(Box<treemap::TreemapVm>),
    Dupes(Box<dupes::DupesVm>),
}

impl ScreensVm {
    pub fn from_core(_core: &AppCore, input: &InputState, _regions: &Regions) -> ScreensVm {
        if !crate::input::screens::shown(&input.screens) {
            return ScreensVm::default();
        }
        let screen = match &input.screens.screen {
            Some(Screen::Treemap(t)) => Some(ScreenVm::Treemap(Box::new(t.vm.clone()))),
            Some(Screen::Dupes(d)) => Some(ScreenVm::Dupes(Box::new(d.vm.clone()))),
            None => None,
        };
        ScreensVm { visible: true, screen }
    }
}

/// Step 1 (`view::prepare`): what the shown screen needs from `&mut AppCore` (worker
/// results, the cell layout for the area). A hidden screen is not pumped: its results
/// wait in the channels until it shows again.
pub fn prepare(core: &mut AppCore, input: &mut InputState, regions: &Regions) {
    if !crate::input::screens::shown(&input.screens) {
        return;
    }
    match input.screens.screen.as_mut() {
        Some(Screen::Treemap(t)) => treemap::prepare(core, t, regions.area),
        Some(Screen::Dupes(d)) => dupes::prepare(core, d, regions.area),
        None => {}
    }
}

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

    /// Models for `view::tests::draw_every_view_at_tiny_sizes`.
    pub(crate) fn tiny_vms() -> Vec<ScreensVm> {
        vec![
            ScreensVm { visible: true, screen: Some(ScreenVm::Treemap(Box::new(treemap::tests::vm()))) },
            ScreensVm { visible: true, screen: Some(ScreenVm::Dupes(Box::new(dupes::tests::vm()))) },
        ]
    }
}
