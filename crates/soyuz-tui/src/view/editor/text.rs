//! The text editor's view ([`soyuz_appcore::editors::text::TextDoc`]): the bars (find /
//! replace, go to line, save as), the lossy / mixed line endings notice, the line-number
//! gutter, the visible rows (soft wrap or horizontal scroll, `[editor] wrap`), syntax
//! colours from the core's highlighter, selection and find matches, the caret, and the
//! status line.
//!
//! Scrolling is the terminal UI's own ([`DocView`]: first line, first wrapped row of it,
//! first column): the core's row cache counts one cell per character, the terminal two
//! for wide ones. [`scroll_into_view`] runs when the core asks for it
//! (`scroll_to_caret`, `center_caret`).

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use soyuz_appcore::color::Rgba8;
use soyuz_appcore::editors::text::buffer::Pos;
use soyuz_appcore::editors::text::{HL_BUDGET, TextDoc};
use soyuz_appcore::services::mailbox::Wake;
use soyuz_appcore::tr;
use soyuz_core::config::Editor;

use super::layout::{CONTROL, Kind, LineLayout};
use super::{BarVm, bar, bar_focused, draw_bars, fill_row, put};
use crate::input::editor::{Bar, DocView, Geom, SaveAs};
use crate::layout::{HitTarget, Hits};
use crate::theme::{ColorMode, Role, TuiTheme};
use crate::view::text as fit;

/// What a cell is, for its style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CellKind {
    #[default]
    Text,
    /// Visible whitespace.
    Whitespace,
    /// A control character (`¤`).
    Control,
}

/// A find match under a cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MatchMark {
    #[default]
    None,
    Match,
    /// The selected match.
    Current,
}

/// One drawn cluster (or the blank part of a clipped one).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CellVm {
    pub sym: String,
    /// Cells it takes.
    pub width: u16,
    /// Syntax colour (opaque sRGB).
    pub fg: Option<[u8; 3]>,
    pub italic: bool,
    pub kind: CellKind,
    pub selected: bool,
    pub mark: MatchMark,
}

/// One visual row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextRow {
    /// The line number on the line's first row.
    pub number: Option<usize>,
    /// The caret's line.
    pub current: bool,
    /// The line shown (`EditorCell::top`).
    pub line: usize,
    /// The display column of the row's first cell (`EditorCell::left`).
    pub left: usize,
    pub cells: Vec<CellVm>,
}

/// The text editor as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextVm {
    pub bars: Vec<BarVm>,
    /// A notice row (lossy decoding, mixed line endings) and whether it warns.
    pub notice: Option<(String, bool)>,
    /// Width of the line-number gutter (0: none).
    pub gutter: u16,
    pub rows: Vec<TextRow>,
    /// The caret in the text area (column, row) while it has the keyboard.
    pub cursor: Option<(u16, u16)>,
    /// The last line (clicks below the text land at its end).
    pub last_line: usize,
    pub status: bool,
    pub status_left: String,
    pub status_right: String,
    pub read_only: bool,
    /// "Empty file".
    pub empty: Option<String>,
}

/// Settings and frame facts for [`prepare`].
pub struct Env<'a> {
    pub cfg: &'a Editor,
    /// The editor has the keyboard.
    pub focused: bool,
    /// The dark theme is shown (syntax colours).
    pub dark: bool,
    pub wake: &'a Wake,
}

/// Columns per tab stop (`[editor] tab_width`).
pub fn tab_width(cfg: &Editor) -> usize {
    usize::from(cfg.tab_width).clamp(1, 16)
}

/// The visual rows of `line` (one without soft wrap).
pub fn line_rows(lay: &LineLayout, g: &Geom) -> Vec<usize> {
    if g.wrap { lay.rows(g.cols) } else { vec![0] }
}

/// `(line, row)` moved by `delta` visual rows (clamped to the document).
pub fn walk(t: &TextDoc, g: &Geom, (mut line, mut row): (usize, usize), delta: isize) -> (usize, usize) {
    let n = t.buf.line_count();
    let rows_of = |l: usize| {
        if g.wrap { LineLayout::new(t.buf.line(l), g.tab).rows(g.cols).len() } else { 1 }
    };
    if delta > 0 {
        for _ in 0..delta {
            if row + 1 < rows_of(line) {
                row += 1;
            } else if line + 1 < n {
                line += 1;
                row = 0;
            } else {
                break;
            }
        }
    } else {
        for _ in 0..delta.unsigned_abs() {
            if row > 0 {
                row -= 1;
            } else if line > 0 {
                line -= 1;
                row = rows_of(line) - 1;
            } else {
                break;
            }
        }
    }
    (line, row)
}

