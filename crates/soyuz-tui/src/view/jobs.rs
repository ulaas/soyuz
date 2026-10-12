//! The bottom panel: running and finished jobs (activity) and their history — Phase T3.
//!
//! The panel is the core's activity panel (`ui.bottom.activity_shown()`): it opens by
//! itself when a job runs for more than a second (`jobs.auto_open_activity`, like the GUI;
//! the keyboard stays where it was), with `ToggleActivityPanel` (Ctrl+Shift+J) or
//! `FocusActivityPanel` (Alt+J, which also gives it the keyboard). It takes
//! `[tui] jobs_rows` rows above the status line ([`rows`], `Regions::bottom`).
//!
//! Layout: a header row with the tabs (Jobs / History, the GUI's labels), the key hints
//! while focused and a close mark; then two lines per job / history entry, drawn from the
//! core's rows (`soyuz_appcore::models::jobs_rows`, which the GUI's cards use too):
//!
//! ```text
//!  Jobs · 1 ─ History ──────────────────────────────────────── x
//! > [c] Copying "big.bin" to /dst  big.bin          [Pause] [Cancel]
//!       [#######-------]  45%  45 MiB of 100 MiB     3 MiB/s · 18 s left
//! ```
//!
//! [`prepare`] keeps the list's cursor inside the rows and records how many fit;
//! [`JobsVm::from_core`] builds only the rows in view. Click targets: `BottomTab`,
//! `JobRow` (both lines) and `JobButton` (`crate::input::jobs` handles them and the keys
//! while `TuiFocus::Jobs`).

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::icon::IconId;
use soyuz_appcore::models::jobs_rows::{self, DoneMark, HistoryRow, JobRow, RowButton, Tone};
use soyuz_appcore::models::panels::ActivityTab;
use soyuz_appcore::tr;

use crate::dialog::safe;
use crate::hits::JobButton;
use crate::icons::{self, IconSet};
use crate::input::InputState;
use crate::layout::{HitTarget, Hits, Regions};
use crate::theme::{Role, TuiTheme};
use crate::view::text;

/// Lines per job / history entry.
pub const ITEM_LINES: u16 = 2;
/// Cells of the progress bar at most.
const BAR_MAX: usize = 24;
/// The bar is left out below this panel width.
const BAR_MIN_WIDTH: usize = 40;

/// The jobs panel as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JobsVm {
    /// The panel is drawn.
    pub visible: bool,
    /// It has the keyboard.
    pub focused: bool,
    /// The tab labels (Jobs, History).
    pub tabs: Vec<String>,
    /// The active tab (0: jobs, 1: history).
    pub tab: usize,
    /// The rows in view, starting with row `first`.
    pub items: Vec<JobItem>,
    pub first: usize,
    /// Rows in the list.
    pub len: usize,
    /// The cursor row.
    pub cursor: usize,
    /// Title and hint of an empty list.
    pub empty: Option<(String, String)>,
    /// Key hints (shown while focused).
    pub hints: String,
}

/// A job or history entry as drawn (two lines).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobItem {
    pub icon: IconId,
    pub icon_tone: Tone,
    pub title: String,
    /// Undone history entries are dimmed.
    pub dim: bool,
    /// A tag after the title (Verified, Undone).
    pub tag: Option<(String, Tone)>,
    /// The state line (jobs) after the title.
    pub line: String,
    pub line_tone: Tone,
    /// The progress bar (jobs).
    pub bar: Option<Bar>,
    /// The second line's text (counts for jobs, when and what for history).
    pub stats: String,
    pub stats_tone: Tone,
    /// Right of the second line (speed, ETA).
    pub right: String,
    pub buttons: Vec<ButtonVm>,
}

/// A progress bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bar {
    /// Done, in thousandths; `None` while unknown.
    pub permille: Option<u16>,
    pub tone: Tone,
}

/// A button on a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonVm {
    pub kind: JobButton,
    pub label: String,
    pub enabled: bool,
}

