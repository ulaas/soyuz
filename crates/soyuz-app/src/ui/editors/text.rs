//! Draws the text editor ([`crate::editors::text::TextDoc`]): toolbar, find / replace
//! and go-to bars, the virtualized code view (only the visible rows are laid out) with
//! the line-number gutter, and the status bar.

use std::collections::HashMap;
use std::sync::Arc;

use egui::text::{CCursor, LayoutJob, TextFormat};
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, EventFilter, FontId, Galley, Id, Key, Rect, Sense, Stroke, Ui, pos2,
    vec2,
};

use super::widgets::{self, SCROLLBAR};
use crate::editors::text::buffer::Pos;
use crate::editors::text::search::Match;
use crate::editors::text::{Cell, HARD_WRAP, HL_BUDGET, LineMap, TextDoc};
use crate::services::preview_gfx::rgb;
use crate::theme::{Palette, icons::ph};
use crate::ui::gfx::color32;

const TOOLBAR_H: f32 = 34.0;
const BAR_H: f32 = 34.0;
const STATUS_H: f32 = 24.0;

/// What the surrounding panel should do after a frame.
#[derive(Default)]
pub struct TextOutput {
    pub save: bool,
    pub toggle_wrap: bool,
    pub toggle_whitespace: bool,
}

/// Settings and theme for one frame.
pub struct TextEnv<'a> {
    pub cfg: &'a soyuz_core::config::Editor,
    pub p: &'a Palette,
    /// Focusable widget ids of the editor (the app routes keys to the editor when one of
    /// them has focus).
    pub focus: &'a mut Vec<Id>,
    pub id: Id,
}

pub fn show(doc: &mut TextDoc, ui: &mut Ui, rect: Rect, env: &mut TextEnv<'_>) -> TextOutput {
    let mut out = TextOutput::default();
    let p = env.p;
    let cfg = env.cfg;
    let tab = (cfg.tab_width as usize).clamp(1, 16);
    // Toolbar.
    let tb = Rect::from_min_size(rect.min, vec2(rect.width(), TOOLBAR_H));
    toolbar(doc, ui, tb, env, &mut out);
    let mut top = tb.bottom();
    if doc.find.open {
        let h = if doc.find.replace { BAR_H * 2.0 - 4.0 } else { BAR_H };
        let r = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), h));
        find_bar(doc, ui, r, env);
        top += h;
    }
    if doc.goto.is_some() {
        let r = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), BAR_H));
        goto_bar(doc, ui, r, env);
        top += BAR_H;
    }
    let notice = match doc.notice() {
        Some(true) => Some((ph::WARNING, p.warning, tr!("editors-text-lossy"), [tr!("editors-text-edit-anyway")])),
        Some(false) => Some((ph::INFO, p.info, tr!("editors-text-mixed-eol"), [tr!("common-ok")])),
        None => None,
    };
    if let Some((glyph, color, text, buttons)) = notice {
        let r = Rect::from_min_size(pos2(rect.left() + 8.0, top + 2.0), vec2(rect.width() - 16.0, 30.0));
        if widgets::banner(ui, r, p, glyph, color, text, &buttons).is_some() {
            doc.dismiss_notice();
        }
        top += 36.0;
    }
    let status = Rect::from_min_max(pos2(rect.left(), rect.bottom() - STATUS_H), rect.max);
    let body = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), status.top()));
    self::body(doc, ui, body, env, tab);
    self::status(doc, ui, status, p, tab);
    out
}

