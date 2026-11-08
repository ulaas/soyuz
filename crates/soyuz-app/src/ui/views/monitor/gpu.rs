//! The GPU page: per adapter the load chart, the engines, memory (bar and chart),
//! temperature, power, fan and clocks.

use egui::{Align2, Id, Rect, ScrollArea, Sense, Ui, UiBuilder, pos2, vec2};
use soyuz_sysmon::Metric;

use super::charts::{Chart, Hue, Series, Unit, hue, meter};
use super::{PageCtx, badge, card, card_title, fact};
use crate::models::monitor::labels;
use crate::theme::{font, icons::ph};

const GAP: f32 = 12.0;

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
    let fmt = c.fmt();
    let style = c.style();
    let span = c.span();
    let pane = c.pane.0;
    let temp_unit = c.cfg().temperature_unit;
    let dash = PageCtx::dash().to_string();
    let series: Vec<_> = (0..frame.gpus.len())
        .map(|i| (c.series(&Metric::GpuLoad(i as u16)), c.series(&Metric::GpuMem(i as u16))))
        .collect();
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    ScrollArea::vertical().id_salt(("soyuz.monitor.gpu", pane)).auto_shrink([false, false]).show(&mut child, |ui| {
        ui.add_space(4.0);
        let w = ui.available_width();
        for (i, g) in frame.gpus.iter().enumerate() {
            let (load, mem) = &series[i];
            // Load card with the engines on the right.
            let ch = font::grow(250.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, ch), Sense::hover());
            card(ui, &p, r, Id::unique(("soyuz.monitor.gpu.load", pane, i)), false);
            let painter = ui.painter_at(r);
            let title = if g.name.is_empty() { format!("GPU {i}") } else { g.name.clone() };
            let y = card_title(&painter, &p, r, ph::GRAPHICS_CARD, hue(&p, Hue::Gpu), &title);
            let mut bx = r.left() + 14.0;
            if !g.vendor.is_empty() {
                bx += badge(&painter, &p, pos2(bx, y - 2.0), &g.vendor, p.text_secondary) + 6.0;
            }
            if let Some(t) = g.mem_total {
                badge(&painter, &p, pos2(bx, y - 2.0), &soyuz_core::format_size(t, style), p.text_secondary);
            }
            super::big_number(
                &painter,
                &p,
                pos2(r.left() + 14.0, y + 20.0),
                &g.percent.map_or(dash.clone(), labels::percent),
                "",
                200.0,
            );
            let wide = w >= font::grow(760.0);
            let eng_w = if wide && !g.engines.is_empty() { font::grow(230.0) } else { 0.0 };
            let plot = Rect::from_min_max(
                pos2(r.left() + 12.0, y + font::grow(60.0)),
                pos2(r.right() - 12.0 - eng_w - if eng_w > 0.0 { 12.0 } else { 0.0 }, r.bottom() - 10.0),
            );
            let s = [Series::new(tr!("monitor-gui-load-label"), hue(&p, Hue::Gpu), load)];
            Chart::new(span, Unit::Percent, fmt).y_max(100.0).show(
                ui,
                plot,
                Id::unique(("soyuz.monitor.gpu.chart", pane, i)),
                &p,
                &s,
            );
            if eng_w > 0.0 {
                let ex = r.right() - 12.0 - eng_w;
                let mut ey = y + 22.0;
                super::caption(&painter, &p, pos2(ex, ey), tr!("monitor-gui-engines"));
                ey += 14.0;
                let mut engines: Vec<_> = g.engines.iter().collect();
                engines.sort_by(|a, b| b.percent.total_cmp(&a.percent));
                for e in engines.iter().take(((r.bottom() - 10.0 - ey) / font::grow(22.0)).floor() as usize) {
                    let lg = crate::ui::widgets::trunc_galley(
                        &painter,
                        &e.name,
                        font::ui(11.0),
                        p.text_secondary,
                        eng_w * 0.5,
                    );
                    painter.galley(pos2(ex, ey), lg, p.text_secondary);
                    let vg = painter.layout_no_wrap(labels::percent(e.percent), font::numbers(11.0), p.text_primary);
                    painter.galley(pos2(ex + eng_w - vg.size().x, ey), vg, p.text_primary);
                    let bar = Rect::from_min_size(pos2(ex, ey + 15.0), vec2(eng_w, 3.0));
                    meter(&painter, &p, bar, e.percent / 100.0, hue(&p, Hue::Gpu), &[]);
                    ey += font::grow(22.0);
                }
            }
            ui.add_space(GAP);
            // Memory and sensors.
            let two = w >= font::grow(640.0);
            let cw = if two { ((w - GAP) / 2.0).floor() } else { w };
            let h = font::grow(176.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, if two { h } else { h * 2.0 + GAP }), Sense::hover());
            let mr = Rect::from_min_size(r.min, vec2(cw, h));
            let sr = if two {
                Rect::from_min_size(pos2(r.left() + cw + GAP, r.top()), vec2(cw, h))
            } else {
                Rect::from_min_size(pos2(r.left(), r.top() + h + GAP), vec2(cw, h))
            };
            card(ui, &p, mr, Id::unique(("soyuz.monitor.gpu.mem", pane, i)), false);
            let painter = ui.painter_at(mr);
            let y = card_title(&painter, &p, mr, ph::MEMORY, hue(&p, Hue::Memory), tr!("monitor-gui-gpu-memory"));
            match (g.mem_used, g.mem_total) {
                (Some(u), Some(t)) if t > 0 => {
                    super::big_number(
                        &painter,
                        &p,
                        pos2(mr.left() + 14.0, y),
                        &soyuz_core::format_size(u, style),
                        &tr!(
                            "monitor-gui-mem-sub",
                            percent = labels::percent(u as f32 / t as f32 * 100.0).as_str(),
                            total = soyuz_core::format_size(t, style).as_str()
                        ),
                        mr.width() - 28.0,
                    );
                    // The shared-memory row sits at the bottom: the chart ends above it,
                    // or the row covers the line (at a fifth of the card when a fifth of the
                    // memory is in use).
                    let bottom =
                        if g.shared_used.is_some() { mr.bottom() - font::grow(24.0) - 6.0 } else { mr.bottom() - 10.0 };
                    let plot = Rect::from_min_max(
                        pos2(mr.left() + 12.0, y + font::grow(50.0)),
                        pos2(mr.right() - 12.0, bottom),
                    );
                    let s = [Series::new(tr!("monitor-gui-gpu-memory"), hue(&p, Hue::Memory), mem)];
                    Chart::mini(span, Unit::Bytes, fmt).y_max(t as f32).show(
                        ui,
                        plot,
                        Id::unique(("soyuz.monitor.gpu.mem.chart", pane, i)),
                        &p,
                        &s,
                    );
                }
                (Some(u), None) => {
                    super::big_number(
                        &painter,
                        &p,
                        pos2(mr.left() + 14.0, y),
                        &soyuz_core::format_size(u, style),
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
                    &p,
                    mr.left() + 14.0,
                    mr.bottom() - font::grow(24.0),
                    mr.width() - 28.0,
                    tr!("monitor-gui-gpu-shared"),
                    &soyuz_core::format_size(sh, style),
                );
            }
            card(ui, &p, sr, Id::unique(("soyuz.monitor.gpu.sensors", pane, i)), false);
            let painter = ui.painter_at(sr);
            let mut y = card_title(&painter, &p, sr, ph::THERMOMETER, hue(&p, Hue::Temp), tr!("monitor-page-sensors"));
            let fw = sr.width() - 28.0;
            let fx = sr.left() + 14.0;
            y = fact(
                &painter,
                &p,
                fx,
                y,
                fw,
                tr!("monitor-gui-temperature"),
                &g.temperature_c.map_or(dash.clone(), |t| labels::temperature(t, temp_unit)),
            );
            y = fact(
                &painter,
                &p,
                fx,
                y,
                fw,
                tr!("monitor-gui-power"),
                &g.power_w.map_or(dash.clone(), |v| super::charts::fmt_value(Unit::Watts, v, fmt)),
            );
            y = fact(
                &painter,
                &p,
                fx,
                y,
                fw,
                tr!("monitor-gui-fan"),
                &g.fan_rpm.map_or(dash.clone(), |v| super::charts::fmt_value(Unit::Rpm, v as f32, fmt)),
            );
            y = fact(
                &painter,
                &p,
                fx,
                y,
                fw,
                tr!("monitor-gui-clock"),
                &g.clock_mhz.map_or(dash.clone(), |v| super::charts::fmt_value(Unit::Mhz, v as f32, fmt)),
            );
            fact(
                &painter,
                &p,
                fx,
                y,
                fw,
                tr!("monitor-gui-mem-clock"),
                &g.mem_clock_mhz.map_or(dash.clone(), |v| super::charts::fmt_value(Unit::Mhz, v as f32, fmt)),
            );
            ui.add_space(GAP);
        }
        ui.add_space(16.0);
    });
}
