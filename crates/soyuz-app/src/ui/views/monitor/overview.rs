//! The Overview page: hero cards (number, sub-line, live chart) that open their pages, the
//! system identity strip, the top processes by CPU and memory, and the volume bars.

use std::sync::Arc;

use egui::{Align2, Color32, Id, Rect, ScrollArea, Sense, Ui, UiBuilder, pos2, vec2};
use soyuz_core::config::monitor::MonitorPage;
use soyuz_sysmon::{Frame, Metric, RowId};

use super::charts::{Chart, Hue, Series, Unit, hue, meter};
use super::{PageCtx, big_number, card, card_title, grid_cols};
use crate::models::monitor::{MonitorCmd, labels};
use crate::theme::font::semibold;
use crate::theme::{Palette, font, icons::ph};
use crate::ui::widgets::trunc_galley;

const GAP: f32 = 12.0;

struct Hero<'a> {
    page: MonitorPage,
    glyph: &'static str,
    color: Color32,
    title: &'a str,
    value: String,
    sub: String,
    series: Vec<Series<'a>>,
    unit: Unit,
    y_max: Option<f32>,
}

pub fn show(ui: &mut Ui, rect: Rect, c: &mut PageCtx<'_>) {
    let p = c.p.clone();
    let Some(frame) = c.frame.clone() else {
        super::empty(ui, rect, ph::GAUGE, tr!("monitor-no-data"), tr!("monitor-gui-waiting"));
        return;
    };
    let style = c.style();
    let fmt = c.fmt();
    let span = c.span();
    let temp_unit = c.cfg().temperature_unit;
    // Series first (they borrow the monitor state), then the cards borrow them.
    let cpu = c.series(&Metric::CpuTotal);
    let mem = c.series(&Metric::MemPercent);
    let dr = c.series(&Metric::DiskRead);
    let dw = c.series(&Metric::DiskWrite);
    let rx = c.series(&Metric::NetRx);
    let tx = c.series(&Metric::NetTx);
    let gpu = (!frame.gpus.is_empty()).then(|| c.series(&Metric::GpuLoad(0)));
    let temp = frame.max_temp_c().map(|_| c.series(&Metric::MaxTemp));
    let (read, write) = frame.disk_totals();
    let (rxr, txr) = frame.net_totals();
    let ncpu = frame.ncpu();
    let mut heroes: Vec<Hero<'_>> = vec![
        Hero {
            page: MonitorPage::Cpu,
            glyph: ph::CPU,
            color: hue(&p, Hue::Cpu),
            title: labels::page_title(MonitorPage::Cpu),
            value: labels::percent(frame.cpu.percent),
            sub: match (frame.cpu.mhz_avg, frame.load) {
                (Some(mhz), _) => tr!(
                    "monitor-gui-cpu-sub",
                    cores = ncpu,
                    clock = super::charts::fmt_value(Unit::Mhz, mhz as f32, fmt).as_str()
                ),
                (None, Some(l)) => {
                    tr!("monitor-gui-cpu-sub-load", cores = ncpu, load = format!("{:.2}", l[0]).as_str())
                }
                (None, None) => tr!("monitor-gui-cpu-sub-plain", cores = ncpu),
            },
            series: vec![Series::new(labels::page_title(MonitorPage::Cpu), hue(&p, Hue::Cpu), &cpu)],
            unit: Unit::Percent,
            y_max: Some(100.0),
        },
        Hero {
            page: MonitorPage::Memory,
            glyph: ph::MEMORY,
            color: hue(&p, Hue::Memory),
            title: labels::page_title(MonitorPage::Memory),
            value: soyuz_core::format_size(frame.memory.used, style),
            sub: tr!(
                "monitor-gui-mem-sub",
                percent = labels::percent(frame.memory.percent).as_str(),
                total = soyuz_core::format_size(frame.memory.raw.total, style).as_str()
            ),
            series: vec![Series::new(labels::page_title(MonitorPage::Memory), hue(&p, Hue::Memory), &mem)],
            unit: Unit::Percent,
            y_max: Some(100.0),
        },
        Hero {
            page: MonitorPage::Disks,
            glyph: ph::HARD_DRIVES,
            color: hue(&p, Hue::DiskRead),
            title: labels::page_title(MonitorPage::Disks),
            value: labels::rate(read + write, style),
            sub: tr!(
                "monitor-gui-disk-sub",
                read = labels::rate(read, style).as_str(),
                write = labels::rate(write, style).as_str()
            ),
            series: vec![
                Series::new(tr!("monitor-gui-read"), hue(&p, Hue::DiskRead), &dr),
                Series::new(tr!("monitor-gui-write"), hue(&p, Hue::DiskWrite), &dw).line(),
            ],
            unit: Unit::Rate,
            y_max: None,
        },
        Hero {
            page: MonitorPage::Network,
            glyph: ph::NETWORK,
            color: hue(&p, Hue::NetRx),
            title: labels::page_title(MonitorPage::Network),
            value: labels::rate(rxr + txr, style),
            sub: tr!(
                "monitor-gui-net-sub",
                rx = labels::rate(rxr, style).as_str(),
                tx = labels::rate(txr, style).as_str()
            ),
            series: vec![
                Series::new(tr!("monitor-gui-received"), hue(&p, Hue::NetRx), &rx),
                Series::new(tr!("monitor-gui-sent"), hue(&p, Hue::NetTx), &tx).line(),
            ],
            unit: Unit::Rate,
            y_max: None,
        },
    ];
    if let (Some(g), Some(pts)) = (frame.gpus.first(), gpu.as_deref()) {
        heroes.push(Hero {
            page: MonitorPage::Gpu,
            glyph: ph::GRAPHICS_CARD,
            color: hue(&p, Hue::Gpu),
            title: labels::page_title(MonitorPage::Gpu),
            value: g.percent.map_or(PageCtx::dash().to_string(), labels::percent),
            sub: match (g.mem_used, g.mem_total) {
                (Some(u), Some(t)) => tr!(
                    "monitor-gui-gpu-sub",
                    used = soyuz_core::format_size(u, style).as_str(),
                    total = soyuz_core::format_size(t, style).as_str()
                ),
                _ => g.name.clone(),
            },
            series: vec![Series::new(labels::page_title(MonitorPage::Gpu), hue(&p, Hue::Gpu), pts)],
            unit: Unit::Percent,
            y_max: Some(100.0),
        });
    }
    if let Some(b) = frame.batteries.first() {
        let left = b.seconds_left.map(labels::uptime);
        let state = if b.charging {
            tr!("monitor-gui-battery-charging").to_string()
        } else if b.on_ac {
            tr!("monitor-gui-battery-ac").to_string()
        } else {
            tr!("monitor-gui-battery-discharging").to_string()
        };
        heroes.push(Hero {
            page: MonitorPage::Sensors,
            glyph: if b.charging { ph::BATTERY_CHARGING } else { ph::BATTERY_HIGH },
            color: hue(&p, Hue::Power),
            title: tr!("monitor-gui-battery"),
            value: labels::percent(b.percent),
            sub: match left {
                Some(l) => format!("{state} · {l}"),
                None => state,
            },
            series: Vec::new(),
            unit: Unit::Percent,
            y_max: Some(100.0),
        });
    } else if let (Some(t), Some(pts)) = (frame.max_temp_c(), temp.as_deref()) {
        heroes.push(Hero {
            page: MonitorPage::Sensors,
            glyph: ph::THERMOMETER,
            color: hue(&p, Hue::Temp),
            title: tr!("monitor-gui-hottest"),
            value: labels::temperature(t, temp_unit),
            sub: frame
                .sensors
                .iter()
                .filter(|s| s.kind == soyuz_sysmon::SensorKind::Temperature)
                .max_by(|a, b| a.value.total_cmp(&b.value))
                .map(|s| if s.label.is_empty() { s.group.clone() } else { format!("{} · {}", s.group, s.label) })
                .unwrap_or_default(),
            series: vec![Series::new(tr!("monitor-gui-hottest"), hue(&p, Hue::Temp), pts)],
            unit: Unit::Temp(temp_unit),
            y_max: None,
        });
    }

    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    ScrollArea::vertical().id_salt(("soyuz.monitor.overview", c.pane.0)).auto_shrink([false, false]).show(
        &mut child,
        |ui| {
            ui.add_space(4.0);
            let w = ui.available_width();
            // ---- hero cards ----
            let (cols, cw) = grid_cols(w, font::grow(236.0), GAP);
            let ch = font::grow(156.0);
            for (ri, row) in heroes.chunks(cols).enumerate() {
                let (r, _) = ui.allocate_exact_size(vec2(w, ch), Sense::hover());
                for (i, h) in row.iter().enumerate() {
                    let cr = Rect::from_min_size(pos2(r.left() + i as f32 * (cw + GAP), r.top()), vec2(cw, ch));
                    hero_card(ui, cr, c, &p, h, span, fmt, Id::unique(("soyuz.monitor.hero", c.pane.0, ri, i)));
                }
                ui.add_space(GAP);
            }
            // ---- identity ----
            identity(ui, w, c, &p, &frame);
            ui.add_space(GAP);
            // ---- top processes and volumes ----
            let three = w >= font::grow(900.0);
            let two = w >= font::grow(600.0);
            let n = if three {
                3
            } else if two {
                2
            } else {
                1
            };
            let cw = ((w - GAP * (n as f32 - 1.0)) / n as f32).floor();
            let h = font::grow(226.0);
            let (r, _) = ui.allocate_exact_size(
                vec2(
                    w,
                    if n == 1 {
                        h * 3.0 + GAP * 2.0
                    } else if n == 2 {
                        h * 2.0 + GAP
                    } else {
                        h
                    },
                ),
                Sense::hover(),
            );
            let cell = |k: usize| -> Rect {
                match n {
                    3 => Rect::from_min_size(pos2(r.left() + k as f32 * (cw + GAP), r.top()), vec2(cw, h)),
                    2 => match k {
                        0 => Rect::from_min_size(r.min, vec2(cw, h)),
                        1 => Rect::from_min_size(pos2(r.left() + cw + GAP, r.top()), vec2(cw, h)),
                        _ => Rect::from_min_size(pos2(r.left(), r.top() + h + GAP), vec2(w, h)),
                    },
                    _ => Rect::from_min_size(pos2(r.left(), r.top() + k as f32 * (h + GAP)), vec2(w, h)),
                }
            };
            top_card(ui, cell(0), c, &p, &frame, true);
            top_card(ui, cell(1), c, &p, &frame, false);
            volumes_card(ui, cell(2), c, &p);
            ui.add_space(16.0);
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn hero_card(
    ui: &mut Ui,
    r: Rect,
    c: &mut PageCtx<'_>,
    p: &Palette,
    h: &Hero<'_>,
    span: (i64, i64),
    fmt: super::charts::Fmt,
    id: Id,
) {
    let resp = card(ui, p, r, id, true);
    crate::ui::a11y::button(&resp, &format!("{}: {}", h.title, h.value));
    let painter = ui.painter_at(r);
    let y = card_title(&painter, p, r, h.glyph, h.color, h.title);
    big_number(&painter, p, pos2(r.left() + 14.0, y + 2.0), &h.value, &h.sub, r.width() - 28.0);
    let plot =
        Rect::from_min_max(pos2(r.left() + 12.0, y + font::grow(50.0)), pos2(r.right() - 12.0, r.bottom() - 10.0));
    if h.series.is_empty() {
        // A battery card: a meter instead of a chart.
        if let Ok(v) = h.value.trim_end_matches('%').parse::<f32>() {
            let bar = Rect::from_center_size(pos2(plot.center().x, plot.bottom() - 10.0), vec2(plot.width(), 6.0));
            meter(&painter, p, bar, v / 100.0, h.color, &[]);
        }
    } else {
        let mut chart = Chart::mini(span, h.unit, fmt);
        chart.y_max = h.y_max;
        chart.live_dot = true;
        chart.show(ui, plot, id.with("chart"), p, &h.series);
    }
    // The card's own hit area is under the chart's, so the whole card opens the page.
    if resp.clicked() || ui.interact(plot, id.with("plotclick"), Sense::click()).clicked() {
        c.cmds.push(crate::cmd::Cmd::FocusPane(c.pane));
        c.cmd(MonitorCmd::SetPage(h.page));
    }
    let arrow = pos2(r.right() - 16.0, r.top() + 22.0);
    painter.text(
        arrow,
        Align2::CENTER_CENTER,
        ph::ARROW_RIGHT,
        crate::theme::icons::font(12.0),
        if resp.hovered() { p.text_secondary } else { p.text_tertiary.gamma_multiply(0.6) },
    );
}

fn identity(ui: &mut Ui, w: f32, c: &mut PageCtx<'_>, p: &Palette, frame: &Frame) {
    let info = c.info.clone();
    let style = c.style();
    let dash = PageCtx::dash().to_string();
    let mut facts: Vec<(&str, String)> = vec![
        (tr!("monitor-gui-host"), if info.hostname.is_empty() { dash.clone() } else { info.hostname.clone() }),
        (tr!("monitor-gui-os"), if info.os.is_empty() { dash.clone() } else { info.os.clone() }),
        (tr!("monitor-gui-kernel-version"), if info.kernel.is_empty() { dash.clone() } else { info.kernel.clone() }),
        (tr!("monitor-gui-processor"), if info.cpu_model.is_empty() { dash.clone() } else { info.cpu_model.clone() }),
        (
            tr!("monitor-gui-cores"),
            if info.logical_cores > 0 {
                tr!("monitor-gui-cores-value", physical = info.physical_cores, logical = info.logical_cores)
            } else {
                dash.clone()
            },
        ),
        (
            tr!("monitor-gui-ram"),
            match &info.memory_kind {
                Some(k) => format!("{} · {k}", soyuz_core::format_size(frame.memory.raw.total, style)),
                None => soyuz_core::format_size(frame.memory.raw.total, style),
            },
        ),
        (tr!("monitor-gui-uptime"), labels::uptime(frame.uptime_secs)),
    ];
    if let Some(l) = frame.load {
        facts.push((tr!("monitor-gui-load"), format!("{:.2}  {:.2}  {:.2}", l[0], l[1], l[2])));
    }
    if let Some(v) = &info.virtualization {
        facts.push((tr!("monitor-gui-virtualization"), v.clone()));
    }
    let (cols, cw) = grid_cols(w - 28.0, font::grow(200.0), 16.0);
    let rows = facts.len().div_ceil(cols);
    let h = 14.0 + rows as f32 * font::grow(34.0) + 4.0;
    let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    card(ui, p, r, Id::unique(("soyuz.monitor.identity", c.pane.0)), false);
    let painter = ui.painter_at(r);
    for (i, (label, value)) in facts.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        let x = r.left() + 14.0 + col as f32 * (cw + 16.0);
        let y = r.top() + 12.0 + row as f32 * font::grow(34.0);
        painter.text(
            pos2(x, y),
            Align2::LEFT_TOP,
            crate::ui::widgets::caption_upper(label),
            font::ui(9.5),
            p.text_tertiary,
        );
        let g = trunc_galley(&painter, value, font::ui(12.5), p.text_primary, cw);
        painter.galley(pos2(x, y + 13.0), g, p.text_primary);
    }
}

fn top_card(ui: &mut Ui, r: Rect, c: &mut PageCtx<'_>, p: &Palette, frame: &Arc<Frame>, by_cpu: bool) {
    let style = c.style();
    let cfg = c.cfg().clone();
    card(ui, p, r, Id::unique(("soyuz.monitor.top", c.pane.0, by_cpu)), false);
    let painter = ui.painter_at(r);
    let (glyph, color, title) = if by_cpu {
        (ph::CPU, hue(p, Hue::Cpu), tr!("monitor-gui-top-cpu"))
    } else {
        (ph::MEMORY, hue(p, Hue::Memory), tr!("monitor-gui-top-memory"))
    };
    let mut y = card_title(&painter, p, r, glyph, color, title) + 2.0;
    let ncpu = frame.ncpu().max(1) as f32;
    let mut procs: Vec<&soyuz_sysmon::ProcRow> = frame.procs.iter().filter(|q| !q.raw.idle).collect();
    if by_cpu {
        procs.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent));
    } else {
        procs.sort_by_key(|a| std::cmp::Reverse(a.raw.rss));
    }
    let row_h = font::grow(26.0);
    let n = (((r.bottom() - 10.0 - y) / row_h).floor() as usize).min(8);
    let max_v = procs.first().map_or(1.0, |q| if by_cpu { q.cpu_percent.max(1.0) } else { q.raw.rss.max(1) as f32 });
    for (i, q) in procs.iter().take(n).enumerate() {
        let rr = Rect::from_min_size(pos2(r.left() + 8.0, y), vec2(r.width() - 16.0, row_h));
        let resp = ui.interact(rr, Id::unique(("soyuz.monitor.top.row", c.pane.0, by_cpu, i)), Sense::click());
        if resp.hovered() {
            painter.rect_filled(
                rr,
                crate::theme::tokens::radius::same(crate::theme::tokens::radius::ROW),
                p.bg_surface2,
            );
        }
        let value = if by_cpu {
            let v = match cfg.cpu_scale {
                soyuz_core::config::monitor::CpuScale::Machine => q.cpu_percent / ncpu,
                soyuz_core::config::monitor::CpuScale::PerCore => q.cpu_percent,
            };
            labels::percent(v)
        } else {
            soyuz_core::format_size(q.raw.rss, style)
        };
        let share = if by_cpu { q.cpu_percent / max_v } else { q.raw.rss as f32 / max_v };
        let vg = painter.layout_no_wrap(value.clone(), font::numbers(12.0), p.text_primary);
        let name_w = rr.width() - vg.size().x - 24.0;
        let ng = trunc_galley(&painter, &q.raw.name, font::ui(12.5), p.text_primary, name_w);
        painter.galley(pos2(rr.left() + 8.0, rr.top() + 3.0), ng, p.text_primary);
        painter.galley(pos2(rr.right() - 8.0 - vg.size().x, rr.top() + 3.0), vg, p.text_primary);
        let bar = Rect::from_min_size(
            pos2(rr.left() + 8.0, rr.bottom() - 6.0),
            vec2(((rr.width() - 16.0) * share.clamp(0.0, 1.0)).max(2.0), 3.0),
        );
        painter.rect_filled(
            Rect::from_min_size(bar.min, vec2(rr.width() - 16.0, 3.0)),
            crate::theme::tokens::radius::same(2),
            p.border_subtle,
        );
        painter.rect_filled(bar, crate::theme::tokens::radius::same(2), color);
        crate::ui::a11y::button(&resp, &format!("{} {value}", q.raw.name));
        if resp.on_hover_text(format!("{} · PID {}", q.raw.name, q.raw.key.pid)).clicked() {
            c.cmds.push(crate::cmd::Cmd::FocusPane(c.pane));
            c.cmd(MonitorCmd::SetPage(MonitorPage::Processes));
            c.cmd(MonitorCmd::Select(Some(RowId::Proc(q.raw.key))));
        }
        y += row_h;
    }
    if procs.is_empty() {
        painter.text(r.center(), Align2::CENTER_CENTER, tr!("monitor-no-data"), font::ui(12.0), p.text_tertiary);
    }
}

