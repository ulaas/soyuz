//! The CPU, Memory, Disks, Network, GPU and Sensors pages, in btop's boxes: blocks per
//! device that reflow with the width and scroll with the arrows and Page Up / Page Down.

use soyuz_appcore::models::monitor::{MonitorState, labels};
use soyuz_appcore::tr;
use soyuz_core::config::monitor::{CoreView, MonitorPage};
use soyuz_sysmon::{DiskFrame, Frame, Metric, NetFrame, SensorKind};

use super::boxes::{Frame as BoxFrame, hue};
use super::canvas::Area;
use super::chart::Chart;
use super::{Cx, flow, percent_axis, series_of};
use crate::theme::{Ramp, Role};
use crate::view::text;

/// A chart of one or two metrics on the live window.
struct Live<'a> {
    metrics: Vec<(Metric, String, Ramp)>,
    y_max: Option<f32>,
    fmt: &'a dyn Fn(f32) -> String,
    bytes: bool,
    /// The second metric grows down from the middle (in and out).
    mirror: bool,
}

fn live_chart(cx: &mut Cx<'_>, m: &mut MonitorState, frame: &Frame, a: Area, spec: &Live<'_>) {
    let window = cx.window(frame);
    let secs = cx.config.monitor.graph_span();
    let width = usize::from(a.w) * 2;
    let pts: Vec<_> = spec.metrics.iter().map(|(metric, ..)| m.series(metric, secs, width)).collect();
    let series = spec.metrics.iter().zip(&pts).map(|((_, l, r), p)| series_of(l.clone(), p, *r)).collect();
    let chart = Chart {
        series,
        from: window.0,
        to: window.1,
        y_max: spec.y_max,
        fmt: spec.fmt,
        cursor: None,
        legend: true,
        bytes: spec.bytes,
        mirror: spec.mirror,
    };
    cx.chart(a, &chart);
}

/// Scrolled lines from the top of `a`, with the page's scroll state.
fn scrolled(cx: &mut Cx<'_>, a: Area, total: usize, mut f: impl FnMut(&mut Cx<'_>, Area)) {
    let max = total.saturating_sub(usize::from(a.h));
    cx.nav.page_scroll = cx.nav.page_scroll.min(max);
    cx.nav.page_rows = usize::from(a.h);
    cx.nav.page_max = max;
    let old = cx.c.clip(a);
    let moved = Area { y: a.y - cx.nav.page_scroll as i32, h: total.min(usize::from(u16::MAX)) as u16, ..a };
    f(cx, moved);
    cx.c.unclip(old);
}

/// A page without scrolling.
fn fixed(cx: &mut Cx<'_>) {
    cx.nav.page_scroll = 0;
    cx.nav.page_max = 0;
}

fn hz(mhz: u32) -> String {
    if mhz >= 1000 { format!("{:.2} GHz", f64::from(mhz) / 1000.0) } else { format!("{mhz} MHz") }
}

/// `label ■■■■■■■■····· value` in `row`: the meter takes what the label and the value
/// leave (at most `max_bar` cells).
#[allow(clippy::too_many_arguments)]
fn meter_row(cx: &mut Cx<'_>, row: Area, label: &str, label_w: u16, share: f32, ramp: Ramp, value: &str, max_bar: u16) {
    let lw = label_w.min(row.w);
    cx.c.text(row.x, row.y, lw, label, Role::Muted);
    let vw = (text::width(value) as u16 + 1).min(row.w.saturating_sub(lw));
    cx.c.right(row.x + row.w - vw, row.y, vw, value, Role::Text);
    let room = row.w.saturating_sub(lw + vw + 1);
    if room >= 4 {
        cx.bar(Area { x: row.x + lw + 1, w: room.min(max_bar), ..row }, share, ramp);
    }
}

