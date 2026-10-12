//! Image edit operations (pure): rotate, flip, crop and resize, replayed on the display
//! copy and on the full-resolution image at save time, plus the save-format policy.

use image::DynamicImage;

/// A crop rectangle in normalized coordinates (0..1) of the image *as currently shown*
/// (after the previous operations).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NormRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl NormRect {
    /// Ordered and clamped to 0..1.
    pub fn normalized(self) -> Self {
        let c = |v: f32| v.clamp(0.0, 1.0);
        NormRect {
            x0: c(self.x0.min(self.x1)),
            y0: c(self.y0.min(self.y1)),
            x1: c(self.x0.max(self.x1)),
            y1: c(self.y0.max(self.y1)),
        }
    }

    /// Pixel rectangle `(x, y, w, h)` in a `w × h` image (at least 1 × 1).
    pub fn to_pixels(self, w: u32, h: u32) -> (u32, u32, u32, u32) {
        let r = self.normalized();
        let x = ((r.x0 * w as f32).round() as u32).min(w.saturating_sub(1));
        let y = ((r.y0 * h as f32).round() as u32).min(h.saturating_sub(1));
        let x1 = ((r.x1 * w as f32).round() as u32).clamp(x + 1, w.max(x + 1));
        let y1 = ((r.y1 * h as f32).round() as u32).clamp(y + 1, h.max(y + 1));
        (x, y, x1 - x, y1 - y)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImgOp {
    RotateCw,
    RotateCcw,
    FlipH,
    FlipV,
    Crop(NormRect),
    /// Scale to exactly `w × h` pixels (the terminal UI's resize bar).
    Resize {
        w: u32,
        h: u32,
    },
}

impl ImgOp {
    pub fn apply(&self, img: &DynamicImage) -> DynamicImage {
        match self {
            ImgOp::RotateCw => img.rotate90(),
            ImgOp::RotateCcw => img.rotate270(),
            ImgOp::FlipH => img.fliph(),
            ImgOp::FlipV => img.flipv(),
            ImgOp::Crop(r) => {
                let (x, y, w, h) = r.to_pixels(img.width(), img.height());
                img.crop_imm(x, y, w, h)
            }
            ImgOp::Resize { w, h } => img.resize_exact((*w).max(1), (*h).max(1), image::imageops::FilterType::Lanczos3),
        }
    }
}

/// Replay `ops` on `img`.
pub fn replay(img: &DynamicImage, ops: &[ImgOp]) -> DynamicImage {
    let mut cur = img.clone();
    for op in ops {
        cur = op.apply(&cur);
    }
    cur
}

/// Size after `ops` without touching pixels (for the status line).
pub fn size_after(mut w: u32, mut h: u32, ops: &[ImgOp]) -> (u32, u32) {
    for op in ops {
        match op {
            ImgOp::RotateCw | ImgOp::RotateCcw => std::mem::swap(&mut w, &mut h),
            ImgOp::FlipH | ImgOp::FlipV => {}
            ImgOp::Crop(r) => {
                let (_, _, cw, ch) = r.to_pixels(w, h);
                (w, h) = (cw, ch);
            }
            ImgOp::Resize { w: rw, h: rh } => (w, h) = ((*rw).max(1), (*rh).max(1)),
        }
    }
    (w, h)
}

/// How an image file can be written back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveMode {
    /// Re-encoded without loss (PNG, BMP, TIFF, lossless WebP, ICO, GIF palette).
    Lossless(image::ImageFormat),
    /// JPEG: no pure-Rust lossless transform is available, so it is re-encoded.
    Lossy(image::ImageFormat),
    /// No encoder for this format.
    Unsupported,
}

pub fn save_mode(format: Option<image::ImageFormat>) -> SaveMode {
    use image::ImageFormat as F;
    match format {
        Some(f @ (F::Png | F::Bmp | F::Tiff | F::WebP | F::Ico | F::Gif)) => SaveMode::Lossless(f),
        Some(F::Jpeg) => SaveMode::Lossy(F::Jpeg),
        _ => SaveMode::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn sample() -> DynamicImage {
        // 3 × 2: red green blue / white black gray.
        let mut im = RgbaImage::new(3, 2);
        let px = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255], [0, 0, 0], [128, 128, 128]];
        for (i, c) in px.iter().enumerate() {
            im.put_pixel(i as u32 % 3, i as u32 / 3, Rgba([c[0], c[1], c[2], 255]));
        }
        DynamicImage::ImageRgba8(im)
    }

    fn at(img: &DynamicImage, x: u32, y: u32) -> [u8; 3] {
        let p = img.to_rgba8().get_pixel(x, y).0;
        [p[0], p[1], p[2]]
    }

    #[test]
    fn rotate_flip_crop() {
        let img = sample();
        let r = ImgOp::RotateCw.apply(&img);
        assert_eq!((r.width(), r.height()), (2, 3));
        assert_eq!(at(&r, 1, 0), [255, 0, 0], "top-left moves to top-right");
        let back = replay(&img, &[ImgOp::RotateCw, ImgOp::RotateCcw]);
        assert_eq!(back.to_rgba8(), img.to_rgba8());
        let f = ImgOp::FlipH.apply(&img);
        assert_eq!(at(&f, 0, 0), [0, 0, 255]);
        let v = ImgOp::FlipV.apply(&img);
        assert_eq!(at(&v, 0, 0), [255, 255, 255]);
        let c = ImgOp::Crop(NormRect { x0: 0.34, y0: 0.0, x1: 1.0, y1: 0.5 }).apply(&img);
        assert_eq!((c.width(), c.height()), (2, 1));
        assert_eq!(at(&c, 0, 0), [0, 255, 0]);
        let ops = [ImgOp::RotateCw, ImgOp::Crop(NormRect { x0: 0.0, y0: 0.0, x1: 0.5, y1: 1.0 })];
        assert_eq!(size_after(3, 2, &ops), (1, 3));
        let out = replay(&img, &ops);
        assert_eq!((out.width(), out.height()), (1, 3));
    }

    #[test]
    fn resize_scales_to_the_exact_size() {
        let img = sample();
        let r = ImgOp::Resize { w: 6, h: 1 }.apply(&img);
        assert_eq!((r.width(), r.height()), (6, 1));
        let ops = [ImgOp::RotateCw, ImgOp::Resize { w: 10, h: 0 }];
        assert_eq!(size_after(3, 2, &ops), (10, 1), "never zero");
        assert_eq!(replay(&img, &ops).width(), 10);
    }

    #[test]
    fn crop_rect_is_clamped_and_never_empty() {
        let r = NormRect { x0: 1.2, y0: 0.5, x1: -0.3, y1: 0.5 };
        let (x, y, w, h) = r.to_pixels(10, 10);
        assert_eq!((x, w), (0, 10));
        assert!(h >= 1 && y < 10);
    }

    #[test]
    fn save_policy() {
        use image::ImageFormat as F;
        assert_eq!(save_mode(Some(F::Png)), SaveMode::Lossless(F::Png));
        assert_eq!(save_mode(Some(F::Jpeg)), SaveMode::Lossy(F::Jpeg));
        assert_eq!(save_mode(None), SaveMode::Unsupported);
    }
}
