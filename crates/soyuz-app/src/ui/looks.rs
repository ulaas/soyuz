//! Glue for the "looks" features (folder colors, themes, icons, motion) that several views share: live settings,
//! the per-frame pump, the "Folder color" menu and small painters (extension badges,
//! folder emblems, color dots, skeleton rows).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use egui::{Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Stroke, Ui, pos2, vec2};
use soyuz_core::config::Config;

use crate::services::folder_colors::{self, EMBLEMS, FolderColor};
use crate::services::tags::TAG_NAMES;
use crate::theme::icons::{self, ph};
use crate::theme::{Palette, palette};
use crate::ui::widgets::{menu_item, menu_separator, submenu};

static EXT_BADGES: AtomicBool = AtomicBool::new(true);
static SPRING_ON: AtomicBool = AtomicBool::new(true);
static SPRING_MS: AtomicU32 = AtomicU32::new(700);

/// Apply the settings read while painting / by workers (startup + every config change).
pub fn apply_settings(c: &Config) {
    EXT_BADGES.store(c.appearance.extension_badges, Ordering::Relaxed);
    SPRING_ON.store(c.behaviour.spring_loaded_folders, Ordering::Relaxed);
    SPRING_MS.store(c.behaviour.spring_load_delay_ms.clamp(100, 10_000), Ordering::Relaxed);
    crate::services::thumbs::set_video_frames(c.appearance.video_thumbnails);
    crate::services::thumb_disk::set_cap_bytes(c.appearance.thumb_disk_cache_mib.saturating_mul(1024 * 1024));
    if c.appearance.video_thumbnails && c.appearance.show_thumbnails {
        crate::services::thumb_video::probe_in_background();
    }
}

pub fn extension_badges() -> bool {
    EXT_BADGES.load(Ordering::Relaxed)
}

/// `[behaviour] spring_loaded_folders` delay in seconds (`None` = off).
pub fn spring_delay() -> Option<f64> {
    SPRING_ON.load(Ordering::Relaxed).then(|| f64::from(SPRING_MS.load(Ordering::Relaxed)) / 1000.0)
}

/// Once per frame (before the shell draws).
pub fn begin_frame(ctx: &egui::Context) {
    if folder_colors::pump(&crate::host::egui_wake(ctx)) {
        ctx.request_repaint_after(std::time::Duration::from_millis(450));
    }
}

/// Translated name of tag color `i` (`TAG_NAMES` order).
fn color_name(i: usize) -> &'static str {
    match i {
        0 => tr!("app-color-red"),
        1 => tr!("app-color-orange"),
        2 => tr!("app-color-yellow"),
        3 => tr!("app-color-green"),
        4 => tr!("app-color-cyan"),
        5 => tr!("app-color-blue"),
        6 => tr!("app-color-purple"),
        _ => tr!("app-color-gray"),
    }
}

/// Translated label of an emblem id (`EMBLEMS`).
fn emblem_label(id: &str) -> &'static str {
    match id {
        "star" => tr!("app-emblem-star"),
        "heart" => tr!("app-emblem-heart"),
        "code" => tr!("app-emblem-code"),
        "image" => tr!("app-emblem-image"),
        "music" => tr!("app-emblem-music"),
        "film" => tr!("app-emblem-film"),
        "briefcase" => tr!("app-emblem-briefcase"),
        "book" => tr!("app-emblem-book"),
        "cloud" => tr!("app-emblem-cloud"),
        "lock" => tr!("app-emblem-lock"),
        "game" => tr!("app-emblem-game"),
        _ => tr!("app-emblem-archive"),
    }
}