pub(super) fn cpu(cx: &mut Cx<'_>, m: &mut MonitorState, frame: &Frame, mut a: Area) {
    let cores = &frame.cpu.cores;
    let view = cx.config.monitor.core_view;
    let cell_w: u16 = match view {
        CoreView::Heat => 12,
        CoreView::Bars => 34,
        CoreView::Graphs => 28,
    };
    let fr = cx.frame_rows();
    let inner_w = a.w.saturating_sub(if fr == 2 { 2 } else { 0 });
    let per_row = (inner_w / (cell_w + 2)).max(1) as usize;
    let core_rows = cores.len().div_ceil(per_row);
    let chart_h = cx.chart_h((a.h / 3).clamp(4, 24));
    let percent = labels::percent(frame.cpu.percent);
    cx.announce = format!("{} {percent}", labels::page_title(MonitorPage::Cpu));
    // The graph's box: the facts, then the graph.
    let top = a.take_top((chart_h + 1 + fr).min(a.h));
    let title = labels::page_title(MonitorPage::Cpu);
    let mut inner = cx.boxed(top, BoxFrame { title, info: &percent, ..BoxFrame::default() }, hue::CPU);
    let mut facts = vec![tr!(
        "tui-monitor-user-system",
        user = labels::percent(frame.cpu.user),
        system = labels::percent(frame.cpu.system)
    )];
    if frame.cpu.iowait > 0.0 {
        facts.push(tr!("tui-monitor-iowait", value = labels::percent(frame.cpu.iowait)));
    }
    if let Some(mhz) = frame.cpu.mhz_avg {
        facts.push(hz(mhz));
    }
    if let Some(t) = frame.cpu.temp_c {
        facts.push(labels::temperature(t, cx.config.monitor.temperature_unit));
    }
    if let Some(c) = frame.cpu.ctx_per_sec {
        facts.push(tr!("tui-monitor-ctx-switches", count = labels::count(c as u64)));
    }
    if inner.h > 0 {
        let row = inner.take_top(1);
        cx.info(row, &format!("{percent} · {}", facts.join(" · ")), tr!("tui-monitor-hint-cpu"));
    }
    let spec = Live {
        metrics: vec![
            (Metric::CpuTotal, tr!("tui-monitor-total").to_string(), Ramp::Heat),
            (Metric::CpuSystem, tr!("tui-monitor-system").to_string(), Ramp::Hue(1)),
        ],
        y_max: Some(100.0),
        fmt: &percent_axis,
        bytes: false,
        mirror: false,
    };
    live_chart(cx, m, frame, inner, &spec);
    // The cores' box, scrolled.
    let count = cores.len().to_string();
    let title = tr!("tui-monitor-logical-cpus");
    let inner = cx.boxed(a, BoxFrame { title, info: &count, ..BoxFrame::default() }, hue::CPU);
    let window = cx.window(frame);
    let secs = cx.config.monitor.graph_span();
    scrolled(cx, inner, core_rows, |cx, r| {
        for (i, core) in cores.iter().enumerate() {
            let x = r.x + (i % per_row) as u16 * (cell_w + 2);
            let y = r.y + (i / per_row) as i32;
            let cell = Area { x, y, w: cell_w.min(r.w), h: 1 };
            let share = core.percent / 100.0;
            let heat = Ramp::Heat.at(share);
            let label = format!("{i:>3}");
            let pct = format!("{:>3.0}%", core.percent);
            cx.c.text(cell.x, y, 3, &label, Role::Muted);
            match view {
                CoreView::Heat => {
                    let glyph = if cx.look.ascii { "#" } else { "■" };
                    let lvl = if cx.look.sr { "" } else { glyph };
                    cx.c.text(cell.x + 4, y, 2, lvl, heat);
                    cx.c.right(cell.x + 6, y, cell.w.saturating_sub(6), &pct, heat);
                }
                CoreView::Bars => {
                    let bw = cell.w.saturating_sub(10);
                    cx.bar(Area { x: cell.x + 4, y, w: bw, h: 1 }, share, Ramp::Heat);
                    cx.c.right(cell.x + 4 + bw, y, 6, &pct, Role::Text);
                }
                CoreView::Graphs => {
                    let bw = cell.w.saturating_sub(10);
                    let pts = m.series(&Metric::Core(i as u16), secs, usize::from(bw) * 2);
                    cx.spark(Area { x: cell.x + 4, y, w: bw, h: 1 }, &pts, window, Some(100.0), Ramp::Heat);
                    cx.c.right(cell.x + 4 + bw, y, 6, &pct, Role::Text);
                }
            }
        }
    });
}

