//! The system monitor in the terminal: what a pane shows at `monitor:`, the same thirteen
//! pages as the GUI (`soyuz_core::config::monitor::MonitorPage`) on the core's shared state
//! (`soyuz_appcore::models::monitor`).
//!
//! The frame's three steps (`crate::view`) work like this here:
//! 1. [`prepare`] has `&mut AppCore`: it reads the table cache and the series and lays the
//!    page out into a [`Canvas`] (text runs, painted rects, click targets) of the pane's
//!    size. No OS call happens here: frames, lists and details all arrive through the
//!    core's workers; a redraw comes from the hub's wake.
//! 2. [`decorate`] puts the finished [`MonitorVm`] on the pane's model.
//! 3. [`MonitorVm::draw`] replays the canvas.
//!
//! Identity is never colour alone: charts have legends and differ in shape, bars carry
//! numbers, change marks are `+ - ~`. Screen-reader mode ([`Look::sr`]) draws no charts or
//! bars (a text line says the values) and keeps one thing per row; [`MonitorVm::announce`]
//! starts the status sentence.
//!
//! Keys and mouse: `crate::input::monitor`.

pub mod canvas;
pub mod chart;
pub mod dialogs;
mod hardware;
mod help;
mod history;
mod lists;
mod overview;
mod processes;
mod snapshots;

use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::monitor::{MonitorState, labels};
use soyuz_appcore::state::pane::PaneId;
use soyuz_appcore::tr;
use soyuz_core::config::Config;
use soyuz_core::config::monitor::MonitorPage;
use soyuz_core::{Location, format_size};
use soyuz_sysmon::{Frame, Pt};

pub use self::canvas::Hit;
use self::canvas::{Area, Canvas};
use self::chart::{Chart, ChartStyle, Series};
pub use self::history::step_cursor;
pub use self::processes::{fit_columns, next_view};
pub use self::snapshots::DIFF_SORTS;
use crate::icons::IconSet;
use crate::input::InputState;
use crate::input::monitor::{FieldKind, PaneNav};
use crate::layout::{Hits, Regions};
use crate::theme::{Role, TuiTheme};
use crate::view::PaneVm;
use crate::view::text;

/// A monitor page, ready to draw.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MonitorVm {
    pub page: MonitorPage,
    pub focused: bool,
    pub canvas: Canvas,
    /// What a screen reader hears first: the focused row, or the page.
    pub announce: String,
    /// Rows of the page's table (processes: after the filter).
    pub rows: usize,
    /// Processes that match the filter.
    pub matches: usize,
    /// The filter's parse error.
    pub filter_error: Option<String>,
    /// The help overlay is open.
    pub help: bool,
}

impl MonitorVm {
    /// Draw into `rect`.
    pub fn draw(&self, buf: &mut Buffer, rect: Rect, pane: PaneId, theme: &TuiTheme, hits: &mut Hits) {
        self.canvas.draw(buf, rect, pane, self.focused, theme, hits);
    }

    /// The page's text, one string per drawing step (tests).
    pub fn text(&self) -> String {
        let mut rows: Vec<(u16, u16, &str)> = self
            .canvas
            .ops
            .iter()
            .filter_map(|o| match o {
                canvas::Op::Text { x, y, text, .. } => Some((*y, *x, text.as_str())),
                _ => None,
            })
            .collect();
        rows.sort_by_key(|(y, x, _)| (*y, *x));
        rows.iter().map(|(_, _, t)| *t).collect::<Vec<_>>().join(" ")
    }
}

/// How this frame draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    pub icons: IconSet,
    pub ascii: bool,
    /// Screen-reader mode: no charts, no bars.
    pub sr: bool,
    pub chart: ChartStyle,
}

/// What building a page works with.
pub(super) struct Cx<'a> {
    pub c: &'a mut Canvas,
    pub config: &'a Config,
    pub nav: &'a mut PaneNav,
    pub look: Look,
    pub focused: bool,
    pub announce: String,
    pub rows: usize,
    pub matches: usize,
    pub filter_error: Option<String>,
}

