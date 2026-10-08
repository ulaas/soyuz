//! Widgets shared by the Properties, attributes, batch rename and link dialogs: an
//! underlined tab strip, key/value rows, tri-state checkboxes, small badges, a copy
//! button and a date-time picker (calendar popup + time fields). All follow the design
//! tokens (PLAN §1.2).

use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Popup, PopupCloseBehavior, Rect, Response, RichText,
    Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};

use soyuz_core::Locale;
use soyuz_core::locale::{month_names, weekday_abbr};

use crate::cmd::Cmd;
use crate::theme::tokens::radius;
use crate::theme::{Palette, icons, icons::ph, palette};
use crate::ui::views::flow::semibold;

/// Underlined text tabs with icons; returns `true` when the selection changed.
pub fn tab_strip(ui: &mut Ui, tabs: &[(&str, &str)], selected: &mut usize) -> bool {
    let p = palette(ui.ctx());
    let mut changed = false;
    let (full, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    ui.painter().line_segment(
        [pos2(full.left(), full.bottom() - 0.5), pos2(full.right(), full.bottom() - 0.5)],
        Stroke::new(1.0, p.border_subtle),
    );
    let mut x = full.left();
    let font = FontId::proportional(12.5);
    for (i, (glyph, label)) in tabs.iter().enumerate() {
        let g = ui.painter().layout_no_wrap(label.to_string(), font.clone(), p.text_primary);
        let w = g.size().x + 20.0 + 18.0;
        let r = Rect::from_min_size(pos2(x, full.top()), vec2(w, full.height()));
        let resp = ui.interact(r, ui.id().with(("tab", i)), Sense::click());
        let on = *selected == i;
        let color = if on {
            p.text_primary
        } else if resp.hovered() {
            p.text_secondary
        } else {
            p.text_tertiary
        };
        if resp.hovered() && !on {
            ui.painter().rect_filled(r.shrink2(vec2(2.0, 4.0)), CornerRadius::same(radius::ROW), p.bg_surface2);
        }
        ui.painter().text(pos2(r.left() + 10.0, r.center().y), Align2::LEFT_CENTER, *glyph, icons::font(14.0), color);
        let tg = ui.painter().layout_no_wrap(label.to_string(), font.clone(), color);
        ui.painter().galley(pos2(r.left() + 28.0, r.center().y - tg.size().y / 2.0), tg, color);
        if on {
            let bar = Rect::from_min_max(pos2(r.left() + 6.0, r.bottom() - 2.0), pos2(r.right() - 6.0, r.bottom()));
            ui.painter().rect_filled(bar, CornerRadius::same(1), p.accent);
        }
        if resp.clicked() && !on {
            *selected = i;
            changed = true;
        }
        x += w + 2.0;
    }
    ui.add_space(10.0);
    changed
}

/// Width of the label column of [`kv_row`].
pub const KV_LABEL_W: f32 = 118.0;

/// `label   value` row; the value is selectable (copyable) text.
pub fn kv_row(ui: &mut Ui, label: &str, value: &str) -> Response {
    kv_row_with(ui, label, |ui| {
        let p = palette(ui.ctx());
        ui.add(egui::Label::new(RichText::new(value).color(p.text_primary)).wrap().selectable(true))
    })
}

/// `label   <custom widget>` row.
pub fn kv_row_with<R>(ui: &mut Ui, label: &str, value: impl FnOnce(&mut Ui) -> R) -> R {
    let p = palette(ui.ctx());
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(KV_LABEL_W, 20.0), Sense::hover());
        ui.painter().text(
            pos2(r.left(), r.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(12.0),
            p.text_tertiary,
        );
        ui.vertical(|ui| {
            ui.set_min_height(20.0);
            value(ui)
        })
        .inner
    })
    .inner
}

/// Uppercase caption above a group of rows.
pub fn caption(ui: &mut Ui, text: &str) {
    let p = palette(ui.ctx());
    // Vertical spacing only: inside a horizontal row the space would indent the caption.
    let vertical = !ui.layout().is_horizontal();
    if vertical {
        ui.add_space(6.0);
    }
    ui.label(RichText::new(super::caption_upper(text)).font(FontId::proportional(10.5)).color(p.text_tertiary));
    if vertical {
        ui.add_space(2.0);
    }
}

/// 1 px horizontal rule.
pub fn rule(ui: &mut Ui) {
    let p = palette(ui.ctx());
    ui.add_space(6.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, p.border_subtle);
    ui.add_space(6.0);
}

