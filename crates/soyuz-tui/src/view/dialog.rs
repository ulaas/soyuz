//! The dialog fallback (`crate::dialog`): a box (`[tui] borders`) in the middle with the body and
//! the key hints. Text is wrapped by display width ([`super::text::wrap`]), so the box is
//! exactly as tall as its content (T3 brings the real dialog views).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};

use super::text;
use crate::borders;
use crate::dialog::DialogView;
use crate::layout::Hits;
use crate::theme::{Role, TuiTheme};

/// Draw `d` centred in `area`.
pub fn draw_dialog(f: &mut Frame<'_>, area: Rect, d: &DialogView, theme: &TuiTheme, _hits: &mut Hits) {
    if area.is_empty() {
        return;
    }
    let width = area.width.saturating_sub(4).clamp(1, 72).min(area.width);
    let inner_w = usize::from(width.saturating_sub(2)).max(1);
    // Wrapped body lines, one blank line, the wrapped hints.
    let mut lines: Vec<Line<'_>> = d
        .body
        .iter()
        .flat_map(|l| text::wrap(l, inner_w))
        .map(|l| Line::from(l).style(theme.style(Role::Text)))
        .collect();
    lines.push(Line::from(""));
    lines.extend(text::wrap(&d.hints, inner_w).into_iter().map(|l| Line::from(l).style(theme.style(Role::KeyHint))));
    let height = (lines.len() + 2).min(usize::from(area.height)) as u16;
    let r = Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 2, width, height };
    f.render_widget(Clear, r);
    let title = text::truncate_end(&d.title, inner_w.saturating_sub(2));
    let block = borders::overlay_block(theme.borders(), theme.style(Role::Border)).title(format!(" {title} "));
    f.render_widget(Paragraph::new(lines).block(block), r);
}
