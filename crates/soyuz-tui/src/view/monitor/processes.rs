//! The Processes page: the table from `[monitor]` columns (order, widths: config width / 8
//! cells, 0 = default, Name flexible), list / tree / by program / by user, the filter line
//! and the details panel. The rows come from the core's `TableCache`, cells only for the
//! rows on screen; selection and scroll follow the row's identity, not its index.

use soyuz_appcore::models::monitor::{DetailsTab, MonitorView, labels};
use soyuz_appcore::tr;
use soyuz_core::config::monitor::{CpuScale, ProcColumn, ProcColumnConfig, ProcView};
use soyuz_sysmon::{
    CellValue, Frame, Hub, ProcKey, ProcState, RowId, RowKind, TableRow, cell, cell_total, position_of,
};

use super::canvas::{Area, Hit};

use super::{Cx, follow};
use crate::icons;
use crate::input::monitor::FieldKind;
use crate::theme::{Ramp, Role};
use crate::view::text;
use soyuz_appcore::icon::IconId;

/// Narrowest Name column.
const NAME_MIN: u16 = 14;
/// Cells of the mark in front of each row.
const PREFIX: u16 = 2;

/// A column on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub col: ProcColumn,
    /// Offset from the row's left edge.
    pub x: u16,
    pub w: u16,
}

/// The terminal width of a column: `width / 8`, the default for 0.
fn cells_of(c: &ProcColumnConfig) -> u16 {
    let points = if c.width > 0.0 { c.width } else { c.id.default_width() };
    ((points / 8.0).round() as u16).clamp(3, 200)
}

/// Which columns stay first when a pane is too narrow for all.
fn importance(c: ProcColumn) -> u8 {
    use ProcColumn as C;
    match c {
        C::Name => 0,
        C::Cpu => 1,
        C::Memory => 2,
        C::Pid => 3,
        C::Gpu | C::DiskRead | C::DiskWrite | C::MemoryPercent => 4,
        C::Path | C::Command | C::Description | C::Cgroup => 9,
        _ => 5,
    }
}

/// Fit `columns` into `width` cells: the important ones first (Name always), Name takes
/// what is left. Returns the placed columns in configured order and how many were left out.
pub fn fit_columns(columns: &[ProcColumnConfig], width: u16) -> (Vec<Placed>, usize) {
    let room = width.saturating_sub(PREFIX);
    // The Name column is always shown, even if a hand-edited config dropped it.
    let named = columns.iter().any(|c| c.id == ProcColumn::Name);
    let mut order: Vec<usize> = (0..columns.len()).filter(|&i| columns[i].id != ProcColumn::Name).collect();
    order.sort_by_key(|&i| (importance(columns[i].id), i));
    let mut used = NAME_MIN.min(room);
    let mut keep = vec![false; columns.len()];
    for i in order {
        let w = cells_of(&columns[i]);
        if used + 1 + w <= room {
            used += 1 + w;
            keep[i] = true;
        }
    }
    let hidden = keep.iter().enumerate().filter(|(i, k)| !**k && columns[*i].id != ProcColumn::Name).count();
    let name_w = NAME_MIN.min(room) + room.saturating_sub(used);
    let mut out = Vec::new();
    let mut x = PREFIX;
    if !named {
        out.push(Placed { col: ProcColumn::Name, x, w: name_w });
        x += name_w + 1;
    }
    for (i, c) in columns.iter().enumerate() {
        if c.id == ProcColumn::Name {
            out.push(Placed { col: ProcColumn::Name, x, w: name_w });
            x += name_w + 1;
        } else if keep[i] {
            let w = cells_of(c);
            out.push(Placed { col: c.id, x, w });
            x += w + 1;
        }
    }
    (out, hidden)
}

/// What the page reads from the table cache for one frame.
struct Slice {
    len: usize,
    position: Option<usize>,
    matches: usize,
    rows: Vec<TableRow>,
    first: usize,
    selected: Option<RowId>,
}

