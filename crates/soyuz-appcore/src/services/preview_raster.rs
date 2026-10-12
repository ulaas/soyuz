//! Pixels for frontends that only draw uploaded pictures (the terminal UI): SVG
//! rasterized with `resvg` and a still render of a 3D model ([`preview_model::project`]
//! filled into an RGBA buffer). The GUI draws both itself (egui's SVG loader, a live
//! mesh), so these run only when the request asks for them
//! (`RichLimits::terminal`). Toolkit-free: no GUI crate is involved.

use image::RgbaImage;
use image::imageops::FilterType;

use super::preview_gfx::{Rgba, TriSink};
use super::preview_model::{self, Model, View};

/// A rasterized SVG.
#[derive(Clone, Debug)]
pub struct SvgPixels {
    /// Premultiplied RGBA, `size[0] * size[1] * 4` bytes.
    pub rgba: Vec<u8>,
    pub size: [usize; 2],
    /// The document's own size in user units.
    pub own: [f32; 2],
}

/// Rasterize SVG `bytes` so the longest side is `longest` pixels (aspect kept).
pub fn svg(bytes: &[u8], longest: u32) -> Result<SvgPixels, String> {
    use resvg::{tiny_skia, usvg};
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(bytes, &opt).map_err(|e| e.to_string())?;
    let size = tree.size();
    let (sw, sh) = (size.width(), size.height());
    if !(sw.is_finite() && sh.is_finite()) || sw <= 0.0 || sh <= 0.0 {
        return Err("empty SVG".into());
    }
    let scale = longest.max(1) as f32 / sw.max(sh);
    let w = ((sw * scale).round() as u32).clamp(1, 8192);
    let h = ((sh * scale).round() as u32).clamp(1, 8192);
    let mut pm = tiny_skia::Pixmap::new(w, h).ok_or_else(|| "SVG too large".to_string())?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pm.as_mut());
    Ok(SvgPixels { rgba: pm.take(), size: [w as usize, h as usize], own: [sw, sh] })
}

/// Triangles filled into an RGBA buffer, back to front (painter's order, as
/// [`preview_model::project`] emits them).
struct Fill {
    img: RgbaImage,
}

impl TriSink for Fill {
    fn reserve(&mut self, _tris: usize) {}

    fn tri(&mut self, p: [[f32; 2]; 3], color: Rgba) {
        let (w, h) = (self.img.width() as f32, self.img.height() as f32);
        let min_x = p.iter().map(|q| q[0]).fold(f32::INFINITY, f32::min).floor().max(0.0);
        let max_x = p.iter().map(|q| q[0]).fold(f32::NEG_INFINITY, f32::max).ceil().min(w - 1.0);
        let min_y = p.iter().map(|q| q[1]).fold(f32::INFINITY, f32::min).floor().max(0.0);
        let max_y = p.iter().map(|q| q[1]).fold(f32::NEG_INFINITY, f32::max).ceil().min(h - 1.0);
        if !(min_x <= max_x && min_y <= max_y) {
            return;
        }
        let edge = |a: [f32; 2], b: [f32; 2], x: f32, y: f32| (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
        let area = edge(p[0], p[1], p[2][0], p[2][1]);
        if area == 0.0 || !area.is_finite() {
            return;
        }
        let px = image::Rgba(color);
        for y in min_y as u32..=max_y as u32 {
            for x in min_x as u32..=max_x as u32 {
                let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
                let e0 = edge(p[1], p[2], cx, cy);
                let e1 = edge(p[2], p[0], cx, cy);
                let e2 = edge(p[0], p[1], cx, cy);
                let inside =
                    if area > 0.0 { e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0 } else { e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0 };
                if inside {
                    self.img.put_pixel(x, y, px);
                }
            }
        }
    }
}

/// A still render of `m` from the default view, `size` × `size` pixels (straight RGBA,
/// transparent background), drawn at twice the size and scaled down for smooth edges.
pub fn model_still(m: &Model, size: u32) -> RgbaImage {
    let size = size.clamp(16, 2048);
    let big = size * 2;
    let mut fill = Fill { img: RgbaImage::new(big, big) };
    let s = big as f32;
    // Neutral, like the GUI's tint without the accent.
    preview_model::project(m, View::default(), [s / 2.0, s * 0.48], s * 0.40, [0.9, 0.9, 0.92], &mut fill);
    image::imageops::resize(&fill.img, size, size, FilterType::Triangle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_rasterizes_to_the_longest_side() {
        let doc = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#ff0000"/></svg>"##;
        let SvgPixels { rgba: px, size, own } = svg(doc, 100).unwrap();
        assert_eq!(size, [100, 50]);
        assert_eq!(own, [40.0, 20.0]);
        assert_eq!(px.len(), 100 * 50 * 4);
        assert_eq!(&px[..4], &[255, 0, 0, 255]);
        assert!(svg(b"not svg", 100).is_err());
        assert!(svg(b"", 100).is_err());
    }

    #[test]
    fn model_still_draws_opaque_pixels() {
        let stl = "solid t\n facet normal 0 0 1\n outer loop\n vertex 0 0 0\n vertex 1 0 0\n vertex 0 1 0\n endloop\n endfacet\n facet normal 0 0 1\n outer loop\n vertex 0 0 0\n vertex 0 1 0\n vertex 0 0 1\n endloop\n endfacet\nendsolid t\n";
        let m = preview_model::parse(stl.as_bytes(), "stl", None, 1 << 20).unwrap();
        let img = model_still(&m, 64);
        assert_eq!(img.dimensions(), (64, 64));
        let opaque = img.pixels().filter(|p| p[3] > 200).count();
        assert!(opaque > 50, "{opaque} pixels drawn");
        assert_eq!(img.get_pixel(0, 0)[3], 0, "transparent background");
        // Degenerate input never panics.
        let mut f = Fill { img: RgbaImage::new(4, 4) };
        f.tri([[0.0, 0.0], [0.0, 0.0], [0.0, 0.0]], [1, 2, 3, 255]);
        f.tri([[f32::NAN, 0.0], [9e9, -9e9], [3.0, 3.0]], [1, 2, 3, 255]);
        f.tri([[-10.0, -10.0], [-5.0, -10.0], [-5.0, -5.0]], [1, 2, 3, 255]);
    }
}
