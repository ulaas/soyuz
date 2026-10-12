//! Toast lines above the status line, newest last (step T2.8 styles them).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::toasts::ToastKind;

use super::text;
use crate::dialog::safe;
use crate::layout::{HitTarget, Hits};
use crate::theme::{Role, TuiTheme};

/// Toasts shown at once (the newest).
pub const MAX_TOASTS: usize = 3;

/// A toast line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToastLine {
    pub kind: ToastKind,
    pub text: String,
}

/// The toasts in view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToastsVm {
    /// Newest last.
    pub lines: Vec<ToastLine>,
}

/// Toasts that are not closing.
fn live(core: &AppCore) -> impl Iterator<Item = &soyuz_appcore::models::toasts::Live> {
    core.ui.toasts.live.iter().filter(|l| l.closing.is_none())
}

impl ToastsVm {
    pub fn from_core(core: &AppCore) -> ToastsVm {
        let live: Vec<_> = live(core).collect();
        let lines = live[live.len().saturating_sub(MAX_TOASTS)..]
            .iter()
            .map(|l| {
                let mut text = safe(&l.toast.title);
                if let Some(b) = &l.toast.body {
                    text.push_str(" — ");
                    text.push_str(&safe(b));
                }
                ToastLine { kind: l.toast.kind, text }
            })
            .collect();
        ToastsVm { lines }
    }
}

/// Rows the toasts want (`layout::Facts::toasts`).
pub fn lines(core: &AppCore) -> u16 {
    live(core).count().min(MAX_TOASTS) as u16
}

fn toast_tag(kind: ToastKind) -> &'static str {
    match kind {
        ToastKind::Info => "[i]",
        ToastKind::Success => "[ok]",
        ToastKind::Warning => "[!]",
        ToastKind::Error => "[x]",
    }
}

/// Draw the toasts into `rect`; when it is shorter than the list, the newest win.
pub fn draw_toasts(f: &mut Frame<'_>, rect: Rect, m: &ToastsVm, theme: &TuiTheme, hits: &mut Hits) {
    if rect.is_empty() {
        return;
    }
    let first = m.lines.len().saturating_sub(usize::from(rect.height));
    let w = usize::from(rect.width);
    let lines: Vec<Line<'_>> = m.lines[first..]
        .iter()
        .map(|t| {
            Line::from(text::truncate_end(&format!("{} {}", toast_tag(t.kind), t.text), w))
                .style(theme.style(Role::of_toast(t.kind)))
        })
        .collect();
    for i in 0..lines.len() {
        hits.push(Rect { y: rect.y + i as u16, height: 1, ..rect }, HitTarget::Toast(first + i));
    }
    f.render_widget(Paragraph::new(lines), rect);
}
