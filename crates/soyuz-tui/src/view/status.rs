//! The status line: item counts on the left, a key hint on the right (step T2.8 adds the
//! filter field and more).

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::widgets::Paragraph;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::tr;

use super::text;
use crate::layout::{HitTarget, Hits};
use crate::theme::{Role, TuiTheme};

/// The status line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusVm {
    /// "12 items · 2 selected" (the focused pane).
    pub counts: String,
    /// Key hint on the right.
    pub hint: String,
}

impl StatusVm {
    pub fn from_core(core: &AppCore) -> StatusVm {
        let pane = core.ws.pane();
        let mut counts = tr!("tui-items", count = pane.visible.len());
        let selected = pane.selection.len();
        if selected > 0 {
            counts.push_str(" · ");
            counts.push_str(&tr!("tui-selected", count = selected));
        }
        StatusVm { counts, hint: tr!("tui-hint").to_string() }
    }
}

/// Whether the status line takes a row (`layout::Facts::status`).
pub fn shown(_core: &AppCore) -> bool {
    true
}

/// Draw the status line into `rect`.
pub fn draw_status(f: &mut Frame<'_>, rect: Rect, m: &StatusVm, theme: &TuiTheme, hits: &mut Hits) {
    if rect.is_empty() {
        return;
    }
    let bar = theme.style(Role::StatusBar);
    let w = usize::from(rect.width);
    let counts = text::truncate_end(&format!(" {}", m.counts), w);
    f.render_widget(Paragraph::new(counts.as_str()).style(bar), rect);
    // The hint only where it fits next to the counts.
    let hint = format!("{} ", m.hint);
    if text::width(&hint) < w.saturating_sub(text::width(&counts)) {
        f.render_widget(Paragraph::new(hint).alignment(Alignment::Right).style(bar), rect);
    }
    hits.push(rect, HitTarget::Status);
}