fn toolbar(doc: &mut TextDoc, ui: &mut Ui, r: Rect, env: &mut TextEnv<'_>, out: &mut TextOutput) {
    let p = env.p;
    let id = env.id;
    let x = std::cell::Cell::new(r.left() + 6.0);
    let btn = |ui: &mut Ui, glyph: &str, tip: &str, enabled: bool, on: bool| -> bool {
        let b = Rect::from_min_size(pos2(x.get(), r.top() + 4.0), vec2(26.0, 26.0));
        x.set(x.get() + 28.0);
        if on {
            ui.painter().rect_filled(b, CornerRadius::same(4), p.accent_subtle);
        }
        widgets::icon_at(ui, b, id.with(("tb", tip)), glyph, tip, enabled, p)
    };
    let dirty = doc.is_dirty();
    out.save |= btn(ui, ph::FLOPPY_DISK, tr!("editors-tb-save"), dirty && !doc.read_only, false);
    if btn(ui, ph::ARROW_COUNTER_CLOCKWISE, tr!("editors-tb-undo"), doc.history.can_undo(), false) {
        doc.undo();
    }
    if btn(ui, ph::ARROW_CLOCKWISE, tr!("editors-tb-redo"), doc.history.can_redo(), false) {
        doc.redo();
    }
    x.set(x.get() + 8.0);
    let find_open = doc.find.open;
    let replace_open = find_open && doc.find.replace;
    let b = |ui: &mut Ui, glyph: &str, tip: &str, on: bool| -> bool {
        let b = Rect::from_min_size(pos2(x.get(), r.top() + 4.0), vec2(26.0, 26.0));
        x.set(x.get() + 28.0);
        if on {
            ui.painter().rect_filled(b, CornerRadius::same(4), p.accent_subtle);
        }
        widgets::icon_at(ui, b, id.with(("tb2", tip)), glyph, tip, true, p)
    };
    if b(ui, ph::MAGNIFYING_GLASS, tr!("editors-tb-find"), find_open && !replace_open) {
        doc.toggle_find();
    }
    if b(ui, ph::SWAP, tr!("editors-tb-replace"), replace_open) {
        doc.toggle_replace();
    }
    if b(ui, ph::ARROW_LINE_DOWN, tr!("editors-tb-goto"), doc.goto.is_some()) {
        doc.toggle_goto();
    }
    x.set(x.get() + 8.0);
    out.toggle_wrap |= b(ui, ph::TEXT_ALIGN_LEFT, tr!("editors-tb-wrap"), env.cfg.wrap);
    out.toggle_whitespace |= b(ui, ph::PARAGRAPH, tr!("editors-tb-whitespace"), env.cfg.show_whitespace);
    if doc.read_only {
        ui.painter().text(
            pos2(r.right() - 10.0, r.center().y),
            Align2::RIGHT_CENTER,
            tr!("editors-read-only"),
            FontId::proportional(11.5),
            p.warning,
        );
    }
    ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.border_subtle));
}

