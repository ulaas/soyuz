//! The GPU page: per adapter the load chart, the engines, memory (bar and chart),
//! temperature, power, fan and clocks. Two or more adapters sit side by side, one column
//! each, so the same row compares them; a narrow pane wraps the columns.

use egui::{Align2, Id, Rect, ScrollArea, Sense, Ui, UiBuilder, pos2, vec2};
use soyuz_sysmon::{GpuRaw, Metric, Pt};

use super::charts::{Chart, Fmt, Hue, Series, Unit, hue, meter};
use super::{PageCtx, badge, card, card_title, fact};
use crate::models::monitor::labels;
use crate::theme::{Palette, font, icons::ph};

const GAP: f32 = 12.0;

/// What every card of the page needs besides its adapter.
struct Look<'a> {
    p: &'a Palette,
    fmt: Fmt,
    style: soyuz_core::SizeStyle,
    span: (i64, i64),
    pane: u64,
    temp_unit: soyuz_core::config::monitor::TempUnit,
    dash: &'a str,
}

/// The narrowest column an adapter gets before the columns wrap.
fn min_column() -> f32 {
    font::grow(340.0)
}

/// The engines list sits beside the load chart from this card width on; narrower cards
/// show it as a card of its own under the load card.
fn engines_beside() -> f32 {
    font::grow(760.0)
}

fn engine_row() -> f32 {
    font::grow(22.0)
}

pub fn show(ui: &mut Ui, rect: Rect, c: &mut PageCtx<'_>) {
    let p = c.p.clone();
    let Some(frame) = c.frame.clone() else {
        super::empty(ui, rect, ph::GRAPHICS_CARD, tr!("monitor-no-data"), tr!("monitor-gui-waiting"));
        return;
    };
    if frame.gpus.is_empty() {
        super::empty(ui, rect, ph::GRAPHICS_CARD, tr!("monitor-gui-no-gpu"), tr!("monitor-gui-no-gpu-sub"));
        return;
    }
    let dash = PageCtx::dash().to_string();
    let look = Look {
        p: &p,
        fmt: c.fmt(),
        style: c.style(),
        span: c.span(),
        pane: c.pane.0,
        temp_unit: c.cfg().temperature_unit,
        dash: &dash,
    };
    let series: Vec<_> = (0..frame.gpus.len())
        .map(|i| (c.series(&Metric::GpuLoad(i as u16)), c.series(&Metric::GpuMem(i as u16))))
        .collect();
    let n = frame.gpus.len();
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    ScrollArea::vertical().id_salt(("soyuz.monitor.gpu", look.pane)).auto_shrink([false, false]).show(
        &mut child,
        |ui| {
            ui.add_space(4.0);
            let w = ui.available_width();
            let cols = if n >= 2 { super::grid_cols(w, min_column(), GAP).0.min(n) } else { 1 };
            let cw = ((w - GAP * (cols as f32 - 1.0)) / cols as f32).floor();
            let beside = cw >= engines_beside();
            for (row, chunk) in frame.gpus.chunks(cols).enumerate() {
                let first = row * cols;
                let x = |r: Rect, k: usize| r.left() + k as f32 * (cw + GAP);

                // Load, with the engines on the right when the card is wide.
                let (r, _) = ui.allocate_exact_size(vec2(w, font::grow(250.0)), Sense::hover());
                for (k, g) in chunk.iter().enumerate() {
                    let cr = Rect::from_min_size(pos2(x(r, k), r.top()), vec2(cw, r.height()));
                    load_card(ui, cr, first + k, g, &series[first + k].0, beside, &look);
                }
                ui.add_space(GAP);

                // The engines under the load card in narrow columns, the rows aligned across
                // the adapters by the longest list.
                let per_line = if cw >= font::grow(440.0) { 2 } else { 1 };
                let most = chunk.iter().map(|g| g.engines.len()).max().unwrap_or(0);
                if !beside && most > 0 {
                    let lines = most.div_ceil(per_line).min(8);
                    let h = font::grow(46.0) + lines as f32 * engine_row() + 6.0;
                    let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                    for (k, g) in chunk.iter().enumerate() {
                        let cr = Rect::from_min_size(pos2(x(r, k), r.top()), vec2(cw, h));
                        engines_card(ui, cr, first + k, g, per_line, &look);
                    }
                    ui.add_space(GAP);
                }

                // Memory and sensors: side by side in a wide column, stacked otherwise.
                let two = cw >= font::grow(640.0);
                let h = font::grow(176.0);
                let (r, _) = ui.allocate_exact_size(vec2(w, if two { h } else { h * 2.0 + GAP }), Sense::hover());
                for (k, g) in chunk.iter().enumerate() {
                    let left = x(r, k);
                    let half = if two { ((cw - GAP) / 2.0).floor() } else { cw };
                    let mr = Rect::from_min_size(pos2(left, r.top()), vec2(half, h));
                    let sr = if two {
                        Rect::from_min_size(pos2(left + half + GAP, r.top()), vec2(half, h))
                    } else {
                        Rect::from_min_size(pos2(left, r.top() + h + GAP), vec2(half, h))
                    };
                    memory_card(ui, mr, first + k, g, &series[first + k].1, &look);
                    sensors_card(ui, sr, first + k, g, &look);
                }
                ui.add_space(GAP);
            }
            ui.add_space(16.0);
        },
    );
}

