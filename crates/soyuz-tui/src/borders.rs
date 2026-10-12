//! Box borders (`[tui] borders`): pane blocks, the palette, dialogs and toasts all get
//! their [`Block`] from here, so one setting changes every box.
//!
//! | [`TuiBorders`] | Lines |
//! |---|---|
//! | `Rounded` (default) | `╭─╮ │ ╰─╯` |
//! | `Plain` | `┌─┐ │ └─┘` |
//! | `Ascii` | `+-+ \| +-+` (terminals and fonts without box drawing) |
//! | `None` | no lines: [`block`] has no border at all, [`overlay_block`] keeps a blank frame |
//!
//! Views call [`block`] for boxes that sit in the layout (pane blocks) and
//! [`overlay_block`] for boxes drawn over other content (palette, dialogs, toasts), which
//! need a frame to stand apart even without lines. Lines that are not boxes (the splitters
//! and pane titles of dual / quad layouts, the Miller column separators, the palette's
//! separator) take their characters from [`lines`], so the setting reaches them too.
//!
//! The setting travels with the theme (`TuiTheme::borders`, rebuilt on `[tui]` changes).
//! With `icons = "ascii"` and `borders = "ascii"` / `"none"` the screen is pure ASCII:
//! [`asciify`] replaces the typographic punctuation of labels and translations.

use ratatui::buffer::Buffer;
use ratatui::style::Style;
use ratatui::symbols::border;
use ratatui::widgets::{Block, Borders};
use soyuz_appcore::core::AppCore;
pub use soyuz_core::config::TuiBorders;

/// The border style the core's settings ask for (`[tui] borders`).
pub fn from_core(core: &AppCore) -> TuiBorders {
    core.config.tui.borders
}

/// `+-|` boxes.
pub const ASCII: border::Set<'static> = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

/// The line set of `b` (`None` for no lines).
pub fn border_set(b: TuiBorders) -> Option<border::Set<'static>> {
    match b {
        TuiBorders::Rounded => Some(border::ROUNDED),
        TuiBorders::Plain => Some(border::PLAIN),
        TuiBorders::Ascii => Some(ASCII),
        TuiBorders::None => None,
    }
}

/// The characters of lines that are not boxes (one cell each).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lines {
    pub horizontal: &'static str,
    pub vertical: &'static str,
    /// Where a horizontal and a vertical line cross.
    pub cross: &'static str,
    /// A horizontal line meeting a box's left / right side (`├` / `┤`).
    pub tee_left: &'static str,
    pub tee_right: &'static str,
}

/// The line characters of `b`: box drawing, `-` `|` `+` in ASCII, blanks with no lines.
pub fn lines(b: TuiBorders) -> Lines {
    match b {
        TuiBorders::Rounded | TuiBorders::Plain => {
            Lines { horizontal: "─", vertical: "│", cross: "┼", tee_left: "├", tee_right: "┤" }
        }
        TuiBorders::Ascii => Lines { horizontal: "-", vertical: "|", cross: "+", tee_left: "+", tee_right: "+" },
        TuiBorders::None => Lines { horizontal: " ", vertical: " ", cross: " ", tee_left: " ", tee_right: " " },
    }
}

/// The ASCII look-alike of a typographic character the views and translations use (same
/// width: one cell).
fn ascii_of(s: &str) -> Option<&'static str> {
    Some(match s {
        "…" => "~",
        "·" | "•" | "∙" => "*",
        "×" | "✕" => "x",
        "‹" | "«" | "←" | "◂" => "<",
        "›" | "»" | "→" | "▸" => ">",
        "↑" | "▴" => "^",
        "↓" | "▾" => "v",
        "—" | "–" | "─" | "━" => "-",
        "│" | "┃" => "|",
        "┼" | "├" | "┤" | "┬" | "┴" | "╭" | "╮" | "╰" | "╯" | "┌" | "┐" | "└" | "┘" => "+",
        "“" | "”" | "„" => "\"",
        "‘" | "’" => "'",
        "\u{a0}" => " ",
        _ => return None,
    })
}

