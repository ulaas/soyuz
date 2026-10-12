//! Glyphs for the core's frontend-neutral icon ids ([`IconId`]) and file classes.
//!
//! Three sets ([`IconSet`], `[tui] icons`):
//! * **Nerd**: Nerd Font private-use glyphs, hand-mapped by meaning onto the Font Awesome
//!   4.7 range (`nf-fa-*`, U+F000–U+F2E0) that every Nerd Font patches in, so one family
//!   covers them all. Ids without an exact match fall back to the nearest family glyph
//!   (every git id → `code-fork`, every selection id → `object-group`, …).
//! * **Unicode**: BMP symbols with text presentation (arrows, geometric shapes, misc
//!   technical), no emoji, so terminals draw them one cell wide.
//! * **Ascii**: one or two plain characters; works everywhere.
//!
//! Every glyph is one or two cells wide and free of control characters (tested).

use soyuz_appcore::file_class::FileClass;
use soyuz_appcore::icon::IconId;
use soyuz_core::config::{TuiIcons, TuiSection};

/// Which glyphs the terminal can show. The default (ASCII) is what the plain theme of the
/// view tests draws with; the app reads `[tui] icons` ([`IconSet::from_config`], Unicode
/// by default).
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

impl IconSet {
    /// The configured icon set (`[tui] icons`).
    pub fn from_config(tui: &TuiSection) -> IconSet {
        match tui.icons {
            TuiIcons::Nerd => IconSet::Nerd,
            TuiIcons::Unicode => IconSet::Unicode,
            TuiIcons::Ascii => IconSet::Ascii,
        }
    }
}

/// The glyph of `id` in `set`.
pub fn glyph(id: IconId, set: IconSet) -> &'static str {
    let (nerd, unicode, ascii) = table(id);
    match set {
        IconSet::Nerd => nerd,
        IconSet::Unicode => unicode,
        IconSet::Ascii => ascii,
    }
}

