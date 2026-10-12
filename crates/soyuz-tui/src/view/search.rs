//! The view of the search bar and its status row (Phase T5): drawn into `Regions::search`.
//!
//! Skeleton stub (Phase T5 + T6, step S0): W3-search-bar fills it (the model's fields, `prepare`,
//! the drawing).

use ratatui::Frame;
use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;

use crate::input::InputState;
use crate::layout::{Hits, Regions};
use crate::theme::TuiTheme;

/// What it shows (plain data, no floats: `ViewModel` is `Eq`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchVm {}

impl SearchVm {
    pub fn from_core(_core: &AppCore, _input: &InputState, _regions: &Regions) -> SearchVm {
        SearchVm::default()
    }
}

/// Rows the bar and its status row want at full width (`Facts::search_rows`; the layout
/// adds one below 80 columns); 0: hidden.
pub fn rows(_core: &AppCore) -> u16 {
    0
}

/// Step 1 of a frame (`view::prepare`): what the view needs from `&mut AppCore`.
pub fn prepare(_core: &mut AppCore, _input: &mut InputState, _regions: &Regions) {}

/// Draw `m` into `area`, recording click targets in `hits`.
pub fn draw_search(_f: &mut Frame<'_>, _area: Rect, _m: &SearchVm, _theme: &TuiTheme, _hits: &mut Hits) {}
