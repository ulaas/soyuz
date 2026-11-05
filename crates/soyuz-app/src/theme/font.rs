//! Text sizes of custom-painted surfaces. Every size in the UI code is written as it looks
//! at the default 13 pt body (`font::ui(12.5)`); here it follows `[appearance] font_size`
//! and the `[fonts.*] size_offset` keys, by the same rule as the egui text styles
//! (`apply::style` + `apply::offset_text`), so a label and a row of equal design size
//! always get the very same font. Called for every painted string: two relaxed atomic
//! loads and no allocation.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU32, Ordering};

use egui::{FontFamily, FontId};

use super::fonts;
use super::tokens::type_scale;

const ONE: u32 = 0x3F80_0000; // 1.0_f32

static K: AtomicU32 = AtomicU32::new(ONE);
static UI_OFFSET: AtomicU32 = AtomicU32::new(0);
static SEMIBOLD_OFFSET: AtomicU32 = AtomicU32::new(0);
static CODE_OFFSET: AtomicU32 = AtomicU32::new(0);
static NUMBERS_OFFSET: AtomicU32 = AtomicU32::new(0);

static SEMIBOLD: LazyLock<FontFamily> = LazyLock::new(|| FontFamily::Name(fonts::SEMIBOLD.into()));
static NUMBERS: LazyLock<FontFamily> = LazyLock::new(|| FontFamily::Name(fonts::NUMBERS.into()));

/// The factor of `[appearance] font_size` over the default body size.
pub fn factor(font_size: f32) -> f32 {
    if font_size > 4.0 { font_size / type_scale::BODY } else { 1.0 }
}

/// Sizes land on quarter points: equal text sizes share one font in the atlas, and every
/// size written in the code stays exactly what it says at the default body size.
pub fn snap(size: f32) -> f32 {
    (size * 4.0).round() / 4.0
}

/// `size` under a factor and a role offset, as the text styles compute it.
fn resolve(size: f32, k: f32, offset: f32) -> f32 {
    snap(snap(size * k) + offset).max(4.0)
}

fn load(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}

/// Remember `[appearance] font_size` and the `[fonts.*] size_offset` values (from
/// `theme::install`, at startup and on every config change).
pub fn configure(font_size: f32, ui: f32, semibold: f32, code: f32, numbers: f32) {
    K.store(factor(font_size).to_bits(), Ordering::Relaxed);
    UI_OFFSET.store(ui.to_bits(), Ordering::Relaxed);
    SEMIBOLD_OFFSET.store(semibold.to_bits(), Ordering::Relaxed);
    CODE_OFFSET.store(code.to_bits(), Ordering::Relaxed);
    NUMBERS_OFFSET.store(numbers.to_bits(), Ordering::Relaxed);
}

/// The interface size for design size `size` (for `RichText::size`).
pub fn px(size: f32) -> f32 {
    resolve(size, load(&K), load(&UI_OFFSET))
}

/// The interface font.
pub fn ui(size: f32) -> FontId {
    FontId::proportional(px(size))
}

/// The heading weight (`[fonts.ui_semibold]`): names of the current folder and tab, titles.
pub fn semibold(size: f32) -> FontId {
    FontId::new(resolve(size, load(&K), load(&SEMIBOLD_OFFSET)), SEMIBOLD.clone())
}

/// A semibold heading; from 17 pt the display family ([`fonts::heading`]).
pub fn heading(size: f32) -> FontId {
    let id = fonts::heading(size);
    FontId::new(resolve(size, load(&K), load(&SEMIBOLD_OFFSET)), id.family)
}

/// Monospace text outside the terminal and hex grids (those have their own size keys).
pub fn mono(size: f32) -> FontId {
    FontId::monospace(resolve(size, load(&K), load(&CODE_OFFSET)))
}

/// Sizes, dates, counters and progress figures (`[fonts.numbers]`).
pub fn numbers(size: f32) -> FontId {
    FontId::new(resolve(size, load(&K), load(&NUMBERS_OFFSET)), NUMBERS.clone())
}

/// A fixed height that holds a line of `size` design points: `min` while the text fits,
/// so nothing moves at the default size, and more once the text outgrows it.
pub fn height(min: f32, size: f32) -> f32 {
    min.max((px(size) * 1.5).ceil())
}

/// How much wider text is than at the default size, never below 1: a target keeps its size
/// with smaller text.
pub fn growth() -> f32 {
    (px(type_scale::BODY) / type_scale::BODY).max(1.0)
}

/// A fixed size laid out around text (two-line rows, label columns, side panels): grows
/// with the text.
pub fn grow(v: f32) -> f32 {
    (v * growth()).round()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_is_the_rule_of_the_text_styles() {
        for (font_size, off) in [(13.0, 0.0), (11.0, 0.0), (17.0, 0.0), (15.0, 2.0), (11.0, -4.0), (17.0, 8.0)] {
            let k = factor(font_size);
            let mut s = crate::theme::apply::style(&crate::theme::ORBIT_DARK, font_size, false);
            crate::theme::apply::offset_text(&mut s, off, off, off);
            let body = s.text_styles[&egui::TextStyle::Body].size;
            assert_eq!(body, resolve(type_scale::BODY, k, off), "body at {font_size} {off:+}");
            let small = s.text_styles[&egui::TextStyle::Small].size;
            assert_eq!(small, resolve(type_scale::SMALL, k, off), "small at {font_size} {off:+}");
            let title = s.text_styles[&egui::TextStyle::Name(crate::theme::apply::TITLE.into())].size;
            assert_eq!(title, resolve(type_scale::TITLE, k, off), "title at {font_size} {off:+}");
            let mono = s.text_styles[&egui::TextStyle::Monospace].size;
            assert_eq!(mono, resolve(type_scale::MONOSPACE, k, off), "mono at {font_size} {off:+}");
        }
    }

    #[test]
    fn the_default_size_changes_nothing() {
        for size in [9.5, 10.0, 10.5, 11.0, 11.5, 12.0, 12.5, 13.0, 13.5, 14.0, 14.5, 15.0, 16.5, 18.0, 19.0, 22.0] {
            assert_eq!(resolve(size, 1.0, 0.0), size);
        }
        assert_eq!(factor(13.0), 1.0);
        assert_eq!(factor(0.0), 1.0, "a nonsense size is ignored");
    }

    #[test]
    fn sizes_follow_and_stay_readable() {
        assert_eq!(resolve(13.0, factor(17.0), 0.0), 17.0);
        assert_eq!(resolve(13.0, factor(11.0), 0.0), 11.0);
        assert_eq!(resolve(13.0, 1.0, 2.0), 15.0);
        assert_eq!(resolve(4.0, 1.0, -4.0), 4.0, "never below 4 pt");
        let r = resolve(11.5, factor(14.0), 0.0);
        assert_eq!(r, snap(r), "on the quarter-point grid");
    }
}