/// Visual rows from `a` to `b` (`b` after `a`), counting at most `cap`.
fn distance(t: &TextDoc, g: &Geom, a: (usize, usize), b: (usize, usize), cap: usize) -> usize {
    if b <= a {
        return 0;
    }
    let mut cur = a;
    let mut n = 0;
    while cur < b && n < cap {
        let next = walk(t, g, cur, 1);
        if next == cur {
            break;
        }
        cur = next;
        n += 1;
    }
    n
}

/// Scroll `v` so the caret is visible (`center`: in the middle when it was not).
pub fn scroll_into_view(t: &TextDoc, v: &mut DocView, center: bool) {
    let g = v.geom;
    let rows = g.rows.max(1);
    let head = t.buf.clamp(t.sel.head);
    let lay = LineLayout::new(t.buf.line(head.line), g.tab);
    let lrows = line_rows(&lay, &g);
    let caret = (head.line, lay.row_of(&lrows, head.col));
    let top = (v.top, v.top_row);
    let visible = caret >= top && distance(t, &g, top, caret, rows) < rows;
    if !visible {
        let (l, r) = if center {
            walk(t, &g, caret, -((rows / 2) as isize))
        } else if caret < top {
            caret
        } else {
            walk(t, &g, caret, -(rows as isize - 1))
        };
        v.top = l;
        v.top_row = r;
    }
    if !g.wrap {
        let col = lay.col_of(head.col);
        let cols = g.cols.max(1);
        if col < v.left {
            v.left = col.saturating_sub(4.min(cols / 4));
        } else if col >= v.left + cols {
            v.left = col + 1 + 4.min(cols / 4) - cols;
        }
    } else {
        v.left = 0;
    }
}

/// Keep `v`'s first row inside the document.
pub fn clamp_view(t: &TextDoc, v: &mut DocView) {
    let n = t.buf.line_count();
    if v.top >= n {
        v.top = n - 1;
        v.top_row = 0;
    }
    if v.geom.wrap {
        let rows = LineLayout::new(t.buf.line(v.top), v.geom.tab).rows(v.geom.cols).len();
        v.top_row = v.top_row.min(rows - 1);
    } else {
        v.top_row = 0;
    }
}

