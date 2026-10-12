//! Display-width aware text fitting for terminal cells.
//!
//! Widths are counted per grapheme cluster with `unicode-width`, exactly the way ratatui
//! measures what it writes into the buffer, so a fitted string never spills over its
//! rect and a cluster (CJK ideograph, ZWJ emoji, letter + combining marks) is never cut
//! in half. Input must already be terminal-safe ([`crate::dialog::safe`]).

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The ellipsis put where text was cut.
pub const ELLIPSIS: &str = "…";

/// Cells `s` takes on the terminal.
pub fn width(s: &str) -> usize {
    s.graphemes(true).map(UnicodeWidthStr::width).sum()
}

/// The longest prefix of `s` (whole graphemes) at most `max` cells wide, with its width.
fn head(s: &str, max: usize) -> (&str, usize) {
    let mut w = 0;
    for (i, g) in s.grapheme_indices(true) {
        let gw = g.width();
        if w + gw > max {
            return (&s[..i], w);
        }
        w += gw;
    }
    (s, w)
}

/// The longest suffix of `s` (whole graphemes) at most `max` cells wide, with its width.
fn tail(s: &str, max: usize) -> (&str, usize) {
    let mut w = 0;
    for (i, g) in s.grapheme_indices(true).rev() {
        let gw = g.width();
        if w + gw > max {
            return (&s[i + g.len()..], w);
        }
        w += gw;
    }
    (s, w)
}

/// `s` cut at the end to `max` cells (`long na…`).
pub fn truncate_end(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let (h, _) = head(s, max - 1);
    format!("{h}{ELLIPSIS}")
}

/// `s` cut in the middle to `max` cells (`long…name`): the start keeps the larger half.
pub fn truncate_middle(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let budget = max - 1;
    let (h, hw) = head(s, budget.div_ceil(2));
    let (t, _) = tail(s, budget - hw);
    format!("{h}{ELLIPSIS}{t}")
}

/// The extension (with its dot) worth keeping when a file name is cut: not for dot files
/// and not when it is long.
fn extension(name: &str) -> Option<&str> {
    let dot = name.rfind('.')?;
    let ext = &name[dot..];
    (dot > 0 && ext.len() > 1 && width(ext) <= 8).then_some(ext)
}

/// A file name fitted into `max` cells: the stem is cut in the middle and the extension
/// stays (`a_ver…name.txt`); when the extension would take more than half the room the
/// whole name is cut in the middle. `keep_ext` off (folders) cuts the whole name.
pub fn truncate_name(name: &str, max: usize, keep_ext: bool) -> String {
    if width(name) <= max {
        return name.to_string();
    }
    if keep_ext
        && let Some(ext) = extension(name)
        && width(ext) * 2 <= max
    {
        let stem = &name[..name.len() - ext.len()];
        return format!("{}{ext}", truncate_middle(stem, max - width(ext)));
    }
    truncate_middle(name, max)
}

/// `s` padded with spaces (or cut at the end) to exactly `w` cells.
pub fn fit(s: &str, w: usize) -> String {
    let mut out = truncate_end(s, w);
    let have = width(&out);
    out.extend(std::iter::repeat_n(' ', w.saturating_sub(have)));
    out
}

