//! The Snapshots page: the saved states, and a comparison of two of them (or one with now):
//! totals, then a table of the processes with `+ - ~` marks.

use soyuz_appcore::models::monitor::{CompareTarget, MonitorView, SnapshotsState, labels};
use soyuz_appcore::tr;
use soyuz_core::config::monitor::MonitorPage;
use soyuz_sysmon::{Change, Comparison, DiffSort, MatchBy, ProcDiff};

use super::canvas::{Area, Hit};
use super::{Cx, follow};
use crate::icons;
use crate::input::monitor::FieldKind;
use crate::theme::Role;
use soyuz_appcore::icon::IconId;

/// The columns of the comparison, in sort-key order of [`DIFF_SORTS`].
pub const DIFF_SORTS: [DiffSort; 8] = [
    DiffSort::Name,
    DiffSort::Change,
    DiffSort::CpuDelta,
    DiffSort::RssDelta,
    DiffSort::ThreadsDelta,
    DiffSort::HandlesDelta,
    DiffSort::ReadDelta,
    DiffSort::WriteDelta,
];

fn signed_size(cx: &Cx<'_>, a: u64, b: u64) -> String {
    if b >= a { format!("+{}", cx.size(b - a)) } else { format!("-{}", cx.size(a - b)) }
}

fn signed(a: i64, b: i64) -> String {
    format!("{:+}", b - a)
}

fn signed_f(a: f32, b: f32) -> String {
    format!("{:+.1}", b - a)
}

pub(super) fn draw(cx: &mut Cx<'_>, v: &mut MonitorView, a: Area) {
    let s = &v.snapshots;
    if s.comparison.is_some() {
        return comparison(cx, s, a);
    }
    if s.comparing {
        return super::message(cx, a, tr!("tui-monitor-comparing"), false);
    }
    list(cx, s, a);
}

fn list(cx: &mut Cx<'_>, s: &SnapshotsState, mut a: Area) {
    let caption = a.take_top(1);
    let side = |id: Option<&str>| {
        id.map_or_else(|| tr!("tui-monitor-side-none").to_string(), |i| s.label_of(i).unwrap_or(i).to_string())
    };
    let a_label = side(s.a.as_deref());
    let b_label = match &s.b {
        CompareTarget::Live => tr!("tui-monitor-side-now").to_string(),
        CompareTarget::Snapshot(id) => side(Some(id)),
    };
    let left = tr!("tui-monitor-compare-sides", a = a_label, b = b_label);
    cx.info(caption, &left, tr!("tui-monitor-hint-snapshots"));
    if let Some(e) = &s.error
        && a.h >= 3
    {
        let row = a.take_top(1);
        cx.c.text(row.x, row.y, row.w, &crate::dialog::safe(e), Role::Danger);
    }
    let len = s.metas.len();
    cx.rows = len;
    if len == 0 {
        return super::message(cx, a, tr!("tui-monitor-no-snapshots"), false);
    }
    if a.h >= 2 {
        let h = a.take_top(1);
        cx.c.text(h.x + 3, h.y, h.w.saturating_sub(3), tr!("tui-monitor-col-snapshot"), Role::Muted);
        cols(
            cx,
            h,
            &[
                tr!("tui-monitor-col-taken").into(),
                tr!("tui-monitor-processes").into(),
                labels::page_title(MonitorPage::Cpu).into(),
                labels::page_title(MonitorPage::Memory).into(),
            ],
            Role::Muted,
        );
    }
    let rows = usize::from(a.h);
    let top = follow(cx.nav.snap_top, s.cursor, rows, len);
    cx.nav.snap_top = top;
    cx.nav.rows = rows;
    for (i, m) in s.metas.iter().enumerate().skip(top).take(rows) {
        let y = a.y + (i - top) as i32;
        let area = Area { y, h: 1, ..a };
        let mark = match (s.a.as_deref() == Some(m.id.as_str()), s.b == CompareTarget::Snapshot(m.id.clone())) {
            (true, _) => "A ",
            (_, true) => "B ",
            _ => "  ",
        };
        cx.c.text(a.x, y, 2, mark, Role::Accent);
        let label = if m.auto { format!("{} ({})", m.label, tr!("tui-monitor-auto")) } else { m.label.clone() };
        let name_w = a.w.saturating_sub(cols_w(a.w) + 4);
        cx.c.text(a.x + 3, y, name_w, &crate::dialog::safe(&label), Role::Text);
        let cells = [
            labels::time(m.taken_ms, cx.config),
            m.procs.to_string(),
            labels::percent(m.cpu_percent),
            labels::percent(m.mem_percent),
        ];
        cols(cx, area, &cells, Role::Muted);
        if i == s.cursor {
            cx.c.paint(area, if cx.focused { Role::Cursor } else { Role::CursorInactive });
            cx.c.set_caret(a.x, y);
            cx.announce = format!("{label}, {}, {}", cells[0], tr!("tui-monitor-row-of", index = i + 1, total = len));
        }
        cx.c.hit(area, Hit::SnapRow(i));
    }
}

