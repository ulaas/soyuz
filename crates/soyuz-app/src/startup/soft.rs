//! A small CPU rasterizer for egui's tessellated output (roadmap R5, `[performance]
//! cpu_first_frame`): draws the real UI into a BGRA pixel buffer while the GPU renderer
//! is still starting.
//!
//! It mirrors egui-wgpu's gamma-framebuffer path so the CPU frames and the first GPU
//! frame look the same: vertex color × texel in gamma space (both premultiplied sRGB),
//! premultiplied "over" blending in gamma space, scissor rectangles rounded like
//! egui-wgpu's, bilinear filtering for linear textures. Anti-aliasing comes from egui's
//! feathered vertices, so no coverage computation is needed. Rows are split into bands
//! drawn on scoped threads.

use std::collections::HashMap;
use std::sync::Arc;

use egui::epaint::textures::{TextureFilter, TexturesDelta};
use egui::epaint::{ClippedPrimitive, ColorImage, ImageData, ImageDelta, Primitive};
use egui::{Color32, TextureId, TextureOptions};

/// One texture (premultiplied sRGBA, row-major).
struct Texture {
    size: [usize; 2],
    pixels: Vec<Color32>,
    options: TextureOptions,
}

/// The textures egui has handed out so far (font atlas, images, icons).
#[derive(Default)]
pub struct Textures {
    map: HashMap<TextureId, Texture>,
}

impl Textures {
    /// Apply the `set` part of a frame's delta (call before painting).
    pub fn set(&mut self, delta: &TexturesDelta) {
        for (id, deltas) in &delta.set {
            for d in deltas {
                self.apply(*id, d);
            }
        }
    }

    /// Apply the `free` part of a frame's delta (call after painting); this consumes it.
    pub fn free(&mut self, mut delta: TexturesDelta) {
        for id in &delta.free {
            self.map.remove(id);
        }
        delta.clear();
    }

    fn apply(&mut self, id: TextureId, d: &ImageDelta) {
        let ImageData::Color(img) = &d.image;
        let [w, h] = img.size;
        match d.pos {
            None => {
                self.map.insert(id, Texture { size: img.size, pixels: img.pixels.clone(), options: d.options });
            }
            Some([x0, y0]) => {
                let Some(t) = self.map.get_mut(&id) else { return };
                t.options = d.options;
                for y in 0..h {
                    let ty = y0 + y;
                    if ty >= t.size[1] {
                        break;
                    }
                    let n = w.min(t.size[0].saturating_sub(x0));
                    let dst = ty * t.size[0] + x0;
                    t.pixels[dst..dst + n].copy_from_slice(&img.pixels[y * w..y * w + n]);
                }
            }
        }
    }

