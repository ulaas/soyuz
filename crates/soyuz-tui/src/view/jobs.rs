//! The bottom panel: running and finished jobs (activity) and their history — Phase T3.
//!
//! Stub until T3 fills it: [`rows`] decides whether the panel takes rows
//! (`Regions::bottom`, height `[tui] jobs_rows`), [`prepare`] runs with the core mutable,
//! [`JobsVm::from_core`] builds the model and [`draw_jobs`] draws it, recording
//! `BottomTab`, `JobRow` and `JobButton` targets (`crate::input::jobs` handles them and
//! the keys while `TuiFocus::Jobs`).

use ratatui::Frame;
use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;

use crate::input::InputState;
use crate::layout::{Hits, Regions};
use crate::theme::TuiTheme;

/// The jobs panel as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JobsVm {
    /// The panel is drawn.
    pub visible: bool,
    /// It has the keyboard.
    pub focused: bool,
}

/// Whether the bottom panel is open (T3: the core's activity panel).
pub fn shown(_core: &AppCore) -> bool {
    false
}

/// Rows the bottom panel wants (`layout::Facts::bottom_rows`): `[tui] jobs_rows` while it
/// is open, else 0.
pub fn rows(core: &AppCore) -> u16 {
    if shown(core) { core.config.tui.jobs_rows_clamped() } else { 0 }
}

/// Step 1 of a frame (`view::prepare`).
pub fn prepare(_core: &mut AppCore, _input: &mut InputState, _regions: &Regions) {}

impl JobsVm {
    pub fn from_core(core: &AppCore, input: &InputState) -> JobsVm {
        JobsVm { visible: shown(core), focused: input.jobs.focused }
    }
}

/// Draw the panel into `area` (`Regions::bottom`).
pub fn draw_jobs(_f: &mut Frame<'_>, area: Rect, m: &JobsVm, _theme: &TuiTheme, _hits: &mut Hits) {
    if area.is_empty() || !m.visible {
        // T3: the panel.
    }
}
