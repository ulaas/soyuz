//! Names too long for their space (`[appearance] name_truncation`): a file name keeps its
//! start and extension, folders and paths lose their middle, or everything is cut at the
//! end. Rows repaint constantly, so where to cut is worked out once per text and width from
//! the glyph advances of one layout, and remembered; a repaint only hashes the key and
//! lays out the short text, which egui caches. Names with hidden controls are cut at the
//! end: their revealed spelling does not map onto their characters.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use egui::text::TextWrapping;
use egui::{Color32, FontId, Galley, Painter};
use soyuz_core::config::NameTruncation;

use super::{revealed_job, trunc_galley, wrapped_galley};

/// The ellipsis between the kept parts.
const ELLIPSIS: char = '\u{2026}';
/// Longest extension (dot included) that counts as one: `.backup-2021` reads as a name.
const MAX_EXT_CHARS: usize = 9;
/// Text cut for a width is remembered up to this many times, then forgotten at once.
const MAX_CUTS: usize = 8192;

static MODE: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static CUTS: RefCell<HashMap<u64, Cut>> = RefCell::new(HashMap::new());
}

/// Apply `[appearance] name_truncation` (startup and every config change: fonts may have
/// changed too, so what was measured is forgotten).
pub fn set_mode(mode: NameTruncation) {
    let code = match mode {
        NameTruncation::KeepExtension => 0,
        NameTruncation::Middle => 1,
        NameTruncation::End => 2,
    };
    MODE.store(code, Ordering::Relaxed);
    CUTS.with(|c| c.borrow_mut().clear());
}

fn mode() -> NameTruncation {
    match MODE.load(Ordering::Relaxed) {
        0 => NameTruncation::KeepExtension,
        1 => NameTruncation::Middle,
        _ => NameTruncation::End,
    }
}

/// A fitted line of text, and whether it had to lose characters (full name on hover).
pub struct Fitted {
    pub galley: Arc<Galley>,
    pub elided: bool,
}

#[derive(Clone)]
enum Cut {
    /// The text fits as it is.
    Whole,
    /// The text to show instead.
    Short(Arc<str>),
    /// Too narrow for a cut with both ends: an ordinary end cut.
    End,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum Keep {
    /// Start and extension (`keep_extension`), or the middle goes (`middle`).
    FileName,
    /// Always the middle goes.
    Ends,
}

/// A file or folder name of a row, fitted into `max_w`.
pub fn entry_name(painter: &Painter, name: &str, is_dir: bool, font: FontId, color: Color32, max_w: f32) -> Fitted {
    fit(painter, name, if is_dir { Keep::Ends } else { Keep::FileName }, font, color, max_w)
}

/// A path or a folder name (crumb, tab title), fitted into `max_w`: its middle goes.
pub fn ends(painter: &Painter, text: &str, font: FontId, color: Color32, max_w: f32) -> Fitted {
    fit(painter, text, Keep::Ends, font, color, max_w)
}

/// [`entry_name`] wrapped onto at most `rows` lines (grid captions); paint it at the
/// horizontal center of the target area.
pub fn wrapped_entry_name(
    painter: &Painter,
    name: &str,
    is_dir: bool,
    font: FontId,
    color: Color32,
    max_w: f32,
    rows: usize,
) -> Fitted {
    let galley = wrapped_galley(painter, name, font.clone(), color, max_w, rows);
    if !galley.elided || end_only(name) {
        let elided = galley.elided;
        return Fitted { galley, elided };
    }
    let keep = if is_dir { Keep::Ends } else { Keep::FileName };
    let w = quantize(max_w);
    let key = cut_key(painter, name, keep, &font, w, rows);
    let cut = remembered(key, || cut_wrapped(painter, name, keep, &font, w, rows));
    match cut {
        Cut::Short(s) => Fitted { galley: wrapped_galley(painter, &s, font, color, max_w, rows), elided: true },
        _ => Fitted { galley, elided: true },
    }
}

fn end_only(text: &str) -> bool {
    mode() == NameTruncation::End || soyuz_core::naming::has_hidden_controls(text)
}

fn fit(painter: &Painter, text: &str, keep: Keep, font: FontId, color: Color32, max_w: f32) -> Fitted {
    if end_only(text) {
        let galley = trunc_galley(painter, text, font, color, max_w);
        let elided = galley.elided;
        return Fitted { galley, elided };
    }
    let w = quantize(max_w);
    let key = cut_key(painter, text, keep, &font, w, 0);
    let cut = remembered(key, || cut_line(painter, text, keep, &font, w));
    match cut {
        Cut::Whole => Fitted { galley: painter.layout_job(revealed_job(text, &font, color)), elided: false },
        Cut::Short(s) => Fitted { galley: painter.layout_job(revealed_job(&s, &font, color)), elided: true },
        Cut::End => {
            let galley = trunc_galley(painter, text, font, color, max_w);
            let elided = galley.elided;
            Fitted { galley, elided }
        }
    }
}

/// Half-point steps, rounded up: dragging a column edge measures less, and a caller that
/// sized its room from the text itself must not see that text cut.
fn quantize(max_w: f32) -> f32 {
    if max_w.is_finite() { (max_w.max(8.0) * 2.0).ceil() / 2.0 } else { f32::MAX }
}

fn cut_key(painter: &Painter, text: &str, keep: Keep, font: &FontId, w: f32, rows: usize) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    keep.hash(&mut h);
    font.family.hash(&mut h);
    font.size.to_bits().hash(&mut h);
    w.to_bits().hash(&mut h);
    rows.hash(&mut h);
    painter.ctx().pixels_per_point().to_bits().hash(&mut h);
    h.finish()
}

