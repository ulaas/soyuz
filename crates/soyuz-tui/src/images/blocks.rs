//! Half-block images: each cell shows two image pixels stacked (`▀` / `▄`, foreground
//! and background colour), converted for the terminal's [`ColorMode`] when drawn, so a
//! colour-mode change needs no new encoding. Transparent pixels leave the cell's
//! background (translucent ones are blended over it when it is an RGB colour).

use image::RgbaImage;
use image::imageops::FilterType;
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::Color;
use soyuz_appcore::color::Rgba8;

use crate::theme::{ColorMode, nearest_256};

/// Scale `img` to `w` × `h` half-block pixels (`h` is twice the cell rows, or one less).
pub fn encode(img: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    let (w, h) = (w.max(1), h.max(1));
    if img.dimensions() == (w, h) {
        return img.clone();
    }
    image::imageops::resize(img, w, h, FilterType::Triangle)
}

/// The 16 ANSI colours with the xterm default values (what most schemes are close to).
const ANSI16: [(Color, [u8; 3]); 16] = [
    (Color::Black, [0, 0, 0]),
    (Color::Red, [205, 0, 0]),
    (Color::Green, [0, 205, 0]),
    (Color::Yellow, [205, 205, 0]),
    (Color::Blue, [0, 0, 238]),
    (Color::Magenta, [205, 0, 205]),
    (Color::Cyan, [0, 205, 205]),
    (Color::Gray, [229, 229, 229]),
    (Color::DarkGray, [127, 127, 127]),
    (Color::LightRed, [255, 0, 0]),
    (Color::LightGreen, [0, 255, 0]),
    (Color::LightYellow, [255, 255, 0]),
    (Color::LightBlue, [92, 92, 255]),
    (Color::LightMagenta, [255, 0, 255]),
    (Color::LightCyan, [0, 255, 255]),
    (Color::White, [255, 255, 255]),
];

/// The nearest of the 16 ANSI colours (weighted RGB distance).
pub fn nearest_16(c: [u8; 3]) -> Color {
    let d = |a: [u8; 3]| {
        let e = |i: usize, w: i32| w * (i32::from(a[i]) - i32::from(c[i])).pow(2);
        e(0, 3) + e(1, 4) + e(2, 2)
    };
    ANSI16.iter().min_by_key(|(_, rgb)| d(*rgb)).map_or(Color::Reset, |(col, _)| *col)
}

/// An RGB colour for `mode` (`None` for [`ColorMode::Mono`]).
pub fn map(c: [u8; 3], mode: ColorMode) -> Option<Color> {
    match mode {
        ColorMode::Truecolor => Some(Color::Rgb(c[0], c[1], c[2])),
        ColorMode::Ansi256 => Some(Color::Indexed(nearest_256(Rgba8 { r: c[0], g: c[1], b: c[2], a: 255 }))),
        ColorMode::Ansi16 => Some(nearest_16(c)),
        ColorMode::Mono => None,
    }
}

/// One pixel over the cell's background: `None` when it stays transparent.
fn over(p: [u8; 4], bg: Option<Color>) -> Option<[u8; 3]> {
    match p[3] {
        0 => None,
        255 => Some([p[0], p[1], p[2]]),
        a => match bg {
            Some(Color::Rgb(r, g, b)) => {
                let a = u32::from(a);
                let mix = |f: u8, b: u8| ((u32::from(f) * a + u32::from(b) * (255 - a) + 127) / 255) as u8;
                Some([mix(p[0], r), mix(p[1], g), mix(p[2], b)])
            }
            _ if a >= 128 => Some([p[0], p[1], p[2]]),
            _ => None,
        },
    }
}

