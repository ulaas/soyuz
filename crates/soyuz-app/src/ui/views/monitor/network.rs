//! The Network page: the totals chart, then one card per interface (received and sent on
//! one axis, link, addresses, MAC, errors) with a toggle for virtual interfaces.

use egui::{Align2, Id, Rect, ScrollArea, Sense, Ui, UiBuilder, pos2, vec2};
use soyuz_sysmon::Metric;

use super::charts::{Chart, Hue, Series, Unit, hue};
use super::{PageCtx, badge, card, card_title, fact};
use crate::models::monitor::{MonitorCmd, labels};
use crate::theme::{font, icons::ph};
use crate::ui::widgets::chip;

const GAP: f32 = 12.0;

pub fn show(ui: &mut Ui, rect: Rect, c: &mut PageCtx<'_>) {
    let p = c.p.clone();
    let Some(frame) = c.frame.clone() else {
        super::empty(ui, rect, ph::NETWORK, tr!("monitor-no-data"), tr!("monitor-gui-waiting"));
        return;
    };
    let fmt = c.fmt();
    let style = c.style();
    let span = c.span();
    let pane = c.pane.0;
    let show_virtual = c.cfg().show_virtual_networks;
    let dash = PageCtx::dash().to_string();
    let rx = c.series(&Metric::NetRx);
    let tx = c.series(&Metric::NetTx);
    let nets: Vec<&soyuz_sysmon::NetFrame> = frame.nets.iter().filter(|n| show_virtual || !n.virtual_if).collect();
    let series: Vec<_> = nets
        .iter()
        .map(|n| (c.series(&Metric::NetRxOf(n.id.clone())), c.series(&Metric::NetTxOf(n.id.clone()))))
        .collect();
    let (rxr, txr) = frame.net_totals();
    let hidden = frame.nets.len() - nets.len();
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    ScrollArea::vertical().id_salt(("soyuz.monitor.network", pane)).auto_shrink([false, false]).show(
        &mut child,
        |ui| {
            ui.add_space(4.0);
            let w = ui.available_width();
            // ---- totals ----
            let ch = font::grow(220.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, ch), Sense::hover());
            card(ui, &p, r, Id::unique(("soyuz.monitor.net.main", pane)), false);
            let painter = ui.painter_at(r);
            let y = card_title(&painter, &p, r, ph::NETWORK, hue(&p, Hue::NetRx), tr!("monitor-gui-net-total"));
            super::big_number(
                &painter,
                &p,
                pos2(r.left() + 14.0, y),
                &labels::rate(rxr + txr, style),
                &tr!(
                    "monitor-gui-net-sub",
                    rx = labels::rate(rxr, style).as_str(),
                    tx = labels::rate(txr, style).as_str()
                ),
                r.width() * 0.6,
            );
            let plot = Rect::from_min_max(
                pos2(r.left() + 12.0, y + font::grow(50.0)),
                pos2(r.right() - 12.0, r.bottom() - 10.0),
            );
            let s = [
                Series::new(tr!("monitor-gui-received"), hue(&p, Hue::NetRx), &rx),
                Series::new(tr!("monitor-gui-sent"), hue(&p, Hue::NetTx), &tx).line(),
            ];
            Chart::new(span, Unit::Rate, fmt).show(ui, plot, Id::unique(("soyuz.monitor.net.chart", pane)), &p, &s);
            ui.add_space(GAP);
            // ---- interfaces ----
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(tr!("monitor-gui-interfaces", count = nets.len()))
                        .font(crate::theme::font::semibold(12.5))
                        .color(p.text_primary),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if hidden > 0 && !show_virtual {
                        tr!("monitor-gui-show-virtual-n", count = hidden)
                    } else {
                        tr!("monitor-gui-show-virtual").to_string()
                    };
                    if chip(ui, Some(ph::EYE), &label, show_virtual).clicked() {
                        c.cmd(MonitorCmd::ToggleVirtualNets);
                    }
                });
            });
            ui.add_space(6.0);
            let two = w >= font::grow(980.0);
            let cw = if two { ((w - GAP) / 2.0).floor() } else { w };
            let ch = font::grow(236.0);
            let per_row = if two { 2 } else { 1 };
            for row in 0..nets.len().div_ceil(per_row) {
                let (r, _) = ui.allocate_exact_size(vec2(w, ch), Sense::hover());
                for k in 0..per_row {
                    let i = row * per_row + k;
                    let Some(n) = nets.get(i) else { break };
                    let cr = Rect::from_min_size(pos2(r.left() + k as f32 * (cw + GAP), r.top()), vec2(cw, ch));
                    card(ui, &p, cr, Id::unique(("soyuz.monitor.net", pane, i)), false);
                    let painter = ui.painter_at(cr);
                    let glyph = if n.virtual_if {
                        ph::ARROWS_LEFT_RIGHT
                    } else if n.description.to_lowercase().contains("wi-fi")
                        || n.name.to_lowercase().contains("wi-fi")
                        || n.name.starts_with("wl")
                    {
                        ph::WIFI_HIGH
                    } else {
                        ph::NETWORK
                    };
                    let color = if n.up { hue(&p, Hue::NetRx) } else { p.text_tertiary };
                    let y = card_title(&painter, &p, cr, glyph, color, &n.name);
                    let mut bx = cr.left() + 14.0;
                    let (state, sc) = if n.up {
                        (tr!("monitor-gui-up"), p.success)
                    } else {
                        (tr!("monitor-gui-down"), p.text_tertiary)
                    };
                    bx += badge(&painter, &p, pos2(bx, y - 2.0), state, sc) + 6.0;
                    if n.virtual_if {
                        bx +=
                            badge(&painter, &p, pos2(bx, y - 2.0), tr!("monitor-gui-virtual"), p.text_secondary) + 6.0;
                    }
                    if let Some(b) = n.link_bps {
                        bx += badge(&painter, &p, pos2(bx, y - 2.0), &labels::bits(b), p.text_secondary) + 6.0;
                    }
                    if !n.description.is_empty() && n.description != n.name {
                        let g = crate::ui::widgets::trunc_galley(
                            &painter,
                            &n.description,
                            font::ui(11.0),
                            p.text_tertiary,
                            cr.right() - bx - 14.0,
                        );
                        painter.galley(pos2(bx + 4.0, y + 6.0 - g.size().y / 2.0), g, p.text_tertiary);
                    }
                    let facts_w = font::grow(210.0);
                    let wide_card = cw >= font::grow(560.0);
                    let plot_right = if wide_card { cr.right() - facts_w - 20.0 } else { cr.right() - 12.0 };
                    let plot =
                        Rect::from_min_max(pos2(cr.left() + 12.0, y + 22.0), pos2(plot_right, cr.bottom() - 10.0));
                    let (rxs, txs) = &series[i];
                    let s = [
                        Series::new(tr!("monitor-gui-received"), hue(&p, Hue::NetRx), rxs),
                        Series::new(tr!("monitor-gui-sent"), hue(&p, Hue::NetTx), txs).line(),
                    ];
                    Chart::new(span, Unit::Rate, fmt).show(
                        ui,
                        plot,
                        Id::unique(("soyuz.monitor.net.chart", pane, i)),
                        &p,
                        &s,
                    );
                    if wide_card {
                        let fx = cr.right() - facts_w - 8.0;
                        let fw = facts_w - 6.0;
                        let mut fy = y + 22.0;
                        fy = fact(
                            &painter,
                            &p,
                            fx,
                            fy,
                            fw,
                            tr!("monitor-gui-received"),
                            &labels::rate(n.rx_rate, style),
                        );
                        fy = fact(&painter, &p, fx, fy, fw, tr!("monitor-gui-sent"), &labels::rate(n.tx_rate, style));
                        fy = fact(
                            &painter,
                            &p,
                            fx,
                            fy,
                            fw,
                            tr!("monitor-gui-rx-total"),
                            &soyuz_core::format_size(n.rx_bytes, style),
                        );
                        fy = fact(
                            &painter,
                            &p,
                            fx,
                            fy,
                            fw,
                            tr!("monitor-gui-tx-total"),
                            &soyuz_core::format_size(n.tx_bytes, style),
                        );
                        fy = fact(
                            &painter,
                            &p,
                            fx,
                            fy,
                            fw,
                            tr!("monitor-gui-packets"),
                            &format!("{:.0} / {:.0}", n.rx_packets_rate, n.tx_packets_rate),
                        );
                        let errs = n.rx_errors + n.tx_errors;
                        fy = fact(&painter, &p, fx, fy, fw, tr!("monitor-gui-errors"), &labels::count(errs));
                        fy = fact(&painter, &p, fx, fy, fw, "MAC", n.mac.as_deref().unwrap_or(&dash));
                        for a in n.addresses.iter().take(3) {
                            if fy + 16.0 > cr.bottom() {
                                break;
                            }
                            let g =
                                crate::ui::widgets::trunc_galley(&painter, a, font::mono(10.5), p.text_secondary, fw);
                            painter.galley(pos2(fx + fw - g.size().x, fy), g, p.text_secondary);
                            fy += font::grow(16.0);
                        }
                    } else {
                        let line =
                            format!("↓ {}   ↑ {}", labels::rate(n.rx_rate, style), labels::rate(n.tx_rate, style));
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
            if nets.is_empty() {
                let (r, _) = ui.allocate_exact_size(vec2(w, 100.0), Sense::hover());
                card(ui, &p, r, Id::unique(("soyuz.monitor.net.none", pane)), false);
                ui.painter().text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    tr!("monitor-gui-no-interfaces"),
                    font::ui(12.5),
                    p.text_tertiary,
                );
            }
            ui.add_space(16.0);
        },
    );
}
