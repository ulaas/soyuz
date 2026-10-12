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
//! need a frame to stand apart even without lines.

use ratatui::style::Style;
use ratatui::symbols::border;
use ratatui::widgets::{Block, Borders};
use soyuz_appcore::core::AppCore;

/// The `[tui] borders` setting.
///
/// Integration: `soyuz_core::config::TuiBorders` (same variants) is added in parallel;
/// replace this enum with `pub use soyuz_core::config::TuiBorders;` and switch
/// [`from_core`] to `core.config.tui.borders`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TuiBorders {
    #[default]
    Rounded,
    Plain,
    Ascii,
    None,
}

/// The border style the core's settings ask for (the one switch point, see
/// [`TuiBorders`]).
pub fn from_core(_core: &AppCore) -> TuiBorders {
    TuiBorders::default()
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
}
