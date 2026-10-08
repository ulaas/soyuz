//! The egui terminal widget: input handling (keyboard, mouse, selection, scrollbar)
//! and grid painting.
//!
//! Painting: the term is locked only to copy the visible cells (a snapshot). Each row is
//! turned into background runs plus a few galleys — contiguous cells whose glyph advance
//! equals the cell width share one `LayoutJob` (one section per style); wide chars,
//! combining sequences and glyphs from fallback fonts get their own galley placed at their
//! column so the grid never drifts. Rows are cached by a hash of their resolved content.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point};
use alacritty_terminal::selection::{Selection, SelectionRange};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{Term, TermMode, point_to_viewport, viewport_to_point};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use egui::text::{ByteIndex, LayoutJob, LayoutSection};
use egui::{
    Color32, CursorIcon, Event, EventFilter, FontId, Galley, Id, Key, Modifiers, PointerButton, Pos2, Rect, Sense,
    Stroke, StrokeKind, TextFormat, Ui, Vec2, pos2, vec2,
};

use crate::backend::Listener;
use crate::keys::{self, ALT_IS_META};
use crate::selection::{ClickTracker, row_span, selection_type};
use crate::theme::lerp_rgb;
use crate::{TermResponse, TermTheme, TerminalView, url};

const PAD: f32 = 4.0;
const SCROLLBAR_W: f32 = 10.0;
const BLINK: Duration = Duration::from_millis(530);
/// Stop blinking (and repainting) after this long without input or cursor movement.
const BLINK_TIMEOUT: Duration = Duration::from_secs(30);
const CWD_POLL: Duration = Duration::from_secs(2);

/// Per-terminal UI state (lives in `Terminal`, only touched by the UI thread).
pub(crate) struct ViewState {
    rows: Vec<Option<(u64, RowPaint)>>,
    glyph_ok: HashMap<char, bool>,
    metrics_key: u64,
    atlas_fill: f32,
    click: ClickTracker,
    selecting: bool,
    mouse_down: Option<u8>,
    last_mouse_cell: Option<(usize, usize)>,
    scroll_accum: f32,
    blink_start: Instant,
    last_cursor: Option<(usize, usize)>,
    prev_focused: bool,
    reported_focus: Option<bool>,
    skip_text: bool,
    last_cwd_poll: Option<Instant>,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            rows: Vec::new(),
            glyph_ok: HashMap::new(),
            metrics_key: 0,
            atlas_fill: 0.0,
            click: ClickTracker::default(),
            selecting: false,
            mouse_down: None,
            last_mouse_cell: None,
            scroll_accum: 0.0,
            blink_start: Instant::now(),
            last_cursor: None,
            prev_focused: false,
            reported_focus: None,
            skip_text: false,
            last_cwd_poll: None,
        }
    }
}

#[derive(Clone)]
struct RowPaint {
    bgs: Vec<(u16, u16, Color32)>,
    texts: Vec<TextPiece>,
}

#[derive(Clone)]
struct TextPiece {
    col: u16,
    /// Cells covered (for centering standalone glyphs).
    span: u16,
    galley: Arc<Galley>,
    bold: bool,
    standalone: bool,
}

/// Copy of the visible terminal state (taken under the term lock).
struct Snapshot {
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,
    cursor: Option<(usize, usize)>,
    cursor_shape: CursorShape,
    selection: Option<SelectionRange>,
    display_offset: usize,
    history: usize,
    colors: Colors,
}

impl Snapshot {
    fn take(term: &Term<Listener>) -> Snapshot {
        let cols = term.columns();
        let rows = term.screen_lines();
        let content = term.renderable_content();
        let off = content.display_offset;
        let mut cells = vec![Cell::default(); cols * rows];
        for ind in content.display_iter {
            if let Some(vp) = point_to_viewport(off, ind.point)
                && vp.line < rows
                && vp.column.0 < cols
            {
                cells[vp.line * cols + vp.column.0] = ind.cell.clone();
            }
        }
        let cursor = point_to_viewport(off, content.cursor.point)
            .filter(|p| p.line < rows && p.column.0 < cols)
            .map(|p| (p.line, p.column.0));
        Snapshot {
            cols,
            rows,
            cells,
            cursor,
            cursor_shape: content.cursor.shape,
            selection: content.selection,
            display_offset: off,
            history: term.grid().history_size(),
            colors: *content.colors,
        }
    }

    fn row(&self, r: usize) -> &[Cell] {
        &self.cells[r * self.cols..(r + 1) * self.cols]
    }
}

/// Resolved paint style of one cell.
#[derive(Clone, Copy, PartialEq)]
struct Style {
    fg: Color32,
    bg: Color32,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
}

struct Palette<'a> {
    theme: &'a TermTheme,
    p256: [Color32; 256],
    colors: &'a Colors,
    default_bg: Color32,
}

fn rgb32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