/// The geometry, scrolling and model of one frame.
pub fn prepare(t: &mut TextDoc, v: &mut DocView, save_as: Option<&mut SaveAs>, area: Rect, env: &Env<'_>) -> TextVm {
    t.sync_changes();
    let mut vm = TextVm { read_only: t.read_only, last_line: t.buf.line_count() - 1, ..TextVm::default() };
    // Bars.
    let w = area.width;
    if v.bar == Bar::Replace && !(t.find.open && t.find.replace) {
        v.bar = if t.find.open { Bar::Find } else { Bar::Text };
    }
    if v.bar == Bar::Find && !t.find.open {
        v.bar = Bar::Text;
    }
    if v.bar == Bar::Goto && t.goto.is_none() {
        v.bar = Bar::Text;
    }
    if v.bar == Bar::SaveAs && save_as.is_none() {
        v.bar = Bar::Text;
    }
    if t.find.open {
        let (status, error) = t.find_status();
        let chips = vec![
            ("Aa".to_string(), t.find.query.case),
            ("W".to_string(), t.find.query.whole_word),
            (".*".to_string(), t.find.query.regex),
        ];
        let focused = bar_focused(env.focused, v, Bar::Find);
        vm.bars.push(bar(
            tr!("editors-find-hint").into(),
            &t.find.query.text.clone(),
            &mut v.find_edit,
            w,
            24,
            focused,
            chips,
            status,
            error,
        ));
        if t.find.replace {
            let hint = if t.find.query.regex { tr!("editors-replace-hint-regex") } else { tr!("editors-replace-hint") };
            let focused = bar_focused(env.focused, v, Bar::Replace);
            vm.bars.push(bar(
                hint.into(),
                &t.find.replace_text.clone(),
                &mut v.replace_edit,
                w,
                24,
                focused,
                Vec::new(),
                tr!("tui-editor-replace-keys").into(),
                false,
            ));
        }
    }
    if let Some(goto) = t.goto.clone() {
        let focused = bar_focused(env.focused, v, Bar::Goto);
        let note = tr!("editors-goto-range", count = t.buf.line_count());
        vm.bars.push(bar(
            tr!("editors-goto-hint").into(),
            &goto,
            &mut v.goto_edit,
            w,
            20,
            focused,
            Vec::new(),
            note,
            false,
        ));
    }
    if let Some(s) = save_as {
        let focused = bar_focused(env.focused, v, Bar::SaveAs);
        let (note, error) = match &s.error {
            Some(e) => (e.clone(), true),
            None if s.busy => (tr!("tui-editor-saving").into(), false),
            None => (String::new(), false),
        };
        vm.bars.push(bar(
            tr!("tui-editor-save-as").into(),
            &s.text.clone(),
            &mut s.edit,
            w,
            0,
            focused,
            Vec::new(),
            note,
            error,
        ));
    }
    vm.notice = match t.notice() {
        Some(true) => Some((format!("{}  ({})", tr!("editors-text-lossy"), tr!("tui-editor-notice-edit")), true)),
        Some(false) => Some((format!("{}  ({})", tr!("editors-text-mixed-eol"), tr!("tui-editor-notice-ok")), false)),
        None => None,
    };
    let head_rows = vm.bars.len() as u16 + u16::from(vm.notice.is_some());
    vm.status = area.height >= head_rows + 2;
    let text_h = area.height.saturating_sub(head_rows + u16::from(vm.status));
    // Gutter and text columns.
    let digits = (t.buf.line_count().max(1).ilog10() as u16 + 1).max(3);
    vm.gutter = if w >= digits + 1 + 8 { digits + 1 } else { 0 };
    let cols = usize::from(w.saturating_sub(vm.gutter)).max(1);
    let g = Geom { cols, rows: usize::from(text_h).max(1), wrap: env.cfg.wrap, tab: tab_width(env.cfg) };
    if v.geom != g {
        v.geom = g;
        t.scroll_to_caret = true;
    }
    t.page_rows = g.rows.saturating_sub(1).max(1);
    clamp_view(t, v);
    if std::mem::take(&mut t.scroll_to_caret) {
        let center = std::mem::take(&mut t.center_caret);
        scroll_into_view(t, v, center);
    }
    // Syntax colours for the visible lines (more frames when over budget).
    let last = (v.top + g.rows).min(t.buf.line_count() - 1);
    if !t.hl.prepare(t.buf.lines(), v.top, last, env.dark, HL_BUDGET) {
        (env.wake)();
    }
    if text_h > 0 {
        build_rows(t, v, &mut vm, env);
    }
    let caret_visible = env.focused && v.bar == Bar::Text;
    if caret_visible {
        let head = t.buf.clamp(t.sel.head);
        let lay = LineLayout::new(t.buf.line(head.line), g.tab);
        let lrows = line_rows(&lay, &g);
        let left = if g.wrap { lay.row_col(&lrows, lay.row_of(&lrows, head.col)) } else { v.left };
        let col = lay.col_of(head.col);
        vm.cursor = vm.rows.iter().position(|r| r.line == head.line && r.left == left).and_then(|i| {
            let x = col.checked_sub(left)?;
            (x < g.cols || (g.wrap && x == g.cols)).then(|| (x.min(g.cols - 1) as u16, i as u16))
        });
    }
    if t.buf.line_count() == 1 && t.buf.line(0).is_empty() && !caret_visible {
        vm.empty = Some(tr!("editors-text-empty").into());
    }
    // Status line.
    let head = t.sel.head;
    let mut left = tr!("editors-text-status-pos", line = head.line + 1, col = t.caret_col());
    if !t.sel.is_empty() {
        left.push_str("  ·  ");
        left.push_str(&tr!("editors-text-status-selected", count = t.selected_chars()));
    }
    left.push_str("  ·  ");
    left.push_str(&tr!("editors-text-status-lines", count = t.buf.line_count()));
    vm.status_left = left;
    let lang = t.language().unwrap_or(tr!("editors-text-plain")).to_string();
    let indent = if t.uses_tabs {
        tr!("editors-text-indent-tabs").to_string()
    } else {
        tr!("editors-text-indent-spaces", count = g.tab)
    };
    let mut right = format!("{lang}  ·  {indent}  ·  {}  ·  {}", t.encoding.label(), t.eol.label());
    if t.read_only {
        right = format!("{}  ·  {right}", tr!("editors-read-only"));
    }
    vm.status_right = right;
    vm
}

