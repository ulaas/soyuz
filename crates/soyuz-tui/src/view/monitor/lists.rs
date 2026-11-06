//! The Connections, Services and Startup pages: sortable, filtered tables of the core's
//! `ListState`s. A list is sorted again only when it or its order changed.

use soyuz_appcore::models::monitor::lists::{self, endpoint};
use soyuz_appcore::models::monitor::{ListState, MonitorView, Sortable, labels};
use soyuz_appcore::tr;
use soyuz_core::config::monitor::MonitorPage;
use soyuz_sysmon::{Frame, ServiceState};

use super::canvas::{Area, Hit};
use super::{Cx, follow};
use crate::icons;
use crate::input::monitor::{FieldKind, VisKey};
use crate::theme::Role;
use soyuz_appcore::icon::IconId;

/// A column of a list page.
struct Col {
    title: &'static str,
    /// Cells; 0: takes what is left.
    w: u16,
    right: bool,
    /// Left out when the pane is narrower than this.
    min_pane: u16,
    /// The column of the core's sort.
    sort: u8,
}

/// Where the columns go in `width` cells: `(x, w)` per column, width 0 when left out.
fn place(cols: &[Col], width: u16) -> Vec<(u16, u16)> {
    let shown = |c: &Col| width >= c.min_pane;
    let fixed: u16 = cols.iter().filter(|c| shown(c)).map(|c| if c.w > 0 { c.w + 1 } else { 0 }).sum();
    let flex = cols.iter().filter(|c| shown(c) && c.w == 0).count() as u16;
    let each = width.saturating_sub(fixed + flex.saturating_sub(1)) / flex.max(1);
    let mut x = 0u16;
    cols.iter()
        .map(|c| {
            if !shown(c) {
                return (0, 0);
            }
            let w = if c.w > 0 { c.w } else { each };
            let at = x;
            x = x.saturating_add(w + 1);
            (at, w)
        })
        .collect()
}

/// The cells of a row.
type RowFn<'a, T> = dyn Fn(&Cx<'_>, &T) -> Vec<(String, Role)> + 'a;

/// One page: caption, titles, rows and filter line.
struct Page<'a> {
    cols: &'a [Col],
    kind: FieldKind,
    title: &'a str,
    hint: &'a str,
    /// Which of `nav.list_top` this page uses.
    slot: usize,
}

