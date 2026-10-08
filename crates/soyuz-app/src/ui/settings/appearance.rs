//! Appearance page: theme, accent, density, fonts, scale, motion, icons and thumbnails,
//! a live preview card, and per-token color overrides (`[theme.colors]`).

use egui::{Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use soyuz_core::config::{Backdrop, FallbackFonts, IconStyle};

use super::widgets::{color_swatch, group, number, segmented, small_button, switch, text, to_hex};
use crate::app::SoyuzApp;
use crate::theme::icons::ph;
use crate::theme::tokens::parse_hex;
use crate::theme::{Palette, icons, palette};

/// Accent presets: (name, hex).
const ACCENTS: [(&str, &str); 8] = [
    ("Orbit blue", "#5B8CFF"),
    ("Violet", "#8B5CF6"),
    ("Pink", "#EC4899"),
    ("Red", "#F2555A"),
    ("Amber", "#F59E0B"),
    ("Green", "#22C55E"),
    ("Teal", "#14B8A6"),
    ("Sky", "#38BDF8"),
];

/// Token name → getter (same names as `[theme.colors]` / PLAN §1.2).
/// `(token name, label, getter)`.
pub type Token = (&'static str, &'static str, fn(&Palette) -> Color32);

pub const TOKENS: [Token; 33] = [
    ("bg.window", "Window background", |p| p.bg_window),
    ("bg.surface1", "Sidebar, toolbars", |p| p.bg_surface1),
    ("bg.surface2", "Hover rows, inputs, cards", |p| p.bg_surface2),
    ("bg.surface3", "Popups, menus, dialogs", |p| p.bg_surface3),
    ("bg.sunken", "Terminal, hex view", |p| p.bg_sunken),
    ("border.subtle", "Separators", |p| p.border_subtle),
    ("border.strong", "Focused controls", |p| p.border_strong),
    ("text.primary", "Primary text", |p| p.text_primary),
    ("text.secondary", "Secondary text", |p| p.text_secondary),
    ("text.tertiary", "Hints, placeholders", |p| p.text_tertiary),
    ("accent", "Accent", |p| p.accent),
    ("accent.hover", "Accent (hover)", |p| p.accent_hover),
    ("accent.subtle", "Selection fill", |p| p.accent_subtle),
    ("success", "Success", |p| p.success),
    ("warning", "Warning", |p| p.warning),
    ("danger", "Danger", |p| p.danger),
    ("info", "Info", |p| p.info),
    ("glyph.folder", "Folder icons", |p| p.glyph_folder),
    ("glyph.image", "Image icons", |p| p.glyph_image),
    ("glyph.video", "Video icons", |p| p.glyph_video),
    ("glyph.audio", "Audio icons", |p| p.glyph_audio),
    ("glyph.archive", "Archive icons", |p| p.glyph_archive),
    ("glyph.code", "Code icons", |p| p.glyph_code),
    ("glyph.doc", "Document icons", |p| p.glyph_doc),
    ("glyph.exe", "Program icons", |p| p.glyph_exe),
    ("tag.red", "Tag red", |p| p.tags[0]),
    ("tag.orange", "Tag orange", |p| p.tags[1]),
    ("tag.yellow", "Tag yellow", |p| p.tags[2]),
    ("tag.green", "Tag green", |p| p.tags[3]),
    ("tag.cyan", "Tag cyan", |p| p.tags[4]),
    ("tag.blue", "Tag blue", |p| p.tags[5]),
    ("tag.purple", "Tag purple", |p| p.tags[6]),
    ("tag.gray", "Tag gray", |p| p.tags[7]),
];

/// Accent plus derived hover / subtle shades as `[theme.colors]` entries.
pub fn accent_overrides(accent: Color32) -> [(String, String); 3] {
    let [r, g, b, _] = accent.to_array();
    let lift = |c: u8| (c as f32 + (255.0 - c as f32) * 0.2).round() as u8;
    let hover = Color32::from_rgb(lift(r), lift(g), lift(b));
    let subtle = Color32::from_rgba_unmultiplied(r, g, b, 44);
    [
        ("accent".into(), to_hex(accent)),
        ("accent.hover".into(), to_hex(hover)),
        ("accent.subtle".into(), to_hex(subtle)),
    ]
}

fn set_accent(colors: &mut std::collections::BTreeMap<String, String>, c: Color32) {
    for (k, v) in accent_overrides(c) {
        colors.insert(k, v);
    }
}

pub fn page(ui: &mut Ui, app: &mut SoyuzApp, q: &str) -> bool {
    let p = palette(ui.ctx());
    let s = &mut app.settings;
    let d = &mut s.draft;
    let ctx = "Appearance theme colors";
    let mut any = false;
    if q.trim().is_empty() {
        preview_card(ui, &p, d.appearance.density.row_height(), d.appearance.icon_style == IconStyle::Colored);
        any = true;
    }
    any |= group(ui, q, ctx, "Theme", |r| {
        r.row_kw("Theme", "Follow the system, or force dark or light.", "dark light mode", |ui| {
            let mut v = match d.appearance.theme.to_ascii_lowercase().as_str() {
                "dark" => 1,
                "light" => 2,
                _ => 0,
            };
            if segmented(ui, &mut v, &[(0, "System"), (1, "Dark"), (2, "Light")]) {
                d.appearance.theme = ["system", "dark", "light"][v].to_string();
            }
        });
        r.row_kw("Accent color", "Selection, focus rings, primary buttons and links.", "highlight", |ui| {
            let current = d.theme.colors.get("accent").and_then(|h| parse_hex(h)).unwrap_or(p.accent);
            let mut custom = current;
            if color_swatch(ui, &mut custom, d.theme.colors.contains_key("accent")) {
                set_accent(&mut d.theme.colors, custom);
            }
            ui.add_space(4.0);
            for (name, hex) in ACCENTS.iter().rev() {
                let c = parse_hex(hex).unwrap_or(p.accent);
                let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                let selected =
                    if d.theme.colors.contains_key("accent") { to_hex(current) == *hex } else { *hex == "#5B8CFF" };
                ui.painter().circle_filled(rect.center(), 9.0, c);
                if selected {
                    ui.painter().circle_stroke(rect.center(), 11.0, Stroke::new(2.0, c));
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        ph::CHECK,
                        icons::font(10.0),
                        Color32::WHITE,
                    );
                } else if resp.hovered() {
                    ui.painter().circle_stroke(rect.center(), 11.0, Stroke::new(1.0, p.border_strong));
                }
                if resp.on_hover_text(*name).clicked() {
                    if *hex == "#5B8CFF" {
                        for k in ["accent", "accent.hover", "accent.subtle"] {
                            d.theme.colors.remove(k);
                        }
                    } else {
                        set_accent(&mut d.theme.colors, c);
                    }
                }
            }
        });
        r.row_kw("Reduce motion", "Turn off animations (panels, dialogs, toasts).", "animation accessibility", |ui| {
            switch(ui, &mut d.appearance.reduced_motion);
        });
        r.row_kw(
            "High contrast",
            "Black / white surfaces, stronger text, borders and focus rings.",
            "accessibility a11y contrast vision",
            |ui| {
                switch(ui, &mut d.appearance.high_contrast);
            },
        );
    });
    any |= group(ui, q, ctx, "Density & text", |r| {
        r.block(
            "Density",
            "Compact, cozy, comfortable or touch: list rows of 24, 28, 32 or 40 px; touch also enlarges buttons and menus for fingers. Click a preview.",
            |ui| {
                super::density_gallery::gallery(ui, &mut d.appearance.density);
            },
        );
        r.row_kw("Font size", "Base size of all UI text.", "text scale", |ui| {
            number(ui, &mut d.appearance.font_size, 11.0..=17.0, " pt");
        });
        r.row_kw(
            "Interface scale",
            "Zoom the whole window (Ctrl+= / Ctrl+- / Ctrl+0; applies when you release the slider).",
            "ui scale zoom dpi",
            |ui| {
                let mut v = s.scale_temp.unwrap_or(d.appearance.ui_scale);
                let resp = number(ui, &mut v, 0.5..=3.0, "×");
                if resp.dragged() {
                    s.scale_temp = Some(v);
                } else {
                    if resp.changed() || s.scale_temp.is_some() {
                        d.appearance.ui_scale = (v * 20.0).round() / 20.0;
                    }
                    s.scale_temp = None;
                }
            },
        );
        r.row_kw(
            "Interface font",
            "Font family (matched against installed font files) or a .ttf/.otf path. Bundled: Inter.",
            "typeface",
            |ui| {
                text(ui, &mut d.appearance.font, "Inter", 180.0);
            },
        );
        r.row_kw("Monospace font", "Used by the terminal, hex and code previews.", "typeface code", |ui| {
            text(ui, &mut d.appearance.mono_font, "JetBrains Mono", 180.0);
        });
        r.row_kw(
            "Fallback fonts",
            "System fonts for Chinese, Japanese, Korean and emoji names, loaded in the background. Automatic: when a name needs them; at startup: also covers terminal output.",
            "cjk chinese japanese korean emoji tofu glyph unicode font fallback",
            |ui| {
                segmented(
                    ui,
                    &mut d.appearance.fallback_fonts,
                    &[(FallbackFonts::Auto, "Automatic"), (FallbackFonts::Startup, "At startup"), (FallbackFonts::Off, "Off")],
                );
            },
        );
    });
    any |= group(ui, q, ctx, "Icons & thumbnails", |r| {
        r.row("File icons", "Colored by type, or monochrome.", |ui| {
            segmented(ui, &mut d.appearance.icon_style, &[(IconStyle::Colored, "Colored"), (IconStyle::Mono, "Mono")]);
        });
        r.row_kw("Thumbnails", "Show image and video thumbnails instead of icons.", "preview pictures", |ui| {
            switch(ui, &mut d.appearance.show_thumbnails);
        });
        r.row_kw("Default thumbnail size", "Starting zoom of grid views.", "icon size zoom", |ui| {
            number(ui, &mut d.appearance.thumbnail_size, 32..=256, " px");
        });
        r.row("Sidebar width", "", |ui| {
            number(ui, &mut d.appearance.sidebar_width, 160.0..=420.0, " px");
        });
    });
    any |= group(ui, q, ctx, "Window", |r| {
        let desc = if crate::ui::titlebar::Chrome::live_switch() {
            "Soyuz's own title bar with the logo, the tabs and themed window controls. Off: the system title bar."
        } else {
            "Soyuz's title bar with the tabs under the window buttons. Off: the system title bar. Takes effect after a restart."
        };
        r.row_kw("Custom title bar", desc, "titlebar window frame decorations chrome caption", |ui| {
            switch(ui, &mut d.appearance.custom_titlebar);
        });
        r.row_kw(
            "Backdrop",
            "Windows 11: show the system Mica or Acrylic material behind the window. Takes effect after a restart.",
            "mica acrylic translucent transparent blur material",
            |ui| {
                segmented(
                    ui,
                    &mut d.appearance.backdrop,
                    &[(Backdrop::None, "None"), (Backdrop::Mica, "Mica"), (Backdrop::Acrylic, "Acrylic")],
                );
            },
        );
        r.row_kw(
            "Snap layouts",
            "Windows 11: hovering the maximize button opens the Snap Layouts flyout. Takes effect after a restart.",
            "snap maximize layouts flyout",
            |ui| {
                switch(ui, &mut d.appearance.snap_layouts);
            },
        );
    });
    any |= group(ui, q, ctx, "Theme colors", |r| {
        r.block(
            "",
            "Override any design token. Overrides apply to both dark and light themes; a dot marks overridden tokens.",
            |ui| {
                let mut reset_all = false;
                ui.horizontal(|ui| {
                    let n = d.theme.colors.len();
                    ui.label(
                        RichText::new(format!("{n} override{}", if n == 1 { "" } else { "s" }))
                            .size(11.5)
                            .color(p.text_tertiary),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if n > 0 && small_button(ui, Some(ph::ARROW_COUNTER_CLOCKWISE), "Reset colors", true).clicked()
                        {
                            reset_all = true;
                        }
                    });
                });
                ui.add_space(6.0);
                let cols = ((ui.available_width() / 250.0).floor() as usize).max(1);
                egui::Grid::new("soyuz.settings.tokens").num_columns(cols).spacing(vec2(18.0, 6.0)).show(ui, |ui| {
                    for (i, (name, label, get)) in TOKENS.iter().enumerate() {
                        let overridden = d.theme.colors.contains_key(*name);
                        let mut c = d.theme.colors.get(*name).and_then(|h| parse_hex(h)).unwrap_or_else(|| get(&p));
                        ui.horizontal(|ui| {
                            ui.set_min_width(230.0);
                            if color_swatch(ui, &mut c, overridden) {
                                d.theme.colors.insert(name.to_string(), to_hex(c));
                            }
                            ui.add_space(4.0);
                            ui.vertical(|ui| {
                                ui.label(RichText::new(*label).size(12.0).color(p.text_primary));
                                ui.label(RichText::new(*name).size(10.5).monospace().color(p.text_tertiary));
                            });
                            if overridden
                                && small_button(ui, Some(ph::X), "", false)
                                    .on_hover_text("Use the theme's color")
                                    .clicked()
                            {
                                d.theme.colors.remove(*name);
                            }
                        });
                        if (i + 1) % cols == 0 {
                            ui.end_row();
                        }
                    }
                });
                if reset_all {
                    d.theme.colors.clear();
                }
            },
        );
    });
    any
}

