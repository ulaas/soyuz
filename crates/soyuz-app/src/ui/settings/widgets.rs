//! Settings building blocks: searchable groups of rows (each group is a card), and the
//! controls used in rows (switch, text segmented control, dropdown, number slider,
//! text field, color swatch).

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Response, RichText, Sense, Stroke, StrokeKind,
    TextEdit, Ui, pos2, vec2,
};

use crate::theme::fonts::SEMIBOLD;
use crate::theme::{icons, palette};

/// Upper-case `s` for the current language (Turkish dotted i: "Biçim" → "BİÇİM").
pub fn upper(s: &str) -> String {
    if crate::i18n::current() == soyuz_core::Locale::TrTr {
        s.chars().map(|c| if c == 'i' { 'İ' } else { c }).collect::<String>().to_uppercase()
    } else {
        s.to_uppercase()
    }
}

/// Whether `query` (lower-case words) matches any of `texts`.
pub fn matches(query: &str, texts: &[&str]) -> bool {
    if query.trim().is_empty() {
        return true;
    }
    let hay = texts.join(" ").to_lowercase();
    query.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
}

/// Rows of one group. In probe mode (`ui == None`) rows only record whether they match.
pub struct Rows<'a> {
    ui: Option<&'a mut Ui>,
    query: &'a str,
    /// Extra text matched for every row (section + group title).
    context: &'a str,
    pub matched: bool,
    first: bool,
}

impl Rows<'_> {
    /// A labelled row with a control on the right.
    pub fn row(&mut self, label: &str, desc: &str, control: impl FnOnce(&mut Ui)) {
        self.row_kw(label, desc, "", control);
    }

    /// Like [`row`](Self::row) with extra search keywords.
    pub fn row_kw(&mut self, label: &str, desc: &str, keywords: &str, control: impl FnOnce(&mut Ui)) {
        if !matches(self.query, &[label, desc, keywords, self.context]) {
            return;
        }
        self.matched = true;
        let Some(ui) = self.ui.as_deref_mut() else { return };
        separator(ui, &mut self.first);
        let p = palette(ui.ctx());
        // The label column takes what the control leaves (measured last frame), so long
        // labels (translations) wrap less next to narrow controls such as switches.
        let width_id = egui::Id::new(("soyuz.settings.row_ctrl_w", label));
        let ctrl_w: Option<f32> = ui.ctx().data(|d| d.get_temp(width_id));
        ui.horizontal_top(|ui| {
            let w = ui.available_width();
            let lo = (w * 0.35).max(160.0_f32.min(w));
            let left_w = ctrl_w.map_or((w * 0.5).max(160.0), |c| (w - c - 16.0).clamp(lo, (w * 0.62).max(lo)));
            let left = ui.allocate_ui_with_layout(vec2(left_w, 0.0), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.add_space(9.0);
                ui.label(RichText::new(label).size(13.0).color(p.text_primary));
                if !desc.is_empty() {
                    ui.add(egui::Label::new(RichText::new(desc).size(11.5).color(p.text_tertiary)).wrap());
                }
                ui.add_space(9.0);
            });
            let h = left.response.rect.height().max(40.0);
            let rem = ui.available_width();
            // Controls in this row use the row label as their accessible name.
            ui.ctx().data_mut(|d| d.insert_temp(row_label_id(), label.to_string()));
            let ctrl = ui.allocate_ui_with_layout(vec2(rem, h), Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                control(ui)
            });
            let used = ctrl.response.rect.width();
            if ctrl_w.is_none_or(|c| (c - used).abs() > 0.5) {
                ui.ctx().data_mut(|d| d.insert_temp(width_id, used));
                ui.ctx().request_repaint();
            }
        });
    }

    /// A full-width block (lists, editors) under an optional label.
    pub fn block(&mut self, label: &str, desc: &str, body: impl FnOnce(&mut Ui)) {
        if !matches(self.query, &[label, desc, self.context]) {
            return;
        }
        self.matched = true;
        let Some(ui) = self.ui.as_deref_mut() else { return };
        separator(ui, &mut self.first);
        let p = palette(ui.ctx());
        ui.add_space(8.0);
        if !label.is_empty() {
            ui.label(RichText::new(label).size(13.0).color(p.text_primary));
        }
        if !desc.is_empty() {
            ui.add(egui::Label::new(RichText::new(desc).size(11.5).color(p.text_tertiary)).wrap());
        }
        if !label.is_empty() || !desc.is_empty() {
            ui.add_space(6.0);
        }
        body(ui);
        ui.add_space(8.0);
    }
}

fn row_label_id() -> egui::Id {
    egui::Id::new("soyuz.settings.row_label")
}

/// Accessible name for a control: the label of the settings row being drawn.
fn row_label(ui: &Ui) -> String {
    ui.ctx().data(|d| d.get_temp::<String>(row_label_id())).unwrap_or_default()
}

