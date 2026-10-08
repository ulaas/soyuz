//! Settings → Fonts (`[fonts]`): one card per font role with a family field (suggests
//! installed fonts), an "embedded font" switch, a size offset, a reset button, a live
//! preview in that role's font and a notice when the font is not installed.

use egui::{Align, FontId, Layout, Margin, RichText, Sense, TextEdit, Ui, vec2};
use soyuz_core::config::{Config, FontRoleKind, Fonts, HintingStrength};

use super::widgets::{group, number, segmented, small_button, switch};
use crate::app::SoyuzApp;
use crate::theme::fonts::{self as tfonts, Embedded, Resolved};
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

fn roles() -> [RoleInfo; 6] {
    [
        RoleInfo {
            kind: FontRoleKind::Ui,
            label: tr!("settings-fonts-interface"),
            desc: tr!("settings-fonts-interface-desc"),
            hint: Fonts::DEFAULT_UI,
            sample: tr!("settings-fonts-sample-ui"),
        },
        RoleInfo {
            kind: FontRoleKind::UiSemibold,
            label: tr!("settings-fonts-headings"),
            desc: tr!("settings-fonts-headings-desc"),
            hint: tr!("settings-fonts-headings-hint"),
            sample: tr!("settings-fonts-sample-headings"),
        },
        RoleInfo {
            kind: FontRoleKind::Terminal,
            label: tr!("settings-section-terminal"),
            desc: tr!("settings-fonts-terminal-desc"),
            hint: Fonts::DEFAULT_MONO,
            sample: "PS C:\\Users\\me> git status  \u{2502} 0O 1lI |{}[]  \u{2713}",
        },
        RoleInfo {
            kind: FontRoleKind::Code,
            label: tr!("settings-fonts-code"),
            desc: tr!("settings-fonts-code-desc"),
            hint: Fonts::DEFAULT_MONO,
            sample: "fn main() { let x = 0x1F; /* 0O 1lI */ println!(\"{x}\"); }",
        },
        RoleInfo {
            kind: FontRoleKind::Hex,
            label: tr!("settings-fonts-hex"),
            desc: tr!("settings-fonts-hex-desc"),
            hint: Fonts::DEFAULT_MONO,
            sample: "00000010  4D 5A 90 00 03 00 00 00  MZ\u{2026}\u{2026}\u{2026}",
        },
        RoleInfo {
            kind: FontRoleKind::Numbers,
            label: tr!("settings-fonts-numbers"),
            desc: tr!("settings-fonts-numbers-desc"),
            hint: tr!("settings-fonts-numbers-hint"),
            sample: "1,234,567  \u{b7}  111.1 MiB  \u{b7}  888.8 KiB  \u{b7}  2026-10-08 14:32  \u{b7}  98 %",
        },
    ]
}

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
    let roles = roles();
    let base: Vec<f32> = roles.iter().map(|r| base_size(d, r.kind)).collect();
    for (title, range) in [
        (tr!("settings-fonts-interface"), 0..2),
        (tr!("settings-fonts-monospace"), 2..5),
        (tr!("settings-fonts-numbers"), 5..6),
    ] {
        any |= group(ui, q, ctx, title, |r| {
            for (i, info) in roles[range.clone()].iter().enumerate() {
                let size = base[range.start + i];
                r.block(info.label, info.desc, |ui| role_editor(ui, &p, &mut d.fonts, info, size, &status));
            }
        });
    }
    any |= group(ui, q, ctx, tr!("settings-fonts-rendering"), |r| {
        r.row_kw(
            tr!("settings-fonts-hinting"),
            tr!("settings-fonts-hinting-desc"),
            "sharp crisp cleartype grid",
            |ui| {
                switch(ui, &mut d.fonts.hinting);
            },
        );
        r.row_kw(
            tr!("settings-fonts-subpixel"),
            tr!("settings-fonts-subpixel-desc"),
            "subpixel binning blur spacing kerning",
            |ui| {
                switch(ui, &mut d.fonts.subpixel_positioning);
            },
        );
        r.row_kw(
            tr!("settings-fonts-hinting-strength"),
            tr!("settings-fonts-hinting-strength-desc"),
            "sharp crisp blurry fuzzy grid fit",
            |ui| {
                ui.add_enabled_ui(d.fonts.hinting, |ui| {
                    segmented(
                        ui,
                        &mut d.fonts.hinting_strength,
                        &[
                            (HintingStrength::Crisp, tr!("settings-fonts-hinting-crisp")),
                            (HintingStrength::Normal, tr!("settings-fonts-hinting-normal")),
                            (HintingStrength::Light, tr!("settings-fonts-hinting-light")),
                        ],
                    );
                });
            },
        );
        r.row_kw(
            tr!("settings-fonts-gamma-dark"),
            tr!("settings-fonts-gamma-desc"),
            "weight bold thin contrast gamma dark",
            |ui| gamma_slider(ui, &mut d.fonts.text_gamma_dark),
        );
        r.row_kw(
            tr!("settings-fonts-gamma-light"),
            tr!("settings-fonts-gamma-desc"),
            "weight bold thin contrast gamma light",
            |ui| gamma_slider(ui, &mut d.fonts.text_gamma_light),
        );
        r.row_kw(tr!("settings-fonts-reset-all"), tr!("settings-fonts-reset-all-desc"), "default restore", |ui| {
            if small_button(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), tr!("common-reset"), false).clicked() {
                d.fonts = Fonts::default();
            }
        });
    });
    any
}

