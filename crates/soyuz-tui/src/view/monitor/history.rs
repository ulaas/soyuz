//! The History page: the recorded charts stacked over a time range, with a moving cursor
//! that shows the values and the top processes at that moment.

use soyuz_appcore::models::monitor::{HistoryState, MonitorView, labels};
use soyuz_appcore::tr;
use soyuz_core::config::Config;
use soyuz_core::config::monitor::MonitorPage;
use soyuz_sysmon::{Frame, Metric, Pt};

use super::canvas::Area;
use super::chart::{Chart, HUES};
use super::{Cx, percent_axis, series_of};
use crate::theme::Role;

/// How a chart's values read.
#[derive(Clone, Copy)]
enum Unit {
    Percent,
    Rate,
    Temp,
    Count,
}

fn unit_text(u: Unit, v: f32, config: &Config) -> String {
    match u {
        Unit::Percent => percent_axis(v),
        Unit::Rate => labels::rate(f64::from(v), config.general.size_style),
        Unit::Temp => labels::temperature(v, config.monitor.temperature_unit),
        Unit::Count => format!("{v:.0}"),
    }
}

/// `5 min`, `1 h`, `3 d`.
pub(super) fn range_label(secs: u32) -> String {
    if secs >= 86_400 {
        tr!("tui-monitor-range-days", count = secs / 86_400)
    } else if secs >= 3600 {
        tr!("tui-monitor-range-hours", count = secs / 3600)
    } else {
        tr!("tui-monitor-range-minutes", count = secs / 60)
    }
}

/// A chart of the page: its metrics with their names, the axis top and the unit.
type ChartDef = (Vec<(Metric, String)>, Option<f32>, Unit);

/// The charts of the page.
fn charts(frame: &Frame) -> Vec<ChartDef> {
    let t = |s: &str| s.to_string();
    let mut out = vec![
        (vec![(Metric::CpuTotal, t(labels::page_title(MonitorPage::Cpu)))], Some(100.0), Unit::Percent),
        (
            vec![
                (Metric::MemPercent, t(labels::page_title(MonitorPage::Memory))),
                (Metric::SwapPercent, t(tr!("tui-monitor-swap"))),
            ],
            Some(100.0),
            Unit::Percent,
        ),
        (
            vec![(Metric::DiskRead, t(tr!("tui-monitor-read"))), (Metric::DiskWrite, t(tr!("tui-monitor-write")))],
            None,
            Unit::Rate,
        ),
        (
            vec![(Metric::NetRx, t(tr!("tui-monitor-download"))), (Metric::NetTx, t(tr!("tui-monitor-upload")))],
            None,
            Unit::Rate,
        ),
    ];
    for (i, g) in frame.gpus.iter().enumerate().take(8) {
        out.push((vec![(Metric::GpuLoad(i as u16), g.name.clone())], Some(100.0), Unit::Percent));
    }
    out.push((vec![(Metric::MaxTemp, t(tr!("tui-monitor-temperature")))], None, Unit::Temp));
    out.push((vec![(Metric::ProcessCount, t(tr!("tui-monitor-processes")))], None, Unit::Count));
    out
}

/// The value of `pts` near `t`, on a chart spanning `span_ms`.
fn at(pts: &[Pt], t: i64, span_ms: i64) -> Option<f32> {
    Pt::near(pts, t, span_ms).map(|p| p.avg)
}

/// The next cursor position for a key, in screen columns of the window: `dir` -1 or 1.
pub fn step_cursor(h: &HistoryState, now_ms: i64, width: u16, dir: i64) -> i64 {
    let (from, to) = h.window(now_ms);
    let step = ((to - from) / i64::from(width.max(1))).max(1);
    match h.cursor_ms {
        Some(t) => (t + dir * step).clamp(from, to),
        None => to,
    }
}

