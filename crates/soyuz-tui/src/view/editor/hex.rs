//! The hex editor's view ([`soyuz_appcore::editors::hex::HexDoc`]): the bars (find bytes
//! or text, go to offset, save as), the "too large for text" notice, rows of 16 bytes
//! (offset, hex in two groups of 8, the text column), changed bytes highlighted, the
//! cursor on the byte (its high or low digit while typing) in the active column, and the
//! status line. Editing is overwrite only, like the GUI.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;
use soyuz_appcore::editors::hex::{HexDoc, ROW};
use soyuz_appcore::tr;

use super::{BarVm, bar, bar_focused, draw_bars, fill_row, put};
use crate::input::editor::{Bar, DocView, Geom, SaveAs};
use crate::layout::{HitTarget, Hits};
use crate::theme::{Role, TuiTheme};
use crate::view::text as fit;

/// One byte as drawn: its value (`None` while its page loads) and whether it changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ByteVm {
    pub value: Option<u8>,
    pub patched: bool,
}

/// One row of 16 bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HexRow {
    /// The file row (offset / 16).
    pub row: u64,
    pub bytes: Vec<ByteVm>,
}

/// The hex editor as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HexVm {
    pub bars: Vec<BarVm>,
    pub notice: Option<String>,
    /// Hex digits of the offsets.
    pub digits: u16,
    pub rows: Vec<HexRow>,
    /// The cursor: its row index in `rows` and byte column.
    pub cursor: Option<(usize, usize)>,
    /// The second digit of the cursor byte is next.
    pub nibble: bool,
    /// The cursor is in the text column.
    pub text_col: bool,
    /// The byte view has the keyboard.
    pub focused: bool,
    pub status: bool,
    pub status_left: String,
    pub status_right: String,
    pub read_only: bool,
}

/// Hex digits for the offsets of a file `len` bytes long (at least 8).
pub fn offset_digits(len: u64) -> u16 {
    let max = len.saturating_sub(1);
    ((64 - max.leading_zeros()).div_ceil(4) as u16).max(8)
}

/// The column of byte `i`'s hex digits, from the start of the row.
pub fn hex_x(digits: u16, i: u64) -> u16 {
    digits + 2 + i as u16 * 3 + u16::from(i >= 8)
}

/// The column of the text column.
pub fn text_x(digits: u16) -> u16 {
    hex_x(digits, 16) + 1
}

/// The byte under column `dx` of a row: `(column, in the text column)`.
pub fn hit_byte(digits: u16, dx: u16) -> Option<(u64, bool)> {
    let tx = text_x(digits);
    if dx >= tx {
        return Some((u64::from(dx - tx).min(ROW - 1), true));
    }
    if dx + 1 < hex_x(digits, 0) {
        return None;
    }
    let i = (0..ROW).rev().find(|&i| dx + 1 >= hex_x(digits, i)).unwrap_or(0);
    Some((i, false))
}

