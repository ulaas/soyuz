//! Images in the terminal (`[tui] images`, `[tui] cell_pixels`) through `ratatui-image`:
//! kitty, sixel, iTerm2 or half-blocks — Phase T4 (previews, thumbnails, grid cells).
//!
//! Stub: maps the settings to what the picker needs. T4 creates the
//! `ratatui_image::picker::Picker` (querying the terminal for `auto` before the input
//! reader starts, like the keyboard protocol query in `app::run`), keeps it on the app
//! and draws images with it.

use ratatui_image::picker::ProtocolType;
use soyuz_core::config::{TuiImages, TuiSection};

/// How images are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageMode {
    /// No images.
    Off,
    /// Ask the terminal (half-blocks when it does not answer).
    Auto,
    /// This protocol, whatever the terminal says.
    Forced(ProtocolType),
}

/// The mode `[tui] images` asks for.
pub fn mode(tui: &TuiSection) -> ImageMode {
    match tui.images {
        TuiImages::Off => ImageMode::Off,
        TuiImages::Auto => ImageMode::Auto,
        TuiImages::Kitty => ImageMode::Forced(ProtocolType::Kitty),
        TuiImages::Sixel => ImageMode::Forced(ProtocolType::Sixel),
        TuiImages::Iterm2 => ImageMode::Forced(ProtocolType::Iterm2),
        TuiImages::Halfblocks => ImageMode::Forced(ProtocolType::Halfblocks),
    }
}

/// The cell size in pixels when the terminal does not report it (`[tui] cell_pixels`,
/// clamped).
pub fn cell_pixels(tui: &TuiSection) -> (u16, u16) {
    tui.cell_pixels_clamped()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_map_to_modes() {
        let t = |images| TuiSection { images, ..TuiSection::default() };
        assert_eq!(mode(&TuiSection::default()), ImageMode::Auto);
        assert_eq!(mode(&t(TuiImages::Off)), ImageMode::Off);
        assert_eq!(mode(&t(TuiImages::Sixel)), ImageMode::Forced(ProtocolType::Sixel));
        assert_eq!(mode(&t(TuiImages::Halfblocks)), ImageMode::Forced(ProtocolType::Halfblocks));
        assert_eq!(cell_pixels(&TuiSection { cell_pixels: [0, 999], ..TuiSection::default() }), (1, 200));
    }
}