pub(super) fn draw(cx: &mut Cx<'_>, v: &mut MonitorView, frame: &Frame, mut a: Area) {
    let h = &v.history;
    let (from, to) = h.window(frame.at_ms);
    let live = h.end_ms.is_none();
    let caption = a.take_top(1);
    let when = if live { tr!("tui-monitor-history-live").to_string() } else { labels::time(to, cx.config) };
    let left = tr!("tui-monitor-history-range", range = range_label(h.range_secs), when = when);
    cx.info(caption, &left, tr!("tui-monitor-hint-history"));
    cx.announce = format!("{}, {left}", labels::page_title(MonitorPage::History));
    // The cursor's reading sits at the bottom.
    let list = charts(frame);
    let mut tops_area = None;
    if let Some(t) = h.cursor_ms {
        let n = if h.tops.is_some() && a.h >= 16 && !cx.look.sr { 5 } else { 1 };
        tops_area = Some((a.take_bottom(n), t));
    }
    let n = list.len() as u16;
    let chart_h = cx.chart_h((a.h / n.max(1)).clamp(4, 16));
    let total = usize::from(chart_h) * usize::from(n);
    let max = total.saturating_sub(usize::from(a.h));
    cx.nav.page_scroll = cx.nav.page_scroll.min(max);
    cx.nav.page_rows = usize::from(a.h);
    cx.nav.page_max = max;
    let scroll = cx.nav.page_scroll as i32;
    let old = cx.c.clip(a);
    let config = cx.config;
    for (i, (metrics, y_max, unit)) in list.iter().enumerate() {
        let y = a.y + (i as i32) * i32::from(chart_h) - scroll;
        if y + i32::from(chart_h) <= a.y || y >= a.y + i32::from(a.h) {
            continue;
        }
        let pts: Vec<_> = metrics.iter().map(|(m, _)| h.series(m)).collect();
        let series = metrics
            .iter()
            .zip(&pts)
            .enumerate()
            .map(|(k, ((_, label), p))| series_of(label.clone(), p, HUES[(i + k) % HUES.len()]))
            .collect();
        let unit = *unit;
        let fmt = move |x: f32| unit_text(unit, x, config);
        let bytes = matches!(unit, Unit::Rate);
        let chart = Chart { series, from, to, y_max: *y_max, fmt: &fmt, cursor: h.cursor_ms, legend: true, bytes };
        // One blank row between charts.
        let area = Area { x: a.x, y, w: a.w, h: chart_h.saturating_sub(u16::from(chart_h > 2)) };
        cx.chart(area, &chart);
    }
    cx.c.unclip(old);
    if let Some((area, t)) = tops_area {
        let mut parts = vec![labels::time(t, cx.config)];
        for (metrics, _, unit) in &list {
            for (m, label) in metrics {
                if let Some(val) = at(&h.series(m), t, i64::from(h.range_secs) * 1000) {
                    parts.push(format!("{label} {}", unit_text(*unit, val, cx.config)));
                }
            }
        }
        let line = parts.join(" · ");
        cx.c.text(area.x, area.y, area.w, &line, Role::Highlight);
        cx.c.set_caret(area.x, area.y);
        cx.announce = line;
        if area.h >= 2
            && let Some(tops) = &h.tops
        {
            let boxes = Area { y: area.y + 1, h: area.h - 1, ..area }.columns(2, 2);
            cx.c.text(boxes[0].x, boxes[0].y, boxes[0].w, tr!("tui-monitor-top-cpu"), Role::GroupHeader);
            cx.c.text(boxes[1].x, boxes[1].y, boxes[1].w, tr!("tui-monitor-top-memory"), Role::GroupHeader);
            for (i, p) in tops.by_cpu.iter().take(usize::from(area.h - 2)).enumerate() {
                // Recorded with 100 = one CPU, shown like the table's column.
                let cpu = match cx.config.monitor.cpu_scale {
                    soyuz_core::config::monitor::CpuScale::Machine => p.cpu / frame.ncpu().max(1) as f32,
                    soyuz_core::config::monitor::CpuScale::PerCore => p.cpu,
                };
                let line = format!("{} {}", p.name, labels::percent(cpu));
                cx.c.text(boxes[0].x, boxes[0].y + 1 + i as i32, boxes[0].w, &line, Role::Text);
            }
            for (i, p) in tops.by_mem.iter().take(usize::from(area.h - 2)).enumerate() {
                let line = format!("{} {}", p.name, cx.size(p.mem));
                cx.c.text(boxes[1].x, boxes[1].y + 1 + i as i32, boxes[1].w, &line, Role::Text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_read_in_big_units() {
        assert_eq!(range_label(300), "5 min");
        assert_eq!(range_label(3600), "1 h");
        assert_eq!(range_label(3 * 86_400), "3 d");
    }

    #[test]
    fn the_cursor_starts_at_the_right_edge_and_stays_in_the_window() {
        let mut h = HistoryState::default();
        let now = 10_000_000;
        assert_eq!(step_cursor(&h, now, 100, -1), now);
        h.cursor_ms = Some(now);
        assert_eq!(step_cursor(&h, now, 100, 1), now);
        assert_eq!(step_cursor(&h, now, 100, -1), now - 36_000);
    }

    #[test]
    fn values_come_from_the_nearest_point() {
        let pts = [
            Pt { t: 0, avg: 1.0, max: 1.0 },
            Pt { t: 10_000, avg: f32::NAN, max: f32::NAN },
            Pt { t: 20_000, avg: 3.0, max: 3.0 },
        ];
        assert_eq!(at(&pts, 2_000, 60_000), Some(1.0));
        assert_eq!(at(&pts, 11_000, 60_000), Some(3.0));
        assert_eq!(at(&pts, 900_000, 60_000), None);
        assert_eq!(at(&[], 2, 60_000), None);
    }
}