/// Draw a list page; `row` gives the cells of one of its rows.
fn page<T: Sortable + Send + 'static>(
    cx: &mut Cx<'_>,
    l: &mut ListState<T>,
    frame: Option<&Frame>,
    generation: u64,
    mut a: Area,
    p: &Page<'_>,
    row: &RowFn<'_, T>,
) {
    let key = VisKey {
        generation,
        filter: l.filter_text.clone(),
        sort: l.sort,
        descending: l.descending,
        rows: l.rows.len(),
        slot: p.slot,
    };
    if cx.nav.vis_key.as_ref() != Some(&key) {
        cx.nav.vis = l.visible(frame);
        cx.nav.vis_key = Some(key);
    }
    let len = cx.nav.vis.len();
    l.cursor = l.cursor.min(len.saturating_sub(1));
    cx.rows = len;
    let editing = cx.nav.field.as_ref().is_some_and(|f| f.kind == p.kind);
    let filter_row = ((editing || !l.filter_text.is_empty()) && a.h >= 3).then(|| a.take_bottom(1));
    if a.h >= 4 {
        let caption = a.take_top(1);
        let left = tr!("tui-monitor-list-count", title = p.title, count = labels::count(l.rows.len() as u64));
        cx.info(caption, &left, p.hint);
    }
    let places = place(p.cols, a.w);
    if a.h >= 2 {
        let h = a.take_top(1);
        let caret = icons::glyph(if l.descending { IconId::CaretDown } else { IconId::CaretUp }, cx.look.icons);
        for (c, (x, w)) in p.cols.iter().zip(&places) {
            if *w == 0 {
                continue;
            }
            let sorted = l.sort == c.sort;
            let label = if sorted { format!("{} {caret}", c.title) } else { c.title.to_string() };
            let role = if sorted { Role::Accent } else { Role::Muted };
            if c.right {
                cx.c.right(a.x + x, h.y, *w, &label, role);
            } else {
                cx.c.text(a.x + x, h.y, *w, &label, role);
            }
            cx.c.hit(Area { x: a.x + x, w: *w + 1, ..h }, Hit::ListHeader(c.sort));
        }
    }
    let rows_h = usize::from(a.h);
    let top = if std::mem::take(&mut cx.nav.follow) {
        follow(cx.nav.list_top[p.slot], l.cursor, rows_h, len)
    } else {
        cx.nav.list_top[p.slot].min(len.saturating_sub(rows_h.max(1)))
    };
    cx.nav.list_top[p.slot] = top;
    cx.nav.rows = rows_h;
    for i in top..(top + rows_h).min(len) {
        let idx = cx.nav.vis[i];
        let y = a.y + (i - top) as i32;
        let cells = row(cx, &l.rows[idx]);
        for ((c, (x, w)), (txt, role)) in p.cols.iter().zip(&places).zip(&cells) {
            if *w == 0 {
                continue;
            }
            if c.right {
                cx.c.right(a.x + x, y, *w, txt, *role);
            } else {
                cx.c.text(a.x + x, y, *w, txt, *role);
            }
        }
        let area = Area { y, h: 1, ..a };
        if i == l.cursor {
            cx.c.paint(area, if cx.focused { Role::Cursor } else { Role::CursorInactive });
            cx.c.set_caret(a.x, y);
            let parts: Vec<&str> = cells.iter().map(|(t, _)| t.as_str()).filter(|t| !t.is_empty()).collect();
            cx.announce = format!("{}, {}", parts.join(", "), tr!("tui-monitor-row-of", index = i + 1, total = len));
        }
        cx.c.hit(area, Hit::ListRow(i));
    }
    if len == 0 {
        let msg = if l.loading() {
            tr!("tui-monitor-loading")
        } else if l.rows.is_empty() {
            tr!("tui-monitor-list-empty")
        } else {
            tr!("tui-monitor-no-matches")
        };
        super::message(cx, a, msg, false);
    }
    if let Some(r) = filter_row {
        cx.filter_line(r, p.kind, &l.filter_text, Some((len, l.rows.len())), "");
    }
}

pub(super) fn connections(cx: &mut Cx<'_>, v: &mut MonitorView, frame: Option<&Frame>, generation: u64, a: Area) {
    let col = |title, w, right, min_pane, sort| Col { title, w, right, min_pane, sort };
    let cols = [
        col(tr!("tui-monitor-col-proto"), 6, false, 0, 0),
        col(tr!("tui-monitor-col-local"), 0, false, 0, 1),
        col(tr!("tui-monitor-col-remote"), 0, false, 40, 2),
        col(tr!("tui-monitor-col-state"), 11, false, 60, 3),
        col(labels::column_title(soyuz_core::config::monitor::ProcColumn::Pid), 7, true, 50, 4),
        col(tr!("tui-monitor-col-process"), 16, false, 100, 5),
    ];
    let p = Page {
        cols: &cols,
        kind: FieldKind::ListFilter,
        title: labels::page_title(MonitorPage::Connections),
        hint: tr!("tui-monitor-hint-connections"),
        slot: 0,
    };
    let row = |_: &Cx<'_>, c: &soyuz_sysmon::Connection| {
        let (proto, state_role) = (
            match c.proto {
                soyuz_sysmon::Proto::Tcp => "TCP",
                soyuz_sysmon::Proto::Tcp6 => "TCP6",
                soyuz_sysmon::Proto::Udp => "UDP",
                soyuz_sysmon::Proto::Udp6 => "UDP6",
            },
            if c.state.eq_ignore_ascii_case("established") { Role::Success } else { Role::Muted },
        );
        vec![
            (proto.to_string(), Role::Muted),
            (endpoint(&c.local_addr, c.local_port), Role::Text),
            (endpoint(&c.remote_addr, c.remote_port), Role::Text),
            (c.state.clone(), state_role),
            (c.pid.map_or_else(|| "–".to_string(), |x| x.to_string()), Role::Muted),
            (lists::process_name(frame, c.pid), Role::Text),
        ]
    };
    page(cx, &mut v.connections, frame, generation, a, &p, &row);
}