/// Whether the bottom panel is open: the core's activity panel.
pub fn shown(core: &AppCore) -> bool {
    core.ui.bottom.activity_shown()
}

/// Rows the bottom panel wants (`layout::Facts::bottom_rows`): `[tui] jobs_rows` while it
/// is open, else 0.
pub fn rows(core: &AppCore) -> u16 {
    if shown(core) { core.config.tui.jobs_rows_clamped() } else { 0 }
}

/// Whether the history tab is active.
pub fn history_tab(core: &AppCore) -> bool {
    core.fileops.activity.tab == ActivityTab::History
}

/// Rows in the active tab's list.
pub fn len(core: &AppCore) -> usize {
    if history_tab(core) { core.fileops.jobs.history().len() } else { core.fileops.jobs.snapshots().len() }
}

/// Rows of `area` that fit in the list (below the header).
pub fn items_fit(area: Rect) -> usize {
    usize::from(area.height.saturating_sub(1) / ITEM_LINES)
}

/// Step 1 of a frame (`view::prepare`): the keyboard leaves a closed panel; the cursor
/// stays inside the list (back to the top when the tab changed) and in view.
pub fn prepare(core: &mut AppCore, input: &mut InputState, regions: &Regions) {
    let st = &mut input.jobs;
    if !shown(core) {
        st.focused = false;
        return;
    }
    let history = history_tab(core);
    if st.history != history {
        st.history = history;
        st.list = Default::default();
    }
    st.height = items_fit(regions.bottom).max(1);
    let n = len(core);
    st.list.clamp(n, st.height);
    if st.follow {
        st.follow = false;
        st.list.show_cursor(st.height);
    }
}

/// The role of a tone.
pub fn role(t: Tone) -> Role {
    match t {
        Tone::Text => Role::Text,
        Tone::Muted => Role::Muted,
        Tone::Faint => Role::Faint,
        Tone::Accent => Role::Accent,
        Tone::Info => Role::Info,
        Tone::Success => Role::Success,
        Tone::Warning => Role::Warning,
        Tone::Danger => Role::Danger,
        Tone::Folder => Role::Folder,
    }
}

/// The hit button of a row button.
pub fn hit_button(b: RowButton) -> JobButton {
    match b {
        RowButton::Pause => JobButton::Pause,
        RowButton::Resume => JobButton::Resume,
        RowButton::Cancel => JobButton::Cancel,
        RowButton::Resolve => JobButton::Resolve,
        RowButton::Remove => JobButton::Dismiss,
        RowButton::RetryFailed => JobButton::Retry,
        RowButton::Undo => JobButton::Undo,
        RowButton::Redo => JobButton::Redo,
    }
}

/// The row button of a hit button (`None`: not a row button).
pub fn row_button(b: JobButton) -> Option<RowButton> {
    Some(match b {
        JobButton::Pause => RowButton::Pause,
        JobButton::Resume => RowButton::Resume,
        JobButton::Cancel => RowButton::Cancel,
        JobButton::Resolve => RowButton::Resolve,
        JobButton::Dismiss => RowButton::Remove,
        JobButton::Retry => RowButton::RetryFailed,
        JobButton::Undo => RowButton::Undo,
        JobButton::Redo => RowButton::Redo,
        JobButton::Details | JobButton::Close => return None,
    })
}

/// The short label of a button.
fn label(b: RowButton) -> String {
    match b {
        RowButton::Pause => tr!("tui-jobs-pause"),
        RowButton::Resume => tr!("tui-jobs-resume"),
        RowButton::Cancel => tr!("tui-jobs-cancel"),
        RowButton::Resolve => tr!("tui-jobs-resolve"),
        RowButton::Remove => tr!("tui-jobs-remove"),
        RowButton::RetryFailed => tr!("tui-jobs-retry"),
        RowButton::Undo => tr!("tui-jobs-undo"),
        RowButton::Redo => tr!("tui-jobs-redo"),
    }
    .to_string()
}