/// Small rounded badge (`Match`, `3 conflicts`).
pub fn badge(ui: &mut Ui, icon: Option<&str>, text: &str, color: Color32) -> Response {
    let font = FontId::proportional(11.0);
    let g = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let icon_w = if icon.is_some() { 15.0 } else { 0.0 };
    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 14.0 + icon_w, 20.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::same(10), color.gamma_multiply(0.16));
    let mut x = r.left() + 7.0;
    if let Some(i) = icon {
        ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, i, icons::font(11.5), color);
        x += icon_w;
    }
    ui.painter().galley(pos2(x, r.center().y - g.size().y / 2.0), g, color);
    resp
}

/// Small icon button that copies `text` (via `Cmd::CopyText`).
pub fn copy_button(ui: &mut Ui, text: &str, cmds: &mut Vec<Cmd>) -> Response {
    let r = small_icon_button(ui, ph::COPY, tr!("common-copy"));
    if r.clicked() {
        cmds.push(Cmd::CopyText(text.to_string()));
    }
    r
}

/// 22 px icon button.
pub fn small_icon_button(ui: &mut Ui, glyph: &str, tip: &str) -> Response {
    let p = palette(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(radius::CONTROL), p.bg_surface2);
    }
    let c = if resp.hovered() { p.text_primary } else { p.text_secondary };
    ui.painter().text(r.center(), Align2::CENTER_CENTER, glyph, icons::font(13.0), c);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// Tri-state value for multi-selection editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tri {
    Off,
    On,
    Mixed,
}

impl Tri {
    pub fn from_counts(on: usize, total: usize) -> Tri {
        if on == 0 {
            Tri::Off
        } else if on == total {
            Tri::On
        } else {
            Tri::Mixed
        }
    }
}

/// Checkbox with a third "mixed" state (dash). A click turns mixed/off on, on off.
pub fn tri_check(ui: &mut Ui, state: &mut Tri, label: &str, enabled: bool) -> Response {
    let p = palette(ui.ctx());
    let font = FontId::proportional(12.5);
    let g = ui.painter().layout_no_wrap(label.to_string(), font, p.text_primary);
    let (r, resp) = ui.allocate_exact_size(
        vec2(18.0 + 8.0 + g.size().x, 22.0),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    let bx = Rect::from_center_size(pos2(r.left() + 8.0, r.center().y), vec2(15.0, 15.0));
    let on = *state != Tri::Off;
    let alpha = if enabled { 1.0 } else { 0.45 };
    if on {
        ui.painter().rect_filled(bx, CornerRadius::same(radius::CONTROL), p.accent.gamma_multiply(alpha));
    } else {
        ui.painter().rect_filled(bx, CornerRadius::same(radius::CONTROL), p.bg_surface2);
        let stroke = if resp.hovered() { p.text_tertiary } else { p.border_strong };
        ui.painter().rect_stroke(bx, CornerRadius::same(radius::CONTROL), Stroke::new(1.0, stroke), StrokeKind::Inside);
    }
    match state {
        Tri::On => {
            ui.painter().text(bx.center(), Align2::CENTER_CENTER, ph::CHECK, icons::font(11.0), Color32::WHITE);
        }
        Tri::Mixed => {
            let l = Rect::from_center_size(bx.center(), vec2(7.0, 2.0));
            ui.painter().rect_filled(l, CornerRadius::same(1), Color32::WHITE);
        }
        Tri::Off => {}
    }
    let tc = if enabled { p.text_primary } else { p.text_tertiary };
    let tg = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.5), tc);
    ui.painter().galley(pos2(bx.right() + 8.0, r.center().y - tg.size().y / 2.0), tg, tc);
    if resp.clicked() {
        *state = if *state == Tri::On { Tri::Off } else { Tri::On };
    }
    resp
}

/// Two-state checkbox in the same style as [`tri_check`].
pub fn check(ui: &mut Ui, value: &mut bool, label: &str) -> Response {
    let mut t = if *value { Tri::On } else { Tri::Off };
    let r = tri_check(ui, &mut t, label, ui.is_enabled());
    if r.clicked() {
        *value = t == Tri::On;
    }
    r
}

// ---------------------------------------------------------------------------------------
// Date-time picker
// ---------------------------------------------------------------------------------------

/// `2026-10-08 14:03:21` in local time.
pub fn format_local(t: SystemTime) -> String {
    DateTime::<Local>::from(t).format("%Y-%m-%d %H:%M:%S").to_string()
}