pub(super) fn memory(cx: &mut Cx<'_>, m: &mut MonitorState, frame: &Frame, mut a: Area) {
    let mem = &frame.memory;
    let raw = &mem.raw;
    let total = raw.total.max(1) as f32;
    let swap = raw.swap_total > 0;
    fixed(cx);
    cx.announce = tr!(
        "tui-monitor-memory-used",
        used = cx.size(mem.used),
        total = cx.size(raw.total),
        percent = labels::percent(mem.percent)
    );
    let announce = cx.announce.clone();
    let cached = raw.cached.unwrap_or(0);
    let buffers = raw.buffers.unwrap_or(0);
    let free = raw.total.saturating_sub(mem.used).saturating_sub(cached).saturating_sub(buffers);
    // btop's memory box: one meter per kind, each in its own hue.
    let parts: Vec<(String, u64, Ramp)> = [
        (tr!("tui-monitor-used").to_string(), mem.used, Ramp::Heat),
        (tr!("tui-monitor-available").to_string(), raw.available, Ramp::Hue(7)),
        (tr!("tui-monitor-cached").to_string(), cached, Ramp::Hue(1)),
        (tr!("tui-monitor-buffers").to_string(), buffers, Ramp::Hue(2)),
        (tr!("tui-monitor-free").to_string(), free, Ramp::Hue(6)),
    ]
    .into_iter()
    .filter(|(label, v, _)| *v > 0 || label == tr!("tui-monitor-free"))
    .collect();
    let opt = |o: Option<u64>| o.map(|v| cx.size(v));
    let mut facts: Vec<String> = Vec::new();
    for (key, v) in [
        (tr!("tui-monitor-shared"), opt(raw.shared)),
        (tr!("tui-monitor-kernel"), opt(raw.kernel)),
        (tr!("tui-monitor-compressed"), opt(raw.compressed)),
    ] {
        if let Some(v) = v {
            facts.push(format!("{key} {v}"));
        }
    }
    if let (Some(u), Some(l)) = (raw.commit_used, raw.commit_limit) {
        facts.push(format!("{} {} / {}", tr!("tui-monitor-commit"), cx.size(u), cx.size(l)));
    }
    let paging = match (mem.page_in_rate, mem.page_out_rate) {
        (Some(i), Some(o)) => tr!("tui-monitor-paging", page_in = cx.rate(i), page_out = cx.rate(o)),
        _ => String::new(),
    };
    let fr = cx.frame_rows();
    let rows = 1 + parts.len() as u16 + u16::from(!facts.is_empty()) + u16::from(!paging.is_empty());
    let top = a.take_top((rows + fr).min(a.h));
    let title = labels::page_title(MonitorPage::Memory);
    let percent = labels::percent(mem.percent);
    let total_s = cx.size(raw.total);
    let mut inner = cx.boxed(top, BoxFrame { title, info: &percent, foot: &total_s }, hue::MEMORY);
    let line = inner.take_top(1);
    cx.c.bold(line.x, line.y, line.w, &announce, Role::Text);
    let label_w = parts.iter().map(|(l, ..)| text::width(l) as u16).max().unwrap_or(0) + 1;
    for (label, v, ramp) in &parts {
        if inner.h == 0 {
            break;
        }
        let row = inner.take_top(1);
        let value = format!("{} {}", labels::percent(*v as f32 / total * 100.0), cx.size(*v));
        meter_row(cx, row, label, label_w, *v as f32 / total, *ramp, &value, 60);
    }
    for line in [facts.join(" · "), paging] {
        if !line.is_empty() && inner.h > 0 {
            let row = inner.take_top(1);
            cx.c.text(row.x, row.y, row.w, &line, Role::Muted);
        }
    }
    // The graphs share what is left.
    let n = if swap { 2 } else { 1 };
    let each = a.h / n;
    if each < fr + 1 {
        return;
    }
    let graph = a.take_top(each);
    let inner = cx.boxed(graph, BoxFrame { title, info: &percent, ..BoxFrame::default() }, hue::MEMORY);
    let spec = Live {
        metrics: vec![(Metric::MemPercent, title.to_string(), Ramp::Heat)],
        y_max: Some(100.0),
        fmt: &percent_axis,
        bytes: false,
        mirror: false,
    };
    live_chart(cx, m, frame, inner, &spec);
    if swap {
        let used = format!("{} / {}", cx.size(raw.swap_used), cx.size(raw.swap_total));
        let title = tr!("tui-monitor-swap");
        let inner = cx.boxed(a, BoxFrame { title, info: &used, ..BoxFrame::default() }, hue::SWAP);
        let spec = Live {
            metrics: vec![(Metric::SwapPercent, title.to_string(), Ramp::Hue(5))],
            y_max: Some(100.0),
            fmt: &percent_axis,
            bytes: false,
            mirror: false,
        };
        live_chart(cx, m, frame, inner, &spec);
    }
}