/// `(nerd, unicode, ascii)` of every id.
fn table(id: IconId) -> (&'static str, &'static str, &'static str) {
    use IconId as I;
    match id {
        // Windows, apps.
        I::AppWindow => ("\u{f2d0}", "▣", "[]"),
        I::Browsers => ("\u{f2d2}", "❐", "[]"),
        I::Desktop => ("\u{f108}", "▭", "D"),
        I::DesktopTower => ("\u{f233}", "▯", "PC"),
        I::Sidebar | I::SidebarSimple => ("\u{f0db}", "▌", "|["),
        I::Terminal | I::TerminalWindow => ("\u{f120}", "❯", ">_"),
        I::Command => ("\u{f292}", "⌘", ":"),
        I::Keyboard => ("\u{f11c}", "⌨", "kb"),
        // Arrows.
        I::ArrowsClockwise => ("\u{f021}", "⟳", "<>"),
        I::ArrowsDownUp => ("\u{f07d}", "⇅", "^v"),
        I::ArrowsLeftRight => ("\u{f07e}", "⇄", "<>"),
        I::ArrowBendDownRight => ("\u{f149}", "↳", "->"),
        I::ArrowBendUpRight | I::ArrowUUpRight => ("\u{f064}", "↷", "~>"),
        I::ArrowClockwise => ("\u{f01e}", "↻", "@>"),
        I::ArrowCounterClockwise => ("\u{f0e2}", "↺", "<@"),
        I::ArrowFatLeft => ("\u{f0a8}", "⇦", "<="),
        I::ArrowFatRight => ("\u{f0a9}", "⇨", "=>"),
        I::ArrowLeft => ("\u{f060}", "←", "<-"),
        I::ArrowRight => ("\u{f061}", "→", "->"),
        I::ArrowUp => ("\u{f062}", "↑", "^"),
        I::ArrowLineDown => ("\u{f175}", "⤓", "v_"),
        I::ArrowLineUp => ("\u{f176}", "⤒", "^-"),
        I::ArrowSquareOut | I::ArrowSquareUpRight => ("\u{f08e}", "↗", "^>"),
        I::ArrowSquareRight => ("\u{f152}", "⇥", "->"),
        I::ArrowUUpLeft => ("\u{f112}", "↶", "<~"),
        I::CaretDoubleDown => ("\u{f103}", "⇊", "vv"),
        I::CaretDoubleLeft => ("\u{f100}", "«", "<<"),
        I::CaretDoubleRight => ("\u{f101}", "»", ">>"),
        I::CaretDoubleUp => ("\u{f102}", "⇈", "^^"),
        I::CaretDown => ("\u{f107}", "▾", "v"),
        I::CaretLeft => ("\u{f104}", "◂", "<"),
        I::CaretRight => ("\u{f105}", "▸", ">"),
        I::CaretUp => ("\u{f106}", "▴", "^"),
        I::DownloadSimple => ("\u{f019}", "⤓", "dl"),
        I::FlowArrow => ("\u{f074}", "↝", "~>"),
        I::SortAscending => ("\u{f160}", "⇡", "^="),
        // Files and folders.
        I::File => ("\u{f016}", "□", "-"),
        I::FileCode => ("\u{f1c9}", "λ", "<>"),
        I::FileDoc => ("\u{f1c2}", "¶", "W"),
        I::FileMagnifyingGlass | I::FileSearch => ("\u{f002}", "⌕", "?f"),
        I::FilePdf => ("\u{f1c1}", "◰", "P"),
        I::FilePlus => ("\u{f196}", "⊕", "+f"),
        I::FilePpt => ("\u{f1c4}", "◳", "S"),
        I::FileText => ("\u{f0f6}", "≡", "t"),
        I::FileXls => ("\u{f1c3}", "▦", "X"),
        I::FileZip | I::Archive => ("\u{f1c6}", "▨", "Z"),
        I::Folder | I::FolderSimple => ("\u{f07b}", "■", "/"),
        I::FolderOpen => ("\u{f07c}", "▣", "/"),
        I::FolderPlus => ("\u{f114}", "⊞", "+/"),
        I::Binary => ("\u{f2db}", "⌗", "01"),
        I::Scroll => ("\u{f15c}", "§", "S"),
        I::Notepad => ("\u{f24a}", "✎", "n"),
        I::FloppyDisk => ("\u{f0c7}", "⎙", "sv"),
        // Media.
        I::Image => ("\u{f1c5}", "◩", "I"),
        I::Images => ("\u{f03e}", "◩", "Im"),
        I::FilmSlate | I::FilmStrip => ("\u{f1c8}", "►", "V"),
        I::MusicNote => ("\u{f001}", "♪", "A"),
        I::MusicNotes => ("\u{f1c7}", "♫", "A"),
        I::Waveform => ("\u{f028}", "≋", "~"),
        I::Play => ("\u{f04b}", "►", ">"),
        I::Pause => ("\u{f04c}", "‖", "||"),
        I::Stop => ("\u{f04d}", "▪", "[]"),
        I::StopCircle => ("\u{f28d}", "⊙", "()"),
        I::GameController => ("\u{f11b}", "⌖", "G"),
        // Edit and clipboard.
        I::Clipboard | I::ClipboardText => ("\u{f0ea}", "⎘", "cb"),
        I::Copy | I::CopySimple => ("\u{f0c5}", "⧉", "cp"),
        I::Scissors => ("\u{f0c4}", "✂", "x"),
        I::PencilLine | I::PencilSimple => ("\u{f040}", "✎", "ed"),
        I::NotePencil => ("\u{f044}", "✎", "ed"),
        I::Textbox => ("\u{f246}", "▭", "[_"),
        I::TextAa => ("\u{f031}", "Aa", "Aa"),
        I::TextT => ("\u{f034}", "T", "T"),
        I::TextColumns => ("\u{f039}", "▥", "=="),
        I::Broom => ("\u{f12d}", "⌫", "cl"),
        I::Trash => ("\u{f1f8}", "⌧", "rm"),
        I::TrashSimple => ("\u{f014}", "⌧", "rm"),
        I::MagicWand => ("\u{f0d0}", "✦", "*"),
        I::Sparkle => ("\u{f069}", "✦", "*"),
        // Selection.
        I::CheckSquare | I::CheckSquareOffset => ("\u{f046}", "☑", "[x"),
        I::SelectionAll => ("\u{f247}", "▣", "[*"),
        I::SelectionInverse => ("\u{f248}", "◪", "[~"),
        I::SelectionPlus => ("\u{f196}", "⊞", "[+"),
        I::SelectionSlash => ("\u{f147}", "⊘", "[-"),
        I::Rectangle => ("\u{f096}", "▭", "[]"),
        I::RectangleDashed => ("\u{f096}", "⬚", "[]"),
        // Search and filters.
        I::MagnifyingGlass => ("\u{f002}", "⌕", "?"),
        I::MagnifyingGlassMinus => ("\u{f010}", "⊖", "-"),
        I::MagnifyingGlassPlus => ("\u{f00e}", "⊕", "+"),
        I::Funnel | I::FunnelX => ("\u{f0b0}", "▽", "Y"),
        // Layout and views.
        I::Columns | I::SquareSplitHorizontal => ("\u{f0db}", "◫", "||"),
        I::Rows => ("\u{f00b}", "☰", "=="),
        I::List => ("\u{f03a}", "☰", "=="),
        I::ListBullets => ("\u{f0ca}", "•", "*"),
        I::ListDashes => ("\u{f0c9}", "≡", "--"),
        I::SquaresFour => ("\u{f009}", "⊞", "##"),
        I::Stack => ("\u{f24d}", "≣", "="),
        I::TreeStructure => ("\u{f0e8}", "├", "+-"),
        I::TreeView => ("\u{f03c}", "├", "|-"),
        I::ChartPie => ("\u{f200}", "◔", "%"),
        I::Sliders => ("\u{f1de}", "≑", "=|"),
        I::CircleHalf => ("\u{f042}", "◐", "()"),
        // Places and devices.
        I::House => ("\u{f015}", "⌂", "~"),
        I::HardDrive => ("\u{f0a0}", "▭", "hd"),
        I::HardDrives => ("\u{f233}", "☰", "hd"),
        I::Cloud => ("\u{f0c2}", "☁", "cl"),
        I::Globe => ("\u{f0ac}", "◎", "@"),
        I::Planet => ("\u{f135}", "♁", "o"),
        I::Briefcase => ("\u{f0b1}", "◫", "B"),
        I::BookOpen => ("\u{f02d}", "§", "B"),
        I::Tray => ("\u{f01c}", "⎕", "[]"),
        I::Cube => ("\u{f1b2}", "◈", "#"),
        I::PuzzlePiece => ("\u{f12e}", "✣", "+"),
        // Marks.
        I::BookmarkSimple => ("\u{f02e}", "⚑", "B"),
        I::Heart => ("\u{f004}", "♥", "<3"),
        I::Star => ("\u{f005}", "★", "*"),
        I::PushPin => ("\u{f08d}", "⚲", "!"),
        I::Tag | I::TagSimple => ("\u{f02b}", "⌗", "#"),
        I::Link | I::LinkSimple => ("\u{f0c1}", "↪", "@"),
        I::LinkBreak => ("\u{f127}", "↯", "@!"),
        // Time.
        I::Calendar => ("\u{f073}", "☷", "ca"),
        I::CalendarCheck => ("\u{f274}", "☑", "c+"),
        I::CalendarDots => ("\u{f133}", "☷", "c."),
        I::Clock => ("\u{f017}", "◷", "t"),
        I::ClockCounterClockwise => ("\u{f1da}", "↺", "h"),
        I::Hourglass => ("\u{f252}", "⧗", ".."),
        // Status.
        I::Info => ("\u{f05a}", "ⓘ", "i"),
        I::Question => ("\u{f059}", "?", "?"),
        I::WarningCircle => ("\u{f06a}", "⚠", "!"),
        I::SealCheck => ("\u{f0a3}", "✔", "ok"),
        I::ShieldCheck => ("\u{f132}", "✔", "ok"),
        I::LockSimple => ("\u{f023}", "⚿", "L"),
        I::Fingerprint => ("\u{f084}", "⌬", "id"),
        I::Lightning => ("\u{f0e7}", "ϟ", "!"),
        I::Fire => ("\u{f06d}", "♨", "^"),
        I::Pulse => ("\u{f21e}", "∿", "~"),
        I::Power => ("\u{f011}", "⏻", "O"),
        I::Eye => ("\u{f06e}", "◉", "o"),
        I::EyeSlash => ("\u{f070}", "◌", "-"),
        I::HandWaving => ("\u{f256}", "☺", "hi"),
        I::Robot => ("\u{f17b}", "⌬", "AI"),
        I::GearSix => ("\u{f013}", "⚙", "*"),
        // Code and git.
        I::Code => ("\u{f121}", "λ", "<>"),
        I::GitBranch | I::GitFork => ("\u{f126}", "⎇", "Y"),
        I::GitDiff => ("\u{f0ec}", "±", "+-"),
        // Close.
        I::X => ("\u{f00d}", "✕", "x"),
        I::XCircle => ("\u{f057}", "⊗", "x"),
        I::XSquare => ("\u{f2d3}", "⊠", "x"),
        I::PlusSquare => ("\u{f0fe}", "⊞", "+"),
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

/// The glyph of a file class in `set`.
pub fn class_glyph(class: FileClass, set: IconSet) -> &'static str {
    glyph(class_icon(class), set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::text::width;

    const SETS: [IconSet; 3] = [IconSet::Ascii, IconSet::Unicode, IconSet::Nerd];

    #[test]
    fn every_glyph_is_narrow_and_safe() {
        assert_eq!(IconId::ALL.len(), 165);
        for set in SETS {
            for &id in IconId::ALL {
                let g = glyph(id, set);
                assert!((1..=2).contains(&width(g)), "{id:?} {set:?}: {g:?}");
                assert!(!g.chars().any(char::is_control), "{id:?}");
            }
        }
        assert_eq!(glyph(class_icon(FileClass::Folder), IconSet::Ascii), "/");
    }

    #[test]
    fn sets_use_their_own_characters() {
        for &id in IconId::ALL {
            let n = glyph(id, IconSet::Nerd);
            assert_eq!(n.chars().count(), 1, "{id:?}: one Nerd glyph");
            assert!(n.chars().all(|c| ('\u{E000}'..='\u{F8FF}').contains(&c)), "{id:?}: {n:?} is not private use");
            assert!(glyph(id, IconSet::Ascii).is_ascii(), "{id:?}");
            let u = glyph(id, IconSet::Unicode);
            assert!(u.chars().all(|c| (c as u32) < 0x1_0000 && !('\u{E000}'..='\u{F8FF}').contains(&c)), "{id:?}");
        }
    }

    #[test]
    fn class_glyphs() {
        assert_eq!(class_glyph(FileClass::Folder, IconSet::Nerd), "\u{f07b}");
        assert_eq!(class_glyph(FileClass::Pdf, IconSet::Nerd), "\u{f1c1}");
        assert_eq!(class_glyph(FileClass::Other, IconSet::Unicode), "□");
        assert_eq!(IconSet::from_config(&TuiSection::default()), IconSet::Unicode);
        for (icons, set) in [(TuiIcons::Nerd, IconSet::Nerd), (TuiIcons::Ascii, IconSet::Ascii)] {
            assert_eq!(IconSet::from_config(&TuiSection { icons, ..TuiSection::default() }), set);
        }
    }
}
