//! Appearance → color theme gallery, accent picker and theme import / export (B2).

use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use soyuz_core::config::Config;

use super::widgets::{color_swatch, segmented, small_button};
use crate::theme::accent::AccentChoice;
use crate::theme::icons::ph;
use crate::theme::themes::{self, ThemeDef};
use crate::theme::tokens::parse_hex;
use crate::theme::{Palette, icons, library};

/// Accent presets: (name message key, hex).
pub const ACCENTS: [(&str, &str); 8] = [
    ("settings-accent-orbit-blue", "#5B8CFF"),
    ("settings-accent-violet", "#8B5CF6"),
    ("settings-accent-pink", "#EC4899"),
    ("settings-accent-red", "#F2555A"),
    ("settings-accent-amber", "#F59E0B"),
    ("settings-accent-green", "#22C55E"),
    ("settings-accent-teal", "#14B8A6"),
    ("settings-accent-sky", "#38BDF8"),
];

/// Buttons under the gallery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeAction {
    Import,
    Export,
    OpenFolder,
}

const CARD: egui::Vec2 = vec2(156.0, 92.0);

/// The theme cards (click to apply, live) plus import / export buttons.
pub fn gallery(ui: &mut Ui, d: &mut Config, p: &Palette) -> Option<ThemeAction> {
    let mut action = None;
    let all = library::all();
    let current = themes::slug(&d.appearance.color_theme);
    let hc = d.appearance.high_contrast;
    let cols = ((ui.available_width() + 10.0) / (CARD.x + 10.0)).floor().max(1.0) as usize;
    for row in all.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for t in row {
                let selected = !hc && t.id == current;
                if card(ui, p, t, selected).clicked() {
                    d.appearance.color_theme = t.id.clone();
                    d.appearance.high_contrast = false;
                }
            }
        });
        ui.add_space(6.0);
    }
    if hc {
        ui.label(egui::RichText::new(tr!("settings-theme-hc-on")).size(11.5).color(p.warning));
    }
    for w in library::warnings_of(&current) {
        ui.label(egui::RichText::new(format!("{current}: {w}")).size(11.5).color(p.warning));
    }
    for e in library::errors() {
        let name = e.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        ui.label(egui::RichText::new(format!("{name}: {}", e.error)).size(11.5).color(p.danger));
    }
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if small_button(ui, Some(ph::DOWNLOAD_SIMPLE), tr!("settings-theme-import"), false).clicked() {
            action = Some(ThemeAction::Import);
        }
        if small_button(ui, Some(ph::EXPORT), tr!("settings-theme-export"), false)
            .on_hover_text(tr!("settings-theme-export-tooltip"))
            .clicked()
        {
            action = Some(ThemeAction::Export);
        }
        if small_button(ui, Some(ph::FOLDER_OPEN), tr!("settings-theme-folder"), false)
            .on_hover_text(library::themes_dir().display().to_string())
            .clicked()
        {
            action = Some(ThemeAction::OpenFolder);
        }
    });
    action
}

/// Shown name of a theme: built-in names that are words are translated, the rest
/// (proper names, user themes) stay as written in the file.
fn display_name(t: &ThemeDef) -> &str {
    if t.builtin && t.id == "high-contrast" { tr!("settings-theme-high-contrast") } else { &t.name }
}

