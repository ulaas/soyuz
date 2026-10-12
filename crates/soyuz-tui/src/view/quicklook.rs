//! Quick Look: the large preview over the files (Space) — Phase T4.
//!
//! Stub until T4 fills it (and removes the Quick Look reset in `crate::unsupported`):
//! [`QuickLookVm::from_core`] builds the model and [`draw_quicklook`] draws it over the
//! whole screen, recording a `QuickLook` target (`crate::input::quicklook` handles the
//! keys while `TuiFocus::QuickLook`).

use ratatui::Frame;
use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;

use crate::input::InputState;
use crate::layout::{Hits, Regions};
use crate::theme::TuiTheme;

/// The Quick Look overlay as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QuickLookVm {
    /// The overlay is drawn.
    pub visible: bool,
}

/// Step 1 of a frame (`view::prepare`).
pub fn prepare(_core: &mut AppCore, _input: &mut InputState, _regions: &Regions) {}

impl QuickLookVm {
    pub fn from_core(core: &AppCore, input: &InputState) -> QuickLookVm {
        QuickLookVm { visible: crate::input::quicklook::open(core, input) }
    }
}

/// Draw the overlay over `area` (the whole screen).
pub fn draw_quicklook(_f: &mut Frame<'_>, area: Rect, m: &QuickLookVm, _theme: &TuiTheme, _hits: &mut Hits) {
    if area.is_empty() || !m.visible {
        // T4: the overlay.
    }
}
