//! The Sensors page: temperatures grouped by chip with meters to their maximum and
//! critical points, fans, power, voltages, currents, and the batteries.

use egui::{Align2, Id, Rect, ScrollArea, Sense, Ui, UiBuilder, pos2, vec2};
use soyuz_sysmon::{SensorKind, SensorRaw};

use super::charts::{Hue, hue, meter};
use super::{PageCtx, card, card_title, fact, grid_cols};
use crate::models::monitor::labels;
use crate::theme::{font, icons::ph};

const GAP: f32 = 12.0;

pub fn show(ui: &mut Ui, rect: Rect, c: &mut PageCtx<'_>) {
    let p = c.p.clone();
    let Some(frame) = c.frame.clone() else {
        super::empty(ui, rect, ph::THERMOMETER, tr!("monitor-no-data"), tr!("monitor-gui-waiting"));
        return;
    };
    if frame.sensors.is_empty() && frame.batteries.is_empty() {
        super::empty(ui, rect, ph::THERMOMETER, tr!("monitor-gui-no-sensors"), tr!("monitor-gui-no-sensors-sub"));
        return;
    }
    let unit = c.cfg().temperature_unit;
    let fmt = c.fmt();
    let pane = c.pane.0;
    let dash = PageCtx::dash().to_string();
    // Groups in order of first appearance.
    let mut groups: Vec<(&str, Vec<&SensorRaw>)> = Vec::new();
    for s in &frame.sensors {
        match groups.iter_mut().find(|(g, _)| *g == s.group.as_str()) {
            Some((_, v)) => v.push(s),
            None => groups.push((s.group.as_str(), vec![s])),
        }
    }
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    ScrollArea::vertical().id_salt(("soyuz.monitor.sensors", pane)).auto_shrink([false, false]).show(
        &mut child,
        |ui| {
            ui.add_space(4.0);
            let w = ui.available_width();
            // ---- batteries ----
            if !frame.batteries.is_empty() {
                let (cols, cw) = grid_cols(w, font::grow(300.0), GAP);
                let h = font::grow(170.0);
                for (ri, chunk) in frame.batteries.chunks(cols).enumerate() {
                    let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                    for (k, b) in chunk.iter().enumerate() {
                        let cr = Rect::from_min_size(pos2(r.left() + k as f32 * (cw + GAP), r.top()), vec2(cw, h));
                        card(ui, &p, cr, Id::unique(("soyuz.monitor.battery", pane, ri, k)), false);
                        let painter = ui.painter_at(cr);
                        let glyph = if b.charging { ph::BATTERY_CHARGING } else { ph::BATTERY_HIGH };
                        let title =
                            if b.name.is_empty() { tr!("monitor-gui-battery").to_string() } else { b.name.clone() };
                        let y = card_title(&painter, &p, cr, glyph, hue(&p, Hue::Power), &title);
                        let state = if b.charging {
                            tr!("monitor-gui-battery-charging")
                        } else if b.on_ac {
                            tr!("monitor-gui-battery-ac")
                        } else {
                            tr!("monitor-gui-battery-discharging")
                        };
                        super::big_number(
                            &painter,
                            &p,
                            pos2(cr.left() + 14.0, y),
                            &labels::percent(b.percent),
                            state,
                            cw - 28.0,
                        );
                        let bar =
                            Rect::from_min_size(pos2(cr.left() + 14.0, y + font::grow(50.0)), vec2(cw - 28.0, 6.0));
                        let color = if b.percent < 15.0 && !b.charging { p.danger } else { hue(&p, Hue::Power) };
                        meter(&painter, &p, bar, b.percent / 100.0, color, &[]);
                        let fx = cr.left() + 14.0;
                        let fw = cw - 28.0;
                        let mut fy = bar.bottom() + 10.0;
                        fy = fact(
                            &painter,
                            &p,
                            fx,
                            fy,
                            fw,
                            tr!("monitor-gui-time-left"),
                            &b.seconds_left.map_or(dash.clone(), labels::uptime),
                        );
                        fy = fact(
                            &painter,
                            &p,
                            fx,
                            fy,
                            fw,
                            tr!("monitor-gui-power"),
                            &b.power_w.map_or(dash.clone(), |v| format!("{v:+.1} W")),
                        );
                        fy = fact(
                            &painter,
                            &p,
                            fx,
                            fy,
                            fw,
                            tr!("monitor-gui-health"),
                            &b.health_percent.map_or(dash.clone(), labels::percent),
                        );
                        fact(
                            &painter,
                            &p,
                            fx,
                            fy,
                            fw,
                            tr!("monitor-gui-cycles"),
                            &b.cycles.map_or(dash.clone(), |v| labels::count(u64::from(v))),
                        );
                    }
                    ui.add_space(GAP);
                }
            }
            // ---- sensor groups ----
            let (cols, cw) = grid_cols(w, font::grow(320.0), GAP);
            let row_h = font::grow(26.0);
            let mut items: Vec<(Rect, usize)> = Vec::new();
            // Lay groups out as cards in columns; a simple row flow keeps it readable.
            let heights: Vec<f32> =
                groups.iter().map(|(_, v)| font::grow(44.0) + v.len() as f32 * row_h + 8.0).collect();
            let mut gi = 0;
            while gi < groups.len() {
                let n = cols.min(groups.len() - gi);
                let h = heights[gi..gi + n].iter().copied().fold(0.0, f32::max);
                let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                for k in 0..n {
                    items.push((
                        Rect::from_min_size(pos2(r.left() + k as f32 * (cw + GAP), r.top()), vec2(cw, heights[gi + k])),
                        gi + k,
                    ));
                }
                gi += n;
                ui.add_space(GAP);
            }
            for (cr, gi) in items {
                let (name, sensors) = &groups[gi];
                card(ui, &p, cr, Id::unique(("soyuz.monitor.sensor.group", pane, gi)), false);
                let painter = ui.painter_at(cr);
                let kind = sensors.first().map_or(SensorKind::Temperature, |s| s.kind);
                let (glyph, color) = match kind {
                    SensorKind::Temperature => (ph::THERMOMETER, hue(&p, Hue::Temp)),
                    SensorKind::Fan => (ph::FAN, hue(&p, Hue::Cpu)),
                    SensorKind::Power => (ph::LIGHTNING, hue(&p, Hue::Power)),
                    SensorKind::Voltage | SensorKind::Current => (ph::PLUG, hue(&p, Hue::Neutral)),
                };
                let title = if name.is_empty() { tr!("monitor-page-sensors").to_string() } else { (*name).to_string() };
                let mut y = card_title(&painter, &p, cr, glyph, color, &title) + 2.0;
                let fx = cr.left() + 14.0;
                let fw = cr.width() - 28.0;
                let fan_max = sensors
                    .iter()
                    .filter(|s| s.kind == SensorKind::Fan)
                    .map(|s| s.max.unwrap_or(s.value))
                    .fold(1.0f32, f32::max);
                for s in sensors {
                    let label = if s.label.is_empty() { s.id.as_str() } else { s.label.as_str() };
                    let lg =
                        crate::ui::widgets::trunc_galley(&painter, label, font::ui(11.5), p.text_secondary, fw * 0.4);
                    painter.galley(pos2(fx, y + 3.0), lg, p.text_secondary);
                    let value = labels::sensor_value(s.kind, s.value, unit);
                    let vg = painter.layout_no_wrap(value, font::numbers(11.5), p.text_primary);
                    painter.galley(pos2(fx + fw - vg.size().x, y + 3.0), vg, p.text_primary);
                    let bar_x = fx + fw * 0.42;
                    let bar_w = fw * 0.58 - 70.0;
                    if bar_w > 30.0 {
                        let bar = Rect::from_min_size(pos2(bar_x, y + 9.0), vec2(bar_w, 5.0));
                        match s.kind {
                            SensorKind::Temperature => {
                                let top = s.critical.or(s.max).map_or(105.0, |m| m.max(s.value + 5.0));
                                let frac = (s.value / top).clamp(0.0, 1.0);
                                let hot =
                                    s.critical.is_some_and(|c| s.value >= c) || s.max.is_some_and(|m| s.value >= m);
                                let col = if hot { p.danger } else { hue(&p, Hue::Temp) };
                                let mut marks = Vec::new();
                                if let Some(m) = s.max {
                                    marks.push((m / top, p.warning));
                                }
                                if let Some(cr_) = s.critical {
                                    marks.push((cr_ / top, p.danger));
                                }
                                meter(&painter, &p, bar, frac, col, &marks);
                                if hot {
                                    painter.text(
                                        pos2(bar.right() + 10.0, bar.center().y),
                                        Align2::LEFT_CENTER,
                                        ph::WARNING,
                                        crate::theme::icons::font(11.0),
                                        p.danger,
                                    );
                                }
                            }
                            SensorKind::Fan => meter(&painter, &p, bar, s.value / fan_max, hue(&p, Hue::Cpu), &[]),
                            SensorKind::Power => {
                                let top = s.max.unwrap_or(s.value.max(1.0) * 1.5);
                                meter(&painter, &p, bar, s.value / top, hue(&p, Hue::Power), &[]);
                            }
                            SensorKind::Voltage | SensorKind::Current => {}
                        }
                    }
                    y += row_h;
                }
                let _ = fmt;
            }
            ui.add_space(16.0);
        },
    );
}
