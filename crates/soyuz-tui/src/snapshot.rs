//! Snapshot tests of the terminal UI (Phase T8): a drawn `Buffer` serialized as a text
//! grid, a style mask and a legend, compared with `tests/snapshots/<name>.snap`.
//!
//! - [`serialize`]: deterministic, LF only. Each row is framed by `|` so trailing spaces
//!   stay visible; wide characters take their cell plus a `~` continuation cell in both
//!   grids. The header names the size and the ratatui version ([`RATATUI`]).
//! - [`assert_snapshot`]: compare with the stored file, or write it under
//!   `SOYUZ_BLESS_SNAPSHOTS=1` ([`BLESS_ENV`], refused on CI). On a mismatch (or a
//!   missing file) it writes `<name>.snap.new` next to it and panics with the first
//!   differing lines and the bless command.
//! - [`SnapOpts`]: text only or with styles, substring redaction (same width) and masked
//!   rects (blanked before serializing: the crumbs row, the status row's volatile parts).
//!
//! The format:
//!
//! ```text
//! # soyuz-tui snapshot: view__browse_dual (100x30, ratatui 0.30.2)
//! |╭ data ──╮ … |      text rows
//! --- styles
//! |AAAABBBBA… |      one letter per cell, assigned in first-seen order (row-major)
//! --- legend
//! A fg=Reset bg=Reset
//! B fg=Blue bg=Reset +BOLD
//! C fg=#1E1E2E bg=#89B4FA +REVERSED
//! ```
//!
//! Colours are named for the 16 ANSI colours, `#RRGGBB` for RGB and `i<n>` for indexed
//! ones; an underline colour (`ul=`) is listed only when it is set to something other
//! than `Reset` and ratatui's `underline-color` feature is on.
//!
//! Used by the in-crate view snapshots (`view::snapshots`, tier A) and the headless app
//! snapshots (`tests/headless/snapshots.rs`, tier B, through `common::assert_screen`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr;

/// Set to `1` to write the snapshots instead of comparing (never on CI).
pub const BLESS_ENV: &str = "SOYUZ_BLESS_SNAPSHOTS";

/// The ratatui version the snapshots were drawn with (in every header, so an upgrade that
/// changes rendering is obvious in the diffs; a unit test keeps it equal to `Cargo.lock`).
pub const RATATUI: &str = "0.30.2";

/// How a buffer is serialized.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SnapOpts {
    /// Add the style mask and its legend (else text only, e.g. for tiny-size sweeps).
    pub styles: bool,
    /// Volatile substrings and their replacements (padded or cut to the same width, so
    /// the cell count stays fixed), e.g. the sandbox path → `<DATA>`.
    pub redact: Vec<(String, String)>,
    /// Rects blanked before serializing (text and style), e.g. from `App::regions()`.
    pub mask: Vec<Rect>,
}

impl SnapOpts {
    /// Text and styles.
    pub fn styled() -> SnapOpts {
        SnapOpts { styles: true, ..SnapOpts::default() }
    }

    /// Text only.
    pub fn text() -> SnapOpts {
        SnapOpts::default()
    }

    /// These options with `rects` blanked too.
    pub fn masked(mut self, rects: impl IntoIterator<Item = Rect>) -> SnapOpts {
        self.mask.extend(rects);
        self
    }

    /// These options with `from` replaced by `to` (same width).
    pub fn redacted(mut self, from: impl Into<String>, to: impl Into<String>) -> SnapOpts {
        self.redact.push((from.into(), to.into()));
        self
    }
}

/// Where the snapshot files live: `crates/soyuz-tui/tests/snapshots`.
pub fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("snapshots")
}