impl Palette<'_> {
    fn color(&self, c: Color) -> Color32 {
        match c {
            Color::Spec(rgb) => rgb32(rgb),
            Color::Indexed(i) => self.colors[usize::from(i)].map_or(self.p256[usize::from(i)], rgb32),
            Color::Named(n) => {
                if let Some(rgb) = self.colors[n] {
                    return rgb32(rgb);
                }
                let idx = n as usize;
                if idx < 16 {
                    return self.theme.ansi[idx];
                }
                match n {
                    NamedColor::Foreground | NamedColor::BrightForeground => self.theme.fg,
                    NamedColor::Background => self.theme.bg,
                    NamedColor::Cursor => self.theme.cursor,
                    NamedColor::DimForeground => self.theme.dim(self.theme.fg),
                    dim => {
                        let base = (dim as usize).saturating_sub(NamedColor::DimBlack as usize).min(7);
                        self.theme.dim(self.theme.ansi[base])
                    }
                }
            }
        }
    }

    fn style(&self, cell: &Cell) -> Style {
        let f = cell.flags;
        let bold = f.contains(Flags::BOLD);
        let mut fg_spec = cell.fg;
        if bold && self.theme.bright_bold {
            fg_spec = match fg_spec {
                Color::Named(n) if (n as usize) < 8 => Color::Named(n.to_bright()),
                Color::Indexed(i) if i < 8 => Color::Indexed(i + 8),
                other => other,
            };
        }
        let mut fg = self.color(fg_spec);
        let mut bg = self.color(cell.bg);
        if f.contains(Flags::DIM) {
            fg = lerp_rgb(bg, fg, 0.66);
        }
        if f.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if f.contains(Flags::HIDDEN) {
            fg = bg;
        }
        Style {
            fg,
            bg,
            bold,
            italic: f.contains(Flags::ITALIC),
            underline: f.intersects(Flags::ALL_UNDERLINES),
            strike: f.contains(Flags::STRIKEOUT),
        }
    }
}

