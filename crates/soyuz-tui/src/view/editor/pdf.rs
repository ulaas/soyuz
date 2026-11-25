//! The PDF viewer's terminal view ([`soyuz_appcore::editors::pdf::PdfDoc`]): the text of
//! the current page, word-wrapped to the panel and scrollable, with the find matches
//! highlighted; the bars (find, go to page), a key hint and a status line ("Page 3 of
//! 12", the find count, the document's title and PDF version).
//!
//! The terminal shows no page pictures: the document runs with `PdfDoc::text_only`, so
//! its pump asks the workers for page text only (the current page and its neighbours),
//! never for page bitmaps or tiles, and `set_viewport` / `visible_pages` are never called.
//!
//! Keys (`crate::input::editor`): the viewer's page commands (`KeyContext::Pdf`: N / P
//! next / previous page, Ctrl+Home / Ctrl+End first / last page), the editor commands a
//! PDF reuses (find, F3 / Shift+F3, go to page, the find options, the document keys);
//! Up / Down scroll a row, Page Up / Page Down (Space / Shift+Space) a screen and turn the
//! page at its end, Left / Right turn the page, Home / End go to the page's top / bottom,
//! Ctrl+C copies the page's text. A password-locked document shows its message and asks
//! for the password on Enter (the archive password prompt, `editors::ask_pdf_password`).

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use soyuz_appcore::editors::pdf::{PdfDoc, PdfFind};
use soyuz_appcore::tr;
use unicode_width::UnicodeWidthChar;

use super::text::MatchMark;
use super::{BarVm, Keys, bar, bar_focused, draw_bars, fill_row, put};
use crate::input::editor::{Bar, DocView, Geom};
use crate::layout::{HitTarget, Hits};
use crate::theme::{Role, TuiTheme};
use crate::view::text as fit;

/// A run of cells with one look.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Seg {
    pub text: String,
    pub mark: MatchMark,
}

/// The PDF view as drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PdfVm {
    pub bars: Vec<BarVm>,
    /// The visible rows of the page's text.
    pub rows: Vec<Vec<Seg>>,
    /// Instead of the text: the page is being read, or it has none.
    pub empty: Option<String>,
    /// The terminal's cursor goes on the first text row (the view has the keyboard and no
    /// bar does: screen readers read the line under it).
    pub caret: bool,
    /// The keys.
    pub hint: String,
    pub status: bool,
    pub status_left: String,
    pub status_right: String,
}

/// The cells a character takes (controls show as one blank cell).
fn cell_width(c: char) -> usize {
    if c.is_control() { 1 } else { c.width().unwrap_or(0) }
}

/// The character drawn for `c`.
fn shown(c: char) -> char {
    if c.is_control() { ' ' } else { c }
}

/// The visual rows of `chars` (a page's text) `cols` cells wide, as char-index ranges:
/// one or more per line (`\n` ends a line), broken after a space when one fits, else
/// where the row is full.
pub fn wrap(chars: &[char], cols: usize) -> Vec<Range<usize>> {
    let cols = cols.max(1);
    let n = chars.len();
    let mut rows = Vec::new();
    let mut start = 0;
    loop {
        let end = chars[start..].iter().position(|c| *c == '\n').map_or(n, |p| start + p);
        if start == end {
            rows.push(start..start);
        }
        let mut s = start;
        while s < end {
            let (mut w, mut e, mut brk) = (0, s, None);
            while e < end {
                let cw = cell_width(chars[e]);
                if w + cw > cols && e > s {
                    break;
                }
                w += cw;
                if chars[e] == ' ' {
                    brk = Some(e + 1);
                }
                e += 1;
            }
            if e < end
                && let Some(b) = brk.filter(|b| *b > s)
            {
                e = b;
            }
            rows.push(s..e);
            s = e;
        }
        if end >= n {
            break;
        }
        start = end + 1;
    }
    rows
}