fn find_bar(doc: &mut TextDoc, ui: &mut Ui, r: Rect, env: &mut TextEnv<'_>) {
    let p = env.p;
    let id = env.id;
    ui.painter().rect_filled(r, CornerRadius::ZERO, p.bg_surface2.gamma_multiply(0.5));
    let row1 = Rect::from_min_size(r.min, vec2(r.width(), BAR_H));
    let field_w = (r.width() - 300.0).clamp(120.0, 420.0);
    let fr = Rect::from_min_size(pos2(row1.left() + 8.0, row1.top() + 5.0), vec2(field_w, 24.0));
    let fid = id.with("find.field");
    let before = doc.find.query.text.clone();
    let resp = widgets::field(ui, fr, fid, &mut doc.find.query.text, tr!("editors-find-hint"), true);
    env.focus.push(fid);
    if std::mem::take(&mut doc.find.focus_find) {
        resp.request_focus();
        if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), fid) {
            let n = doc.find.query.text.chars().count();
            st.cursor.set_char_range(Some(egui::text::CCursorRange::two(CCursor::new(0), CCursor::new(n))));
            st.store(ui.ctx(), fid);
        }
    }
    if (resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter)) {
        let back = ui.input(|i| i.modifiers.shift);
        doc.find_step(!back);
        resp.request_focus();
    }
    let mut x = fr.right() + 6.0;
    let mut chip = |ui: &mut Ui, label: &str, tip: &str, v: &mut bool| {
        let cr = Rect::from_min_size(pos2(x, row1.top() + 6.0), vec2(26.0, 22.0));
        x += 28.0;
        if widgets::chip(ui, cr, id.with(("chip", label)), label, tip, *v, p) {
            *v = !*v;
        }
    };
    chip(ui, "Aa", tr!("editors-find-match-case"), &mut doc.find.query.case);
    chip(ui, "W", tr!("editors-find-whole-word"), &mut doc.find.query.whole_word);
    chip(ui, ".*", tr!("editors-find-regex"), &mut doc.find.query.regex);
    if before != doc.find.query.text {
        // Incremental: jump to the first match from the selection start.
        doc.find_incremental();
    }
    let (count, is_error) = doc.find_status();
    let color = if is_error { p.danger } else { p.text_secondary };
    ui.painter().text(pos2(x + 4.0, row1.center().y), Align2::LEFT_CENTER, &count, FontId::proportional(11.5), color);
    let right = row1.right() - 8.0;
    let ir = |i: f32| Rect::from_min_size(pos2(right - 26.0 * (i + 1.0), row1.top() + 5.0), vec2(24.0, 24.0));
    if widgets::icon_at(ui, ir(0.0), id.with("find.close"), ph::X, tr!("editors-find-close"), true, p) {
        doc.find.open = false;
        doc.focus_request = true;
    }
    if widgets::icon_at(ui, ir(1.0), id.with("find.next"), ph::ARROW_DOWN, tr!("editors-find-next"), true, p) {
        doc.find_step(true);
    }
    if widgets::icon_at(ui, ir(2.0), id.with("find.prev"), ph::ARROW_UP, tr!("editors-find-prev"), true, p) {
        doc.find_step(false);
    }
    if doc.find.replace {
        let row2 = Rect::from_min_size(pos2(r.left(), row1.bottom() - 4.0), vec2(r.width(), BAR_H));
        let rr = Rect::from_min_size(pos2(row2.left() + 8.0, row2.top() + 5.0), vec2(field_w, 24.0));
        let rid = id.with("replace.field");
        let hint = if doc.find.query.regex { tr!("editors-replace-hint-regex") } else { tr!("editors-replace-hint") };
        let resp = widgets::field(ui, rr, rid, &mut doc.find.replace_text, hint, true);
        env.focus.push(rid);
        if (resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter)) {
            doc.replace_current();
            resp.request_focus();
        }
        let br = |x: f32, w: f32| Rect::from_min_size(pos2(x, row2.top() + 5.0), vec2(w, 24.0));
        // Buttons fit their (translated) labels.
        let label_w = |ui: &Ui, s: &str| {
            let g = ui.painter().layout_no_wrap(s.to_string(), FontId::proportional(12.0), p.text_primary);
            (g.size().x + 20.0).max(70.0)
        };
        let (l1, l2) = (tr!("editors-replace-one"), tr!("editors-replace-all"));
        let b1 = br(rr.right() + 6.0, label_w(ui, l1));
        let b2 = br(b1.right() + 6.0, label_w(ui, l2));
        for (b, label, all) in [(b1, l1, false), (b2, l2, true)] {
            let resp = ui.interact(b, id.with(("rep", all)), Sense::click());
            crate::ui::a11y::button(&resp, label);
            let fill = if resp.hovered() { p.bg_surface3 } else { p.bg_surface2 };
            ui.painter().rect_filled(b, CornerRadius::same(4), fill);
            ui.painter().rect_stroke(
                b,
                CornerRadius::same(4),
                Stroke::new(1.0, p.border_subtle),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(b.center(), Align2::CENTER_CENTER, label, FontId::proportional(12.0), p.text_primary);
            if resp.clicked() && !doc.read_only {
                if all {
                    doc.replace_all();
                } else {
                    doc.replace_current();
                }
            }
        }
    }
    ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.border_subtle));
}

fn goto_bar(doc: &mut TextDoc, ui: &mut Ui, r: Rect, env: &mut TextEnv<'_>) {
    let p = env.p;
    let id = env.id.with("goto.field");
    ui.painter().rect_filled(r, CornerRadius::ZERO, p.bg_surface2.gamma_multiply(0.5));
    let n = doc.buf.line_count();
    let fr = Rect::from_min_size(pos2(r.left() + 8.0, r.top() + 5.0), vec2(180.0, 24.0));
    let Some(text) = doc.goto.as_mut() else { return };
    let resp = widgets::field(ui, fr, id, text, tr!("editors-goto-hint"), true);
    env.focus.push(id);
    if std::mem::take(&mut doc.goto_focus) {
        resp.request_focus();
    }
    ui.painter().text(
        pos2(fr.right() + 10.0, r.center().y),
        Align2::LEFT_CENTER,
        tr!("editors-goto-range", count = n),
        FontId::proportional(11.5),
        p.text_secondary,
    );
    if (resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter)) {
        doc.submit_goto();
    }
    ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.border_subtle));
}