fn separator(ui: &mut Ui, first: &mut bool) {
    if *first {
        *first = false;
        return;
    }
    let p = palette(ui.ctx());
    let spacing = std::mem::replace(&mut ui.spacing_mut().item_spacing.y, 0.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, p.border_subtle);
    ui.spacing_mut().item_spacing.y = spacing;
}

/// A titled card of rows; hidden entirely when searching and nothing matches.
/// Returns whether anything was drawn.
pub fn group(ui: &mut Ui, query: &str, context: &str, title: &str, mut body: impl FnMut(&mut Rows<'_>)) -> bool {
    let ctx_text = format!("{context} {title}");
    if !query.trim().is_empty() {
        let mut probe = Rows { ui: None, query, context: &ctx_text, matched: false, first: true };
        body(&mut probe);
        if !probe.matched {
            return false;
        }
    }
    let p = palette(ui.ctx());
    ui.add_space(14.0);
    if !title.is_empty() {
        ui.label(RichText::new(upper(title)).font(FontId::proportional(10.5)).color(p.text_tertiary));
        ui.add_space(6.0);
    }
    Frame::new()
        .fill(p.bg_surface2.gamma_multiply(if p.dark { 0.7 } else { 1.0 }))
        .stroke(Stroke::new(1.0, p.border_subtle))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(14, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut rows = Rows { ui: Some(ui), query, context: &ctx_text, matched: false, first: true };
            body(&mut rows);
        });
    true
}

/// iOS-style animated switch. Returns the response (`changed()` when toggled).
pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let p = palette(ui.ctx());
    let size = vec2(36.0, 20.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let (label, state) = (row_label(ui), *on);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, state, &label));
    let t = ui.ctx().animate_bool_with_time(resp.id, *on, crate::theme::anim(ui.ctx(), 0.12));
    let bg = lerp_color(p.border_strong, p.accent, t);
    let r = CornerRadius::same(10);
    ui.painter().rect_filled(rect, r, bg);
    if resp.hovered() {
        ui.painter().rect_stroke(rect, r, Stroke::new(1.0, p.accent.gamma_multiply(0.5)), StrokeKind::Outside);
    }
    let x = egui::lerp(rect.left() + 10.0..=rect.right() - 10.0, t);
    ui.painter().circle_filled(pos2(x, rect.center().y + 0.5), 8.0, Color32::from_black_alpha(40));
    ui.painter().circle_filled(pos2(x, rect.center().y), 8.0, Color32::WHITE);
    if resp.has_focus() {
        crate::ui::widgets::focus_ring(ui, rect);
    }
    resp
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

/// Segmented control with text labels. Returns `true` when the selection changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let p = palette(ui.ctx());
    let font = FontId::proportional(12.0);
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, l)| ui.painter().layout_no_wrap(l.to_string(), font.clone(), p.text_primary).size().x + 20.0)
        .collect();
    let total = vec2(widths.iter().sum::<f32>() + 4.0, 28.0);
    let (rect, _) = ui.allocate_exact_size(total, Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(7), p.bg_surface1.gamma_multiply(if p.dark { 0.8 } else { 1.0 }));
    painter.rect_stroke(rect, CornerRadius::same(7), Stroke::new(1.0, p.border_subtle), StrokeKind::Inside);
    let sel = options.iter().position(|(v, _)| v == value).unwrap_or(0);
    let id = ui.id().with(("seg", rect.min.x as i32, rect.min.y as i32));
    let target_x: f32 = widths[..sel].iter().sum();
    let x = ui.ctx().animate_value_with_time(id, target_x, crate::theme::anim(ui.ctx(), 0.14));
    let w = ui.ctx().animate_value_with_time(id.with("w"), widths[sel], crate::theme::anim(ui.ctx(), 0.14));
    let thumb = egui::Rect::from_min_size(pos2(rect.left() + 2.0 + x, rect.top() + 2.0), vec2(w, 24.0));
    ui.painter().rect_filled(thumb, CornerRadius::same(5), p.accent);
    let mut changed = false;
    let mut cx = rect.left() + 2.0;
    for (i, ((v, label), w)) in options.iter().zip(&widths).enumerate() {
        let r = egui::Rect::from_min_size(pos2(cx, rect.top() + 2.0), vec2(*w, 24.0));
        cx += w;
        let resp = ui.interact(r, id.with(i), Sense::click());
        let name = format!("{}: {label}", row_label(ui));
        resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, i == sel, &name));
        if resp.has_focus() {
            crate::ui::widgets::focus_ring(ui, r);
        }
        let color = if i == sel {
            Color32::WHITE
        } else if resp.hovered() {
            p.text_primary
        } else {
            p.text_secondary
        };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, *label, font.clone(), color);
        if resp.clicked() && i != sel {
            *value = *v;
            changed = true;
        }
    }
    changed
}