/// The find bar's note: the current match and the count ("+" while the sweep runs).
fn find_note(f: &PdfFind) -> String {
    if !f.active() || f.query.text.trim().is_empty() {
        return String::new();
    }
    let more = if f.done() { "" } else { "+" };
    let n = f.count();
    match (n, f.ordinal()) {
        (0, _) if f.done() => tr!("editors-find-no-results").into(),
        (0, _) => "…".into(),
        (_, Some(i)) => tr!("editors-find-position", current = i, total = n, more = more),
        (_, None) => tr!("editors-find-results", count = n, more = more),
    }
}

/// The segments of row `r` of `chars`, with the find marks of `marks` (char ranges, the
/// current one flagged).
fn segments(chars: &[char], r: Range<usize>, marks: &[(Range<usize>, bool)]) -> Vec<Seg> {
    let mut out: Vec<Seg> = Vec::new();
    for i in r {
        let mark = match marks.iter().find(|(m, _)| m.contains(&i)) {
            Some((_, true)) => MatchMark::Current,
            Some((_, false)) => MatchMark::Match,
            None => MatchMark::None,
        };
        match out.last_mut() {
            Some(s) if s.mark == mark => s.text.push(shown(chars[i])),
            _ => out.push(Seg { text: shown(chars[i]).to_string(), mark }),
        }
    }
    out
}

/// The geometry, scrolling and model of one frame (the document is ready).
pub fn prepare(p: &mut PdfDoc, v: &mut DocView, area: Rect, focused: bool, keys: &Keys) -> PdfVm {
    let mut vm = PdfVm::default();
    if matches!(v.bar, Bar::Replace | Bar::SaveAs | Bar::Crop | Bar::Resize)
        || v.bar == Bar::Find && !p.find.open
        || v.bar == Bar::Goto && p.goto.is_none()
    {
        v.bar = Bar::Text;
    }
    let w = area.width;
    if p.find.open {
        let chips = vec![("Aa".to_string(), p.find.query.case), ("W".to_string(), p.find.query.whole_word)];
        let fo = bar_focused(focused, v, Bar::Find);
        let note = find_note(&p.find);
        let text = p.find.query.text.clone();
        vm.bars.push(bar(tr!("editors-find-hint").into(), &text, &mut v.find_edit, w, 24, fo, chips, note, false));
    }
    if let Some(goto) = p.goto.clone() {
        let fo = bar_focused(focused, v, Bar::Goto);
        let (note, error) = match &v.field_error {
            Some(e) => (e.clone(), true),
            None => (tr!("editors-goto-range", count = p.page_count()), false),
        };
        vm.bars.push(bar(
            tr!("tui-editor-pdf-page").into(),
            &goto,
            &mut v.goto_edit,
            w,
            20,
            fo,
            Vec::new(),
            note,
            error,
        ));
    } else {
        v.field_error = None;
    }
    let head = vm.bars.len() as u16;
    vm.status = area.height >= head + 2;
    let mut text_h = area.height.saturating_sub(head + u16::from(vm.status));
    vm.hint = tr!(
        "tui-editor-pdf-keys",
        next = keys.next_page.as_str(),
        prev = keys.prev_page.as_str(),
        goto = keys.goto.as_str(),
        find = keys.find.as_str()
    );
    if text_h > 3 {
        // The key hint above the status line.
        text_h -= 1;
    } else {
        vm.hint.clear();
    }
    let rows_h = usize::from(text_h);
    let cols = usize::from(w.saturating_sub(1)).max(1);
    v.geom = Geom { cols, rows: rows_h.max(1), wrap: true, tab: 1 };
    // A new page starts at its top (or at its bottom when paging back with Page Up).
    let page = p.page();
    if v.pdf_page != Some(page) {
        v.top = if v.pdf_end { usize::MAX } else { 0 };
        v.pdf_page = Some(page);
    }
    v.pdf_end = false;
    match p.page_text(page) {
        None => {
            vm.empty = Some(tr!("tui-editor-pdf-reading").into());
            v.pdf_rows = 0;
        }
        Some(t) if t.trim().is_empty() => {
            vm.empty = Some(tr!("tui-editor-pdf-no-text").into());
            v.pdf_rows = 0;
            v.top = 0;
        }
        Some(t) => {
            let chars: Vec<char> = t.chars().collect();
            let rows = wrap(&chars, cols);
            v.pdf_rows = rows.len();
            v.top = v.top.min(rows.len().saturating_sub(rows_h));
            let marks: Vec<(Range<usize>, bool)> = if p.find.open {
                p.find
                    .page_matches(page)
                    .iter()
                    .enumerate()
                    .map(|(k, m)| (m.chars.clone(), p.find.current == Some((page, k))))
                    .collect()
            } else {
                Vec::new()
            };
            // A new current match on this page comes into view (a third from the top).
            if p.find.current != v.pdf_cur {
                v.pdf_cur = p.find.current;
                if let Some((_, k)) = p.find.current.filter(|c| c.0 == page)
                    && let Some(m) = p.find.page_matches(page).get(k)
                {
                    let row = rows.partition_point(|r| r.end <= m.chars.start).min(rows.len().saturating_sub(1));
                    if row < v.top || row >= v.top + rows_h {
                        v.top = row.saturating_sub(rows_h / 3).min(rows.len().saturating_sub(rows_h));
                    }
                }
            }
            vm.rows = rows.iter().skip(v.top).take(rows_h).map(|r| segments(&chars, r.clone(), &marks)).collect();
        }
    }
    vm.caret = focused && v.bar == Bar::Text;
    let mut left = p.page_label(page);
    let note = find_note(&p.find);
    if p.find.open && !note.is_empty() {
        left.push_str("  ·  ");
        left.push_str(&note);
    }
    vm.status_left = left;
    let mut right = Vec::new();
    if let Some(info) = p.info() {
        if let Some(t) = info.title.as_deref().filter(|t| !t.trim().is_empty()) {
            right.push(crate::dialog::safe(t));
        }
        right.push(match &info.version {
            Some(ver) => format!("PDF {ver}"),
            None => "PDF".into(),
        });
    }
    vm.status_right = right.join("  ·  ");
    vm
}

