//! The viewer's find bar, in the text editor's find-bar look: the query field, match
//! case / whole word chips, the running count ("3 of 12", "…" while the sweep runs) and
//! previous / next / close. Enter steps forward, Shift+Enter back.

use egui::{CornerRadius, Key, Rect, Stroke, Ui, pos2, vec2};

use super::super::widgets;
use super::PdfEnv;
use crate::editors::pdf::PdfDoc;
use crate::theme::icons::ph;

pub(super) fn show(doc: &mut PdfDoc, ui: &mut Ui, r: Rect, env: &mut PdfEnv<'_>) {
    let p = env.p;
    let id = env.id.with("pdf.find");
    ui.painter().rect_filled(r, CornerRadius::ZERO, p.bg_surface2.gamma_multiply(0.5));
    let field_w = (r.width() - 260.0).clamp(110.0, 420.0);
    let fr = Rect::from_min_size(pos2(r.left() + 8.0, r.center().y - 12.0), vec2(field_w, 24.0));
    let fid = id.with("field");
    let before = doc.find.query.text.clone();
    let resp = widgets::field(ui, fr, fid, &mut doc.find.query.text, tr!("pdf-ui-find-hint"), false);
    env.focus.push(fid);
    if std::mem::take(&mut doc.find.focus_find) {
        resp.request_focus();
        if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), fid) {
            let n = doc.find.query.text.chars().count();
            st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(n),
            )));
            st.store(ui.ctx(), fid);
        }
    }
    doc.field_focus |= resp.has_focus();
    if (resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter)) {
        let back = ui.input(|i| i.modifiers.shift);
        doc.find_step(!back);
        resp.request_focus();
    }
    let mut changed = before != doc.find.query.text;
    let mut x = fr.right() + 6.0;
    let mut chip = |ui: &mut Ui, label: &str, tip: &str, v: &mut bool| {
        let cr = Rect::from_min_size(pos2(x, r.center().y - 11.0), vec2(26.0, 22.0));
        x += 28.0;
        if widgets::chip(ui, cr, id.with(("chip", label)), label, tip, *v, p) {
            *v = !*v;
            changed = true;
        }
    };
    chip(ui, "Aa", tr!("editors-find-match-case"), &mut doc.find.query.case);
    chip(ui, "W", tr!("editors-find-whole-word"), &mut doc.find.query.whole_word);
    if changed {
        doc.find_changed();
    }
    let right = r.right() - 8.0;
    let ir = |i: f32| Rect::from_min_size(pos2(right - 26.0 * (i + 1.0), r.center().y - 12.0), vec2(24.0, 24.0));
    let (count, none) = super::find_status(&doc.find);
    let room = (ir(2.0).left() - x - 10.0).max(0.0);
    if room > 20.0 && !count.is_empty() {
        let color = if none { p.warning } else { p.text_secondary };
        let g = crate::ui::widgets::trunc_galley(ui.painter(), &count, crate::theme::font::ui(11.5), color, room);
        ui.painter().galley(pos2(x + 4.0, r.center().y - g.size().y / 2.0), g, color);
    }
    if widgets::icon_at(ui, ir(0.0), id.with("close"), ph::X, tr!("editors-find-close"), true, p) {
        doc.close_find();
        doc.focus_request = true;
    }
    let any = doc.find.count() > 0;
    if widgets::icon_at(ui, ir(1.0), id.with("next"), ph::ARROW_DOWN, tr!("editors-find-next"), any, p) {
        doc.find_step(true);
    }
    if widgets::icon_at(ui, ir(2.0), id.with("prev"), ph::ARROW_UP, tr!("editors-find-prev"), any, p) {
        doc.find_step(false);
    }
    ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.border_subtle));
}