/// `text_gamma_*` slider: 0 = egui's curve for the theme ("auto"), else 0.3 … 1.5.
fn gamma_slider(ui: &mut Ui, value: &mut f32) {
    let auto = tr!("settings-automatic").to_string();
    let s = egui::Slider::new(value, 0.0..=1.5)
        .step_by(0.05)
        .trailing_fill(true)
        .custom_formatter(move |v, _| if v < 0.25 { auto.clone() } else { format!("{v:.2}") });
    ui.add_sized([220.0, 20.0], s);
    if *value > 0.0 && *value < 0.25 {
        *value = 0.0;
    }
}

fn role_editor(ui: &mut Ui, p: &Palette, fonts: &mut Fonts, info: &RoleInfo, base: f32, status: &[Resolved]) {
    let kind = info.kind;
    let default = Fonts::default().role(kind).clone();
    let role = fonts.role_mut(kind);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add_enabled_ui(!role.embedded, |ui| family_field(ui, p, &mut role.family, info.hint, kind));
        ui.label(RichText::new(tr!("settings-fonts-embedded")).size(12.0).color(p.text_secondary));
        switch(ui, &mut role.embedded);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let changed = *role != default;
            let reset = ui.add_enabled_ui(changed, |ui| small_button(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), "", false));
            if reset.inner.on_hover_text(tr!("settings-fonts-reset-one")).clicked() {
                *role = default.clone();
            }
            number(ui, &mut role.size_offset, -4.0..=8.0, " pt").on_hover_text(tr!("settings-fonts-offset"));
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
                    ui.label(
                        RichText::new(tr!("settings-fonts-using", font = r.source.describe()))
                            .size(11.5)
                            .color(p.text_tertiary),
                    );
                }
            }
        });
    }
}