fn remembered(key: u64, compute: impl FnOnce() -> Cut) -> Cut {
    if let Some(cut) = CUTS.with(|c| c.borrow().get(&key).cloned()) {
        return cut;
    }
    let cut = compute();
    CUTS.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= MAX_CUTS {
            c.clear();
        }
        c.insert(key, cut.clone());
    });
    cut
}

fn ellipsis_width(painter: &Painter, font: &FontId) -> f32 {
    painter.layout_no_wrap(ELLIPSIS.to_string(), font.clone(), Color32::WHITE).size().x
}

fn cut_line(painter: &Painter, text: &str, keep: Keep, font: &FontId, w: f32) -> Cut {
    let full = painter.layout_job(revealed_job(text, font, Color32::WHITE));
    if full.size().x <= w {
        return Cut::Whole;
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let advances: Vec<f32> = full.rows.iter().flat_map(|r| r.glyphs.iter().map(|g| g.advance_width)).collect();
    if advances.len() != chars.len() {
        return Cut::End;
    }
    let room = w - ellipsis_width(painter, font) - 1.0;
    let only: Vec<char> = chars.iter().map(|c| c.1).collect();
    match cut_points(&only, &advances, room, keep_extension(keep)) {
        Some((head, tail)) => Cut::Short(join(text, &chars, head, tail)),
        None => Cut::End,
    }
}

/// The caption wrapped onto `rows` lines: everything above the last line stays, and the
/// last line takes the cut.
fn cut_wrapped(painter: &Painter, text: &str, keep: Keep, font: &FontId, w: f32, rows: usize) -> Cut {
    let mut job = revealed_job(text, font, Color32::WHITE);
    job.wrap = TextWrapping { max_width: w, max_rows: usize::MAX, break_anywhere: false, ..Default::default() };
    let full = painter.layout_job(job);
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let advances: Vec<f32> = full.rows.iter().flat_map(|r| r.glyphs.iter().map(|g| g.advance_width)).collect();
    if advances.len() != chars.len() || full.rows.len() <= rows {
        return Cut::End;
    }
    let above: usize = full.rows[..rows - 1].iter().map(|r| r.glyphs.len()).sum();
    let room = w - ellipsis_width(painter, font) - 1.0;
    let rest: Vec<char> = chars[above..].iter().map(|c| c.1).collect();
    match cut_points(&rest, &advances[above..], room, keep_extension(keep)) {
        Some((head, tail)) => Cut::Short(join(text, &chars, above + head, above + tail)),
        // The last line is too short for the cut: the name loses its last line.
        None => Cut::End,
    }
}

fn keep_extension(keep: Keep) -> bool {
    keep == Keep::FileName && mode() == NameTruncation::KeepExtension
}

fn join(text: &str, chars: &[(usize, char)], head: usize, tail: usize) -> Arc<str> {
    let byte = |i: usize| chars.get(i).map_or(text.len(), |c| c.0);
    let mut out = String::with_capacity(text.len());
    out.push_str(text[..byte(head)].trim_end());
    out.push(ELLIPSIS);
    out.push_str(&text[byte(tail)..]);
    out.into()
}

/// Where the extension starts (index into `chars`). `.tar.gz` counts as one; a leading dot
/// (`.bashrc`) does not make an extension.
fn extension_start(chars: &[char]) -> Option<usize> {
    let dot = chars.iter().rposition(|c| *c == '.').filter(|d| *d > 0)?;
    let ext = &chars[dot..];
    if ext.len() < 2 || ext.len() > MAX_EXT_CHARS || ext.iter().any(|c| c.is_whitespace()) {
        return None;
    }
    let tar: Vec<char> = ".tar".chars().collect();
    if dot > tar.len() && chars[dot - tar.len()..dot].iter().map(char::to_ascii_lowercase).eq(tar) {
        return Some(dot - 4);
    }
    Some(dot)
}

/// Combining marks, joiners and variation selectors stay with the character before them.
fn joins_previous(c: char) -> bool {
    matches!(c as u32,
        0x300..=0x36F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x200D | 0x20D0..=0x20FF
        | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F | 0x1F3FB..=0x1F3FF | 0xE0100..=0xE01EF)
}

/// How many characters to keep at the start (`.0`) and from which index the tail runs
/// (`.1`), to fit `room`. `None` when no start would be left: an end cut says more.
fn cut_points(chars: &[char], advances: &[f32], room: f32, keep_ext: bool) -> Option<(usize, usize)> {
    let n = chars.len();
    let width = |r: std::ops::Range<usize>| advances[r].iter().sum::<f32>();
    let ext = if keep_ext { extension_start(chars) } else { None };
    // The extension stays unless it would take more than half of the room.
    let fixed_tail = ext.filter(|e| width(*e..n) * 2.0 <= room);
    let head_room = match fixed_tail {
        Some(e) => room - width(e..n),
        None => room / 2.0,
    };
    let mut head = 0;
    let mut used = 0.0;
    while head < n && used + advances[head] <= head_room {
        used += advances[head];
        head += 1;
    }
    while head > 0 && head < n && joins_previous(chars[head]) {
        head -= 1;
        used -= advances[head];
    }
    if head == 0 {
        return None;
    }
    let tail = match fixed_tail {
        Some(e) => e,
        None => {
            let mut tail = n;
            let mut left = room - used;
            while tail > head && advances[tail - 1] <= left {
                tail -= 1;
                left -= advances[tail];
            }
            while tail < n && joins_previous(chars[tail]) {
                tail += 1;
            }
            tail
        }
    };
    Some((head, tail.max(head)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character 10 wide.
    fn cut(name: &str, room: f32, keep_ext: bool) -> Option<String> {
        let chars: Vec<char> = name.chars().collect();
        let (head, tail) = cut_points(&chars, &vec![10.0; chars.len()], room, keep_ext)?;
        let s = |r: &[char]| r.iter().collect::<String>();
        Some(format!("{}…{}", s(&chars[..head]), s(&chars[tail..])))
    }

    #[test]
    fn the_extension_stays() {
        assert_eq!(cut("report-final-v3.pdf", 100.0, true).unwrap(), "report….pdf");
    }

    #[test]
    fn the_middle_goes() {
        assert_eq!(cut("report-final-v3.pdf", 100.0, false).unwrap(), "repor…3.pdf");
    }

    #[test]
    fn archives_keep_both_extensions() {
        assert_eq!(cut("backup-of-everything.tar.gz", 140.0, true).unwrap(), "backup-….tar.gz");
    }

    #[test]
    fn an_extension_too_long_for_the_room_is_not_kept() {
        // `.pdf` would take more than half of 60: the middle goes instead.
        assert_eq!(cut("report.pdf", 60.0, true).unwrap(), "rep…pdf");
    }

    #[test]
    fn leading_dots_and_long_tails_are_no_extension() {
        assert_eq!(extension_start(&".bashrc".chars().collect::<Vec<_>>()), None);
        assert_eq!(extension_start(&"a.backup-2021-final".chars().collect::<Vec<_>>()), None);
        assert_eq!(extension_start(&"my file.tx t".chars().collect::<Vec<_>>()), None);
        assert_eq!(extension_start(&"a.TAR.gz".chars().collect::<Vec<_>>()), Some(1));
        assert_eq!(extension_start(&"a.txt".chars().collect::<Vec<_>>()), Some(1));
    }

    #[test]
    fn no_start_left_means_an_end_cut() {
        assert_eq!(cut("abcdef", 5.0, false), None);
    }

    #[test]
    fn combining_marks_stay_with_their_letter() {
        // `e` + combining acute at the cut: both go.
        let name = "abe\u{301}defgh";
        assert_eq!(cut(name, 60.0, false).unwrap(), "ab…efgh");
    }

    /// One real layout: the result fits, and repainting does not measure again.
    #[test]
    fn real_text_fits_and_is_remembered() {
        set_mode(NameTruncation::KeepExtension);
        let ctx = egui::Context::default();
        let mut out = Vec::new();
        for _ in 0..2 {
            let mut output = ctx.run_ui(Default::default(), |ui| {
                let f = entry_name(
                    ui.painter(),
                    "a-rather-long-report-name-final-v3.pdf",
                    false,
                    FontId::proportional(13.0),
                    Color32::WHITE,
                    120.0,
                );
                assert!(f.elided);
                assert!(f.galley.size().x <= 120.0, "{}", f.galley.size().x);
                out.push(f.galley.job.text.clone());
                let whole = entry_name(ui.painter(), "a.pdf", false, FontId::proportional(13.0), Color32::WHITE, 120.0);
                assert!(!whole.elided);
            });
            output.textures_delta.clear();
        }
        assert!(out[0].ends_with("….pdf"), "{}", out[0]);
        assert_eq!(out[0], out[1]);
        assert_eq!(CUTS.with(|c| c.borrow().len()), 2);
    }
}