/// The geometry, scrolling, page requests and model of one frame.
pub fn prepare(h: &mut HexDoc, v: &mut DocView, save_as: Option<&mut SaveAs>, area: Rect, focused: bool) -> HexVm {
    h.pump();
    let mut vm =
        HexVm { digits: offset_digits(h.len), read_only: h.read_only, text_col: h.text_col, ..HexVm::default() };
    let w = area.width;
    if v.bar == Bar::Find && h.find.is_none() || v.bar == Bar::Goto && h.goto.is_none() {
        v.bar = Bar::Text;
    }
    if v.bar == Bar::SaveAs && save_as.is_none() || v.bar == Bar::Replace {
        v.bar = Bar::Text;
    }
    if let Some(f) = &h.find {
        let fo = bar_focused(focused, v, Bar::Find);
        let chip = (if f.hex { "HEX" } else { "TXT" }.to_string(), true);
        let note = f.status.clone().unwrap_or_else(|| {
            if f.hex { tr!("editors-hex-find-hint-hex") } else { tr!("editors-hex-find-hint-text") }.into()
        });
        let text = f.text.clone();
        vm.bars.push(bar(tr!("editors-find-hint").into(), &text, &mut v.find_edit, w, 24, fo, vec![chip], note, false));
    }
    if let Some(g) = h.goto.clone() {
        let fo = bar_focused(focused, v, Bar::Goto);
        let note = tr!("editors-hex-goto-hint").into();
        vm.bars.push(bar(tr!("tui-editor-offset").into(), &g, &mut v.goto_edit, w, 24, fo, Vec::new(), note, false));
    }
    if let Some(s) = save_as {
        let fo = bar_focused(focused, v, Bar::SaveAs);
        let (note, error) = match &s.error {
            Some(e) => (e.clone(), true),
            None if s.busy => (tr!("tui-editor-saving").into(), false),
            None => (String::new(), false),
        };
        let text = s.text.clone();
        vm.bars.push(bar(tr!("tui-editor-save-as").into(), &text, &mut s.edit, w, 0, fo, Vec::new(), note, error));
    }
    vm.notice = if let Some(e) = &h.error {
        Some(tr!("editors-hex-read-failed", error = crate::dialog::safe(e)))
    } else if h.too_large_for_text && h.read_only {
        Some(format!("{}  ({})", tr!("editors-hex-too-large"), tr!("tui-editor-notice-allow")))
    } else {
        None
    };
    let head = vm.bars.len() as u16 + u16::from(vm.notice.is_some());
    vm.status = area.height >= head + 2;
    let rows = usize::from(area.height.saturating_sub(head + u16::from(vm.status))).max(1);
    v.geom = Geom { cols: usize::from(w), rows, wrap: false, tab: 1 };
    h.page_rows = (rows as u64).saturating_sub(1).max(1);
    h.scroll_cursor_into_view(rows as f64);
    let total = h.total_rows();
    h.scroll_row = h.scroll_row.clamp(0.0, (total.saturating_sub(1)) as f64).floor();
    let first = h.scroll_row as u64;
    let n = (rows as u64).min(total - first.min(total));
    if h.len > 0 && n > 0 {
        h.ensure_pages(first * ROW, ((first + n) * ROW).min(h.len) - 1);
    }
    for r in first..first + n {
        let base = r * ROW;
        if base >= h.len && h.len > 0 {
            break;
        }
        let bytes = (base..(base + ROW).min(h.len))
            .map(|off| ByteVm { value: h.byte(off), patched: h.patches.get(off).is_some() })
            .collect();
        vm.rows.push(HexRow { row: r, bytes });
    }
    vm.focused = focused && v.bar == Bar::Text;
    vm.nibble = h.nibble == 1;
    vm.cursor = vm.rows.iter().position(|r| r.row == h.cursor / ROW).map(|i| (i, (h.cursor % ROW) as usize));
    let mut s = tr!("editors-hex-status", hex = format!("{:X}", h.cursor), offset = h.cursor, count = h.len);
    if !h.patches.is_empty() {
        s.push_str("  ·  ");
        s.push_str(&tr!("editors-hex-status-changed", count = h.patches.len()));
    }
    vm.status_left = s;
    let mode = if h.read_only { tr!("editors-read-only") } else { tr!("editors-hex-overwrite") };
    let col = if h.text_col { tr!("editors-hex-text-column") } else { tr!("editors-hex-hex-column") };
    vm.status_right = format!("{col}  ·  {mode}");
    vm
}

/// The text column's character for `b`.
fn printable(b: u8) -> char {
    if (0x20..0x7F).contains(&b) { b as char } else { '.' }
}