/// The buffer's rows as text (one string per row, cells' symbols in order).
pub fn lines(buf: &Buffer) -> Vec<String> {
    let area = buf.area;
    (area.top()..area.bottom())
        .map(|y| (area.left()..area.right()).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect()
}

/// `buf` in the snapshot format, without the header line.
pub fn serialize(buf: &Buffer, opts: &SnapOpts) -> String {
    let buf = prepared(buf, opts);
    let area = buf.area;
    let mut text = String::new();
    let mut mask = String::new();
    let mut legend: Vec<String> = Vec::new();
    let mut letters: HashMap<String, char> = HashMap::new();
    for y in area.top()..area.bottom() {
        text.push('|');
        mask.push('|');
        let mut covered = 0usize;
        for x in area.left()..area.right() {
            let cell = &buf[(x, y)];
            if covered > 0 {
                // The cell under the previous wide character (ratatui resets it).
                covered -= 1;
                text.push('~');
                mask.push('~');
                continue;
            }
            let sym = cell.symbol();
            let width = sym.width();
            covered = width.saturating_sub(1);
            text.push_str(if sym.is_empty() { " " } else { sym });
            let key = style_key(cell);
            let letter = *letters.entry(key.clone()).or_insert_with(|| {
                let l = letter(legend.len());
                legend.push(format!("{l} {key}"));
                l
            });
            mask.push(letter);
        }
        text.push_str("|\n");
        mask.push_str("|\n");
    }
    if !opts.styles {
        return text;
    }
    let mut out = text;
    out.push_str("--- styles\n");
    out.push_str(&mask);
    out.push_str("--- legend\n");
    for l in legend {
        out.push_str(&l);
        out.push('\n');
    }
    out
}

/// The whole snapshot file of `buf` (header + [`serialize`]).
pub fn document(name: &str, buf: &Buffer, opts: &SnapOpts) -> String {
    let area = buf.area;
    format!(
        "# soyuz-tui snapshot: {name} ({}x{}, ratatui {RATATUI})\n{}",
        area.width,
        area.height,
        serialize(buf, opts)
    )
}

/// Compare `buf` with snapshot `name` (`<dir>/<name>.snap`), or bless it.
///
/// Snapshots are drawn in en-US: the formatting locale is process-wide, so a test that
/// switched it would make every snapshot drawn at the same time flaky. This asserts it.
pub fn assert_snapshot(name: &str, buf: &Buffer, opts: &SnapOpts) {
    assert_eq!(
        soyuz_core::locale::current(),
        soyuz_core::locale::Locale::EnUs,
        "snapshot {name}: snapshots are drawn in en-US (a test switched the process-wide locale)"
    );
    assert_file(&format!("{name}.snap"), &document(name, buf, opts));
}

/// Compare `fresh` with `<dir>/<file>` (line ends normalized), or write it under
/// [`BLESS_ENV`]. On a mismatch writes `<file>.new` and panics with a short diff.
pub fn assert_file(file: &str, fresh: &str) {
    let path = dir().join(file);
    let new = PathBuf::from(format!("{}.new", path.display()));
    if blessing() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("snapshot folder");
        }
        std::fs::write(&path, fresh).unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
        let _ = std::fs::remove_file(&new);
        return;
    }
    let stored = std::fs::read_to_string(&path).ok().map(|s| s.replace("\r\n", "\n"));
    if stored.as_deref() == Some(fresh) {
        let _ = std::fs::remove_file(&new);
        return;
    }
    let _ = std::fs::write(&new, fresh);
    let how = format!("bless with {BLESS_ENV}=1 (never on CI), or compare with {}", new.display());
    match stored {
        None => panic!("snapshot {file} is missing (wrote {}); {how}", new.display()),
        Some(stored) => panic!("snapshot {file} differs:\n{}{how}", diff(&stored, fresh)),
    }
}

/// `SOYUZ_BLESS_SNAPSHOTS=1`; panics when it is set on CI.
fn blessing() -> bool {
    let on = std::env::var(BLESS_ENV).is_ok_and(|v| v == "1");
    if on && on_ci(&|k| std::env::var(k).ok()) {
        panic!("{BLESS_ENV} is refused on CI: bless locally (or from the uploaded *.snap.new files)");
    }
    on
}

/// Running on CI (`GITHUB_ACTIONS` or `CI` set and not `false`).
fn on_ci(env: &dyn Fn(&str) -> Option<String>) -> bool {
    ["GITHUB_ACTIONS", "CI"].iter().any(|k| env(k).is_some_and(|v| !v.is_empty() && v != "false" && v != "0"))
}

/// The first differing lines (at most five), with their line numbers.
fn diff(stored: &str, fresh: &str) -> String {
    let (a, b): (Vec<&str>, Vec<&str>) = (stored.lines().collect(), fresh.lines().collect());
    let mut out = String::new();
    let mut shown = 0;
    for i in 0..a.len().max(b.len()) {
        let (old, new) = (a.get(i), b.get(i));
        if old == new {
            continue;
        }
        out.push_str(&format!(
            "  line {}:\n    - {}\n    + {}\n",
            i + 1,
            old.unwrap_or(&"<none>"),
            new.unwrap_or(&"<none>")
        ));
        shown += 1;
        if shown == 5 {
            out.push_str("  …\n");
            break;
        }
    }
    out
}