impl Cx<'_> {
    pub fn size(&self, b: u64) -> String {
        format_size(b, self.config.general.size_style)
    }

    pub fn rate(&self, r: f64) -> String {
        labels::rate(r, self.config.general.size_style)
    }

    /// The window of the live charts: `(from, to)`.
    pub fn window(&self, frame: &Frame) -> (i64, i64) {
        (frame.at_ms - i64::from(self.config.monitor.graph_seconds) * 1000, frame.at_ms)
    }

    /// A chart, or in screen-reader mode one line that says the same.
    pub fn chart(&mut self, a: Area, chart: &Chart<'_>) {
        if self.look.sr {
            let parts: Vec<String> = chart
                .series
                .iter()
                .map(|s| {
                    let v = chart::latest(s.pts).map(chart.fmt).unwrap_or_else(|| "–".into());
                    format!("{} {v}", s.label)
                })
                .collect();
            self.c.text(a.x, a.y, a.w, &parts.join(", "), Role::Text);
        } else {
            chart::draw(self.c, a, chart, self.look.chart);
        }
    }

    /// How high a chart of `rows` is drawn.
    pub fn chart_h(&self, rows: u16) -> u16 {
        if self.look.sr { 1 } else { rows }
    }

    /// A bar, nothing in screen-reader mode.
    pub fn bar(&mut self, a: Area, share: f32, role: Role) {
        if !self.look.sr {
            chart::bar(self.c, a, share, role, self.look.chart);
        }
    }

    /// A sparkline, nothing in screen-reader mode.
    pub fn spark(&mut self, a: Area, pts: &[Pt], window: (i64, i64), y_max: Option<f32>, role: Role) {
        if !self.look.sr {
            chart::spark(self.c, a, pts, window, y_max, role, self.look.chart);
        }
    }

    /// The page's one-line caption: facts on the left, key hints on the right.
    pub fn info(&mut self, a: Area, left: &str, right: &str) {
        let lw = text::width(left) as u16;
        self.c.text(a.x, a.y, a.w, left, Role::Muted);
        // One thing per row for a reader; the hints are in the help.
        if !self.look.sr && !right.is_empty() && lw + 2 < a.w {
            let room = a.w - lw - 2;
            let rw = text::width(right) as u16;
            if rw <= room {
                self.c.text(a.x + a.w - rw, a.y, rw, right, Role::KeyHint);
            }
        }
    }

    /// A one-line filter: the open field, or what filters now.
    pub fn filter_line(&mut self, a: Area, kind: FieldKind, shown: &str, count: Option<(usize, usize)>, extra: &str) {
        if a.h == 0 || a.w == 0 {
            return;
        }
        let field = self.nav.field.as_mut().filter(|f| f.kind == kind);
        let editing = field.is_some();
        let label = "/ ";
        let suffix = match count {
            Some((n, of)) => format!("{extra} {}", tr!("tui-monitor-match-count", count = n, total = of)),
            None => extra.to_string(),
        };
        let sw = text::width(&suffix) as u16;
        let room = a.w.saturating_sub(2 + sw + 1).max(1);
        self.c.text(a.x, a.y, 2, label, Role::KeyHint);
        let (body, col) = match field {
            Some(f) => {
                let (t, col) = f.edit.view(&crate::dialog::safe(&f.text), room);
                (t, Some(col))
            }
            None => (text::truncate_end(&crate::dialog::safe(shown), usize::from(room)), None),
        };
        self.c.text(a.x + 2, a.y, room, &body, if editing { Role::Input } else { Role::Text });
        if let Some(col) = col {
            let x = a.x + 2 + col.min(room.saturating_sub(1));
            self.c.paint(Area { x, w: 1, ..a }, Role::Cursor);
            self.c.set_caret(x, a.y);
        }
        if sw > 0 && sw + 3 < a.w {
            self.c.text(a.x + a.w - sw, a.y, sw, &suffix, Role::Muted);
        }
        self.c.hit(a, Hit::FilterLine);
    }
}

