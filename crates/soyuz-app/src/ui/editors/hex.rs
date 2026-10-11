//! Draws the hex editor ([`crate::editors::hex::HexDoc`]): toolbar, go-to / find bar,
//! the virtualized 16-bytes-per-row view and the data inspector.

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, CornerRadius, EventFilter, FontId, Id, Key, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::widgets::{self, SCROLLBAR};
use crate::editors::hex::{HexDoc, ROW};
use crate::theme::{Palette, icons::ph};

const TOOLBAR_H: f32 = 34.0;
const BAR_H: f32 = 34.0;
const STATUS_H: f32 = 24.0;
const INSPECTOR_W: f32 = 230.0;

#[derive(Default)]
pub struct HexOutput {
    pub save: bool,
}

pub fn show(
    doc: &mut HexDoc,
    ui: &mut Ui,
    rect: Rect,
    p: &Palette,
    font_size: f32,
    focus: &mut Vec<Id>,
    id: Id,
) -> HexOutput {
    doc.pump();
    let mut out = HexOutput::default();
    let tb = Rect::from_min_size(rect.min, vec2(rect.width(), TOOLBAR_H));
    toolbar(doc, ui, tb, p, id, &mut out);
    let mut top = tb.bottom();
    if doc.goto.is_some() || doc.find.is_some() {
        let r = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), BAR_H));
        bar(doc, ui, r, p, id, focus);
        top += BAR_H;
    }
    if doc.too_large_for_text && doc.read_only {
        let r = Rect::from_min_size(pos2(rect.left() + 8.0, top + 2.0), vec2(rect.width() - 16.0, 30.0));
        if widgets::banner(
            ui,
            r,
            p,
            ph::INFO,
            p.info,
            tr!("editors-hex-too-large"),
            &[tr!("editors-hex-allow-editing")],
        )
        .is_some()
        {
            doc.read_only = false;
        }
        top += 36.0;
    }
    if let Some(e) = &doc.error {
        let r = Rect::from_min_size(pos2(rect.left() + 8.0, top + 2.0), vec2(rect.width() - 16.0, 30.0));
        widgets::banner(ui, r, p, ph::WARNING, p.danger, &tr!("editors-hex-read-failed", error = e), &[]);
        top += 36.0;
    }
    let status = Rect::from_min_max(pos2(rect.left(), rect.bottom() - STATUS_H), rect.max);
    let body = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), status.top()));
    self::body(doc, ui, body, p, font_size, focus, id);
    self::status(doc, ui, status, p);
    out
}

fn toolbar(doc: &mut HexDoc, ui: &mut Ui, r: Rect, p: &Palette, id: Id, out: &mut HexOutput) {
    let x = std::cell::Cell::new(r.left() + 6.0);
    let btn = |ui: &mut Ui, glyph: &str, tip: &str, enabled: bool, on: bool| -> bool {
        let b = Rect::from_min_size(pos2(x.get(), r.top() + 4.0), vec2(26.0, 26.0));
        x.set(x.get() + 28.0);
        if on {
            ui.painter().rect_filled(b, CornerRadius::same(4), p.accent_subtle);
        }
        widgets::icon_at(ui, b, id.with(("hx", tip)), glyph, tip, enabled, p)
    };
    out.save |= btn(ui, ph::FLOPPY_DISK, tr!("editors-tb-save"), doc.is_dirty(), false);
    let (can_undo, can_redo) = (doc.patches.can_undo(), doc.patches.can_redo());
    if btn(ui, ph::ARROW_COUNTER_CLOCKWISE, tr!("editors-tb-undo"), can_undo, false) {
        doc.undo();
    }
    if btn(ui, ph::ARROW_CLOCKWISE, tr!("editors-tb-redo"), can_redo, false) {
        doc.redo();
    }
    x.set(x.get() + 8.0);
    let goto_on = doc.goto.is_some();
    let find_on = doc.find.is_some();
    let b = |ui: &mut Ui, glyph: &str, tip: &str, on: bool| -> bool {
        let b = Rect::from_min_size(pos2(x.get(), r.top() + 4.0), vec2(26.0, 26.0));
        x.set(x.get() + 28.0);
        if on {
            ui.painter().rect_filled(b, CornerRadius::same(4), p.accent_subtle);
        }
        widgets::icon_at(ui, b, id.with(("hx2", tip)), glyph, tip, true, p)
    };
    if b(ui, ph::MAGNIFYING_GLASS, tr!("editors-hex-tb-find"), find_on) {
        doc.toggle_find();
    }
    if b(ui, ph::ARROW_LINE_DOWN, tr!("editors-hex-tb-goto"), goto_on) {
        doc.toggle_goto();
    }
    x.set(x.get() + 8.0);
    let lock = doc.read_only;
    if b(ui, if lock { ph::LOCK_SIMPLE } else { ph::LOCK_SIMPLE_OPEN }, tr!("editors-hex-tb-read-only"), lock) {
        doc.read_only = !doc.read_only;
    }
    // Byte order for the inspector.
    let label = if doc.big_endian { tr!("editors-hex-big-endian") } else { tr!("editors-hex-little-endian") };
    let label_w = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(11.5), p.text_secondary).size().x;
    let bw = (label_w + 16.0).max(92.0);
    let br = Rect::from_min_size(pos2(r.right() - 8.0 - bw, r.top() + 6.0), vec2(bw, 22.0));
    let resp = ui.interact(br, id.with("endian"), Sense::click());
    crate::ui::a11y::button(&resp, label);
    ui.painter().rect_filled(br, CornerRadius::same(4), if resp.hovered() { p.bg_surface3 } else { p.bg_surface2 });
    ui.painter().text(br.center(), Align2::CENTER_CENTER, label, FontId::proportional(11.5), p.text_secondary);
    if resp.on_hover_text(tr!("editors-hex-endian-tip")).clicked() {
        doc.big_endian = !doc.big_endian;
    }
    ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.border_subtle));
}

