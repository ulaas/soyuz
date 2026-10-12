//! Glyphs for the core's frontend-neutral icon ids ([`IconId`]) and file classes.
//!
//! Stub of T2.0: ASCII placeholders only. T2.2 fills the Unicode and Nerd Font sets and
//! wires the set to `[tui]` config (T2.1). Every glyph must be one or two cells wide and
//! terminal-safe.

use soyuz_appcore::file_class::FileClass;
use soyuz_appcore::icon::IconId;

/// Which glyphs the terminal can show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum IconSet {
    /// Plain ASCII: works everywhere.
    #[default]
    Ascii,
    /// Unicode symbols from common terminal fonts.
    Unicode,
    /// Nerd Font private-use glyphs.
    Nerd,
}

/// The glyph of `id` in `set`.
pub fn glyph(id: IconId, _set: IconSet) -> &'static str {
    match id {
        IconId::Folder | IconId::FolderOpen | IconId::FolderSimple => "/",
        IconId::Link | IconId::LinkSimple => "@",
        IconId::X | IconId::XCircle | IconId::XSquare => "x",
        IconId::CaretRight => ">",
        IconId::CaretLeft => "<",
        IconId::CaretDown => "v",
        IconId::CaretUp => "^",
        _ => "*",
    }
}

/// The icon of a file class.
pub fn class_icon(class: FileClass) -> IconId {
    match class {
        FileClass::Folder => IconId::Folder,
        FileClass::Image => IconId::Image,
        FileClass::Video => IconId::FilmStrip,
        FileClass::Audio => IconId::MusicNote,
        FileClass::Archive => IconId::FileZip,
        FileClass::Code => IconId::FileCode,
        FileClass::Pdf => IconId::FilePdf,
        FileClass::Doc => IconId::FileDoc,
        FileClass::Sheet => IconId::FileXls,
        FileClass::Slides => IconId::FilePpt,
        FileClass::Text => IconId::FileText,
        FileClass::Exe => IconId::AppWindow,
        FileClass::Font => IconId::TextAa,
        FileClass::Link => IconId::Link,
        FileClass::Other => IconId::File,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::text::width;

    #[test]
    fn every_glyph_is_narrow_and_safe() {
        for set in [IconSet::Ascii, IconSet::Unicode, IconSet::Nerd] {
            for &id in IconId::ALL {
                let g = glyph(id, set);
                assert!((1..=2).contains(&width(g)), "{id:?} {set:?}: {g:?}");
                assert!(!g.chars().any(char::is_control), "{id:?}");
            }
        }
        assert_eq!(glyph(class_icon(FileClass::Folder), IconSet::Ascii), "/");
    }
}