/// Draw the half-block pixels `px` at the top left of `rect` (clipped to it).
pub fn render(px: &RgbaImage, rect: Rect, buf: &mut Buffer, mode: ColorMode) {
    let (w, h) = px.dimensions();
    let rect = rect.intersection(buf.area);
    for cy in 0..u32::from(rect.height) {
        let top_y = cy * 2;
        if top_y >= h {
            break;
        }
        for cx in 0..u32::from(rect.width).min(w) {
            let pos = (rect.x + cx as u16, rect.y + cy as u16);
            let Some(cell) = buf.cell_mut(pos) else { continue };
            let bg = cell.bg;
            let top = over(px.get_pixel(cx, top_y).0, Some(bg));
            let bottom = if top_y + 1 < h { over(px.get_pixel(cx, top_y + 1).0, Some(bg)) } else { None };
            let top = top.and_then(|c| map(c, mode));
            let bottom = bottom.and_then(|c| map(c, mode));
            match (top, bottom) {
                (None, None) => continue,
                (Some(t), Some(b)) if t == b => {
                    cell.set_char('█').set_fg(t);
                }
                (Some(t), Some(b)) => {
                    cell.set_char('▀').set_fg(t).set_bg(b);
                }
                (Some(t), None) => {
                    cell.set_char('▀').set_fg(t);
                }
                (None, Some(b)) => {
                    cell.set_char('▄').set_fg(b);
                }
            }
            cell.set_diff_option(CellDiffOption::None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_follow_the_mode() {
        assert_eq!(map([1, 2, 3], ColorMode::Truecolor), Some(Color::Rgb(1, 2, 3)));
        assert_eq!(map([255, 0, 0], ColorMode::Ansi256), Some(Color::Indexed(196)));
        assert_eq!(map([250, 10, 10], ColorMode::Ansi16), Some(Color::LightRed));
        assert_eq!(map([10, 10, 10], ColorMode::Ansi16), Some(Color::Black));
        assert_eq!(map([250, 250, 250], ColorMode::Ansi16), Some(Color::White));
        assert_eq!(map([0, 0, 230], ColorMode::Ansi16), Some(Color::Blue));
        assert_eq!(map([1, 2, 3], ColorMode::Mono), None);
    }

    #[test]
    fn alpha_blends_over_rgb_backgrounds() {
        assert_eq!(over([200, 0, 0, 0], None), None);
        assert_eq!(over([200, 0, 0, 255], None), Some([200, 0, 0]));
        assert_eq!(over([200, 0, 0, 128], Some(Color::Rgb(0, 0, 100))), Some([100, 0, 50]));
        assert_eq!(over([200, 0, 0, 200], Some(Color::Reset)), Some([200, 0, 0]));
        assert_eq!(over([200, 0, 0, 20], Some(Color::Reset)), None);
    }

    #[test]
    fn half_blocks_pick_the_right_character() {
        // 3 x 3 pixels: columns = opaque/opaque, opaque/transparent, transparent/opaque
        // over two rows; the third row is the top of the second cell row.
        let mut img = RgbaImage::new(3, 3);
        let red = image::Rgba([255, 0, 0, 255]);
        let blue = image::Rgba([0, 0, 255, 255]);
        img.put_pixel(0, 0, red);
        img.put_pixel(0, 1, blue);
        img.put_pixel(1, 0, red);
        img.put_pixel(2, 1, blue);
        img.put_pixel(0, 2, red);
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 3));
        render(&img, Rect::new(0, 0, 4, 3), &mut buf, ColorMode::Truecolor);
        let c = |x, y| buf.cell((x, y)).unwrap().clone();
        assert_eq!((c(0, 0).symbol(), c(0, 0).fg, c(0, 0).bg), ("▀", Color::Rgb(255, 0, 0), Color::Rgb(0, 0, 255)));
        assert_eq!((c(1, 0).symbol(), c(1, 0).fg, c(1, 0).bg), ("▀", Color::Rgb(255, 0, 0), Color::Reset));
        assert_eq!((c(2, 0).symbol(), c(2, 0).fg), ("▄", Color::Rgb(0, 0, 255)));
        assert_eq!(c(3, 0).symbol(), " ", "outside the image");
        assert_eq!(c(0, 1).symbol(), "▀");
        assert_eq!(c(1, 1).symbol(), " ", "transparent");
        assert_eq!(c(0, 2).symbol(), " ", "below the image");
        // Same colour twice: a full block.
        let mut one = RgbaImage::new(1, 2);
        one.put_pixel(0, 0, red);
        one.put_pixel(0, 1, red);
        let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
        render(&one, Rect::new(0, 0, 1, 1), &mut buf, ColorMode::Ansi16);
        assert_eq!((buf[(0, 0)].symbol(), buf[(0, 0)].fg), ("█", Color::LightRed));
    }

    #[test]
    fn encode_scales() {
        let img = RgbaImage::new(40, 20);
        assert_eq!(encode(&img, 10, 5).dimensions(), (10, 5));
        assert_eq!(encode(&img, 0, 0).dimensions(), (1, 1));
    }
}
