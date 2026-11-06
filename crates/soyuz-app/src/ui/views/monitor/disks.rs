//! The Disks page: one card per physical disk (read and write on one axis, busy, queue,
//! IOPS, capacity) and the mounted volumes.

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
        super::empty(ui, rect, ph::HARD_DRIVES, tr!("monitor-no-data"), tr!("monitor-gui-waiting"));
        return;
    };
    let fmt = c.fmt();
    let style = c.style();
    let span = c.span();
    let pane = c.pane.0;
    let dash = PageCtx::dash().to_string();
    let series: Vec<_> = frame
        .disks
        .iter()
        .map(|d| (c.series(&Metric::DiskReadOf(d.id.clone())), c.series(&Metric::DiskWriteOf(d.id.clone()))))
        .collect();
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    ScrollArea::vertical().id_salt(("soyuz.monitor.disks", pane)).auto_shrink([false, false]).show(&mut child, |ui| {
        ui.add_space(4.0);
        let w = ui.available_width();
        if frame.disks.is_empty() {
            let (r, _) = ui.allocate_exact_size(vec2(w, 120.0), Sense::hover());
            card(ui, &p, r, Id::unique(("soyuz.monitor.disks.none", pane)), false);
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                tr!("monitor-gui-no-disks"),
                font::ui(12.5),
                p.text_tertiary,
            );
            ui.add_space(GAP);
        }
        let two = w >= font::grow(980.0);
        let cw = if two { ((w - GAP) / 2.0).floor() } else { w };
        let ch = font::grow(236.0);
        let n = frame.disks.len();
        let per_row = if two { 2 } else { 1 };
        for row in 0..n.div_ceil(per_row) {
            let (r, _) = ui.allocate_exact_size(vec2(w, ch), Sense::hover());
            for k in 0..per_row {
                let i = row * per_row + k;
                let Some(d) = frame.disks.get(i) else { break };
                let cr = Rect::from_min_size(pos2(r.left() + k as f32 * (cw + GAP), r.top()), vec2(cw, ch));
                card(ui, &p, cr, Id::unique(("soyuz.monitor.disk", pane, i)), false);
                let painter = ui.painter_at(cr);
                let title = if d.model.is_empty() { d.id.clone() } else { d.model.clone() };
                let y = card_title(&painter, &p, cr, ph::HARD_DRIVE, hue(&p, Hue::DiskRead), &title);
                // Badges: id, kind, capacity.
                let mut bx = cr.left() + 14.0;
                if !d.model.is_empty() {
                    bx += badge(&painter, &p, pos2(bx, y - 2.0), &d.id, p.text_secondary) + 6.0;
                }
                if let Some(rot) = d.rotational {
                    bx +=
                        badge(&painter, &p, pos2(bx, y - 2.0), if rot { "HDD" } else { "SSD" }, p.text_secondary) + 6.0;
                }
                if let Some(s) = d.size {
                    badge(
                        &painter,
                        &p,
                        pos2(bx, y - 2.0),
                        &soyuz_core::format_size(s, soyuz_core::SizeStyle::Decimal),
                        p.text_secondary,
                    );
                }
                let facts_w = font::grow(190.0);
                let wide_card = cw >= font::grow(520.0);
                let plot_right = if wide_card { cr.right() - facts_w - 20.0 } else { cr.right() - 12.0 };
                let plot = Rect::from_min_max(pos2(cr.left() + 12.0, y + 22.0), pos2(plot_right, cr.bottom() - 10.0));
                let (rd, wr) = &series[i];
                let s = [
                    Series::new(tr!("monitor-gui-read"), hue(&p, Hue::DiskRead), rd),
                    Series::new(tr!("monitor-gui-write"), hue(&p, Hue::DiskWrite), wr).line(),
                ];
                Chart::new(span, Unit::Rate, fmt).show(
                    ui,
                    plot,
                    Id::unique(("soyuz.monitor.disk.chart", pane, i)),
                    &p,
                    &s,
                );
                if wide_card {
                    let fx = cr.right() - facts_w - 8.0;
                    let mut fy = y + 22.0;
                    let fw = facts_w - 6.0;
                    if let Some(b) = d.busy_percent {
                        let bar = Rect::from_min_size(pos2(fx, fy + 4.0), vec2(fw, 5.0));
                        meter(&painter, &p, bar, b / 100.0, hue(&p, Hue::DiskRead), &[]);
                        fy += 14.0;
                    }
                    fy = fact(
                        &painter,
                        &p,
                        fx,
                        fy,
                        fw,
                        tr!("monitor-gui-busy"),
                        &d.busy_percent.map_or(dash.clone(), labels::percent),
                    );
                    fy = fact(
                        &painter,
                        &p,
                        fx,
                        fy,
                        fw,
                        tr!("monitor-gui-queue"),
                        &d.queue.map_or(dash.clone(), |q| format!("{q:.1}")),
                    );
                    fy = fact(&painter, &p, fx, fy, fw, tr!("monitor-gui-read"), &labels::rate(d.read_rate, style));
                    fy = fact(&painter, &p, fx, fy, fw, tr!("monitor-gui-write"), &labels::rate(d.write_rate, style));
                    fy = fact(
                        &painter,
                        &p,
                        fx,
                        fy,
                        fw,
                        tr!("monitor-gui-iops"),
                        &format!("{:.0} / {:.0}", d.read_iops, d.write_iops),
                    );
                    fy = fact(
                        &painter,
                        &p,
                        fx,
                        fy,
                        fw,
                        tr!("monitor-gui-read-total"),
                        &soyuz_core::format_size(d.read_bytes, style),
                    );
                    fact(
                        &painter,
                        &p,
                        fx,
                        fy,
                        fw,
                        tr!("monitor-gui-write-total"),
                        &soyuz_core::format_size(d.write_bytes, style),
                    );
                } else {
                    // Compact: one line of figures under the title.
                    let line = format!(
                        "{}  ·  {} {}  ·  {} {}",
                        d.busy_percent.map_or(dash.clone(), |b| tr!(
                            "monitor-gui-busy-value",
                            value = labels::percent(b).as_str()
                        )),
                        tr!("monitor-gui-read"),
                        labels::rate(d.read_rate, style),
                        tr!("monitor-gui-write"),
                        labels::rate(d.write_rate, style)
                    );
                    painter.text(
                        pos2(cr.right() - 14.0, y + 6.0),
                        Align2::RIGHT_CENTER,
                        line,
                        font::numbers(11.0),
                        p.text_secondary,
                    );
                }
            }
            ui.add_space(GAP);
        }
        // ---- volumes ----
        let vh = font::grow(40.0) + c.volumes.len().max(1) as f32 * font::grow(34.0) + 8.0;
        let (r, _) = ui.allocate_exact_size(vec2(w, vh.min(font::grow(420.0))), Sense::hover());
        super::overview::volumes_card(ui, r, c, &p);
        ui.add_space(16.0);
    });
}
