//! Terminal rendering. M0: a placeholder panel; M8 paints the alacritty grid
//! (runs of identical style coalesced into one `LayoutJob` per row).

use egui::{Align2, FontId, Sense, Ui};

use crate::{TermResponse, TermTheme, Terminal};

pub(crate) fn placeholder(
    ui: &mut Ui,
    _term: &Terminal,
    focused: bool,
    theme: Option<&TermTheme>,
    font: Option<FontId>,
) -> TermResponse {
    let fallback = TermTheme::dark();
    let theme = theme.unwrap_or(&fallback);
    let font = font.unwrap_or_else(|| FontId::monospace(12.5));
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click());
    ui.painter().rect_filled(rect, 0.0, theme.bg);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, "Terminal not available yet", font, theme.ansi[8]);
    TermResponse {
        wants_focus: response.clicked() && !focused,
        response,
        link_clicked: None,
        escape_on_empty_line: false,
    }
}