fn job_item(r: &JobRow) -> JobItem {
    let icon = match r.done_mark {
        Some(DoneMark::Check) => IconId::SealCheck,
        Some(DoneMark::Warning) => IconId::WarningCircle,
        Some(DoneMark::Prohibit) => IconId::XCircle,
        None => jobs_rows::op_icon(r.op),
    };
    let icon_tone = if r.done_mark.is_some() { r.status } else { jobs_rows::op_tone(r.op) };
    let tag = r.badge.map(|b| (b.to_string(), if r.summary.is_some() { Tone::Success } else { Tone::Accent }));
    let permille = r.fraction.map(|f| (f.clamp(0.0, 1.0) * 1000.0).round() as u16);
    JobItem {
        icon,
        icon_tone,
        title: safe(&r.title),
        dim: false,
        tag,
        line: safe(&r.line),
        line_tone: r.line_tone,
        bar: Some(Bar { permille, tone: r.status }),
        stats: r.stats.clone(),
        stats_tone: Tone::Faint,
        right: r.speed.clone(),
        buttons: r.buttons.iter().map(|&b| ButtonVm { kind: hit_button(b), label: label(b), enabled: true }).collect(),
    }
}

fn history_item(r: &HistoryRow) -> JobItem {
    let tag = if r.undone {
        Some((tr!("panels-history-undone").to_string(), Tone::Faint))
    } else if r.verified {
        Some((tr!("panels-verified").to_string(), Tone::Success))
    } else {
        None
    };
    JobItem {
        icon: jobs_rows::op_icon(r.op),
        icon_tone: if r.undone { Tone::Faint } else { jobs_rows::op_tone(r.op) },
        title: safe(&r.title),
        dim: r.undone,
        tag,
        line: String::new(),
        line_tone: Tone::Muted,
        bar: None,
        stats: safe(&r.sub),
        stats_tone: if r.failed { Tone::Warning } else { Tone::Faint },
        right: String::new(),
        buttons: r
            .buttons
            .iter()
            .map(|&(b, enabled)| ButtonVm { kind: hit_button(b), label: label(b), enabled })
            .collect(),
    }
}

impl JobsVm {
    pub fn from_core(core: &AppCore, input: &InputState) -> JobsVm {
        let visible = shown(core);
        if !visible {
            return JobsVm::default();
        }
        let st = &input.jobs;
        let history = history_tab(core);
        let n = len(core);
        let range = st.list.visible(n, st.height.max(1));
        let items: Vec<JobItem> = if history {
            let now = SystemTime::now();
            let all = core.fileops.jobs.history();
            range.clone().map(|i| history_item(&HistoryRow::new(&all[all.len() - 1 - i], now))).collect()
        } else {
            let snaps = jobs_rows::sorted_snapshots(core);
            snaps[range.clone()].iter().map(|s| job_item(&JobRow::new(core, s))).collect()
        };
        let empty = (n == 0).then(|| {
            if history {
                (tr!("panels-history-empty").to_string(), tr!("panels-history-empty-hint").to_string())
            } else {
                (tr!("panels-jobs-empty").to_string(), tr!("panels-jobs-empty-hint").to_string())
            }
        });
        JobsVm {
            visible,
            focused: st.focused,
            tabs: vec![jobs_rows::jobs_tab_label(core), tr!("panels-tab-history").to_string()],
            tab: usize::from(history),
            items,
            first: range.start,
            len: n,
            cursor: st.list.cursor,
            empty,
            hints: if history { tr!("tui-jobs-hints-history") } else { tr!("tui-jobs-hints") }.to_string(),
        }
    }
}

/// Write `s` at `(x, y)`, at most up to `end` (cut with "…"); returns the next column.
fn put(buf: &mut Buffer, x: u16, y: u16, end: u16, s: &str, style: Style) -> u16 {
    if x >= end || s.is_empty() {
        return x;
    }
    let max = usize::from(end - x);
    let s = text::truncate_end(s, max);
    let (nx, _) = buf.set_stringn(x, y, &s, max, style);
    nx.min(end)
}