/// Right-click → "Folder color" for the selected folders.
pub fn folder_color_menu(ui: &mut Ui, dirs: &[PathBuf]) {
    if dirs.is_empty() {
        return;
    }
    let p = palette(ui.ctx());
    let current = folder_colors::style_of(&dirs[0]).unwrap_or_default();
    submenu(ui, Some(ph::PALETTE), tr!("app-looks-folder-color"), |ui| {
        ui.set_width(210.0);
        // A row of color dots.
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing.x = 2.0;
            for i in 0..TAG_NAMES.len() {
                let name = color_name(i);
                let c = p.tags[i];
                let (r, resp) = ui.allocate_exact_size(vec2(22.0, 26.0), egui::Sense::click());
                let selected = current.color == Some(FolderColor::Tag(i as u8));
                ui.painter().circle_filled(r.center(), 7.5, c);
                if selected {
                    ui.painter().circle_stroke(r.center(), 10.0, Stroke::new(2.0, c));
                } else if resp.hovered() {
                    ui.painter().circle_stroke(r.center(), 10.0, Stroke::new(1.0, p.border_strong));
                }
                crate::ui::a11y::button(&resp, &tr!("app-looks-color-folder", color = name));
                if resp.on_hover_text(name).clicked() {
                    folder_colors::set_color(dirs, Some(FolderColor::Tag(i as u8)));
                    ui.close();
                }
            }
        });
        if menu_item(ui, Some(ph::PROHIBIT), tr!("app-looks-no-color"), None, current.color.is_some()).clicked() {
            folder_colors::set_color(dirs, None);
            ui.close();
        }
        menu_separator(ui);
        submenu(ui, Some(ph::SPARKLE), tr!("app-looks-emblem"), |ui| {
            ui.set_width(180.0);
            for (id, _, glyph) in EMBLEMS {
                let on = current.emblem.as_deref() == Some(id);
                let resp = menu_item(ui, Some(glyph), emblem_label(id), if on { Some("✓") } else { None }, true);
                if resp.clicked() {
                    folder_colors::set_emblem(dirs, if on { None } else { Some(id) });
                    ui.close();
                }
            }
            menu_separator(ui);
            if menu_item(ui, Some(ph::PROHIBIT), tr!("app-looks-no-emblem"), None, current.emblem.is_some()).clicked() {
                folder_colors::set_emblem(dirs, None);
                ui.close();
            }
        });
    });
}

/// Folder glyph color: the folder's own color, else `default`.
pub fn folder_tint(path: &std::path::Path, p: &Palette, default: Color32) -> Color32 {
    folder_colors::color_of(path, p).unwrap_or(default)
}

/// The folder's emblem drawn on a folder glyph centered at `c` (glyph size `size`).
/// Skipped below 22 pt where it would be unreadable.
pub fn paint_emblem(painter: &Painter, path: &std::path::Path, c: Pos2, size: f32, folder: Color32, alpha: f32) {
    if size < 22.0 {
        return;
    }
    let Some(g) = folder_colors::emblem_of(path) else { return };
    // The folder body sits a little below the glyph's center.
    let at = c + vec2(0.0, size * 0.07);
    let fg = if luminance(folder) > 0.55 { Color32::from_black_alpha(170) } else { Color32::from_white_alpha(230) };
    painter.text(at, Align2::CENTER_CENTER, g, icons::font_fill(size * 0.36), fg.gamma_multiply(alpha));
}

/// A colored dot (breadcrumb crumbs, tabs) with a ring in the surface color.
pub fn paint_dot(painter: &Painter, c: Pos2, color: Color32, ring: Color32) {
    painter.circle_filled(c, 4.6, ring);
    painter.circle_filled(c, 3.4, color);
}

fn luminance(c: Color32) -> f32 {
    (0.2126 * f32::from(c.r()) + 0.7152 * f32::from(c.g()) + 0.0722 * f32::from(c.b())) / 255.0
}