/// Step 1: lay out the page of each drawn monitor pane.
pub fn prepare(core: &mut AppCore, input: &mut InputState, regions: &Regions) {
    let tab = core.ws.tab();
    let shown: Vec<(PaneId, Rect, bool)> = regions
        .panes
        .iter()
        .filter_map(|p| {
            let pane = tab.panes.get(p.index)?;
            matches!(pane.location, Location::Monitor).then_some((pane.id, p.rect, p.index == tab.focused))
        })
        .collect();
    input.monitor.vms.retain(|id, _| shown.iter().any(|(s, ..)| s == id));
    input.monitor.panes.retain(|id, _| core.ws.find_pane(*id).is_some());
    let look = {
        let icons = IconSet::from_config(&core.config.tui);
        let sr = regions.screen_reader;
        Look {
            icons: if sr { IconSet::Ascii } else { icons },
            ascii: icons == IconSet::Ascii,
            sr,
            chart: ChartStyle::of(icons),
        }
    };
    for (id, rect, focused) in shown {
        let nav = input.monitor.panes.entry(id).or_default();
        let vm = build(core, nav, id, rect, focused, look);
        // The affinity editor opens when the core has read the process's CPU set.
        let wants = core.ui.monitor.peek(id).is_some_and(|v| v.affinity.is_some());
        let open = wants && !nav.affinity_open;
        nav.affinity_open = wants;
        input.monitor.vms.insert(id, Arc::new(vm));
        if open {
            input.dialogs.tui.push(crate::input::monitor::dialogs::affinity_dialog(id));
        }
    }
}

/// The status line says what fits a page, not a file list's counts and sort.
pub fn decorate_status(input: &InputState, core: &AppCore, status: &mut crate::view::StatusVm) {
    let Some(vm) = input.monitor.vms.get(&core.ws.pane().id) else { return };
    status.counts = labels::page_title(vm.page).to_string();
    status.hidden.clear();
    status.selected.clear();
    status.free.clear();
    status.mode.clear();
    status.sort.clear();
    status.hint = tr!("tui-monitor-status-hint").to_string();
}

/// Step 2: the finished pages go on the panes' models.
pub fn decorate(input: &InputState, panes: &mut [PaneVm]) {
    for p in panes {
        if let Some(vm) = input.monitor.vms.get(&p.id) {
            p.monitor = Some(vm.clone());
            p.message = None;
            p.footer = None;
            p.lines.clear();
        }
    }
}

/// Lay out the page of pane `pane` for a `rect`-sized canvas.
fn build(core: &mut AppCore, nav: &mut PaneNav, pane: PaneId, rect: Rect, focused: bool, look: Look) -> MonitorVm {
    let cfg = core.config.monitor.clone();
    let AppCore { ui, config, .. } = core;
    let m = &mut ui.monitor;
    let page = m.view(pane, &cfg).page;
    let mut c = Canvas::new(rect.width, rect.height);
    let mut cx = Cx {
        c: &mut c,
        config,
        nav,
        look,
        focused,
        announce: labels::page_title(page).to_string(),
        rows: 0,
        matches: 0,
        filter_error: None,
    };
    let paused = m.paused();
    let mut a = Area { x: 0, y: 0, w: rect.width, h: rect.height };
    strip(&mut cx, a.take_top(1), page, paused);
    draw_page(&mut cx, m, pane, page, a);
    let help = cx.nav.help.is_some();
    if help {
        help::draw(&mut cx, Area { x: 0, y: 0, w: rect.width, h: rect.height }, page);
    }
    let (announce, rows, matches, filter_error) = (cx.announce, cx.rows, cx.matches, cx.filter_error);
    MonitorVm { page, focused, canvas: c, announce, rows, matches, filter_error, help }
}

fn draw_page(cx: &mut Cx<'_>, m: &mut MonitorState, pane: PaneId, page: MonitorPage, a: Area) {
    if a.h == 0 {
        return;
    }
    let frame = m.frame.clone();
    let generation = m.generation;
    let cfg = cx.config.monitor.clone();
    // Pages that can draw without a reading.
    match page {
        MonitorPage::Snapshots => return snapshots::draw(cx, m.view(pane, &cfg), a),
        MonitorPage::Services => return lists::services(cx, m.view(pane, &cfg), frame.as_deref(), generation, a),
        MonitorPage::Startup => return lists::startup(cx, m.view(pane, &cfg), frame.as_deref(), generation, a),
        MonitorPage::Connections => return lists::connections(cx, m.view(pane, &cfg), frame.as_deref(), generation, a),
        _ => {}
    }
    let Some(frame) = frame else {
        message(cx, a, tr!("monitor-no-data"), false);
        return;
    };
    match page {
        MonitorPage::Overview => overview::draw(cx, m, &frame, a),
        MonitorPage::Processes => processes::draw(cx, m.view(pane, &cfg), &frame, generation, a),
        MonitorPage::Cpu => hardware::cpu(cx, m, &frame, a),
        MonitorPage::Memory => hardware::memory(cx, m, &frame, a),
        MonitorPage::Disks => hardware::disks(cx, m, &frame, a),
        MonitorPage::Network => hardware::network(cx, m, &frame, a),
        MonitorPage::Gpu => hardware::gpu(cx, m, &frame, a),
        MonitorPage::Sensors => hardware::sensors(cx, &frame, a),
        MonitorPage::History => history::draw(cx, m.view(pane, &cfg), &frame, a),
        MonitorPage::Connections | MonitorPage::Services | MonitorPage::Startup | MonitorPage::Snapshots => {}
    }
}