fn to_naive(t: SystemTime) -> NaiveDateTime {
    DateTime::<Local>::from(t).naive_local()
}

fn from_naive(n: NaiveDateTime) -> Option<SystemTime> {
    Local.from_local_datetime(&n).earliest().map(SystemTime::from)
}

/// Parse `YYYY-MM-DD[ HH:MM[:SS]]` (local time).
pub fn parse_local(s: &str) -> Option<SystemTime> {
    let s = s.trim();
    for f in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(s, f) {
            return from_naive(n);
        }
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().and_then(|d| from_naive(d.and_time(NaiveTime::MIN)))
}

#[derive(Clone, Default)]
struct PickerState {
    /// Month shown in the calendar (first day).
    month: Option<NaiveDate>,
    text: Option<String>,
}

fn days_in_month(d: NaiveDate) -> u32 {
    let (y, m) = (d.year(), d.month());
    let next = if m == 12 { NaiveDate::from_ymd_opt(y + 1, 1, 1) } else { NaiveDate::from_ymd_opt(y, m + 1, 1) };
    next.and_then(|n| n.pred_opt()).map_or(30, |l| l.day())
}

fn add_months(d: NaiveDate, delta: i32) -> NaiveDate {
    let total = d.year() * 12 + d.month0() as i32 + delta;
    NaiveDate::from_ymd_opt(total.div_euclid(12), total.rem_euclid(12) as u32 + 1, 1).unwrap_or(d)
}

/// A date-time field: an editable text box (`YYYY-MM-DD HH:MM:SS`) plus a calendar
/// button opening a month grid with time fields. `value = None` shows `placeholder`.
/// Returns `true` when the value changed this frame.
pub fn datetime_field(ui: &mut Ui, id: Id, value: &mut Option<SystemTime>, placeholder: &str, enabled: bool) -> bool {
    let p = palette(ui.ctx());
    let mut st: PickerState = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let shown = value.map(format_local).unwrap_or_default();
        let mut text = st.text.clone().unwrap_or_else(|| shown.clone());
        let resp = ui.add_enabled(
            enabled,
            egui::TextEdit::singleline(&mut text)
                .hint_text(placeholder)
                .desired_width(162.0)
                .font(FontId::monospace(12.0))
                .margin(egui::Margin::symmetric(8, 4)),
        );
        if resp.changed() {
            st.text = Some(text.clone());
        }
        let bad = st.text.as_deref().is_some_and(|t| !t.trim().is_empty() && parse_local(t).is_none());
        if bad {
            ui.painter().rect_stroke(
                resp.rect,
                CornerRadius::same(radius::CONTROL),
                Stroke::new(1.0, p.danger),
                StrokeKind::Outside,
            );
        }
        if resp.lost_focus()
            && let Some(t) = st.text.take()
            && let Some(v) = parse_local(&t)
            && Some(v) != *value
        {
            *value = Some(v);
            changed = true;
        }
        let btn =
            ui.add_enabled_ui(enabled, |ui| small_icon_button(ui, ph::CALENDAR_DOTS, tr!("dialogs-date-pick"))).inner;
        Popup::from_toggle_button_response(&btn)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .width(236.0)
            .show(|ui| {
                if calendar(ui, &mut st, value) {
                    changed = true;
                }
            });
    });
    if changed {
        st.text = None;
    }
    ui.data_mut(|d| d.insert_temp(id, st));
    changed
}

