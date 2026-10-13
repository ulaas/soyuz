//! The duplicate finder screen's view (Phase T7, agent C).
//!
//! Skeleton (Phase T7 wave 0): filled by agent C.

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::layout::Hits;
use crate::theme::TuiTheme;

/// What the duplicate finder screen draws.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DupesVm {}

/// Draw the screen over `area`.
pub fn draw(_f: &mut Frame<'_>, _area: Rect, _m: &DupesVm, _theme: &TuiTheme, _hits: &mut Hits) {}