/// Extension label badge at the bottom-right of a file icon (`icon` = glyph box).
pub fn paint_ext_badge(painter: &Painter, p: &Palette, icon: Rect, label: &str, color: Color32, alpha: f32) {
    if label.is_empty() || icon.width() < 40.0 {
        return;
    }
    let size = (icon.width() * 0.15).clamp(8.5, 12.0);
    let g = painter.layout_no_wrap(label.to_string(), crate::ui::views::flow::semibold(size), Color32::WHITE);
    let w = (g.size().x + size * 0.9).max(size * 2.2);
    let h = size + 5.0;
    let r = Rect::from_min_size(
        pos2(icon.center().x + icon.width() * 0.36 - w, icon.bottom() - icon.height() * 0.18 - h),
        vec2(w, h),
    );
    let r = r.translate(vec2((icon.right() - 2.0 - r.right()).min(0.0) + w * 0.18, 0.0));
    painter.rect_filled(r.expand(1.5), CornerRadius::same(5), p.bg_surface1.gamma_multiply(alpha));
    painter.rect_filled(r, CornerRadius::same(4), color.gamma_multiply(alpha));
    let text = if luminance(color) > 0.62 { Color32::from_black_alpha(200) } else { Color32::WHITE };
    painter.galley(r.center() - g.size() / 2.0, g, text.gamma_multiply(alpha));
}

/// A duration pill at the bottom-right of a video thumbnail / placeholder.
pub fn paint_duration(painter: &Painter, r: Rect, label: &str) {
    let g = painter.layout_no_wrap(label.to_string(), FontId::proportional(10.5), Color32::WHITE);
    let pill = Rect::from_min_size(
        pos2(r.right() - g.size().x - 12.0, r.bottom() - g.size().y - 8.0),
        vec2(g.size().x + 8.0, g.size().y + 3.0),
    );
    painter.rect_filled(pill, CornerRadius::same(4), Color32::from_black_alpha(170));
    painter.galley(pill.center() - g.size() / 2.0, g, Color32::WHITE);
}

/// Shimmering placeholder rows / cells while a listing streams in.
pub fn paint_skeleton(ui: &Ui, rect: Rect, grid: bool, row_h: f32) {
    let ctx = ui.ctx();
    let p = palette(ctx);
    let painter = ui.painter_at(rect);
    let still = crate::theme::reduced_motion(ctx);
    let t = if still { 0.0 } else { (ctx.input(|i| i.time) % 1.4 / 1.4) as f32 };
    let base = if p.dark { p.bg_surface2 } else { p.bg_sunken };
    let glow = if p.dark { p.bg_surface3 } else { p.bg_surface2 };
    // A soft highlight band sweeping left to right.
    let band_x = rect.left() - 200.0 + t * (rect.width() + 400.0);
    let shade = |x: f32| {
        let d = ((x - band_x).abs() / 180.0).min(1.0);
        crate::ui::settings::widgets::lerp_color(glow, base, d)
    };
    if grid {
        let cell = vec2(112.0, 132.0);
        let cols = ((rect.width() - 24.0) / (cell.x + 6.0)).floor().max(1.0) as usize;
        let rows = (rect.height() / (cell.y + 6.0)).ceil() as usize;
        for r in 0..rows.min(8) {
            for c in 0..cols {
                let min =
                    pos2(rect.left() + 12.0 + c as f32 * (cell.x + 6.0), rect.top() + 10.0 + r as f32 * (cell.y + 6.0));
                let icon = Rect::from_min_size(min + vec2(16.0, 10.0), vec2(cell.x - 32.0, cell.x - 32.0));
                painter.rect_filled(icon, CornerRadius::same(10), shade(icon.center().x));
                let line = Rect::from_min_size(pos2(min.x + 22.0, icon.bottom() + 12.0), vec2(cell.x - 44.0, 9.0));
                painter.rect_filled(line, CornerRadius::same(4), shade(line.center().x));
            }
        }
    } else {
        let n = (rect.height() / row_h).ceil() as usize;
        for i in 0..n.min(40) {
            let y = rect.top() + 8.0 + i as f32 * row_h;
            let cy = y + row_h / 2.0;
            // Stable pseudo-random name widths.
            let w = 120.0 + ((i * 73 + 29) % 11) as f32 * 18.0;
            let icon = Rect::from_center_size(pos2(rect.left() + 24.0, cy), vec2(16.0, 16.0));
            painter.rect_filled(icon, CornerRadius::same(4), shade(icon.center().x));
            let name = Rect::from_min_size(pos2(rect.left() + 42.0, cy - 5.0), vec2(w.min(rect.width() * 0.5), 10.0));
            painter.rect_filled(name, CornerRadius::same(5), shade(name.center().x));
            for (k, cw) in [(0, 70.0), (1, 54.0)] {
                let x = rect.right() - 24.0 - (k as f32 + 1.0) * 120.0;
                if x > name.right() + 30.0 {
                    let cell = Rect::from_min_size(pos2(x, cy - 4.0), vec2(cw, 8.0));
                    painter.rect_filled(cell, CornerRadius::same(4), shade(cell.center().x));
                }
            }
        }
    }
    if !still {
        ctx.request_repaint();
    }
}

