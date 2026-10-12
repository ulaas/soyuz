//! The view of the AI assistant panel and its MCP approval cards (Phase T5): drawn into
//! `Regions::ai`.
//!
//! Skeleton stub (Phase T5 + T6, step S0): W2-ai-panel (composer and JSON edits: W3) fills it (the
//! model's fields, `prepare`, the drawing).

use ratatui::Frame;
use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;

use crate::input::InputState;
use crate::layout::{Hits, Regions};
use crate::theme::TuiTheme;

/// What it shows (plain data, no floats: `ViewModel` is `Eq`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AiVm {}

impl AiVm {
    pub fn from_core(_core: &AppCore, _input: &InputState, _regions: &Regions) -> AiVm {
        AiVm::default()
    }
}

/// It is drawn this frame.
pub fn shown(_core: &AppCore) -> bool {
    false
}

/// Step 1 of a frame (`view::prepare`): what the view needs from `&mut AppCore`.
pub fn prepare(core: &mut AppCore, _input: &mut InputState, _regions: &Regions) {
    // No view yet: an opened panel (Ctrl+I, an MCP request in `panel` mode) closes again
    // with a "not available" toast.
    if core.ai.panel.open {
        core.ai.panel.open = false;
        core.tui_unavailable();
    }
}

/// Draw `m` into `area`, recording click targets in `hits`.
pub fn draw_ai(_f: &mut Frame<'_>, _area: Rect, _m: &AiVm, _theme: &TuiTheme, _hits: &mut Hits) {}