/// A copy of `buf` with the masks blanked and the redactions applied.
fn prepared(buf: &Buffer, opts: &SnapOpts) -> Buffer {
    let mut buf = buf.clone();
    let area = buf.area;
    for r in &opts.mask {
        let r = r.intersection(area);
        for y in r.top()..r.bottom() {
            for x in r.left()..r.right() {
                buf[(x, y)].reset();
            }
        }
    }
    for (from, to) in &opts.redact {
        if from.is_empty() {
            continue;
        }
        for y in area.top()..area.bottom() {
            redact_row(&mut buf, y, from, to);
        }
    }
    buf
}

/// Replace each `from` in row `y` (spelled by consecutive cells) by `to`, padded with
/// spaces or cut to the same number of cells. Styles stay.
fn redact_row(buf: &mut Buffer, y: u16, from: &str, to: &str) {
    let area = buf.area;
    let mut x = area.left();
    while x < area.right() {
        // The cells from `x` that spell `from`.
        let mut spelled = String::new();
        let mut end = x;
        while end < area.right() && spelled.len() < from.len() {
            spelled.push_str(buf[(end, y)].symbol());
            end += 1;
        }
        if spelled != from {
            x += 1;
            continue;
        }
        let mut chars = to.chars();
        for cx in x..end {
            let c = chars.next().unwrap_or(' ');
            buf[(cx, y)].set_char(c);
        }
        x = end;
    }
}

/// A cell's style for the legend.
fn style_key(cell: &Cell) -> String {
    let mut key = format!("fg={} bg={}", color(cell.fg), color(cell.bg));
    if let Some(ul) = underline_color(cell) {
        key.push_str(&format!(" ul={ul}"));
    }
    for m in cell.modifier.iter() {
        key.push_str(&format!(" +{}", modifier(m)));
    }
    key
}

fn color(c: Color) -> String {
    match c {
        Color::Indexed(i) => format!("i{i}"),
        c => c.to_string(),
    }
}

fn modifier(m: Modifier) -> String {
    let name = format!("{m:?}");
    // `Modifier(BOLD)` in some ratatui versions, `BOLD` in others.
    name.trim_start_matches("Modifier(").trim_end_matches(')').to_string()
}

/// The cell's underline colour when ratatui keeps one (feature `underline-color`) and it
/// is not `Reset`. Read through the style's `Debug` text, which names it only when the
/// feature is on, so this compiles either way.
fn underline_color(cell: &Cell) -> Option<String> {
    let debug = format!("{:?}", cell.style());
    let start = debug.find(".underline_color(Color::")? + ".underline_color(Color::".len();
    let rest = &debug[start..];
    // The colour's own `Debug` may hold parentheses (`Rgb(1, 2, 3)`).
    let mut depth = 0usize;
    let end = rest.char_indices().find_map(|(i, c)| match c {
        '(' => {
            depth += 1;
            None
        }
        ')' if depth == 0 => Some(i),
        ')' => {
            depth -= 1;
            None
        }
        _ => None,
    })?;
    let name = &rest[..end];
    (name != "Reset").then(|| name.to_string())
}

/// The `n`-th style letter: `A`–`Z`, `a`–`z`, `0`–`9`, then Latin-1 letters.
fn letter(n: usize) -> char {
    const ASCII: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    ASCII.chars().nth(n).unwrap_or_else(|| char::from_u32(0xC0 + (n - ASCII.len()) as u32).unwrap_or('?'))
}

/// Read a snapshot file back (tests; LF).
pub fn read(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.replace("\r\n", "\n"))
}

#[cfg(test)]
mod tests {
    use ratatui::style::Style;

    use super::*;