/// Cells the facts of a snapshot row take in a page `w` wide.
fn cols_w(w: u16) -> u16 {
    if w >= 70 {
        44
    } else if w >= 40 {
        20
    } else {
        0
    }
}

/// The right-hand facts of a snapshot row: the date, processes, CPU and memory.
fn cols(cx: &mut Cx<'_>, a: Area, cells: &[String; 4], role: Role) {
    let total = cols_w(a.w);
    if total == 0 {
        return;
    }
    let mut x = a.x + a.w - total;
    if total == 44 {
        cx.c.text(x, a.y, 23, &cells[0], role);
        x += 24;
    } else {
        cx.c.text(x, a.y, 19, &cells[0], role);
        return;
    }
    for c in &cells[1..] {
        cx.c.right(x, a.y, 6, c, role);
        x += 7;
    }
}

fn comparison(cx: &mut Cx<'_>, s: &SnapshotsState, mut a: Area) {
    let Some(c) = s.comparison.clone() else { return };
    let filter_shown = cx.nav.field.as_ref().is_some_and(|f| f.kind == FieldKind::DiffFilter) || !s.filter.is_empty();
    let filter_row = (filter_shown && a.h >= 3).then(|| a.take_bottom(1));
    let by = match s.match_by {
        MatchBy::Process => tr!("tui-monitor-match-process"),
        MatchBy::Program => tr!("tui-monitor-match-program"),
    };
    let same = if s.show_same { tr!("tui-monitor-same-shown") } else { tr!("tui-monitor-same-hidden") };
    let left = tr!(
        "tui-monitor-compare-summary",
        added = c.added,
        removed = c.removed,
        changed = c.changed,
        by = by,
        same = same
    );
    let caption = a.take_top(1);
    cx.info(caption, &left, tr!("tui-monitor-hint-compare"));
    if a.h >= 8 {
        totals(cx, &c, a.take_top(2));
    }
    // mark, name, then the deltas, the first ones kept longest.
    let mut columns: Vec<(u8, u16, &str)> =
        vec![(2, 9, labels::page_title(MonitorPage::Cpu)), (3, 11, labels::page_title(MonitorPage::Memory))];
    for extra in [
        (4u8, 8u16, tr!("tui-monitor-col-threads")),
        (5, 8, tr!("tui-monitor-col-handles")),
        (6, 11, tr!("tui-monitor-read")),
        (7, 11, tr!("tui-monitor-write")),
    ] {
        columns.push(extra);
    }
    let name_min = 16u16;
    let mut used = 2 + name_min;
    columns.retain(|(_, w, _)| {
        if used + w < a.w {
            used += w + 1;
            true
        } else {
            false
        }
    });
    let name_w = a.w.saturating_sub(used) + name_min.min(a.w.saturating_sub(2));
    if a.h >= 2 {
        let h = a.take_top(1);
        let caret = icons::glyph(if s.descending { IconId::CaretDown } else { IconId::CaretUp }, cx.look.icons);
        let title = |sort: u8, text: &str| {
            if DIFF_SORTS[usize::from(sort)] == s.sort { format!("{text} {caret}") } else { text.to_string() }
        };
        cx.c.text(h.x + 2, h.y, name_w, &title(0, tr!("tui-monitor-col-process")), Role::Muted);
        cx.c.hit(Area { x: h.x, w: name_w + 2, ..h }, Hit::DiffHeader(0));
        let mut x = h.x + 2 + name_w;
        for (sort, w, label) in &columns {
            x += 1;
            let role = if DIFF_SORTS[usize::from(*sort)] == s.sort { Role::Accent } else { Role::Muted };
            cx.c.right(x, h.y, *w, &title(*sort, label), role);
            cx.c.hit(Area { x, w: *w, ..h }, Hit::DiffHeader(*sort));
            x += w;
        }
    }
    let len = s.shown.len();
    cx.rows = len;
    let rows = usize::from(a.h);
    let top = follow(cx.nav.diff_top, s.diff_cursor, rows, len);
    cx.nav.diff_top = top;
    cx.nav.rows = rows;
    for (i, &idx) in s.shown.iter().enumerate().skip(top).take(rows) {
        let Some(d) = c.procs.get(idx) else { continue };
        let y = a.y + (i - top) as i32;
        let area = Area { y, h: 1, ..a };
        let (mark, role, word) = change_look(d.change);
        cx.c.text(a.x, y, 2, mark, role);
        let mut name = d.name.to_string();
        if s.match_by == MatchBy::Program && d.count_a.max(d.count_b) > 1 {
            name = format!("{name} ({} -> {})", d.count_a, d.count_b);
        }
        cx.c.text(a.x + 2, y, name_w, &name, role);
        let mut x = a.x + 2 + name_w;
        for (sort, w, _) in &columns {
            x += 1;
            cx.c.right(x, y, *w, &delta(cx, d, *sort), Role::Text);
            x += w;
        }
        if i == s.diff_cursor {
            cx.c.paint(area, if cx.focused { Role::Cursor } else { Role::CursorInactive });
            cx.c.set_caret(a.x, y);
            cx.announce = format!("{word}, {name}, {}", tr!("tui-monitor-row-of", index = i + 1, total = len));
        }
        cx.c.hit(area, Hit::DiffRow(i));
    }
    if len == 0 {
        super::message(cx, a, tr!("tui-monitor-no-matches"), false);
    }
    if let Some(r) = filter_row {
        cx.filter_line(r, FieldKind::DiffFilter, &s.filter, Some((len, c.procs.len())), "");
    }
}