/// Draw the hex editor into `area` (the panel's body).
pub fn draw(f: &mut Frame<'_>, area: Rect, m: &HexVm, doc: u64, theme: &TuiTheme, hits: &mut Hits) {
    let mut cursor = None;
    let buf = f.buffer_mut();
    let mut y = draw_bars(buf, area, &m.bars, doc, theme, hits, &mut cursor);
    if let Some(msg) = &m.notice
        && y < area.bottom()
    {
        put(buf, area, area.x + 1, y, msg, usize::from(area.width), theme.style(Role::Warning));
        y += 1;
    }
    let bottom = if m.status { area.bottom().saturating_sub(1) } else { area.bottom() };
    let d = m.digits;
    for (i, r) in m.rows.iter().enumerate() {
        let ry = y + i as u16;
        if ry >= bottom {
            break;
        }
        let cur_row = m.cursor.is_some_and(|(ci, _)| ci == i);
        let off_role = if cur_row { Role::Muted } else { Role::Faint };
        put(
            buf,
            area,
            area.x,
            ry,
            &format!("{:0w$X}", r.row * ROW, w = usize::from(d)),
            usize::from(d),
            theme.style(off_role),
        );
        for (c, b) in r.bytes.iter().enumerate() {
            let role = if b.patched { Role::Warning } else { Role::Text };
            let style = theme.style(role);
            let (hx, tx) = (area.x.saturating_add(hex_x(d, c as u64)), area.x.saturating_add(text_x(d) + c as u16));
            let (hs, ts) = match b.value {
                Some(v) => (format!("{v:02X}"), printable(v).to_string()),
                None => ("··".to_string(), "·".to_string()),
            };
            put(buf, area, hx, ry, &hs, 2, style);
            put(
                buf,
                area,
                tx,
                ry,
                &ts,
                1,
                if b.value.is_some_and(|v| printable(v) == '.' && v != b'.') {
                    theme.style(Role::Faint)
                } else {
                    style
                },
            );
            if cur_row && m.cursor.is_some_and(|(_, cc)| cc == c) {
                // The active column's cell is reversed (the digit being typed), the other
                // column's underlined.
                let digit = hx + u16::from(m.nibble);
                let (strong, weak): (Vec<u16>, Vec<u16>) =
                    if m.text_col { (vec![tx], vec![hx, hx + 1]) } else { (vec![digit], vec![tx]) };
                for x in strong {
                    if x < area.right()
                        && let Some(cell) = buf.cell_mut((x, ry))
                    {
                        cell.modifier.insert(if m.focused { Modifier::REVERSED } else { Modifier::UNDERLINED });
                        if m.focused {
                            cursor = Some((x, ry));
                        }
                    }
                }
                for x in weak {
                    if x < area.right()
                        && let Some(cell) = buf.cell_mut((x, ry))
                    {
                        cell.modifier.insert(Modifier::UNDERLINED);
                    }
                }
            }
        }
        hits.push(Rect::new(area.x, ry, area.width, 1), HitTarget::EditorCell { doc, top: r.row as usize, left: 0 });
    }
    if m.status && area.height > 0 {
        let sy = area.bottom() - 1;
        fill_row(buf, area, area.x, sy, theme.style(Role::StatusBar));
        let rw = fit::width(&m.status_right) as u16;
        let lw = area.width.saturating_sub(rw + 3);
        let left = fit::truncate_end(&m.status_left, usize::from(lw));
        put(buf, area, area.x + 1, sy, &left, usize::from(lw), theme.style(Role::StatusBar));
        if area.width > rw + 1 {
            let role = if m.read_only { Role::Warning } else { Role::StatusBar };
            let style = theme.style(Role::StatusBar).patch(theme.style(role));
            put(buf, area, area.right() - rw - 1, sy, &m.status_right, usize::from(rw), style);
        }
    }
    if let Some((x, y)) = cursor {
        f.set_cursor_position(Position { x, y });
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample_vm() -> HexVm {
        let bytes = (0u8..16).map(|b| ByteVm { value: Some(b.wrapping_mul(17)), patched: b == 3 }).collect();
        HexVm {
            digits: 8,
            rows: vec![HexRow { row: 0, bytes }, HexRow { row: 1, bytes: vec![ByteVm::default(); 4] }],
            cursor: Some((0, 3)),
            nibble: true,
            focused: true,
            status: true,
            status_left: "Offset 0x3 (3)".into(),
            status_right: "Hex column  ·  Overwrite".into(),
            ..HexVm::default()
        }
    }

    #[test]
    fn geometry() {
        assert_eq!(offset_digits(0), 8);
        assert_eq!(offset_digits(0x1_0000_0000), 8);
        assert_eq!(offset_digits(0x1_0000_0001), 9);
        assert_eq!(hex_x(8, 0), 10);
        assert_eq!(hex_x(8, 8), 10 + 24 + 1);
        assert_eq!(text_x(8), 10 + 48 + 1 + 1);
        assert_eq!(hit_byte(8, 10), Some((0, false)));
        assert_eq!(hit_byte(8, 11), Some((0, false)));
        assert_eq!(hit_byte(8, 12), Some((1, false)), "the gap before a byte belongs to it");
        assert_eq!(hit_byte(8, 13), Some((1, false)));
        assert_eq!(hit_byte(8, 35), Some((8, false)));
        assert_eq!(hit_byte(8, 60), Some((0, true)));
        assert_eq!(hit_byte(8, 99), Some((15, true)));
        assert_eq!(hit_byte(8, 2), None, "the offset");
    }

    #[test]
    fn rows_and_cursor() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut term = Terminal::new(TestBackend::new(80, 4)).unwrap();
        let mut hits = Hits::default();
        let m = sample_vm();
        term.draw(|f| draw(f, Rect::new(0, 0, 80, 4), &m, 3, &TuiTheme::plain(), &mut hits)).unwrap();
        let buf = term.backend().buffer().clone();
        let line = |y: u16| (0..80).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>();
        assert!(line(0).starts_with("00000000  00 11 22 33 44 55 66 77  88 99 AA BB"), "{:?}", line(0));
        assert!(line(1).starts_with("00000010  ·· ··"), "{:?}", line(1));
        // Byte 3, second digit: column 10 + 9 + 1.
        assert!(buf[(20, 0)].modifier.contains(Modifier::REVERSED));
        assert!(buf[(63, 0)].modifier.contains(Modifier::UNDERLINED), "the text column's cell");
        assert_eq!(hits.at(40, 1), Some(HitTarget::EditorCell { doc: 3, top: 1, left: 0 }));
        assert!(line(3).contains("Offset 0x3"), "{:?}", line(3));
    }
}
