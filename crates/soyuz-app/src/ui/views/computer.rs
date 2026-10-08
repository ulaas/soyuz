//! "This PC": volume cards with free-space bars.

use egui::{Align2, CornerRadius, FontId, Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};
use soyuz_core::{Location, SizeStyle, format_size};
use soyuz_fs::{VolumeInfo, VolumeKind};

use super::ViewCtx;
use super::flow::semibold;
use crate::cmd::Cmd;
use crate::state::pane::Pane;
use crate::theme::tokens::radius;
use crate::theme::{icons as ticons, icons::ph, palette};
use crate::ui::widgets::{trunc_galley, usage_bar};

pub fn volume_glyph(v: &VolumeInfo) -> &'static str {
    match v.kind {
        VolumeKind::Removable => ph::USB,
        VolumeKind::Network => ph::GLOBE,
        VolumeKind::Optical => ph::DISC,
        _ => ph::HARD_DRIVE,
    }
}

/// Localized kind name for a volume without its own label ("Local Disk").
pub fn volume_kind_label(kind: VolumeKind) -> &'static str {
    match kind {
        VolumeKind::Fixed => tr!("views-volume-fixed"),
        VolumeKind::Removable => tr!("views-volume-removable"),
        VolumeKind::Network => tr!("views-volume-network"),
        VolumeKind::Optical => tr!("views-volume-optical"),
        VolumeKind::Ram => tr!("views-volume-ram"),
        VolumeKind::Unknown => tr!("views-computer-drive"),
    }
}

/// Display name of a volume: its own label, else the localized kind name, followed by
/// the mount (`Local Disk (C:)`, `Yerel Disk (C:)`, `Backup (D:)`).
pub fn volume_label(v: &VolumeInfo) -> String {
    let mount = soyuz_fs::strip_verbatim(&v.mount).to_string_lossy().trim_end_matches(['\\', '/']).to_string();
    let mount = if mount.is_empty() { "/".to_string() } else { mount };
    let base = if v.label.is_empty() { volume_kind_label(v.kind).to_string() } else { v.label.clone() };
    if base.contains(&mount) { base } else { format!("{base} ({mount})") }
}

pub fn show(ui: &mut Ui, rect: Rect, pane: &mut Pane, v: &mut ViewCtx<'_>) {
    let p = palette(ui.ctx());
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(20.0, 16.0))));
    ScrollArea::vertical().id_salt(("soyuz.computer", pane.id.0)).auto_shrink([false, false]).show(&mut child, |ui| {
        ui.label(egui::RichText::new(tr!("views-computer-title")).font(semibold(15.0)).color(p.text_primary));
        ui.add_space(12.0);
        let card = vec2(280.0, 78.0);
        let cols = ((ui.available_width() + 12.0) / (card.x + 12.0)).floor().max(1.0) as usize;
        for row in v.volumes.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for vol in row {
                    let (r, resp) = ui.allocate_exact_size(card, Sense::click());
                    let hover = resp.hovered();
                    let fill = if hover { p.bg_surface2 } else { p.bg_window };
                    ui.painter().rect_filled(r, CornerRadius::same(radius::CARD), fill);
                    ui.painter().rect_stroke(
                        r,
                        CornerRadius::same(radius::CARD),
                        Stroke::new(1.0, if hover { p.border_strong } else { p.border_subtle }),
                        StrokeKind::Inside,
                    );
                    let icon_r = Rect::from_min_size(r.min + vec2(14.0, 17.0), vec2(44.0, 44.0));
                    ui.painter().rect_filled(icon_r, CornerRadius::same(10), p.info.gamma_multiply(0.12));
                    ui.painter().text(
                        icon_r.center(),
                        Align2::CENTER_CENTER,
                        volume_glyph(vol),
                        ticons::font_fill(24.0),
                        p.info,
                    );
                    let tx = icon_r.right() + 12.0;
                    let label = trunc_galley(
                        ui.painter(),
                        &volume_label(vol),
                        semibold(13.0),
                        p.text_primary,
                        r.right() - tx - 12.0,
                    );
                    ui.painter().galley(pos2(tx, r.top() + 14.0), label, p.text_primary);
                    if vol.total > 0 {
                        let used = 1.0 - vol.free as f32 / vol.total as f32;
                        let bar = Rect::from_min_size(pos2(tx, r.top() + 38.0), vec2(r.right() - tx - 14.0, 5.0));
                        usage_bar(ui, &p, bar, used);
                        let text = tr!(
                            "views-computer-free-of",
                            free = format_size(vol.free, SizeStyle::Decimal),
                            total = format_size(vol.total, SizeStyle::Decimal)
                        );
                        ui.painter().text(
                            pos2(tx, r.top() + 56.0),
                            Align2::LEFT_CENTER,
                            text,
                            FontId::proportional(11.5),
                            p.text_secondary,
                        );
                    } else {
                        let kind =
                            if vol.fs.is_empty() { tr!("views-computer-drive").to_string() } else { vol.fs.clone() };
                        ui.painter().text(
                            pos2(tx, r.top() + 46.0),
                            Align2::LEFT_CENTER,
                            kind,
                            FontId::proportional(11.5),
                            p.text_secondary,
                        );
                    }
                    if resp.clicked() {
                        v.cmds.push(Cmd::FocusPane(pane.id));
                        v.cmds.push(Cmd::Navigate {
                            pane: pane.id,
                            location: Location::Dir(vol.mount.clone()),
                            select: None,
                        });
                    }
                }
            });
            ui.add_space(12.0);
        }
        if v.volumes.is_empty() {
            ui.label(egui::RichText::new(tr!("views-computer-looking")).color(p.text_tertiary));
        }
    });
}
