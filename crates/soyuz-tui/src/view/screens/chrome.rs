//! What the two tool screens draw alike (Phase T7): the title row with its close mark,
//! `[label]` buttons, a progress row, a centred note, the inline confirm bar and the
//! one-row text writers they are built from.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::layout::{HitTarget, Hits};
use crate::theme::{Role, TuiTheme};
use crate::view::text;

/// `HitTarget::ConfirmBar` ids.
pub mod confirm {
    /// Move to the trash.
    pub const YES: u8 = 0;
    pub const CANCEL: u8 = 1;
}

/// Write `s` at `(x, y)`, at most up to `end` (cut with "…"); returns the next column.
pub fn put(buf: &mut Buffer, x: u16, y: u16, end: u16, s: &str, style: Style) -> u16 {
    if x >= end || s.is_empty() {
        return x;
    }
    let max = usize::from(end - x);
    let s = text::truncate_end(s, max);
    let (nx, _) = buf.set_stringn(x, y, &s, max, style);
    nx.min(end)
}

/// Write `s` as a click target; returns the next column.
#[allow(clippy::too_many_arguments)]
pub fn put_hit(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    end: u16,
    s: &str,
    style: Style,
    hits: &mut Hits,
    t: HitTarget,
) -> u16 {
    let nx = put(buf, x, y, end, s, style);
    if nx > x {
        hits.push(Rect::new(x, y, nx - x, 1), t);
    }
    nx
}

/// A `[label]` button followed by one blank; returns the next column. A button that does
/// not fit whole is left out.
#[allow(clippy::too_many_arguments)]
pub fn button(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    end: u16,
    label: &str,
    role: Role,
    theme: &TuiTheme,
    hits: &mut Hits,
    t: HitTarget,
) -> u16 {
    let s = format!("[{label}]");
    if x.saturating_add(text::width(&s) as u16) > end {
        return x;
    }
    let nx = put_hit(buf, x, y, end, &s, theme.style(role), hits, t);
    nx.saturating_add(1).min(end)
}

/// Width of `[label]` and its blank.
pub fn button_w(label: &str) -> u16 {
    text::width(label) as u16 + 3
}

/// The title row: bold title, muted subtitle, `×` at the right edge (`ScreenClose`).
pub fn header(buf: &mut Buffer, r: Rect, title: &str, sub: &str, theme: &TuiTheme, hits: &mut Hits) {
    if r.is_empty() {
        return;
    }
    let close_w = 3.min(r.width);
    let end = r.right() - close_w;
    let x = put(buf, r.x, r.y, end, title, theme.style(Role::Title).add_modifier(Modifier::BOLD));
    if !sub.is_empty() {
        put(buf, x.saturating_add(2).min(end), r.y, end, sub, theme.style(Role::Muted));
    }
    put_hit(buf, end, r.y, r.right(), " × ", theme.style(Role::Muted), hits, HitTarget::ScreenClose);
}

/// A progress bar (`done` of `total`; a dotted track while the total is unknown) and
/// `label` after it, in one row.
pub fn progress(buf: &mut Buffer, r: Rect, done: u64, total: u64, label: &str, theme: &TuiTheme) {
    if r.is_empty() {
        return;
    }
    let bar_w = (r.width / 4).clamp(0, 24);
    if bar_w >= 4 {
        let fill = (u64::from(bar_w) * done.min(total)).checked_div(total).unwrap_or(0) as u16;
        let full = if theme.ascii_only() { "#" } else { "█" };
        let empty = if theme.ascii_only() { "." } else { "░" };
        put(buf, r.x, r.y, r.x + fill, &full.repeat(usize::from(fill)), theme.style(Role::Progress));
        put(
            buf,
            r.x + fill,
            r.y,
            r.x + bar_w,
            &empty.repeat(usize::from(bar_w - fill)),
            theme.style(Role::ProgressTrack),
        );
        put(buf, r.x + bar_w + 1, r.y, r.right(), label, theme.style(Role::Muted));
    } else {
        put(buf, r.x, r.y, r.right(), label, theme.style(Role::Muted));
    }
}

/// A note centred in `r`: a title in `role` and wrapped muted text.
pub fn note(buf: &mut Buffer, r: Rect, title: &str, body: &str, role: Role, theme: &TuiTheme) {
    if r.is_empty() {
        return;
    }
    let mut lines = vec![(title.to_string(), theme.style(role).add_modifier(Modifier::BOLD))];
    if !body.is_empty() {
        lines.extend(text::wrap(body, usize::from(r.width)).into_iter().map(|l| (l, theme.style(Role::Muted))));
    }
    let top = r.y + r.height.saturating_sub(lines.len() as u16) / 2;
    for ((s, style), y) in lines.iter().zip(top..r.bottom()) {
        let w = (text::width(s) as u16).min(r.width);
        put(buf, r.x + (r.width - w) / 2, y, r.right(), s, *style);
    }
}

/// The inline confirm bar: the question in the danger role, then `[yes]` and `[cancel]`
/// (`ConfirmBar` targets) at the right when they fit.
pub fn confirm_bar(
    buf: &mut Buffer,
    r: Rect,
    question: &str,
    yes: &str,
    cancel: &str,
    theme: &TuiTheme,
    hits: &mut Hits,
) {
    if r.is_empty() {
        return;
    }
    let yes = format!("Enter {yes}");
    let cancel = format!("Esc {cancel}");
    let bw = button_w(&yes) + button_w(&cancel);
    let (text_end, bx) = if r.width >= bw + 8 { (r.right() - bw, r.right() - bw) } else { (r.right(), r.right()) };
    put(buf, r.x, r.y, text_end.saturating_sub(1).max(r.x), question, theme.style(Role::Danger));
    let x = button(buf, bx, r.y, r.right(), &yes, Role::Warning, theme, hits, HitTarget::ConfirmBar(confirm::YES));
    button(buf, x, r.y, r.right(), &cancel, Role::Button, theme, hits, HitTarget::ConfirmBar(confirm::CANCEL));
}

/// A one-row part taken from the top (`y` advances), empty when it does not fit.
pub fn take(area: Rect, y: &mut u16, end: u16) -> Rect {
    if *y < end {
        let r = Rect::new(area.x, *y, area.width, 1);
        *y += 1;
        r
    } else {
        Rect::default()
    }
}

/// A one-row part taken from the bottom (`end` moves up), empty when it does not fit.
pub fn take_bottom(area: Rect, y: u16, end: &mut u16) -> Rect {
    if *end > y {
        *end -= 1;
        Rect::new(area.x, *end, area.width, 1)
    } else {
        Rect::default()
    }
}

/// `area` without one blank column on each side when it is wide enough.
pub fn inner(area: Rect) -> Rect {
    if area.width > 8 { Rect::new(area.x + 1, area.y, area.width - 2, area.height) } else { area }
}
