//! `shot:<file>` of `SOYUZ_SCRIPT` in the terminal UI (Phase T8): the drawn `Buffer` saved
//! in the format the extension names ([`ShotFormat`]):
//! - `.txt`: the text, one line per row;
//! - `.snap`: the snapshot format (`crate::snapshot::document` with styles);
//! - `.svg`: one `<text>` per run of equally styled characters with per-cell `x`
//!   positions (so columns line up in any monospace font), rect backgrounds, a fixed dark
//!   terminal palette for the named colours; box drawing and block characters
//!   (U+2500–U+259F) drawn as lines and rects, so they join between rows.

use std::fmt::Write as _;
use std::io;
use std::path::Path;

use ratatui::buffer::{Buffer, Cell};
use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr;

/// A shot's file format, from its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotFormat {
    Txt,
    Snap,
    Svg,
}

impl ShotFormat {
    /// The format of `path` (by extension, any case); `None` when unknown.
    pub fn from_path(path: &Path) -> Option<ShotFormat> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "txt" => Some(ShotFormat::Txt),
            "snap" => Some(ShotFormat::Snap),
            "svg" => Some(ShotFormat::Svg),
            _ => None,
        }
    }
}

/// `buf` as text, one line per row (trailing spaces trimmed), LF line ends.
pub fn text(buf: &Buffer) -> String {
    crate::snapshot::lines(buf).into_iter().map(|l| l.trim_end().to_string() + "\n").collect()
}

/// Save `buf` to `path` in the format its extension names.
pub fn write(path: &Path, buf: &Buffer) -> io::Result<()> {
    match ShotFormat::from_path(path) {
        Some(ShotFormat::Txt) => std::fs::write(path, text(buf)),
        Some(ShotFormat::Snap) => {
            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("shot");
            let opts = crate::snapshot::SnapOpts::styled();
            std::fs::write(path, crate::snapshot::document(name, buf, &opts))
        }
        Some(ShotFormat::Svg) => std::fs::write(path, svg(buf)),
        None => Err(io::Error::new(io::ErrorKind::InvalidInput, "unknown shot format (use .txt, .snap or .svg)")),
    }
}

// ---- SVG ----

/// One cell in pixels.
const CELL_W: f32 = 9.0;
const CELL_H: f32 = 18.0;
/// The text baseline below a row's top.
const BASELINE: f32 = 14.0;
const FONT_SIZE: u32 = 15;
const FONTS: &str = "'Cascadia Mono', Consolas, 'DejaVu Sans Mono', Menlo, monospace";
/// The terminal's own colours (`Color::Reset`).
const DEFAULT_FG: &str = "#D4D4D4";
const DEFAULT_BG: &str = "#1E1E1E";
/// The 16 ANSI colours (a dark terminal's), in index order.
const ANSI: [&str; 16] = [
    "#000000", "#CD3131", "#0DBC79", "#E5E510", "#2472C8", "#BC3FBC", "#11A8CD", "#E5E5E5", "#666666", "#F14C4C",
    "#23D18B", "#F5F543", "#3B8EEA", "#D670D6", "#29B8DB", "#FFFFFF",
];

