//! Positioned page text: characters with boxes in rendered page points, lines, search,
//! hit testing and word / line ranges. Engine-neutral: the backend feeds [`RawGlyph`]s
//! in content-stream order and [`PageText::build`] reconstructs spaces and lines.
//!
//! Limits: content order, not reading order (multi-column pages interleave only when the
//! producer wrote them that way); no right-to-left reordering; no OCR.

use std::ops::Range;

/// One glyph as the engine drew it, in rendered page points (y down).
#[derive(Clone, Debug, PartialEq)]
pub struct RawGlyph {
    /// The Unicode text of the glyph (a ligature can be several chars).
    pub text: String,
    /// The glyph origin on the baseline.
    pub origin: [f32; 2],
    /// Where the next glyph would start (origin of the next glyph, or origin + advance).
    pub end: [f32; 2],
    /// From the baseline to the ascender (≈ 0.8 em, pointing "up" on the page).
    pub up: [f32; 2],
    /// From the baseline to the descender (≈ 0.2 em, pointing "down").
    pub down: [f32; 2],
}

/// One character of [`PageText::text`] and its box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharBox {
    /// Byte offset of the character in [`PageText::text`].
    pub byte: usize,
    /// Corners (x, y) in rendered page points: baseline-start + descent, baseline-end +
    /// descent, baseline-end + ascent, baseline-start + ascent. Rotated text gives a
    /// rotated quad.
    pub quad: [f32; 8],
}

impl CharBox {
    /// Axis-aligned bounds `[x0, y0, x1, y1]`.
    pub fn rect(&self) -> [f32; 4] {
        let q = &self.quad;
        let xs = [q[0], q[2], q[4], q[6]];
        let ys = [q[1], q[3], q[5], q[7]];
        [
            xs.iter().copied().fold(f32::INFINITY, f32::min),
            ys.iter().copied().fold(f32::INFINITY, f32::min),
            xs.iter().copied().fold(f32::NEG_INFINITY, f32::max),
            ys.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        ]
    }

    fn center(&self) -> [f32; 2] {
        let r = self.rect();
        [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0]
    }
}

/// The text of one page. Every char of `text` (including the spaces and line breaks
/// put back between glyph runs) has a [`CharBox`] at the same index in `chars`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageText {
    pub text: String,
    pub chars: Vec<CharBox>,
    /// Char-index ranges of the lines, in order, without their trailing `\n`.
    pub lines: Vec<Range<usize>>,
}

/// A text search.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Query {
    pub text: String,
    /// Match case exactly.
    pub case: bool,
    /// Only whole words.
    pub whole_word: bool,
}

/// One hit: the char range and one rectangle per line it covers (`[x0, y0, x1, y1]`).
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    pub chars: Range<usize>,
    pub rects: Vec<[f32; 4]>,
}

fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn add(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}
fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}
fn len(a: [f32; 2]) -> f32 {
    dot(a, a).sqrt()
}
fn lerp(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn quad(start: [f32; 2], end: [f32; 2], up: [f32; 2], down: [f32; 2]) -> [f32; 8] {
    let (a, b, c, d) = (add(start, down), add(end, down), add(end, up), add(start, up));
    [a[0], a[1], b[0], b[1], c[0], c[1], d[0], d[1]]
}

impl PageText {
    /// Rebuild text from glyphs: a gap wider than ~0.18 em on the same baseline becomes a
    /// space, a baseline change (or a jump back) a line break.
    pub fn build(glyphs: &[RawGlyph]) -> PageText {
        let mut out = PageText::default();
        let mut line_start = 0usize;
        let mut prev: Option<&RawGlyph> = None;
        for g in glyphs {
            if g.text.is_empty() {
                continue;
            }
            let em = len(sub(g.up, g.down)).max(0.01);
            if let Some(p) = prev {
                let pem = len(sub(p.up, p.down)).max(0.01);
                let dir = {
                    let v = sub(p.end, p.origin);
                    let l = len(v);
                    if l > 1e-4 {
                        [v[0] / l, v[1] / l]
                    } else {
                        // Perpendicular to "up" (clockwise in y-down space).
                        let u = p.up;
                        let l = len(u).max(1e-4);
                        [-u[1] / l, u[0] / l]
                    }
                };
                let normal = [-dir[1], dir[0]];
                let off = sub(g.origin, p.end);
                let along = dot(off, dir);
                let across = dot(sub(g.origin, p.origin), normal).abs();
                let same_line = across < 0.5 * pem.min(em) && along > -0.6 * pem.max(em);
                if !same_line {
                    let nl_at = p.end;
                    out.lines.push(line_start..out.chars.len());
                    out.push_char('\n', quad(nl_at, nl_at, p.up, p.down));
                    line_start = out.chars.len();
                } else if along > 0.18 * pem.max(em)
                    && !out.text.ends_with(char::is_whitespace)
                    && !g.text.starts_with(char::is_whitespace)
                {
                    out.push_char(' ', quad(p.end, g.origin, p.up, p.down));
                }
            }
            let n = g.text.chars().count() as f32;
            for (i, c) in g.text.chars().enumerate() {
                let s = lerp(g.origin, g.end, i as f32 / n);
                let e = lerp(g.origin, g.end, (i + 1) as f32 / n);
                out.push_char(c, quad(s, e, g.up, g.down));
            }
            prev = Some(g);
        }
        if out.chars.len() > line_start {
            out.lines.push(line_start..out.chars.len());
        }
        out
    }

    fn push_char(&mut self, c: char, quad: [f32; 8]) {
        self.chars.push(CharBox { byte: self.text.len(), quad });
        self.text.push(c);
    }

    /// Byte offset of char index `i` (`text.len()` at the end).
    fn byte_of(&self, i: usize) -> usize {
        self.chars.get(i).map_or(self.text.len(), |c| c.byte)
    }

    /// The text of a char range.
    pub fn slice_text(&self, r: Range<usize>) -> &str {
        let (a, b) = (self.byte_of(r.start.min(self.chars.len())), self.byte_of(r.end.min(self.chars.len())));
        &self.text[a..b.max(a)]
    }

    /// The char at index `i`.
    fn char_at(&self, i: usize) -> Option<char> {
        self.text[self.byte_of(i)..].chars().next()
    }

    /// Index of the line holding char `i`.
    pub fn line_of(&self, i: usize) -> Option<usize> {
        let k = self.lines.partition_point(|l| l.end <= i);
        self.lines.get(k).filter(|l| l.contains(&i)).map(|_| k)
    }

    /// The char under `p` (within `slack` points of its box), else the horizontally
    /// nearest char whose box spans `p.y`. `None` away from any text.
    pub fn hit(&self, p: [f32; 2], slack: f32) -> Option<usize> {
        let inside =
            |r: [f32; 4]| p[0] >= r[0] - slack && p[0] <= r[2] + slack && p[1] >= r[1] - slack && p[1] <= r[3] + slack;
        if let Some(i) = self.chars.iter().position(|c| inside(c.rect()) && self.char_at_is_text(c)) {
            return Some(i);
        }
        // Same band, nearest horizontally.
        self.chars
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                let r = c.rect();
                p[1] >= r[1] - slack && p[1] <= r[3] + slack && self.char_at_is_text(c)
            })
            .min_by(|(_, a), (_, b)| {
                let da = (a.center()[0] - p[0]).abs();
                let db = (b.center()[0] - p[0]).abs();
                da.total_cmp(&db)
            })
            .map(|(i, _)| i)
    }

    fn char_at_is_text(&self, c: &CharBox) -> bool {
        self.text[c.byte..].chars().next().is_some_and(|ch| ch != '\n')
    }

    /// The char nearest to `p` (drag selection past the text).
    pub fn nearest(&self, p: [f32; 2]) -> Option<usize> {
        self.chars
            .iter()
            .enumerate()
            .filter(|(_, c)| self.char_at_is_text(c))
            .min_by(|(_, a), (_, b)| {
                let d = |c: &CharBox| {
                    let m = c.center();
                    // Vertical distance counts more: stay on the line under the pointer.
                    (m[0] - p[0]).abs() + 4.0 * (m[1] - p[1]).abs()
                };
                d(a).total_cmp(&d(b))
            })
            .map(|(i, _)| i)
    }

    /// The word around char `i` (letters / digits / `_`, or the single other char).
    pub fn word_at(&self, i: usize) -> Range<usize> {
        let is_word = |k: usize| self.char_at(k).is_some_and(|c| c.is_alphanumeric() || c == '_');
        if i >= self.chars.len() {
            return i..i;
        }
        if !is_word(i) {
            return i..i + 1;
        }
        let (mut a, mut b) = (i, i + 1);
        while a > 0 && is_word(a - 1) {
            a -= 1;
        }
        while b < self.chars.len() && is_word(b) {
            b += 1;
        }
        a..b
    }

    /// The line around char `i`.
    pub fn line_at(&self, i: usize) -> Range<usize> {
        self.line_of(i).map_or(i..i, |k| self.lines[k].clone())
    }

    /// One rectangle per line for a char range (`\n` boxes skipped).
    pub fn rects(&self, r: Range<usize>) -> Vec<[f32; 4]> {
        let mut out: Vec<[f32; 4]> = Vec::new();
        let mut cur_line = None;
        for i in r.start..r.end.min(self.chars.len()) {
            let c = &self.chars[i];
            if !self.char_at_is_text(c) {
                continue;
            }
            let line = self.line_of(i);
            let b = c.rect();
            match out.last_mut() {
                Some(last) if line == cur_line => {
                    last[0] = last[0].min(b[0]);
                    last[1] = last[1].min(b[1]);
                    last[2] = last[2].max(b[2]);
                    last[3] = last[3].max(b[3]);
                }
                _ => out.push(b),
            }
            cur_line = line;
        }
        out
    }
}

