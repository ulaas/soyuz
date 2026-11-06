//! The Overview page: who this machine is, a tile per resource with a small chart, and the
//! top processes by CPU and memory.

use soyuz_appcore::models::monitor::{MonitorState, labels};
use soyuz_appcore::tr;
use soyuz_core::config::monitor::MonitorPage;
use soyuz_sysmon::{Frame, Metric, ProcRow, SystemInfo};

use super::canvas::{Area, Hit};
use super::chart::{self, Chart, HUES};
use super::{Cx, percent_axis, series_of};
use crate::theme::Role;
use crate::view::text;

/// `host · Windows 11 · 10.0 · x86_64`, then the hardware and how long it has been up.
pub(super) fn identity(cx: &Cx<'_>, info: &SystemInfo, frame: &Frame) -> Vec<String> {
    let join = |parts: Vec<String>| parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" · ");
    let first = join(vec![info.hostname.clone(), info.os.clone(), info.kernel.clone(), info.arch.clone()]);
    let cores = if info.physical_cores > 0 && info.physical_cores != info.logical_cores {
        tr!("tui-monitor-cores-threads", cores = info.physical_cores, threads = info.logical_cores)
    } else {
        tr!("tui-monitor-cores", cores = frame.ncpu())
    };
    let load = frame.load.map_or(String::new(), |l| {
        tr!(
            "tui-monitor-load",
            one = format!("{:.2}", l[0]),
            five = format!("{:.2}", l[1]),
            fifteen = format!("{:.2}", l[2])
        )
    });
    let second = join(vec![
        info.cpu_model.clone(),
        cores,
        tr!("tui-monitor-ram", size = cx.size(frame.memory.raw.total)),
        tr!("tui-monitor-uptime", time = labels::uptime(frame.uptime_secs)),
        load,
    ]);
    vec![first, second]
}

/// Rows of each top list at most.
const TOPS_MAX: usize = 12;

/// A resource tile.
struct Tile {
    title: String,
    value: String,
    sub: String,
    metric: Metric,
    second: Option<(Metric, String)>,
    y_max: Option<f32>,
    page: MonitorPage,
    rate: bool,
}

fn tiles(cx: &Cx<'_>, frame: &Frame) -> Vec<Tile> {
    let (read, write) = frame.disk_totals();
    let (rx, tx) = frame.net_totals();
    let mem = &frame.memory;
    let mut out = vec![
        Tile {
            title: labels::page_title(MonitorPage::Cpu).into(),
            value: labels::percent(frame.cpu.percent),
            sub: frame.cpu.temp_c.map_or_else(
                || {
                    tr!(
                        "tui-monitor-user-system",
                        user = labels::percent(frame.cpu.user),
                        system = labels::percent(frame.cpu.system)
                    )
                },
                |t| labels::temperature(t, cx.config.monitor.temperature_unit),
            ),
            metric: Metric::CpuTotal,
            second: None,
            y_max: Some(100.0),
            page: MonitorPage::Cpu,
            rate: false,
        },
        Tile {
            title: labels::page_title(MonitorPage::Memory).into(),
            value: labels::percent(mem.percent),
            sub: format!("{} / {}", cx.size(mem.used), cx.size(mem.raw.total)),
            metric: Metric::MemPercent,
            second: None,
            y_max: Some(100.0),
            page: MonitorPage::Memory,
            rate: false,
        },
        Tile {
            title: labels::page_title(MonitorPage::Disks).into(),
            value: cx.rate(read + write),
            sub: tr!("tui-monitor-read-write", read = cx.rate(read), write = cx.rate(write)),
            metric: Metric::DiskRead,
            second: Some((Metric::DiskWrite, tr!("tui-monitor-write").into())),
            y_max: None,
            page: MonitorPage::Disks,
            rate: true,
        },
        Tile {
            title: labels::page_title(MonitorPage::Network).into(),
            value: cx.rate(rx + tx),
            sub: tr!("tui-monitor-down-up", down = cx.rate(rx), up = cx.rate(tx)),
            metric: Metric::NetRx,
            second: Some((Metric::NetTx, tr!("tui-monitor-upload").into())),
            y_max: None,
            page: MonitorPage::Network,
            rate: true,
        },
    ];
    if let Some(g) = frame.gpus.first() {
        out.push(Tile {
            title: labels::page_title(MonitorPage::Gpu).into(),
            value: g.percent.map_or_else(|| "–".into(), labels::percent),
            sub: match (g.mem_used, g.mem_total) {
                (Some(u), Some(t)) => format!("{} / {}", cx.size(u), cx.size(t)),
                _ => g.name.clone(),
            },
            metric: Metric::GpuLoad(0),
            second: None,
            y_max: Some(100.0),
            page: MonitorPage::Gpu,
            rate: false,
        });
    }
    if mem.raw.swap_total > 0 {
        out.push(Tile {
            title: tr!("tui-monitor-swap").into(),
            value: labels::percent(mem.swap_percent),
            sub: format!("{} / {}", cx.size(mem.raw.swap_used), cx.size(mem.raw.swap_total)),
            metric: Metric::SwapPercent,
            second: None,
            y_max: Some(100.0),
            page: MonitorPage::Memory,
            rate: false,
        });
    }
    if let Some(t) = frame.max_temp_c() {
        out.push(Tile {
            title: tr!("tui-monitor-temperature").into(),
            value: labels::temperature(t, cx.config.monitor.temperature_unit),
            sub: String::new(),
            metric: Metric::MaxTemp,
            second: None,
            y_max: None,
            page: MonitorPage::Sensors,
            rate: false,
        });
    }
    out.push(Tile {
        title: tr!("tui-monitor-processes").into(),
        value: labels::count(u64::from(frame.counts.processes)),
        sub: tr!("tui-monitor-threads", count = labels::count(frame.counts.threads)),
        metric: Metric::ProcessCount,
        second: None,
        y_max: None,
        page: MonitorPage::Processes,
        rate: false,
    });
    out
}

