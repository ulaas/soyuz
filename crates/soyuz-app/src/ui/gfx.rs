//! egui side of the neutral preview / thumbnail graphics (`services::preview_gfx`):
//! the texture uploader the workers call, texture handle access, colours, styled text
//! lines as `LayoutJob`s and model triangles straight into an `epaint::Mesh`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use egui::text::{ByteIndex, LayoutJob, LayoutSection, TextFormat, TextWrapping};
use egui::{Color32, ColorImage, FontFamily, FontId, Stroke, TextureHandle, TextureId, TextureOptions, Vec2, epaint};

use crate::services::preview_gfx::{FontHint, Rgba, StyledLine, Tex, TextFont, TriSink, Uploader};

/// Names the media player uploads under: one per player (`player:<n>`), every frame.
const PLAYER_PREFIX: &str = "player:";

/// Uploads worker pixels as egui textures (linear filtering), exactly as the workers
/// used to with `ctx.load_texture`. The media player's frames (`player:<n>`) update one
/// texture in place ([`TextureHandle::set`]) instead of allocating one per frame; a new
/// player name lets the previous player's texture go.
pub fn egui_uploader(ctx: &egui::Context) -> Uploader {
    let ctx = ctx.clone();
    let reused: Mutex<HashMap<String, TextureHandle>> = Mutex::new(HashMap::new());
    Arc::new(move |name, px| {
        let img = if px.premultiplied {
            ColorImage::from_rgba_premultiplied(px.size, px.rgba)
        } else {
            ColorImage::from_rgba_unmultiplied(px.size, px.rgba)
        };
        if name.starts_with(PLAYER_PREFIX) {
            let mut map = reused.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(h) = map.get_mut(&name) {
                h.set(img, TextureOptions::LINEAR);
                return Arc::new(h.clone());
            }
            map.retain(|k, _| !k.starts_with(PLAYER_PREFIX));
            let h = ctx.load_texture(name.clone(), img, TextureOptions::LINEAR);
            map.insert(name, h.clone());
            return Arc::new(h);
        }
        Arc::new(ctx.load_texture(name, img, TextureOptions::LINEAR))
    })
}

/// egui access to an uploaded [`Tex`].
pub trait TexExt {
    /// The egui texture (`None` only if it was made by another uploader).
    fn egui(&self) -> Option<&TextureHandle>;
    /// The texture id to paint with.
    fn id(&self) -> TextureId;
    /// Size in pixels as a vector.
    fn size_vec2(&self) -> Vec2;
}

impl TexExt for Tex {
    fn egui(&self) -> Option<&TextureHandle> {
        self.handle::<TextureHandle>()
    }
    fn id(&self) -> TextureId {
        self.egui().map_or_else(TextureId::default, TextureHandle::id)
    }
    fn size_vec2(&self) -> Vec2 {
        egui::vec2(self.size[0] as f32, self.size[1] as f32)
    }
}

/// A neutral colour as `Color32` (both are premultiplied sRGBA: bit-identical).
pub fn color32(c: Rgba) -> Color32 {
    Color32::from_rgba_premultiplied(c[0], c[1], c[2], c[3])
}

/// A `Color32` as a neutral colour.
pub fn rgba(c: Color32) -> Rgba {
    c.to_array()
}

/// A neutral font request as an egui font id.
pub fn font_id(f: TextFont) -> FontId {
    let family = match f.family {
        FontHint::Monospace => FontFamily::Monospace,
        FontHint::Proportional => FontFamily::Proportional,
    };
    FontId::new(f.size, family)
}

/// One styled line as a single-row `LayoutJob` (egui has no bold flag; runs keep their
/// font, colour, italics and underline).
pub fn layout_job(line: &StyledLine) -> LayoutJob {
    let sections = line
        .runs
        .iter()
        .map(|r| {
            let color = color32(r.style.color);
            LayoutSection {
                leading_space: 0.0,
                byte_range: ByteIndex(r.range.start)..ByteIndex(r.range.end),
                format: TextFormat {
                    font_id: font_id(r.style.font),
                    color,
                    italics: r.style.italic,
                    underline: if r.style.underline { Stroke::new(1.0, color) } else { Stroke::NONE },
                    ..Default::default()
                },
            }
        })
        .collect();
    LayoutJob {
        text: line.text.clone(),
        sections,
        wrap: TextWrapping { max_rows: 1, ..Default::default() },
        ..Default::default()
    }
}