/// A short message in the middle of `a`.
pub(super) fn message(cx: &mut Cx<'_>, a: Area, msg: &str, failed: bool) {
    let w = text::width(msg).min(usize::from(a.w)) as u16;
    let y = a.y + i32::from(a.h.saturating_sub(1) / 3);
    let x = a.x + (a.w - w) / 2;
    cx.c.text(x, y, w, msg, if failed { Role::Danger } else { Role::Muted });
    if cx.announce.is_empty() {
        cx.announce = msg.to_string();
    }
}

/// How a strip of tabs fits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StripFit {
    /// Full titles with their numbers.
    Full,
    /// Short titles with their numbers.
    ShortNumbered,
    /// Short titles only.
    Short,
    /// Short titles, one cell apart.
    Tight,
    /// The page's name and arrows.
    Collapsed,
}

impl StripFit {
    /// Cells between two tabs.
    fn gap(self) -> usize {
        if self == StripFit::Tight { 1 } else { 2 }
    }
}

/// The short title of a page.
pub(super) fn short_title(p: MonitorPage) -> &'static str {
    // i18n-dynamic: tui-monitor-tab-
    soyuz_appcore::i18n::t_dyn(&format!("tui-monitor-tab-{}", p.id()))
}

/// The number key of a page: 1-9, then 0.
fn page_key(i: usize) -> String {
    if i < 9 {
        (i + 1).to_string()
    } else if i == 9 {
        "0".into()
    } else {
        String::new()
    }
}

fn tab_label(p: MonitorPage, i: usize, fit: StripFit) -> String {
    match fit {
        StripFit::Full => format!("{} {}", page_key(i), labels::page_title(p)),
        StripFit::ShortNumbered => format!("{} {}", page_key(i), short_title(p)),
        _ => short_title(p).to_string(),
    }
}

fn strip_fit(width: u16, indicator: u16) -> StripFit {
    let room = usize::from(width.saturating_sub(indicator));
    let total = |fit: StripFit| -> usize {
        let parts: usize =
            MonitorPage::ALL.iter().enumerate().map(|(i, p)| text::width(tab_label(*p, i, fit).trim())).sum();
        parts + fit.gap() * (MonitorPage::ALL.len() - 1)
    };
    for fit in [StripFit::Full, StripFit::ShortNumbered, StripFit::Short, StripFit::Tight] {
        if total(fit) <= room {
            return fit;
        }
    }
    StripFit::Collapsed
}