/// The cells of the visible rows.
fn build_rows(t: &TextDoc, v: &DocView, vm: &mut TextVm, env: &Env<'_>) {
    let g = v.geom;
    let (sa, sb) = t.sel.range();
    let head = t.buf.clamp(t.sel.head);
    let matches: &[soyuz_appcore::editors::text::search::Match] = if t.find.open { &t.find.matches } else { &[] };
    let show_ws = env.cfg.show_whitespace;
    let mut line = v.top;
    let mut row = v.top_row;
    while vm.rows.len() < g.rows && line < t.buf.line_count() {
        let text = t.buf.line(line);
        let lay = LineLayout::new(text, g.tab);
        let lrows = line_rows(&lay, &g);
        let spans = t.hl.get(line);
        let a = matches.partition_point(|m| m.line < line);
        let b = matches.partition_point(|m| m.line <= line);
        let line_matches = &matches[a..b];
        while row < lrows.len() && vm.rows.len() < g.rows {
            let (first, end) = if g.wrap {
                (lrows[row], lrows.get(row + 1).copied().unwrap_or(lay.glyphs.len()))
            } else {
                (lay.glyphs.partition_point(|gl| gl.col < v.left), lay.glyphs.len())
            };
            let left = if g.wrap { lay.row_col(&lrows, row) } else { v.left };
            let mut out = TextRow {
                number: (row == 0).then_some(line + 1),
                current: line == head.line,
                line,
                left,
                cells: Vec::new(),
            };
            let mut x = left;
            let right = left + g.cols;
            // Clusters starting before the first column (no wrap): their visible part.
            if !g.wrap
                && let Some(gl) = lay.glyphs.iter().find(|gl| gl.col < left && gl.col + gl.width > left)
            {
                let wv = (gl.col + gl.width - left).min(g.cols);
                out.cells.push(CellVm { sym: " ".repeat(wv), width: wv as u16, ..cell_for(gl, text, spans, show_ws) });
                x += wv;
            }
            for gl in lay.glyphs[first.min(lay.glyphs.len())..end].iter() {
                if gl.col < x {
                    continue;
                }
                if gl.col >= right {
                    break;
                }
                let mut c = cell_for(gl, text, spans, show_ws);
                let pos = Pos::new(line, gl.byte);
                c.selected = pos >= sa && pos < sb;
                c.mark = match line_matches.iter().find(|m| gl.byte >= m.start && gl.byte < m.end) {
                    Some(m) if m.start_pos() == sa && m.end_pos() == sb => MatchMark::Current,
                    Some(_) => MatchMark::Match,
                    None => MatchMark::None,
                };
                if gl.col + gl.width > right {
                    // Cut by the right edge: blank.
                    let wv = right - gl.col;
                    c.sym = " ".repeat(wv);
                    c.width = wv as u16;
                }
                x = gl.col + usize::from(c.width);
                out.cells.push(c);
            }
            // The line break of a selected line.
            let last_row = row + 1 == lrows.len();
            if last_row && line >= sa.line && line < sb.line && x < right {
                out.cells.push(CellVm { sym: " ".into(), width: 1, selected: true, ..CellVm::default() });
            }
            vm.rows.push(out);
            row += 1;
        }
        line += 1;
        row = 0;
    }
}

fn cell_for(
    gl: &super::layout::Glyph,
    text: &str,
    spans: Option<&std::sync::Arc<Vec<soyuz_appcore::editors::text::highlight::Span>>>,
    show_ws: bool,
) -> CellVm {
    let w = gl.width;
    let (sym, kind) = match gl.kind {
        Kind::Text => (text[gl.byte..gl.byte + gl.len].to_string(), CellKind::Text),
        Kind::Space if show_ws => ("·".to_string(), CellKind::Whitespace),
        Kind::Space => (" ".to_string(), CellKind::Text),
        Kind::Tab if show_ws => (format!("→{}", " ".repeat(w - 1)), CellKind::Whitespace),
        Kind::Tab => (" ".repeat(w), CellKind::Text),
        Kind::Control => (CONTROL.to_string(), CellKind::Control),
    };
    // Spans are in line order: the first one ending after the cluster's start.
    let span = spans.and_then(|s| {
        let i = s.partition_point(|s| s.end as usize <= gl.byte);
        s.get(i).filter(|s| s.start as usize <= gl.byte)
    });
    CellVm {
        sym,
        width: w as u16,
        fg: span.map(|s| [s.color[0], s.color[1], s.color[2]]),
        italic: span.is_some_and(|s| s.italic),
        kind,
        ..CellVm::default()
    }
}