pub(super) fn volumes_card(ui: &mut Ui, r: Rect, c: &mut PageCtx<'_>, p: &Palette) {
    card(ui, p, r, Id::unique(("soyuz.monitor.volumes", c.pane.0)), false);
    let painter = ui.painter_at(r);
    let mut y = card_title(&painter, p, r, ph::HARD_DRIVE, p.info, tr!("monitor-gui-volumes")) + 2.0;
    let row_h = font::grow(34.0);
    let n = ((r.bottom() - 8.0 - y) / row_h).floor().max(0.0) as usize;
    if c.volumes.is_empty() {
        painter.text(r.center(), Align2::CENTER_CENTER, tr!("views-computer-looking"), font::ui(12.0), p.text_tertiary);
        return;
    }
    for vol in c.volumes.iter().take(n) {
        let x = r.left() + 14.0;
        let w = r.width() - 28.0;
        let label = crate::ui::views::computer::volume_label(vol);
        let g = trunc_galley(&painter, &label, semibold(12.0), p.text_primary, w * 0.6);
        painter.galley(pos2(x, y), g, p.text_primary);
        if vol.total > 0 {
            let used = 1.0 - vol.free as f32 / vol.total as f32;
            let text = tr!(
                "views-computer-free-of",
                free = soyuz_core::format_size(vol.free, soyuz_core::SizeStyle::Decimal),
                total = soyuz_core::format_size(vol.total, soyuz_core::SizeStyle::Decimal)
            );
            let tg = trunc_galley(&painter, &text, font::numbers(11.0), p.text_tertiary, w * 0.4);
            painter.galley(pos2(x + w - tg.size().x, y + 1.0), tg, p.text_tertiary);
            let bar = Rect::from_min_size(pos2(x, y + 19.0), vec2(w, 4.0));
            crate::ui::widgets::usage_bar(ui, p, bar, used);
        }
        y += row_h;
    }
    if c.volumes.len() > n {
        painter.text(
            pos2(r.right() - 14.0, r.bottom() - 10.0),
            Align2::RIGHT_CENTER,
            format!("+{}", c.volumes.len() - n),
            font::numbers(10.5),
            p.text_tertiary,
        );
    }
}