fn card(ui: &mut Ui, p: &Palette, t: &ThemeDef, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(CARD, Sense::click());
    crate::ui::a11y::button(&resp, &tr!("settings-theme-a11y", name = display_name(t)));
    let (dark, light) = t.palettes();
    let painter = ui.painter_at(rect.expand(3.0));
    let r = CornerRadius::same(8);
    let swatch = Rect::from_min_size(rect.min, vec2(rect.width(), 62.0));
    let half = swatch.width() / 2.0;
    for (i, pal) in [&dark, &light].into_iter().enumerate() {
        let x0 = swatch.left() + i as f32 * half;
        let area = Rect::from_min_size(pos2(x0, swatch.top()), vec2(half, swatch.height()));
        let cr = if i == 0 {
            CornerRadius { nw: 8, sw: 0, ne: 0, se: 0 }
        } else {
            CornerRadius { nw: 0, sw: 0, ne: 8, se: 0 }
        };
        painter.rect_filled(area, cr, pal.bg_window);
        // A mini window: sidebar, a selected row and two plain rows.
        let side = Rect::from_min_size(area.min + vec2(6.0, 8.0), vec2(18.0, area.height() - 16.0));
        painter.rect_filled(side, CornerRadius::same(pal.radius(3)), pal.bg_surface1);
        let mut y = area.top() + 10.0;
        for (k, w) in [(0, 40.0), (1, 30.0), (2, 36.0)] {
            let row = Rect::from_min_size(pos2(side.right() + 5.0, y), vec2(w, 9.0));
            if k == 0 {
                painter.rect_filled(row.expand2(vec2(2.0, 1.5)), CornerRadius::same(pal.radius(3)), pal.accent_subtle);
            }
            let dot = [pal.glyph_folder, pal.glyph_image, pal.glyph_code][k];
            painter.circle_filled(pos2(row.left() + 3.0, row.center().y), 2.5, dot);
            painter.rect_filled(
                Rect::from_min_size(pos2(row.left() + 8.0, row.center().y - 1.5), vec2(w - 10.0, 3.0)),
                CornerRadius::same(1),
                if k == 0 { pal.text_primary } else { pal.text_tertiary },
            );
            y += 14.0;
        }
        painter.circle_filled(pos2(area.right() - 10.0, area.bottom() - 9.0), 4.0, pal.accent);
    }
    let label = Rect::from_min_max(pos2(rect.left(), swatch.bottom()), rect.max);
    painter.rect_filled(label, CornerRadius { nw: 0, ne: 0, sw: 8, se: 8 }, p.bg_surface2);
    painter.text(
        pos2(label.left() + 10.0, label.center().y),
        Align2::LEFT_CENTER,
        display_name(t),
        FontId::proportional(12.0),
        p.text_primary,
    );
    if !t.builtin {
        painter.text(
            pos2(label.right() - 10.0, label.center().y),
            Align2::RIGHT_CENTER,
            tr!("settings-theme-custom-tag"),
            FontId::proportional(10.5),
            p.text_tertiary,
        );
    }
    let stroke = if selected {
        Stroke::new(2.0, p.accent)
    } else if resp.hovered() {
        Stroke::new(1.0, p.border_strong)
    } else {
        Stroke::new(1.0, p.border_subtle)
    };
    painter.rect_stroke(rect, r, stroke, if selected { StrokeKind::Outside } else { StrokeKind::Inside });
    if selected {
        let c = pos2(rect.right() - 12.0, rect.top() + 12.0);
        painter.circle_filled(c, 8.0, p.accent);
        painter.text(c, Align2::CENTER_CENTER, ph::CHECK, icons::font(10.0), Color32::WHITE);
    }
    resp.on_hover_text(if t.author.is_empty() {
        display_name(t).to_string()
    } else {
        tr!("settings-theme-tooltip", name = display_name(t), author = &t.author)
    })
}

/// Accent: the theme's, the OS accent, or a preset / custom color.
pub fn accent_row(ui: &mut Ui, d: &mut Config, p: &Palette) {
    let (mut choice, _) = AccentChoice::parse(&d.appearance.accent);
    // An accent in `[theme.colors]` (older settings) wins over everything; picking here
    // replaces it.
    let legacy = d.theme.colors.get("accent").and_then(|h| parse_hex(h));
    if let Some(c) = legacy {
        choice = AccentChoice::Custom(c);
    }
    let set = |d: &mut Config, c: AccentChoice| {
        for k in ["accent", "accent.hover", "accent.subtle"] {
            d.theme.colors.remove(k);
        }
        d.appearance.accent = c.to_config();
    };
    let current = match choice {
        AccentChoice::Custom(c) => c,
        _ => p.accent,
    };
    let mut mode = match choice {
        AccentChoice::Theme => 0,
        AccentChoice::System => 1,
        AccentChoice::Custom(_) => 2,
    };
    let before = mode;
    segmented(
        ui,
        &mut mode,
        &[
            (0, tr!("settings-appearance-theme")),
            (1, tr!("settings-appearance-mode-system")),
            (2, tr!("settings-custom")),
        ],
    );
    if mode != before {
        match mode {
            0 => set(d, AccentChoice::Theme),
            1 => set(d, AccentChoice::System),
            _ => set(d, AccentChoice::Custom(current)),
        }
    }
    ui.add_space(10.0);
    for (name, hex) in ACCENTS {
        let c = parse_hex(hex).unwrap_or(p.accent);
        let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
        let selected = choice == AccentChoice::Custom(c);
        ui.painter().circle_filled(rect.center(), 9.0, c);
        if selected {
            ui.painter().circle_stroke(rect.center(), 11.0, Stroke::new(2.0, c));
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, ph::CHECK, icons::font(10.0), Color32::WHITE);
        } else if resp.hovered() {
            ui.painter().circle_stroke(rect.center(), 11.0, Stroke::new(1.0, p.border_strong));
        }
        if resp.on_hover_text(crate::i18n::t_dyn(name)).clicked() {
            set(d, AccentChoice::Custom(c));
        }
    }
    ui.add_space(4.0);
    let mut custom = current;
    if color_swatch(ui, &mut custom, matches!(choice, AccentChoice::Custom(_))) {
        set(d, AccentChoice::Custom(custom));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn accent_names_are_translated() {
        for (key, _) in super::ACCENTS {
            for l in [soyuz_core::Locale::EnUs, soyuz_core::Locale::TrTr] {
                assert!(crate::i18n::has_in(key, l), "{key}");
            }
        }
    }
}
