//! The History page: range chips, a live button, stacked small multiples of the recorded
//! metrics sharing one time axis and one crosshair (the average line with a faint maximum
//! band, breaks where nothing was recorded), panning by dragging, and under the crosshair
//! the value of every chart plus the top processes of that moment.

use egui::{Align2, Id, Rect, ScrollArea, Sense, Ui, UiBuilder, pos2, vec2};
use soyuz_sysmon::{Metric, Pt};

use super::charts::{Chart, Hue, Series, Unit, fmt_value, hue};
use super::{PageCtx, caption, card};
use crate::models::monitor::{HistoryState, MonitorCmd, history_metrics, labels};
use crate::theme::font::semibold;
use crate::theme::{Palette, font, icons::ph};
use crate::ui::widgets::{Btn, chip, pill_button};

const GAP: f32 = 10.0;

/// Title, unit, hue and fixed top of a recorded metric.
fn describe(m: &Metric, temp: soyuz_core::config::monitor::TempUnit) -> (String, Unit, Hue, Option<f32>) {
    match m {
        Metric::CpuTotal => (
            labels::page_title(soyuz_core::config::monitor::MonitorPage::Cpu).to_string(),
            Unit::Percent,
            Hue::Cpu,
            Some(100.0),
        ),
        Metric::CpuUser => (tr!("monitor-gui-user").to_string(), Unit::Percent, Hue::Cpu, Some(100.0)),
        Metric::CpuSystem => (tr!("monitor-gui-system").to_string(), Unit::Percent, Hue::Cpu, Some(100.0)),
        Metric::Core(i) => (format!("CPU {i}"), Unit::Percent, Hue::Cpu, Some(100.0)),
        Metric::MemPercent => (
            labels::page_title(soyuz_core::config::monitor::MonitorPage::Memory).to_string(),
            Unit::Percent,
            Hue::Memory,
            Some(100.0),
        ),
        Metric::MemUsed => (tr!("monitor-gui-mem-in-use").to_string(), Unit::Bytes, Hue::Memory, None),
        Metric::SwapPercent => (tr!("monitor-gui-swap").to_string(), Unit::Percent, Hue::Swap, Some(100.0)),
        Metric::DiskRead => (tr!("monitor-gui-disk-read").to_string(), Unit::Rate, Hue::DiskRead, None),
        Metric::DiskWrite => (tr!("monitor-gui-disk-write").to_string(), Unit::Rate, Hue::DiskWrite, None),
        Metric::DiskReadOf(id) => (format!("{} {id}", tr!("monitor-gui-read")), Unit::Rate, Hue::DiskRead, None),
        Metric::DiskWriteOf(id) => (format!("{} {id}", tr!("monitor-gui-write")), Unit::Rate, Hue::DiskWrite, None),
        Metric::DiskBusyOf(id) => {
            (format!("{} {id}", tr!("monitor-gui-busy")), Unit::Percent, Hue::DiskRead, Some(100.0))
        }
        Metric::NetRx => (tr!("monitor-gui-net-received").to_string(), Unit::Rate, Hue::NetRx, None),
        Metric::NetTx => (tr!("monitor-gui-net-sent").to_string(), Unit::Rate, Hue::NetTx, None),
        Metric::NetRxOf(id) => (format!("{} {id}", tr!("monitor-gui-received")), Unit::Rate, Hue::NetRx, None),
        Metric::NetTxOf(id) => (format!("{} {id}", tr!("monitor-gui-sent")), Unit::Rate, Hue::NetTx, None),
        Metric::GpuLoad(i) => (format!("GPU {i}"), Unit::Percent, Hue::Gpu, Some(100.0)),
        Metric::GpuMem(i) => (format!("{} {i}", tr!("monitor-gui-gpu-memory")), Unit::Bytes, Hue::Gpu, None),
        Metric::MaxTemp => (tr!("monitor-gui-hottest").to_string(), Unit::Temp(temp), Hue::Temp, None),
        Metric::ProcessCount => (tr!("monitor-gui-processes").to_string(), Unit::Count, Hue::Neutral, None),
    }
}

fn range_label(secs: u32) -> String {
    super::charts::short_duration(u64::from(secs))
}