pub(super) fn services(cx: &mut Cx<'_>, v: &mut MonitorView, frame: Option<&Frame>, generation: u64, a: Area) {
    let col = |title, w, right, min_pane, sort| Col { title, w, right, min_pane, sort };
    let cols = [
        col(tr!("tui-monitor-col-service"), 0, false, 0, 0),
        col(tr!("tui-monitor-col-state"), 11, false, 30, 1),
        col(tr!("tui-monitor-col-start"), 12, false, 50, 2),
        col(labels::column_title(soyuz_core::config::monitor::ProcColumn::Pid), 7, true, 64, 3),
        col(tr!("tui-monitor-col-unit"), 0, false, 90, 4),
    ];
    let p = Page {
        cols: &cols,
        kind: FieldKind::ListFilter,
        title: labels::page_title(MonitorPage::Services),
        hint: tr!("tui-monitor-hint-services"),
        slot: 1,
    };
    let row = |_: &Cx<'_>, s: &soyuz_sysmon::ServiceInfo| {
        let state = match s.state {
            ServiceState::Running => Role::Success,
            ServiceState::Failed => Role::Danger,
            ServiceState::Stopped => Role::Muted,
            _ => Role::Warning,
        };
        let name = if s.display_name.is_empty() { s.name.clone() } else { s.display_name.clone() };
        vec![
            (name, Role::Text),
            (labels::service_state(s.state).to_string(), state),
            (s.start_mode.clone(), Role::Muted),
            (s.pid.map_or_else(|| "–".to_string(), |x| x.to_string()), Role::Muted),
            (s.name.clone(), Role::Muted),
        ]
    };
    page(cx, &mut v.services, frame, generation, a, &p, &row);
}

pub(super) fn startup(cx: &mut Cx<'_>, v: &mut MonitorView, frame: Option<&Frame>, generation: u64, a: Area) {
    let col = |title, w, right, min_pane, sort| Col { title, w, right, min_pane, sort };
    let cols = [
        col(tr!("tui-monitor-col-name"), 26, false, 0, 0),
        col(tr!("tui-monitor-col-enabled"), 8, false, 44, 1),
        col(tr!("tui-monitor-col-source"), 22, false, 70, 2),
        col(tr!("tui-monitor-col-command"), 0, false, 36, 3),
    ];
    let p = Page {
        cols: &cols,
        kind: FieldKind::ListFilter,
        title: labels::page_title(MonitorPage::Startup),
        hint: tr!("tui-monitor-hint-lists"),
        slot: 2,
    };
    let row = |_: &Cx<'_>, s: &soyuz_sysmon::StartupItem| {
        let enabled = if s.enabled { tr!("monitor-yes") } else { tr!("monitor-no") };
        vec![
            (s.name.clone(), Role::Text),
            (enabled.to_string(), if s.enabled { Role::Success } else { Role::Muted }),
            (s.source.clone(), Role::Muted),
            (s.command.clone(), Role::Muted),
        ]
    };
    page(cx, &mut v.startup, frame, generation, a, &p, &row);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_panes_leave_columns_out() {
        let col = |w, min_pane| Col { title: "x", w, right: false, min_pane, sort: 0 };
        let cols = [col(6, 0), col(0, 0), col(0, 40), col(12, 60)];
        let p = place(&cols, 30);
        assert_eq!((p[0].1, p[2].1, p[3].1), (6, 0, 0));
        assert_eq!(p[1], (7, 23));
        let wide = place(&cols, 100);
        assert_eq!(wide[3].1, 12);
        assert_eq!(wide[1].1, wide[2].1);
    }
}
