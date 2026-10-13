//! Toasts: boxes stacked in the top-right corner of the content area, newest on top,
//! drawn over the panes. Each box has the kind's icon and colour role with the title,
//! the body line and the action's hint (its label and key). Timed toasts count down in
//! the core's timers (`App::tick_toasts`) and go away by themselves; sticky ones carry a
//! close mark and stay until dismissed. Toasts take no layout rows ([`lines`] is 0).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::icon::IconId;
use soyuz_appcore::keys::chord_label;
use soyuz_appcore::models::toasts::ToastKind;

use super::text;
use crate::borders;
use crate::dialog::safe;
use crate::icons::{self, IconSet};
use crate::layout::{HitTarget, Hits, Regions};
use crate::theme::{Role, TuiTheme};

/// Toasts shown at once (the newest).
pub const MAX_TOASTS: usize = 3;
/// Widest box, in cells (borders included).
pub const MAX_WIDTH: u16 = 48;
/// Narrowest box (unless the screen is narrower).
pub const MIN_WIDTH: u16 = 24;

/// A toast.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToastLine {
    /// `Live::id` (to dismiss it).
    pub id: u64,
    pub kind: ToastKind,
    pub title: String,
    pub body: Option<String>,
    /// The action's hint: "Undo (Ctrl+Z)".
    pub action: Option<String>,
    /// Stays until dismissed (no timer).
    pub sticky: bool,
}

/// The toasts in view (drawn with the theme's icon set and box lines).
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
            .map(|l| ToastLine {
                id: l.id,
                kind: l.toast.kind,
                title: safe(&l.toast.title),
                body: l.toast.body.as_deref().map(safe),
                action: l.toast.action.as_ref().map(|(label, action)| match chord_label(&core.keymap, action) {
                    Some(k) => format!("{} ({k})", safe(label)),
                    None => safe(label),
                }),
                sticky: l.toast.duration.is_none(),
            })
            .collect();
        ToastsVm { lines }
    }
}

/// Rows the toasts take from the layout (`layout::Facts::toasts`): none, they float.
pub fn lines(_core: &AppCore) -> u16 {
    0
}

/// Where toasts may go: below the tab strip, the location line and the search bar
/// (while it is open, so toasts never cover its buttons), above the status line (the
/// whole screen when only the list is drawn).
pub fn area(r: &Regions) -> Rect {
    if r.list_only {
        return r.area;
    }
    let top = r.tabs.bottom().max(r.crumbs.bottom()).max(r.search.bottom()).max(r.area.y);
    let bottom = if r.status.is_empty() { r.area.bottom() } else { r.status.y };
    Rect { y: top, height: bottom.saturating_sub(top), ..r.area }
}

/// The icon of a toast kind (the GUI's glyphs).
pub fn toast_icon(kind: ToastKind) -> IconId {
    match kind {
        ToastKind::Info => IconId::Info,
        ToastKind::Success => IconId::SealCheck,
        ToastKind::Warning => IconId::WarningCircle,
        ToastKind::Error => IconId::XCircle,
    }
}

/// The mark in front of the title: the icon, or a tag in ASCII.
fn toast_mark(kind: ToastKind, set: IconSet) -> &'static str {
    match set {
        IconSet::Ascii => match kind {
            ToastKind::Info => "[i]",
            ToastKind::Success => "[ok]",
            ToastKind::Warning => "[!]",
            ToastKind::Error => "[x]",
        },
        _ => icons::glyph(toast_icon(kind), set),
    }
}