/// `buf` as an SVG document.
pub fn svg(buf: &Buffer) -> String {
    let area = buf.area;
    let (w, h) = (f32::from(area.width) * CELL_W, f32::from(area.height) * CELL_H);
    let mut bg = String::new();
    let mut text = String::new();
    let mut shapes = String::new();
    for y in area.top()..area.bottom() {
        let top = f32::from(y - area.top()) * CELL_H;
        let cells = row_cells(buf, y);
        // Background runs.
        let mut i = 0;
        while i < cells.len() {
            let fill = &cells[i].colors.1;
            let mut j = i + 1;
            while j < cells.len() && cells[j].colors.1 == *fill && cells[j].col == cells[j - 1].col + cells[j - 1].width
            {
                j += 1;
            }
            if fill != DEFAULT_BG {
                let x = f32::from(cells[i].col) * CELL_W;
                let end = cells[j - 1].col + cells[j - 1].width;
                let _ = writeln!(
                    bg,
                    r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{fill}"/>"#,
                    n(x),
                    n(top),
                    n(f32::from(end - cells[i].col) * CELL_W),
                    n(CELL_H)
                );
            }
            i = j;
        }
        // Text runs and drawn shapes.
        let mut i = 0;
        while i < cells.len() {
            let c = &cells[i];
            let x = f32::from(c.col) * CELL_W;
            if c.hidden || c.sym.trim().is_empty() {
                i += 1;
                continue;
            }
            if let Some(shape) = c.sym.chars().next().filter(|_| c.sym.chars().count() == 1).and_then(shape) {
                draw_shape(&mut shapes, &shape, x, top, &c.colors.0);
                i += 1;
                continue;
            }
            // A run: single characters with the same text style, next to each other.
            let mut j = i + 1;
            if c.sym.chars().count() == 1 {
                while j < cells.len() {
                    let d = &cells[j];
                    let joins = d.col == cells[j - 1].col + cells[j - 1].width
                        && d.sym.chars().count() == 1
                        && !d.sym.trim().is_empty()
                        && !d.hidden
                        && d.text_style() == c.text_style()
                        && d.sym.chars().next().and_then(shape).is_none();
                    if !joins {
                        break;
                    }
                    j += 1;
                }
            }
            let run = &cells[i..j];
            let xs: Vec<String> = run.iter().map(|r| n(f32::from(r.col) * CELL_W)).collect();
            let content: String = run.iter().map(|r| escape(&r.sym)).collect();
            let _ = writeln!(
                text,
                r#"<text x="{}" y="{}" fill="{}"{}>{content}</text>"#,
                xs.join(" "),
                n(top + BASELINE),
                c.colors.0,
                c.attrs()
            );
            i = j;
        }
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}" font-family="{FONTS}" font-size="{FONT_SIZE}">"#,
        n(w),
        n(h),
        n(w),
        n(h)
    );
    let _ = writeln!(out, r#"<rect width="{}" height="{}" fill="{DEFAULT_BG}"/>"#, n(w), n(h));
    out.push_str(&bg);
    out.push_str(&text);
    if !shapes.is_empty() {
        out.push_str("<g stroke-linecap=\"square\" fill=\"none\">\n");
        out.push_str(&shapes);
        out.push_str("</g>\n");
    }
    out.push_str("</svg>\n");
    out
}

/// A drawn cell of a row (continuation cells of wide characters left out).
struct SvgCell {
    col: u16,
    width: u16,
    sym: String,
    /// Foreground and background after `REVERSED`.
    colors: (String, String),
    modifier: Modifier,
    hidden: bool,
}

impl SvgCell {
    fn text_style(&self) -> (&str, Modifier) {
        (
            &self.colors.0,
            self.modifier
                & (Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED | Modifier::DIM | Modifier::CROSSED_OUT),
        )
    }