/// The style of a cell.
fn cell_style(c: &CellVm, theme: &TuiTheme) -> Style {
    let mut style = match c.kind {
        CellKind::Text => theme.style(Role::Text),
        CellKind::Whitespace => theme.style(Role::Faint),
        CellKind::Control => theme.style(Role::Warning),
    };
    let colors = matches!(theme.mode(), ColorMode::Truecolor | ColorMode::Ansi256);
    let syntax = c.fg.filter(|_| colors && c.kind == CellKind::Text).map(|[r, g, b]| {
        // The light theme's "plain" grey reads better as the theme's text colour.
        let plain = !theme.is_dark() && [r, g, b] == [0x32, 0x32, 0x32];
        (!plain).then(|| crate::theme::to_color(Rgba8::rgb(r, g, b), theme.mode()))
    });
    if let Some(Some(color)) = syntax {
        style = style.fg(color);
    }
    if c.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    match c.mark {
        MatchMark::Match => style = style.patch(theme.style(Role::Highlight)),
        MatchMark::Current => style = style.patch(theme.style(Role::Highlight)).add_modifier(Modifier::REVERSED),
        MatchMark::None => {}
    }
    if c.selected {
        style = style.patch(theme.style(Role::Selected));
        if let Some(Some(color)) = syntax {
            style = style.fg(color);
        }
    }
    style
}

