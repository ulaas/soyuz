//! The CPU, Memory, Disks, Network, GPU and Sensors pages: blocks per device that reflow with
//! the width and scroll with the arrows and Page Up / Page Down.

use soyuz_appcore::models::monitor::{MonitorState, labels};
use soyuz_appcore::tr;
use soyuz_core::config::monitor::{CoreView, MonitorPage};
use soyuz_sysmon::{DiskFrame, Frame, Metric, NetFrame, SensorKind};

use super::canvas::Area;
use super::chart::{self, Chart, HUES};
use super::{Cx, flow, percent_axis, series_of};
use crate::theme::Role;
use crate::view::text;

/// A chart of one or two metrics on the live window.
struct Live<'a> {
    metrics: Vec<(Metric, String, Role)>,
    y_max: Option<f32>,
    fmt: &'a dyn Fn(f32) -> String,
    bytes: bool,
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

fn hz(mhz: u32) -> String {
    if mhz >= 1000 { format!("{:.2} GHz", f64::from(mhz) / 1000.0) } else { format!("{mhz} MHz") }
}

pub(super) fn cpu(cx: &mut Cx<'_>, m: &mut MonitorState, frame: &Frame, a: Area) {
    let cores = &frame.cpu.cores;
    let view = cx.config.monitor.core_view;
    let cell_w: u16 = match view {
        CoreView::Heat => 12,
        CoreView::Bars => 34,
        CoreView::Graphs => 28,
    };
    let per_row = (a.w / (cell_w + 2)).max(1) as usize;
    let core_rows = cores.len().div_ceil(per_row);
    let chart_h = cx.chart_h((a.h / 3).clamp(4, 24));
    let total = usize::from(chart_h) + 4 + core_rows;
    cx.announce = format!("{} {}", labels::page_title(MonitorPage::Cpu), labels::percent(frame.cpu.percent));
    scrolled(cx, a, total, |cx, mut r| {
        let info_row = r.take_top(1);
        let mut facts = vec![
            labels::percent(frame.cpu.percent),
            tr!(
                "tui-monitor-user-system",
                user = labels::percent(frame.cpu.user),
                system = labels::percent(frame.cpu.system)
            ),
        ];
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
        cx.info(info_row, &facts.join(" · "), tr!("tui-monitor-hint-cpu"));
        let fmt = percent_axis;
        let spec = Live {
            metrics: vec![
                (Metric::CpuTotal, tr!("tui-monitor-total").to_string(), Role::Accent),
                (Metric::CpuSystem, tr!("tui-monitor-system").to_string(), Role::Info),
            ],
            y_max: Some(100.0),
            fmt: &fmt,
            bytes: false,
        };
        live_chart(cx, m, frame, r.take_top(chart_h), &spec);
        r.take_top(1);
        let title = r.take_top(1);
        cx.c.bold(title.x, title.y, title.w, tr!("tui-monitor-logical-cpus"), Role::GroupHeader);
        let window = cx.window(frame);
        let secs = cx.config.monitor.graph_span();
        for (i, core) in cores.iter().enumerate() {
            let x = r.x + (i % per_row) as u16 * (cell_w + 2);
            let y = r.y + (i / per_row) as i32;
            let cell = Area { x, y, w: cell_w.min(r.w), h: 1 };
            let share = core.percent / 100.0;
            let role = chart::heat_role(share);
            let label = format!("{i:>3}");
            let pct = format!("{:>3.0}%", core.percent);
            cx.c.text(cell.x, y, 3, &label, Role::Muted);
            match view {
                CoreView::Heat => {
                    let glyph = if cx.look.ascii { "#" } else { "█" };
                    let lvl = if cx.look.sr { "" } else { glyph };
                    cx.c.text(cell.x + 4, y, 2, lvl, role);
                    cx.c.right(cell.x + 6, y, cell.w.saturating_sub(6), &pct, role);
                }
                CoreView::Bars => {
                    let bw = cell.w.saturating_sub(10);
                    cx.bar(Area { x: cell.x + 4, y, w: bw, h: 1 }, share, role);
                    cx.c.right(cell.x + 4 + bw, y, 6, &pct, Role::Text);
                }
                CoreView::Graphs => {
                    let bw = cell.w.saturating_sub(10);
                    let pts = m.series(&Metric::Core(i as u16), secs, usize::from(bw));
                    cx.spark(Area { x: cell.x + 4, y, w: bw, h: 1 }, &pts, window, Some(100.0), role);
                    cx.c.right(cell.x + 4 + bw, y, 6, &pct, Role::Text);
                }
            }
        }
    });
}