/// The title an adapter goes by: its name, or `GPU <index>` when it has none.
pub fn gpu_title(i: usize, g: &GpuRaw) -> String {
    if g.name.is_empty() { format!("GPU {i}") } else { g.name.clone() }
}

fn load_card(ui: &mut Ui, r: Rect, i: usize, g: &GpuRaw, load: &[Pt], beside: bool, l: &Look<'_>) {
    let p = l.p;
    card(ui, p, r, Id::unique(("soyuz.monitor.gpu.load", l.pane, i)), false);
    let painter = ui.painter_at(r);
    let y = card_title(&painter, p, r, ph::GRAPHICS_CARD, hue(p, Hue::Gpu), &gpu_title(i, g));
    let mut bx = r.left() + 14.0;
    if !g.vendor.is_empty() {
        bx += badge(&painter, p, pos2(bx, y - 2.0), &g.vendor, p.text_secondary) + 6.0;
    }
    if let Some(t) = g.mem_total {
        badge(&painter, p, pos2(bx, y - 2.0), &soyuz_core::format_size(t, l.style), p.text_secondary);
    }
    super::big_number(
        &painter,
        p,
        pos2(r.left() + 14.0, y + 20.0),
        &g.percent.map_or(l.dash.to_string(), labels::percent),
        "",
        200.0,
    );
    let eng_w = if beside && !g.engines.is_empty() { font::grow(230.0) } else { 0.0 };
    let plot = Rect::from_min_max(
        pos2(r.left() + 12.0, y + font::grow(60.0)),
        pos2(r.right() - 12.0 - eng_w - if eng_w > 0.0 { 12.0 } else { 0.0 }, r.bottom() - 10.0),
    );
    let s = [Series::new(tr!("monitor-gui-load-label"), hue(p, Hue::Gpu), load)];
    Chart::new(l.span, Unit::Percent, l.fmt).y_max(100.0).show(
        ui,
        plot,
        Id::unique(("soyuz.monitor.gpu.chart", l.pane, i)),
        p,
        &s,
    );
    if eng_w > 0.0 {
        let ex = r.right() - 12.0 - eng_w;
        let ey = y + 22.0;
        super::caption(&painter, p, pos2(ex, ey), tr!("monitor-gui-engines"));
        let fit = ((r.bottom() - 10.0 - ey - 14.0) / engine_row()).floor() as usize;
        engine_list(&painter, p, g, ex, ey + 14.0, eng_w, 1, fit);
    }
}

fn engines_card(ui: &mut Ui, r: Rect, i: usize, g: &GpuRaw, per_line: usize, l: &Look<'_>) {
    let p = l.p;
    card(ui, p, r, Id::unique(("soyuz.monitor.gpu.engines", l.pane, i)), false);
    let painter = ui.painter_at(r);
    let y = card_title(&painter, p, r, ph::GRAPHICS_CARD, hue(p, Hue::Gpu), tr!("monitor-gui-engines"));
    if g.engines.is_empty() {
        painter.text(
            pos2(r.left() + 14.0, y + 4.0),
            Align2::LEFT_TOP,
            tr!("monitor-gui-not-reported"),
            font::ui(12.0),
            p.text_tertiary,
        );
        return;
    }
    let fit = ((r.bottom() - 6.0 - y) / engine_row()).floor() as usize * per_line;
    engine_list(&painter, p, g, r.left() + 14.0, y, r.width() - 28.0, per_line, fit);
}

/// The busiest engines first, `per_line` to a line, at most `fit` of them.
#[allow(clippy::too_many_arguments)]
fn engine_list(painter: &egui::Painter, p: &Palette, g: &GpuRaw, x: f32, y: f32, w: f32, per_line: usize, fit: usize) {
    let per_line = per_line.max(1);
    let ew = ((w - GAP * (per_line as f32 - 1.0)) / per_line as f32).floor();
    let mut engines: Vec<_> = g.engines.iter().collect();
    engines.sort_by(|a, b| b.percent.total_cmp(&a.percent));
    for (n, e) in engines.iter().take(fit).enumerate() {
        let ex = x + (n % per_line) as f32 * (ew + GAP);
        let ey = y + (n / per_line) as f32 * engine_row();
        let lg = crate::ui::widgets::trunc_galley(painter, &e.name, font::ui(11.0), p.text_secondary, ew * 0.6);
        painter.galley(pos2(ex, ey), lg, p.text_secondary);
        let vg = painter.layout_no_wrap(labels::percent(e.percent), font::numbers(11.0), p.text_primary);
        painter.galley(pos2(ex + ew - vg.size().x, ey), vg, p.text_primary);
        let bar = Rect::from_min_size(pos2(ex, ey + 15.0), vec2(ew, 3.0));
        meter(painter, p, bar, e.percent / 100.0, hue(p, Hue::Gpu), &[]);
    }
}