/// Themed dropdown over `options`. Returns `true` when changed.
pub fn dropdown<T: PartialEq + Clone>(ui: &mut Ui, id: &str, value: &mut T, options: &[(T, &str)], width: f32) -> bool {
    let current = options.iter().find(|(v, _)| v == value).map(|(_, l)| *l).unwrap_or(tr!("settings-custom"));
    let mut changed = false;
    egui::ComboBox::from_id_salt(("soyuz.settings.dd", id)).selected_text(current).width(width).show_ui(ui, |ui| {
        for (v, label) in options {
            if ui.selectable_label(v == value, *label).clicked() && v != value {
                *value = v.clone();
                changed = true;
            }
        }
    });
    changed
}

/// Number with a slider and a value label.
pub fn number<T: egui::emath::Numeric>(
    ui: &mut Ui,
    value: &mut T,
    range: std::ops::RangeInclusive<T>,
    suffix: &str,
) -> Response {
    let mut s = egui::Slider::new(value, range).show_value(true).trailing_fill(true);
    if !suffix.is_empty() {
        s = s.suffix(suffix);
    }
    ui.add_sized([220.0, 20.0], s)
}

/// Single-line text field.
pub fn text(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> Response {
    ui.add(TextEdit::singleline(value).hint_text(hint).desired_width(width).margin(Margin::symmetric(8, 5)))
}

/// Monospace text field.
pub fn code(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> Response {
    ui.add(
        TextEdit::singleline(value)
            .hint_text(hint)
            .font(egui::TextStyle::Monospace)
            .desired_width(width)
            .margin(Margin::symmetric(8, 5)),
    )
}

/// Small text button (secondary style); `danger` paints it red on hover.
pub fn small_button(ui: &mut Ui, icon: Option<&str>, label: &str, danger: bool) -> Response {
    let p = palette(ui.ctx());
    let font = FontId::proportional(12.0);
    let g = ui.painter().layout_no_wrap(label.to_string(), font, p.text_primary);
    let iw = if icon.is_some() { if label.is_empty() { 14.0 } else { 18.0 } } else { 0.0 };
    let w = g.size().x + iw + if label.is_empty() { 14.0 } else { 20.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 26.0), Sense::click());
    crate::ui::a11y::button(&resp, if label.is_empty() { icon.unwrap_or("") } else { label });
    let hover = resp.hovered();
    let fg = if danger && hover {
        p.danger
    } else if hover {
        p.text_primary
    } else {
        p.text_secondary
    };
    let fill = if hover { p.bg_surface3 } else { p.bg_surface2 };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, p.border_subtle), StrokeKind::Inside);
    let mut x = rect.left() + if label.is_empty() { 7.0 } else { 10.0 };
    if let Some(i) = icon {
        ui.painter().text(pos2(x, rect.center().y), Align2::LEFT_CENTER, i, icons::font(13.0), fg);
        x += iw;
    }
    ui.painter().galley(pos2(x, rect.center().y - g.size().y / 2.0), g, fg);
    if resp.has_focus() {
        crate::ui::widgets::focus_ring(ui, rect);
    }
    resp
}

/// A color swatch that opens egui's picker. Returns `true` when the color changed.
pub fn color_swatch(ui: &mut Ui, color: &mut Color32, overridden: bool) -> bool {
    let p = palette(ui.ctx());
    let before = *color;
    let resp = egui::widgets::color_picker::color_edit_button_srgba(ui, color, egui::color_picker::Alpha::OnlyBlend);
    if overridden {
        let dot = pos2(resp.rect.right() + 3.0, resp.rect.top() - 1.0);
        ui.painter().circle_filled(dot, 3.0, p.accent);
    }
    *color != before
}

/// `#RRGGBB` (or `#RRGGBBAA` when translucent).
pub fn to_hex(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{r:02X}{g:02X}{b:02X}{a:02X}") }
}

/// Heading used inside blocks.
pub fn subheading(ui: &mut Ui, text: &str) {
    let p = palette(ui.ctx());
    ui.label(
        RichText::new(text).font(FontId::new(12.5, egui::FontFamily::Name(SEMIBOLD.into()))).color(p.text_secondary),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_matching() {
        assert!(matches("", &["anything"]));
        assert!(matches("font size", &["Font size", "Base size of UI text"]));
        assert!(matches("SIZE", &["Font size"]));
        assert!(!matches("color", &["Font size", "desc"]));
        assert!(matches("term font", &["Font size", "", "Terminal"]));
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(to_hex(Color32::from_rgb(0x5B, 0x8C, 0xFF)), "#5B8CFF");
        let c = Color32::from_rgba_unmultiplied(10, 20, 30, 128);
        assert_eq!(to_hex(c), "#0A141E80");
        assert_eq!(crate::theme::tokens::parse_hex(&to_hex(c)).map(|x| x.a()), Some(128));
    }
}