fn empty(cx: &mut Cx<'_>, a: Area, msg: &str) {
    super::message(cx, a, msg, false);
    cx.nav.page_max = 0;
}

fn disk_block(cx: &mut Cx<'_>, m: &mut MonitorState, frame: &Frame, d: &DiskFrame, a: Area) {
    let style = cx.config.general.size_style;
    let fmt = move |v: f32| labels::rate(f64::from(v), style);
    let mut title = d.id.clone();
    if !d.model.is_empty() {
        title = format!("{title} · {}", d.model);
    }
    if let Some(s) = d.size {
        title = format!("{title} · {}", cx.size(s));
    }
    if let Some(rot) = d.rotational {
        title = format!("{title} · {}", if rot { tr!("tui-monitor-disk-hdd") } else { tr!("tui-monitor-disk-ssd") });
    }
    let busy_txt = d.busy_percent.map_or_else(|| "–".to_string(), labels::percent);
    let inner = cx.boxed(a, BoxFrame { title: &title, info: &busy_txt, ..BoxFrame::default() }, hue::DISKS);
    if inner.h == 0 {
        return;
    }
    let line = inner.row(0);
    let label = tr!("tui-monitor-busy");
    match d.busy_percent {
        Some(b) => {
            let lw = text::width(label) as u16 + 1;
            meter_row(cx, line, label, lw, b / 100.0, Ramp::Heat, &busy_txt, 40);
        }
        None => cx.c.text(line.x, line.y, line.w, &format!("{label} {busy_txt}"), Role::Muted),
    }
    let spec = Live {
        metrics: vec![
            (Metric::DiskReadOf(d.id.clone()), tr!("tui-monitor-read").to_string(), Ramp::Down),
            (Metric::DiskWriteOf(d.id.clone()), tr!("tui-monitor-write").to_string(), Ramp::Up),
        ],
        y_max: None,
        fmt: &fmt,
        bytes: true,
        mirror: true,
    };
    let chart_h = inner.h.saturating_sub(2);
    live_chart(cx, m, frame, inner.below(1).h_of(chart_h), &spec);
    let foot = inner.row(inner.h.saturating_sub(1));
    let queue = d.queue.map_or(String::new(), |q| format!(" · {} {q:.1}", tr!("tui-monitor-queue")));
    let text_ = tr!(
        "tui-monitor-disk-foot",
        read = format!("{:.0}", d.read_iops),
        write = format!("{:.0}", d.write_iops),
        read_total = cx.size(d.read_bytes),
        write_total = cx.size(d.write_bytes)
    );
    if inner.h >= 2 {
        cx.c.text(foot.x, foot.y, foot.w, &format!("{text_}{queue}"), Role::Muted);
    }
}