fn bar(doc: &mut HexDoc, ui: &mut Ui, r: Rect, p: &Palette, id: Id, focus: &mut Vec<Id>) {
    ui.painter().rect_filled(r, CornerRadius::ZERO, p.bg_surface2.gamma_multiply(0.5));
    let fr = Rect::from_min_size(
        pos2(r.left() + 8.0, r.top() + 5.0),
        vec2(240.0_f32.min(r.width() - 200.0).max(120.0), 24.0),
    );
    let len = doc.len;
    if let Some(text) = doc.goto.as_mut() {
        let fid = id.with("hex.goto");
        let resp = widgets::field(ui, fr, fid, text, tr!("editors-hex-goto-hint"), true);
        focus.push(fid);
        if std::mem::take(&mut doc.goto_focus) {
            resp.request_focus();
        }
        let hint = format!("0 – 0x{:X}", len.saturating_sub(1));
        ui.painter().text(
            pos2(fr.right() + 10.0, r.center().y),
            Align2::LEFT_CENTER,
            hint,
            FontId::proportional(11.5),
            p.text_secondary,
        );
        if (resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter)) {
            doc.submit_goto();
        }
    } else if let Some(f) = doc.find.as_mut() {
        let fid = id.with("hex.find");
        let hint = if f.hex { tr!("editors-hex-find-hint-hex") } else { tr!("editors-hex-find-hint-text") };
        let resp = widgets::field(ui, fr, fid, &mut f.text, hint, true);
        focus.push(fid);
        if std::mem::take(&mut f.focus) {
            resp.request_focus();
        }
        let cr = Rect::from_min_size(pos2(fr.right() + 6.0, r.top() + 6.0), vec2(40.0, 22.0));
        let mode = if f.hex { "HEX" } else { "TXT" };
        let toggled = widgets::chip(ui, cr, id.with("hex.mode"), mode, tr!("editors-hex-mode-tip"), true, p);
        if toggled {
            doc.toggle_find_mode();
        }
        let Some(f) = doc.find.as_ref() else { return };
        let status = f.status.clone().unwrap_or_default();
        let color = if f.found { p.success } else { p.text_secondary };
        ui.painter().text(
            pos2(cr.right() + 10.0, r.center().y),
            Align2::LEFT_CENTER,
            status,
            FontId::proportional(11.5),
            color,
        );
        let enter = (resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter));
        let back = ui.input(|i| i.modifiers.shift);
        let right = r.right() - 8.0;
        let ir = |i: f32| Rect::from_min_size(pos2(right - 26.0 * (i + 1.0), r.top() + 5.0), vec2(24.0, 24.0));
        let close = widgets::icon_at(ui, ir(0.0), id.with("hf.close"), ph::X, tr!("editors-find-close"), true, p);
        let next = widgets::icon_at(ui, ir(1.0), id.with("hf.next"), ph::ARROW_DOWN, tr!("editors-find-next"), true, p);
        let prev = widgets::icon_at(ui, ir(2.0), id.with("hf.prev"), ph::ARROW_UP, tr!("editors-find-prev"), true, p);
        if close {
            doc.close_bars();
        } else if enter || next || prev {
            doc.find_step(!(prev || (enter && back)));
            if enter {
                resp.request_focus();
            }
        }
    }
    ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.border_subtle));
}