fn memory_card(ui: &mut Ui, mr: Rect, i: usize, g: &GpuRaw, mem: &[Pt], l: &Look<'_>) {
    let p = l.p;
    card(ui, p, mr, Id::unique(("soyuz.monitor.gpu.mem", l.pane, i)), false);
    let painter = ui.painter_at(mr);
    let y = card_title(&painter, p, mr, ph::MEMORY, hue(p, Hue::Memory), tr!("monitor-gui-gpu-memory"));
    match (g.mem_used, g.mem_total) {
        (Some(u), Some(t)) if t > 0 => {
            super::big_number(
                &painter,
                p,
                pos2(mr.left() + 14.0, y),
                &soyuz_core::format_size(u, l.style),
                &tr!(
                    "monitor-gui-mem-sub",
                    percent = labels::percent(u as f32 / t as f32 * 100.0).as_str(),
                    total = soyuz_core::format_size(t, l.style).as_str()
                ),
                mr.width() - 28.0,
            );
            // The shared-memory row sits at the bottom: the chart ends above it, or the row
            // covers the line (at a fifth of the card when a fifth of the memory is in use).
            let bottom =
                if g.shared_used.is_some() { mr.bottom() - font::grow(24.0) - 6.0 } else { mr.bottom() - 10.0 };
            let plot =
                Rect::from_min_max(pos2(mr.left() + 12.0, y + font::grow(50.0)), pos2(mr.right() - 12.0, bottom));
            let s = [Series::new(tr!("monitor-gui-gpu-memory"), hue(p, Hue::Memory), mem)];
            Chart::mini(l.span, Unit::Bytes, l.fmt).y_max(t as f32).show(
                ui,
                plot,
                Id::unique(("soyuz.monitor.gpu.mem.chart", l.pane, i)),
                p,
                &s,
            );
        }
        (Some(u), None) => {
            super::big_number(
                &painter,
                p,
                pos2(mr.left() + 14.0, y),
                &soyuz_core::format_size(u, l.style),
                "",
                mr.width() - 28.0,
            );
        }
        _ => {
            painter.text(
                pos2(mr.left() + 14.0, y + 10.0),
                Align2::LEFT_TOP,
                tr!("monitor-gui-not-reported"),
                font::ui(12.0),
                p.text_tertiary,
            );
        }
    }
    if let Some(sh) = g.shared_used {
        fact(
            &painter,
            p,
            mr.left() + 14.0,
            mr.bottom() - font::grow(24.0),
            mr.width() - 28.0,
            tr!("monitor-gui-gpu-shared"),
            &soyuz_core::format_size(sh, l.style),
        );
    }
}

fn sensors_card(ui: &mut Ui, sr: Rect, i: usize, g: &GpuRaw, l: &Look<'_>) {
    let p = l.p;
    card(ui, p, sr, Id::unique(("soyuz.monitor.gpu.sensors", l.pane, i)), false);
    let painter = ui.painter_at(sr);
    let mut y = card_title(&painter, p, sr, ph::THERMOMETER, hue(p, Hue::Temp), tr!("monitor-page-sensors"));
    let fw = sr.width() - 28.0;
    let fx = sr.left() + 14.0;
    let dash = || l.dash.to_string();
    let mhz = |v: Option<u32>| v.map_or_else(dash, |v| super::charts::fmt_value(Unit::Mhz, v as f32, l.fmt));
    let facts = [
        (tr!("monitor-gui-temperature"), g.temperature_c.map_or_else(dash, |t| labels::temperature(t, l.temp_unit))),
        (tr!("monitor-gui-power"), g.power_w.map_or_else(dash, |v| super::charts::fmt_value(Unit::Watts, v, l.fmt))),
        (tr!("monitor-gui-fan"), g.fan_rpm.map_or_else(dash, |v| super::charts::fmt_value(Unit::Rpm, v as f32, l.fmt))),
        (tr!("monitor-gui-clock"), mhz(g.clock_mhz)),
        (tr!("monitor-gui-mem-clock"), mhz(g.mem_clock_mhz)),
    ];
    for (label, value) in &facts {
        y = fact(&painter, p, fx, y, fw, label, value);
    }
}
