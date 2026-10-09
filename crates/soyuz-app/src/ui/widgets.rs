//! Small reusable widgets that follow the design tokens: nav rows, icon buttons,
//! section headers, keyboard badges, the focus ring, usage bars, menu items, segmented
//! controls, a compact slider, spinners and truncated text.

use std::sync::Arc;

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align2, Color32, CornerRadius, FontId, Galley, Painter, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind,
    TextStyle, Ui, WidgetInfo, WidgetType, pos2, vec2,
};

use crate::theme::apply::CAPTION;
use crate::theme::tokens::{radius, size, space};
use crate::theme::{Palette, icons, palette};

pub mod dropdown;

/// 2 px accent ring with a 1 px `bg.window` gap, drawn around `rect` (PLAN §1.2).
pub fn focus_ring(ui: &Ui, rect: Rect) {
    let p = palette(ui.ctx());
    let painter = ui.painter();
    painter.rect_stroke(
        rect.expand(1.0),
        CornerRadius::same(radius::ROW + 1),
        Stroke::new(1.0, p.bg_window),
        StrokeKind::Outside,
    );
    painter.rect_stroke(
        rect.expand(2.0),
        CornerRadius::same(radius::ROW + 2),
        Stroke::new(2.0, p.accent),
        StrokeKind::Outside,
    );
}

/// Upper-case for captions in the current UI language (Turkish maps `i` to `İ`, which
/// `str::to_uppercase` does not).
pub fn caption_upper(s: &str) -> String {
    if crate::i18n::current() == soyuz_core::Locale::TrTr {
        s.chars().map(|c| if c == 'i' { "\u{130}".to_string() } else { c.to_uppercase().collect() }).collect()
    } else {
        s.to_uppercase()
    }
}

/// Uppercase section caption ("FAVORITES").
pub fn section_header(ui: &mut Ui, text: &str) {
    let p = palette(ui.ctx());
    ui.add_space(space::LG);
    ui.horizontal(|ui| {
        ui.add_space(space::MD);
        ui.label(RichText::new(caption_upper(text)).text_style(TextStyle::Name(CAPTION.into())).color(p.text_tertiary));
    });
    ui.add_space(space::XS);
}

/// Collapsible section caption with a chevron that appears on hover. Returns the
/// header response (click toggles `open`).
pub fn collapsible_header(ui: &mut Ui, text: &str, open: &mut bool) -> Response {
    let p = palette(ui.ctx());
    ui.add_space(space::MD);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    if resp.clicked() {
        *open = !*open;
    }
    let is_open = *open;
    resp.widget_info(|| WidgetInfo::selected(WidgetType::CollapsingHeader, true, is_open, text));
    if resp.has_focus() {
        focus_ring(ui, rect.shrink2(vec2(4.0, 0.0)));
    }
    if ui.is_rect_visible(rect) {
        let hovered = resp.hovered();
        let color = if hovered { p.text_secondary } else { p.text_tertiary };
        ui.painter().text(
            pos2(rect.left() + 10.0, rect.center().y),
            Align2::LEFT_CENTER,
            caption_upper(text),
            FontId::proportional(11.0),
            color,
        );
        let t = ui.ctx().animate_bool(resp.id.with("open"), *open);
        let alpha = if hovered { 1.0 } else { 0.0 };
        let a = ui.ctx().animate_value_with_time(resp.id.with("hover"), alpha, crate::theme::anim(ui.ctx(), 0.12));
        if a > 0.01 {
            let glyph = icons::ph::CARET_RIGHT;
            let c = pos2(rect.right() - 12.0, rect.center().y);
            // Rotate by swapping glyphs at the halfway point (cheap and crisp).
            let g = if t > 0.5 { icons::ph::CARET_DOWN } else { glyph };
            ui.painter().text(c, Align2::CENTER_CENTER, g, icons::font(11.0), p.text_tertiary.gamma_multiply(a));
        }
    }
    resp
}

/// One sidebar navigation row: icon + label, hover / active fills, optional usage bar
/// and right-aligned trailing text.
pub struct NavRow<'a> {
    pub icon: &'a str,
    pub label: &'a str,
    pub active: bool,
    pub icon_color: Option<Color32>,
    /// `(used fraction 0..=1)` drawn as a thin bar under the label.
    pub usage: Option<f32>,
}