fn draw_tile(cx: &mut Cx<'_>, m: &mut MonitorState, frame: &Frame, t: &Tile, role: Role, a: Area) {
    let window = cx.window(frame);
    let secs = cx.config.monitor.graph_seconds;
    let vw = text::width(&t.value) as u16;
    cx.c.bold(a.x, a.y, a.w.saturating_sub(vw + 1), &t.title, Role::Title);
    cx.c.right(a.x, a.y, a.w, &t.value, Role::Text);
    if a.h >= 2 {
        cx.c.text(a.x, a.y + 1, a.w, &t.sub, Role::Muted);
    }
    cx.c.hit(a, Hit::Tab(t.page));
    let body = a.below(2);
    if body.h == 0 || cx.look.sr {
        return;
    }
    let width = usize::from(body.w) * 2;
    let pts = m.series(&t.metric, secs, width);
    if body.h == 1 {
        cx.spark(body, &pts, window, t.y_max, role);
        return;
    }
    let second = t.second.as_ref().map(|(mt, l)| (m.series(mt, secs, width), l.clone()));
    let style = cx.config.general.size_style;
    let fmt_rate = move |v: f32| labels::rate(f64::from(v), style);
    let fmt_num = |v: f32| format!("{v:.0}");
    let fmt: &dyn Fn(f32) -> String = if t.rate {
        &fmt_rate
    } else if t.y_max.is_some() {
        &percent_axis
    } else {
        &fmt_num
    };
    let two = second.is_some();
    let mut series = vec![series_of(t.title.clone(), &pts, role)];
    if let Some((p, l)) = &second {
        series.push(series_of(l.clone(), p, Role::Info));
    }
    // The tile's own lines say the value; two series need their names.
    let chart =
        Chart { series, from: window.0, to: window.1, y_max: t.y_max, fmt, cursor: None, legend: two, bytes: t.rate };
    chart::draw(cx.c, body, &chart, cx.look.chart);
}

/// The top processes by a key, largest first.
pub(super) fn top_by(frame: &Frame, n: usize, key: impl Fn(&ProcRow) -> f64) -> Vec<&ProcRow> {
    let mut all: Vec<&ProcRow> = frame.procs.iter().filter(|p| !p.raw.idle).collect();
    all.sort_by(|a, b| key(b).total_cmp(&key(a)).then_with(|| a.raw.key.pid.cmp(&b.raw.key.pid)));
    all.truncate(n);
    all
}