/// The height of a device block with a chart of `chart_h` rows and `extra` rows of text.
fn block_h(cx: &Cx<'_>, chart_h: u16, extra: u16) -> u16 {
    chart_h + extra + cx.frame_rows()
}

pub(super) fn disks(cx: &mut Cx<'_>, m: &mut MonitorState, frame: &Frame, a: Area) {
    if frame.disks.is_empty() {
        return empty(cx, a, tr!("tui-monitor-no-disks"));
    }
    let list = &frame.disks;
    let chart_h = cx.chart_h((a.h / list.len().clamp(1, 3) as u16).saturating_sub(5).clamp(4, 12));
    let h = block_h(cx, chart_h, 2);
    let (read, write) = frame.disk_totals();
    cx.announce = format!(
        "{}, {}",
        labels::page_title(MonitorPage::Disks),
        tr!("tui-monitor-read-write", read = cx.rate(read), write = cx.rate(write))
    );
    flow(cx, a, list.len(), 56, h, |cx, i, b| disk_block(cx, m, frame, &list[i], b));
}

fn net_block(cx: &mut Cx<'_>, m: &mut MonitorState, frame: &Frame, n: &NetFrame, a: Area) {
    let style = cx.config.general.size_style;
    let fmt = move |v: f32| labels::rate(f64::from(v), style);
    let state = if n.up { tr!("tui-monitor-net-up") } else { tr!("tui-monitor-net-down") };
    let mut title = n.name.clone();
    if let Some(l) = n.link_bps {
        title = format!("{title} · {}", labels::bits(l));
    }
    let h = if n.up { hue::NETWORK } else { Ramp::Faint };
    let inner = cx.boxed(a, BoxFrame { title: &title, info: state, ..BoxFrame::default() }, h);
    if inner.h == 0 {
        return;
    }
    let mut facts: Vec<String> = n.addresses.iter().take(2).cloned().collect();
    if let Some(mac) = &n.mac {
        facts.push(mac.clone());
    }
    let line = inner.row(0);
    cx.c.text(line.x, line.y, line.w, &facts.join(" · "), Role::Muted);
    let spec = Live {
        metrics: vec![
            (Metric::NetRxOf(n.id.clone()), tr!("tui-monitor-download").to_string(), Ramp::Down),
            (Metric::NetTxOf(n.id.clone()), tr!("tui-monitor-upload").to_string(), Ramp::Up),
        ],
        y_max: None,
        fmt: &fmt,
        bytes: true,
        mirror: true,
    };
    let chart_h = inner.h.saturating_sub(2);
    live_chart(cx, m, frame, inner.below(1).h_of(chart_h), &spec);
    let foot = inner.row(inner.h.saturating_sub(1));
    let text_ = tr!(
        "tui-monitor-net-foot",
        down = cx.size(n.rx_bytes),
        up = cx.size(n.tx_bytes),
        errors = labels::count(n.rx_errors + n.tx_errors)
    );
    if inner.h >= 2 {
        cx.c.text(foot.x, foot.y, foot.w, &text_, Role::Muted);
    }
}

pub(super) fn network(cx: &mut Cx<'_>, m: &mut MonitorState, frame: &Frame, mut a: Area) {
    let show_virtual = cx.config.monitor.show_virtual_networks;
    let list: Vec<&NetFrame> = frame.nets.iter().filter(|n| show_virtual || !n.virtual_if).collect();
    let (rx, tx) = frame.net_totals();
    let summary = tr!("tui-monitor-down-up", down = cx.rate(rx), up = cx.rate(tx));
    let row = a.take_top(1);
    cx.info(row, &summary, tr!("tui-monitor-hint-network"));
    cx.announce = format!("{}, {summary}", labels::page_title(MonitorPage::Network));
    if list.is_empty() {
        return empty(cx, a, tr!("tui-monitor-no-networks"));
    }
    let chart_h = cx.chart_h((a.h / list.len().clamp(1, 3) as u16).saturating_sub(5).clamp(4, 12));
    let h = block_h(cx, chart_h, 2);
    flow(cx, a, list.len(), 56, h, |cx, i, b| net_block(cx, m, frame, list[i], b));
}