pub fn show(ui: &mut Ui, rect: Rect, c: &mut PageCtx<'_>) {
    let p = c.p.clone();
    let pane = c.pane.0;
    let fmt = c.fmt();
    let temp = c.cfg().temperature_unit;
    let config = c.config;
    let frame = c.frame.clone();
    let now = frame.as_ref().map_or(0, |f| f.at_ms);
    let metrics = history_metrics(frame.as_deref());
    let (range_secs, end_ms, cursor_ms, tops, oldest) = {
        let h = &c.view().history;
        (h.range_secs, h.end_ms, h.cursor_ms, h.tops.clone(), h.oldest_ms)
    };
    let series: Vec<std::sync::Arc<Vec<Pt>>> = {
        let h = &c.view().history;
        metrics.iter().map(|m| h.series(m)).collect()
    };
    let to = end_ms.unwrap_or(now);
    let window = (to - i64::from(range_secs) * 1000, to);
    let mut pending: Vec<MonitorCmd> = Vec::new();
    // ---- toolbar ----
    let tb = Rect::from_min_size(rect.min, vec2(rect.width(), font::grow(40.0)));
    let mut bar = ui.new_child(
        UiBuilder::new().max_rect(tb.shrink2(vec2(0.0, 6.0))).layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    bar.spacing_mut().item_spacing.x = 4.0;
    // Recording: what it is doing, and what can be done about it.
    let recording = soyuz_sysmon::Hub::global().recording();
    match recording {
        soyuz_sysmon::Recording::Off => {
            if pill_button(&mut bar, Some(ph::RECORD), tr!("monitor-gui-record"), Btn::Primary)
                .on_hover_text(tr!("monitor-gui-record-tip"))
                .clicked()
            {
                pending.push(MonitorCmd::RecordStart);
            }
        }
        state => {
            let paused = state == soyuz_sysmon::Recording::Paused;
            let (glyph, label) = if paused {
                (ph::PLAY, tr!("monitor-gui-record-resume"))
            } else {
                (ph::PAUSE, tr!("monitor-gui-record-pause"))
            };
            if pill_button(&mut bar, Some(glyph), label, Btn::AccentSoft).clicked() {
                pending.push(MonitorCmd::RecordPause);
            }
            if pill_button(&mut bar, Some(ph::STOP), tr!("monitor-gui-record-stop"), Btn::AccentSoft).clicked() {
                pending.push(MonitorCmd::RecordStop);
            }
        }
    }
    bar.add_space(10.0);
    for r in HistoryState::RANGES {
        if chip(&mut bar, None, &range_label(r), r == range_secs).clicked() {
            pending.push(MonitorCmd::HistoryRange(r));
        }
    }
    bar.add_space(8.0);
    if end_ms.is_some() {
        if pill_button(&mut bar, Some(ph::PLAY), tr!("monitor-gui-live"), Btn::AccentSoft).clicked() {
            pending.push(MonitorCmd::HistoryLive);
        }
    } else {
        bar.label(
            egui::RichText::new(format!("{}  {}", ph::RECORD, ""))
                .font(crate::theme::icons::font(11.0))
                .color(p.success),
        );
        bar.label(egui::RichText::new(tr!("monitor-gui-live")).font(font::ui(11.5)).color(p.text_secondary));
    }
    bar.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let (state, color) = match recording {
            soyuz_sysmon::Recording::Off => (tr!("monitor-gui-not-recording"), p.text_tertiary),
            soyuz_sysmon::Recording::On => (tr!("monitor-gui-recording"), p.danger),
            soyuz_sysmon::Recording::Paused => (tr!("monitor-gui-recording-paused"), p.warning),
        };
        if let Some(o) = oldest {
            let text = tr!("monitor-gui-recorded-since", time = super::charts::clock(o, true).as_str());
            ui.label(egui::RichText::new(text).font(font::ui(11.0)).color(p.text_tertiary));
            ui.add_space(8.0);
        }
        ui.label(egui::RichText::new(state).font(font::ui(11.5)).color(p.text_secondary));
        if recording != soyuz_sysmon::Recording::Off {
            let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
            ui.painter().circle_filled(dot.center(), 3.5, color);
        }
    });
    let body = Rect::from_min_max(pos2(rect.left(), tb.bottom()), rect.max);
    let wide = body.width() >= font::grow(900.0);
    let side_w = if wide { font::grow(260.0) } else { 0.0 };
    let charts_rect =
        Rect::from_min_max(body.min, pos2(body.right() - side_w - if wide { GAP } else { 0.0 }, body.bottom()));
    let side_rect = Rect::from_min_max(pos2(body.right() - side_w, body.top()), body.max);

    // ---- charts ----
    let mut hover_t: Option<i64> = None;
    let mut any_hover = false;
    let mut pan: Option<i64> = None;
    let mut values: Vec<(String, egui::Color32, String)> = Vec::new();
    let mut child = ui.new_child(UiBuilder::new().max_rect(charts_rect));
    ScrollArea::vertical().id_salt(("soyuz.monitor.history", pane)).auto_shrink([false, false]).show(
        &mut child,
        |ui| {
            ui.add_space(2.0);
            let w = ui.available_width();
            let ch = font::grow(118.0);
            for (i, m) in metrics.iter().enumerate() {
                let (title, unit, h, ymax) = describe(m, temp);
                let (r, _) = ui.allocate_exact_size(vec2(w, ch), Sense::hover());
                card(ui, &p, r, Id::unique(("soyuz.monitor.history.card", pane, i)), false);
                let painter = ui.painter_at(r);
                let pts = &series[i];
                painter.text(
                    pos2(r.left() + 12.0, r.top() + 13.0),
                    Align2::LEFT_CENTER,
                    &title,
                    semibold(12.0),
                    p.text_primary,
                );
                let plot =
                    Rect::from_min_max(pos2(r.left() + 8.0, r.top() + 24.0), pos2(r.right() - 10.0, r.bottom() - 6.0));
                if pts.is_empty() {
                    painter.text(
                        plot.center(),
                        Align2::CENTER_CENTER,
                        tr!("monitor-gui-nothing-recorded"),
                        font::ui(11.5),
                        p.text_tertiary,
                    );
                }
                let s = [Series::new(title.clone(), hue(&p, h), pts).band()];
                let mut chart = Chart::new(window, unit, fmt).absolute_time().draggable().no_legend().cursor(cursor_ms);
                chart.y_max = ymax;
                chart.live_dot = end_ms.is_none();
                let out = chart.show(ui, plot, Id::unique(("soyuz.monitor.history.chart", pane, i)), &p, &s);
                if out.response.hovered() {
                    any_hover = true;
                    if let Some(t) = out.hover_t {
                        hover_t = Some(t);
                    }
                }
                if out.response.dragged() && out.plot.width() > 0.0 {
                    let dx = out.response.drag_delta().x;
                    if dx.abs() >= 1.0 {
                        let secs = (dx / out.plot.width() * range_secs as f32).round() as i64;
                        pan = Some(pan.unwrap_or(0) + secs);
                    }
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                }
                if let Some(t) = cursor_ms {
                    let v = match soyuz_sysmon::Pt::near(pts, t, i64::from(range_secs) * 1000) {
                        Some(pt) => {
                            let v = fmt_value(unit, pt.avg, fmt);
                            if pt.max.is_finite() && (pt.max - pt.avg).abs() > f32::EPSILON {
                                tr!(
                                    "monitor-gui-avg-max",
                                    avg = v.as_str(),
                                    max = fmt_value(unit, pt.max, fmt).as_str()
                                )
                            } else {
                                v
                            }
                        }
                        None => "–".to_string(),
                    };
                    values.push((title.clone(), hue(&p, h), v));
                } else if let Some(last) = pts.iter().rev().find(|q| !q.is_break()) {
                    values.push((title.clone(), hue(&p, h), fmt_value(unit, last.avg, fmt)));
                }
                ui.add_space(GAP);
            }
            ui.add_space(12.0);
        },
    );
    if let Some(by) = pan {
        if by != 0 {
            pending.push(MonitorCmd::HistoryPan(by));
        }
    } else if any_hover {
        if hover_t != cursor_ms && hover_t.is_some() {
            pending.push(MonitorCmd::HistoryCursor(hover_t));
        }
    } else if cursor_ms.is_some() && !wide {
        pending.push(MonitorCmd::HistoryCursor(None));
    } else if cursor_ms.is_some() {
        // Keep the readout on the last hovered moment while the pointer is in the side panel.
        let over_side = ui.input(|i| i.pointer.hover_pos()).is_some_and(|q| side_rect.contains(q));
        if !over_side {
            pending.push(MonitorCmd::HistoryCursor(None));
        }
    }

    // ---- readout ----
    if wide {
        readout(
            ui,
            side_rect,
            &p,
            pane,
            cursor_ms.or(Some(window.1)),
            cursor_ms.is_some(),
            &values,
            tops.as_ref(),
            config,
            c.style(),
        );
    }
    for cmd in pending {
        c.cmd(cmd);
    }
}