/// The header strip: pages, and at the right edge the pause or interval indicator.
fn strip(cx: &mut Cx<'_>, a: Area, page: MonitorPage, paused: bool) {
    if a.h == 0 || a.w == 0 {
        return;
    }
    let index = MonitorPage::ALL.iter().position(|p| *p == page).unwrap_or(0);
    let total = MonitorPage::ALL.len();
    if cx.look.sr {
        let line = tr!("tui-monitor-page-of", page = labels::page_title(page), index = index + 1, total = total);
        cx.c.text(a.x, a.y, a.w, &line, Role::Title);
        cx.c.set_caret(a.x, a.y);
        return;
    }
    let indicator = if paused {
        tr!("tui-monitor-paused").to_string()
    } else {
        let ms = cx.config.monitor.interval_ms;
        tr!(
            "tui-monitor-interval",
            secs = format!("{:.2}", f64::from(ms) / 1000.0).trim_end_matches('0').trim_end_matches('.').to_string()
        )
    };
    let iw = text::width(&indicator) as u16;
    let show_indicator = a.w >= iw + 12;
    if show_indicator {
        let x = a.x + a.w - iw;
        cx.c.text(x, a.y, iw, &indicator, if paused { Role::Warning } else { Role::Muted });
        cx.c.hit(Area { x, w: iw, ..a }, Hit::Pause);
    }
    let room = if show_indicator { a.w - iw - 1 } else { a.w };
    let fit = strip_fit(room, 0);
    if fit == StripFit::Collapsed {
        let (prev, next) = if cx.look.ascii { ("<", ">") } else { ("‹", "›") };
        cx.c.text(a.x, a.y, 1, prev, Role::KeyHint);
        cx.c.hit(Area { x: a.x, w: 2, ..a }, Hit::Step(false));
        let label = format!("{}/{} {}", index + 1, total, labels::page_title(page));
        let w = room.saturating_sub(4);
        cx.c.bold(a.x + 2, a.y, w, &label, Role::TabActive);
        if room >= 3 {
            let nx = (a.x + 2 + (text::width(&label) as u16).min(w) + 1).min(a.x + room - 1);
            cx.c.text(nx, a.y, 1, next, Role::KeyHint);
            cx.c.hit(Area { x: nx.saturating_sub(1), w: 2, ..a }, Hit::Step(true));
        }
        return;
    }
    let mut x = a.x;
    for (i, p) in MonitorPage::ALL.iter().enumerate() {
        let label = tab_label(*p, i, fit);
        let label = label.trim();
        let w = text::width(label) as u16;
        if *p == page {
            cx.c.bold(x, a.y, w, label, Role::TabActive);
        } else {
            cx.c.text(x, a.y, w, label, Role::TabInactive);
        }
        cx.c.hit(Area { x, w, ..a }, Hit::Tab(*p));
        x = x.saturating_add(w + fit.gap() as u16);
    }
}

/// Lay blocks out in columns of at least `min_w` cells, scrolled by the page's offset:
/// `f(cx, i, area)` draws block `i`.
pub(super) fn flow(
    cx: &mut Cx<'_>,
    a: Area,
    count: usize,
    min_w: u16,
    block_h: u16,
    mut f: impl FnMut(&mut Cx<'_>, usize, Area),
) {
    let cols = (a.w / min_w.max(1)).max(1).min(count.max(1) as u16);
    let rows = count.div_ceil(usize::from(cols));
    let total = rows * (usize::from(block_h) + 1);
    let max = total.saturating_sub(usize::from(a.h));
    cx.nav.page_scroll = cx.nav.page_scroll.min(max);
    cx.nav.page_rows = usize::from(a.h);
    cx.nav.page_max = max;
    let old = cx.c.clip(a);
    let grid = a.columns(cols, 2);
    for i in 0..count {
        let col = grid[i % usize::from(cols)];
        let y = a.y + ((i / usize::from(cols)) * (usize::from(block_h) + 1)) as i32 - cx.nav.page_scroll as i32;
        if y + i32::from(block_h) <= a.y || y >= a.y + i32::from(a.h) {
            continue;
        }
        f(cx, i, Area { x: col.x, y, w: col.w, h: block_h });
    }
    cx.c.unclip(old);
}

/// A chart's value text for percentages.
pub(super) fn percent_axis(v: f32) -> String {
    format!("{v:.0}%")
}

/// Keep `cursor` in a window of `rows` rows from `top`.
pub(super) fn follow(top: usize, cursor: usize, rows: usize, len: usize) -> usize {
    let mut top = top;
    if rows > 0 {
        if cursor < top {
            top = cursor;
        } else if cursor >= top + rows {
            top = cursor + 1 - rows;
        }
    }
    top.min(len.saturating_sub(rows.max(1)))
}

/// A chart of `metric`-like series on the live window, for pages.
pub(super) fn series_of<'a>(label: impl Into<String>, pts: &'a [Pt], role: Role) -> Series<'a> {
    Series { label: label.into(), pts, role }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_strip_collapses_in_steps() {
        assert_eq!(strip_fit(400, 0), StripFit::Full);
        assert_eq!(strip_fit(10, 0), StripFit::Collapsed);
        let narrow = strip_fit(80, 8);
        assert!(matches!(narrow, StripFit::Short | StripFit::Tight), "{narrow:?}");
    }

    #[test]
    fn follow_keeps_the_cursor_in_view() {
        assert_eq!(follow(0, 3, 10, 100), 0);
        assert_eq!(follow(0, 12, 10, 100), 3);
        assert_eq!(follow(5, 2, 10, 100), 2);
        assert_eq!(follow(95, 99, 10, 100), 90);
    }
}
