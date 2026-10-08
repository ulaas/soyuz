//! Settings → Fonts (`[fonts]`): one card per font role with a family field (suggests
//! installed fonts), an "embedded font" switch, a size offset, a reset button, a live
//! preview in that role's font and a notice when the font is not installed.

use egui::{Align, FontId, Layout, Margin, RichText, Sense, TextEdit, Ui, vec2};
use soyuz_core::config::{Config, FontRoleKind, Fonts};

use super::widgets::{group, number, small_button, switch};
use crate::app::SoyuzApp;
use crate::theme::fonts::{self as tfonts, Resolved};
use crate::theme::icons::ph;
use crate::theme::{Palette, palette, sysfonts};

struct RoleInfo {
    kind: FontRoleKind,
    label: &'static str,
    desc: &'static str,
    /// Placeholder of the family field (what an empty field means).
    hint: &'static str,
    sample: &'static str,
}

const ROLES: [RoleInfo; 6] = [
    RoleInfo {
        kind: FontRoleKind::Ui,
        label: "Interface",
        desc: "Labels, menus, buttons and file names. Default: Segoe UI (falls back to the embedded Inter).",
        hint: Fonts::DEFAULT_UI,
        sample: "The quick brown fox jumps over the lazy dog \u{2014} Documents, Pictures, Ünïcödé",
    },
    RoleInfo {
        kind: FontRoleKind::UiSemibold,
        label: "Headings",
        desc: "Titles, dialog headers and section names. Empty: the semibold face of the interface font.",
        hint: "Semibold of the interface font",
        sample: "Properties \u{b7} Downloads \u{b7} Settings",
    },
    RoleInfo {
        kind: FontRoleKind::Terminal,
        label: "Terminal",
        desc: "The integrated terminal. Default: Consolas, then Cascadia Mono, then the embedded JetBrains Mono.",
        hint: Fonts::DEFAULT_MONO,
        sample: "PS C:\\Users\\me> git status  \u{2502} 0O 1lI |{}[]  \u{2713}",
    },
    RoleInfo {
        kind: FontRoleKind::Code,
        label: "Code editor",
        desc: "The text editor, code previews and other monospace text. Default: Consolas, then Cascadia Code.",
        hint: Fonts::DEFAULT_MONO,
        sample: "fn main() { let x = 0x1F; /* 0O 1lI */ println!(\"{x}\"); }",
    },
    RoleInfo {
        kind: FontRoleKind::Hex,
        label: "Hex view",
        desc: "The hex editor and hex preview.",
        hint: Fonts::DEFAULT_MONO,
        sample: "00000010  4D 5A 90 00 03 00 00 00  MZ\u{2026}\u{2026}\u{2026}",
    },
    RoleInfo {
        kind: FontRoleKind::Numbers,
        label: "Numbers",
        desc: "Sizes, dates and progress figures in the file list and the activity panel. Empty: the interface font (Segoe UI digits are tabular, so columns line up).",
        hint: "Same as the interface font",
        sample: "1,234,567  \u{b7}  111.1 MiB  \u{b7}  888.8 KiB  \u{b7}  2026-10-08 14:32  \u{b7}  98 %",
    },
];

/// Text size the preview of a role uses (the role's base size; offset added later).
fn base_size(c: &Config, kind: FontRoleKind) -> f32 {
    match kind {
        FontRoleKind::Ui => c.appearance.font_size,
        FontRoleKind::UiSemibold => 15.0,
        FontRoleKind::Terminal => c.terminal.font_size,
        FontRoleKind::Code | FontRoleKind::Hex => c.editor.font_size,
        FontRoleKind::Numbers => 12.5,
    }
}

pub fn page(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let p = palette(ui.ctx());
    sysfonts::warm_catalog();
    let status = tfonts::status();
    let d = &mut app.settings.draft;
    let ctx = "Fonts typeface font family size";
    let mut any = false;
    let base: Vec<f32> = ROLES.iter().map(|r| base_size(d, r.kind)).collect();
    for (title, range) in [("Interface", 0..2), ("Monospace", 2..5), ("Numbers", 5..6)] {
        any |= group(ui, q, ctx, title, |r| {
            for (i, info) in ROLES[range.clone()].iter().enumerate() {
                let size = base[range.start + i];
                r.block(info.label, info.desc, |ui| role_editor(ui, &p, &mut d.fonts, info, size, &status));
            }
        });
    }
    any |= group(ui, q, ctx, "Rendering", |r| {
        r.row_kw(
            "Hinting",
            "Snap letter stems to the pixel grid: crisper text at small sizes (Segoe UI and Consolas are hand-hinted).",
            "sharp crisp cleartype grid",
            |ui| {
                switch(ui, &mut d.fonts.hinting);
            },
        );
        r.row_kw(
            "Sub-pixel positioning",
            "Place letters at fractional pixel positions: more even spacing, slightly softer stems.",
            "subpixel binning blur spacing kerning",
            |ui| {
                switch(ui, &mut d.fonts.subpixel_positioning);
            },
        );
        r.row_kw("Reset all fonts", "Back to Segoe UI / Consolas with no size offsets.", "default restore", |ui| {
            if small_button(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), "Reset", false).clicked() {
                d.fonts = Fonts::default();
            }
        });
    });
    any
}