/// The tree guides of `row`, with the fold mark; empty for a flat row.
fn guides(row: &TableRow, ascii: bool, sr: bool) -> (String, Option<usize>) {
    let depth = usize::from(row.depth);
    if sr {
        return ("  ".repeat(depth.min(16)), None);
    }
    let (bar, tee, elbow) = if ascii { ("| ", "|-", "`-") } else { ("│ ", "├─", "└─") };
    let mut s = String::new();
    for d in 1..depth {
        s.push_str(if d < 64 && row.ancestors_last & (1 << d) != 0 { "  " } else { bar });
    }
    if depth > 0 {
        s.push_str(if row.last_sibling { elbow } else { tee });
    }
    let fold_at = row.has_children.then(|| text::width(&s));
    if row.has_children {
        s.push_str(match (row.collapsed, ascii) {
            (true, true) => "+ ",
            (false, true) => "- ",
            (true, false) => "▸ ",
            (false, false) => "▾ ",
        });
    }
    (s, fold_at)
}

/// A value's bar on load columns (when the cell has room for it).
fn inline_bar(col: ProcColumn) -> bool {
    matches!(col, ProcColumn::Cpu | ProcColumn::Gpu | ProcColumn::MemoryPercent)
}

fn value_role(col: ProcColumn, v: &CellValue, heat: Option<f32>, heat_on: bool) -> Role {
    if let CellValue::State(s) = v {
        return match s {
            ProcState::Running => Role::Success,
            ProcState::Zombie => Role::Danger,
            ProcState::Stopped | ProcState::Suspended | ProcState::DiskWait => Role::Warning,
            _ => Role::Muted,
        };
    }
    if heat_on && let Some(h) = heat {
        return if h >= 0.85 {
            Role::Danger
        } else if h >= 0.6 {
            Role::Warning
        } else {
            Role::Text
        };
    }
    if col.numeric() { Role::Text } else { Role::Muted }
}