/// `s` wrapped into lines of at most `w` cells: breaks at spaces where it can, splits
/// words longer than a line between graphemes. Always at least one line.
pub fn wrap(s: &str, w: usize) -> Vec<String> {
    let w = w.max(1);
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0;
    for word in s.split(' ') {
        let mut word = word;
        let mut ww = width(word);
        // Room for a space plus the word on this line?
        let sep = usize::from(!cur.is_empty());
        if cur_w + sep + ww <= w {
            if sep == 1 {
                cur.push(' ');
            }
            cur.push_str(word);
            cur_w += sep + ww;
            continue;
        }
        if !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
        }
        while ww > w {
            let (h, hw) = head(word, w);
            // A single cluster wider than the line: put it alone.
            let (h, hw) = if h.is_empty() {
                let g = word.graphemes(true).next().unwrap_or_default();
                (g, g.width())
            } else {
                (h, hw)
            };
            lines.push(h.to_string());
            word = &word[h.len()..];
            ww -= hw;
        }
        cur.push_str(word);
        cur_w = ww;
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths() {
        assert_eq!(width("abc"), 3);
        assert_eq!(width("日本語"), 6);
        // e + combining acute: one cell.
        assert_eq!(width("e\u{301}"), 1);
        assert_eq!(width(""), 0);
    }

    #[test]
    fn end_and_middle() {
        assert_eq!(truncate_end("abcdef", 6), "abcdef");
        assert_eq!(truncate_end("abcdef", 4), "abc…");
        assert_eq!(truncate_end("abcdef", 1), "…");
        assert_eq!(truncate_end("abcdef", 0), "");
        assert_eq!(truncate_middle("abcdefgh", 5), "ab…gh");
        assert_eq!(truncate_middle("abcdefgh", 6), "abc…gh");
        assert_eq!(truncate_middle("abcdefgh", 1), "…");
    }

    #[test]
    fn names_keep_their_extension() {
        assert_eq!(truncate_name("a_very_long_name.txt", 12), "a_ve…ame.txt");
        assert_eq!(width(&truncate_name("a_very_long_name.txt", 12)), 12);
        // Folders and dot files: cut as a whole.
        assert_eq!(super::truncate_name("folder.with.dots", 8, false), "fold…ots");
        assert_eq!(truncate_name(".gitignore_long", 8), ".git…ong");
        // The extension would take too much room.
        assert_eq!(truncate_name("ab.longext", 6), "ab.…xt");
        assert_eq!(truncate_name("short.txt", 20), "short.txt");
    }

    fn truncate_name(n: &str, max: usize) -> String {
        super::truncate_name(n, max, true)
    }

    #[test]
    fn cjk_is_never_split() {
        let s = "日本語のファイル名.txt";
        for max in 0..=width(s) {
            let t = truncate_name(s, max);
            assert!(width(&t) <= max, "{max}: {t:?}");
            assert!(t.chars().all(|c| c != '\u{fffd}'));
        }
        // An odd budget cannot hold half an ideograph: one cell stays empty.
        assert_eq!(truncate_end("日本語", 4), "日…");
        assert_eq!(width(&truncate_end("日本語", 4)), 3);
    }

    #[test]
    fn zwj_emoji_stay_whole() {
        let family = "👨\u{200d}👩\u{200d}👧";
        let s = format!("{family}{family}{family}.png");
        for max in 0..=width(&s) {
            let t = truncate_name(&s, max);
            assert!(width(&t) <= max, "{max}: {t:?}");
            // Every cluster in the result is either a whole family or plain text.
            for g in t.graphemes(true) {
                assert!(!g.starts_with('\u{200d}') && !g.ends_with('\u{200d}'), "{t:?}");
                if g.contains('\u{200d}') {
                    assert_eq!(g, family, "{t:?}");
                }
            }
        }
    }

    #[test]
    fn combining_marks_stay_with_their_letter() {
        let s = "cafe\u{301}-cafe\u{301}-cafe\u{301}";
        for max in 0..=width(s) {
            let t = truncate_middle(s, max);
            assert!(width(&t) <= max);
            assert!(!t.starts_with('\u{301}') && !t.contains("…\u{301}"), "{t:?}");
        }
        assert_eq!(truncate_end(s, 5), "cafe\u{301}…");
    }

    #[test]
    fn fit_pads_and_cuts() {
        assert_eq!(fit("ab", 4), "ab  ");
        assert_eq!(fit("abcdef", 4), "abc…");
        assert_eq!(fit("日本", 3), "日…");
        assert_eq!(fit("日本", 1), "…");
    }

    #[test]
    fn wrapping() {
        assert_eq!(wrap("the quick brown fox", 9), vec!["the quick", "brown fox"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("", 4), vec![""]);
        assert_eq!(wrap("日本語テキスト", 5), vec!["日本", "語テ", "キス", "ト"]);
        for l in wrap("a 日本語テキスト and more words", 3) {
            assert!(width(&l) <= 3, "{l:?}");
        }
        // Wider than the line: alone, never dropped.
        assert_eq!(wrap("日", 1), vec!["日"]);
    }
}
