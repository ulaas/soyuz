//! The hex editor: a virtualized 16-bytes-per-row view over any file size (64 KiB pages
//! read on a worker thread), overwrite editing in the hex or text column, a data
//! inspector, go to offset and find bytes / text. Saving writes the patched bytes to a
//! temp file and renames it over the original.

pub mod io;
pub mod model;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossbeam_channel::Receiver;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, CornerRadius, Event, EventFilter, FontId, Id, Key, Rect, Sense, Stroke, Ui, pos2, vec2};

use io::{PAGE, Pager, SearchReply};
use model::Patches;

use super::widgets::{self, SCROLLBAR};
use crate::services::mailbox::Wake;
use crate::theme::{Palette, icons::ph};

const ROW: u64 = 16;
const TOOLBAR_H: f32 = 34.0;
const BAR_H: f32 = 34.0;
const STATUS_H: f32 = 24.0;
const INSPECTOR_W: f32 = 230.0;
/// Pages kept in memory.
const MAX_PAGES: usize = 256;

#[derive(Default)]
pub struct HexOutput {
    pub save: bool,
}

struct FindBar {
    text: String,
    hex: bool,
    status: Option<String>,
    focus: bool,
}

pub struct HexDoc {
    pub path: PathBuf,
    pub len: u64,
    generation: u64,
    pager: Pager,
    wake: Wake,
    pages: HashMap<u64, Arc<[u8]>>,
    requested: HashSet<u64>,
    error: Option<String>,
    pub patches: Patches,
    pub cursor: u64,
    nibble: u8,
    text_col: bool,
    scroll_row: f64,
    scroll_to_cursor: bool,
    pub read_only: bool,
    /// Opened read-only because the file is too large for the text editor.
    pub too_large_for_text: bool,
    big_endian: bool,
    goto: Option<String>,
    goto_focus: bool,
    find: Option<FindBar>,
    search: Option<(Receiver<SearchReply>, Arc<AtomicBool>, bool)>,
    pub focus_request: bool,
    page_rows: u64,
}

impl HexDoc {
    pub fn new(path: PathBuf, len: u64, wake: Wake) -> Self {
        HexDoc {
            pager: Pager::new(path.clone(), wake.clone()),
            path,
            len,
            generation: 1,
            wake,
            pages: HashMap::new(),
            requested: HashSet::new(),
            error: None,
            patches: Patches::default(),
            cursor: 0,
            nibble: 0,
            text_col: false,
            scroll_row: 0.0,
            scroll_to_cursor: false,
            read_only: false,
            too_large_for_text: false,
            big_endian: false,
            goto: None,
            goto_focus: false,
            find: None,
            search: None,
            focus_request: true,
            page_rows: 16,
        }
    }

    pub fn is_dirty(&self) -> bool {
        !self.patches.is_empty()
    }

    /// The file changed on disk (or was saved): drop cached pages.
    pub fn reload(&mut self, len: u64) {
        self.len = len;
        self.generation += 1;
        self.pages.clear();
        self.requested.clear();
        self.error = None;
        self.cursor = self.cursor.min(len.saturating_sub(1));
    }

    pub fn undo(&mut self) {
        if let Some(off) = self.patches.undo() {
            self.cursor = off;
            self.scroll_to_cursor = true;
        }
    }

    pub fn redo(&mut self) {
        if let Some(off) = self.patches.redo() {
            self.cursor = off;
            self.scroll_to_cursor = true;
        }
    }

    pub fn open_goto(&mut self) {
        self.goto = Some(String::new());
        self.goto_focus = true;
        self.find = None;
    }

    pub fn open_find(&mut self) {
        let hex = self.find.as_ref().is_none_or(|f| f.hex);
        let text = self.find.take().map(|f| f.text).unwrap_or_default();
        self.find = Some(FindBar { text, hex, status: None, focus: true });
        self.goto = None;
    }