pub(super) fn draw(cx: &mut Cx<'_>, v: &mut MonitorView, frame: &Frame, generation: u64, mut a: Area) {
    let cfg = &cx.config.monitor;
    let editing = cx.nav.field.as_ref().is_some_and(|f| f.kind == FieldKind::ProcFilter);
    let show_filter = (editing || !v.filter_text.is_empty()) && a.h >= 3;
    let filter_row = if show_filter { Some(a.take_bottom(1)) } else { None };
    let details_h = if cfg.details_panel && a.h >= 14 && !cx.look.sr {
        ((cfg.details_height / 18.0).round() as u16).clamp(4, a.h / 2)
    } else {
        0
    };
    let details = a.take_bottom(details_h);
    let info_row = if a.h >= 6 { Some(a.take_top(1)) } else { None };
    let header = if a.h >= 2 { Some(a.take_top(1)) } else { None };
    let (placed, hidden) = fit_columns(&cfg.columns, a.w);

    // One borrow of the table cache: everything the frame needs from it.
    let selected = v.selected.clone();
    let nav_top = cx.nav.top;
    let rows_h = usize::from(a.h);
    let keep_in_view = cx.nav.follow || (cfg.follow_selected && !cx.nav.free_scroll);
    let cursor_was = cx.nav.cursor;
    let slice = {
        let rows = v.rows(frame, generation, cfg);
        let len = rows.len();
        let mut position = selected.as_ref().and_then(|id| position_of(rows, frame, id));
        let mut new_selected = None;
        if position.is_none() && len > 0 {
            // First frame, or the selected process went away: the row that took its place.
            let i = cursor_was.min(len - 1);
            position = Some(i);
            new_selected = Some(rows[i].id(frame));
        }
        let top = match (position, keep_in_view) {
            (Some(p), true) => follow(nav_top, p, rows_h, len),
            _ => nav_top.min(len.saturating_sub(rows_h.max(1))),
        };
        let matches = rows.iter().filter(|r| r.matched && matches!(r.kind, RowKind::Proc(_))).count();
        let end = (top + rows_h).min(len);
        Slice { len, position, matches, rows: rows[top..end].to_vec(), first: top, selected: new_selected }
    };
    if slice.selected.is_some() {
        v.selected = slice.selected.clone();
    }
    cx.nav.top = slice.first;
    cx.nav.rows = rows_h;
    cx.nav.cursor = slice.position.unwrap_or(0);
    cx.nav.follow = false;
    cx.rows = slice.len;
    cx.matches = slice.matches;
    cx.filter_error = v.filter_error.as_ref().map(|e| e.message.clone());

    if let Some(r) = info_row {
        info_line(cx, v, frame, r, hidden);
    }
    if let Some(h) = header {
        header_row(cx, &placed, h);
    }
    let scale = cfg.cpu_scale;
    let ncpu = frame.ncpu();
    let now = frame.at_ms;
    let heat_on = cfg.heat_cells;
    let total_mem = frame.memory.raw.total;
    for (i, row) in slice.rows.iter().enumerate() {
        let index = slice.first + i;
        let y = a.y + i as i32;
        let area = Area { y, h: 1, ..a };
        let cursor = slice.position == Some(index);
        let proc_row = row.proc(frame);
        let key = proc_row.map(|p| p.raw.key);
        let marked = key.is_some_and(|k| v.marked.contains(&k));
        let mark = if marked { "* " } else { "  " };
        cx.c.text(a.x, y, PREFIX, mark, Role::Selected);
        let dim = !row.matched;
        let added = key.is_some_and(|k| frame.added.contains(&k));
        let group = matches!(row.kind, RowKind::Group { .. });
        for p in &placed {
            let x = a.x + p.x;
            if x >= a.x + a.w {
                break;
            }
            if p.col == ProcColumn::Name {
                let (guide, _) = guides(row, cx.look.ascii, cx.look.sr);
                let gw = text::width(&guide) as u16;
                let name = match &row.kind {
                    RowKind::Group { label, count } => {
                        let label: &str = if label.is_empty() { tr!("tui-monitor-unknown-user") } else { label };
                        format!("{label} ({count})")
                    }
                    RowKind::Proc(_) => proc_row.map_or(String::new(), |p| p.raw.name.to_string()),
                };
                cx.c.text(x, y, gw.min(p.w), &guide, Role::Faint);
                let role = if dim {
                    Role::Faint
                } else if group {
                    Role::GroupHeader
                } else if added {
                    Role::Success
                } else {
                    Role::Text
                };
                let nw = p.w.saturating_sub(gw);
                if group {
                    cx.c.bold(x + gw, y, nw, &name, role);
                } else {
                    cx.c.text(x + gw, y, nw, &name, role);
                }
                continue;
            }
            let value = match (&row.totals, proc_row) {
                (Some(t), Some(pr)) => match cell_total(p.col, t, ncpu, scale) {
                    CellValue::Missing => cell(p.col, pr, now, ncpu, scale),
                    val => val,
                },
                (Some(t), None) => cell_total(p.col, t, ncpu, scale),
                (None, Some(pr)) => cell(p.col, pr, now, ncpu, scale),
                (None, None) => CellValue::Missing,
            };
            if group && matches!(value, CellValue::Missing) {
                continue;
            }
            let txt = labels::cell_text(&value, cx.config);
            let heat = labels::cell_heat(p.col, &value, total_mem);
            let role = if dim { Role::Faint } else { value_role(p.col, &value, heat, heat_on) };
            if p.col.numeric() {
                let tw = text::width(&txt) as u16;
                if heat_on && inline_bar(p.col) && !cx.look.sr && p.w >= tw + 3 {
                    let share = match value {
                        CellValue::Percent(pc) if p.col == ProcColumn::MemoryPercent => pc / 25.0,
                        CellValue::Percent(pc) => pc / 100.0,
                        _ => 0.0,
                    };
                    if share > 0.0 {
                        let bw = p.w - tw - 1;
                        let ramp = if dim { Ramp::Faint } else { Ramp::Heat };
                        cx.bar(Area { x, y, w: bw.min(4), h: 1 }, share, ramp);
                    }
                }
                cx.c.right(x, y, p.w, &txt, role);
            } else {
                cx.c.text(x, y, p.w, &txt, role);
            }
        }
        if cursor {
            cx.c.paint(area, if cx.focused { Role::Cursor } else { Role::CursorInactive });
            cx.c.set_caret(a.x, y);
            cx.announce = announce_row(cx, row, frame, index, slice.len);
        }
        cx.c.hit(area, Hit::ProcRow(index));
        let (_, fold_at) = guides(row, cx.look.ascii, cx.look.sr);
        if let (Some(at), Some(name)) = (fold_at, placed.iter().find(|p| p.col == ProcColumn::Name)) {
            cx.c.hit(Area { x: a.x + name.x + at as u16, y, w: 2, h: 1 }, Hit::ProcFold(index));
        }
    }
    if slice.len == 0 {
        let msg = if frame.procs.is_empty() { tr!("monitor-no-data") } else { tr!("tui-monitor-no-matches") };
        super::message(cx, a, msg, false);
    }
    if details_h > 0 {
        draw_details(cx, v, frame, details);
    }
    if let Some(r) = filter_row {
        let error = v.filter_error.as_ref().map(|e| format!("! {}", crate::dialog::safe(&e.message)));
        let regex = if v.filter_regex { tr!("tui-monitor-filter-regex") } else { "" };
        let extra = match &error {
            Some(e) => e.clone(),
            None => regex.to_string(),
        };
        cx.filter_line(r, FieldKind::ProcFilter, &v.filter_text, Some((slice.matches, frame.procs.len())), &extra);
    }
}