/// Draw the toasts into the top-right corner of `area` ([`area`]), newest on top;
/// older ones that do not fit are left out.
///
/// Screen-reader mode (T8): no boxes; the newest toast is written into the status row
/// instead (`crate::a11y::status_line`).
pub fn draw_toasts(f: &mut Frame<'_>, area: Rect, m: &ToastsVm, theme: &TuiTheme, hits: &mut Hits) {
    if area.is_empty() || theme.screen_reader() {
        return;
    }
    let mut y = area.y;
    for (i, t) in m.lines.iter().enumerate().rev() {
        let role = Role::of_toast(t.kind);
        let mark = toast_mark(t.kind, theme.icons());
        let title = format!("{mark} {}", t.title);
        let mut rows: Vec<(String, Role)> = vec![(title, Role::Title)];
        if let Some(b) = &t.body {
            rows.push((b.clone(), Role::Text));
        }
        if let Some(a) = &t.action {
            rows.push((a.clone(), Role::KeyHint));
        }
        // Borders and one cell of padding on each side.
        let want = rows.iter().map(|(s, _)| text::width(s)).max().unwrap_or(0) + 4;
        let width = (want as u16).clamp(MIN_WIDTH, MAX_WIDTH).min(area.width);
        let height = rows.len() as u16 + 2;
        if width < 5 || y + height > area.bottom() {
            break;
        }
        let r = Rect { x: area.right() - width, y, width, height };
        let inner_w = usize::from(width - 4);
        let lines: Vec<Line<'_>> = rows
            .iter()
            .enumerate()
            .map(|(row, (s, rr))| {
                let s = text::truncate_end(s, inner_w);
                if row == 0 {
                    // The mark in the kind's colour, the title bold in it too.
                    let (m, rest) = s.split_at(s.find(' ').unwrap_or(s.len()));
                    Line::from(vec![
                        Span::styled(format!(" {m}"), theme.style(role)),
                        Span::styled(rest.to_string(), theme.style(role).patch(theme.style(*rr))),
                    ])
                } else {
                    Line::from(Span::styled(format!(" {s}"), theme.style(*rr)))
                }
            })
            .collect();
        let mut block = borders::overlay_block(theme.borders(), theme.style(Role::Border));
        if t.sticky {
            block = block.title_top(Line::from(Span::styled("x", theme.style(Role::Muted))).right_aligned());
        }
        f.render_widget(Clear, r);
        f.render_widget(Paragraph::new(lines).block(block), r);
        hits.push(r, HitTarget::Toast(i));
        y += height;
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Modifier;

    use super::*;
    use crate::borders::TuiBorders;

    fn toast(id: u64, kind: ToastKind, title: &str) -> ToastLine {
        ToastLine { id, kind, title: title.into(), body: None, action: None, sticky: false }
    }

    fn draw_with(m: &ToastsVm, b: TuiBorders, w: u16, h: u16) -> (Vec<String>, ratatui::buffer::Buffer, Hits) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Hits::default();
        let theme = TuiTheme::plain().with_borders(b).with_icons(IconSet::Ascii);
        t.draw(|f| draw_toasts(f, Rect::new(0, 1, w, h - 1), m, &theme, &mut hits)).unwrap();
        let buf = t.backend().buffer().clone();
        let lines = (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect()).collect();
        (lines, buf, hits)
    }

    #[test]
    fn stacked_top_right_newest_first() {
        let mut undo = toast(2, ToastKind::Success, "Moved 3 items");
        undo.body = Some("to the trash".into());
        undo.action = Some("Undo (Ctrl+Z)".into());
        let m = ToastsVm { lines: vec![toast(1, ToastKind::Info, "Copied"), undo] };
        let (lines, buf, hits) = draw_with(&m, TuiBorders::Ascii, 40, 12);
        assert_eq!(lines[0].trim(), "", "row 0 is outside the area");
        assert_eq!(lines[1], "                +----------------------+");
        assert_eq!(lines[2], "                | [ok] Moved 3 items   |");
        assert_eq!(lines[3], "                | to the trash         |");
        assert_eq!(lines[4], "                | Undo (Ctrl+Z)        |");
        assert_eq!(lines[5], "                +----------------------+");
        assert_eq!(lines[7], "                | [i] Copied           |");
        assert!(buf[(23, 2)].modifier.contains(Modifier::BOLD), "the title is bold");
        assert!(buf[(18, 4)].modifier.contains(Modifier::BOLD), "the key hint is bold");
        assert_eq!(hits.at(20, 3), Some(HitTarget::Toast(1)));
        assert_eq!(hits.at(20, 7), Some(HitTarget::Toast(0)));
        assert_eq!(hits.at(5, 3), None);
    }

    #[test]
    fn sticky_mark_borders_and_overflow() {
        let mut s = toast(1, ToastKind::Error, "Failed");
        s.sticky = true;
        let m = ToastsVm { lines: vec![toast(0, ToastKind::Info, "old"), s] };
        // Room for one box only: the newest.
        let (lines, _, hits) = draw_with(&m, TuiBorders::Rounded, 30, 5);
        assert_eq!(lines[1], "      ╭─────────────────────x╮", "{lines:?}");
        assert_eq!(lines[2], "      │ [x] Failed           │");
        assert_eq!(lines[3], "      ╰──────────────────────╯");
        assert!(hits.iter().all(|(_, t)| *t == HitTarget::Toast(1)));
        // No lines: a blank frame.
        let (lines, _, _) = draw_with(&m, TuiBorders::None, 30, 5);
        assert_eq!(lines[2].trim_end(), "        [x] Failed");
    }

    #[test]
    fn area_starts_below_the_search_bar() {
        use crate::layout::Facts;
        use soyuz_core::config::TuiSection;
        let tui = TuiSection::default();
        let closed = Regions::compute(Rect::new(0, 0, 100, 30), &tui, &Facts::default());
        assert_eq!(area(&closed).y, closed.crumbs.bottom());
        let open = Regions::compute(Rect::new(0, 0, 100, 30), &tui, &Facts { search_rows: 2, ..Facts::default() });
        assert!(!open.search.is_empty());
        let a = area(&open);
        assert_eq!(a.y, open.search.bottom(), "toasts never cover the search bar's buttons");
        assert!(a.intersection(open.search).is_empty());
    }

    #[test]
    fn tiny_areas_never_panic() {
        let mut long = toast(1, ToastKind::Warning, &"日本語 warning ".repeat(8));
        long.body = Some("👨\u{200d}👩\u{200d}👧 body".into());
        long.action = Some("Retry (F5)".into());
        let m = ToastsVm { lines: vec![long.clone(), long.clone(), long] };
        for w in 0..=30 {
            for h in 1..=8 {
                let _ = draw_with(&m, TuiBorders::Rounded, w, h);
            }
        }
    }
}