/// Model triangles go straight into an egui mesh (white UV, flat colour per triangle).
pub struct MeshSink(pub epaint::Mesh);

impl TriSink for MeshSink {
    fn reserve(&mut self, tris: usize) {
        self.0.vertices.reserve(tris * 3);
        self.0.indices.reserve(tris * 3);
    }
    fn tri(&mut self, pts: [[f32; 2]; 3], color: Rgba) {
        let color = color32(color);
        let idx = self.0.vertices.len() as u32;
        for [x, y] in pts {
            self.0.vertices.push(epaint::Vertex { pos: egui::pos2(x, y), uv: epaint::WHITE_UV, color });
        }
        self.0.indices.extend_from_slice(&[idx, idx + 1, idx + 2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::preview_gfx::{Pixels, RunStyle};

    #[test]
    fn colours_round_trip_bit_identically() {
        for c in [Color32::WHITE, Color32::from_rgb(1, 2, 3), Color32::from_rgba_unmultiplied(200, 100, 50, 77)] {
            assert_eq!(color32(rgba(c)), c);
        }
    }

    #[test]
    fn styled_lines_match_appended_jobs() {
        let font = TextFont::monospace(12.0);
        let mut line = StyledLine::default();
        line.append("let", RunStyle { italic: true, ..RunStyle::plain(font, [10, 20, 30, 255]) });
        line.append(" x", RunStyle::plain(font, [255; 4]));
        let mut want = LayoutJob::default();
        let fmt = |c: Color32, italics| TextFormat {
            font_id: FontId::monospace(12.0),
            color: c,
            italics,
            ..Default::default()
        };
        want.append("let", 0.0, fmt(Color32::from_rgb(10, 20, 30), true));
        want.append(" x", 0.0, fmt(Color32::WHITE, false));
        want.wrap.max_rows = 1;
        assert_eq!(layout_job(&line), want);
    }

    #[test]
    fn player_frames_update_one_texture_in_place() {
        let ctx = egui::Context::default();
        let up = egui_uploader(&ctx);
        let px = [0u8, 255, 0, 255].repeat(4);
        let a = Tex::upload(&up, "player:1".into(), Pixels { size: [2, 2], rgba: &px, premultiplied: false });
        let b = Tex::upload(&up, "player:1".into(), Pixels { size: [2, 2], rgba: &px, premultiplied: false });
        assert_eq!(a.id(), b.id(), "the same player name reuses its texture");
        // A size change is a full update of the same texture too.
        let px2 = [0u8, 0, 255, 255].repeat(6);
        let c = Tex::upload(&up, "player:1".into(), Pixels { size: [3, 2], rgba: &px2, premultiplied: false });
        assert_eq!(a.id(), c.id());
        assert_eq!(c.egui().map(TextureHandle::size), Some([3, 2]));
        // Another player gets its own texture; the old name is forgotten (a later upload
        // under it allocates again).
        let d = Tex::upload(&up, "player:2".into(), Pixels { size: [2, 2], rgba: &px, premultiplied: false });
        assert_ne!(a.id(), d.id());
        let e = Tex::upload(&up, "player:1".into(), Pixels { size: [2, 2], rgba: &px, premultiplied: false });
        assert_ne!(e.id(), a.id());
        // Other names stay one texture per upload.
        let f = Tex::upload(&up, "poster".into(), Pixels { size: [2, 2], rgba: &px, premultiplied: false });
        let g = Tex::upload(&up, "poster".into(), Pixels { size: [2, 2], rgba: &px, premultiplied: false });
        assert_ne!(f.id(), g.id());
    }

    #[test]
    fn uploads_make_egui_textures() {
        let ctx = egui::Context::default();
        let up = egui_uploader(&ctx);
        let px = [255u8, 0, 0, 128].repeat(6);
        let t = Tex::upload(&up, "t".into(), Pixels { size: [3, 2], rgba: &px, premultiplied: false });
        assert_eq!(t.egui().map(TextureHandle::size), Some([3, 2]));
        assert_eq!(t.size_vec2(), egui::vec2(3.0, 2.0));
        assert_ne!(t.id(), TextureId::default());
    }
}