/// Every non-overlapping hit of `q` on the page. Whitespace in the query matches any run
/// of whitespace (also a line break); case folds per char unless `q.case`.
pub fn find(t: &PageText, q: &Query) -> Vec<Match> {
    let fold = |c: char| {
        if c.is_whitespace() {
            ' '
        } else if q.case {
            c
        } else {
            c.to_lowercase().next().unwrap_or(c)
        }
    };
    let mut needle: Vec<char> = Vec::new();
    for c in q.text.trim().chars().map(fold) {
        if !(c == ' ' && needle.last() == Some(&' ')) {
            needle.push(c);
        }
    }
    if needle.is_empty() {
        return Vec::new();
    }
    let hay: Vec<char> = t.text.chars().map(fold).collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = Vec::new();
    let mut i = 0;
    while i < hay.len() {
        let mut j = i;
        let mut ok = true;
        for &n in &needle {
            if n == ' ' {
                if j >= hay.len() || hay[j] != ' ' {
                    ok = false;
                    break;
                }
                while j < hay.len() && hay[j] == ' ' {
                    j += 1;
                }
            } else if j < hay.len() && hay[j] == n {
                j += 1;
            } else {
                ok = false;
                break;
            }
        }
        if ok && q.whole_word {
            let before = i.checked_sub(1).map(|k| hay[k]).is_some_and(is_word);
            let after = hay.get(j).copied().is_some_and(is_word);
            ok = !before && !after;
        }
        if ok {
            out.push(Match { chars: i..j, rects: t.rects(i..j) });
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Glyphs of `s` from `(x, y)`, `w` points apart, 10 pt type.
    fn run(s: &str, x: f32, y: f32, w: f32) -> Vec<RawGlyph> {
        s.chars()
            .enumerate()
            .filter(|(_, c)| *c != ' ')
            .map(|(i, c)| {
                let x0 = x + i as f32 * w;
                RawGlyph {
                    text: c.to_string(),
                    origin: [x0, y],
                    end: [x0 + w * 0.9, y],
                    up: [0.0, -8.0],
                    down: [0.0, 2.0],
                }
            })
            .collect()
    }

    #[test]
    fn spaces_and_lines_come_back() {
        let mut g = run("Hello world", 10.0, 50.0, 6.0);
        g.extend(run("second line", 10.0, 70.0, 6.0));
        let t = PageText::build(&g);
        assert_eq!(t.text, "Hello world\nsecond line");
        assert_eq!(t.chars.len(), t.text.chars().count());
        assert_eq!(t.lines, vec![0..11, 12..23]);
        assert_eq!(t.slice_text(t.lines[1].clone()), "second line");
        assert_eq!(t.line_of(3), Some(0));
        assert_eq!(t.line_of(11), None, "the line break belongs to no line");
        let r = t.chars[0].rect();
        assert_eq!(r, [10.0, 42.0, 15.4, 52.0]);
    }

    #[test]
    fn find_rects_words_and_hits() {
        let mut g = run("Hello world", 10.0, 50.0, 6.0);
        g.extend(run("hello again", 10.0, 70.0, 6.0));
        let t = PageText::build(&g);
        let m = find(&t, &Query { text: "hello".into(), ..Default::default() });
        assert_eq!(m.len(), 2);
        assert_eq!(m[1].chars, 12..17);
        assert_eq!(m[0].rects.len(), 1);
        assert!((m[0].rects[0][0] - 10.0).abs() < 1e-3 && m[0].rects[0][3] > 50.0);
        assert_eq!(find(&t, &Query { text: "hello".into(), case: true, ..Default::default() }).len(), 1);
        // Whitespace in the query spans the line break.
        let m = find(&t, &Query { text: "world  hello".into(), ..Default::default() });
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].rects.len(), 2, "one rect per line");
        assert_eq!(find(&t, &Query { text: "ell".into(), whole_word: true, ..Default::default() }).len(), 0);
        assert_eq!(find(&t, &Query { text: "again".into(), whole_word: true, ..Default::default() }).len(), 1);
        assert!(find(&t, &Query { text: "  ".into(), ..Default::default() }).is_empty());

        assert_eq!(t.hit([12.0, 47.0], 0.5), Some(0));
        assert_eq!(t.hit([500.0, 47.0], 0.5), Some(10), "same band, nearest char");
        assert_eq!(t.hit([12.0, 300.0], 0.5), None);
        assert_eq!(t.nearest([12.0, 300.0]), Some(12));
        assert_eq!(t.word_at(7), 6..11);
        assert_eq!(t.word_at(5), 5..6, "the space alone");
        assert_eq!(t.line_at(14), 12..23);
    }

    #[test]
    fn ligatures_split_their_box() {
        let g = vec![RawGlyph {
            text: "fi".into(),
            origin: [0.0, 10.0],
            end: [10.0, 10.0],
            up: [0.0, -8.0],
            down: [0.0, 2.0],
        }];
        let t = PageText::build(&g);
        assert_eq!(t.text, "fi");
        assert_eq!(t.chars[1].rect()[0], 5.0);
        assert_eq!(t.chars[1].byte, 1);
    }
}