    /// Every live texture as a full upload, so a renderer that starts later (the GPU one)
    /// gets what the CPU frames already consumed from egui's texture manager.
    pub fn full_deltas(&self) -> Vec<(TextureId, ImageDelta)> {
        self.map
            .iter()
            .map(|(id, t)| {
                let img = ColorImage::new(t.size, t.pixels.clone());
                (*id, ImageDelta::full(ImageData::Color(Arc::new(img)), t.options))
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// A target buffer: `w × h` pixels, BGRA bytes (premultiplied), top-down.
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub bgra: Vec<u8>,
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Canvas {
        Canvas { w, h, bgra: vec![0; w * h * 4] }
    }
}

/// Paint `prims` (tessellated at `ppp` pixels per point) over `clear` (premultiplied
/// sRGBA) into `canvas`. `threads` = row bands drawn in parallel (1 = this thread).
pub fn paint(
    canvas: &mut Canvas,
    prims: &[ClippedPrimitive],
    tex: &Textures,
    ppp: f32,
    clear: Color32,
    threads: usize,
) {
    let (w, h) = (canvas.w, canvas.h);
    if w == 0 || h == 0 {
        return;
    }
    let clear_px = [clear.b(), clear.g(), clear.r(), clear.a()];
    let bands = threads.clamp(1, 16).min(h);
    let rows_per = h.div_ceil(bands);
    let chunks: Vec<(usize, &mut [u8])> =
        canvas.bgra.chunks_mut(rows_per * w * 4).enumerate().map(|(i, c)| (i * rows_per, c)).collect();
    let work = |y0: usize, buf: &mut [u8]| {
        for px in buf.as_chunks_mut::<4>().0 {
            *px = clear_px;
        }
        let y1 = y0 + buf.len() / (w * 4);
        let mut band = Band { buf, w, y0, y1 };
        for p in prims {
            if let Primitive::Mesh(mesh) = &p.primitive
                && let Some(t) = tex.map.get(&mesh.texture_id)
            {
                // egui-wgpu's scissor: rounded to whole pixels, clamped to the target.
                let clip = p.clip_rect;
                let cx0 = ((clip.min.x * ppp).round().max(0.0) as usize).min(w);
                let cy0 = ((clip.min.y * ppp).round().max(0.0) as usize).min(h);
                let cx1 = ((clip.max.x * ppp).round().max(0.0) as usize).min(w);
                let cy1 = ((clip.max.y * ppp).round().max(0.0) as usize).min(h);
                let cy0 = cy0.max(band.y0);
                let cy1 = cy1.min(band.y1);
                if cx0 >= cx1 || cy0 >= cy1 {
                    continue;
                }
                let clip = [cx0, cy0, cx1, cy1];
                for tri in mesh.indices.as_chunks::<3>().0 {
                    let v = |i: u32| &mesh.vertices[i as usize];
                    band.triangle([v(tri[0]), v(tri[1]), v(tri[2])], ppp, t, clip);
                }
            }
        }
    };
    if bands == 1 {
        for (y0, buf) in chunks {
            work(y0, buf);
        }
    } else {
        std::thread::scope(|s| {
            for (y0, buf) in chunks {
                s.spawn(move || work(y0, buf));
            }
        });
    }
}

/// Rows `y0..y1` of the canvas.
struct Band<'a> {
    buf: &'a mut [u8],
    w: usize,
    y0: usize,
    y1: usize,
}

#[derive(Clone, Copy)]
struct P {
    x: f32,
    y: f32,
}

fn edge(a: P, b: P, x: f32, y: f32) -> f32 {
    (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x)
}

/// Top-left fill rule (for the orientation used below): a pixel center exactly on a
/// shared edge belongs to exactly one of the two triangles.
fn owns_edge(a: P, b: P) -> bool {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    dy < 0.0 || (dy == 0.0 && dx > 0.0)
}

impl Band<'_> {
    fn triangle(&mut self, v: [&egui::epaint::Vertex; 3], ppp: f32, t: &Texture, clip: [usize; 4]) {
        let mut p = [0, 1, 2].map(|i| P { x: v[i].pos.x * ppp, y: v[i].pos.y * ppp });
        let mut v = v;
        let mut area = edge(p[0], p[1], p[2].x, p[2].y);
        if area.abs() < 1e-8 || !area.is_finite() {
            return;
        }
        if area < 0.0 {
            p.swap(1, 2);
            v.swap(1, 2);
            area = -area;
        }
        // Pixel centers at +0.5.
        let minx = p.iter().map(|q| q.x).fold(f32::INFINITY, f32::min);
        let maxx = p.iter().map(|q| q.x).fold(f32::NEG_INFINITY, f32::max);
        let miny = p.iter().map(|q| q.y).fold(f32::INFINITY, f32::min);
        let maxy = p.iter().map(|q| q.y).fold(f32::NEG_INFINITY, f32::max);
        let x0 = ((minx - 0.5).ceil().max(clip[0] as f32)) as usize;
        let x1 = ((maxx - 0.5).floor() + 1.0).min(clip[2] as f32).max(0.0) as usize;
        let y0 = ((miny - 0.5).ceil().max(clip[1] as f32)) as usize;
        let y1 = ((maxy - 0.5).floor() + 1.0).min(clip[3] as f32).max(0.0) as usize;
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let own = [owns_edge(p[1], p[2]), owns_edge(p[2], p[0]), owns_edge(p[0], p[1])];
        let inv = 1.0 / area;
        let col = v.map(|q| q.color.to_array().map(f32::from));
        let uv = v.map(|q| (q.uv.x, q.uv.y));
        let flat_color = v[0].color == v[1].color && v[1].color == v[2].color;
        let flat_uv = uv[0] == uv[1] && uv[1] == uv[2];
        let linear = t.options.magnification == TextureFilter::Linear;
        let flat_texel = flat_uv.then(|| sample(t, uv[0].0, uv[0].1, linear));
        // Constant source color (solid fills): computed once.
        let flat_src = match (flat_color, flat_texel) {
            (true, Some(tx)) => Some(modulate(col[0], tx)),
            _ => None,
        };
        // Edge function steps per pixel in x.
        let dx = [p[1].y - p[2].y, p[2].y - p[0].y, p[0].y - p[1].y];
        for y in y0..y1 {
            let fy = y as f32 + 0.5;
            let fx = x0 as f32 + 0.5;
            let mut e = [edge(p[1], p[2], fx, fy), edge(p[2], p[0], fx, fy), edge(p[0], p[1], fx, fy)];
            let row = (y - self.y0) * self.w;
            for x in x0..x1 {
                let inside = (0..3).all(|i| e[i] > 0.0 || (e[i] == 0.0 && own[i]));
                if inside {
                    let src = match flat_src {
                        Some(s) => s,
                        None => {
                            let b = [e[0] * inv, e[1] * inv, e[2] * inv];
                            let c = [0, 1, 2, 3].map(|k| b[0] * col[0][k] + b[1] * col[1][k] + b[2] * col[2][k]);
                            let tx = match flat_texel {
                                Some(tx) => tx,
                                None => {
                                    let u = b[0] * uv[0].0 + b[1] * uv[1].0 + b[2] * uv[2].0;
                                    let w = b[0] * uv[0].1 + b[1] * uv[1].1 + b[2] * uv[2].1;
                                    sample(t, u, w, linear)
                                }
                            };
                            modulate(c, tx)
                        }
                    };
                    blend(&mut self.buf[(row + x) * 4..(row + x) * 4 + 4], src);
                }
                for i in 0..3 {
                    e[i] += dx[i];
                }
            }
        }
    }
}

/// Vertex color (0–255 floats, RGBA) × texel → premultiplied RGBA bytes.
fn modulate(c: [f32; 4], tx: [f32; 4]) -> [u8; 4] {
    [0, 1, 2, 3].map(|k| (c[k] * tx[k] / 255.0 + 0.5).clamp(0.0, 255.0) as u8)
}

/// Premultiplied "over" into a BGRA pixel.
fn blend(dst: &mut [u8], src: [u8; 4]) {
    let a = u32::from(src[3]);
    if a == 255 {
        dst.copy_from_slice(&[src[2], src[1], src[0], 255]);
        return;
    }
    if src == [0, 0, 0, 0] {
        return;
    }
    let k = 255 - a;
    let mix = |s: u8, d: u8| (u32::from(s) + (u32::from(d) * k + 127) / 255).min(255) as u8;
    dst[0] = mix(src[2], dst[0]);
    dst[1] = mix(src[1], dst[1]);
    dst[2] = mix(src[0], dst[2]);
    dst[3] = mix(src[3], dst[3]);
}

/// Texel at normalized `(u, v)` (clamped), RGBA 0–255 floats.
fn sample(t: &Texture, u: f32, v: f32, linear: bool) -> [f32; 4] {
    let [w, h] = t.size;
    if w == 0 || h == 0 {
        return [255.0; 4];
    }
    let at = |x: usize, y: usize| t.pixels[y * w + x].to_array().map(f32::from);
    if !linear {
        let x = ((u * w as f32) as isize).clamp(0, w as isize - 1) as usize;
        let y = ((v * h as f32) as isize).clamp(0, h as isize - 1) as usize;
        return at(x, y);
    }
    let fx = (u * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
    let fy = (v * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (fx as usize, fy as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (ax, ay) = (fx - x0 as f32, fy - y0 as f32);
    if ax < 1e-3 && ay < 1e-3 {
        return at(x0, y0);
    }
    let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
    [0, 1, 2, 3].map(|k| {
        let top = a[k] + (b[k] - a[k]) * ax;
        let bot = c[k] + (d[k] - c[k]) * ax;
        top + (bot - top) * ay
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::epaint::{Mesh, Vertex};
    use egui::{Pos2, Rect, pos2};

    fn white_tex() -> Textures {
        let mut t = Textures::default();
        let mut d = TexturesDelta::default();
        let img = ColorImage::new([1, 1], vec![Color32::WHITE]);
        d.set.insert(
            TextureId::default(),
            [ImageDelta::full(ImageData::Color(Arc::new(img)), TextureOptions::LINEAR)].into(),
        );
        t.set(&d);
        d.clear();
        t
    }

    fn rect_mesh(r: Rect, c: Color32) -> Mesh {
        let mut m = Mesh::default();
        m.add_colored_rect(r, c);
        m
    }

    fn px(c: &Canvas, x: usize, y: usize) -> [u8; 4] {
        let i = (y * c.w + x) * 4;
        [c.bgra[i], c.bgra[i + 1], c.bgra[i + 2], c.bgra[i + 3]]
    }

    #[test]
    fn rects_fill_exact_pixels_without_seams() {
        let tex = white_tex();
        let full = Rect::from_min_max(Pos2::ZERO, pos2(8.0, 8.0));
        let prims = vec![ClippedPrimitive {
            clip_rect: full,
            primitive: Primitive::Mesh(rect_mesh(
                Rect::from_min_max(pos2(2.0, 2.0), pos2(6.0, 6.0)),
                Color32::from_rgb(200, 100, 50),
            )),
        }];
        for threads in [1, 3] {
            let mut c = Canvas::new(8, 8);
            paint(&mut c, &prims, &tex, 1.0, Color32::from_rgb(10, 20, 30), threads);
            assert_eq!(px(&c, 0, 0), [30, 20, 10, 255], "clear color, BGRA");
            assert_eq!(px(&c, 2, 2), [50, 100, 200, 255]);
            assert_eq!(px(&c, 5, 5), [50, 100, 200, 255]);
            assert_eq!(px(&c, 6, 5), [30, 20, 10, 255]);
            // Two triangles share the diagonal: a half-transparent fill blends once.
        }
        let half = Color32::from_rgba_premultiplied(100, 0, 0, 128);
        let prims = vec![ClippedPrimitive { clip_rect: full, primitive: Primitive::Mesh(rect_mesh(full, half)) }];
        let mut c = Canvas::new(8, 8);
        paint(&mut c, &prims, &tex, 1.0, Color32::BLACK, 1);
        let first = px(&c, 0, 0);
        assert!((0..8).all(|i| px(&c, i, i) == first && px(&c, 7 - i, i) == first), "no double blend on the diagonal");
        assert_eq!(first, [0, 0, 100, 255]);
    }

    /// Any triangle covers exactly the pixel centers inside it (checked by brute force),
    /// in either winding, and a textured quad samples its texels.
    #[test]
    fn triangles_match_a_brute_force_coverage() {
        let tex = white_tex();
        let tris = [
            [pos2(0.3, 0.2), pos2(13.7, 2.1), pos2(4.2, 11.9)],
            [pos2(1.0, 1.0), pos2(2.5, 14.0), pos2(15.0, 7.25)],
            [pos2(10.0, 0.0), pos2(0.0, 3.0), pos2(15.0, 15.0)],
        ];
        for t in tris {
            let mut m = Mesh::default();
            for p in t {
                m.vertices.push(Vertex { pos: p, uv: egui::epaint::WHITE_UV, color: Color32::WHITE });
            }
            m.indices = vec![0, 1, 2];
            let prims = vec![ClippedPrimitive {
                clip_rect: Rect::from_min_max(Pos2::ZERO, pos2(16.0, 16.0)),
                primitive: Primitive::Mesh(m),
            }];
            let mut c = Canvas::new(16, 16);
            paint(&mut c, &prims, &tex, 1.0, Color32::BLACK, 1);
            let a = P { x: t[0].x, y: t[0].y };
            let b = P { x: t[1].x, y: t[1].y };
            let d = P { x: t[2].x, y: t[2].y };
            let area = edge(a, b, d.x, d.y);
            for y in 0..16 {
                for x in 0..16 {
                    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let e = [edge(b, d, fx, fy), edge(d, a, fx, fy), edge(a, b, fx, fy)];
                    let inside = e.iter().all(|v| v * area.signum() > 0.0);
                    assert_eq!(px(&c, x, y)[0] == 255, inside, "{t:?} at {x},{y}");
                }
            }
        }
        // A quad showing a 2x1 texture (red | blue), nearest filtering.
        let mut tex = white_tex();
        let id = TextureId::Managed(3);
        let mut d = TexturesDelta::default();
        let img = ColorImage::new([2, 1], vec![Color32::RED, Color32::BLUE]);
        d.set.insert(id, [ImageDelta::full(ImageData::Color(Arc::new(img)), TextureOptions::NEAREST)].into());
        tex.set(&d);
        d.clear();
        let mut m = Mesh::with_texture(id);
        m.add_rect_with_uv(
            Rect::from_min_max(Pos2::ZERO, pos2(4.0, 2.0)),
            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let prims = vec![ClippedPrimitive {
            clip_rect: Rect::from_min_max(Pos2::ZERO, pos2(4.0, 2.0)),
            primitive: Primitive::Mesh(m),
        }];
        let mut c = Canvas::new(4, 2);
        paint(&mut c, &prims, &tex, 1.0, Color32::BLACK, 1);
        assert_eq!(px(&c, 0, 1), [0, 0, 255, 255]);
        assert_eq!(px(&c, 3, 0), [255, 0, 0, 255]);
    }

    #[test]
    fn scissor_and_scale_follow_egui_wgpu() {
        let tex = white_tex();
        let r = Rect::from_min_max(Pos2::ZERO, pos2(4.0, 4.0));
        let prims = vec![ClippedPrimitive {
            clip_rect: Rect::from_min_max(pos2(1.0, 1.0), pos2(2.0, 4.0)),
            primitive: Primitive::Mesh(rect_mesh(r, Color32::WHITE)),
        }];
        let mut c = Canvas::new(8, 8);
        paint(&mut c, &prims, &tex, 2.0, Color32::BLACK, 2);
        assert_eq!(px(&c, 1, 3), [0, 0, 0, 255]);
        assert_eq!(px(&c, 2, 2), [255, 255, 255, 255]);
        assert_eq!(px(&c, 3, 7), [255, 255, 255, 255]);
        assert_eq!(px(&c, 4, 4), [0, 0, 0, 255]);
    }

    #[test]
    fn texture_patches_and_reupload() {
        let mut t = white_tex();
        let mut d = TexturesDelta::default();
        let id = TextureId::Managed(7);
        let img = ColorImage::new([2, 2], vec![Color32::RED; 4]);
        d.set.insert(id, [ImageDelta::full(ImageData::Color(Arc::new(img)), TextureOptions::NEAREST)].into());
        t.set(&d);
        d.clear();
        let mut d = TexturesDelta::default();
        let patch = ColorImage::new([1, 1], vec![Color32::BLUE]);
        d.set.insert(
            id,
            [ImageDelta::partial([1, 1], ImageData::Color(Arc::new(patch)), TextureOptions::NEAREST)].into(),
        );
        t.set(&d);
        d.clear();
        assert_eq!(sample(&t.map[&id], 0.75, 0.75, false), Color32::BLUE.to_array().map(f32::from));
        assert_eq!(sample(&t.map[&id], 0.25, 0.25, false), Color32::RED.to_array().map(f32::from));
        assert_eq!(t.full_deltas().len(), 2);
        let mut d = TexturesDelta::default();
        d.free.insert(id);
        t.free(d);
        assert_eq!(t.len(), 1);
        let _ = Vertex::default();
    }
}