fn info_line(cx: &mut Cx<'_>, v: &MonitorView, frame: &Frame, r: Area, hidden: usize) {
    let cfg = &cx.config.monitor;
    let scale = match cfg.cpu_scale {
        CpuScale::Machine => tr!("tui-monitor-scale-machine"),
        CpuScale::PerCore => tr!("tui-monitor-scale-core"),
    };
    let mut parts = vec![
        labels::view_title(cfg.view).to_string(),
        tr!(
            "tui-monitor-proc-count",
            count = labels::count(u64::from(frame.counts.processes)),
            running = frame.counts.running
        ),
        scale.to_string(),
    ];
    if !v.marked.is_empty() {
        parts.push(tr!("tui-monitor-marked", count = v.marked.len()));
    }
    if hidden > 0 {
        parts.push(tr!("tui-monitor-columns-hidden", count = hidden));
    }
    cx.info(r, &parts.join(" · "), tr!("tui-monitor-hint-processes"));
}

fn header_row(cx: &mut Cx<'_>, placed: &[Placed], h: Area) {
    let cfg = &cx.config.monitor;
    let caret = icons::glyph(if cfg.sort_descending { IconId::CaretDown } else { IconId::CaretUp }, cx.look.icons);
    for p in placed {
        let title = labels::column_title(p.col);
        let sorted = cfg.sort_by == p.col;
        let label = if sorted { format!("{title} {caret}") } else { title.to_string() };
        let x = h.x + p.x;
        let role = if sorted { Role::Accent } else { Role::Muted };
        if p.col.numeric() {
            cx.c.right(x, h.y, p.w, &label, role);
        } else {
            cx.c.text(x, h.y, p.w, &label, role);
        }
        cx.c.hit(Area { x, w: p.w + 1, ..h }, Hit::ProcHeader(p.col));
    }
}

/// `chrome, process 123, CPU 1.2%, memory 40 MiB, row 3 of 200`.
fn announce_row(cx: &Cx<'_>, row: &TableRow, frame: &Frame, index: usize, len: usize) -> String {
    let at = tr!("tui-monitor-row-of", index = index + 1, total = len);
    let ncpu = frame.ncpu();
    let scale = cx.config.monitor.cpu_scale;
    match (&row.kind, row.proc(frame)) {
        (RowKind::Proc(_), Some(p)) => {
            let cpu = labels::cell_text(&cell(ProcColumn::Cpu, p, frame.at_ms, ncpu, scale), cx.config);
            tr!(
                "tui-monitor-announce-proc",
                name = p.raw.name.to_string(),
                pid = p.raw.key.pid,
                cpu = cpu,
                memory = cx.size(p.raw.rss),
                state = labels::state(p.raw.state),
                at = at
            )
        }
        (RowKind::Group { label, count }, _) => {
            tr!("tui-monitor-announce-group", name = label.to_string(), count = *count, at = at)
        }
        _ => at,
    }
}

