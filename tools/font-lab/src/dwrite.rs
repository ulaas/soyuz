//! Windows' own glyph rasterizer, as a reference: DirectWrite's raw coverage textures,
//! with the lab's shaping and compositing around them.

use std::path::Path;

use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_ISOLATED, DWRITE_FONT_FACE_TYPE_TRUETYPE, DWRITE_FONT_SIMULATIONS_NONE, DWRITE_GLYPH_OFFSET,
    DWRITE_GLYPH_RUN, DWRITE_MEASURING_MODE, DWRITE_RENDERING_MODE, DWRITE_RENDERING_MODE_ALIASED,
    DWRITE_TEXTURE_ALIASED_1x1, DWRITE_TEXTURE_CLEARTYPE_3x1, DWriteCreateFactory, IDWriteFactory, IDWriteFontFile,
};
use windows::core::{HSTRING, Result};

/// Coverage of every line, three values per pixel. `lines`: baseline and
/// `(glyph, advance, x offset, y offset)` per glyph.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    path: &Path,
    em: f32,
    lines: &[(f32, Vec<(u32, f32, f32, f32)>)],
    left: f32,
    w: u32,
    h: u32,
    mode: i32,
    measuring: i32,
) -> Result<Vec<f32>> {
    let (w, h) = (i64::from(w), i64::from(h));
    let mut out = vec![0.0f32; (w * h * 3) as usize];
    // SAFETY: plain COM calls; every pointer outlives the call that takes it, and the
    // buffer has the size the returned bounds ask for.
    unsafe {
        let factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_ISOLATED)?;
        let file: IDWriteFontFile = factory.CreateFontFileReference(&HSTRING::from(path), None)?;
        let face =
            factory.CreateFontFace(DWRITE_FONT_FACE_TYPE_TRUETYPE, &[Some(file)], 0, DWRITE_FONT_SIMULATIONS_NONE)?;
        let mode = DWRITE_RENDERING_MODE(mode);
        let aliased = mode == DWRITE_RENDERING_MODE_ALIASED;
        let kind = if aliased { DWRITE_TEXTURE_ALIASED_1x1 } else { DWRITE_TEXTURE_CLEARTYPE_3x1 };
        for (base, glyphs) in lines {
            if glyphs.is_empty() {
                continue;
            }
            let ids: Vec<u16> = glyphs.iter().map(|g| g.0 as u16).collect();
            let adv: Vec<f32> = glyphs.iter().map(|g| g.1).collect();
            let off: Vec<DWRITE_GLYPH_OFFSET> =
                glyphs.iter().map(|g| DWRITE_GLYPH_OFFSET { advanceOffset: g.2, ascenderOffset: -g.3 }).collect();
            let mut run = DWRITE_GLYPH_RUN {
                fontFace: std::mem::ManuallyDrop::new(Some(face.clone())),
                fontEmSize: em,
                glyphCount: ids.len() as u32,
                glyphIndices: ids.as_ptr(),
                glyphAdvances: adv.as_ptr(),
                glyphOffsets: off.as_ptr(),
                isSideways: false.into(),
                bidiLevel: 0,
            };
            let analysis =
                factory.CreateGlyphRunAnalysis(&run, 1.0, None, mode, DWRITE_MEASURING_MODE(measuring), left, *base);
            std::mem::ManuallyDrop::drop(&mut run.fontFace);
            let analysis = analysis?;
            let r = analysis.GetAlphaTextureBounds(kind)?;
            let (bw, bh) = (i64::from(r.right - r.left), i64::from(r.bottom - r.top));
            if bw <= 0 || bh <= 0 {
                continue;
            }
            let per: i64 = if aliased { 1 } else { 3 };
            let mut buf = vec![0u8; (bw * bh * per) as usize];
            analysis.CreateAlphaTexture(kind, &r, &mut buf)?;
            for y in 0..bh {
                let dy = y + i64::from(r.top);
                if !(0..h).contains(&dy) {
                    continue;
                }
                for x in 0..bw {
                    let dx = x + i64::from(r.left);
                    if !(0..w).contains(&dx) {
                        continue;
                    }
                    let src = ((y * bw + x) * per) as usize;
                    let dst = ((dy * w + dx) * 3) as usize;
                    for c in 0..3 {
                        let v = buf[src + if aliased { 0 } else { c }];
                        out[dst + c] = out[dst + c].max(f32::from(v) / 255.0);
                    }
                }
            }
        }
    }
    Ok(out)
}
