//! Snapshot tests of the terminal UI (Phase T8): a drawn `Buffer` serialized as a text
//! grid, a style mask and a legend, compared with `tests/snapshots/<name>.snap`.
//!
//! - [`serialize`]: deterministic, LF only. Each row is framed by `|` so trailing spaces
//!   stay visible; wide characters take their cell plus a `~` continuation cell in both
//!   grids. The header names the size, the look and the ratatui version.
//! - [`assert_snapshot`]: compare with the stored file, or write it under
//!   `SOYUZ_BLESS_SNAPSHOTS=1` ([`BLESS_ENV`], refused on CI). On a mismatch (or a
//!   missing file) it writes `<name>.snap.new` next to it and panics with the first
//!   differing line and the bless command.
//! - [`SnapOpts`]: text only or with styles, substring redaction (same width) and masked
//!   rects (blanked before serializing: the crumbs row, the status row's volatile parts).
//!
//! Used by the in-crate view snapshots (`view::snapshots`, tier A) and the headless app
//! snapshots (`tests/headless/snapshots.rs`, tier B, through `common::assert_screen`).
//!
//! Wave 0 skeleton: the API; the format and the compare come with Phase T8 P2.

use std::path::PathBuf;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// Set to `1` to write the snapshots instead of comparing (never on CI).
pub const BLESS_ENV: &str = "SOYUZ_BLESS_SNAPSHOTS";

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

/// `buf` in the snapshot format.
pub fn serialize(buf: &Buffer, opts: &SnapOpts) -> String {
    let _ = opts;
    lines(buf).into_iter().map(|l| format!("|{l}|\n")).collect()
}

/// Compare `buf` with snapshot `name` (`<dir>/<name>.snap`), or bless it.
pub fn assert_snapshot(name: &str, buf: &Buffer, opts: &SnapOpts) {
    let _ = (buf, opts);
    panic!("snapshot {name}: snapshot compare is not implemented yet (Phase T8 P2)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_follow_the_cells() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 3, 2));
        buf.set_string(0, 0, "ab", ratatui::style::Style::default());
        assert_eq!(lines(&buf), ["ab ", "   "]);
        assert_eq!(serialize(&buf, &SnapOpts::text()), "|ab |\n|   |\n");
    }
}