pub(super) fn gpu(cx: &mut Cx<'_>, m: &mut MonitorState, frame: &Frame, a: Area) {
    let list = &frame.gpus;
    if list.is_empty() {
        return empty(cx, a, tr!("tui-monitor-no-gpus"));
    }
    let engines = list.iter().map(|g| g.engines.len().min(4)).max().unwrap_or(0) as u16;
    let chart_h = cx.chart_h((a.h / list.len().clamp(1, 2) as u16).saturating_sub(6 + engines).clamp(3, 12));
    let h = block_h(cx, chart_h, 2 + engines);
    let unit = cx.config.monitor.temperature_unit;
    flow(cx, a, list.len(), 56, h, |cx, i, b| {
        let g = &list[i];
        let vendor = if g.vendor.is_empty() { String::new() } else { format!(" · {}", g.vendor) };
        let title = format!("{}{vendor}", g.name);
        let load = g.percent.map_or_else(|| "–".into(), labels::percent);
        let b = cx.boxed(b, BoxFrame { title: &title, info: &load, ..BoxFrame::default() }, hue::GPU);
        let spec = Live {
            metrics: vec![(Metric::GpuLoad(i as u16), tr!("tui-monitor-load-label").to_string(), Ramp::Heat)],
            y_max: Some(100.0),
            fmt: &percent_axis,
            bytes: false,
            mirror: false,
        };
        live_chart(cx, m, frame, b.h_of(chart_h.min(b.h)), &spec);
        let mut y = chart_h;
        let label_w = 16u16;
        if let (Some(u), Some(t)) = (g.mem_used, g.mem_total) {
            let value = format!("{} / {}", cx.size(u), cx.size(t));
            let share = u as f32 / t.max(1) as f32;
            meter_row(cx, b.row(y), tr!("tui-monitor-gpu-memory"), label_w, share, Ramp::Hue(4), &value, 40);
        }
        y += 1;
        for e in g.engines.iter().take(4) {
            let value = labels::percent(e.percent);
            meter_row(cx, b.row(y), &e.name, label_w, e.percent / 100.0, Ramp::Heat, &value, 40);
            y += 1;
        }
        let mut facts = Vec::new();
        if let Some(t) = g.temperature_c {
            facts.push(labels::temperature(t, unit));
        }
        if let Some(p) = g.power_w {
            facts.push(format!("{p:.0} W"));
        }
        if let Some(f) = g.fan_rpm {
            facts.push(format!("{f} RPM"));
        }
        if let Some(c) = g.clock_mhz {
            facts.push(hz(c));
        }
        if let Some(c) = g.mem_clock_mhz {
            facts.push(format!("{} {}", tr!("tui-monitor-gpu-memory"), hz(c)));
        }
        if y < b.h {
            let row = b.row(y);
            cx.c.text(row.x, row.y, row.w, &facts.join(" · "), Role::Muted);
        }
    });
}

fn kind_label(k: SensorKind) -> &'static str {
    match k {
        SensorKind::Temperature => tr!("tui-monitor-temperature"),
        SensorKind::Fan => tr!("tui-monitor-fan"),
        SensorKind::Power => tr!("tui-monitor-power"),
        SensorKind::Voltage => tr!("tui-monitor-voltage"),
        SensorKind::Current => tr!("tui-monitor-current"),
    }
}