    fn attrs(&self) -> String {
        let m = self.modifier;
        let mut a = String::new();
        if m.contains(Modifier::BOLD) {
            a.push_str(r#" font-weight="bold""#);
        }
        if m.contains(Modifier::ITALIC) {
            a.push_str(r#" font-style="italic""#);
        }
        match (m.contains(Modifier::UNDERLINED), m.contains(Modifier::CROSSED_OUT)) {
            (true, true) => a.push_str(r#" text-decoration="underline line-through""#),
            (true, false) => a.push_str(r#" text-decoration="underline""#),
            (false, true) => a.push_str(r#" text-decoration="line-through""#),
            (false, false) => {}
        }
        if m.contains(Modifier::DIM) {
            a.push_str(r#" opacity="0.6""#);
        }
        a
    }
}

fn row_cells(buf: &Buffer, y: u16) -> Vec<SvgCell> {
    let area = buf.area;
    let mut out = Vec::new();
    let mut x = area.left();
    while x < area.right() {
        let cell: &Cell = &buf[(x, y)];
        let sym = if cell.symbol().is_empty() { " " } else { cell.symbol() };
        let width = (sym.width() as u16).clamp(1, area.right() - x);
        let (mut fg, mut bg) = (hex(cell.fg, DEFAULT_FG), hex(cell.bg, DEFAULT_BG));
        if cell.modifier.contains(Modifier::REVERSED) {
            std::mem::swap(&mut fg, &mut bg);
        }
        out.push(SvgCell {
            col: x - area.left(),
            width,
            sym: sym.to_string(),
            colors: (fg, bg),
            modifier: cell.modifier,
            hidden: cell.modifier.contains(Modifier::HIDDEN),
        });
        x += width;
    }
    out
}

/// `#RRGGBB` of `c`; `default` for `Reset`.
fn hex(c: Color, default: &str) -> String {
    let named = |i: usize| ANSI[i].to_string();
    match c {
        Color::Reset => default.to_string(),
        Color::Black => named(0),
        Color::Red => named(1),
        Color::Green => named(2),
        Color::Yellow => named(3),
        Color::Blue => named(4),
        Color::Magenta => named(5),
        Color::Cyan => named(6),
        Color::Gray => named(7),
        Color::DarkGray => named(8),
        Color::LightRed => named(9),
        Color::LightGreen => named(10),
        Color::LightYellow => named(11),
        Color::LightBlue => named(12),
        Color::LightMagenta => named(13),
        Color::LightCyan => named(14),
        Color::White => named(15),
        Color::Rgb(r, g, b) => format!("#{r:02X}{g:02X}{b:02X}"),
        Color::Indexed(i) => indexed(i),
    }
}

/// The xterm 256-colour palette.
fn indexed(i: u8) -> String {
    match i {
        0..=15 => ANSI[usize::from(i)].to_string(),
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            let (r, g, b) = (level(i / 36), level((i / 6) % 6), level(i % 6));
            format!("#{r:02X}{g:02X}{b:02X}")
        }
        232..=255 => {
            let v = 8 + (i - 232) * 10;
            format!("#{v:02X}{v:02X}{v:02X}")
        }
    }
}

/// A number without a needless fraction.
fn n(v: f32) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
    out
}

/// How a box-drawing or block character is drawn.
#[derive(Clone, Debug, PartialEq)]
enum Shape {
    /// Arms from the cell's centre to its edges (up, right, down, left): 1 light, 2 heavy,
    /// 3 double. `rounded`: a light corner drawn as a curve.
    Lines { arms: [u8; 4], dashed: bool, rounded: bool },
    /// `╱` (1), `╲` (2), `╳` (3).
    Diagonal(u8),
    /// Rects in cell fractions (x, y, w, h), filled with the foreground at `opacity`.
    Blocks { rects: Vec<(f32, f32, f32, f32)>, opacity: f32 },
}

/// The shape of `c` (U+2500–U+259F); `None` draws the glyph as text.
fn shape(c: char) -> Option<Shape> {
    let lines = |a: &str, dashed: bool, rounded: bool| {
        let d: Vec<u8> = a.bytes().map(|b| b - b'0').collect();
        Some(Shape::Lines { arms: [d[0], d[1], d[2], d[3]], dashed, rounded })
    };
    let block = |rects: Vec<(f32, f32, f32, f32)>| Some(Shape::Blocks { rects, opacity: 1.0 });
    let (ul, ur, ll, lr) = ((0.0, 0.0, 0.5, 0.5), (0.5, 0.0, 0.5, 0.5), (0.0, 0.5, 0.5, 0.5), (0.5, 0.5, 0.5, 0.5));
    match c {
        '─' => lines("0101", false, false),
        '━' => lines("0202", false, false),
        '│' => lines("1010", false, false),
        '┃' => lines("2020", false, false),
        '┄' | '┈' | '╌' => lines("0101", true, false),
        '┅' | '┉' | '╍' => lines("0202", true, false),
        '┆' | '┊' | '╎' => lines("1010", true, false),
        '┇' | '┋' | '╏' => lines("2020", true, false),
        '┌' => lines("0110", false, false),
        '┏' => lines("0220", false, false),
        '┐' => lines("0011", false, false),
        '┓' => lines("0022", false, false),
        '└' => lines("1100", false, false),
        '┗' => lines("2200", false, false),
        '┘' => lines("1001", false, false),
        '┛' => lines("2002", false, false),
        '├' => lines("1110", false, false),
        '┣' => lines("2220", false, false),
        '┤' => lines("1011", false, false),
        '┫' => lines("2022", false, false),
        '┬' => lines("0111", false, false),
        '┳' => lines("0222", false, false),
        '┴' => lines("1101", false, false),
        '┻' => lines("2202", false, false),
        '┼' => lines("1111", false, false),
        '╋' => lines("2222", false, false),
        '═' => lines("0303", false, false),
        '║' => lines("3030", false, false),
        '╔' => lines("0330", false, false),
        '╗' => lines("0033", false, false),
        '╚' => lines("3300", false, false),
        '╝' => lines("3003", false, false),
        '╠' => lines("3330", false, false),
        '╣' => lines("3033", false, false),
        '╦' => lines("0333", false, false),
        '╩' => lines("3303", false, false),
        '╬' => lines("3333", false, false),
        '╭' => lines("0110", false, true),
        '╮' => lines("0011", false, true),
        '╯' => lines("1001", false, true),
        '╰' => lines("1100", false, true),
        '╴' => lines("0001", false, false),
        '╵' => lines("1000", false, false),
        '╶' => lines("0100", false, false),
        '╷' => lines("0010", false, false),
        '╸' => lines("0002", false, false),
        '╹' => lines("2000", false, false),
        '╺' => lines("0200", false, false),
        '╻' => lines("0020", false, false),
        '╼' => lines("0201", false, false),
        '╽' => lines("1020", false, false),
        '╾' => lines("0102", false, false),
        '╿' => lines("2010", false, false),
        '╱' => Some(Shape::Diagonal(1)),
        '╲' => Some(Shape::Diagonal(2)),
        '╳' => Some(Shape::Diagonal(3)),
        '▀' => block(vec![(0.0, 0.0, 1.0, 0.5)]),
        '▁'..='▇' => {
            let k = (c as u32 - '▀' as u32) as f32 / 8.0;
            block(vec![(0.0, 1.0 - k, 1.0, k)])
        }
        '█' => block(vec![(0.0, 0.0, 1.0, 1.0)]),
        '▉'..='▏' => block(vec![(0.0, 0.0, ('▐' as u32 - c as u32) as f32 / 8.0, 1.0)]),
        '▐' => block(vec![(0.5, 0.0, 0.5, 1.0)]),
        '░' | '▒' | '▓' => Some(Shape::Blocks {
            rects: vec![(0.0, 0.0, 1.0, 1.0)],
            opacity: (c as u32 - '░' as u32 + 1) as f32 / 4.0,
        }),
        '▔' => block(vec![(0.0, 0.0, 1.0, 0.125)]),
        '▕' => block(vec![(0.875, 0.0, 0.125, 1.0)]),
        '▖' => block(vec![ll]),
        '▗' => block(vec![lr]),
        '▘' => block(vec![ul]),
        '▙' => block(vec![ul, ll, lr]),
        '▚' => block(vec![ul, lr]),
        '▛' => block(vec![ul, ur, ll]),
        '▜' => block(vec![ul, ur, lr]),
        '▝' => block(vec![ur]),
        '▞' => block(vec![ur, ll]),
        '▟' => block(vec![ur, ll, lr]),
        _ => None,
    }
}

/// Draw `s` in the cell at (`x`, `y`) in `color`.
fn draw_shape(out: &mut String, s: &Shape, x: f32, y: f32, color: &str) {
    let (cx, cy) = (x + CELL_W / 2.0, y + CELL_H / 2.0);
    let line = |out: &mut String, (x1, y1): (f32, f32), (x2, y2): (f32, f32), width: f32, dashed: bool| {
        let dash = if dashed { r#" stroke-dasharray="2 2""# } else { "" };
        let _ = writeln!(
            out,
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{color}" stroke-width="{}"{dash}/>"#,
            n(x1),
            n(y1),
            n(x2),
            n(y2),
            n(width)
        );
    };
    match s {
        Shape::Lines { arms, dashed, rounded } => {
            // The edge each arm reaches: up, right, down, left.
            let ends = [(cx, y), (x + CELL_W, cy), (cx, y + CELL_H), (x, cy)];
            if *rounded {
                let mut on = (0..4).filter(|&i| arms[i] > 0).map(|i| ends[i]);
                if let (Some(a), Some(b)) = (on.next(), on.next()) {
                    let _ = writeln!(
                        out,
                        r#"<path d="M{} {} Q{} {} {} {}" stroke="{color}" stroke-width="1.2"/>"#,
                        n(a.0),
                        n(a.1),
                        n(cx),
                        n(cy),
                        n(b.0),
                        n(b.1)
                    );
                }
                return;
            }
            for (i, &w) in arms.iter().enumerate() {
                let end = ends[i];
                match w {
                    0 => {}
                    1 | 2 => line(out, (cx, cy), end, if w == 1 { 1.2 } else { 2.4 }, *dashed),
                    _ => {
                        // Double: two light lines, offset across the arm.
                        let vertical = i % 2 == 0;
                        for o in [-1.5, 1.5] {
                            let (dx, dy) = if vertical { (o, 0.0) } else { (0.0, o) };
                            // Reach past the centre so the corners close.
                            let (sx, sy) = if vertical { (cx + dx, cy - dy.abs()) } else { (cx, cy + dy) };
                            let start = if vertical {
                                (sx, if end.1 < cy { cy + 1.5 } else { cy - 1.5 })
                            } else {
                                (if end.0 < cx { cx + 1.5 } else { cx - 1.5 }, sy)
                            };
                            line(out, start, (end.0 + dx, end.1 + dy), 1.0, *dashed);
                        }
                    }
                }
            }
        }
        Shape::Diagonal(k) => {
            if k & 1 != 0 {
                line(out, (x, y + CELL_H), (x + CELL_W, y), 1.2, false);
            }
            if k & 2 != 0 {
                line(out, (x, y), (x + CELL_W, y + CELL_H), 1.2, false);
            }
        }
        Shape::Blocks { rects, opacity } => {
            let op = if *opacity < 1.0 { format!(r#" fill-opacity="{opacity}""#) } else { String::new() };
            for (rx, ry, rw, rh) in rects {
                let _ = writeln!(
                    out,
                    r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{color}" stroke="none"{op}/>"#,
                    n(x + rx * CELL_W),
                    n(y + ry * CELL_H),
                    n(rw * CELL_W),
                    n(rh * CELL_H)
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    use super::*;

    #[test]
    fn format_from_extension() {
        assert_eq!(ShotFormat::from_path(Path::new("a/b.TXT")), Some(ShotFormat::Txt));
        assert_eq!(ShotFormat::from_path(Path::new("b.svg")), Some(ShotFormat::Svg));
        assert_eq!(ShotFormat::from_path(Path::new("b.snap")), Some(ShotFormat::Snap));
        assert_eq!(ShotFormat::from_path(Path::new("b.png")), None);
        assert_eq!(ShotFormat::from_path(Path::new("b")), None);
    }

    /// The golden 10 × 3 buffer: a rounded box, colours, a reversed cell, a wide
    /// character, a block and markup-sensitive text.
    fn golden() -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 3));
        buf.set_string(0, 0, "╭─ a<b ─╮", Style::new().fg(Color::Blue));
        buf.set_string(0, 1, "│", Style::new().fg(Color::Blue));
        buf.set_string(1, 1, "日", Style::new().add_modifier(Modifier::BOLD));
        buf.set_string(3, 1, "ok", Style::new().fg(Color::Black).bg(Color::Rgb(0x89, 0xb4, 0xfa)));
        buf.set_string(5, 1, "&", Style::new().add_modifier(Modifier::REVERSED));
        buf.set_string(6, 1, "▒█", Style::new().fg(Color::Indexed(196)));
        buf.set_string(8, 1, "│", Style::new().fg(Color::Blue));
        buf.set_string(0, 2, "╰────────╯", Style::new().fg(Color::Blue));
        buf
    }

    #[test]
    fn golden_text() {
        assert_eq!(text(&golden()), "╭─ a<b ─╮\n│日 ok&▒█│\n╰────────╯\n");
    }

    #[test]
    fn golden_snap() {
        crate::snapshot::assert_file(
            "shot__golden.snap",
            &crate::snapshot::document("shot__golden", &golden(), &crate::snapshot::SnapOpts::styled()),
        );
    }

    #[test]
    fn golden_svg() {
        let s = svg(&golden());
        crate::snapshot::assert_file("shot__golden.svg", &s);
        // Box drawing is drawn, not typed: no box character is left as text.
        assert!(!s.contains('─') && !s.contains('╭') && !s.contains('█') && !s.contains('▒'), "{s}");
        assert!(s.contains(r#"<path d="M9 9 Q4.5 9 4.5 18""#), "the rounded corner is a curve: {s}");
        // Per-cell positions: the wide character takes two cells, `ok` follows at cell 3.
        assert!(s.contains(r##"<text x="9" y="32" fill="#D4D4D4" font-weight="bold">日</text>"##), "{s}");
        assert!(s.contains(r##"<text x="27 36" y="32" fill="#000000">ok</text>"##), "{s}");
        assert!(s.contains("&lt;") && s.contains("&amp;"), "markup is escaped: {s}");
        // The reversed cell swaps its colours; the RGB background is a rect.
        assert!(s.contains(r##"fill="#D4D4D4"/>"##) && s.contains(r##"fill="#89B4FA"/>"##), "{s}");
        assert!(s.contains(r##"fill="#FF0000" stroke="none" fill-opacity="0.5""##), "indexed 196 shade: {s}");
    }

    #[test]
    fn write_picks_the_format() {
        let tmp = tempfile::tempdir().unwrap();
        for f in ["s.txt", "s.snap", "s.svg"] {
            write(&tmp.path().join(f), &golden()).unwrap();
        }
        let snap = std::fs::read_to_string(tmp.path().join("s.snap")).unwrap();
        assert!(snap.starts_with("# soyuz-tui snapshot: s (10x3,") && snap.contains("--- legend"), "{snap}");
        assert!(std::fs::read_to_string(tmp.path().join("s.svg")).unwrap().starts_with("<svg "));
        assert!(write(&tmp.path().join("s.png"), &golden()).is_err());
    }

    #[test]
    fn every_box_and_block_character_has_a_shape_or_falls_back() {
        // Nothing in the range panics, and the common ones are shapes.
        let drawn = ('\u{2500}'..='\u{259F}').filter(|&c| shape(c).is_some()).count();
        assert!(drawn >= 90, "{drawn}");
        for c in '\u{2500}'..='\u{259F}' {
            if let Some(s) = shape(c) {
                let mut out = String::new();
                draw_shape(&mut out, &s, 0.0, 0.0, "#FFFFFF");
                assert!(!out.is_empty(), "{c}");
            }
        }
        assert_eq!(indexed(16), "#000000");
        assert_eq!(indexed(231), "#FFFFFF");
        assert_eq!(indexed(232), "#080808");
    }
}