impl NavRow<'_> {
    pub fn show(self, ui: &mut Ui) -> Response {
        let p = palette(ui.ctx());
        let base = size::ROW_COZY.max(ui.spacing().interact_size.y + 4.0);
        let height = if self.usage.is_some() { base + 8.0 } else { base };
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click_and_drag());
        crate::ui::a11y::toggle(&response, self.label, self.active);
        if ui.is_rect_visible(rect) {
            let painter = ui.painter_at(rect.expand(3.0));
            let rounding = CornerRadius::same(radius::ROW);
            let hover = ui.ctx().animate_bool_with_time(
                response.id.with("hover"),
                response.hovered(),
                crate::theme::anim(ui.ctx(), 0.10),
            );
            if self.active {
                painter.rect_filled(rect, rounding, p.accent_subtle);
            } else if hover > 0.0 {
                painter.rect_filled(rect, rounding, p.bg_surface2.gamma_multiply(hover));
            }
            let icon_font = if self.active { icons::font_fill(size::ICON) } else { icons::font(size::ICON) };
            let icon_color = if self.active { p.accent } else { self.icon_color.unwrap_or(p.text_secondary) };
            let text_y = if self.usage.is_some() { rect.top() + base / 2.0 } else { rect.center().y };
            painter.text(pos2(rect.left() + 10.0, text_y), Align2::LEFT_CENTER, self.icon, icon_font, icon_color);
            let label_color = if self.active { p.text_primary } else { p.text_primary.gamma_multiply(0.9) };
            let max_w = rect.width() - 34.0 - 8.0;
            let galley = trunc_galley(&painter, self.label, FontId::proportional(13.0), label_color, max_w);
            painter.galley(pos2(rect.left() + 34.0, text_y - galley.size().y / 2.0), galley, label_color);
            if let Some(used) = self.usage {
                let bar = Rect::from_min_size(
                    pos2(rect.left() + 34.0, rect.bottom() - 9.0),
                    vec2((rect.width() - 46.0).max(10.0), 3.0),
                );
                usage_bar(ui, &p, bar, used);
            }
            if response.has_focus() {
                focus_ring(ui, rect);
            }
        }
        response
    }
}

/// Thin free-space bar: accent normally, `warning` when < 15 % free, `danger` < 5 %.
pub fn usage_bar(ui: &Ui, p: &Palette, rect: Rect, used: f32) {
    let used = used.clamp(0.0, 1.0);
    let free = 1.0 - used;
    let color = if free < 0.05 {
        p.danger
    } else if free < 0.15 {
        p.warning
    } else {
        p.accent
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(2), p.border_subtle);
    let mut fill = rect;
    fill.set_width((rect.width() * used).max(2.0));
    painter.rect_filled(fill, CornerRadius::same(2), color);
}

/// Frameless square icon button (fill appears on hover).
pub fn icon_button(ui: &mut Ui, glyph: &str, tooltip: &str, enabled: bool) -> Response {
    icon_toggle(ui, glyph, tooltip, enabled, false)
}

/// Icon button with an "on" state (accent glyph on a subtle accent fill).
pub fn icon_toggle(ui: &mut Ui, glyph: &str, tooltip: &str, enabled: bool, on: bool) -> Response {
    let p = palette(ui.ctx());
    let side = (ui.spacing().interact_size.y + 6.0).max(30.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), if enabled { Sense::click() } else { Sense::hover() });
    if on {
        crate::ui::a11y::toggle(&resp, tooltip, true);
    } else {
        crate::ui::a11y::button(&resp, tooltip);
    }
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool_with_time(
            resp.id.with("h"),
            enabled && resp.hovered(),
            crate::theme::anim(ui.ctx(), 0.10),
        );
        let rounding = CornerRadius::same(radius::ROW);
        if on {
            ui.painter().rect_filled(rect, rounding, p.accent_subtle);
        } else if hover > 0.0 {
            ui.painter().rect_filled(rect, rounding, p.bg_surface2.gamma_multiply(hover));
        }
        if resp.is_pointer_button_down_on() && enabled {
            ui.painter().rect_filled(rect, rounding, p.bg_surface3.gamma_multiply(0.6));
        }
        let color = if !enabled {
            p.text_tertiary.gamma_multiply(0.6)
        } else if on {
            p.accent
        } else if hover > 0.5 {
            p.text_primary
        } else {
            p.text_secondary
        };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, icons::font(size::ICON_TOOLBAR - 2.0), color);
        if resp.has_focus() {
            focus_ring(ui, rect);
        }
    }
    if tooltip.is_empty() { resp } else { resp.on_hover_text(tooltip) }
}