    pub fn close_bars(&mut self) -> bool {
        let any = self.find.is_some() || self.goto.is_some();
        self.find = None;
        self.goto = None;
        if let Some((_, cancel, _)) = self.search.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        if any {
            self.focus_request = true;
        }
        any
    }

    pub fn find_step(&mut self, forward: bool) {
        let Some(f) = self.find.as_mut() else {
            self.open_find();
            return;
        };
        match model::parse_needle(&f.text, f.hex) {
            Ok(needle) => {
                if let Some((_, cancel, _)) = self.search.take() {
                    cancel.store(true, Ordering::Relaxed);
                }
                let cancel = Arc::new(AtomicBool::new(false));
                let rx = io::spawn_search(
                    self.path.clone(),
                    self.len,
                    self.patches.map().clone(),
                    needle,
                    self.cursor,
                    forward,
                    cancel.clone(),
                    self.wake.clone(),
                );
                f.status = Some("Searching…".into());
                self.search = Some((rx, cancel, forward));
            }
            Err(e) => f.status = Some(e),
        }
    }

    pub fn go_to(&mut self, off: u64) {
        self.cursor = off.min(self.len.saturating_sub(1));
        self.nibble = 0;
        self.scroll_to_cursor = true;
    }

    /// `SOYUZ_SCRIPT editor:hex=<offset>=<byte>`.
    pub fn devscript_set(&mut self, off: u64, value: u8) {
        self.go_to(off);
        self.set_byte(off, value);
    }

    fn byte(&self, off: u64) -> Option<u8> {
        if off >= self.len {
            return None;
        }
        if let Some(b) = self.patches.get(off) {
            return Some(b);
        }
        self.original(off)
    }

    fn original(&self, off: u64) -> Option<u8> {
        self.pages.get(&(off / PAGE)).and_then(|p| p.get((off % PAGE) as usize).copied())
    }

    fn pump(&mut self) {
        while let Ok(r) = self.pager.rx.try_recv() {
            if r.generation != self.generation {
                continue;
            }
            match r.bytes {
                Ok(b) => {
                    self.pages.insert(r.page, b);
                }
                Err(e) => self.error = Some(e),
            }
        }
        if let Some((rx, _, _)) = &self.search
            && let Ok(res) = rx.try_recv()
        {
            self.search = None;
            let status = match res {
                Ok(Some(off)) => {
                    self.go_to(off);
                    self.text_col = self.find.as_ref().is_some_and(|f| !f.hex);
                    format!("Found at 0x{off:X}")
                }
                Ok(None) => "Not found".into(),
                Err(e) => e,
            };
            if let Some(f) = self.find.as_mut() {
                f.status = Some(status);
            }
        }
    }

    fn ensure_pages(&mut self, first: u64, last: u64) {
        for page in first / PAGE..=last / PAGE {
            if self.requested.insert(page) {
                self.pager.request(self.generation, page);
            }
        }
        if self.pages.len() > MAX_PAGES {
            let keep = first / PAGE;
            let mut far: Vec<u64> = self.pages.keys().copied().collect();
            far.sort_by_key(|p| std::cmp::Reverse(p.abs_diff(keep)));
            for p in far.into_iter().take(self.pages.len() - MAX_PAGES / 2) {
                self.pages.remove(&p);
                self.requested.remove(&p);
            }
        }
    }

    fn set_byte(&mut self, off: u64, value: u8) {
        if self.read_only {
            return;
        }
        if let Some(orig) = self.original(off) {
            self.patches.set(off, orig, value);
        }
    }