fn calendar(ui: &mut Ui, st: &mut PickerState, value: &mut Option<SystemTime>) -> bool {
    let p = palette(ui.ctx());
    let now = to_naive(SystemTime::now());
    let cur = value.map(to_naive).unwrap_or(now);
    let month = st.month.unwrap_or_else(|| cur.date().with_day(1).unwrap_or(cur.date()));
    let locale = soyuz_core::locale::current();
    let mut changed = false;
    let mut set = |n: NaiveDateTime, value: &mut Option<SystemTime>| {
        if let Some(t) = from_naive(n)
            && Some(t) != *value
        {
            *value = Some(t);
            changed = true;
        }
    };
    ui.horizontal(|ui| {
        if small_icon_button(ui, ph::CARET_LEFT, tr!("dialogs-date-prev-month")).clicked() {
            st.month = Some(add_months(month, -1));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if small_icon_button(ui, ph::CARET_RIGHT, tr!("dialogs-date-next-month")).clicked() {
                st.month = Some(add_months(month, 1));
            }
            ui.with_layout(Layout::centered_and_justified(egui::Direction::LeftToRight), |ui| {
                let title = format!("{} {}", month_names(locale)[month.month0() as usize], month.year());
                ui.label(RichText::new(title).font(semibold(13.0)).color(p.text_primary));
            });
        });
    });
    ui.add_space(4.0);
    let cell = vec2(32.0, 26.0);
    let (grid, _) = ui.allocate_exact_size(vec2(cell.x * 7.0, cell.y * 7.0), Sense::hover());
    let days = match locale {
        Locale::EnUs => ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"],
        l => weekday_abbr(l),
    };
    for (i, d) in days.iter().enumerate() {
        let c = pos2(grid.left() + cell.x * (i as f32 + 0.5), grid.top() + cell.y * 0.5);
        ui.painter().text(c, Align2::CENTER_CENTER, *d, FontId::proportional(10.5), p.text_tertiary);
    }
    let offset = month.weekday().num_days_from_monday() as usize;
    for day in 1..=days_in_month(month) {
        let slot = offset + day as usize - 1;
        let (row, col) = (slot / 7 + 1, slot % 7);
        let r = Rect::from_min_size(pos2(grid.left() + cell.x * col as f32, grid.top() + cell.y * row as f32), cell);
        let resp = ui.interact(r, ui.id().with(("day", month, day)), Sense::click());
        let date = month.with_day(day).unwrap_or(month);
        let selected = date == cur.date() && value.is_some();
        let today = date == now.date();
        let inner = r.shrink(2.0);
        if selected {
            ui.painter().rect_filled(inner, CornerRadius::same(radius::ROW), p.accent);
        } else if resp.hovered() {
            ui.painter().rect_filled(inner, CornerRadius::same(radius::ROW), p.bg_surface2);
        }
        if today && !selected {
            ui.painter().rect_stroke(
                inner,
                CornerRadius::same(radius::ROW),
                Stroke::new(1.0, p.accent),
                StrokeKind::Inside,
            );
        }
        let c = if selected { Color32::WHITE } else { p.text_primary };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, day.to_string(), FontId::proportional(12.0), c);
        if resp.clicked() {
            set(date.and_time(cur.time()), value);
        }
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let (mut h, mut m, mut s) = (cur.hour(), cur.minute(), cur.second());
        let a = ui.add(egui::DragValue::new(&mut h).range(0..=23).custom_formatter(|v, _| format!("{v:02}")));
        ui.label(":");
        let b = ui.add(egui::DragValue::new(&mut m).range(0..=59).custom_formatter(|v, _| format!("{v:02}")));
        ui.label(":");
        let c = ui.add(egui::DragValue::new(&mut s).range(0..=59).custom_formatter(|v, _| format!("{v:02}")));
        if (a.changed() || b.changed() || c.changed())
            && let Some(t) = NaiveTime::from_hms_opt(h, m, s)
        {
            set(cur.date().and_time(t), value);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(tr!("dialogs-date-now")).clicked() {
                st.month = None;
                set(now, value);
            }
        });
    });
    changed
}

/// Palette-colored status for a state (helper for tables).
pub fn status_color(p: &Palette, ok: bool) -> Color32 {
    if ok { p.success } else { p.danger }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_time_round_trip() {
        let t = parse_local("2024-02-29 13:14:15").unwrap();
        assert_eq!(format_local(t), "2024-02-29 13:14:15");
        assert_eq!(format_local(parse_local("2024-03-01").unwrap()), "2024-03-01 00:00:00");
        assert_eq!(format_local(parse_local("2024-03-01 07:08").unwrap()), "2024-03-01 07:08:00");
        assert!(parse_local("2024-13-01").is_none());
        assert!(parse_local("yesterday").is_none());
    }

    #[test]
    fn month_math() {
        let d = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        assert_eq!(days_in_month(d), 31);
        assert_eq!(days_in_month(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap()), 29);
        assert_eq!(days_in_month(NaiveDate::from_ymd_opt(2023, 2, 1).unwrap()), 28);
        assert_eq!(add_months(d, -1), NaiveDate::from_ymd_opt(2023, 12, 1).unwrap());
        assert_eq!(add_months(d, 13), NaiveDate::from_ymd_opt(2025, 2, 1).unwrap());
        assert_eq!(Tri::from_counts(0, 3), Tri::Off);
        assert_eq!(Tri::from_counts(3, 3), Tri::On);
        assert_eq!(Tri::from_counts(1, 3), Tri::Mixed);
    }
}