pub(super) fn sensors(cx: &mut Cx<'_>, frame: &Frame, a: Area) {
    if frame.sensors.is_empty() && frame.batteries.is_empty() {
        return empty(cx, a, tr!("tui-monitor-no-sensors"));
    }
    let unit = cx.config.monitor.temperature_unit;
    let mut sorted: Vec<_> = frame.sensors.iter().collect();
    sorted.sort_by(|x, y| {
        x.group.cmp(&y.group).then_with(|| (x.kind as u8).cmp(&(y.kind as u8))).then_with(|| x.label.cmp(&y.label))
    });
    let mut groups = 0;
    let mut last = None;
    for s in &sorted {
        if last != Some(&s.group) {
            groups += 1;
            last = Some(&s.group);
        }
    }
    let total = frame.batteries.len() * 2 + usize::from(!frame.batteries.is_empty()) + sorted.len() + groups * 2;
    let hottest = frame.max_temp_c().map_or(String::new(), |t| labels::temperature(t, unit));
    let title = labels::page_title(MonitorPage::Sensors);
    let a = if cx.look.sr {
        a
    } else {
        cx.boxed(a, BoxFrame { title, info: &hottest, ..BoxFrame::default() }, hue::SENSORS)
    };
    scrolled(cx, a, total, |cx, r| {
        let mut y = 0u16;
        for b in &frame.batteries {
            let state = if b.charging {
                tr!("tui-monitor-battery-charging")
            } else if b.on_ac {
                tr!("tui-monitor-battery-plugged")
            } else {
                tr!("tui-monitor-battery-discharging")
            };
            let mut title = format!("{} · {} · {state}", b.name, labels::percent(b.percent));
            if let Some(s) = b.seconds_left {
                title = format!("{title} · {}", labels::uptime(s));
            }
            if let Some(p) = b.power_w {
                title = format!("{title} · {p:.1} W");
            }
            if let Some(h) = b.health_percent {
                title = format!("{title} · {}", tr!("tui-monitor-battery-health", value = labels::percent(h)));
            }
            cx.c.bold(r.x, r.y + i32::from(y), r.w, &title, Role::GroupHeader);
            let bar = r.row(y + 1);
            cx.bar(Area { w: bar.w.min(50), ..bar }, b.percent / 100.0, Ramp::Charge);
            y += 2;
        }
        if !frame.batteries.is_empty() {
            y += 1;
        }
        let mut last: Option<&str> = None;
        for s in &sorted {
            if last != Some(s.group.as_str()) {
                if last.is_some() {
                    y += 1;
                }
                let title = if s.group.is_empty() { tr!("tui-monitor-sensors-other") } else { s.group.as_str() };
                cx.c.bold(r.x, r.y + i32::from(y), r.w, title, Role::GroupHeader);
                y += 1;
                last = Some(s.group.as_str());
            }
            let row = r.row(y);
            let vw = 10u16.min(row.w);
            let lw = (row.w / 3).clamp(10, 36).min(row.w);
            let kw = 12u16;
            let label = if s.label.is_empty() { s.id.as_str() } else { s.label.as_str() };
            cx.c.text(row.x, row.y, lw, label, Role::Text);
            let value = labels::sensor_value(s.kind, s.value, unit);
            let limit = s.critical.or(s.max).filter(|l| *l > 0.0);
            let ramp = if s.kind == SensorKind::Temperature { Ramp::Temp } else { Ramp::Heat };
            let value_role = match limit {
                Some(l) if !cx.look.sr => ramp.at(s.value / l),
                _ => Role::Text,
            };
            cx.c.right(row.x + lw + 1, row.y, vw, &value, value_role);
            let bx = row.x + lw + 1 + vw + 1;
            let room = row.w.saturating_sub(lw + vw + 2);
            if let Some(l) = limit
                && room >= 8
            {
                cx.bar(Area { x: bx, w: room.min(30), ..row }, s.value / l, ramp);
            } else if room > kw {
                cx.c.text(bx, row.y, kw, kind_label(s.kind), Role::Muted);
            }
            y += 1;
        }
    });
}