/// A miniature window rendered with the live palette.
fn preview_card(ui: &mut Ui, p: &Palette, row_h: f32, colored: bool) {
    ui.add_space(14.0);
    let w = ui.available_width();
    let h = 64.0 + row_h * 4.0;
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let painter = ui.painter();
    let r10 = CornerRadius::same(10);
    painter.rect_filled(rect, r10, p.bg_window);
    painter.rect_stroke(rect, r10, Stroke::new(1.0, p.border_subtle), StrokeKind::Inside);
    // Sidebar.
    let side = Rect::from_min_size(rect.min, vec2(150.0, h));
    painter.rect_filled(side, CornerRadius { nw: 10, sw: 10, ne: 0, se: 0 }, p.bg_surface1);
    for (i, (g, label, active)) in
        [(ph::HOUSE, "Home", false), (ph::FOLDER_SIMPLE, "Projects", true), (ph::DOWNLOAD_SIMPLE, "Downloads", false)]
            .iter()
            .enumerate()
    {
        let rr = Rect::from_min_size(pos2(side.left() + 8.0, side.top() + 14.0 + i as f32 * 30.0), vec2(134.0, 26.0));
        if *active {
            painter.rect_filled(rr, CornerRadius::same(6), p.accent_subtle);
        }
        let gfont = if *active { icons::font_fill(13.0) } else { icons::font(13.0) };
        painter.text(
            pos2(rr.left() + 10.0, rr.center().y),
            Align2::LEFT_CENTER,
            *g,
            gfont,
            if *active { p.accent } else { p.text_secondary },
        );
        painter.text(
            pos2(rr.left() + 30.0, rr.center().y),
            Align2::LEFT_CENTER,
            *label,
            FontId::proportional(12.0),
            p.text_primary,
        );
    }
    // Toolbar + breadcrumb.
    let main = Rect::from_min_max(pos2(side.right(), rect.top()), rect.max);
    let crumb =
        Rect::from_min_size(pos2(main.left() + 12.0, main.top() + 10.0), vec2((main.width() - 24.0).max(40.0), 26.0));
    painter.rect_filled(crumb, CornerRadius::same(6), p.bg_surface2);
    painter.text(
        pos2(crumb.left() + 10.0, crumb.center().y),
        Align2::LEFT_CENTER,
        "Home  ›  Projects",
        FontId::proportional(12.0),
        p.text_secondary,
    );
    // Rows.
    let rows = [
        (ph::FOLDER_SIMPLE, "src", p.glyph_folder, "—", true),
        (ph::IMAGE, "screenshot.png", p.glyph_image, "1.4 MiB", false),
        (ph::FILE_CODE, "main.rs", p.glyph_code, "12 KiB", false),
        (ph::FILE_TEXT, "README.md", p.glyph_doc, "3 KiB", false),
    ];
    for (i, (g, name, color, size, selected)) in rows.iter().enumerate() {
        let rr = Rect::from_min_size(
            pos2(main.left() + 12.0, main.top() + 46.0 + i as f32 * row_h),
            vec2(main.width() - 24.0, row_h - 2.0),
        );
        if *selected {
            painter.rect_filled(rr, CornerRadius::same(6), p.accent_subtle);
            painter.rect_stroke(
                rr,
                CornerRadius::same(6),
                Stroke::new(1.0, p.accent.gamma_multiply(0.6)),
                StrokeKind::Inside,
            );
        }
        let gc = if colored { *color } else { p.text_secondary };
        painter.text(pos2(rr.left() + 12.0, rr.center().y), Align2::LEFT_CENTER, *g, icons::font(15.0), gc);
        painter.text(
            pos2(rr.left() + 36.0, rr.center().y),
            Align2::LEFT_CENTER,
            *name,
            FontId::proportional(13.0),
            p.text_primary,
        );
        painter.text(
            pos2(rr.right() - 12.0, rr.center().y),
            Align2::RIGHT_CENTER,
            *size,
            FontId::proportional(12.0),
            p.text_secondary,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_match_theme_overrides() {
        // Every token listed here is accepted by `Palette::apply_overrides`.
        let mut pal = crate::theme::ORBIT_DARK.clone();
        let map = TOKENS.iter().map(|(n, _, _)| (n.to_string(), "#123456".to_string())).collect();
        assert!(pal.apply_overrides(&map).is_empty());
        for (_, _, get) in TOKENS {
            assert_eq!(get(&pal), Color32::from_rgb(0x12, 0x34, 0x56));
        }
    }

    #[test]
    fn accent_shades() {
        let o = accent_overrides(Color32::from_rgb(0x8B, 0x5C, 0xF6));
        assert_eq!(o[0].1, "#8B5CF6");
        let hover = parse_hex(&o[1].1).unwrap();
        assert!(hover.r() > 0x8B && hover.g() > 0x5C);
        assert_eq!(parse_hex(&o[2].1).unwrap().a(), 44);
    }
}