/// Replace the typographic characters of `buf` with ASCII look-alikes (see the module
/// docs). Anything else (names in other scripts) is left as it is.
pub fn asciify(buf: &mut Buffer) {
    for cell in buf.content.iter_mut() {
        if !cell.symbol().is_ascii()
            && let Some(a) = ascii_of(cell.symbol())
        {
            cell.set_symbol(a);
        }
    }
}

/// Cells a border takes on each side ([`block`]).
pub fn thickness(b: TuiBorders) -> u16 {
    u16::from(border_set(b).is_some())
}

/// A box in the layout (pane blocks): all four sides in `style`, or no border at all with
/// [`TuiBorders::None`].
pub fn block<'a>(b: TuiBorders, style: Style) -> Block<'a> {
    match border_set(b) {
        Some(set) => Block::new().borders(Borders::ALL).border_set(set).border_style(style),
        None => Block::new(),
    }
}

/// A box drawn over other content (palette, dialogs, toasts): like [`block`], but with
/// [`TuiBorders::None`] a blank one-cell frame keeps it apart from what is under it (the
/// inner area is the same for every setting).
pub fn overlay_block<'a>(b: TuiBorders, style: Style) -> Block<'a> {
    let set = border_set(b).unwrap_or(border::EMPTY);
    Block::new().borders(Borders::ALL).border_set(set).border_style(style)
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::widgets::Paragraph;

    use super::*;

    fn boxed(b: TuiBorders, overlay: bool) -> Vec<String> {
        let mut t = Terminal::new(TestBackend::new(5, 3)).unwrap();
        t.draw(|f| {
            let blk = if overlay { overlay_block(b, Style::new()) } else { block(b, Style::new()) };
            f.render_widget(Paragraph::new("x").block(blk), Rect::new(0, 0, 5, 3));
        })
        .unwrap();
        let buf = t.backend().buffer();
        (0..3).map(|y| (0..5).map(|x| buf[(x, y)].symbol().to_string()).collect()).collect()
    }

    #[test]
    fn every_border_set() {
        assert_eq!(boxed(TuiBorders::Rounded, false), ["╭───╮", "│x  │", "╰───╯"]);
        assert_eq!(boxed(TuiBorders::Plain, false), ["┌───┐", "│x  │", "└───┘"]);
        assert_eq!(boxed(TuiBorders::Ascii, false), ["+---+", "|x  |", "+---+"]);
        assert_eq!(boxed(TuiBorders::None, false), ["x    ", "     ", "     "]);
        // Overlays keep their inner area without lines.
        assert_eq!(boxed(TuiBorders::None, true), ["     ", " x   ", "     "]);
        assert_eq!(boxed(TuiBorders::Ascii, true), boxed(TuiBorders::Ascii, false));
        assert_eq!((thickness(TuiBorders::Plain), thickness(TuiBorders::None)), (1, 0));
        assert_eq!(TuiBorders::default(), TuiBorders::Rounded);
    }

    #[test]
    fn line_sets_and_asciify() {
        assert_eq!(lines(TuiBorders::Rounded).vertical, "│");
        assert_eq!(lines(TuiBorders::Ascii).cross, "+");
        assert_eq!(lines(TuiBorders::None).horizontal, " ");
        for b in [TuiBorders::Rounded, TuiBorders::Plain, TuiBorders::Ascii, TuiBorders::None] {
            let l = lines(b);
            for s in [l.horizontal, l.vertical, l.cross, l.tee_left, l.tee_right] {
                assert_eq!(unicode_width::UnicodeWidthStr::width(s), 1, "{b:?}");
            }
        }
        let mut buf = Buffer::with_lines(["a…b · c → 日"]);
        asciify(&mut buf);
        let s: String = buf.content.iter().map(|c| c.symbol()).collect();
        assert_eq!(s, "a~b * c > 日 ");
    }
}