/// Folder-enter transition state of one pane.
#[derive(Clone, Debug)]
struct Enter {
    loc: String,
    start: f64,
    dir: f32,
}

/// Seconds of the folder-enter slide / fade.
const ENTER_SECS: f64 = 0.2;

/// Folder-enter transition: `(x offset, opacity)` for the pane's content. Entering
/// a subfolder slides in from the right, going up from the left, other jumps only fade.
pub fn enter_transition(ctx: &egui::Context, pane: u64, location: &soyuz_core::Location) -> (f32, f32) {
    let id = egui::Id::new(("soyuz.looks.enter", pane));
    let loc = location.display();
    let now = ctx.input(|i| i.time);
    let prev: Option<Enter> = ctx.data(|d| d.get_temp(id));
    let state = match prev {
        Some(e) if e.loc == loc => e,
        Some(e) => {
            let dir = enter_direction(&e.loc, &loc);
            let s = Enter { loc, start: now, dir };
            ctx.data_mut(|d| d.insert_temp(id, s.clone()));
            s
        }
        None => {
            // First sight of this pane: no animation.
            let s = Enter { loc, start: now - ENTER_SECS, dir: 0.0 };
            ctx.data_mut(|d| d.insert_temp(id, s.clone()));
            s
        }
    };
    if crate::theme::reduced_motion(ctx) {
        return (0.0, 1.0);
    }
    let t = ((now - state.start) / ENTER_SECS) as f32;
    if t >= 1.0 {
        return (0.0, 1.0);
    }
    ctx.request_repaint();
    let e = crate::ui::motion::ease_out(t);
    ((1.0 - e) * 22.0 * state.dir, 0.25 + 0.75 * e)
}

/// +1 entering a subfolder, -1 going up to an ancestor, 0 otherwise.
pub fn enter_direction(from: &str, to: &str) -> f32 {
    const SEPS: [char; 2] = ['\\', '/'];
    let norm = |s: &str| s.trim_end_matches(SEPS).to_lowercase();
    let (f, t) = (norm(from), norm(to));
    let below = |a: &str, b: &str| a.len() > b.len() && a.starts_with(b) && a[b.len()..].starts_with(SEPS);
    if below(&t, &f) {
        1.0
    } else if below(&f, &t) {
        -1.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_directions() {
        assert_eq!(enter_direction(r"C:\a", r"C:\a\b"), 1.0);
        assert_eq!(enter_direction(r"C:\a\b\c", r"C:\a"), -1.0);
        assert_eq!(enter_direction(r"C:\", r"C:\Users"), 1.0);
        assert_eq!(enter_direction(r"C:\ab", r"C:\a"), 0.0, "prefix of a name is not an ancestor");
        assert_eq!(enter_direction(r"C:\a", r"D:\a"), 0.0);
        assert_eq!(enter_direction("/home/x", "/home/x/y"), 1.0);
    }
}
