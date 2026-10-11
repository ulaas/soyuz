//! Settings → Fonts (`[fonts]`): one card per font role with a family dropdown (embedded
//! and installed fonts, searchable, "Custom…" for other names / files), an "embedded font" switch, a size offset, a reset button, a live
//! preview in that role's font and a notice when the font is not installed.

use egui::{Align, FontId, Layout, Margin, RichText, Ui};
use soyuz_core::config::{Config, FontRoleKind, Fonts, HintingStrength};

use super::widgets::{dd, group, number, segmented, small_button, switch, text_choice};
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
            desc: if cfg!(target_os = "linux") {
                tr!("settings-fonts-interface-desc-linux")
            } else {
                tr!("settings-fonts-interface-desc")
            },
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
        family_field(ui, &mut role.family, info.hint, kind, role.embedded);
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

/// Family dropdown: the role default, the embedded faces (each drawn in its own font),
/// every installed family (searchable), and "Custom…" for other names or a font file.
fn family_field(ui: &mut Ui, family: &mut String, hint: &str, kind: FontRoleKind, embedded_only: bool) {
    tfonts::enable_previews(ui.ctx());
    let installed = sysfonts::catalog_ready().map(|c| c.names());
    if installed.is_none() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    let choices = font_choices(installed.as_deref().unwrap_or_default(), kind);
    let (g_embedded, g_system) = (tr!("settings-fonts-embedded"), tr!("settings-fonts-system-group"));
    let only = tr!("settings-fonts-embedded-only-reason");
    let mut items: Vec<dd::Item<String>> = vec![
        dd::Item::new(String::new(), tr!("settings-fonts-default"))
            .secondary(hint)
            .font(tfonts::preview_family(ui.ctx(), hint))
            .icon(ph::ARROW_COUNTER_CLOCKWISE),
    ];
    for c in &choices {
        let item = dd::Item::new(c.name.clone(), c.name.clone()).font(tfonts::preview_family(ui.ctx(), &c.name));
        items.push(if c.embedded {
            item.group(g_embedded)
        } else {
            item.group(g_system).disabled(embedded_only.then(|| only.to_string()))
        });
    }
    if installed.is_none() {
        items.push(
            dd::Item::new("?loading".to_string(), tr!("settings-fonts-loading"))
                .group(g_system)
                .disabled(Some(tr!("settings-fonts-loading").to_string())),
        );
    }
    // A name typed in another case / with "Regular" still selects its entry.
    let mut value = items
        .iter()
        .find(|it| !it.value.is_empty() && sysfonts::norm(&it.value) == sysfonts::norm(family))
        .map_or_else(|| family.clone(), |it| it.value.clone());
    let key = format!("font.{}", kind.key());
    if text_choice(ui, &key, &mut value, &items, 230.0, tr!("settings-fonts-custom-hint"))
        && sysfonts::norm(&value) != sysfonts::norm(family)
    {
        *family = value;
    }
}

/// One entry of a font dropdown.
#[derive(Clone, Debug, PartialEq)]
pub struct FontChoice {
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

/// The fonts offered for a role: its embedded faces (in their fixed order), then every
/// installed family that is not one of them, sorted case-insensitively.
pub fn font_choices(installed: &[String], kind: FontRoleKind) -> Vec<FontChoice> {
    let mut v: Vec<FontChoice> =
        embedded_for(kind).map(|n| FontChoice { name: n.to_string(), embedded: true }).collect();
    let mut system: Vec<&String> =
        installed.iter().filter(|n| !v.iter().any(|e| sysfonts::norm(&e.name) == sysfonts::norm(n))).collect();
    system.sort_by_key(|n| n.to_lowercase());
    system.dedup_by_key(|n| sysfonts::norm(n));
    v.extend(system.into_iter().map(|n| FontChoice { name: n.clone(), embedded: false }));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: Vec<FontChoice>) -> Vec<String> {
        v.into_iter().map(|s| if s.embedded { format!("{}*", s.name) } else { s.name }).collect()
    }

    #[test]
    fn choices_list_embedded_then_installed() {
        let installed: Vec<String> =
            ["Segoe UI", "arial", "Cascadia Mono", "Consolas", "Cascadia Code", "consolas"].map(String::from).to_vec();
        let mono = names(font_choices(&installed, FontRoleKind::Terminal));
        assert_eq!(
            mono,
            [
                "Cascadia Mono*",
                "JetBrains Mono*",
                "IBM Plex Mono*",
                "Source Code Pro*",
                "arial",
                "Cascadia Code",
                "Consolas",
                "Segoe UI"
            ],
            "the installed Cascadia Mono is the embedded one; duplicates collapse"
        );
        let ui = names(font_choices(&installed, FontRoleKind::Ui));
        assert_eq!(&ui[..4], ["Inter*", "Inter Variable*", "IBM Plex Sans*", "Noto Sans*"]);
        assert!(names(font_choices(&[], FontRoleKind::UiSemibold)).contains(&"Inter SemiBold*".to_string()));
    }
}