    #[test]
    fn lines_follow_the_cells() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 2));
        buf.set_string(0, 0, "ab", Style::default());
        assert_eq!(lines(&buf), ["ab ", "   "]);
        assert_eq!(serialize(&buf, &SnapOpts::text()), "|ab |\n|   |\n");
    }

    #[test]
    fn styles_get_letters_in_first_seen_order() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 2));
        buf.set_string(2, 0, "x", Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD));
        buf.set_string(0, 1, "y", Style::new().bg(Color::Rgb(0x1e, 0x1e, 0x2e)).add_modifier(Modifier::REVERSED));
        buf.set_string(1, 1, "z", Style::new().fg(Color::Indexed(123)));
        buf.set_string(3, 1, "w", Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD));
        assert_eq!(
            serialize(&buf, &SnapOpts::styled()),
            "|  x |\n|yz w|\n--- styles\n|AABA|\n|CDAB|\n--- legend\n\
             A fg=Reset bg=Reset\n\
             B fg=Blue bg=Reset +BOLD\n\
             C fg=Reset bg=#1E1E2E +REVERSED\n\
             D fg=i123 bg=Reset\n"
        );
    }

    #[test]
    fn wide_characters_have_a_continuation_cell_in_both_grids() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 5, 1));
        buf.set_string(0, 0, "日a本", Style::new().fg(Color::Red));
        assert_eq!(lines(&buf), ["日 a本 "]);
        assert_eq!(
            serialize(&buf, &SnapOpts::styled()),
            "|日~a本~|\n--- styles\n|A~AA~|\n--- legend\nA fg=Red bg=Reset\n"
        );
    }

    #[test]
    fn redaction_keeps_the_width_and_masks_blank_rects() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 2));
        buf.set_string(0, 0, "C:\\tmp\\x b", Style::new().fg(Color::Green));
        buf.set_string(0, 1, "C:\\tmp\\x", Style::default());
        let opts = SnapOpts::styled().redacted("C:\\tmp\\x", "<DATA>").masked([Rect::new(0, 1, 3, 1)]);
        let s = serialize(&buf, &opts);
        assert!(s.starts_with("|<DATA>   b  |\n|   tmp\\x    |\n"), "{s}");
        // The redacted cells keep their style.
        assert!(s.contains("|AAAAAAAAAABB|"), "{s}");
        // A longer replacement is cut to the cells it replaces.
        let s = serialize(&buf, &SnapOpts::text().redacted("tmp", "<SANDBOX>"));
        assert!(s.starts_with("|C:\\<SA\\x b  |"), "{s}");
    }

    #[test]
    fn the_diff_names_the_first_differing_lines() {
        let d = diff(
            "a
b
c
", "a
B
",
        );
        assert!(
            d.contains(
                "line 2:
    - b
    + B"
            ) && d.contains(
                "line 3:
    - c
    + <none>"
            ),
            "{d}"
        );
        assert!(!d.contains("line 1"), "{d}");
        let many: String = (0..10)
            .map(|i| {
                format!(
                    "{i}
"
                )
            })
            .collect();
        assert!(diff(&many, "").ends_with(
            "  …
"
        ));
    }

    #[test]
    fn the_stored_files_compare_with_crlf_line_ends() {
        // A checked-out snapshot with CRLF (no `.gitattributes`) still matches.
        let name = "snapshot__crlf_probe";
        let path = dir().join(format!("{name}.snap"));
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        buf.set_string(0, 0, "ok", Style::default());
        let doc = document(name, &buf, &SnapOpts::text());
        // Write the probe with CRLF, compare, remove it again.
        std::fs::write(&path, doc.replace('\n', "\r\n")).unwrap();
        let result = std::panic::catch_unwind(|| {
            if std::env::var(BLESS_ENV).is_err() {
                assert_snapshot(name, &buf, &SnapOpts::text());
            }
        });
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(dir().join(format!("{name}.snap.new")));
        result.expect("CRLF snapshot matches");
    }

    #[test]
    fn bless_is_refused_on_ci() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
        };
        assert!(on_ci(&env(&[("GITHUB_ACTIONS", "true")])));
        assert!(on_ci(&env(&[("CI", "1")])));
        assert!(!on_ci(&env(&[("CI", "false")])));
        assert!(!on_ci(&env(&[])));
    }

    #[test]
    fn the_header_names_size_and_ratatui() {
        let buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        assert_eq!(
            document("x__y", &buf, &SnapOpts::text()),
            format!("# soyuz-tui snapshot: x__y (2x1, ratatui {RATATUI})\n|  |\n")
        );
        // RATATUI follows Cargo.lock (re-bless the snapshots after an upgrade).
        let lock = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock")).unwrap();
        let lock = lock.replace("\r\n", "\n");
        let needle = "name = \"ratatui\"\nversion = \"";
        let at = lock.find(needle).expect("ratatui in Cargo.lock") + needle.len();
        let version = &lock[at..at + lock[at..].find('"').unwrap()];
        assert_eq!(version, RATATUI, "ratatui was upgraded: update snapshot::RATATUI and re-bless the snapshots");
    }

    #[test]
    fn letters_run_past_the_ascii_ones() {
        assert_eq!((letter(0), letter(25), letter(26), letter(61), letter(62)), ('A', 'Z', 'a', '9', 'À'));
    }
}