fn status(doc: &HexDoc, ui: &mut Ui, r: Rect, p: &Palette) {
    ui.painter().hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, p.border_subtle));
    let mut s = tr!("editors-hex-status", hex = format!("{:X}", doc.cursor), offset = doc.cursor, count = doc.len);
    if !doc.patches.is_empty() {
        s.push_str("  ·  ");
        s.push_str(&tr!("editors-hex-status-changed", count = doc.patches.len()));
    }
    ui.painter().text(
        pos2(r.left() + 10.0, r.center().y),
        Align2::LEFT_CENTER,
        s,
        FontId::proportional(11.5),
        p.text_secondary,
    );
    let mode = if doc.read_only { tr!("editors-read-only") } else { tr!("editors-hex-overwrite") };
    let col = if doc.text_col { tr!("editors-hex-text-column") } else { tr!("editors-hex-hex-column") };
    ui.painter().text(
        pos2(r.right() - 10.0, r.center().y),
        Align2::RIGHT_CENTER,
        format!("{col}  ·  {mode}"),
        FontId::proportional(11.5),
        if doc.read_only { p.warning } else { p.text_secondary },
    );
}

fn body(doc: &mut HexDoc, ui: &mut Ui, body: Rect, p: &Palette, font_size: f32, focus: &mut Vec<Id>, id: Id) {
    let font = crate::theme::fonts::hex(ui.ctx(), font_size.clamp(6.0, 40.0));
    let (cw, row_h) =
        ui.ctx().fonts_mut(|f| (f.glyph_width(&font, '0').max(1.0), (f.row_height(&font) * 1.35).round()));
    let digits = crate::ui::preview::hex::offset_digits(doc.len);
    // Inspector beside the rows when everything fits, else as a strip below them.
    let rows_w = 10.0 + (digits as f32 + 2.0 + 49.0 + 1.5 + 16.0) * cw + 16.0 + SCROLLBAR;
    let side = body.width() >= rows_w + INSPECTOR_W;
    let (inspector, view) = if side {
        let i = Rect::from_min_max(pos2(body.right() - INSPECTOR_W, body.top()), body.max);
        (i, Rect::from_min_max(body.min, pos2(i.left(), body.bottom())))
    } else {
        let i = Rect::from_min_max(pos2(body.left(), body.bottom() - 104.0), body.max);
        (i, Rect::from_min_max(body.min, pos2(body.right(), i.top())))
    };
    let vbar = Rect::from_min_max(pos2(view.right() - SCROLLBAR, view.top()), view.max);
    let area = Rect::from_min_max(view.min, pos2(vbar.left(), view.bottom()));
    let total_rows = doc.total_rows();
    let visible = (area.height() / row_h).max(1.0) as f64;
    doc.page_rows = (visible as u64).saturating_sub(1).max(1);
    // Interaction.
    let tid = id.with("hex.view");
    focus.push(tid);
    let resp = ui.interact(area, tid, Sense::click_and_drag());
    let just_focused = std::mem::take(&mut doc.focus_request) && !resp.has_focus();
    if just_focused {
        resp.request_focus();
    }
    let focused = resp.has_focus();
    if focused && !just_focused {
        for ev in super::edit_inputs(ui) {
            if let Some(copied) = doc.input(ev) {
                ui.ctx().copy_text(copied);
            }
        }
    }
    if focused {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                tid,
                EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false },
            )
        });
    }
    if resp.hovered() {
        let d = ui.input_mut(|i| std::mem::take(&mut i.smooth_scroll_delta));
        doc.scroll_row -= (d.y / row_h) as f64;
    }
    doc.scroll_cursor_into_view(visible);
    widgets::vscroll(ui, vbar, id.with("hex.vbar"), total_rows as f64 + visible - 1.0, visible, &mut doc.scroll_row, p);
    doc.scroll_row = doc.scroll_row.clamp(0.0, (total_rows as f64 - 1.0).max(0.0));
    let first = doc.scroll_row.floor() as u64;
    let n = (visible.ceil() as u64 + 1).min(total_rows - first.min(total_rows));
    if doc.len > 0 {
        doc.ensure_pages(first * ROW, ((first + n) * ROW).min(doc.len) - 1);
    }
    // Column geometry.
    let x_off = area.left() + 10.0;
    let x_hex = x_off + (digits as f32 + 2.0) * cw;
    let hex_x = |i: u64| x_hex + (i as f32 * 3.0 + if i >= 8 { 1.0 } else { 0.0 }) * cw;
    let x_txt = hex_x(16) + 1.5 * cw;
    let origin_y = area.top() - (doc.scroll_row - first as f64) as f32 * row_h;
    // Clicks.
    if let Some(pt) = resp.interact_pointer_pos()
        && (resp.clicked() || resp.dragged())
    {
        let row = first as f32 + ((pt.y - origin_y) / row_h).floor();
        let row = row.max(0.0) as u64;
        let (col, text) = if pt.x >= x_txt - cw * 0.5 {
            (((pt.x - x_txt) / cw).floor().clamp(0.0, 15.0) as u64, true)
        } else {
            let mut best = 0;
            for i in 0..16 {
                if pt.x >= hex_x(i) - cw * 0.5 {
                    best = i;
                }
            }
            (best, false)
        };
        doc.click(row, col, text);
    }
    // Paint.
    let painter = ui.painter_at(view);
    painter.rect_filled(view, CornerRadius::ZERO, p.bg_sunken.gamma_multiply(if p.dark { 0.55 } else { 0.35 }));
    // Column header line.
    let changed = p.warning;
    for i in 0..n {
        let row = first + i;
        let y = origin_y + i as f32 * row_h;
        let base = row * ROW;
        if base >= doc.len && doc.len > 0 {
            break;
        }
        let cy = y + row_h / 2.0;
        let cur_in_row = doc.cursor / ROW == row;
        if cur_in_row {
            painter.rect_filled(
                Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), row_h)),
                CornerRadius::ZERO,
                p.bg_surface2.gamma_multiply(0.5),
            );
        }
        painter.text(
            pos2(x_off, cy),
            Align2::LEFT_CENTER,
            format!("{base:0digits$X}"),
            font.clone(),
            if cur_in_row { p.text_secondary } else { p.text_tertiary },
        );
        let mut hex_job = LayoutJob::default();
        let mut txt_job = LayoutJob::default();
        for c in 0..ROW {
            let off = base + c;
            if off >= doc.len {
                break;
            }
            let patched = doc.patches.get(off).is_some();
            let b = doc.byte(off);
            let is_cur = off == doc.cursor;
            if is_cur {
                let hx = Rect::from_min_size(pos2(hex_x(c) - 1.0, y + 1.0), vec2(cw * 2.0 + 2.0, row_h - 2.0));
                let tx = Rect::from_min_size(pos2(x_txt + c as f32 * cw, y + 1.0), vec2(cw, row_h - 2.0));
                let (strong, weak) = if doc.text_col { (tx, hx) } else { (hx, tx) };
                painter.rect_filled(
                    strong,
                    CornerRadius::same(2),
                    p.accent.gamma_multiply(if focused { 0.45 } else { 0.25 }),
                );
                painter.rect_stroke(
                    weak,
                    CornerRadius::same(2),
                    Stroke::new(1.0, p.accent.gamma_multiply(0.6)),
                    egui::StrokeKind::Inside,
                );
                if focused && !doc.text_col && doc.nibble == 1 {
                    let nx = hex_x(c) + cw;
                    painter.rect_filled(
                        Rect::from_min_size(pos2(nx, y + row_h - 3.0), vec2(cw, 2.0)),
                        CornerRadius::ZERO,
                        p.accent,
                    );
                }
            }
            let (hs, ts, color) = match b {
                Some(v) => (
                    format!("{v:02X}"),
                    if (0x20..0x7F).contains(&v) { (v as char).to_string() } else { "·".into() },
                    if patched {
                        changed
                    } else if v == 0 {
                        p.text_tertiary
                    } else {
                        p.text_primary
                    },
                ),
                None => ("··".into(), " ".into(), p.text_tertiary),
            };
            let sep = if c == 7 {
                "  "
            } else if c < ROW - 1 {
                " "
            } else {
                ""
            };
            hex_job.append(&hs, 0.0, TextFormat { font_id: font.clone(), color, ..Default::default() });
            hex_job.append(sep, 0.0, TextFormat { font_id: font.clone(), color, ..Default::default() });
            let tcolor = if patched {
                changed
            } else if ts == "·" {
                p.text_tertiary
            } else {
                p.text_secondary
            };
            txt_job.append(&ts, 0.0, TextFormat { font_id: font.clone(), color: tcolor, ..Default::default() });
        }
        let g = painter.layout_job(hex_job);
        painter.galley(pos2(x_hex, cy - g.size().y / 2.0), g, p.text_primary);
        let g = painter.layout_job(txt_job);
        painter.galley(pos2(x_txt, cy - g.size().y / 2.0), g, p.text_primary);
    }
    painter.vline(x_hex - cw, view.y_range(), Stroke::new(1.0, p.border_subtle));
    painter.vline(x_txt - 0.75 * cw, view.y_range(), Stroke::new(1.0, p.border_subtle));
    if doc.len == 0 {
        painter.text(
            area.center(),
            Align2::CENTER_CENTER,
            tr!("editors-text-empty"),
            FontId::proportional(12.0),
            p.text_tertiary,
        );
    }
    if inspector.height() > 40.0 {
        self::inspector(doc, ui, inspector, p, side);
    }
}