/// An icon glyph as a label, hidden from assistive technology (it sits next to a text
/// that names the same thing; a screen reader would otherwise read the private-use
/// character of the icon font).
pub fn icon_label(ui: &mut Ui, icon: impl Into<egui::WidgetText>) -> Response {
    let r = ui.label(icon);
    crate::ui::a11y::decorative(&r);
    r
}

/// Keyboard shortcut badge (`Ctrl+Shift+P`).
pub fn kbd(ui: &mut Ui, text: &str) -> Response {
    let p = palette(ui.ctx());
    egui::Frame::new()
        .fill(p.bg_surface2)
        .stroke(Stroke::new(1.0, p.border_subtle))
        .corner_radius(CornerRadius::same(radius::CONTROL))
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).text_style(TextStyle::Small).color(p.text_secondary));
        })
        .response
}

/// A 1 px vertical divider of the given height.
pub fn vdivider(ui: &mut Ui, height: f32) {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, height), Sense::hover());
    ui.painter().line_segment([rect.center_top(), rect.center_bottom()], Stroke::new(1.0, p.border_subtle));
}

/// Single-line galley truncated with an ellipsis at `max_w`.
pub fn trunc_galley(painter: &Painter, text: &str, font: FontId, color: Color32, max_w: f32) -> Arc<Galley> {
    let mut job = revealed_job(text, &font, color);
    job.wrap = TextWrapping::truncate_at_width(max_w.max(8.0));
    painter.layout_job(job)
}

/// Tooltip of the hidden-characters badge.
pub fn hidden_controls_tip() -> &'static str {
    tr!("app-hidden-controls-tip")
}

/// Single-line layout of untrusted text (file names): hidden bidi / zero-width / control
/// characters are shown as dimmed `⟨U+XXXX⟩` tokens instead of silently reordering or
/// vanishing (`soyuz_core::naming::name_parts`). Display only; names on disk are untouched.
pub fn revealed_job(text: &str, font: &FontId, color: Color32) -> LayoutJob {
    use soyuz_core::naming::{NamePart, control_token, name_parts};
    let mut job = LayoutJob { break_on_newline: false, ..Default::default() };
    let text_fmt = egui::TextFormat::simple(font.clone(), color);
    for part in name_parts(text) {
        match part {
            NamePart::Text(t) => job.append(t, 0.0, text_fmt.clone()),
            NamePart::Control(c) => job.append(
                &control_token(c),
                1.0,
                egui::TextFormat::simple(FontId::monospace((font.size * 0.8).max(8.0)), color.gamma_multiply(0.5)),
            ),
        }
    }
    job
}

/// `name` for plain labels: hidden controls replaced by visible `⟨U+XXXX⟩` tokens.
pub fn shown_name(name: &str) -> String {
    soyuz_core::naming::reveal_controls(name).into_owned()
}

/// Paint the small warning badge for a name with hidden characters, left-centred at
/// `pos`; returns its width (0 when `name` is clean and nothing was painted).
pub fn paint_hidden_badge(painter: &Painter, p: &Palette, pos: Pos2, name: &str) -> f32 {
    if !soyuz_core::naming::has_hidden_controls(name) {
        return 0.0;
    }
    painter.text(pos2(pos.x + 2.0, pos.y), Align2::LEFT_CENTER, icons::ph::WARNING, icons::font(11.0), p.warning);
    14.0
}

/// Dialog line under a name with hidden characters: warning badge (tooltip) plus the
/// revealed spelling. No-op for clean names.
pub fn hidden_note(ui: &mut Ui, name: &str) {
    if !soyuz_core::naming::has_hidden_controls(name) {
        return;
    }
    let p = palette(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        hidden_badge(ui, name);
        ui.add(
            egui::Label::new(revealed_job(name, &FontId::proportional(11.5), p.warning))
                .truncate()
                .sense(Sense::hover()),
        )
        .on_hover_text(hidden_controls_tip());
    });
}

/// Label-style badge (with tooltip) after a name shown in a dialog; no-op for clean names.
pub fn hidden_badge(ui: &mut Ui, name: &str) {
    if soyuz_core::naming::has_hidden_controls(name) {
        let p = palette(ui.ctx());
        ui.add(egui::Label::new(icons::icon(icons::ph::WARNING, 12.0).color(p.warning)).sense(Sense::hover()))
            .on_hover_text(hidden_controls_tip());
    }
}

/// Text wrapped to at most `rows` lines (grid captions), centered: paint it at the
/// horizontal *center* of the target area (the galley is anchored at its middle).
pub fn wrapped_galley(
    painter: &Painter,
    text: &str,
    font: FontId,
    color: Color32,
    max_w: f32,
    rows: usize,
) -> Arc<Galley> {
    let mut job = revealed_job(text, &font, color);
    job.wrap = TextWrapping { max_width: max_w.max(8.0), max_rows: rows, break_anywhere: false, ..Default::default() };
    job.halign = egui::Align::Center;
    painter.layout_job(job)
}

