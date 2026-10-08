//! Folder thumbnails (roadmap B3): a 2×2 mosaic of the first pictures inside a folder.
//! Runs on the thumbnail workers; only the first [`SCAN_LIMIT`] directory entries are
//! looked at so huge folders stay cheap.

use std::path::{Path, PathBuf};

use egui::ColorImage;
use image::{RgbaImage, imageops};

/// Directory entries examined per folder.
pub const SCAN_LIMIT: usize = 400;
/// Gap between tiles (pixels, at the mosaic's size).
const GAP_DIV: u32 = 32;

/// Up to four picture files of `dir`, by natural name order.
pub fn pick_images(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found: Vec<PathBuf> = rd
        .take(SCAN_LIMIT)
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| super::thumbs::is_thumbnailable(p))
        .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
        .collect();
    found.sort_by(|a, b| {
        let n = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        soyuz_core::naming::natural_cmp(&n(a), &n(b), true)
    });
    found.truncate(4);
    found
}

/// Tile rectangles `(x, y, w, h)` for `n` pictures in a `px` square: one fills it, two
/// split it into columns, three put one tall picture left of two, four make a 2×2 grid.
pub fn cells(n: usize, px: u32) -> Vec<(u32, u32, u32, u32)> {
    let gap = (px / GAP_DIV).max(1);
    let half = (px - gap) / 2;
    let far = half + gap;
    let rest = px - far;
    match n {
        0 => Vec::new(),
        1 => vec![(0, 0, px, px)],
        2 => vec![(0, 0, half, px), (far, 0, rest, px)],
        3 => vec![(0, 0, half, px), (far, 0, rest, half), (far, far, rest, rest)],
        _ => vec![(0, 0, half, half), (far, 0, rest, half), (0, far, half, rest), (far, far, rest, rest)],
    }
}

/// Compose a square `px`×`px` mosaic (see [`cells`]); every picture is center-cropped
/// to its tile. `None` for no pictures.
pub fn compose(tiles: &[RgbaImage], px: u32) -> Option<RgbaImage> {
    if tiles.is_empty() || px < 8 {
        return None;
    }
    let mut out = RgbaImage::new(px, px);
    for (t, (x, y, w, h)) in tiles.iter().zip(cells(tiles.len().min(4), px)) {
        let t = fill(t, w, h);
        imageops::overlay(&mut out, &t, i64::from(x), i64::from(y));
    }
    Some(out)
}

/// Center-crop to the `w`:`h` aspect and resize to `w`×`h`.
fn fill(img: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    let (iw, ih) = img.dimensions();
    let (iw, ih) = (iw.max(1), ih.max(1));
    // Largest crop with the target aspect.
    let (cw, ch) = if u64::from(iw) * u64::from(h) > u64::from(ih) * u64::from(w) {
        ((u64::from(ih) * u64::from(w) / u64::from(h.max(1))).max(1) as u32, ih)
    } else {
        (iw, (u64::from(iw) * u64::from(h) / u64::from(w.max(1))).max(1) as u32)
    };
    let (cw, ch) = (cw.min(iw), ch.min(ih));
    let cropped = imageops::crop_imm(img, (iw - cw) / 2, (ih - ch) / 2, cw, ch).to_image();
    imageops::resize(&cropped, w.max(1), h.max(1), imageops::FilterType::Triangle)
}

/// Decode the first pictures of `dir` and compose the mosaic (worker threads only).
pub fn mosaic(dir: &Path, px: u32, decode: impl Fn(&Path, u32) -> Option<ColorImage>) -> Option<ColorImage> {
    let picks = pick_images(dir);
    let tile_px = if picks.len() == 1 { px } else { px / 2 };
    let tiles: Vec<RgbaImage> = picks
        .iter()
        .filter_map(|p| decode(p, tile_px.max(32)))
        .filter_map(|c| {
            let raw: Vec<u8> = c.pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect();
            RgbaImage::from_raw(c.size[0] as u32, c.size[1] as u32, raw)
        })
        .collect();
    let out = compose(&tiles, px)?;
    let size = [out.width() as usize, out.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, out.as_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba(c))
    }

    #[test]
    fn layouts() {
        assert!(compose(&[], 64).is_none());
        let one = compose(&[solid(300, 100, [255, 0, 0, 255])], 64).unwrap();
        assert_eq!(one.dimensions(), (64, 64));
        assert_eq!(one.get_pixel(63, 63).0, [255, 0, 0, 255], "a single picture fills the square");
        let tiles = [solid(50, 50, [255, 0, 0, 255]), solid(80, 40, [0, 255, 0, 255]), solid(40, 80, [0, 0, 255, 255])];
        let m = compose(&tiles, 64).unwrap();
        assert_eq!(m.get_pixel(5, 5).0, [255, 0, 0, 255]);
        assert_eq!(m.get_pixel(5, 60).0, [255, 0, 0, 255], "three pictures: the first is tall");
        assert_eq!(m.get_pixel(60, 5).0, [0, 255, 0, 255]);
        assert_eq!(m.get_pixel(60, 60).0, [0, 0, 255, 255]);
        assert_eq!(m.get_pixel(32, 5).0[3], 0, "gap between tiles");
        let four = compose(&[tiles[0].clone(), tiles[1].clone(), tiles[2].clone(), tiles[0].clone()], 64).unwrap();
        assert_eq!(four.get_pixel(60, 60).0, [255, 0, 0, 255]);
        for n in 1..=4 {
            let c = cells(n, 100);
            assert_eq!(c.len(), n);
            assert!(c.iter().all(|(x, y, w, h)| x + w <= 100 && y + h <= 100 && *w > 0 && *h > 0));
        }
    }

    #[test]
    fn picks_first_pictures_by_name() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["b10.png", "b2.png", "a.jpg", "notes.txt", ".hidden.png", "c.gif", "d.webp"] {
            std::fs::write(dir.path().join(n), b"x").unwrap();
        }
        std::fs::create_dir(dir.path().join("sub.png")).unwrap();
        let names: Vec<String> =
            pick_images(dir.path()).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["a.jpg", "b2.png", "b10.png", "c.gif"]);
    }

    #[test]
    fn mosaic_from_real_files() {
        let dir = tempfile::tempdir().unwrap();
        solid(40, 40, [9, 9, 9, 255]).save(dir.path().join("a.png")).unwrap();
        let m = mosaic(dir.path(), 64, |p, px| {
            let img = image::open(p).ok()?.thumbnail(px, px).to_rgba8();
            Some(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
        })
        .unwrap();
        assert_eq!(m.size, [64, 64]);
        assert!(mosaic(&dir.path().join("missing"), 64, |_, _| None).is_none());
    }
}
