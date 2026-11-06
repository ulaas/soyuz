//! The CPU page: the big load chart (total, user, system), the machine facts, and the
//! per-logical-CPU view (small graphs, a heat grid or bars).

use egui::{Align2, Id, Rect, ScrollArea, Sense, Ui, UiBuilder, pos2, vec2};
use soyuz_core::config::monitor::CoreView;
use soyuz_sysmon::Metric;

use super::charts::{Chart, Hue, Series, Unit, heat_tile, hue, meter};
use super::{PageCtx, card, card_title, fact, grid_cols, segmented_text};
use crate::models::monitor::{MonitorCmd, labels};
use crate::theme::{font, icons::ph};

const GAP: f32 = 12.0;

pub fn show(ui: &mut Ui, rect: Rect, c: &mut PageCtx<'_>) {
    let p = c.p.clone();
    let Some(frame) = c.frame.clone() else {
        super::empty(ui, rect, ph::CPU, tr!("monitor-no-data"), tr!("monitor-gui-waiting"));
        return;
    };
    let info = c.info.clone();
    let fmt = c.fmt();
    let span = c.span();
    let cfg = c.cfg().clone();
    let total = c.series(&Metric::CpuTotal);
    let user = c.series(&Metric::CpuUser);
    let system = c.series(&Metric::CpuSystem);
    let ncpu = frame.ncpu() as usize;
    let cores: Vec<_> = if cfg.core_view == CoreView::Graphs {
        (0..ncpu.min(256)).map(|i| c.series(&Metric::Core(i as u16))).collect()
    } else {
        Vec::new()
    };
    let pane = c.pane.0;
    let dash = PageCtx::dash().to_string();
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    ScrollArea::vertical().id_salt(("soyuz.monitor.cpu", pane)).auto_shrink([false, false]).show(&mut child, |ui| {
        ui.add_space(4.0);
        let w = ui.available_width();
        // ---- the big chart ----
        let ch = font::grow(250.0);
        let (r, _) = ui.allocate_exact_size(vec2(w, ch), Sense::hover());
        card(ui, &p, r, Id::unique(("soyuz.monitor.cpu.main", pane)), false);
        let painter = ui.painter_at(r);
        let y = card_title(
            &painter,
            &p,
            r,
            ph::CPU,
            hue(&p, Hue::Cpu),
            if info.cpu_model.is_empty() {
                labels::page_title(soyuz_core::config::monitor::MonitorPage::Cpu)
            } else {
                &info.cpu_model
            },
        );
        super::big_number(&painter, &p, pos2(r.left() + 14.0, y), &labels::percent(frame.cpu.percent), "", 200.0);
        let plot =
            Rect::from_min_max(pos2(r.left() + 12.0, y + font::grow(40.0)), pos2(r.right() - 12.0, r.bottom() - 10.0));
        let series = [
            Series::new(tr!("monitor-gui-total"), hue(&p, Hue::Cpu), &total),
            Series::new(
                tr!("monitor-gui-user"),
                crate::theme::tokens::mix(hue(&p, Hue::Cpu), p.text_primary, 0.35),
                &user,
            )
            .line(),
            Series::new(tr!("monitor-gui-system"), hue(&p, Hue::Memory), &system).line(),
        ];
        Chart::new(span, Unit::Percent, fmt).y_max(100.0).show(
            ui,
            plot,
            Id::unique(("soyuz.monitor.cpu.chart", pane)),
            &p,
            &series,
        );
        ui.add_space(GAP);

        // ---- facts ----
        let f = &frame.cpu;
        let facts: Vec<(&str, String)> = vec![
            (
                tr!("monitor-gui-cores"),
                if info.logical_cores > 0 {
                    tr!("monitor-gui-cores-value", physical = info.physical_cores, logical = info.logical_cores)
                } else {
                    labels::count(ncpu as u64)
                },
            ),
            (tr!("monitor-gui-sockets"), if info.sockets > 0 { info.sockets.to_string() } else { dash.clone() }),
            (
                tr!("monitor-gui-clock"),
                f.mhz_avg.map_or(dash.clone(), |m| super::charts::fmt_value(Unit::Mhz, m as f32, fmt)),
            ),
            (
                tr!("monitor-gui-max-clock"),
                info.cpu_max_mhz.map_or(dash.clone(), |m| super::charts::fmt_value(Unit::Mhz, m as f32, fmt)),
            ),
            (tr!("monitor-gui-user"), labels::percent(f.user)),
            (tr!("monitor-gui-system"), labels::percent(f.system)),
            (tr!("monitor-gui-iowait"), if cfg!(windows) { dash.clone() } else { labels::percent(f.iowait) }),
            (tr!("monitor-gui-irq"), labels::percent(f.irq)),
            (
                tr!("monitor-gui-load"),
                frame.load.map_or(dash.clone(), |l| format!("{:.2}  {:.2}  {:.2}", l[0], l[1], l[2])),
            ),
            (
                tr!("monitor-gui-ctx-switches"),
                f.ctx_per_sec
                    .map_or(dash.clone(), |v| tr!("monitor-gui-per-sec", value = labels::count(v as u64).as_str())),
            ),
            (
                tr!("monitor-gui-interrupts"),
                f.irq_per_sec
                    .map_or(dash.clone(), |v| tr!("monitor-gui-per-sec", value = labels::count(v as u64).as_str())),
            ),
            (
                tr!("monitor-gui-temperature"),
                f.temp_c.map_or(dash.clone(), |t| labels::temperature(t, cfg.temperature_unit)),
            ),
            (
                tr!("monitor-gui-cache-l1"),
                info.cache_l1_kb.map_or(dash.clone(), |k| {
                    soyuz_core::format_size(u64::from(k) * 1024, soyuz_core::SizeStyle::Binary)
                }),
            ),
            (
                tr!("monitor-gui-cache-l2"),
                info.cache_l2_kb.map_or(dash.clone(), |k| {
                    soyuz_core::format_size(u64::from(k) * 1024, soyuz_core::SizeStyle::Binary)
                }),
            ),
            (
                tr!("monitor-gui-cache-l3"),
                info.cache_l3_kb.map_or(dash.clone(), |k| {
                    soyuz_core::format_size(u64::from(k) * 1024, soyuz_core::SizeStyle::Binary)
                }),
            ),
            (tr!("monitor-gui-processes"), labels::count(u64::from(frame.counts.processes))),
            (tr!("monitor-gui-threads"), labels::count(frame.counts.threads)),
            (tr!("monitor-gui-handles"), frame.counts.handles.map_or(dash.clone(), labels::count)),
            (tr!("monitor-gui-running"), labels::count(u64::from(frame.counts.running))),
            (tr!("monitor-gui-uptime"), labels::uptime(frame.uptime_secs)),
        ];
        let (cols, cw) = grid_cols(w - 28.0, font::grow(210.0), 20.0);
        let rows = facts.len().div_ceil(cols);
        let fh = 16.0 + rows as f32 * font::grow(19.0);
        let (r, _) = ui.allocate_exact_size(vec2(w, fh), Sense::hover());
        card(ui, &p, r, Id::unique(("soyuz.monitor.cpu.facts", pane)), false);
        let painter = ui.painter_at(r);
        for (i, (l, v)) in facts.iter().enumerate() {
            let x = r.left() + 14.0 + (i % cols) as f32 * (cw + 20.0);
            let y = r.top() + 8.0 + (i / cols) as f32 * font::grow(19.0);
            fact(&painter, &p, x, y, cw, l, v);
        }
        ui.add_space(GAP);

        // ---- logical CPUs ----
        let views = [CoreView::Graphs, CoreView::Heat, CoreView::Bars];
        let titles = [tr!("monitor-gui-core-graphs"), tr!("monitor-gui-core-heat"), tr!("monitor-gui-core-bars")];
        let sel = views.iter().position(|v| *v == cfg.core_view).unwrap_or(0);
        let seg_w = super::segmented_width(ui, &titles);
        let (hr, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
        ui.painter().text(
            pos2(hr.left() + 2.0, hr.center().y),
            Align2::LEFT_CENTER,
            tr!("monitor-gui-logical-cpus", count = ncpu),
            crate::theme::font::semibold(12.5),
            p.text_primary,
        );
        if let Some(i) = segmented_text(
            ui,
            Rect::from_min_size(pos2(hr.right() - seg_w, hr.top() + 1.0), vec2(seg_w, 28.0)),
            Id::unique(("soyuz.monitor.cpu.coreview", pane)),
            &titles,
            sel,
        ) {
            c.cmd(MonitorCmd::SetCoreView(views[i]));
        }
        ui.add_space(6.0);
        match cfg.core_view {
            CoreView::Graphs => {
                let (cols, cw) = grid_cols(w, font::grow(150.0), GAP);
                let chh = font::grow(64.0);
                for (ri, chunk) in frame.cpu.cores.iter().enumerate().collect::<Vec<_>>().chunks(cols).enumerate() {
                    let (r, _) = ui.allocate_exact_size(vec2(w, chh), Sense::hover());
                    for (k, (i, core)) in chunk.iter().enumerate() {
                        let cr = Rect::from_min_size(pos2(r.left() + k as f32 * (cw + GAP), r.top()), vec2(cw, chh));
                        card(ui, &p, cr, Id::unique(("soyuz.monitor.cpu.core", pane, *i)), false);
                        let painter = ui.painter_at(cr);
                        painter.text(
                            pos2(cr.left() + 10.0, cr.top() + 11.0),
                            Align2::LEFT_CENTER,
                            format!("CPU {i}"),
                            font::ui(10.5),
                            p.text_tertiary,
                        );
                        let right = match core.mhz {
                            Some(m) => format!(
                                "{}  {}",
                                labels::percent(core.percent),
                                super::charts::fmt_value(Unit::Mhz, m as f32, fmt)
                            ),
                            None => labels::percent(core.percent),
                        };
                        painter.text(
                            pos2(cr.right() - 10.0, cr.top() + 11.0),
                            Align2::RIGHT_CENTER,
                            right,
                            font::numbers(10.5),
                            p.text_secondary,
                        );
                        if let Some(pts) = cores.get(*i) {
                            let plot = Rect::from_min_max(
                                pos2(cr.left() + 8.0, cr.top() + 22.0),
                                pos2(cr.right() - 8.0, cr.bottom() - 6.0),
                            );
                            let s = [Series::new(format!("CPU {i}"), hue(&p, Hue::Cpu), pts)];
                            Chart::mini(span, Unit::Percent, fmt).y_max(100.0).show(
                                ui,
                                plot,
                                Id::unique(("soyuz.monitor.cpu.core.chart", pane, *i)),
                                &p,
                                &s,
                            );
                        }
                    }
                    let _ = ri;
                    ui.add_space(GAP);
                }
            }
            CoreView::Heat => {
                let tile = font::grow(if ncpu > 64 { 34.0 } else { 46.0 });
                let (cols, _) = grid_cols(w, tile, 4.0);
                let rows = ncpu.div_ceil(cols.max(1));
                let (r, _) = ui.allocate_exact_size(vec2(w, rows as f32 * (tile + 4.0)), Sense::hover());
                let painter = ui.painter_at(r);
                for (i, core) in frame.cpu.cores.iter().enumerate() {
                    let tr_ = Rect::from_min_size(
                        pos2(r.left() + (i % cols) as f32 * (tile + 4.0), r.top() + (i / cols) as f32 * (tile + 4.0)),
                        vec2(tile, tile),
                    );
                    let t = core.percent / 100.0;
                    let fill = heat_tile(&p, t);
                    painter.rect_filled(tr_, crate::theme::tokens::radius::same(5), fill);
                    let fg = if t > 0.55 { p.on_color(fill) } else { p.text_primary };
                    painter.text(
                        tr_.center() - vec2(0.0, if tile > 40.0 { 6.0 } else { 0.0 }),
                        Align2::CENTER_CENTER,
                        format!("{:.0}", core.percent),
                        font::numbers(if tile > 40.0 { 13.0 } else { 11.0 }),
                        fg,
                    );
                    if tile > 40.0 {
                        painter.text(
                            tr_.center() + vec2(0.0, 10.0),
                            Align2::CENTER_CENTER,
                            format!("{i}"),
                            font::ui(9.0),
                            fg.gamma_multiply(0.7),
                        );
                    }
                    let resp = ui.interact(tr_, Id::unique(("soyuz.monitor.cpu.tile", pane, i)), Sense::hover());
                    crate::ui::a11y::with(ui.ctx(), resp.id, |n| {
                        n.set_role(egui::accesskit::Role::ListItem);
                        n.set_label(format!("CPU {i}: {}", labels::percent(core.percent)).as_str());
                    });
                    let tip = match core.mhz {
                        Some(m) => format!(
                            "CPU {i}: {}  ·  {}",
                            labels::percent(core.percent),
                            super::charts::fmt_value(Unit::Mhz, m as f32, fmt)
                        ),
                        None => format!("CPU {i}: {}", labels::percent(core.percent)),
                    };
                    resp.on_hover_text(tip);
                }
            }
            CoreView::Bars => {
                let (cols, cw) = grid_cols(w, font::grow(260.0), GAP);
                let rh = font::grow(22.0);
                let rows = ncpu.div_ceil(cols.max(1));
                let (r, _) = ui.allocate_exact_size(vec2(w, rows as f32 * rh + 8.0), Sense::hover());
                card(ui, &p, r, Id::unique(("soyuz.monitor.cpu.bars", pane)), false);
                let painter = ui.painter_at(r);
                for (i, core) in frame.cpu.cores.iter().enumerate() {
                    let x = r.left() + 12.0 + (i % cols) as f32 * (cw + GAP);
                    let y = r.top() + 4.0 + (i / cols) as f32 * rh;
                    painter.text(
                        pos2(x, y + rh / 2.0),
                        Align2::LEFT_CENTER,
                        format!("CPU {i}"),
                        font::ui(11.0),
                        p.text_tertiary,
                    );
                    let pct = labels::percent(core.percent);
                    let mhz = core.mhz.map_or(String::new(), |m| super::charts::fmt_value(Unit::Mhz, m as f32, fmt));
                    let right = if mhz.is_empty() { pct } else { format!("{pct}  {mhz}") };
                    let g = painter.layout_no_wrap(right, font::numbers(11.0), p.text_primary);
                    painter.galley(
                        pos2(x + cw - 24.0 - g.size().x, y + rh / 2.0 - g.size().y / 2.0),
                        g,
                        p.text_primary,
                    );
                    let bar =
                        Rect::from_min_size(pos2(x + 56.0, y + rh / 2.0 - 3.0), vec2(cw - 56.0 - 24.0 - 120.0, 6.0));
                    if bar.width() > 20.0 {
                        meter(&painter, &p, bar, core.percent / 100.0, hue(&p, Hue::Cpu), &[]);
                    }
                }
            }
        }
        ui.add_space(16.0);
    });
}