fn inspector(doc: &HexDoc, ui: &mut Ui, r: Rect, p: &Palette, side: bool) {
    let painter = ui.painter_at(r);
    painter.rect_filled(r, CornerRadius::ZERO, p.bg_surface1);
    if side {
        painter.vline(r.left(), r.y_range(), Stroke::new(1.0, p.border_subtle));
    } else {
        painter.hline(r.x_range(), r.top(), Stroke::new(1.0, p.border_subtle));
    }
    painter.text(
        pos2(r.left() + 14.0, r.top() + 16.0),
        Align2::LEFT_CENTER,
        tr!("editors-hex-inspector"),
        FontId::new(12.5, egui::FontFamily::Name(crate::theme::fonts::SEMIBOLD.into())),
        p.text_primary,
    );
    let order = if doc.big_endian { tr!("editors-hex-order-big") } else { tr!("editors-hex-order-little") };
    painter.text(
        pos2(r.left() + 14.0, r.top() + 34.0),
        Align2::LEFT_CENTER,
        order,
        FontId::proportional(11.0),
        p.text_tertiary,
    );
    let rows = doc.inspect();
    if side {
        let mut y = r.top() + 56.0;
        for (label, value) in rows {
            painter.text(
                pos2(r.left() + 14.0, y),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(11.5),
                p.text_secondary,
            );
            let g = crate::ui::widgets::trunc_galley(
                &painter,
                &value,
                FontId::monospace(11.5),
                p.text_primary,
                r.width() - 80.0,
            );
            painter.galley(pos2(r.left() + 66.0, y - g.size().y / 2.0), g, p.text_primary);
            y += 20.0;
        }
    } else {
        // Three columns of four rows to the right of the title.
        let x0 = r.left() + 130.0;
        let col_w = ((r.width() - 140.0) / 3.0).max(80.0);
        for (i, (label, value)) in rows.into_iter().enumerate() {
            let (c, row) = (i / 4, i % 4);
            let x = x0 + c as f32 * col_w;
            let y = r.top() + 16.0 + row as f32 * 22.0;
            painter.text(pos2(x, y), Align2::LEFT_CENTER, label, FontId::proportional(11.5), p.text_secondary);
            let g = crate::ui::widgets::trunc_galley(
                &painter,
                &value,
                FontId::monospace(11.5),
                p.text_primary,
                col_w - 60.0,
            );
            painter.galley(pos2(x + 50.0, y - g.size().y / 2.0), g, p.text_primary);
        }
    }
}