/// Draw the text editor into `area` (the panel's body).
pub fn draw(f: &mut Frame<'_>, area: Rect, m: &TextVm, doc: u64, theme: &TuiTheme, hits: &mut Hits) {
    let mut cursor = None;
    let buf = f.buffer_mut();
    let mut y = draw_bars(buf, area, &m.bars, doc, theme, hits, &mut cursor);
    if let Some((msg, warn)) = &m.notice
        && y < area.bottom()
    {
        let role = if *warn { Role::Warning } else { Role::Info };
        put(buf, area, area.x + 1, y, msg, usize::from(area.width), theme.style(role));
        y += 1;
    }
    let bottom = if m.status { area.bottom().saturating_sub(1) } else { area.bottom() };
    let text_x = area.x + m.gutter.min(area.width);
    let text_w = area.right().saturating_sub(text_x);
    for (i, r) in m.rows.iter().enumerate() {
        let ry = y + i as u16;
        if ry >= bottom {
            break;
        }
        if m.gutter > 0
            && let Some(n) = r.number
        {
            let s = n.to_string();
            let w = m.gutter - 1;
            let x = area.x + w.saturating_sub(s.len() as u16);
            let role = if r.current { Role::Muted } else { Role::Faint };
            put(buf, area, x, ry, &s, usize::from(w), theme.style(role));
        }
        let mut x = text_x;
        for c in &r.cells {
            if x >= area.right() {
                break;
            }
            let style = cell_style(c, theme);
            put(buf, area, x, ry, &c.sym, usize::from(c.width), style);
            x = x.saturating_add(c.width);
        }
        if text_w > 0 {
            hits.push(Rect::new(text_x, ry, text_w, 1), HitTarget::EditorCell { doc, top: r.line, left: r.left });
        }
    }
    // Below the text: clicks land at the end of the last line.
    let after = y + m.rows.len() as u16;
    if after < bottom && text_w > 0 {
        hits.push(
            Rect::new(text_x, after, text_w, bottom - after),
            HitTarget::EditorCell { doc, top: m.last_line, left: usize::MAX / 2 },
        );
    }
    if let Some(e) = &m.empty
        && y < bottom
    {
        put(buf, area, text_x, y, e, usize::from(text_w), theme.style(Role::Faint));
    }
    if let Some((cx, cy)) = m.cursor {
        let (x, yy) = (text_x + cx, y + cy);
        if x < area.right() && yy < bottom {
            if let Some(cell) = buf.cell_mut((x, yy)) {
                cell.modifier.insert(Modifier::REVERSED);
            }
            cursor = Some((x, yy));
        }
    }
    if m.status && area.height > 0 {
        let sy = area.bottom() - 1;
        fill_row(buf, area, area.x, sy, theme.style(Role::StatusBar));
        let rw = fit::width(&m.status_right) as u16;
        let lw = area.width.saturating_sub(rw + 3);
        put(
            buf,
            area,
            area.x + 1,
            sy,
            &fit::truncate_end(&m.status_left, usize::from(lw)),
            usize::from(lw),
            theme.style(Role::StatusBar),
        );
        if area.width > rw + 1 {
            let role = if m.read_only { Role::Warning } else { Role::StatusBar };
            put(
                buf,
                area,
                area.right() - rw - 1,
                sy,
                &m.status_right,
                usize::from(rw),
                theme.style(Role::StatusBar).patch(theme.style(role)),
            );
        }
    }
    if let Some((x, y)) = cursor {
        f.set_cursor_position(Position { x, y });
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample_vm() -> TextVm {
        let cell = |s: &str, w: u16| CellVm { sym: s.into(), width: w, ..CellVm::default() };
        TextVm {
            gutter: 4,
            rows: vec![
                TextRow {
                    number: Some(1),
                    current: true,
                    line: 0,
                    left: 0,
                    cells: vec![cell("a", 1), cell("日", 2), CellVm { selected: true, ..cell("b", 1) }],
                },
                TextRow { number: Some(2), line: 1, cells: vec![cell("¤", 1), cell("   ", 3)], ..TextRow::default() },
            ],
            cursor: Some((1, 0)),
            last_line: 1,
            status: true,
            status_left: "Ln 1, Col 2".into(),
            status_right: "Plain text  ·  UTF-8  ·  LF".into(),
            ..TextVm::default()
        }
    }

    /// The model and the drawing at every tiny size, wrapped or not, with bars open.
    #[test]
    fn prepare_and_draw_at_tiny_sizes() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use soyuz_appcore::editors::text::buffer::Sel;
        use soyuz_appcore::editors::text::encoding::decode;
        let wake: Wake = std::sync::Arc::new(|| {});
        for wrap in [false, true] {
            let cfg = Editor { wrap, show_whitespace: true, ..Editor::default() };
            let mut t = TextDoc::new("a.rs", decode("fn main() {\n\tlet 日本 = \"👍\";\u{1b}\n}\n".as_bytes()));
            t.open_find(true);
            t.find.query.text = "日".into();
            t.goto = Some("2".into());
            t.sel = Sel::new(Pos::new(0, 3), Pos::new(1, 8));
            let mut v = DocView::default();
            for w in 0..=20u16 {
                for h in 0..=6u16 {
                    let env = Env { cfg: &cfg, focused: true, dark: true, wake: &wake };
                    let area = Rect::new(0, 0, w, h);
                    let mut s = SaveAs::default();
                    let m = prepare(&mut t, &mut v, Some(&mut s), area, &env);
                    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                    let mut hits = Hits::default();
                    term.draw(|f| draw(f, area, &m, 1, &TuiTheme::plain(), &mut hits)).unwrap();
                }
            }
        }
    }

    #[test]
    fn rows_gutter_cursor_and_targets() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut term = Terminal::new(TestBackend::new(60, 5)).unwrap();
        let mut hits = Hits::default();
        let m = sample_vm();
        term.draw(|f| draw(f, Rect::new(0, 0, 60, 5), &m, 7, &TuiTheme::plain(), &mut hits)).unwrap();
        let buf = term.backend().buffer().clone();
        let line = |y: u16| (0..60).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>();
        assert!(line(0).starts_with("  1 a日"), "{:?}", line(0));
        assert_eq!(buf[(7, 0)].symbol(), "b", "after the wide character");
        assert!(buf[(5, 0)].modifier.contains(Modifier::REVERSED), "the caret on 日");
        assert!(line(4).contains("Ln 1, Col 2"), "{:?}", line(4));
        assert_eq!(hits.at(6, 1), Some(HitTarget::EditorCell { doc: 7, top: 1, left: 0 }));
        assert_eq!(hits.at(6, 2), Some(HitTarget::EditorCell { doc: 7, top: 1, left: usize::MAX / 2 }));
    }
}