/// The style of a segment.
fn seg_style(s: &Seg, theme: &TuiTheme) -> ratatui::style::Style {
    let base = theme.style(Role::Text);
    match s.mark {
        MatchMark::None => base,
        MatchMark::Match => base.patch(theme.style(Role::Highlight)),
        MatchMark::Current => base.patch(theme.style(Role::Highlight)).add_modifier(ratatui::style::Modifier::REVERSED),
    }
}

/// Draw the PDF view into `area` (the panel's body).
pub fn draw(f: &mut Frame<'_>, area: Rect, m: &PdfVm, doc: u64, theme: &TuiTheme, hits: &mut Hits) {
    let mut cursor = None;
    let buf = f.buffer_mut();
    let y = draw_bars(buf, area, &m.bars, doc, theme, hits, &mut cursor);
    let mut bottom = if m.status { area.bottom().saturating_sub(1) } else { area.bottom() };
    if !m.hint.is_empty() && bottom > y {
        bottom -= 1;
        put(
            buf,
            area,
            area.x + 1,
            bottom,
            &m.hint,
            usize::from(area.width.saturating_sub(1)),
            theme.style(Role::Faint),
        );
    }
    let text = Rect::new(area.x, y, area.width, bottom.saturating_sub(y));
    if !text.is_empty() {
        hits.push(text, HitTarget::EditorCell { doc, top: 0, left: 0 });
        let tx = text.x + 1.min(text.width);
        match &m.empty {
            Some(e) => {
                put(buf, text, tx, text.y, e, usize::from(text.width), theme.style(Role::Muted));
            }
            None => {
                for (i, row) in m.rows.iter().enumerate() {
                    let ry = text.y + i as u16;
                    if ry >= text.bottom() {
                        break;
                    }
                    let mut x = tx;
                    for s in row {
                        if x >= text.right() {
                            break;
                        }
                        x = put(buf, text, x, ry, &s.text, usize::from(text.right() - x), seg_style(s, theme));
                    }
                }
            }
        }
        if m.caret && cursor.is_none() {
            cursor = Some((tx.min(text.right().saturating_sub(1)), text.y));
        }
    }
    if m.status && area.height > 0 {
        let sy = area.bottom() - 1;
        fill_row(buf, area, area.x, sy, theme.style(Role::StatusBar));
        let rw = (fit::width(&m.status_right) as u16).min(area.width / 2);
        let right = fit::truncate_end(&m.status_right, usize::from(rw));
        let lw = area.width.saturating_sub(rw + 3);
        let left = fit::truncate_end(&m.status_left, usize::from(lw));
        put(buf, area, area.x + 1, sy, &left, usize::from(lw), theme.style(Role::StatusBar));
        if rw > 0 && area.width > rw + 1 {
            put(buf, area, area.right() - rw - 1, sy, &right, usize::from(rw), theme.style(Role::StatusBar));
        }
    }
    if let Some((x, y)) = cursor {
        f.set_cursor_position(Position { x, y });
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn rows(s: &str, cols: usize) -> Vec<String> {
        let chars: Vec<char> = s.chars().collect();
        wrap(&chars, cols).into_iter().map(|r| chars[r].iter().collect()).collect()
    }

    pub(crate) fn sample_vm() -> PdfVm {
        let seg = |t: &str, mark| Seg { text: t.into(), mark };
        PdfVm {
            bars: vec![BarVm { label: "Find".into(), field: "Hello".into(), field_w: 8, ..BarVm::default() }],
            rows: vec![
                vec![seg("Hello", MatchMark::Current), seg(" from page one", MatchMark::None)],
                vec![seg("日本語 ", MatchMark::None), seg("Hello", MatchMark::Match)],
            ],
            caret: true,
            hint: "N / P: page".into(),
            status: true,
            status_left: "Page 1 of 2  ·  1 of 2".into(),
            status_right: "Fixture doc  ·  PDF 1.7".into(),
            ..PdfVm::default()
        }
    }

    #[test]
    fn wraps_at_spaces_and_wide_characters() {
        assert_eq!(rows("one two three", 8), ["one two ", "three"]);
        assert_eq!(rows("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(rows("a\n\nb", 4), ["a", "", "b"]);
        assert_eq!(rows("日本語", 4), ["日本", "語"], "two cells each");
        assert_eq!(rows("", 4), [""]);
        // Narrower than one wide character: still one per row, never stuck.
        assert_eq!(rows("日本", 1), ["日", "本"]);
    }

    #[test]
    fn marks_split_segments() {
        let chars: Vec<char> = "say Hello there".chars().collect();
        let s = segments(&chars, 0..chars.len(), &[(4..9, true)]);
        assert_eq!(
            s,
            vec![
                Seg { text: "say ".into(), mark: MatchMark::None },
                Seg { text: "Hello".into(), mark: MatchMark::Current },
                Seg { text: " there".into(), mark: MatchMark::None },
            ]
        );
    }

    #[test]
    fn draws_rows_status_and_cursor() {
        let mut t = Terminal::new(TestBackend::new(60, 8)).unwrap();
        let mut hits = Hits::default();
        let m = sample_vm();
        t.draw(|f| draw(f, Rect::new(0, 0, 60, 8), &m, 3, &TuiTheme::plain(), &mut hits)).unwrap();
        let buf = t.backend().buffer().clone();
        let line = |y: u16| (0..60).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>();
        assert!(line(0).contains("Find") && line(0).contains("Hello"), "{:?}", line(0));
        assert!(line(1).starts_with(" Hello from page one"), "{:?}", line(1));
        assert!(buf[(1, 1)].modifier.contains(ratatui::style::Modifier::REVERSED), "the current match");
        assert!(line(6).contains("N / P: page"), "{:?}", line(6));
        assert!(line(7).contains("Page 1 of 2") && line(7).contains("PDF 1.7"), "{:?}", line(7));
        assert_eq!(hits.at(5, 2), Some(HitTarget::EditorCell { doc: 3, top: 0, left: 0 }));
    }

    #[test]
    fn draws_at_tiny_sizes() {
        let mut m = sample_vm();
        for empty in [false, true] {
            m.empty = empty.then(|| "Reading the page…".into());
            for w in 0..=20u16 {
                for h in 0..=6u16 {
                    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
                    let mut hits = Hits::default();
                    t.draw(|f| draw(f, Rect::new(0, 0, w, h), &m, 1, &TuiTheme::plain(), &mut hits)).unwrap();
                }
            }
        }
    }
}
