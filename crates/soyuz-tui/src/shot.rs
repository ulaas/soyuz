//! `shot:<file>` of `SOYUZ_SCRIPT` in the terminal UI (Phase T8): the drawn `Buffer` saved
//! in the format the extension names ([`ShotFormat`]):
//! - `.txt`: the text, one line per row;
//! - `.snap`: the snapshot format (`crate::snapshot::serialize` with styles);
//! - `.svg`: one `<text>` per run with per-cell `x` positions (so columns line up in any
//!   monospace font), rect backgrounds, the theme's colours; box drawing and block
//!   characters (U+2500–U+259F) drawn as lines and rects, so they join between rows.
//!
//! Wave 0 skeleton: `.txt` works; `.snap` and `.svg` come with Phase T8 P2.

use std::io;
use std::path::Path;

use ratatui::buffer::Buffer;

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

/// `buf` as an SVG document.
pub fn svg(buf: &Buffer) -> String {
    let _ = buf;
    String::new()
}

/// Save `buf` to `path` in the format its extension names.
pub fn write(path: &Path, buf: &Buffer) -> io::Result<()> {
    match ShotFormat::from_path(path) {
        Some(ShotFormat::Txt) => std::fs::write(path, text(buf)),
        Some(ShotFormat::Snap | ShotFormat::Svg) => {
            Err(io::Error::new(io::ErrorKind::Unsupported, "not implemented yet (Phase T8 P2)"))
        }
        None => Err(io::Error::new(io::ErrorKind::InvalidInput, "unknown shot format (use .txt, .snap or .svg)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_from_extension() {
        assert_eq!(ShotFormat::from_path(Path::new("a/b.TXT")), Some(ShotFormat::Txt));
        assert_eq!(ShotFormat::from_path(Path::new("b.svg")), Some(ShotFormat::Svg));
        assert_eq!(ShotFormat::from_path(Path::new("b.snap")), Some(ShotFormat::Snap));
        assert_eq!(ShotFormat::from_path(Path::new("b.png")), None);
        assert_eq!(ShotFormat::from_path(Path::new("b")), None);
    }
}