/// Small FNV-1a style hasher for row cache keys.
struct Fnv(u64);
impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn u64(&mut self, v: u64) {
        self.0 = (self.0 ^ v).wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn is_spacer(cell: &Cell) -> bool {
    cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
}

/// Text of a viewport row with a char-index → column map (spacers skipped).
fn row_text(cells: &[Cell]) -> (String, Vec<usize>) {
    let mut s = String::with_capacity(cells.len());
    let mut map = Vec::with_capacity(cells.len());
    for (col, cell) in cells.iter().enumerate() {
        if is_spacer(cell) {
            continue;
        }
        s.push(if cell.c == '\0' { ' ' } else { cell.c });
        map.push(col);
    }
    (s, map)
}

/// Text left / right of the cursor on the cursor line (for the prompt heuristic).
pub(crate) fn cursor_line_text(term: &Term<Listener>) -> (String, String) {
    let grid = term.grid();
    let point = grid.cursor.point;
    let row = &grid[point.line];
    let mut left = String::new();
    let mut right = String::new();
    for col in 0..term.columns() {
        let cell = &row[Column(col)];
        if is_spacer(cell) {
            continue;
        }
        let c = if cell.c == '\0' { ' ' } else { cell.c };
        if col < point.column.0 { left.push(c) } else { right.push(c) }
    }
    (left, right)
}

struct Geometry {
    grid: Rect,
    cell: Vec2,
    cols: usize,
    rows: usize,
}

impl Geometry {
    fn cell_pos(&self, row: usize, col: usize) -> Pos2 {
        pos2(self.grid.min.x + col as f32 * self.cell.x, self.grid.min.y + row as f32 * self.cell.y)
    }

    /// Viewport cell under `pos` (clamped) and the side of the cell.
    fn cell_at(&self, pos: Pos2) -> (usize, usize, Direction) {
        let x = (pos.x - self.grid.min.x).max(0.0) / self.cell.x;
        let y = (pos.y - self.grid.min.y).max(0.0) / self.cell.y;
        let col = (x as usize).min(self.cols.saturating_sub(1));
        let row = (y as usize).min(self.rows.saturating_sub(1));
        let side = if x >= self.cols as f32 || x - col as f32 > 0.5 { Direction::Right } else { Direction::Left };
        (row, col, side)
    }
}

/// A link under the pointer: the URL and the viewport spans `(row, col_start, col_end_inclusive)`.
struct Link {
    url: String,
    spans: Vec<(usize, usize, usize)>,
}

fn link_at(snap: &Snapshot, row: usize, col: usize) -> Option<Link> {
    let cells = snap.row(row);
    // OSC 8 hyperlinks first.
    if let Some(h) = cells[col].hyperlink() {
        let same = |c: &Cell| c.hyperlink().is_some_and(|x| x.id() == h.id());
        let mut a = col;
        while a > 0 && same(&cells[a - 1]) {
            a -= 1;
        }
        let mut b = col;
        while b + 1 < cells.len() && same(&cells[b + 1]) {
            b += 1;
        }
        return Some(Link { url: h.uri().to_owned(), spans: vec![(row, a, b)] });
    }
    // Plain-text URLs, joining soft-wrapped rows.
    let wraps = |r: usize| snap.row(r).last().is_some_and(|c| c.flags.contains(Flags::WRAPLINE));
    let mut first = row;
    while first > 0 && wraps(first - 1) {
        first -= 1;
    }
    let mut last = row;
    while last + 1 < snap.rows && wraps(last) {
        last += 1;
    }
    let mut text = String::new();
    let mut map: Vec<(usize, usize)> = Vec::new();
    for r in first..=last {
        let (s, m) = row_text(snap.row(r));
        text.push_str(&s);
        map.extend(m.into_iter().map(|c| (r, c)));
    }
    let target = map.iter().position(|&(r, c)| r == row && c == col)?;
    let m = url::find_urls(&text).into_iter().find(|m| m.start <= target && target < m.end)?;
    let mut spans: Vec<(usize, usize, usize)> = Vec::new();
    for &(r, c) in &map[m.start..m.end] {
        match spans.last_mut() {
            Some(s) if s.0 == r => s.2 = c,
            _ => spans.push((r, c, c)),
        }
    }
    Some(Link { url: m.url, spans })
}

pub(crate) fn show(ui: &mut Ui, view: TerminalView<'_>) -> TermResponse {
    let term = view.term;
    let backend = &term.backend;
    let shared = &backend.shared;
    shared.repaint_pending.store(false, std::sync::atomic::Ordering::Release);

    let fallback_theme;
    let theme = match view.theme {
        Some(t) => t,
        None => {
            fallback_theme = TermTheme::default();
            &fallback_theme
        }
    };
    shared.set_theme(theme);
    let font = view.font.clone().unwrap_or_else(|| egui::TextStyle::Monospace.resolve(ui.style()));
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    let (cell_w, cell_h, atlas_fill, fonts_identity) = ctx.fonts_mut(|f| {
        // New font definitions (a `[fonts]` change re-installs them) or new text options
        // (theme switch, text gamma) make egui rebuild its atlas: cached galleys go stale.
        let data = f.definitions().font_data.values().next().map_or(0, |d| Arc::as_ptr(d) as *const () as usize);
        let t = f.options().color_transfer_function;
        let options = (u64::from(t.to_gamma().to_bits()) << 1)
            | u64::from(t == egui::epaint::FontColorTransferFunction::default());
        (f.glyph_width(&font, 'M'), f.row_height(&font), f.font_atlas_fill_ratio(), (data, options))
    });
    let cell_w = cell_w.max(1.0);
    let cell_h = cell_h.max(1.0);

    let id = Id::new(("soyuz-term", backend.id));
    let rect = ui.available_rect_before_wrap();
    let response = ui.interact(rect, id, Sense::click_and_drag());
    ui.advance_cursor_after_rect(rect);

    let mut vs = term.view.lock();

    // ---- focus -------------------------------------------------------------------------
    if view.focused {
        let nobody = ctx.memory(|m| m.focused().is_none());
        if !vs.prev_focused || nobody {
            response.request_focus();
        }
        ctx.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true },
            );
        });
    } else if response.has_focus() {
        response.surrender_focus();
    }
    vs.prev_focused = view.focused;

    // ---- geometry + resize ---------------------------------------------------------------
    let inner = rect.shrink(PAD);
    let sb_rect = Rect::from_min_max(pos2(rect.max.x - SCROLLBAR_W, rect.min.y), rect.max);
    let grid_origin = pos2((inner.min.x * ppp).round() / ppp, (inner.min.y * ppp).round() / ppp);
    let avail_w = (sb_rect.min.x - 2.0 - grid_origin.x).max(0.0);
    let avail_h = (inner.max.y - grid_origin.y).max(0.0);
    let cols = ((avail_w / cell_w).floor() as usize).clamp(2, 1000);
    let rows = ((avail_h / cell_h).floor() as usize).clamp(1, 500);
    if rect.width() > 0.0 && rect.height() > 0.0 {
        term.resize(cols as u16, rows as u16, cell_w.round() as u16, cell_h.round() as u16);
    }

    let metrics_key = {
        let mut h = Fnv::new();
        h.u64(u64::from(cell_w.to_bits()));
        h.u64(u64::from(cell_h.to_bits()));
        h.u64(u64::from(ppp.to_bits()));
        h.u64(u64::from(font.size.to_bits()));
        for b in font.family.to_string().bytes() {
            h.u64(u64::from(b));
        }
        h.u64(fonts_identity.0 as u64);
        h.u64(fonts_identity.1);
        h.0
    };
    // A shrinking atlas fill ratio means egui recreated the font atlas: cached galleys are stale.
    if metrics_key != vs.metrics_key || atlas_fill < vs.atlas_fill {
        vs.rows.clear();
        vs.glyph_ok.clear();
        vs.metrics_key = metrics_key;
    }
    vs.atlas_fill = atlas_fill;

    let mut out = TermResponse {
        response: response.clone(),
        wants_focus: false,
        link_clicked: None,
        escape_on_empty_line: false,
    };

    // ---- input -------------------------------------------------------------------------
    let (events, mods_now, time, pointer_pos, primary_down) =
        ui.input(|i| (i.events.clone(), i.modifiers, i.time, i.pointer.latest_pos(), i.pointer.primary_down()));
    let (mode, display_offset, screen_lines, history) = {
        let t = backend.term.lock();
        (*t.mode(), t.grid().display_offset(), t.screen_lines(), t.grid().history_size())
    };
    // Geometry for mouse mapping uses the term's current size (resize may lag a frame).
    let geo = Geometry {
        grid: Rect::from_min_size(grid_origin, vec2(cols as f32 * cell_w, rows as f32 * cell_h)),
        cell: vec2(cell_w, cell_h),
        cols: cols.min(backend.term.lock().columns()),
        rows: rows.min(screen_lines),
    };
    let mut consumed = vec![false; events.len()];
    let mut input: Vec<u8> = Vec::new();
    let keyboard_active = view.focused && (response.has_focus() || ctx.memory(|m| m.focused().is_none()));

    if keyboard_active {
        for (idx, ev) in events.iter().enumerate() {
            let skip = std::mem::take(&mut vs.skip_text);
            match ev {
                Event::Key { key, pressed: true, modifiers, .. } => {
                    let m = *modifiers;
                    if m.shift && !m.ctrl && !m.alt && !mode.contains(TermMode::ALT_SCREEN) {
                        let scroll = match key {
                            Key::PageUp => Some(Scroll::PageUp),
                            Key::PageDown => Some(Scroll::PageDown),
                            _ => None,
                        };
                        if let Some(s) = scroll {
                            backend.term.lock().scroll_display(s);
                            consumed[idx] = true;
                            continue;
                        }
                    }
                    if *key == Key::Escape && m.is_none() && !mode.contains(TermMode::ALT_SCREEN) {
                        let (left, right) = cursor_line_text(&backend.term.lock());
                        if crate::pty::looks_like_prompt(&left, &right) {
                            out.escape_on_empty_line = true;
                            consumed[idx] = true;
                            continue;
                        }
                    }
                    if let Some(bytes) = keys::encode_key(*key, m, mode) {
                        input.extend_from_slice(&bytes);
                        consumed[idx] = true;
                        vs.skip_text = keys::produces_text_too(*key, m);
                    }
                }
                Event::Key { pressed: false, .. } => {
                    vs.skip_text = skip;
                }
                Event::Text(text) => {
                    consumed[idx] = true;
                    if skip {
                        continue;
                    }
                    if mods_now.alt && !mods_now.ctrl && ALT_IS_META {
                        input.push(0x1b);
                    }
                    input.extend_from_slice(text.as_bytes());
                }
                Event::Ime(egui::ImeEvent::Commit(text)) => {
                    consumed[idx] = true;
                    input.extend_from_slice(text.as_bytes());
                }
                Event::Paste(text) => {
                    consumed[idx] = true;
                    let bytes = keys::paste_bytes(text, mode.contains(TermMode::BRACKETED_PASTE));
                    input.extend_from_slice(&bytes);
                }
                Event::Copy => {
                    consumed[idx] = true;
                    let selected = term.selected_text().filter(|s| !s.is_empty());
                    // Ctrl+Shift+C copies; plain Ctrl+C copies (and clears) only when
                    // something is selected, otherwise it is SIGINT.
                    match selected {
                        Some(text) => {
                            ctx.copy_text(text);
                            if !mods_now.shift {
                                term.clear_selection();
                            }
                        }
                        None if mods_now.shift => {}
                        None => input.push(0x03),
                    }
                }
                Event::Cut => {
                    consumed[idx] = true;
                    if mods_now.ctrl {
                        input.push(0x18);
                    } else if let Some(b) = keys::encode_key(Key::Delete, mods_now, mode) {
                        input.extend_from_slice(&b); // Shift+Delete on Windows
                    }
                }
                _ => vs.skip_text = skip,
            }
        }
    }

    // ---- mouse -------------------------------------------------------------------------
    let sb_resp = ui.interact(sb_rect, id.with("scrollbar"), Sense::click_and_drag());
    let hovered = response.contains_pointer() && !sb_resp.contains_pointer();
    let link_mod = |m: Modifiers| m.command;
    let mouse_mode = mode.intersects(TermMode::MOUSE_MODE) && !mods_now.shift;
    let snapshot_needed_for_links = hovered && link_mod(mods_now);
    let mut selection_changed = false;

    for ev in &events {
        match ev {
            Event::PointerButton { pos, button, pressed, modifiers } => {
                let inside = geo.grid.expand(PAD).contains(*pos) && !sb_rect.contains(*pos);
                if *pressed && !inside {
                    continue;
                }
                let (row, col, side) = geo.cell_at(*pos);
                if *pressed {
                    out.wants_focus |= !view.focused;
                }
                // Ctrl+click on a link.
                if *pressed && *button == PointerButton::Primary && link_mod(*modifiers) {
                    let snap = Snapshot::take(&backend.term.lock());
                    if let Some(link) = link_at(&snap, row, col) {
                        out.link_clicked = Some(link.url);
                        continue;
                    }
                }
                if mouse_mode && !modifiers.shift {
                    let code = match button {
                        PointerButton::Primary => keys::MOUSE_LEFT,
                        PointerButton::Middle => keys::MOUSE_MIDDLE,
                        PointerButton::Secondary => keys::MOUSE_RIGHT,
                        _ => continue,
                    };
                    if !*pressed && vs.mouse_down.is_none() {
                        continue;
                    }
                    vs.mouse_down = if *pressed { Some(code) } else { None };
                    vs.last_mouse_cell = Some((row, col));
                    if let Some(b) = keys::encode_mouse(code, *pressed, false, col, row, *modifiers, mode) {
                        input.extend_from_slice(&b);
                    }
                    continue;
                }
                if *button != PointerButton::Primary {
                    continue;
                }
                let point = viewport_to_point(display_offset, Point::new(row, Column(col)));
                if *pressed {
                    let clicks = vs.click.register(time, (point.line.0, col));
                    let mut t = backend.term.lock();
                    match t.selection.as_mut() {
                        Some(sel) if modifiers.shift && clicks == 1 => sel.update(point, side),
                        _ => t.selection = Some(Selection::new(selection_type(clicks), point, side)),
                    }
                    vs.selecting = true;
                    selection_changed = true;
                } else if vs.selecting {
                    vs.selecting = false;
                    let mut t = backend.term.lock();
                    if t.selection.as_ref().is_some_and(|s| s.is_empty()) {
                        t.selection = None;
                    }
                }
            }
            Event::PointerMoved(pos) => {
                if vs.selecting {
                    let (row, col, side) = geo.cell_at(*pos);
                    let point = viewport_to_point(display_offset, Point::new(row, Column(col)));
                    if let Some(sel) = backend.term.lock().selection.as_mut() {
                        sel.update(point, side);
                    }
                    selection_changed = true;
                } else if mouse_mode && geo.grid.contains(*pos) {
                    let motion = mode.contains(TermMode::MOUSE_MOTION)
                        || (mode.contains(TermMode::MOUSE_DRAG) && vs.mouse_down.is_some());
                    let (row, col, _) = geo.cell_at(*pos);
                    if motion && vs.last_mouse_cell != Some((row, col)) {
                        vs.last_mouse_cell = Some((row, col));
                        let code = vs.mouse_down.unwrap_or(keys::MOUSE_NONE);
                        if let Some(b) = keys::encode_mouse(code, true, true, col, row, mods_now, mode) {
                            input.extend_from_slice(&b);
                        }
                    }
                }
            }
            Event::MouseWheel { unit, delta, modifiers, .. } if hovered => {
                let lines = match unit {
                    egui::MouseWheelUnit::Line => delta.y * 3.0,
                    egui::MouseWheelUnit::Point => delta.y / cell_h,
                    egui::MouseWheelUnit::Page => delta.y * screen_lines as f32,
                };
                vs.scroll_accum += lines;
                let n = vs.scroll_accum.trunc();
                vs.scroll_accum -= n;
                let n = n as i32;
                if n == 0 {
                    continue;
                }
                if mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift {
                    let (row, col, _) = geo.cell_at(pointer_pos.unwrap_or(geo.grid.min));
                    let code = if n > 0 { keys::MOUSE_WHEEL_UP } else { keys::MOUSE_WHEEL_DOWN };
                    for _ in 0..n.unsigned_abs() {
                        if let Some(b) = keys::encode_mouse(code, true, false, col, row, *modifiers, mode) {
                            input.extend_from_slice(&b);
                        }
                    }
                } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) && !modifiers.shift {
                    let key = if n > 0 { Key::ArrowUp } else { Key::ArrowDown };
                    for _ in 0..n.unsigned_abs() {
                        if let Some(b) = keys::encode_key(key, Modifiers::NONE, mode) {
                            input.extend_from_slice(&b);
                        }
                    }
                } else {
                    backend.term.lock().scroll_display(Scroll::Delta(n));
                }
            }
            _ => {}
        }
    }
    if vs.selecting && !primary_down {
        vs.selecting = false; // release happened outside our event stream
    }
    // Auto-scroll while drag-selecting past the top/bottom edge.
    if vs.selecting
        && let Some(p) = pointer_pos
    {
        let dir = if p.y < geo.grid.min.y {
            1
        } else if p.y > geo.grid.max.y {
            -1
        } else {
            0
        };
        if dir != 0 && !mode.contains(TermMode::ALT_SCREEN) {
            let mut t = backend.term.lock();
            t.scroll_display(Scroll::Delta(dir));
            let off = t.grid().display_offset();
            let (row, col, side) = geo.cell_at(p);
            let point = viewport_to_point(off, Point::new(row, Column(col)));
            if let Some(sel) = t.selection.as_mut() {
                sel.update(point, side);
            }
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    // Scrollbar interaction: the thumb centre follows the pointer.
    if history > 0
        && (sb_resp.dragged() || sb_resp.clicked())
        && let Some(p) = sb_resp.interact_pointer_pos()
    {
        let total = (history + screen_lines) as f32;
        let thumb_h = (sb_rect.height() * screen_lines as f32 / total).max(20.0).min(sb_rect.height());
        let track = (sb_rect.height() - thumb_h).max(1.0);
        let f = ((sb_rect.max.y - thumb_h / 2.0 - p.y) / track).clamp(0.0, 1.0);
        let target = (f * history as f32).round() as i32;
        let mut t = backend.term.lock();
        let cur = t.grid().display_offset() as i32;
        if target != cur {
            t.scroll_display(Scroll::Delta(target - cur));
        }
    }

    // Focus in/out reports.
    if mode.contains(TermMode::FOCUS_IN_OUT) && vs.reported_focus != Some(view.focused) {
        if vs.reported_focus.is_some() {
            input.extend_from_slice(keys::focus_report(view.focused));
        }
        vs.reported_focus = Some(view.focused);
    }
    backend.term.lock().is_focused = view.focused;

    if !input.is_empty() {
        term.write_input(input);
        vs.blink_start = Instant::now();
    }
    if consumed.iter().any(|&c| c) {
        ui.input_mut(|i| {
            let mut k = 0;
            i.events.retain(|_| {
                let keep = !consumed.get(k).copied().unwrap_or(false);
                k += 1;
                keep
            });
        });
    }

    // Fallback cwd polling (Linux /proc) while visible.
    if let Some(pid) = backend.pid
        && vs.last_cwd_poll.is_none_or(|t| t.elapsed() >= CWD_POLL)
    {
        vs.last_cwd_poll = Some(Instant::now());
        if let Some(cwd) = crate::pty::process_cwd(pid) {
            shared.set_cwd(cwd);
        }
    }

    // ---- paint -------------------------------------------------------------------------
    let snap = Snapshot::take(&backend.term.lock());
    let pal = Palette {
        theme,
        p256: theme.palette256(),
        colors: &snap.colors,
        default_bg: snap.colors[NamedColor::Background].map_or(theme.bg, rgb32),
    };
    let geo = Geometry {
        grid: Rect::from_min_size(grid_origin, vec2(snap.cols as f32 * cell_w, snap.rows as f32 * cell_h)),
        cell: vec2(cell_w, cell_h),
        cols: snap.cols,
        rows: snap.rows,
    };
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, pal.default_bg);

    if vs.rows.len() != snap.rows {
        vs.rows.resize(snap.rows, None);
    }
    let mut styles: Vec<Style> = Vec::with_capacity(snap.cols);
    for r in 0..snap.rows {
        let cells = snap.row(r);
        styles.clear();
        let mut h = Fnv::new();
        h.u64(snap.cols as u64);
        for cell in cells {
            let st = pal.style(cell);
            h.u64(u64::from(cell.c));
            h.u64(
                u64::from(u32::from_le_bytes(st.fg.to_array())) << 32 | u64::from(u32::from_le_bytes(st.bg.to_array())),
            );
            h.u64(u64::from(cell.flags.bits()));
            if let Some(zw) = cell.zerowidth() {
                for c in zw {
                    h.u64(u64::from(*c) | 1 << 40);
                }
            }
            styles.push(st);
        }
        let key = h.0;
        let cached = matches!(&vs.rows[r], Some((k, _)) if *k == key);
        if !cached {
            let paint = build_row(&mut vs.glyph_ok, &ctx, &painter, &font, cell_w, cells, &styles, pal.default_bg);
            vs.rows[r] = Some((key, paint));
        }
        let Some((_, paint)) = &vs.rows[r] else { continue };
        let y = geo.grid.min.y + r as f32 * cell_h;
        for &(c0, c1, color) in &paint.bgs {
            let x0 = geo.grid.min.x + f32::from(c0) * cell_w;
            let x1 = geo.grid.min.x + f32::from(c1) * cell_w;
            painter.rect_filled(Rect::from_min_max(pos2(x0, y), pos2(x1, y + cell_h)), 0.0, color);
        }
        for piece in &paint.texts {
            let mut x = geo.grid.min.x + f32::from(piece.col) * cell_w;
            if piece.standalone {
                let span_w = f32::from(piece.span) * cell_w;
                x += ((span_w - piece.galley.size().x) / 2.0).max(0.0);
            }
            let pos = pos2(x, y);
            painter.galley(pos, piece.galley.clone(), theme.fg);
            if piece.bold {
                painter.galley(pos + vec2(1.0 / ppp, 0.0), piece.galley.clone(), theme.fg);
            }
        }
    }

    // Selection overlay.
    if let Some(range) = &snap.selection {
        for r in 0..snap.rows {
            let line = Line(r as i32 - snap.display_offset as i32);
            if let Some((c0, c1)) = row_span(range, line, snap.cols - 1) {
                let a = geo.cell_pos(r, c0);
                let b = geo.cell_pos(r, c1 + 1) + vec2(0.0, cell_h);
                painter.rect_filled(Rect::from_min_max(a, b), 0.0, theme.selection);
            }
        }
    }

    // Cursor.
    let cursor_color = snap.colors[NamedColor::Cursor].map_or(theme.cursor, rgb32);
    if vs.last_cursor != snap.cursor {
        vs.last_cursor = snap.cursor;
        vs.blink_start = Instant::now();
    }
    if let Some((row, col)) = snap.cursor
        && snap.cursor_shape != CursorShape::Hidden
        && snap.display_offset == 0
    {
        let cell = &snap.row(row)[col];
        let wide = cell.flags.contains(Flags::WIDE_CHAR);
        let w = if wide { 2.0 * cell_w } else { cell_w };
        let r = Rect::from_min_size(geo.cell_pos(row, col), vec2(w, cell_h));
        let since = vs.blink_start.elapsed();
        let blinking = view.focused && since < BLINK_TIMEOUT;
        let visible = !blinking || (since.as_millis() / BLINK.as_millis()).is_multiple_of(2);
        if blinking {
            let phase = Duration::from_millis((since.as_millis() % BLINK.as_millis()) as u64);
            ctx.request_repaint_after(BLINK.saturating_sub(phase));
        }
        let thin = (2.0_f32).max(1.0 / ppp);
        if !view.focused {
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, cursor_color), StrokeKind::Inside);
        } else if visible {
            match snap.cursor_shape {
                CursorShape::Block => {
                    painter.rect_filled(r, 0.0, cursor_color);
                    if !cell.c.is_whitespace() && cell.c != '\0' {
                        let mut s = String::from(cell.c);
                        if let Some(zw) = cell.zerowidth() {
                            s.extend(zw);
                        }
                        let g = painter.layout_no_wrap(s, font.clone(), pal.default_bg);
                        painter.galley(r.min, g, pal.default_bg);
                    }
                }
                CursorShape::Beam => {
                    painter.rect_filled(Rect::from_min_size(r.min, vec2(thin, cell_h)), 0.0, cursor_color);
                }
                CursorShape::Underline => {
                    let a = pos2(r.min.x, r.max.y - thin);
                    painter.rect_filled(Rect::from_min_max(a, r.max), 0.0, cursor_color);
                }
                CursorShape::HollowBlock | CursorShape::Hidden => {
                    painter.rect_stroke(r, 0.0, Stroke::new(1.0, cursor_color), StrokeKind::Inside);
                }
            }
        }
        if view.focused {
            ctx.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    purpose: egui::IMEPurpose::Terminal,
                    rect,
                    cursor_rect: r,
                    should_interrupt_composition: false,
                });
            });
        }
    }

    // Link hover (Ctrl/Cmd held).
    let mut icon = if hovered && !mouse_mode { Some(CursorIcon::Text) } else { None };
    if snapshot_needed_for_links
        && let Some(p) = pointer_pos
        && geo.grid.contains(p)
    {
        let (row, col, _) = geo.cell_at(p);
        if let Some(link) = link_at(&snap, row, col) {
            for (r, c0, c1) in link.spans {
                let y = geo.grid.min.y + (r + 1) as f32 * cell_h - 1.0;
                let x0 = geo.grid.min.x + c0 as f32 * cell_w;
                let x1 = geo.grid.min.x + (c1 + 1) as f32 * cell_w;
                painter.hline(x0..=x1, y, Stroke::new(1.0, theme.cursor));
            }
            icon = Some(CursorIcon::PointingHand);
        }
    }
    if let Some(icon) = icon {
        ctx.set_cursor_icon(icon);
    }

    // Scrollbar.
    if snap.history > 0 {
        let total = (snap.history + snap.rows) as f32;
        let thumb_h = (sb_rect.height() * snap.rows as f32 / total).max(20.0).min(sb_rect.height());
        let track = sb_rect.height() - thumb_h;
        let f = snap.display_offset as f32 / snap.history as f32;
        let top = sb_rect.min.y + track * (1.0 - f);
        let active = sb_resp.hovered() || sb_resp.dragged();
        let alpha = if active {
            0.45
        } else if snap.display_offset > 0 || hovered {
            0.25
        } else {
            0.12
        };
        let thumb = Rect::from_min_size(pos2(sb_rect.min.x + 3.0, top + 2.0), vec2(SCROLLBAR_W - 5.0, thumb_h - 4.0));
        painter.rect_filled(thumb, 3.0, theme.fg.gamma_multiply(alpha));
    }

    if selection_changed {
        ctx.request_repaint();
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn build_row(
    glyph_ok: &mut HashMap<char, bool>,
    ctx: &egui::Context,
    painter: &egui::Painter,
    font: &FontId,
    cell_w: f32,
    cells: &[Cell],
    styles: &[Style],
    default_bg: Color32,
) -> RowPaint {
    let mut bgs: Vec<(u16, u16, Color32)> = Vec::new();
    for (col, st) in styles.iter().enumerate() {
        if st.bg == default_bg {
            continue;
        }
        let col = col as u16;
        match bgs.last_mut() {
            Some(last) if last.1 == col && last.2 == st.bg => last.1 = col + 1,
            _ => bgs.push((col, col + 1, st.bg)),
        }
    }

    let mut texts = Vec::new();
    // Current shared job: (start col, bold, job).
    let mut job: Option<(u16, bool, LayoutJob)> = None;
    let flush = |job: &mut Option<(u16, bool, LayoutJob)>, texts: &mut Vec<TextPiece>| {
        if let Some((col, bold, j)) = job.take() {
            if j.text.trim_end().is_empty() && j.sections.iter().all(|s| s.format.underline == Stroke::NONE) {
                return;
            }
            let galley = painter.layout_job(j);
            texts.push(TextPiece { col, span: 0, galley, bold, standalone: false });
        }
    };
    let fmt = |st: &Style| TextFormat {
        font_id: font.clone(),
        color: st.fg,
        italics: st.italic,
        underline: if st.underline { Stroke::new(1.0, st.fg) } else { Stroke::NONE },
        strikethrough: if st.strike { Stroke::new(1.0, st.fg) } else { Stroke::NONE },
        ..Default::default()
    };

    // Trailing blanks need no glyphs.
    let end = cells
        .iter()
        .zip(styles)
        .rposition(|(c, st)| !matches!(c.c, ' ' | ' ') || st.underline || st.strike)
        .map_or(0, |i| i + 1);
    for (col, (cell, st)) in cells.iter().zip(styles).enumerate().take(end) {
        if is_spacer(cell) {
            continue;
        }
        let c = if cell.c == '\0' || cell.flags.contains(Flags::HIDDEN) { ' ' } else { cell.c };
        let wide = cell.flags.contains(Flags::WIDE_CHAR);
        let zw = cell.zerowidth().filter(|z| !z.is_empty());
        let decorated = st.underline || st.strike;
        if c == ' ' && !decorated && job.is_none() {
            continue; // leading blanks need no glyphs
        }
        let safe = !wide
            && zw.is_none()
            && (c.is_ascii()
                || *glyph_ok.entry(c).or_insert_with(|| {
                    let w = ctx.fonts_mut(|f| f.glyph_width(font, c));
                    (w - cell_w).abs() < 0.5
                }));
        if !safe {
            flush(&mut job, &mut texts);
            let mut s = String::from(c);
            if let Some(z) = zw {
                s.extend(z);
            }
            let mut j = LayoutJob::default();
            j.append(&s, 0.0, fmt(st));
            let galley = painter.layout_job(j);
            texts.push(TextPiece {
                col: col as u16,
                span: if wide { 2 } else { 1 },
                galley,
                bold: st.bold,
                standalone: true,
            });
            continue;
        }
        if job.as_ref().is_some_and(|(_, bold, _)| *bold != st.bold) {
            flush(&mut job, &mut texts);
            if c == ' ' && !decorated {
                continue;
            }
        }
        let (_, _, j) = job.get_or_insert_with(|| (col as u16, st.bold, LayoutJob::default()));
        let start = j.text.len();
        j.text.push(c);
        let end = j.text.len();
        let format = fmt(st);
        match j.sections.last_mut() {
            Some(last) if last.format == format && last.byte_range.end.0 == start => {
                last.byte_range.end = ByteIndex(end)
            }
            _ => j.sections.push(LayoutSection {
                leading_space: 0.0,
                byte_range: ByteIndex(start)..ByteIndex(end),
                format,
            }),
        }
    }
    flush(&mut job, &mut texts);
    RowPaint { bgs, texts }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TermSettings, Terminal};
    use egui::{RawInput, Rect as ERect};

    struct Harness {
        ctx: egui::Context,
        term: Terminal,
        font: FontId,
    }

    impl Harness {
        fn new() -> Harness {
            let (shell, args): (&str, Vec<String>) =
                if cfg!(windows) { ("cmd.exe", vec!["/q".into()]) } else { ("sh", vec![]) };
            let ctx = egui::Context::default();
            let settings = TermSettings {
                shell: Some(shell.into()),
                args,
                shell_integration: false,
                cwd: std::env::temp_dir(),
                env: vec![("PS1".into(), "$ ".into())],
                ..Default::default()
            };
            let term = Terminal::spawn(settings, ctx.clone(), 42).expect("spawn");
            Harness { ctx, term, font: FontId::monospace(14.0) }
        }

        fn frame(&self, events: Vec<Event>) -> TermResponse {
            let raw = RawInput {
                screen_rect: Some(ERect::from_min_size(Pos2::ZERO, vec2(800.0, 400.0))),
                events,
                ..Default::default()
            };
            let mut resp = None;
            let mut out = self.ctx.run_ui(raw, |ui| {
                resp = Some(TerminalView::new(&self.term).focused(true).font(self.font.clone()).show(ui));
            });
            out.textures_delta.clear();
            resp.expect("shown")
        }

        fn cell(&self) -> Vec2 {
            let f = self.font.clone();
            self.ctx.fonts_mut(|fonts| vec2(fonts.glyph_width(&f, 'M'), fonts.row_height(&f)))
        }

        /// Centre of viewport cell (row, col).
        fn pos(&self, row: usize, col: usize) -> Pos2 {
            let c = self.cell();
            pos2(PAD + (col as f32 + 0.5) * c.x, PAD + (row as f32 + 0.5) * c.y)
        }

        fn wait_for(&self, what: &str, min_count: usize) -> bool {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(15) {
                self.frame(vec![]);
                if self.term.screen_text().matches(what).count() >= min_count {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            false
        }
    }

    fn key(key: Key) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    #[test]
    fn widget_typing_selection_and_scroll() {
        let h = Harness::new();
        h.frame(vec![]);
        std::thread::sleep(Duration::from_millis(500));
        // Typing goes through Event::Text + Enter.
        h.frame(vec![Event::Text("echo soyuz-typed".into()), key(Key::Enter)]);
        assert!(h.wait_for("soyuz-typed", 2), "typed command not echoed/run:\n{}", h.term.screen_text());

        // Drag-select the first five cells of the row that holds the output.
        let text = h.term.screen_text();
        let row = text.lines().position(|l| l.starts_with("soyuz-typed")).expect("output row");
        // End in the right half of cell 4 so that cell is included.
        let (a, b) = (h.pos(row, 0), h.pos(row, 4) + vec2(h.cell().x * 0.3, 0.0));
        let press = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        h.frame(vec![Event::PointerMoved(a), press(a, true)]);
        h.frame(vec![Event::PointerMoved(b)]);
        h.frame(vec![press(b, false)]);
        assert_eq!(h.term.selected_text().as_deref(), Some("soyuz"));

        // Double-click selects the whole word (semantic selection).
        // (A different cell than the drag start, so it does not continue that click.)
        let c = h.pos(row, 2);
        h.frame(vec![press(c, true), press(c, false), press(c, true), press(c, false)]);
        assert_eq!(h.term.selected_text().as_deref(), Some("soyuz-typed"));
        h.term.clear_selection();

        // Produce scrollback, then scroll it with the wheel.
        let cmd = if cfg!(windows) {
            "for /L %i in (1,1,80) do @echo line%i"
        } else {
            "for i in $(seq 1 80); do echo line$i; done"
        };
        h.frame(vec![Event::Text(cmd.into()), key(Key::Enter)]);
        assert!(h.wait_for("line80", 1), "{}", h.term.screen_text());
        let wheel = Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, 2.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        };
        h.frame(vec![Event::PointerMoved(h.pos(3, 3)), wheel]);
        let offset = h.term.backend.term.lock().grid().display_offset();
        assert_eq!(offset, 6, "3 lines per wheel notch");
        // Typing snaps back to the bottom.
        h.frame(vec![Event::Text(" ".into())]);
        assert_eq!(h.term.backend.term.lock().grid().display_offset(), 0);
        h.term.kill();
    }

    #[test]
    fn grid_resizes_to_widget() {
        let h = Harness::new();
        h.frame(vec![]);
        h.frame(vec![]);
        let c = h.cell();
        let t = h.term.backend.term.lock();
        let want_cols = ((800.0 - 2.0 * PAD - SCROLLBAR_W - 2.0 + PAD) / c.x).floor() as usize;
        assert!(t.columns().abs_diff(want_cols) <= 1, "{} vs {want_cols}", t.columns());
        assert_eq!(t.screen_lines(), ((400.0 - 2.0 * PAD) / c.y).floor() as usize);
    }
}