fn draw_tops(cx: &mut Cx<'_>, frame: &Frame, a: Area) {
    let cols = if a.w >= 70 && !cx.look.sr { 2 } else { 1 };
    let boxes = a.columns(cols, 3);
    let rows = if cols == 2 { a.h } else { a.h / 2 };
    let n = usize::from(rows.saturating_sub(1)).min(TOPS_MAX);
    let (ncpu, scale) = (frame.ncpu().max(1), cx.config.monitor.cpu_scale);
    let cpu_share = |p: &ProcRow| match scale {
        soyuz_sysmon::CpuScale::Machine => p.cpu_percent / ncpu as f32,
        soyuz_sysmon::CpuScale::PerCore => p.cpu_percent,
    };
    let lists: [(&str, Vec<&ProcRow>); 2] = [
        (tr!("tui-monitor-top-cpu"), top_by(frame, n, |p| f64::from(p.cpu_percent))),
        (tr!("tui-monitor-top-memory"), top_by(frame, n, |p| p.raw.rss as f64)),
    ];
    for (k, (title, list)) in lists.iter().enumerate() {
        let b = if cols == 2 { boxes[k] } else { Area { y: a.y + i32::from(rows) * k as i32, h: rows, ..a } };
        cx.c.bold(b.x, b.y, b.w, title, Role::GroupHeader);
        for (i, p) in list.iter().enumerate() {
            let y = b.y + 1 + i as i32;
            let value = if k == 0 { labels::percent(cpu_share(p)) } else { cx.size(p.raw.rss) };
            let bar_w = if b.w >= 44 { 10u16 } else { 0 };
            let vw = 9u16;
            let nw = b.w.saturating_sub(vw + bar_w + 2);
            cx.c.text(b.x, y, nw, &p.raw.name, Role::Text);
            let share = if k == 0 { cpu_share(p) / 100.0 } else { p.mem_percent / 100.0 };
            if bar_w > 0 {
                let bx = Area { x: b.x + nw + 1, y, w: bar_w, h: 1 };
                cx.bar(bx, share, chart::heat_role(share));
            }
            cx.c.right(b.x + b.w.saturating_sub(vw), y, vw.min(b.w), &value, Role::Text);
        }
    }
}

pub(super) fn draw(cx: &mut Cx<'_>, m: &mut MonitorState, frame: &Frame, mut a: Area) {
    let info = m.info.clone();
    let lines = identity(cx, &info, frame);
    for l in &lines {
        if a.h > 0 {
            let r = a.take_top(1);
            cx.c.text(r.x, r.y, r.w, l, Role::Muted);
        }
    }
    if a.h > 0 {
        a.take_top(1);
    }
    let ts = tiles(cx, frame);
    let cols = (a.w / 24).clamp(1, 4);
    let rows = ts.len().div_ceil(usize::from(cols)) as u16;
    // The top lists keep a useful length (one column of them below 70 cells: two lists
    // stacked); the tiles grow with the rest.
    let wanted = TOPS_MAX as u16 + 1;
    let tops_keep = if a.h >= 16 { (if a.w >= 70 { wanted } else { 2 * wanted }).min(a.h / 2) } else { 0 };
    let per = (a.h.saturating_sub(tops_keep) / rows.max(1)).clamp(1, 14);
    let per = if cx.look.sr { 2 } else { per };
    let grid = a.columns(cols, 2);
    for (i, t) in ts.iter().enumerate() {
        let col = grid[i % usize::from(cols)];
        let y = a.y + (i / usize::from(cols)) as i32 * i32::from(per);
        let area = Area { x: col.x, y, w: col.w, h: per.saturating_sub(u16::from(per > 2)).max(1) };
        if y < a.y + i32::from(a.h) {
            draw_tile(cx, m, frame, t, HUES[i % HUES.len()], area);
        }
    }
    let used = (rows * per).min(a.h);
    let tops = a.below(used);
    if tops.h >= 3 {
        draw_tops(cx, frame, tops);
    }
    cx.announce = format!(
        "{}, {} {}, {} {}",
        labels::page_title(MonitorPage::Overview),
        labels::page_title(MonitorPage::Cpu),
        labels::percent(frame.cpu.percent),
        labels::page_title(MonitorPage::Memory),
        labels::percent(frame.memory.percent),
    );
}
