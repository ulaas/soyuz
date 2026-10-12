//! Colour roles of the terminal UI.
//!
//! Views never name a colour: they ask the theme for the [`Style`] of a [`Role`]. Phase T1
//! drew in plain monochrome (bold / reversed only); [`TuiTheme::plain`] reproduces that look
//! exactly and stays the fallback for `NO_COLOR` and monochrome terminals. T2.2 adds the
//! coloured themes derived from the design tokens.

use ratatui::style::{Modifier, Style};
use soyuz_appcore::core::AppCore;
use soyuz_appcore::file_class::FileClass;
use soyuz_appcore::models::toasts::ToastKind;

/// What a piece of text or a cell is, for styling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// Ordinary text.
    Text,
    /// Secondary text (sizes, dates, counts, "this folder is empty").
    Muted,
    /// Barely there (separators, placeholders).
    Faint,
    /// Accent (links, matches, the active tab marker).
    Accent,
    /// The location line / titles.
    Title,
    /// Key hints (dialog footers, status hints).
    KeyHint,
    /// A selected row.
    Selected,
    /// The cursor row of the focused pane.
    Cursor,
    /// The cursor row of a pane without focus.
    CursorInactive,
    /// A group header in the list.
    GroupHeader,
    Border,
    BorderFocused,
    Success,
    Warning,
    Danger,
    Info,
    Folder,
    Image,
    Video,
    Audio,
    Archive,
    Code,
    Doc,
    Exe,
    /// File names of other classes.
    File,
    /// A tag colour (`0..` in `[tags]` order).
    Tag(u8),
    /// The status line.
    StatusBar,
    /// The active tab.
    TabActive,
    /// The other tabs.
    TabInactive,
    /// An input field (address, filter, palette).
    Input,
    /// The palette's selected row.
    PaletteSelected,
}

impl Role {
    /// The role of a file name of class `c`.
    pub fn of_class(c: FileClass) -> Role {
        match c {
            FileClass::Folder => Role::Folder,
            FileClass::Image => Role::Image,
            FileClass::Video => Role::Video,
            FileClass::Audio => Role::Audio,
            FileClass::Archive => Role::Archive,
            FileClass::Code => Role::Code,
            FileClass::Pdf | FileClass::Doc | FileClass::Sheet | FileClass::Slides | FileClass::Text => Role::Doc,
            FileClass::Exe => Role::Exe,
            FileClass::Font | FileClass::Link | FileClass::Other => Role::File,
        }
    }

    /// The role of a toast of kind `k`.
    pub fn of_toast(k: ToastKind) -> Role {
        match k {
            ToastKind::Info => Role::Info,
            ToastKind::Success => Role::Success,
            ToastKind::Warning => Role::Warning,
            ToastKind::Error => Role::Danger,
        }
    }
}

/// The styles one frame draws with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TuiTheme {
    // T2.2: the colour table (and the colour depth it was built for) goes here; a theme
    // without colours is the plain one.
}

impl TuiTheme {
    /// Monochrome: the look of Phase T1 (bold and reversed only, terminal colours).
    pub fn plain() -> TuiTheme {
        TuiTheme {}
    }

    /// The theme for the core's current settings. T2.2: colours from the design tokens,
    /// light / dark, `NO_COLOR`, colour depth.
    pub fn for_core(_core: &AppCore) -> TuiTheme {
        TuiTheme::plain()
    }

    /// The style of `role`.
    pub fn style(&self, role: Role) -> Style {
        plain(role)
    }
}

/// [`TuiTheme::plain`]'s styles.
fn plain(role: Role) -> Style {
    let s = Style::new();
    match role {
        Role::Title | Role::KeyHint | Role::Selected | Role::Warning | Role::Danger | Role::GroupHeader => {
            s.add_modifier(Modifier::BOLD)
        }
        Role::Cursor | Role::StatusBar | Role::TabActive | Role::PaletteSelected => s.add_modifier(Modifier::REVERSED),
        Role::CursorInactive => s.add_modifier(Modifier::UNDERLINED),
        Role::Text
        | Role::Muted
        | Role::Faint
        | Role::Accent
        | Role::Border
        | Role::BorderFocused
        | Role::Success
        | Role::Info
        | Role::Folder
        | Role::Image
        | Role::Video
        | Role::Audio
        | Role::Archive
        | Role::Code
        | Role::Doc
        | Role::Exe
        | Role::File
        | Role::Tag(_)
        | Role::TabInactive
        | Role::Input => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_is_monochrome() {
        let t = TuiTheme::plain();
        for r in [Role::Text, Role::Cursor, Role::Selected, Role::StatusBar, Role::Folder, Role::Tag(3)] {
            let s = t.style(r);
            assert_eq!((s.fg, s.bg), (None, None), "{r:?}");
        }
        assert!(t.style(Role::Cursor).add_modifier.contains(Modifier::REVERSED));
        assert!(t.style(Role::Selected).add_modifier.contains(Modifier::BOLD));
        assert_eq!(Role::of_class(FileClass::Pdf), Role::Doc);
        assert_eq!(Role::of_toast(ToastKind::Error), Role::Danger);
    }
}