/// The cells of a bar `cells` wide: `(filled, empty)` glyph and how many are filled.
fn bar_cells(permille: Option<u16>, cells: usize) -> usize {
    permille.map_or(0, |p| (usize::from(p.min(1000)) * cells + 500) / 1000)
}

/// Draw the panel into `area` (`Regions::bottom`).
pub fn draw_jobs(f: &mut Frame<'_>, area: Rect, m: &JobsVm, theme: &TuiTheme, hits: &mut Hits) {
    let area = area.intersection(f.area());
    if area.is_empty() || !m.visible {
        return;
    }
    let buf = f.buffer_mut();
    let base = theme.style(Role::Text);
    buf.set_style(area, base);
    let ascii = theme.icons() == IconSet::Ascii || theme.ascii_only();
    let set = theme.icons();
    // ---- header: tabs, hints, close ----
    let y = area.y;
    let end = area.right();
    let line = crate::borders::lines(theme.borders()).horizontal;
    let line_style = theme.style(if m.focused { Role::BorderFocused } else { Role::Border });
    for x in area.x..end {
        buf.set_string(x, y, line, line_style);
    }
    let close = icons::glyph(IconId::X, set);
    let close_w = text::width(close) as u16 + 2;
    let tabs_end = end.saturating_sub(close_w);
    let mut x = area.x + 1.min(area.width);
    for (i, t) in m.tabs.iter().enumerate() {
        let role = if i == m.tab { Role::TabActive } else { Role::TabInactive };
        let start = x;
        x = put(buf, x, y, tabs_end, &format!(" {t} "), theme.style(role));
        hits.push(Rect::new(start, y, x - start, 1), HitTarget::BottomTab(i));
        x = (x + 1).min(tabs_end);
    }
    if m.focused && !m.hints.is_empty() {
        let w = text::width(&m.hints) as u16 + 2;
        if x + w < tabs_end {
            put(buf, tabs_end - w, y, tabs_end, &format!(" {} ", m.hints), theme.style(Role::KeyHint));
        }
    }
    if close_w <= area.width {
        let cx = end - close_w;
        put(buf, cx, y, end, &format!(" {close} "), theme.style(Role::Muted));
        hits.push(Rect::new(cx, y, close_w, 1), HitTarget::JobButton { row: 0, button: JobButton::Close });
    }
    // ---- the list ----
    let body = Rect { y: area.y + 1, height: area.height - 1, ..area };
    if body.is_empty() {
        return;
    }
    if let Some((title, hint)) = &m.empty {
        let center = |buf: &mut Buffer, y: u16, s: &str, role: Role| {
            let s = text::truncate_end(s, usize::from(body.width));
            let w = text::width(&s) as u16;
            buf.set_string(body.x + (body.width - w) / 2, y, &s, theme.style(role));
        };
        let mid = body.y + body.height.saturating_sub(2) / 2;
        center(buf, mid, title, Role::Muted);
        if mid + 1 < body.bottom() {
            center(buf, mid + 1, hint, Role::Faint);
        }
        return;
    }
    for (k, item) in m.items.iter().enumerate() {
        let index = m.first + k;
        let top = body.y + (k as u16) * ITEM_LINES;
        if top >= body.bottom() {
            break;
        }
        let h = ITEM_LINES.min(body.bottom() - top);
        let rect = Rect::new(body.x, top, body.width, h);
        let cursor = m.focused && index == m.cursor;
        if cursor {
            buf.set_style(rect, theme.style(Role::Selected));
        }
        hits.push(rect, HitTarget::JobRow(index));
        draw_item(buf, rect, item, index, cursor, ascii, theme, hits);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_item(
    buf: &mut Buffer,
    rect: Rect,
    item: &JobItem,
    index: usize,
    cursor: bool,
    ascii: bool,
    theme: &TuiTheme,
    hits: &mut Hits,
) {
    let style = |r: Role| {
        let s = theme.style(r);
        if cursor { theme.style(Role::Selected).patch(s) } else { s }
    };
    let (x0, end) = (rect.x, rect.right());
    let y = rect.y;
    // Buttons, right-aligned on the first line.
    let mut bx = end;
    let mut placed = Vec::new();
    for b in item.buttons.iter().rev() {
        let s = format!("[{}]", b.label);
        let w = text::width(&s) as u16;
        if bx < x0 + w + 12 {
            break;
        }
        bx -= w;
        placed.push((bx, s, b));
        bx = bx.saturating_sub(1);
    }
    for (bxx, s, b) in &placed {
        let role = if b.enabled { Role::Button } else { Role::Disabled };
        put(buf, *bxx, y, end, s, theme.style(role));
        if b.enabled {
            hits.push(
                Rect::new(*bxx, y, text::width(s) as u16, 1),
                HitTarget::JobButton { row: index, button: b.kind },
            );
        }
    }
    let text_end = if placed.is_empty() { end } else { bx };
    let mut x = put(buf, x0, y, text_end, if cursor { "> " } else { "  " }, style(Role::KeyHint));
    x = put(buf, x, y, text_end, icons::glyph(item.icon, theme.icons()), style(role(item.icon_tone)));
    x = put(buf, x, y, text_end, " ", style(Role::Text));
    let title_role = if item.dim { Role::Faint } else { Role::Title };
    x = put(buf, x, y, text_end, &item.title, style(title_role));
    if let Some((t, tone)) = &item.tag {
        x = put(buf, x, y, text_end, &format!(" ({t})"), style(role(*tone)));
    }
    if !item.line.is_empty() {
        put(buf, x, y, text_end, &format!("  {}", item.line), style(role(item.line_tone)));
    }
    // Second line.
    if rect.height < 2 {
        return;
    }
    let y = y + 1;
    let mut x = (x0 + 4).min(end);
    let right_w = text::width(&item.right) as u16;
    let right_x = if right_w > 0 && right_w + 20 < rect.width { end - right_w - 1 } else { end };
    if right_x < end {
        put(buf, right_x, y, end, &item.right, style(Role::Muted));
    }
    let w = usize::from(rect.width);
    if let Some(bar) = item.bar
        && w >= BAR_MIN_WIDTH
    {
        let cells = (w / 4).min(BAR_MAX);
        let filled = bar_cells(bar.permille, cells);
        let (on, off) = if ascii { ("#", "-") } else { ("\u{2588}", "\u{2591}") };
        let fill_role = if bar.tone == Tone::Accent { Role::Progress } else { role(bar.tone) };
        x = put(buf, x, y, right_x, "[", style(Role::Faint));
        x = put(buf, x, y, right_x, &on.repeat(filled), style(fill_role));
        x = put(buf, x, y, right_x, &off.repeat(cells - filled), style(Role::ProgressTrack));
        x = put(buf, x, y, right_x, "]", style(Role::Faint));
        let pct = bar.permille.map(|p| format!(" {:>3}%", (p + 5) / 10)).unwrap_or_default();
        x = put(buf, x, y, right_x, &pct, style(Role::Muted));
        x = put(buf, x, y, right_x, "  ", style(Role::Text));
    } else if let Some(Bar { permille: Some(p), .. }) = item.bar {
        x = put(buf, x, y, right_x, &format!("{:>3}%  ", (p + 5) / 10), style(Role::Muted));
    }
    let stats_end = if right_x < end { right_x.saturating_sub(1) } else { end };
    put(buf, x, y, stats_end.max(x), &item.stats, style(role(item.stats_tone)));
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn item(title: &str, permille: Option<u16>) -> JobItem {
        JobItem {
            icon: IconId::Copy,
            icon_tone: Tone::Accent,
            title: title.into(),
            dim: false,
            tag: None,
            line: "big.bin".into(),
            line_tone: Tone::Muted,
            bar: Some(Bar { permille, tone: Tone::Accent }),
            stats: "45 MiB of 100 MiB".into(),
            stats_tone: Tone::Faint,
            right: "3 MiB/s".into(),
            buttons: vec![
                ButtonVm { kind: JobButton::Pause, label: "Pause".into(), enabled: true },
                ButtonVm { kind: JobButton::Cancel, label: "Cancel".into(), enabled: true },
            ],
        }
    }

    fn vm() -> JobsVm {
        JobsVm {
            visible: true,
            focused: true,
            tabs: vec!["Jobs · 1".into(), "History".into()],
            tab: 0,
            items: vec![item("Copying big.bin", Some(450)), item("Second", None)],
            first: 0,
            len: 2,
            cursor: 1,
            empty: None,
            hints: "P pause".into(),
        }
    }

    fn draw(m: &JobsVm, w: u16, h: u16) -> (Vec<String>, Hits) {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Hits::default();
        let theme = TuiTheme::plain().with_icons(IconSet::Ascii);
        t.draw(|f| draw_jobs(f, Rect::new(0, 0, w, h), m, &theme, &mut hits)).unwrap();
        let buf = t.backend().buffer().clone();
        let lines = (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>()).collect();
        (lines, hits)
    }

    #[test]
    fn rows_bars_buttons_and_targets() {
        let (lines, hits) = draw(&vm(), 80, 5);
        assert!(lines[0].contains(" Jobs · 1 ") && lines[0].contains(" History "), "{:?}", lines[0]);
        assert!(lines[0].contains("P pause"), "{:?}", lines[0]);
        assert!(lines[1].contains("Copying big.bin  big.bin"), "{:?}", lines[1]);
        assert!(lines[1].trim_end().ends_with("[Pause] [Cancel]"), "{:?}", lines[1]);
        assert!(lines[2].contains("[#########-----------]  45%  45 MiB of 100 MiB"), "{:?}", lines[2]);
        assert!(lines[2].trim_end().ends_with("3 MiB/s"), "{:?}", lines[2]);
        assert!(lines[3].starts_with("> "), "the cursor row: {:?}", lines[3]);
        assert_eq!(hits.at(2, 0), Some(HitTarget::BottomTab(0)));
        assert_eq!(hits.at(30, 2), Some(HitTarget::JobRow(0)));
        assert_eq!(hits.at(5, 4), Some(HitTarget::JobRow(1)));
        let pause = lines[1].find("[Pause]").unwrap() as u16;
        assert_eq!(hits.at(pause + 1, 1), Some(HitTarget::JobButton { row: 0, button: JobButton::Pause }));
        assert_eq!(hits.at(79, 0), Some(HitTarget::JobButton { row: 0, button: JobButton::Close }));
    }

    #[test]
    fn empty_list_and_unfocused() {
        let mut m = vm();
        m.items.clear();
        m.len = 0;
        m.focused = false;
        m.empty = Some(("No operations".into(), "Copies show here.".into()));
        let (lines, _) = draw(&m, 40, 5);
        assert!(!lines[0].contains("P pause"), "hints only while focused");
        assert!(lines.iter().any(|l| l.contains("No operations")), "{lines:?}");
    }

    #[test]
    fn bar_rounding() {
        assert_eq!(bar_cells(None, 10), 0);
        assert_eq!(bar_cells(Some(0), 10), 0);
        assert_eq!(bar_cells(Some(449), 10), 4);
        assert_eq!(bar_cells(Some(1000), 10), 10);
        assert_eq!(bar_cells(Some(5000), 10), 10);
    }

    #[test]
    fn tiny_sizes_never_panic() {
        let mut empty = vm();
        empty.items.clear();
        empty.empty = Some(("日本語 empty".into(), "hint".into()));
        let mut wide = vm();
        wide.items[0].title = "日本語 👨\u{200d}👩\u{200d}👧 ".repeat(5);
        for m in [vm(), empty, wide] {
            for w in 0..=20 {
                for h in 0..=6 {
                    let _ = draw(&m, w, h);
                }
            }
        }
    }
}
