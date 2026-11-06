//! The Memory page: the used chart, the composition bar (only what the OS reports), swap,
//! commit charge and page rates.

use egui::{Align2, Id, Rect, ScrollArea, Sense, Ui, UiBuilder, pos2, vec2};
use soyuz_sysmon::Metric;

use super::charts::{Chart, Hue, Series, Unit, composition, hue, meter};
use super::{PageCtx, card, card_title, fact, grid_cols};
use crate::models::monitor::labels;
use crate::theme::{font, icons::ph};

const GAP: f32 = 12.0;

pub fn show(ui: &mut Ui, rect: Rect, c: &mut PageCtx<'_>) {
    let p = c.p.clone();
    let Some(frame) = c.frame.clone() else {
        super::empty(ui, rect, ph::MEMORY, tr!("monitor-no-data"), tr!("monitor-gui-waiting"));
        return;
    };
    let fmt = c.fmt();
    let style = c.style();
    let span = c.span();
    let used = c.series(&Metric::MemUsed);
    let swap = c.series(&Metric::SwapPercent);
    let pane = c.pane.0;
    let dash = PageCtx::dash().to_string();
    let m = &frame.memory;
    let raw = &m.raw;
    let size = |b: u64| soyuz_core::format_size(b, style);
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    ScrollArea::vertical().id_salt(("soyuz.monitor.memory", pane)).auto_shrink([false, false]).show(&mut child, |ui| {
        ui.add_space(4.0);
        let w = ui.available_width();
        // ---- used chart ----
        let ch = font::grow(250.0);
        let (r, _) = ui.allocate_exact_size(vec2(w, ch), Sense::hover());
        card(ui, &p, r, Id::unique(("soyuz.monitor.mem.main", pane)), false);
        let painter = ui.painter_at(r);
        let y = card_title(&painter, &p, r, ph::MEMORY, hue(&p, Hue::Memory), tr!("monitor-gui-mem-in-use"));
        super::big_number(
            &painter,
            &p,
            pos2(r.left() + 14.0, y),
            &size(m.used),
            &tr!(
                "monitor-gui-mem-sub",
                percent = labels::percent(m.percent).as_str(),
                total = size(raw.total).as_str()
            ),
            r.width() * 0.6,
        );
        let plot =
            Rect::from_min_max(pos2(r.left() + 12.0, y + font::grow(50.0)), pos2(r.right() - 12.0, r.bottom() - 10.0));
        let s = [Series::new(tr!("monitor-gui-mem-in-use"), hue(&p, Hue::Memory), &used)];
        Chart::new(span, Unit::Bytes, fmt).y_max(raw.total as f32).show(
            ui,
            plot,
            Id::unique(("soyuz.monitor.mem.chart", pane)),
            &p,
            &s,
        );
        ui.add_space(GAP);

        // ---- composition ----
        let total = raw.total.max(1) as f32;
        let mut parts: Vec<(f32, egui::Color32, &str, u64)> = Vec::new();
        let mem_c = hue(&p, Hue::Memory);
        // "In use" is what is not available; the cache and buffers sit inside the available
        // part on every OS, so they are capped to it and never push the bar past 100 %.
        let mut rest = raw.total.saturating_sub(m.used);
        parts.push((m.used as f32 / total, mem_c, tr!("monitor-gui-mem-in-use"), m.used));
        if let Some(v) = raw.cached {
            let v = v.min(rest);
            rest -= v;
            parts.push((v as f32 / total, mem_c.gamma_multiply(0.55), tr!("monitor-gui-mem-cached"), v));
        }
        if let Some(v) = raw.buffers {
            let v = v.min(rest);
            rest -= v;
            parts.push((v as f32 / total, mem_c.gamma_multiply(0.35), tr!("monitor-gui-mem-buffers"), v));
        }
        parts.push((rest as f32 / total, p.border_strong.gamma_multiply(0.5), tr!("monitor-gui-mem-free"), rest));
        let (lcols, lcw) = grid_cols(w - 28.0, font::grow(150.0), 12.0);
        let lrows = parts.len().div_ceil(lcols);
        let comp_h = 14.0 + 22.0 + 14.0 + 10.0 + lrows as f32 * font::grow(20.0) + 8.0;
        let (r, _) = ui.allocate_exact_size(vec2(w, comp_h), Sense::hover());
        card(ui, &p, r, Id::unique(("soyuz.monitor.mem.comp", pane)), false);
        let painter = ui.painter_at(r);
        let y = card_title(&painter, &p, r, ph::STACK, mem_c, tr!("monitor-gui-mem-composition"));
        let bar = Rect::from_min_size(pos2(r.left() + 14.0, y + 2.0), vec2(r.width() - 28.0, 14.0));
        let segs: Vec<(f32, egui::Color32)> = parts.iter().map(|x| (x.0, x.1)).collect();
        composition(&painter, &p, bar, &segs);
        let ly = bar.bottom() + 12.0;
        for (i, (frac, color, label, bytes)) in parts.iter().enumerate() {
            let x = r.left() + 14.0 + (i % lcols) as f32 * (lcw + 12.0);
            let yy = ly + (i / lcols) as f32 * font::grow(20.0);
            painter.rect_filled(
                Rect::from_min_size(pos2(x, yy + 4.0), vec2(8.0, 8.0)),
                crate::theme::tokens::radius::same(2),
                *color,
            );
            let lg = crate::ui::widgets::trunc_galley(&painter, label, font::ui(11.5), p.text_secondary, lcw * 0.5);
            painter.galley(pos2(x + 14.0, yy), lg, p.text_secondary);
            let vg = painter.layout_no_wrap(
                format!("{}  {}", size(*bytes), labels::percent(frac * 100.0)),
                font::numbers(11.5),
                p.text_primary,
            );
            painter.galley(pos2(x + lcw - vg.size().x, yy), vg, p.text_primary);
        }
        ui.add_space(GAP);

        // ---- swap and commit side by side ----
        let two = w >= font::grow(640.0);
        let cw = if two { ((w - GAP) / 2.0).floor() } else { w };
        let h = font::grow(170.0);
        let (r, _) = ui.allocate_exact_size(vec2(w, if two { h } else { h * 2.0 + GAP }), Sense::hover());
        let sr = Rect::from_min_size(r.min, vec2(cw, h));
        let cr = if two {
            Rect::from_min_size(pos2(r.left() + cw + GAP, r.top()), vec2(cw, h))
        } else {
            Rect::from_min_size(pos2(r.left(), r.top() + h + GAP), vec2(cw, h))
        };
        // Swap.
        card(ui, &p, sr, Id::unique(("soyuz.monitor.mem.swap", pane)), false);
        let painter = ui.painter_at(sr);
        let y = card_title(&painter, &p, sr, ph::ARROWS_DOWN_UP, hue(&p, Hue::Swap), tr!("monitor-gui-swap"));
        if raw.swap_total > 0 {
            super::big_number(
                &painter,
                &p,
                pos2(sr.left() + 14.0, y),
                &size(raw.swap_used),
                &tr!(
                    "monitor-gui-mem-sub",
                    percent = labels::percent(m.swap_percent).as_str(),
                    total = size(raw.swap_total).as_str()
                ),
                sr.width() - 28.0,
            );
            let plot = Rect::from_min_max(
                pos2(sr.left() + 12.0, y + font::grow(50.0)),
                pos2(sr.right() - 12.0, sr.bottom() - 10.0),
            );
            let s = [Series::new(tr!("monitor-gui-swap"), hue(&p, Hue::Swap), &swap)];
            Chart::mini(span, Unit::Percent, fmt).y_max(100.0).show(
                ui,
                plot,
                Id::unique(("soyuz.monitor.mem.swap.chart", pane)),
                &p,
                &s,
            );
        } else {
            painter.text(
                pos2(sr.left() + 14.0, y + 10.0),
                Align2::LEFT_TOP,
                tr!("monitor-gui-no-swap"),
                font::ui(12.0),
                p.text_tertiary,
            );
        }
        // Commit and paging.
        card(ui, &p, cr, Id::unique(("soyuz.monitor.mem.commit", pane)), false);
        let painter = ui.painter_at(cr);
        let mut y = card_title(&painter, &p, cr, ph::DATABASE, hue(&p, Hue::Cpu), tr!("monitor-gui-commit"));
        let fw = cr.width() - 28.0;
        match (raw.commit_used, raw.commit_limit) {
            (Some(u), Some(l)) if l > 0 => {
                let bar = Rect::from_min_size(pos2(cr.left() + 14.0, y + 2.0), vec2(fw, 6.0));
                meter(&painter, &p, bar, u as f32 / l as f32, hue(&p, Hue::Cpu), &[]);
                y += 16.0;
                y = fact(
                    &painter,
                    &p,
                    cr.left() + 14.0,
                    y,
                    fw,
                    tr!("monitor-gui-commit-used"),
                    &format!("{}  {}", size(u), labels::percent(u as f32 / l as f32 * 100.0)),
                );
                y = fact(&painter, &p, cr.left() + 14.0, y, fw, tr!("monitor-gui-commit-limit"), &size(l));
            }
            (Some(u), None) => {
                y = fact(&painter, &p, cr.left() + 14.0, y, fw, tr!("monitor-gui-commit-used"), &size(u));
            }
            _ => {
                y = fact(&painter, &p, cr.left() + 14.0, y, fw, tr!("monitor-gui-commit-used"), &dash);
            }
        }
        y = fact(&painter, &p, cr.left() + 14.0, y, fw, tr!("monitor-gui-available"), &size(raw.available));
        y = fact(
            &painter,
            &p,
            cr.left() + 14.0,
            y,
            fw,
            tr!("monitor-gui-page-in"),
            &m.page_in_rate.map_or(dash.clone(), |v| labels::rate(v, style)),
        );
        y = fact(
            &painter,
            &p,
            cr.left() + 14.0,
            y,
            fw,
            tr!("monitor-gui-page-out"),
            &m.page_out_rate.map_or(dash.clone(), |v| labels::rate(v, style)),
        );
        for (label, v) in [
            (tr!("monitor-gui-mem-kernel"), raw.kernel),
            (tr!("monitor-gui-mem-compressed"), raw.compressed),
            (tr!("monitor-gui-mem-shared"), raw.shared),
        ] {
            if let Some(v) = v
                && y + 16.0 < cr.bottom()
            {
                y = fact(&painter, &p, cr.left() + 14.0, y, fw, label, &size(v));
            }
        }
        ui.add_space(16.0);
    });
}