    fn handle_events(&mut self, ui: &Ui) {
        let events = ui.input(|i| i.events.clone());
        let rows = self.page_rows.max(1);
        for ev in events {
            match ev {
                Event::Text(t) => {
                    for c in t.chars() {
                        if self.cursor >= self.len {
                            break;
                        }
                        if self.text_col {
                            let mut b = [0u8; 4];
                            for &byte in c.encode_utf8(&mut b).as_bytes() {
                                if self.cursor < self.len {
                                    self.set_byte(self.cursor, byte);
                                    self.cursor = (self.cursor + 1).min(self.len - 1);
                                }
                            }
                        } else if let Some(d) = c.to_digit(16) {
                            let Some(cur) = self.byte(self.cursor) else { continue };
                            let d = d as u8;
                            if self.nibble == 0 {
                                self.set_byte(self.cursor, (cur & 0x0F) | (d << 4));
                                self.nibble = 1;
                            } else {
                                self.set_byte(self.cursor, (cur & 0xF0) | d);
                                self.nibble = 0;
                                self.cursor = (self.cursor + 1).min(self.len.saturating_sub(1));
                            }
                        }
                        self.scroll_to_cursor = true;
                    }
                }
                Event::Copy => {
                    if let Some(b) = self.byte(self.cursor) {
                        ui.ctx().copy_text(format!("{b:02X}"));
                    }
                }
                Event::Key { key, pressed: true, modifiers: m, .. } => {
                    let last = self.len.saturating_sub(1);
                    let c = self.cursor;
                    let moved = match key {
                        Key::ArrowLeft => Some(c.saturating_sub(1)),
                        Key::ArrowRight => Some((c + 1).min(last)),
                        Key::ArrowUp => Some(c.saturating_sub(ROW)),
                        Key::ArrowDown => Some(if c + ROW <= last { c + ROW } else { c }),
                        Key::PageUp => Some(c.saturating_sub(ROW * rows)),
                        Key::PageDown => Some((c + ROW * rows).min(last)),
                        Key::Home if m.command => Some(0),
                        Key::End if m.command => Some(last),
                        Key::Home => Some(c - c % ROW),
                        Key::End => Some((c - c % ROW + ROW - 1).min(last)),
                        Key::Tab => {
                            self.text_col = !self.text_col;
                            self.nibble = 0;
                            None
                        }
                        Key::Z if m.command && m.shift => {
                            self.redo();
                            None
                        }
                        Key::Z if m.command => {
                            self.undo();
                            None
                        }
                        Key::Y if m.command => {
                            self.redo();
                            None
                        }
                        _ => None,
                    };
                    if let Some(to) = moved {
                        self.cursor = to;
                        self.nibble = 0;
                        self.scroll_to_cursor = true;
                    }
                }
                _ => {}
            }
        }
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        rect: Rect,
        p: &Palette,
        font_size: f32,
        focus: &mut Vec<Id>,
        id: Id,
    ) -> HexOutput {
        self.pump();
        let mut out = HexOutput::default();
        let tb = Rect::from_min_size(rect.min, vec2(rect.width(), TOOLBAR_H));
        self.toolbar(ui, tb, p, id, &mut out);
        let mut top = tb.bottom();
        if self.goto.is_some() || self.find.is_some() {
            let r = Rect::from_min_size(pos2(rect.left(), top), vec2(rect.width(), BAR_H));
            self.bar(ui, r, p, id, focus);
            top += BAR_H;
        }
        if self.too_large_for_text && self.read_only {
            let r = Rect::from_min_size(pos2(rect.left() + 8.0, top + 2.0), vec2(rect.width() - 16.0, 30.0));
            if widgets::banner(
                ui,
                r,
                p,
                ph::INFO,
                p.info,
                "Too large for the text editor: opened read-only in hex.",
                &["Allow editing"],
            )
            .is_some()
            {
                self.read_only = false;
            }
            top += 36.0;
        }
        if let Some(e) = &self.error {
            let r = Rect::from_min_size(pos2(rect.left() + 8.0, top + 2.0), vec2(rect.width() - 16.0, 30.0));
            widgets::banner(ui, r, p, ph::WARNING, p.danger, &format!("Couldn't read the file: {e}"), &[]);
            top += 36.0;
        }
        let status = Rect::from_min_max(pos2(rect.left(), rect.bottom() - STATUS_H), rect.max);
        let body = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), status.top()));
        self.body(ui, body, p, font_size, focus, id);
        self.status(ui, status, p);
        out
    }

    fn toolbar(&mut self, ui: &mut Ui, r: Rect, p: &Palette, id: Id, out: &mut HexOutput) {
        let x = std::cell::Cell::new(r.left() + 6.0);
        let btn = |ui: &mut Ui, glyph: &str, tip: &str, enabled: bool, on: bool| -> bool {
            let b = Rect::from_min_size(pos2(x.get(), r.top() + 4.0), vec2(26.0, 26.0));
            x.set(x.get() + 28.0);
            if on {
                ui.painter().rect_filled(b, CornerRadius::same(4), p.accent_subtle);
            }
            widgets::icon_at(ui, b, id.with(("hx", tip)), glyph, tip, enabled, p)
        };
        out.save |= btn(ui, ph::FLOPPY_DISK, "Save (Ctrl+S)", self.is_dirty(), false);
        let (can_undo, can_redo) = (self.patches.can_undo(), self.patches.can_redo());
        if btn(ui, ph::ARROW_COUNTER_CLOCKWISE, "Undo (Ctrl+Z)", can_undo, false) {
            self.undo();
        }
        if btn(ui, ph::ARROW_CLOCKWISE, "Redo (Ctrl+Y)", can_redo, false) {
            self.redo();
        }
        x.set(x.get() + 8.0);
        let goto_on = self.goto.is_some();
        let find_on = self.find.is_some();
        let b = |ui: &mut Ui, glyph: &str, tip: &str, on: bool| -> bool {
            let b = Rect::from_min_size(pos2(x.get(), r.top() + 4.0), vec2(26.0, 26.0));
            x.set(x.get() + 28.0);
            if on {
                ui.painter().rect_filled(b, CornerRadius::same(4), p.accent_subtle);
            }
            widgets::icon_at(ui, b, id.with(("hx2", tip)), glyph, tip, true, p)
        };
        if b(ui, ph::MAGNIFYING_GLASS, "Find bytes or text (Ctrl+F)", find_on) {
            if find_on {
                self.find = None;
            } else {
                self.open_find();
            }
        }
        if b(ui, ph::ARROW_LINE_DOWN, "Go to offset (Ctrl+G)", goto_on) {
            if goto_on {
                self.goto = None;
            } else {
                self.open_goto();
            }
        }
        x.set(x.get() + 8.0);
        let lock = self.read_only;
        if b(ui, if lock { ph::LOCK_SIMPLE } else { ph::LOCK_SIMPLE_OPEN }, "Read-only (no editing)", lock) {
            self.read_only = !self.read_only;
        }
        // Byte order for the inspector.
        let label = if self.big_endian { "Big endian" } else { "Little endian" };
        let br = Rect::from_min_size(pos2(r.right() - 100.0, r.top() + 6.0), vec2(92.0, 22.0));
        let resp = ui.interact(br, id.with("endian"), Sense::click());
        crate::ui::a11y::button(&resp, label);
        ui.painter().rect_filled(br, CornerRadius::same(4), if resp.hovered() { p.bg_surface3 } else { p.bg_surface2 });
        ui.painter().text(br.center(), Align2::CENTER_CENTER, label, FontId::proportional(11.5), p.text_secondary);
        if resp.on_hover_text("Byte order used by the data inspector").clicked() {
            self.big_endian = !self.big_endian;
        }
        ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.border_subtle));
    }

    fn bar(&mut self, ui: &mut Ui, r: Rect, p: &Palette, id: Id, focus: &mut Vec<Id>) {
        ui.painter().rect_filled(r, CornerRadius::ZERO, p.bg_surface2.gamma_multiply(0.5));
        let fr = Rect::from_min_size(
            pos2(r.left() + 8.0, r.top() + 5.0),
            vec2(240.0_f32.min(r.width() - 200.0).max(120.0), 24.0),
        );
        if let Some(text) = self.goto.as_mut() {
            let fid = id.with("hex.goto");
            let resp = widgets::field(ui, fr, fid, text, "Offset: 0x1F, 1Fh or 31, +16", true);
            focus.push(fid);
            if std::mem::take(&mut self.goto_focus) {
                resp.request_focus();
            }
            let hint = format!("0 – 0x{:X}", self.len.saturating_sub(1));
            ui.painter().text(
                pos2(fr.right() + 10.0, r.center().y),
                Align2::LEFT_CENTER,
                hint,
                FontId::proportional(11.5),
                p.text_secondary,
            );
            if (resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter)) {
                match model::parse_offset(text, self.cursor) {
                    Some(off) => {
                        self.goto = None;
                        self.go_to(off);
                        self.focus_request = true;
                    }
                    None => text.clear(),
                }
            }
        } else if let Some(f) = self.find.as_mut() {
            let fid = id.with("hex.find");
            let hint = if f.hex { "Hex bytes, e.g. DE AD BE EF" } else { "Text (UTF-8)" };
            let resp = widgets::field(ui, fr, fid, &mut f.text, hint, true);
            focus.push(fid);
            if std::mem::take(&mut f.focus) {
                resp.request_focus();
            }
            let cr = Rect::from_min_size(pos2(fr.right() + 6.0, r.top() + 6.0), vec2(40.0, 22.0));
            if widgets::chip(
                ui,
                cr,
                id.with("hex.mode"),
                if f.hex { "HEX" } else { "TXT" },
                "Search hex bytes or text",
                true,
                p,
            ) {
                f.hex = !f.hex;
                f.status = None;
            }
            let status = f.status.clone().unwrap_or_default();
            let color = if status.starts_with("Found") { p.success } else { p.text_secondary };
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
            let close = widgets::icon_at(ui, ir(0.0), id.with("hf.close"), ph::X, "Close (Esc)", true, p);
            let next = widgets::icon_at(ui, ir(1.0), id.with("hf.next"), ph::ARROW_DOWN, "Next (Enter / F3)", true, p);
            let prev =
                widgets::icon_at(ui, ir(2.0), id.with("hf.prev"), ph::ARROW_UP, "Previous (Shift+Enter)", true, p);
            if close {
                self.close_bars();
            } else if enter || next || prev {
                self.find_step(!(prev || (enter && back)));
                if enter {
                    resp.request_focus();
                }
            }
        }
        ui.painter().hline(r.x_range(), r.bottom() - 0.5, Stroke::new(1.0, p.border_subtle));
    }

    fn status(&self, ui: &mut Ui, r: Rect, p: &Palette) {
        ui.painter().hline(r.x_range(), r.top() + 0.5, Stroke::new(1.0, p.border_subtle));
        let mut s = format!("Offset 0x{:X} ({})  ·  {} bytes", self.cursor, self.cursor, self.len);
        if !self.patches.is_empty() {
            s.push_str(&format!("  ·  {} changed", self.patches.len()));
        }
        ui.painter().text(
            pos2(r.left() + 10.0, r.center().y),
            Align2::LEFT_CENTER,
            s,
            FontId::proportional(11.5),
            p.text_secondary,
        );
        let mode = if self.read_only { "Read-only" } else { "Overwrite" };
        let col = if self.text_col { "Text column" } else { "Hex column" };
        ui.painter().text(
            pos2(r.right() - 10.0, r.center().y),
            Align2::RIGHT_CENTER,
            format!("{col}  ·  {mode}"),
            FontId::proportional(11.5),
            if self.read_only { p.warning } else { p.text_secondary },
        );
    }

    fn body(&mut self, ui: &mut Ui, body: Rect, p: &Palette, font_size: f32, focus: &mut Vec<Id>, id: Id) {
        let font = FontId::monospace(font_size.clamp(6.0, 40.0));
        let (cw, row_h) =
            ui.ctx().fonts_mut(|f| (f.glyph_width(&font, '0').max(1.0), (f.row_height(&font) * 1.35).round()));
        let digits = super::super::ui::preview::hex::offset_digits(self.len);
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
        let total_rows = self.len.div_ceil(ROW).max(1);
        let visible = (area.height() / row_h).max(1.0) as f64;
        self.page_rows = (visible as u64).saturating_sub(1).max(1);
        // Interaction.
        let tid = id.with("hex.view");
        focus.push(tid);
        let resp = ui.interact(area, tid, Sense::click_and_drag());
        let just_focused = std::mem::take(&mut self.focus_request) && !resp.has_focus();
        if just_focused {
            resp.request_focus();
        }
        let focused = resp.has_focus();
        if focused && !just_focused {
            self.handle_events(ui);
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
            self.scroll_row -= (d.y / row_h) as f64;
        }
        let cur_row = (self.cursor / ROW) as f64;
        if std::mem::take(&mut self.scroll_to_cursor) {
            if cur_row < self.scroll_row {
                self.scroll_row = cur_row;
            } else if cur_row > self.scroll_row + visible - 2.0 {
                self.scroll_row = cur_row - visible + 2.0;
            }
        }
        widgets::vscroll(
            ui,
            vbar,
            id.with("hex.vbar"),
            total_rows as f64 + visible - 1.0,
            visible,
            &mut self.scroll_row,
            p,
        );
        self.scroll_row = self.scroll_row.clamp(0.0, (total_rows as f64 - 1.0).max(0.0));
        let first = self.scroll_row.floor() as u64;
        let n = (visible.ceil() as u64 + 1).min(total_rows - first.min(total_rows));
        if self.len > 0 {
            self.ensure_pages(first * ROW, ((first + n) * ROW).min(self.len) - 1);
        }
        // Column geometry.
        let x_off = area.left() + 10.0;
        let x_hex = x_off + (digits as f32 + 2.0) * cw;
        let hex_x = |i: u64| x_hex + (i as f32 * 3.0 + if i >= 8 { 1.0 } else { 0.0 }) * cw;
        let x_txt = hex_x(16) + 1.5 * cw;
        let origin_y = area.top() - (self.scroll_row - first as f64) as f32 * row_h;
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
            let off = (row * ROW + col).min(self.len.saturating_sub(1));
            self.cursor = off;
            self.nibble = 0;
            self.text_col = text;
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
            if base >= self.len && self.len > 0 {
                break;
            }
            let cy = y + row_h / 2.0;
            let cur_in_row = self.cursor / ROW == row;
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
                if off >= self.len {
                    break;
                }
                let patched = self.patches.get(off).is_some();
                let b = self.byte(off);
                let is_cur = off == self.cursor;
                if is_cur {
                    let hx = Rect::from_min_size(pos2(hex_x(c) - 1.0, y + 1.0), vec2(cw * 2.0 + 2.0, row_h - 2.0));
                    let tx = Rect::from_min_size(pos2(x_txt + c as f32 * cw, y + 1.0), vec2(cw, row_h - 2.0));
                    let (strong, weak) = if self.text_col { (tx, hx) } else { (hx, tx) };
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
                    if focused && !self.text_col && self.nibble == 1 {
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
        if self.len == 0 {
            painter.text(
                area.center(),
                Align2::CENTER_CENTER,
                "Empty file",
                FontId::proportional(12.0),
                p.text_tertiary,
            );
        }
        if inspector.height() > 40.0 {
            self.inspector(ui, inspector, p, side);
        }
    }

    fn inspector(&self, ui: &mut Ui, r: Rect, p: &Palette, side: bool) {
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
            "Data inspector",
            FontId::new(12.5, egui::FontFamily::Name(crate::theme::fonts::SEMIBOLD.into())),
            p.text_primary,
        );
        let order = if self.big_endian { "big endian" } else { "little endian" };
        painter.text(
            pos2(r.left() + 14.0, r.top() + 34.0),
            Align2::LEFT_CENTER,
            order,
            FontId::proportional(11.0),
            p.text_tertiary,
        );
        let bytes: Vec<u8> = (self.cursor..(self.cursor + 8).min(self.len)).map_while(|o| self.byte(o)).collect();
        let rows = model::inspect(&bytes, self.big_endian);
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
}