/// A menu row: optional icon, label, right-aligned shortcut. Hover fill, disabled state.
pub fn menu_item(ui: &mut Ui, icon: Option<&str>, label: &str, shortcut: Option<&str>, enabled: bool) -> Response {
    let p = palette(ui.ctx());
    let width = ui.available_width().clamp(160.0, 420.0);
    let h = (ui.spacing().interact_size.y + 4.0).max(28.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, h), if enabled { Sense::click() } else { Sense::hover() });
    crate::ui::a11y::menu_item(&resp, label);
    if ui.is_rect_visible(rect) {
        let hovered = enabled && (resp.hovered() || resp.has_focus());
        if hovered {
            ui.painter().rect_filled(rect, CornerRadius::same(radius::ROW), p.accent_subtle);
        }
        let fg = if enabled { p.text_primary } else { p.text_tertiary };
        let icon_color = if !enabled {
            p.text_tertiary
        } else if hovered {
            p.accent
        } else {
            p.text_secondary
        };
        if let Some(g) = icon {
            ui.painter().text(
                pos2(rect.left() + 10.0, rect.center().y),
                Align2::LEFT_CENTER,
                g,
                icons::font(15.0),
                icon_color,
            );
        }
        // Labels can be folder names: hidden controls are shown as tokens.
        let g = ui.painter().layout_job(revealed_job(label, &FontId::proportional(13.0), fg));
        ui.painter().galley(pos2(rect.left() + 34.0, rect.center().y - g.size().y / 2.0), g, fg);
        if let Some(s) = shortcut {
            ui.painter().text(
                pos2(rect.right() - 10.0, rect.center().y),
                Align2::RIGHT_CENTER,
                s,
                FontId::proportional(11.5),
                p.text_tertiary,
            );
        }
    }
    resp
}

/// A menu row that opens a submenu (caret on the right).
pub fn submenu<R>(ui: &mut Ui, icon: Option<&str>, label: &str, content: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let p = palette(ui.ctx());
    let resp = menu_item(ui, icon, label, None, true);
    ui.painter().text(
        pos2(resp.rect.right() - 10.0, resp.rect.center().y),
        Align2::RIGHT_CENTER,
        icons::ph::CARET_RIGHT,
        icons::font(11.0),
        p.text_tertiary,
    );
    egui::containers::menu::SubMenu::new().show(ui, &resp, content).map(|r| r.inner)
}

/// Menu row with a check mark (when `checked`) in the icon slot.
pub fn menu_check(ui: &mut Ui, checked: bool, label: &str, shortcut: Option<&str>) -> Response {
    menu_item(ui, Some(if checked { icons::ph::CHECK } else { "" }), label, shortcut, true)
}

/// Thin separator line inside a menu.
pub fn menu_separator(ui: &mut Ui) {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
    ui.painter().line_segment(
        [pos2(rect.left() + 6.0, rect.center().y), pos2(rect.right() - 6.0, rect.center().y)],
        Stroke::new(1.0, p.border_subtle),
    );
}

/// Small caption inside a menu ("SORT BY").
pub fn menu_caption(ui: &mut Ui, text: &str) {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    ui.painter().text(
        pos2(rect.left() + 10.0, rect.center().y + 2.0),
        Align2::LEFT_CENTER,
        caption_upper(text),
        FontId::proportional(10.5),
        p.text_tertiary,
    );
}