#[allow(clippy::too_many_arguments)]
fn readout(
    ui: &mut Ui,
    rect: Rect,
    p: &Palette,
    pane: u64,
    at: Option<i64>,
    pinned: bool,
    values: &[(String, egui::Color32, String)],
    tops: Option<&soyuz_sysmon::Tops>,
    config: &soyuz_core::config::Config,
    style: soyuz_core::SizeStyle,
) {
    let r = Rect::from_min_max(pos2(rect.left(), rect.top() + 2.0), pos2(rect.right(), rect.bottom() - 12.0));
    card(ui, p, r, Id::unique(("soyuz.monitor.history.readout", pane)), false);
    let painter = ui.painter_at(r);
    let x = r.left() + 14.0;
    let w = r.width() - 28.0;
    let mut y = r.top() + 12.0;
    caption(
        &painter,
        p,
        pos2(x, y + 5.0),
        if pinned { tr!("monitor-gui-at-cursor") } else { tr!("monitor-gui-latest") },
    );
    y += 18.0;
    if let Some(t) = at {
        painter.text(pos2(x, y), Align2::LEFT_TOP, labels::time(t, config), font::numbers(13.0), p.text_primary);
        y += 24.0;
    }
    for (title, color, value) in values {
        if y + 18.0 > r.bottom() {
            break;
        }
        painter.circle_filled(pos2(x + 4.0, y + 8.0), 3.0, *color);
        let lg = crate::ui::widgets::trunc_galley(&painter, title, font::ui(11.5), p.text_secondary, w * 0.5);
        painter.galley(pos2(x + 14.0, y), lg, p.text_secondary);
        let vg = crate::ui::widgets::trunc_galley(&painter, value, font::numbers(11.5), p.text_primary, w * 0.5 - 14.0);
        painter.galley(pos2(x + w - vg.size().x, y), vg, p.text_primary);
        y += font::grow(19.0);
    }
    if let Some(t) = tops {
        for (cap, list, by_cpu) in
            [(tr!("monitor-gui-top-cpu"), &t.by_cpu, true), (tr!("monitor-gui-top-memory"), &t.by_mem, false)]
        {
            if y + 40.0 > r.bottom() {
                break;
            }
            y += 10.0;
            caption(&painter, p, pos2(x, y + 5.0), cap);
            y += 18.0;
            for tp in list.iter().take(5) {
                if y + 16.0 > r.bottom() {
                    break;
                }
                let v = if by_cpu { labels::percent(tp.cpu) } else { soyuz_core::format_size(tp.mem, style) };
                let vg = painter.layout_no_wrap(v, font::numbers(11.0), p.text_primary);
                let ng = crate::ui::widgets::trunc_galley(
                    &painter,
                    &tp.name,
                    font::ui(11.5),
                    p.text_secondary,
                    w - vg.size().x - 10.0,
                );
                painter.galley(pos2(x, y), ng, p.text_secondary);
                painter.galley(pos2(x + w - vg.size().x, y), vg, p.text_primary);
                y += font::grow(17.0);
            }
        }
    } else if pinned && y + 30.0 < r.bottom() {
        y += 10.0;
        painter.text(pos2(x, y), Align2::LEFT_TOP, tr!("monitor-gui-no-tops"), font::ui(11.0), p.text_tertiary);
    }
}