fn change_look(c: Change) -> (&'static str, Role, &'static str) {
    match c {
        Change::Added => ("+ ", Role::Success, tr!("tui-monitor-change-added")),
        Change::Removed => ("- ", Role::Danger, tr!("tui-monitor-change-removed")),
        Change::Changed => ("~ ", Role::Warning, tr!("tui-monitor-change-changed")),
        Change::Same => ("  ", Role::Muted, tr!("tui-monitor-change-same")),
    }
}

/// The change of column `sort` (an index of [`DIFF_SORTS`]).
fn delta(cx: &Cx<'_>, d: &ProcDiff, sort: u8) -> String {
    let (a, b) = (d.a.unwrap_or_default(), d.b.unwrap_or_default());
    match sort {
        2 => signed_f(a.cpu_percent, b.cpu_percent),
        3 => signed_size(cx, a.rss, b.rss),
        4 => signed(a.threads as i64, b.threads as i64),
        5 => match (a.handles, b.handles) {
            (None, None) => "–".into(),
            (x, y) => signed(x.unwrap_or(0) as i64, y.unwrap_or(0) as i64),
        },
        6 => match (a.read_bytes, b.read_bytes) {
            (None, None) => "–".into(),
            (x, y) => signed_size(cx, x.unwrap_or(0), y.unwrap_or(0)),
        },
        7 => match (a.write_bytes, b.write_bytes) {
            (None, None) => "–".into(),
            (x, y) => signed_size(cx, x.unwrap_or(0), y.unwrap_or(0)),
        },
        _ => String::new(),
    }
}

/// Two rows of machine-wide numbers: `CPU 12% -> 30%`.
fn totals(cx: &mut Cx<'_>, c: &Comparison, a: Area) {
    let t = &c.totals;
    let items: Vec<String> = vec![
        format!(
            "{} {} -> {}",
            labels::page_title(MonitorPage::Cpu),
            labels::percent(t.cpu.0),
            labels::percent(t.cpu.1)
        ),
        format!("{} {} -> {}", labels::page_title(MonitorPage::Memory), cx.size(t.mem_used.0), cx.size(t.mem_used.1)),
        format!("{} {} -> {}", tr!("tui-monitor-processes"), t.processes.0, t.processes.1),
        format!("{} {} -> {}", tr!("tui-monitor-col-threads"), t.threads.0, t.threads.1),
        format!("{} {} -> {}", tr!("tui-monitor-read"), cx.rate(t.disk_read.0), cx.rate(t.disk_read.1)),
        format!("{} {} -> {}", tr!("tui-monitor-write"), cx.rate(t.disk_write.0), cx.rate(t.disk_write.1)),
        format!("{} {} -> {}", tr!("tui-monitor-download"), cx.rate(t.net_rx.0), cx.rate(t.net_rx.1)),
        format!("{} {} -> {}", tr!("tui-monitor-upload"), cx.rate(t.net_tx.0), cx.rate(t.net_tx.1)),
    ];
    let cols = (a.w / 36).clamp(1, 4) as usize;
    let boxes = a.columns(cols as u16, 2);
    for (i, s) in items.iter().enumerate().take(cols * usize::from(a.h)) {
        let b = boxes[i % cols];
        let y = a.y + (i / cols) as i32;
        cx.c.text(b.x, y, b.w, s, Role::Muted);
    }
}