fn status(doc: &mut TextDoc, ui: &mut Ui, r: Rect, p: &Palette, tab: usize) {
    ui.painter().hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, p.border_subtle));
    let head = doc.sel.head;
    let mut left = tr!("editors-text-status-pos", line = head.line + 1, col = doc.caret_col());
    if !doc.sel.is_empty() {
        left.push_str("  ·  ");
        left.push_str(&tr!("editors-text-status-selected", count = doc.selected_chars()));
    }
    left.push_str("  ·  ");
    left.push_str(&tr!("editors-text-status-lines", count = doc.buf.line_count()));
    ui.painter().text(
        pos2(r.left() + 10.0, r.center().y),
        Align2::LEFT_CENTER,
        left,
        FontId::proportional(11.5),
        p.text_secondary,
    );
    let lang = doc.language().unwrap_or(tr!("editors-text-plain"));
    let indent = if doc.uses_tabs {
        tr!("editors-text-indent-tabs").to_string()
    } else {
        tr!("editors-text-indent-spaces", count = tab)
    };
    let right = format!("{lang}  ·  {indent}  ·  {}", doc.encoding.label());
    let eol_rect = Rect::from_min_max(pos2(r.right() - 50.0, r.top() + 2.0), pos2(r.right() - 6.0, r.bottom() - 2.0));
    ui.painter().text(
        pos2(eol_rect.left() - 8.0, r.center().y),
        Align2::RIGHT_CENTER,
        right,
        FontId::proportional(11.5),
        p.text_secondary,
    );
    let resp = ui.interact(eol_rect, ui.scope_id().with("eol"), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(eol_rect, CornerRadius::same(4), p.bg_surface2);
    }
    ui.painter().text(
        eol_rect.center(),
        Align2::CENTER_CENTER,
        doc.eol.label(),
        FontId::proportional(11.5),
        if doc.eol != doc.saved_eol { p.accent } else { p.text_secondary },
    );
    if resp.on_hover_text(tr!("editors-text-eol-tip")).clicked() {
        doc.toggle_eol();
    }
}

