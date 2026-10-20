//! Quick share window: draws `soyuz_appcore::models::quick_share::QuickShare` (the HTTP
//! server, its addresses and QR code, shared with the terminal UI's dialog). The QR code
//! on the left, the address, the items and what happened so far on the right. Closing
//! the window stops sharing.

use std::path::PathBuf;
use std::time::Duration;

use egui::{Align, Color32, FontId, Layout, Rect, RichText, Sense, Ui, vec2};
use soyuz_core::config::RemoteSection;

use super::{button, tool_window};
use crate::app::SoyuzApp;
use crate::cmd::Cmd;
use crate::models::quick_share::{Qr, QuickShare};
use crate::theme::{icons::ph, palette};

/// Side of the drawn code (points, quiet zone included).
const QR_SIDE: f32 = 232.0;
/// Light modules around the code (the QR spec asks for 4).
const QUIET: usize = 4;

pub struct ShareWindow {
    pub open: bool,
    model: QuickShare,
}

impl ShareWindow {
    pub fn new(items: Vec<PathBuf>, cfg: &RemoteSection) -> Self {
        ShareWindow { open: true, model: QuickShare::start(items, cfg) }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut SoyuzApp) {
        let what = self.model.what();
        let keep = tool_window(ctx, "share", tr!("tools-share-title"), &what, ph::QR_CODE, vec2(640.0, 400.0), |ui| {
            self.body(ui, app)
        });
        // The status line counts down and follows downloads.
        ctx.request_repaint_after(Duration::from_secs(1));
        if !keep {
            self.model.stop();
            self.open = false;
        }
    }

    fn body(&mut self, ui: &mut Ui, app: &mut SoyuzApp) {
        let p = palette(ui.ctx());
        let m = &mut self.model;
        let mut close = false;
        ui.horizontal_top(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(QR_SIDE, QR_SIDE), Sense::hover());
            match m.qr() {
                Some(qr) => paint_qr(ui, rect, qr),
                None => {
                    ui.painter().rect_filled(rect, 8.0, p.bg_surface2);
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        ph::QR_CODE,
                        crate::theme::icons::font(64.0),
                        p.text_tertiary,
                    );
                }
            }
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.set_width(ui.available_width());
                let label = |ui: &mut Ui, s: &str| {
                    ui.label(RichText::new(s).font(FontId::proportional(11.5)).color(p.text_tertiary));
                };
                if let Some(problem) = m.problem() {
                    ui.label(RichText::new(problem).color(if m.running() { p.warning } else { p.danger }));
                    ui.add_space(6.0);
                }
                if let Some(url) = m.url().filter(|_| m.running()) {
                    label(ui, tr!("tools-share-address"));
                    ui.add(
                        egui::Label::new(RichText::new(&url).font(FontId::monospace(13.0)).color(p.text_primary))
                            .wrap()
                            .selectable(true),
                    );
                    if m.addresses.len() > 1 {
                        // Other networks this computer is on.
                        ui.horizontal_wrapped(|ui| {
                            for i in 0..m.addresses.len() {
                                let mut on = m.picked == i;
                                if super::toggle_chip(ui, &m.addresses[i].to_string(), &mut on, p.accent).clicked() {
                                    m.pick(i);
                                }
                            }
                        });
                    }
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(tr!("tools-share-intro"))
                            .font(FontId::proportional(12.0))
                            .color(p.text_secondary),
                    );
                    ui.add_space(6.0);
                }
                label(ui, tr!("tools-share-shared"));
                for path in m.items.iter().take(6) {
                    ui.label(RichText::new(soyuz_core::entry::display_name(path)).color(p.text_primary))
                        .on_hover_text(super::tags_ui::show_path(path));
                }
                if m.items.len() > 6 {
                    ui.label(RichText::new(format!("+{}", m.items.len() - 6)).color(p.text_tertiary));
                }
                ui.add_space(6.0);
                ui.label(RichText::new(m.status()).font(FontId::proportional(12.0)).color(p.text_secondary));
            });
        });
        ui.with_layout(Layout::bottom_up(Align::Max), |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if m.running() {
                        if button(ui, Some(ph::STOP), tr!("tools-share-stop"), 0, true).clicked() {
                            close = true;
                        }
                        if let Some(url) = m.url()
                            && button(ui, Some(ph::COPY), tr!("tools-share-copy"), 1, true).clicked()
                        {
                            app.core.cmds.push(Cmd::CopyText(url));
                        }
                    } else if button(ui, Some(ph::CHECK), tr!("tools-done"), 1, true).clicked() {
                        close = true;
                    }
                });
            });
        });
        if close {
            m.stop();
            self.open = false;
        }
    }
}

/// The code as squares: always dark on white (whatever the theme), with its quiet zone.
fn paint_qr(ui: &Ui, rect: Rect, qr: &Qr) {
    let n = qr.width + 2 * QUIET;
    // Whole pixels per module keep the edges crisp.
    let ppp = ui.ctx().pixels_per_point();
    let module = ((rect.width() * ppp / n as f32).floor().max(1.0)) / ppp;
    let side = module * n as f32;
    let origin = rect.center() - vec2(side, side) / 2.0;
    let origin = egui::pos2((origin.x * ppp).round() / ppp, (origin.y * ppp).round() / ppp);
    let painter = ui.painter();
    painter.rect_filled(Rect::from_min_size(origin, vec2(side, side)), 6.0, Color32::WHITE);
    for y in 0..qr.width {
        for x in 0..qr.width {
            if qr.dark(x as isize, y as isize) {
                let min = origin + vec2((x + QUIET) as f32 * module, (y + QUIET) as f32 * module);
                painter.rect_filled(Rect::from_min_size(min, vec2(module, module)), 0.0, Color32::BLACK);
            }
        }
    }
}