fn draw_details(cx: &mut Cx<'_>, v: &MonitorView, frame: &Frame, a: Area) {
    let title_row = a.row(0);
    let rule = if cx.look.ascii { "-" } else { "─" };
    cx.c.text(a.x, title_row.y, a.w, &rule.repeat(usize::from(a.w)), Role::Border);
    let mut x = a.x + 1;
    for t in DetailsTab::available() {
        let label = details_title(*t);
        let w = text::width(label) as u16;
        if *t == v.details.tab {
            cx.c.bold(x, title_row.y, w, label, Role::TabActive);
        } else {
            cx.c.text(x, title_row.y, w, label, Role::TabInactive);
        }
        cx.c.hit(Area { x, w, ..title_row }, Hit::Details(*t));
        x = x.saturating_add(w + 2);
    }
    let body = a.below(1);
    cx.c.hit(body, Hit::DetailsBody);
    let Some(row) = v.selected_row(frame) else {
        let msg = match v.selected {
            Some(RowId::Group(_)) => tr!("tui-monitor-details-group"),
            _ => tr!("tui-monitor-details-none"),
        };
        cx.c.text(body.x, body.y, body.w, msg, Role::Muted);
        return;
    };
    let lines = detail_lines(cx, v, frame, row.raw.key);
    let max = lines.len().saturating_sub(usize::from(body.h));
    cx.nav.details_top = cx.nav.details_top.min(max);
    cx.nav.details_max = max;
    let kw = lines.iter().map(|(k, _)| text::width(k) as u16).max().unwrap_or(0).min(20);
    for (i, (k, val)) in lines.iter().skip(cx.nav.details_top).take(usize::from(body.h)).enumerate() {
        let y = body.y + i as i32;
        cx.c.text(body.x, y, kw, k, Role::Muted);
        let x = body.x + if kw > 0 { kw + 1 } else { 0 };
        cx.c.text(x, y, body.w.saturating_sub(x - body.x), val, Role::Text);
    }
    if v.details.tab == DetailsTab::General && body.w > kw + 34 {
        // The process's own short trail, top right.
        let trail = Hub::global().proc_series(row.raw.key);
        if trail.len() >= 2 {
            let pts: Vec<soyuz_sysmon::Pt> =
                trail.iter().map(|s| soyuz_sysmon::Pt { t: s.t_ms, avg: s.cpu_percent, max: s.cpu_percent }).collect();
            let w = 24u16;
            let window = (pts[0].t, pts[pts.len() - 1].t);
            cx.spark(Area { x: body.x + body.w - w, y: body.y, w, h: 1 }, &pts, window, None, Ramp::Heat);
        }
    }
}

fn details_title(t: DetailsTab) -> &'static str {
    match t {
        DetailsTab::General => tr!("tui-monitor-details-general"),
        DetailsTab::Threads => tr!("tui-monitor-details-threads"),
        DetailsTab::Modules => tr!("tui-monitor-details-modules"),
        DetailsTab::Files => tr!("tui-monitor-details-files"),
        DetailsTab::Environment => tr!("tui-monitor-details-environment"),
        DetailsTab::Sockets => tr!("tui-monitor-details-sockets"),
    }
}