fn body(doc: &mut TextDoc, ui: &mut Ui, body: Rect, env: &mut TextEnv<'_>, tab: usize) {
    let p = env.p;
    let cfg = env.cfg;
    let font = crate::theme::fonts::code(ui.ctx(), cfg.font_size.clamp(6.0, 48.0));
    let (char_w, row_h) =
        ui.ctx().fonts_mut(|f| (f.glyph_width(&font, 'M').max(1.0), (f.row_height(&font) * 1.3).round().max(8.0)));
    let digits = (doc.buf.line_count().max(1).ilog10() as usize + 1).max(3);
    let gutter_w = (digits as f32 + 2.0) * char_w;
    let gutter = Rect::from_min_max(body.min, pos2(body.left() + gutter_w, body.bottom()));
    let vbar = Rect::from_min_max(pos2(body.right() - SCROLLBAR, body.top()), body.max);
    let text_left = gutter.right() + 8.0;
    let mut text = Rect::from_min_max(pos2(text_left, body.top()), pos2(vbar.left(), body.bottom()));
    let cols = if cfg.wrap { (((text.width() - char_w) / char_w).floor() as usize).max(8) } else { HARD_WRAP };
    // Layout caches.
    doc.sync_changes();
    doc.rows.rebuild(&doc.buf, cols, tab);
    let content_w = (doc.rows.max_width.min(cols as u32) as f32 + 2.0) * char_w;
    let need_h = !cfg.wrap && content_w > text.width();
    let hbar = Rect::from_min_max(pos2(text.left(), body.bottom() - SCROLLBAR), pos2(vbar.left(), body.bottom()));
    if need_h {
        text.max.y = hbar.top();
    }
    let visible_rows = (text.height() / row_h).max(1.0) as f64;
    doc.page_rows = (visible_rows as usize).saturating_sub(1).max(1);
    let total = doc.rows.total() as f64;
    let max_scroll = (total - visible_rows + 1.0).max(0.0);

    // Interaction.
    let id = env.id.with("text");
    env.focus.push(id);
    let resp = ui.interact(text, id, Sense::click_and_drag());
    // Focus handed over this frame (e.g. Enter in the go-to bar): this frame's keys
    // belonged to the previous owner.
    let just_focused = std::mem::take(&mut doc.focus_request) && !resp.has_focus();
    if just_focused {
        resp.request_focus();
    }
    let focused = resp.has_focus();
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Text);
        let d = ui.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta));
        if d != egui::Vec2::ZERO {
            doc.scroll_row -= (d.y / row_h) as f64;
            if !cfg.wrap {
                doc.scroll_x -= d.x;
            }
        }
    }
    if focused {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false },
            )
        });
        if !just_focused {
            for ev in super::edit_inputs(ui) {
                if let Some(copied) = doc.input(ev, tab) {
                    ui.ctx().copy_text(copied);
                }
            }
        }
        // Edits from the keys above: refresh row counts before drawing.
        if doc.sync_changes() {
            doc.rows.rebuild(&doc.buf, cols, tab);
        }
    }
    let total = doc.rows.total() as f64;
    let max_scroll = max_scroll.max((total - visible_rows + 1.0).max(0.0));
    // Caret row for scrolling.
    doc.scroll_caret_into_view(cols, tab, visible_rows, cfg.wrap, char_w, text.width());
    // Scrollbars.
    widgets::vscroll(ui, vbar, env.id.with("vbar"), total + visible_rows - 1.0, visible_rows, &mut doc.scroll_row, p);
    doc.scroll_row = doc.scroll_row.clamp(0.0, max_scroll.max(0.0));
    if cfg.wrap {
        doc.scroll_x = 0.0;
    } else {
        let mut sx = doc.scroll_x as f64;
        if need_h {
            widgets::hscroll(ui, hbar, env.id.with("hbar"), content_w as f64, text.width() as f64, &mut sx, p);
        }
        doc.scroll_x = (sx as f32).clamp(0.0, (content_w - text.width()).max(0.0));
    }

    // Pointer → position.
    let first_row = doc.scroll_row.floor() as usize;
    let frac = (doc.scroll_row - first_row as f64) as f32;
    let origin_y = text.top() - frac * row_h;
    let origin_x = text.left() - doc.scroll_x;
    let n_rows = (visible_rows.ceil() as usize + 2).min(doc.rows.total().saturating_sub(first_row));
    let painter = ui.painter_at(body);
    let mut maps: HashMap<usize, LineMap> = HashMap::new();
    let dark = p.dark;
    let (first_line, last_line) =
        (doc.rows.line_of(first_row).0, doc.rows.line_of(first_row + n_rows.saturating_sub(1)).0);
    if !doc.hl.prepare(doc.buf.lines(), first_line, last_line, dark, HL_BUDGET) {
        ui.ctx().request_repaint();
    }
    let row_galley = |doc: &TextDoc, maps: &mut HashMap<usize, LineMap>, row: usize| -> (Arc<Galley>, usize, usize) {
        let (line, sub) = doc.rows.line_of(row);
        let map = maps.entry(line).or_insert_with(|| LineMap::new(doc.buf.line(line), tab));
        let start = sub * cols;
        let end = ((sub + 1) * cols).min(map.disp.len());
        let job = row_job(doc, line, map, start, end, &font, p, cfg.show_whitespace);
        (painter.layout_job(job), line, start)
    };
    let pointer_pos = |doc: &TextDoc, maps: &mut HashMap<usize, LineMap>, pt: egui::Pos2| -> Pos {
        let row = ((pt.y - origin_y) / row_h).floor();
        let row = (first_row as f32 + row).clamp(0.0, (doc.rows.total() - 1) as f32) as usize;
        let (g, line, start) = row_galley(doc, maps, row);
        let c = g.cursor_from_pos(vec2(pt.x - origin_x, row_h / 2.0));
        let map = &maps[&line];
        Pos::new(line, map.byte_of(start + c.index.0))
    };
    if let Some(pt) = resp.interact_pointer_pos() {
        let shift = ui.input(|i| i.modifiers.shift);
        if resp.triple_clicked() {
            let l = pointer_pos(doc, &mut maps, pt).line;
            doc.select_line(l);
        } else if resp.double_clicked() {
            let at = pointer_pos(doc, &mut maps, pt);
            doc.select_word(at);
        } else if resp.drag_started() || (resp.is_pointer_button_down_on() && !doc.dragging && !resp.dragged()) {
            if ui.input(|i| i.pointer.primary_pressed()) {
                let at = pointer_pos(doc, &mut maps, pt);
                doc.press(at, shift);
            }
        } else if resp.dragged() && doc.dragging {
            let at = pointer_pos(doc, &mut maps, pt);
            doc.sel.head = at;
            if pt.y < text.top() || pt.y > text.bottom() {
                doc.scroll_to_caret = true;
                ui.ctx().request_repaint();
            }
        }
    }
    if !resp.is_pointer_button_down_on() && !resp.dragged() {
        doc.dragging = false;
    }

    // Paint.
    painter.rect_filled(body, CornerRadius::ZERO, p.bg_sunken.gamma_multiply(if p.dark { 0.55 } else { 0.35 }));
    painter.rect_filled(gutter, CornerRadius::ZERO, p.bg_surface1);
    let (sa, sb) = doc.sel.range();
    let head = doc.sel.head;
    let match_color = p.warning.gamma_multiply(0.28);
    let current_match = p.warning.gamma_multiply(0.55);
    let sel_color = p.accent_subtle.gamma_multiply(1.6);
    let vis_matches: &[Match] = if doc.find.open && !doc.find.matches.is_empty() {
        let a = doc.find.matches.partition_point(|m| m.line < first_line);
        let b = doc.find.matches.partition_point(|m| m.line <= last_line);
        &doc.find.matches[a..b]
    } else {
        &[]
    };
    let clip = ui.painter_at(text);
    let num_font = crate::theme::fonts::code(ui.ctx(), (cfg.font_size - 1.0).clamp(6.0, 48.0));
    for i in 0..n_rows {
        let row = first_row + i;
        let y = origin_y + i as f32 * row_h;
        let (g, line, start) = row_galley(doc, &mut maps, row);
        let map = &maps[&line];
        let row_end = (start + cols).min(map.disp.len());
        let last_row_of_line = start + cols >= map.disp.len();
        let x_of = |d: usize| -> f32 {
            let d = d.clamp(start, row_end);
            origin_x + g.pos_from_cursor(CCursor::new(d - start)).min.x
        };
        let row_rect = Rect::from_min_size(pos2(text.left(), y), vec2(text.width(), row_h));
        if line == head.line && doc.sel.is_empty() && focused {
            clip.rect_filled(row_rect, CornerRadius::ZERO, p.bg_surface2.gamma_multiply(0.55));
        }
        // Find matches.
        for m in vis_matches.iter().filter(|m| m.line == line) {
            let (da, db) = (map.disp_of(m.start), map.disp_of(m.end));
            if db <= start || da >= row_end.max(start + 1) {
                continue;
            }
            let r = Rect::from_min_max(pos2(x_of(da), y + 1.0), pos2(x_of(db).max(x_of(da) + 2.0), y + row_h - 1.0));
            let cur = sa == m.start_pos() && sb == m.end_pos();
            clip.rect_filled(r, CornerRadius::same(2), if cur { current_match } else { match_color });
        }
        // Selection.
        if !doc.sel.is_empty() && line >= sa.line && line <= sb.line {
            let da = if line == sa.line { map.disp_of(sa.col) } else { 0 };
            let (db, eol) = if line == sb.line { (map.disp_of(sb.col), false) } else { (map.disp.len(), true) };
            if db >= start && da <= row_end && !(da == db && !eol) {
                let x0 = x_of(da.max(start));
                let mut x1 = x_of(db.min(row_end));
                if eol && last_row_of_line {
                    x1 += char_w * 0.6;
                }
                clip.rect_filled(
                    Rect::from_min_max(pos2(x0, y), pos2(x1.max(x0 + 1.0), y + row_h)),
                    CornerRadius::same(2),
                    sel_color,
                );
            }
        }
        clip.galley(pos2(origin_x, y + (row_h - g.size().y) / 2.0), g.clone(), p.text_primary);
        // Caret.
        if line == head.line {
            let d = map.disp_of(head.col);
            let on_row = d >= start && (d < start + cols || last_row_of_line) && d <= row_end;
            if on_row && focused {
                let x = x_of(d);
                let cr = Rect::from_min_max(pos2(x - 0.5, y + 2.0), pos2(x + 1.5, y + row_h - 2.0));
                clip.rect_filled(cr, CornerRadius::ZERO, p.accent);
                ui.ctx().output_mut(|o| {
                    o.ime = Some(egui::output::IMEOutput {
                        purpose: egui::IMEPurpose::Normal,
                        rect: text,
                        cursor_rect: cr,
                        should_interrupt_composition: false,
                    })
                });
            }
        }
        // Line number.
        if start == 0 {
            let color = if line == head.line { p.text_primary } else { p.text_tertiary };
            painter.text(
                pos2(gutter.right() - char_w * 0.8, y + row_h / 2.0),
                Align2::RIGHT_CENTER,
                (line + 1).to_string(),
                num_font.clone(),
                color,
            );
        }
    }
    // Gutter / text divider.
    painter.vline(gutter.right(), body.y_range(), Stroke::new(1.0, p.border_subtle));
    if doc.buf.line_count() == 1 && doc.buf.line(0).is_empty() && !focused {
        clip.text(
            pos2(text.left(), text.top() + row_h / 2.0),
            Align2::LEFT_CENTER,
            tr!("editors-text-empty"),
            FontId::proportional(12.0),
            p.text_tertiary,
        );
    }
}

