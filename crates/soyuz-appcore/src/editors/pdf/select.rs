//! Text selection across pages: char positions, word / line granularity, the per-page
//! char ranges a selection covers and the text it copies.

use std::ops::Range;

use crate::services::pdf::PageText;

/// A char position: page and char index in that page's [`PageText`] (`usize::MAX` =
/// the end of the page).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextPos {
    pub page: u32,
    pub ch: usize,
}

impl TextPos {
    pub const fn new(page: u32, ch: usize) -> TextPos {
        TextPos { page, ch }
    }

    /// The end of `page`.
    pub const fn page_end(page: u32) -> TextPos {
        TextPos { page, ch: usize::MAX }
    }
}

/// What a drag extends by: single chars, whole words (double click) or lines (triple).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Unit {
    #[default]
    Char,
    Word,
    Line,
}

/// A selection: the unit range under the first click and the one under the pointer; it
/// covers both and everything between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: (TextPos, TextPos),
    pub head: (TextPos, TextPos),
    pub unit: Unit,
}

impl Selection {
    /// A selection of exactly `start..end`.
    pub fn range(start: TextPos, end: TextPos) -> Selection {
        Selection { anchor: (start, start), head: (end, end), unit: Unit::Char }
    }

    /// The covered span `start..end` (end exclusive).
    pub fn span(&self) -> (TextPos, TextPos) {
        (self.anchor.0.min(self.head.0), self.anchor.1.max(self.head.1))
    }

    pub fn is_empty(&self) -> bool {
        let (a, b) = self.span();
        a >= b
    }

    /// The char range on `page` (`n` chars), if the selection touches it.
    pub fn on_page(&self, page: u32, n: usize) -> Option<Range<usize>> {
        let (a, b) = self.span();
        if page < a.page || page > b.page {
            return None;
        }
        let s = if page == a.page { a.ch.min(n) } else { 0 };
        let e = if page == b.page { b.ch.min(n) } else { n };
        (s < e).then_some(s..e)
    }

    /// Pages the selection touches.
    pub fn pages(&self) -> std::ops::RangeInclusive<u32> {
        let (a, b) = self.span();
        a.page..=b.page
    }
}

/// The unit range around char `i` of `t` (`ch` is the hit char).
pub fn unit_range(t: &PageText, page: u32, i: usize, unit: Unit) -> (TextPos, TextPos) {
    let r = match unit {
        Unit::Char => i..i,
        Unit::Word => t.word_at(i),
        Unit::Line => t.line_at(i),
    };
    (TextPos::new(page, r.start), TextPos::new(page, r.end))
}

/// The selected text: each page's part, pages joined by a line break. `text(page)` gives
/// a page's text; pages without one are skipped.
pub fn selected_text<'a>(sel: &Selection, text: impl Fn(u32) -> Option<&'a PageText>) -> String {
    let mut out = String::new();
    let mut first = true;
    for page in sel.pages() {
        let Some(t) = text(page) else { continue };
        let Some(r) = sel.on_page(page, t.chars.len()) else { continue };
        if !first {
            out.push('\n');
        }
        first = false;
        out.push_str(t.slice_text(r));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::pdf::text::RawGlyph;

    fn page(s: &str) -> PageText {
        let glyphs: Vec<RawGlyph> = s
            .chars()
            .enumerate()
            .filter(|(_, c)| *c != ' ')
            .map(|(i, c)| {
                let x = i as f32 * 6.0;
                RawGlyph {
                    text: c.to_string(),
                    origin: [x, 20.0],
                    end: [x + 5.4, 20.0],
                    up: [0.0, -8.0],
                    down: [0.0, 2.0],
                }
            })
            .collect();
        PageText::build(&glyphs)
    }

    #[test]
    fn spans_pages_and_copies() {
        let pages = [page("Hello world"), page("second page")];
        let sel = Selection::range(TextPos::new(0, 6), TextPos::new(1, 6));
        assert_eq!(sel.on_page(0, 11), Some(6..11));
        assert_eq!(sel.on_page(1, 11), Some(0..6));
        assert_eq!(sel.on_page(2, 11), None);
        assert_eq!(selected_text(&sel, |p| pages.get(p as usize)), "world\nsecond");
        let all = Selection::range(TextPos::new(0, 0), TextPos::page_end(1));
        assert_eq!(selected_text(&all, |p| pages.get(p as usize)), "Hello world\nsecond page");
        // Backwards drags cover the same span.
        let back = Selection { anchor: (TextPos::new(1, 6), TextPos::new(1, 6)), ..sel };
        let back = Selection { head: (TextPos::new(0, 6), TextPos::new(0, 6)), ..back };
        assert_eq!(back.span(), sel.span());
        assert!(Selection::range(TextPos::new(0, 3), TextPos::new(0, 3)).is_empty());
    }

    #[test]
    fn word_and_line_units() {
        let t = page("Hello world");
        assert_eq!(unit_range(&t, 0, 8, Unit::Word), (TextPos::new(0, 6), TextPos::new(0, 11)));
        assert_eq!(unit_range(&t, 0, 2, Unit::Line), (TextPos::new(0, 0), TextPos::new(0, 11)));
        let w = Selection {
            anchor: unit_range(&t, 0, 1, Unit::Word),
            head: unit_range(&t, 0, 8, Unit::Word),
            unit: Unit::Word,
        };
        assert_eq!(selected_text(&w, |_| Some(&t)), "Hello world");
    }
}
