//! The command palette overlay (step T2.7). Not drawn yet: the palette is closed by
//! `crate::unsupported::palette` right after the input that opened it.

use ratatui::Frame;
use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;

use crate::layout::{Hits, Regions};
use crate::theme::TuiTheme;

/// The palette's input and results.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaletteVm {
    pub open: bool,
}

impl PaletteVm {
    pub fn from_core(core: &AppCore) -> PaletteVm {
        PaletteVm { open: core.palette.open }
    }
}

/// Before the model is built: data that needs `&mut AppCore`.
pub fn prepare(_core: &mut AppCore, _regions: &Regions) {}

/// Draw the palette over `area` (the whole screen) when it is open.
pub fn draw_palette(_f: &mut Frame<'_>, _area: Rect, _m: &PaletteVm, _theme: &TuiTheme, _hits: &mut Hits) {}