/// Layout job for display cells `[start, end)` of a line.
#[allow(clippy::too_many_arguments)]
fn row_job(
    doc: &TextDoc,
    line: usize,
    map: &LineMap,
    start: usize,
    end: usize,
    font: &FontId,
    p: &Palette,
    show_ws: bool,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_rows = 1;
    job.break_on_newline = false;
    let spans = doc.hl.get(line);
    // Display-cell span boundaries.
    let mut dspans: Vec<(usize, usize, Color32, bool)> = Vec::new();
    if let Some(sp) = spans {
        for s in sp.iter() {
            let a = map.disp_of(s.start as usize);
            let b = map.disp_of(s.end as usize);
            if b > start && a < end {
                let color = if !p.dark && s.color == rgb(0x32, 0x32, 0x32) { p.text_primary } else { color32(s.color) };
                dspans.push((a, b, color, s.italic));
            }
        }
    }
    let ws_color = p.text_tertiary.gamma_multiply(0.7);
    let mut k = 0usize;
    let mut run = String::new();
    let mut run_fmt: Option<(Color32, bool)> = None;
    let flush = |job: &mut LayoutJob, run: &mut String, fmt: Option<(Color32, bool)>| {
        if let Some((color, italics)) = fmt
            && !run.is_empty()
        {
            job.append(run, 0.0, TextFormat { font_id: font.clone(), color, italics, ..Default::default() });
            run.clear();
        }
    };
    for d in start..end {
        while k < dspans.len() && dspans[k].1 <= d {
            k += 1;
        }
        let (mut color, italic) =
            if k < dspans.len() && dspans[k].0 <= d { (dspans[k].2, dspans[k].3) } else { (p.text_primary, false) };
        let mut ch = map.disp[d];
        match map.cells[d] {
            Cell::Space if show_ws => {
                ch = '·';
                color = ws_color;
            }
            Cell::TabStart if show_ws => {
                ch = '→';
                color = ws_color;
            }
            Cell::Control => color = p.warning,
            _ => {}
        }
        let fmt = Some((color, italic));
        if fmt != run_fmt {
            flush(&mut job, &mut run, run_fmt);
            run_fmt = fmt;
        }
        run.push(ch);
    }
    flush(&mut job, &mut run, run_fmt);
    if job.text.is_empty() {
        job.append("", 0.0, TextFormat { font_id: font.clone(), color: p.text_primary, ..Default::default() });
    }
    job
}