pub(super) fn memory(cx: &mut Cx<'_>, m: &mut MonitorState, frame: &Frame, a: Area) {
    let mem = &frame.memory;
    let raw = &mem.raw;
    let total = raw.total.max(1) as f32;
    let chart_h = cx.chart_h((a.h.saturating_sub(10) / 2).clamp(4, 40));
    let swap = raw.swap_total > 0;
    let rows = 8 + usize::from(chart_h) * if swap { 2 } else { 1 } + usize::from(swap);
    cx.announce = tr!(
        "tui-monitor-memory-used",
        used = cx.size(mem.used),
        total = cx.size(raw.total),
        percent = labels::percent(mem.percent)
    );
    let announce = cx.announce.clone();
    scrolled(cx, a, rows, |cx, mut r| {
        let line = r.take_top(1);
        cx.c.bold(line.x, line.y, line.w, &announce, Role::Text);
        let cached = raw.cached.unwrap_or(0);
        let buffers = raw.buffers.unwrap_or(0);
        let free = raw.total.saturating_sub(mem.used).saturating_sub(cached).saturating_sub(buffers);
        let ascii = cx.look.ascii;
        let parts: [(String, u64, Role, char); 4] = [
            (tr!("tui-monitor-used").into(), mem.used, HUES[0], if ascii { '#' } else { '█' }),
            (tr!("tui-monitor-cached").into(), cached, HUES[1], if ascii { '=' } else { '▓' }),
            (tr!("tui-monitor-buffers").into(), buffers, HUES[2], if ascii { '+' } else { '▒' }),
            (tr!("tui-monitor-free").into(), free, Role::ProgressTrack, if ascii { '.' } else { '░' }),
        ];
        let bar = r.take_top(1);
        if !cx.look.sr {
            let shares: Vec<(f32, Role, char)> =
                parts.iter().map(|(_, v, ro, g)| (*v as f32 / total, *ro, *g)).collect();
            chart::stacked_bar(cx.c, bar, &shares);
        }
        let mut lg = r.take_top(1);
        for (label, v, role, glyph) in &parts {
            if *v == 0 && label != tr!("tui-monitor-free") {
                continue;
            }
            let g = glyph.to_string();
            let value = format!(" {label} {}   ", cx.size(*v));
            let x = cx.c.spans(lg, &[(&g, *role), (&value, Role::Muted)]);
            lg = Area { w: lg.w.saturating_sub(x - lg.x), x, ..lg };
        }
        r.take_top(1);
        let opt = |o: Option<u64>| o.map(|v| cx.size(v));
        let mut facts: Vec<String> = Vec::new();
        for (key, v) in [
            (tr!("tui-monitor-available"), Some(cx.size(raw.available))),
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
        let row = r.take_top(1);
        cx.c.text(row.x, row.y, row.w, &facts.join(" · "), Role::Muted);
        let row = r.take_top(1);
        if let (Some(i), Some(o)) = (mem.page_in_rate, mem.page_out_rate) {
            cx.c.text(
                row.x,
                row.y,
                row.w,
                &tr!("tui-monitor-paging", page_in = cx.rate(i), page_out = cx.rate(o)),
                Role::Muted,
            );
        }
        r.take_top(1);
        let fmt = percent_axis;
        let spec = Live {
            metrics: vec![(Metric::MemPercent, labels::page_title(MonitorPage::Memory).to_string(), Role::Accent)],
            y_max: Some(100.0),
            fmt: &fmt,
            bytes: false,
        };
        live_chart(cx, m, frame, r.take_top(chart_h), &spec);
        if swap {
            r.take_top(1);
            let spec = Live {
                metrics: vec![(Metric::SwapPercent, tr!("tui-monitor-swap").to_string(), Role::Info)],
                y_max: Some(100.0),
                fmt: &fmt,
                bytes: false,
            };
            live_chart(cx, m, frame, r.take_top(chart_h), &spec);
        }
    });
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
    cx.c.bold(a.x, a.y, a.w, &title, Role::GroupHeader);
    let line = a.row(1);
    let busy_txt = d.busy_percent.map_or_else(|| "–".to_string(), labels::percent);
    let label = format!("{} {busy_txt}", tr!("tui-monitor-busy"));
    let lw = text::width(&label) as u16;
    cx.c.text(line.x, line.y, line.w, &label, Role::Muted);
    if let Some(b) = d.busy_percent
        && line.w > lw + 8
    {
        let share = b / 100.0;
        cx.bar(Area { x: line.x + lw + 1, w: (line.w - lw - 1).min(30), ..line }, share, chart::heat_role(share));
    }
    let spec = Live {
        metrics: vec![
            (Metric::DiskReadOf(d.id.clone()), tr!("tui-monitor-read").to_string(), HUES[0]),
            (Metric::DiskWriteOf(d.id.clone()), tr!("tui-monitor-write").to_string(), HUES[1]),
        ],
        y_max: None,
        fmt: &fmt,
        bytes: true,
    };
    let chart_h = a.h.saturating_sub(3);
    live_chart(cx, m, frame, a.below(2).row(0).h_of(chart_h), &spec);
    let foot = a.row(a.h.saturating_sub(1));
    let queue = d.queue.map_or(String::new(), |q| format!(" · {} {q:.1}", tr!("tui-monitor-queue")));
    let text_ = tr!(
        "tui-monitor-disk-foot",
        read = format!("{:.0}", d.read_iops),
        write = format!("{:.0}", d.write_iops),
        read_total = cx.size(d.read_bytes),
        write_total = cx.size(d.write_bytes)
    );
    cx.c.text(foot.x, foot.y, foot.w, &format!("{text_}{queue}"), Role::Muted);
}

pub(super) fn disks(cx: &mut Cx<'_>, m: &mut MonitorState, frame: &Frame, a: Area) {
    if frame.disks.is_empty() {
        return empty(cx, a, tr!("tui-monitor-no-disks"));
    }
    let list = &frame.disks;
    let h = cx.chart_h((a.h / list.len().clamp(1, 3) as u16).saturating_sub(4).clamp(3, 12)) + 3;
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
    let mut title = format!("{} · {state}", n.name);
    if let Some(l) = n.link_bps {
        title = format!("{title} · {}", labels::bits(l));
    }
    cx.c.bold(a.x, a.y, a.w, &title, if n.up { Role::GroupHeader } else { Role::Muted });
    let mut facts: Vec<String> = n.addresses.iter().take(2).cloned().collect();
    if let Some(mac) = &n.mac {
        facts.push(mac.clone());
    }
    let line = a.row(1);
    cx.c.text(line.x, line.y, line.w, &facts.join(" · "), Role::Muted);
    let spec = Live {
        metrics: vec![
            (Metric::NetRxOf(n.id.clone()), tr!("tui-monitor-download").to_string(), HUES[0]),
            (Metric::NetTxOf(n.id.clone()), tr!("tui-monitor-upload").to_string(), HUES[1]),
        ],
        y_max: None,
        fmt: &fmt,
        bytes: true,
    };
    let chart_h = a.h.saturating_sub(3);
    live_chart(cx, m, frame, a.below(2).row(0).h_of(chart_h), &spec);
    let foot = a.row(a.h.saturating_sub(1));
    let text_ = tr!(
        "tui-monitor-net-foot",
        down = cx.size(n.rx_bytes),
        up = cx.size(n.tx_bytes),
        errors = labels::count(n.rx_errors + n.tx_errors)
    );
    cx.c.text(foot.x, foot.y, foot.w, &text_, Role::Muted);
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
    let h = cx.chart_h((a.h / list.len().clamp(1, 3) as u16).saturating_sub(4).clamp(3, 12)) + 3;
    flow(cx, a, list.len(), 56, h, |cx, i, b| net_block(cx, m, frame, list[i], b));
}

pub(super) fn gpu(cx: &mut Cx<'_>, m: &mut MonitorState, frame: &Frame, a: Area) {
    let list = &frame.gpus;
    if list.is_empty() {
        return empty(cx, a, tr!("tui-monitor-no-gpus"));
    }
    let engines = list.iter().map(|g| g.engines.len().min(4)).max().unwrap_or(0) as u16;
    let chart_h = cx.chart_h((a.h / list.len().clamp(1, 2) as u16).saturating_sub(5 + engines).clamp(3, 12));
    let h = chart_h + 3 + engines;
    let unit = cx.config.monitor.temperature_unit;
    flow(cx, a, list.len(), 56, h, |cx, i, b| {
        let g = &list[i];
        let vendor = if g.vendor.is_empty() { String::new() } else { format!(" · {}", g.vendor) };
        cx.c.bold(b.x, b.y, b.w, &format!("{}{vendor}", g.name), Role::GroupHeader);
        let spec = Live {
            metrics: vec![(Metric::GpuLoad(i as u16), tr!("tui-monitor-load-label").to_string(), HUES[i % HUES.len()])],
            y_max: Some(100.0),
            fmt: &percent_axis,
            bytes: false,
        };
        live_chart(cx, m, frame, b.below(1).row(0).h_of(chart_h), &spec);
        let mut y = 1 + chart_h;
        if let (Some(u), Some(t)) = (g.mem_used, g.mem_total) {
            let row = b.row(y);
            let label = format!("{} {} / {}", tr!("tui-monitor-gpu-memory"), cx.size(u), cx.size(t));
            let lw = text::width(&label) as u16;
            cx.c.text(row.x, row.y, row.w, &label, Role::Muted);
            if row.w > lw + 8 {
                let share = u as f32 / t.max(1) as f32;
                cx.bar(Area { x: row.x + lw + 1, w: (row.w - lw - 1).min(30), ..row }, share, chart::heat_role(share));
            }
        }
        y += 1;
        for e in g.engines.iter().take(4) {
            let row = b.row(y);
            let nw = 16u16.min(row.w);
            cx.c.text(row.x, row.y, nw, &e.name, Role::Muted);
            let value = labels::percent(e.percent);
            if row.w > nw + 8 {
                cx.bar(
                    Area { x: row.x + nw + 1, w: (row.w - nw - 8).min(30), ..row },
                    e.percent / 100.0,
                    chart::heat_role(e.percent / 100.0),
                );
            }
            cx.c.right(row.x + row.w.saturating_sub(6), row.y, 6.min(row.w), &value, Role::Text);
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
        let row = b.row(y);
        cx.c.text(row.x, row.y, row.w, &facts.join(" · "), Role::Muted);
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
            let share = b.percent / 100.0;
            cx.bar(Area { w: bar.w.min(50), ..bar }, share, if share < 0.15 { Role::Danger } else { Role::Progress });
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
            cx.c.right(row.x + lw + 1, row.y, vw, &value, Role::Text);
            let bx = row.x + lw + 1 + vw + 1;
            let limit = s.critical.or(s.max).filter(|l| *l > 0.0);
            let room = row.w.saturating_sub(lw + vw + 2);
            if let Some(l) = limit
                && room >= 8
            {
                let share = s.value / l;
                cx.bar(Area { x: bx, w: room.min(30), ..row }, share, chart::heat_role(share));
            } else if room > kw {
                cx.c.text(bx, row.y, kw, kind_label(s.kind), Role::Muted);
            }
            y += 1;
        }
    });
}