/// Segmented control of icon buttons; returns the clicked index.
pub fn segmented(ui: &mut Ui, items: &[(&str, &str)], selected: usize, cell: egui::Vec2) -> Option<usize> {
    let p = palette(ui.ctx());
    let total = vec2(cell.x * items.len() as f32 + 4.0, cell.y + 4.0);
    let (rect, _) = ui.allocate_exact_size(total, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(radius::ROW), p.bg_surface2);
    let mut clicked = None;
    // Animated selection thumb.
    let target = selected as f32;
    let id = ui.id().with(("segmented", rect.min.x as i32, rect.min.y as i32));
    let pos = ui.ctx().animate_value_with_time(id, target, crate::theme::anim(ui.ctx(), 0.14));
    let thumb = Rect::from_min_size(pos2(rect.left() + 2.0 + pos * cell.x, rect.top() + 2.0), cell);
    ui.painter().rect_filled(thumb, CornerRadius::same(radius::CONTROL + 1), p.bg_surface1);
    ui.painter().rect_stroke(
        thumb,
        CornerRadius::same(radius::CONTROL + 1),
        Stroke::new(1.0, p.border_subtle),
        StrokeKind::Inside,
    );
    for (i, (glyph, tip)) in items.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.left() + 2.0 + i as f32 * cell.x, rect.top() + 2.0), cell);
        let resp = ui.interact(r, id.with(i), Sense::click()).on_hover_text(*tip);
        crate::ui::a11y::toggle(&resp, tip, i == selected);
        if resp.has_focus() {
            focus_ring(ui, r);
        }
        let color = if i == selected {
            p.text_primary
        } else if resp.hovered() {
            p.text_secondary
        } else {
            p.text_tertiary
        };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, *glyph, icons::font(13.0), color);
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// Compact horizontal slider (track + knob). Returns the response; `value` is updated.
pub fn mini_slider(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, width: f32) -> Response {
    let p = palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 18.0), Sense::click_and_drag());
    let (lo, hi) = (*range.start(), *range.end());
    if (resp.dragged() || resp.clicked())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let t = ((pos.x - rect.left() - 6.0) / (rect.width() - 12.0)).clamp(0.0, 1.0);
        *value = lo + t * (hi - lo);
    }
    // Keyboard: arrows nudge the value by 5 % while focused.
    if resp.has_focus() {
        let step = (hi - lo) * 0.05;
        let (l, r) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowUp),
            )
        });
        if l {
            *value = (*value - step).max(lo);
        }
        if r {
            *value = (*value + step).min(hi);
        }
    }
    let v = f64::from(*value);
    resp.widget_info(|| WidgetInfo::slider(true, v, tr!("app-icon-size")));
    let t = ((*value - lo) / (hi - lo)).clamp(0.0, 1.0);
    let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 12.0, 3.0));
    ui.painter().rect_filled(track, CornerRadius::same(2), p.border_strong);
    let mut filled = track;
    filled.set_width(track.width() * t);
    ui.painter().rect_filled(filled, CornerRadius::same(2), p.accent);
    let knob = pos2(track.left() + track.width() * t, rect.center().y);
    let hot = resp.hovered() || resp.dragged();
    ui.painter().circle_filled(knob, if hot { 6.0 } else { 5.0 }, p.text_primary);
    ui.painter().circle_stroke(knob, if hot { 6.0 } else { 5.0 }, Stroke::new(1.0, p.border_strong));
    if resp.has_focus() {
        focus_ring(ui, rect);
    }
    resp
}

/// Indeterminate spinner (rotating arc). Requests repaints while visible.
pub fn spinner(ui: &Ui, center: Pos2, r: f32, color: Color32) {
    let t = ui.input(|i| i.time) as f32;
    let start = t * 6.0;
    let n = 24;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let a = start + (i as f32 / n as f32) * std::f32::consts::PI * 1.4;
            pos2(center.x + r * a.cos(), center.y + r * a.sin())
        })
        .collect();
    ui.painter().add(egui::Shape::line(pts, Stroke::new(2.0, color)));
    ui.ctx().request_repaint();
}

/// A pill-shaped text button with optional icon (primary = accent filled).
pub fn pill_button(ui: &mut Ui, icon: Option<&str>, label: &str, primary: bool) -> Response {
    let p = palette(ui.ctx());
    let font = FontId::proportional(12.5);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, Color32::PLACEHOLDER);
    let icon_w = if icon.is_some() { 20.0 } else { 0.0 };
    let size = vec2(galley.size().x + 24.0 + icon_w, (ui.spacing().interact_size.y + 2.0).max(26.0));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    crate::ui::a11y::button(&resp, label);
    let hover = resp.hovered();
    let (bg, fg) = if primary {
        (if hover { p.accent_hover } else { p.accent }, p.on_accent())
    } else {
        (if hover { p.bg_surface3 } else { p.bg_surface2 }, p.text_primary)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(radius::ROW), bg);
    if !primary {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(radius::ROW),
            Stroke::new(1.0, p.border_subtle),
            StrokeKind::Inside,
        );
    }
    let mut x = rect.left() + 12.0;
    if let Some(g) = icon {
        ui.painter().text(pos2(x, rect.center().y), Align2::LEFT_CENTER, g, icons::font(14.0), fg);
        x += icon_w;
    }
    ui.painter().galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
    if resp.has_focus() {
        focus_ring(ui, rect);
    }
    resp
}