fn role_editor(ui: &mut Ui, p: &Palette, fonts: &mut Fonts, info: &RoleInfo, base: f32, status: &[Resolved]) {
    let kind = info.kind;
    let default = Fonts::default().role(kind).clone();
    let role = fonts.role_mut(kind);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add_enabled_ui(!role.embedded, |ui| family_field(ui, p, &mut role.family, info.hint, kind));
        ui.label(RichText::new("Embedded").size(12.0).color(p.text_secondary));
        switch(ui, &mut role.embedded);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let changed = *role != default;
            let reset = ui.add_enabled_ui(changed, |ui| small_button(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), "", false));
            if reset.inner.on_hover_text("Reset this font").clicked() {
                *role = default.clone();
            }
            number(ui, &mut role.size_offset, -4.0..=8.0, " pt").on_hover_text("Size offset (points)");
        });
    });
    ui.add_space(6.0);
    // Live preview in the installed font of the role, at the draft's size.
    let size = (base + role.offset()).max(6.0);
    let font = FontId::new(size, tfonts::family(kind));
    egui::Frame::new()
        .fill(p.bg_sunken)
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(info.sample).font(font).color(p.text_primary)).truncate());
        });
    if let Some(r) = status.iter().find(|r| r.role == kind) {
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            match &r.notice {
                Some(n) => {
                    ui.label(crate::theme::icons::icon(ph::WARNING, 12.0).color(p.warning));
                    ui.label(RichText::new(n).size(11.5).color(p.warning));
                }
                None => {
                    ui.label(crate::theme::icons::icon(ph::CHECK_CIRCLE, 12.0).color(p.text_tertiary));
                    ui.label(RichText::new(format!("Using {}", r.source.describe())).size(11.5).color(p.text_tertiary));
                }
            }
        });
    }
}

/// Family text field with suggestions from the installed fonts below it.
fn family_field(ui: &mut Ui, p: &Palette, family: &mut String, hint: &str, kind: FontRoleKind) {
    let id = egui::Id::new(("soyuz.settings.font_suggest", kind.key()));
    let resp = ui.add(
        TextEdit::singleline(family)
            .id(id.with("edit"))
            .hint_text(hint)
            .desired_width(220.0)
            .margin(Margin::symmetric(8, 5)),
    );
    let list_hot = ui.ctx().data(|d| d.get_temp::<bool>(id).unwrap_or(false));
    let mut hot = false;
    if resp.has_focus() || list_hot {
        let names =
            sysfonts::catalog_ready().map(|c| suggestions(&c.names(), family, kind.is_mono())).unwrap_or_default();
        if !names.is_empty() {
            let area = egui::Area::new(id.with("popup"))
                .order(egui::Order::Foreground)
                .fixed_pos(resp.rect.left_bottom() + vec2(0.0, 2.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).inner_margin(Margin::same(4)).show(ui, |ui| {
                        ui.set_width(resp.rect.width());
                        for n in names {
                            let (r, sresp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                            if sresp.hovered() {
                                ui.painter().rect_filled(r, egui::CornerRadius::same(4), p.bg_surface2);
                            }
                            ui.painter().text(
                                r.left_center() + vec2(8.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                &n,
                                FontId::proportional(12.5),
                                p.text_primary,
                            );
                            if sresp.clicked() {
                                *family = n.clone();
                            }
                        }
                    });
                });
            hot = area.response.contains_pointer();
            if area.response.clicked_elsewhere() && !resp.has_focus() {
                hot = false;
            }
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, hot));
}

/// Up to 8 installed names containing `typed` (case-insensitive). With nothing typed,
/// a few sensible picks for the role.
pub fn suggestions(names: &[String], typed: &str, mono: bool) -> Vec<String> {
    let t = typed.trim().to_lowercase();
    if t.is_empty() {
        let picks: &[&str] = if mono {
            &["Consolas", "Cascadia Mono", "Cascadia Code", "Lucida Console", "Courier New"]
        } else {
            &["Segoe UI", "Segoe UI Variable", "Arial", "Calibri", "Tahoma", "Verdana"]
        };
        return picks
            .iter()
            .filter(|p| names.iter().any(|n| n.eq_ignore_ascii_case(p)))
            .map(|p| p.to_string())
            .collect();
    }
    let mut v: Vec<String> = names.iter().filter(|n| n.to_lowercase().contains(&t)).cloned().collect();
    v.sort_by_key(|n| !n.to_lowercase().starts_with(&t));
    if v.len() == 1 && v[0].eq_ignore_ascii_case(typed.trim()) {
        return Vec::new(); // already complete
    }
    v.truncate(8);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_filter_and_rank() {
        let names: Vec<String> =
            ["Arial", "Cascadia Code", "Cascadia Mono", "Consolas", "Segoe UI", "Segoe UI Semibold", "Lucida Console"]
                .map(String::from)
                .to_vec();
        assert_eq!(suggestions(&names, "", true), ["Consolas", "Cascadia Mono", "Cascadia Code", "Lucida Console"]);
        assert_eq!(suggestions(&names, "", false), ["Segoe UI", "Arial"]);
        assert_eq!(suggestions(&names, "cons", true), ["Consolas", "Lucida Console"]);
        assert_eq!(suggestions(&names, "segoe", false), ["Segoe UI", "Segoe UI Semibold"]);
        assert!(suggestions(&names, "Consolas", true).is_empty(), "complete names hide the list");
    }
}