/// Family text field with suggestions (embedded fonts first, then installed ones).
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
        let installed = sysfonts::catalog_ready().map(|c| c.names()).unwrap_or_default();
        let names = suggestions(&installed, family, kind);
        if !names.is_empty() {
            let area = egui::Area::new(id.with("popup"))
                .order(egui::Order::Foreground)
                .fixed_pos(resp.rect.left_bottom() + vec2(0.0, 2.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).inner_margin(Margin::same(4)).show(ui, |ui| {
                        ui.set_width(resp.rect.width().max(260.0));
                        for s in names {
                            let (r, sresp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
                            if sresp.hovered() {
                                ui.painter().rect_filled(r, egui::CornerRadius::same(4), p.bg_surface2);
                            }
                            ui.painter().text(
                                r.left_center() + vec2(8.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                &s.name,
                                FontId::proportional(12.5),
                                p.text_primary,
                            );
                            if s.embedded {
                                ui.painter().text(
                                    r.right_center() - vec2(8.0, 0.0),
                                    egui::Align2::RIGHT_CENTER,
                                    tr!("settings-fonts-embedded"),
                                    FontId::proportional(11.0),
                                    p.text_tertiary,
                                );
                            }
                            if sresp.clicked() {
                                *family = s.name.clone();
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

/// One entry of the font suggestion list.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub name: String,
    /// Compiled into Soyuz (always available).
    pub embedded: bool,
}

/// Embedded faces offered for a role: monospace fonts for the monospace roles, every
/// weight for headings, the regular faces otherwise.
fn embedded_for(kind: FontRoleKind) -> impl Iterator<Item = &'static str> {
    Embedded::all()
        .map(Embedded::font)
        .filter(move |f| match kind {
            k if k.is_mono() => f.mono,
            FontRoleKind::UiSemibold => !f.mono,
            _ => !f.mono && f.name == f.family,
        })
        .map(|f| f.name)
}

/// Suggestions for a family field: the embedded fonts of the role, then up to 8
/// installed names containing `typed` (case-insensitive; with nothing typed, a few
/// sensible system picks). Empty when `typed` already names exactly one font.
pub fn suggestions(installed: &[String], typed: &str, kind: FontRoleKind) -> Vec<Suggestion> {
    let t = typed.trim().to_lowercase();
    let mut v: Vec<Suggestion> = embedded_for(kind)
        .filter(|n| t.is_empty() || n.to_lowercase().contains(&t))
        .map(|n| Suggestion { name: n.to_string(), embedded: true })
        .collect();
    let dup = |v: &[Suggestion], n: &str| v.iter().any(|s| sysfonts::norm(&s.name) == sysfonts::norm(n));
    let mut system: Vec<String> = if t.is_empty() {
        let picks: &[&str] = if kind.is_mono() {
            &["Consolas", "Cascadia Code", "Lucida Console", "Courier New"]
        } else {
            &["Segoe UI", "Segoe UI Variable", "Arial", "Calibri", "Tahoma", "Verdana"]
        };
        picks.iter().filter(|p| installed.iter().any(|n| n.eq_ignore_ascii_case(p))).map(|p| p.to_string()).collect()
    } else {
        let mut s: Vec<String> = installed.iter().filter(|n| n.to_lowercase().contains(&t)).cloned().collect();
        s.sort_by_key(|n| !n.to_lowercase().starts_with(&t));
        s
    };
    system.retain(|n| !dup(&v, n));
    system.truncate(8);
    v.extend(system.into_iter().map(|name| Suggestion { name, embedded: false }));
    if v.len() == 1 && v[0].name.eq_ignore_ascii_case(typed.trim()) {
        return Vec::new(); // already complete
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: Vec<Suggestion>) -> Vec<String> {
        v.into_iter().map(|s| if s.embedded { format!("{}*", s.name) } else { s.name }).collect()
    }

    #[test]
    fn suggestions_filter_and_rank() {
        let installed: Vec<String> =
            ["Arial", "Cascadia Code", "Cascadia Mono", "Consolas", "Segoe UI", "Segoe UI Semibold", "Lucida Console"]
                .map(String::from)
                .to_vec();
        let mono = FontRoleKind::Terminal;
        assert_eq!(
            names(suggestions(&installed, "", mono)),
            [
                "Cascadia Mono*",
                "JetBrains Mono*",
                "IBM Plex Mono*",
                "Source Code Pro*",
                "Consolas",
                "Cascadia Code",
                "Lucida Console"
            ]
        );
        assert_eq!(
            names(suggestions(&installed, "", FontRoleKind::Ui)),
            ["Inter*", "IBM Plex Sans*", "Noto Sans*", "Segoe UI", "Arial"]
        );
        assert!(names(suggestions(&installed, "", FontRoleKind::UiSemibold)).contains(&"Inter SemiBold*".to_string()));
        assert_eq!(names(suggestions(&installed, "cons", mono)), ["Consolas", "Lucida Console"]);
        assert_eq!(
            names(suggestions(&installed, "cascadia", mono)),
            ["Cascadia Mono*", "Cascadia Code"],
            "the installed Cascadia Mono is the embedded one"
        );
        assert_eq!(names(suggestions(&installed, "segoe", FontRoleKind::Ui)), ["Segoe UI", "Segoe UI Semibold"]);
        assert!(suggestions(&installed, "Consolas", mono).is_empty(), "complete names hide the list");
        assert!(suggestions(&installed, "ibm plex mono", mono).is_empty());
    }
}