/// The panel's lines: `(label, value)`; an empty label is a line of its own.
fn detail_lines(cx: &Cx<'_>, v: &MonitorView, frame: &Frame, key: ProcKey) -> Vec<(String, String)> {
    let Some(row) = frame.proc(key) else { return Vec::new() };
    let cfg = &cx.config.monitor;
    let (ncpu, scale, now) = (frame.ncpu(), cfg.cpu_scale, frame.at_ms);
    let mut out: Vec<(String, String)> = Vec::new();
    match v.details.tab {
        DetailsTab::General => {
            use ProcColumn as C;
            for col in [
                C::Pid,
                C::Parent,
                C::User,
                C::State,
                C::Cpu,
                C::CpuTime,
                C::Memory,
                C::Private,
                C::Threads,
                C::Handles,
                C::Priority,
                C::Started,
                C::Uptime,
                C::Path,
                C::Command,
                C::Arch,
                C::Elevated,
                C::Session,
                C::Cgroup,
                C::Description,
            ] {
                let value = cell(col, row, now, ncpu, scale);
                if !matches!(value, CellValue::Missing) {
                    out.push((labels::column_title(col).to_string(), labels::cell_text(&value, cx.config)));
                }
            }
        }
        DetailsTab::Sockets => {
            for c in &v.connections.rows {
                if c.pid == Some(key.pid) {
                    let endpoint = soyuz_appcore::models::monitor::lists::endpoint;
                    out.push((
                        format!("{:?}", c.proto),
                        format!(
                            "{} -> {} {}",
                            endpoint(&c.local_addr, c.local_port),
                            endpoint(&c.remote_addr, c.remote_port),
                            c.state
                        ),
                    ));
                }
            }
            if out.is_empty() {
                out.push((String::new(), tr!("tui-monitor-details-empty").to_string()));
            }
        }
        tab => match &v.details.data {
            _ if v.details.loading() => out.push((String::new(), tr!("tui-monitor-loading").to_string())),
            None => out.push((String::new(), tr!("tui-monitor-loading").to_string())),
            Some(Err(e)) => out.push((String::new(), labels::control_error(e))),
            Some(Ok(d)) => {
                match tab {
                    DetailsTab::Threads => {
                        for t in &d.threads {
                            let line = format!("{} {} {}", t.name, labels::state(t.state), labels::cpu_time(t.cpu_ms));
                            out.push((t.tid.to_string(), line));
                        }
                    }
                    DetailsTab::Modules => {
                        for m in &d.modules {
                            out.push((cx.size(m.size), crate::dialog::safe(&m.path)));
                        }
                    }
                    DetailsTab::Files => {
                        for f in &d.open_files {
                            out.push((f.fd.map_or(String::new(), |n| n.to_string()), crate::dialog::safe(&f.path)));
                        }
                    }
                    _ => {
                        for (k, val) in &d.environment {
                            out.push((crate::dialog::safe(k), crate::dialog::safe(val)));
                        }
                    }
                }
                if out.is_empty() {
                    out.push((String::new(), tr!("tui-monitor-details-empty").to_string()));
                }
            }
        },
    }
    out
}

/// The next view of the `V` key.
pub fn next_view(v: ProcView) -> ProcView {
    match v {
        ProcView::List => ProcView::Tree,
        ProcView::Tree => ProcView::ByName,
        ProcView::ByName => ProcView::ByUser,
        ProcView::ByUser => ProcView::List,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(ids: &[ProcColumn]) -> Vec<ProcColumnConfig> {
        ids.iter().map(|id| ProcColumnConfig { id: *id, width: 0.0 }).collect()
    }

    #[test]
    fn name_flexes_and_narrow_panes_keep_the_load_columns() {
        use ProcColumn as C;
        let c = cols(&[C::Name, C::User, C::State, C::Cpu, C::Memory, C::Command]);
        let (wide, hidden) = fit_columns(&c, 200);
        assert_eq!(hidden, 0);
        assert_eq!(wide.len(), 6);
        let name = wide.iter().find(|p| p.col == C::Name).unwrap();
        assert!(name.w > 30, "{name:?}");
        let (narrow, hidden) = fit_columns(&c, 40);
        assert!(narrow.iter().any(|p| p.col == C::Cpu) && narrow.iter().any(|p| p.col == C::Memory));
        assert!(!narrow.iter().any(|p| p.col == C::Command));
        assert!(hidden >= 1);
        assert_eq!(narrow[0].col, C::Name, "the configured order stays");
        for w in 0..20 {
            let (p, _) = fit_columns(&c, w);
            assert_eq!(p.len(), 1, "only Name fits in {w}");
        }
    }

    #[test]
    fn widths_come_from_the_config() {
        use ProcColumn as C;
        let c = vec![ProcColumnConfig { id: C::Name, width: 0.0 }, ProcColumnConfig { id: C::Pid, width: 160.0 }];
        let (p, _) = fit_columns(&c, 100);
        assert_eq!(p.iter().find(|x| x.col == C::Pid).unwrap().w, 20);
    }

    #[test]
    fn guides_draw_the_tree() {
        let row = TableRow {
            kind: RowKind::Proc(0),
            depth: 2,
            has_children: true,
            collapsed: true,
            matched: true,
            last_sibling: true,
            ancestors_last: 0b01,
            totals: None,
        };
        assert_eq!(guides(&row, false, false).0, "│ └─▸ ");
        assert_eq!(guides(&row, true, false).0, "| `-+ ");
        assert_eq!(guides(&TableRow { ancestors_last: 0b11, ..row.clone() }, true, false).0, "  `-+ ");
        assert_eq!(guides(&row, false, true).0, "    ", "a reader gets only the depth");
    }
}
