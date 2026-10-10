//! Font previews: metadata from the `name` / `OS/2` tables (`skrifa`)
//! and a rendered specimen (`ab_glyph` rasterizer): the family name, a pangram at a few
//! sizes, the alphabet and a glyph grid. Specimens are rendered as white coverage masks
//! so the UI can tint them with the theme's text colour.
//!
//! Supports TrueType / OpenType (`ttf`, `otf`), collections (`ttc`, first face) and WOFF 1
//! (zlib tables, unpacked here). WOFF 2 needs a Brotli + glyph-transform decoder and only
//! gets a notice.

use std::io::Read;

use ab_glyph::{Font, FontRef, Glyph, PxScale, ScaleFont, point};

/// Glyphs shown in the grid.
const GRID_GLYPHS: usize = 120;
/// Specimens never grow taller than this (pixels).
const MAX_BLOCK_H: usize = 2400;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontMeta {
    pub family: String,
    pub subfamily: String,
    pub full_name: Option<String>,
    pub version: Option<String>,
    pub designer: Option<String>,
    pub manufacturer: Option<String>,
    pub license: Option<String>,
    pub copyright: Option<String>,
    pub glyphs: u16,
    pub units_per_em: u16,
    pub weight: u16,
    pub italic: bool,
    pub monospaced: bool,
    /// Variation axes `(tag, min, default, max)`.
    pub axes: Vec<(String, f32, f32, f32)>,
    /// Faces in a collection (1 for plain fonts).
    pub faces: u32,
    pub format: &'static str,
}

/// One rendered specimen strip: a white-on-transparent coverage image.
#[derive(Clone)]
pub struct Strip {
    pub caption: String,
    pub width: usize,
    pub height: usize,
    /// Premultiplied white RGBA (alpha = coverage).
    pub rgba: Vec<u8>,
}

/// What the file is.
pub fn format_of(data: &[u8]) -> Option<&'static str> {
    match data.get(..4)? {
        b"wOFF" => Some("WOFF"),
        b"wOF2" => Some("WOFF 2"),
        b"ttcf" => Some(tr!("services-font-collection")),
        b"OTTO" => Some("OpenType (CFF)"),
        [0, 1, 0, 0] | b"true" => Some("TrueType"),
        _ => None,
    }
}

/// Unpack a WOFF 1 file into a plain sfnt.
pub fn woff_to_sfnt(data: &[u8], cap: usize) -> Result<Vec<u8>, String> {
    let u16be = |at: usize| data.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let u32be = |at: usize| data.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let bad = || tr!("services-font-bad-woff").to_string();
    let flavor = u32be(4).ok_or_else(bad)?;
    let num = u16be(12).ok_or_else(bad)? as usize;
    if num == 0 || num > 512 {
        return Err(bad());
    }
    let mut tables = Vec::with_capacity(num);
    for i in 0..num {
        let at = 44 + i * 20;
        let tag = data.get(at..at + 4).ok_or_else(bad)?.to_vec();
        let off = u32be(at + 4).ok_or_else(bad)? as usize;
        let comp = u32be(at + 8).ok_or_else(bad)? as usize;
        let orig = u32be(at + 12).ok_or_else(bad)? as usize;
        let csum = u32be(at + 16).ok_or_else(bad)?;
        if orig > cap {
            return Err(tr!("services-font-woff-table").into());
        }
        let raw = data.get(off..off.checked_add(comp).ok_or_else(bad)?).ok_or_else(bad)?;
        let body = if comp < orig {
            let mut out = Vec::with_capacity(orig);
            flate2::read::ZlibDecoder::new(raw).take(orig as u64).read_to_end(&mut out).map_err(|_| bad())?;
            out
        } else {
            raw.to_vec()
        };
        tables.push((tag, csum, body));
    }
    // sfnt header + table directory, then 4-byte aligned tables.
    let mut entry_sel = 0u16;
    while (1usize << (entry_sel + 1)) <= num {
        entry_sel += 1;
    }
    let search = (1u16 << entry_sel) * 16;
    let mut out = Vec::new();
    out.extend_from_slice(&flavor.to_be_bytes());
    out.extend_from_slice(&(num as u16).to_be_bytes());
    out.extend_from_slice(&search.to_be_bytes());
    out.extend_from_slice(&entry_sel.to_be_bytes());
    out.extend_from_slice(&((num as u16) * 16 - search).to_be_bytes());
    let mut offset = 12 + num * 16;
    for (tag, csum, body) in &tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&csum.to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        offset += body.len().div_ceil(4) * 4;
    }
    for (_, _, body) in &tables {
        out.extend_from_slice(body);
        out.resize(out.len().div_ceil(4) * 4, 0);
    }
    Ok(out)
}

/// Read the metadata of face 0.
pub fn meta(data: &[u8]) -> Result<FontMeta, String> {
    use skrifa::MetadataProvider;
    use skrifa::raw::FileRef;
    use skrifa::string::StringId as N;
    let file = FileRef::new(data).map_err(|e| tr!("services-font-unreadable", error = e.to_string()))?;
    let faces = match &file {
        FileRef::Collection(c) => c.len(),
        FileRef::Font(_) => 1,
    };
    let face = file
        .fonts()
        .next()
        .ok_or(tr!("services-font-empty-collection"))?
        .map_err(|e| tr!("services-font-unreadable", error = e.to_string()))?;
    let name = |id: N| -> Option<String> {
        face.localized_strings(id)
            .english_or_first()
            .map(|s| s.to_string().trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let family = name(N::TYPOGRAPHIC_FAMILY_NAME)
        .or_else(|| name(N::FAMILY_NAME))
        .unwrap_or_else(|| tr!("services-font-unnamed").into());
    let subfamily = name(N::TYPOGRAPHIC_SUBFAMILY_NAME)
        .or_else(|| name(N::SUBFAMILY_NAME))
        .unwrap_or_else(|| tr!("services-font-regular").into());
    let axes =
        face.axes().iter().map(|a| (a.tag().to_string(), a.min_value(), a.default_value(), a.max_value())).collect();
    let attrs = face.attributes();
    let metrics = face.metrics(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default());
    Ok(FontMeta {
        family,
        subfamily,
        full_name: name(N::FULL_NAME),
        version: name(N::VERSION_STRING).map(|v| v.trim_start_matches("Version ").to_string()),
        designer: name(N::DESIGNER),
        manufacturer: name(N::MANUFACTURER),
        license: name(N::LICENSE_DESCRIPTION),
        copyright: name(N::COPYRIGHT_NOTICE),
        glyphs: metrics.glyph_count,
        units_per_em: metrics.units_per_em,
        weight: attrs.weight.value().round().clamp(0.0, 1000.0) as u16,
        italic: !matches!(attrs.style, skrifa::attribute::Style::Normal),
        monospaced: metrics.is_monospace,
        axes,
        faces,
        format: format_of(data).unwrap_or(tr!("services-kind-font")),
    })
}

/// Greedy word wrap of `text` at `px` into lines no wider than `max_w`.
fn wrap<F: Font>(font: &F, px: f32, text: &str, max_w: f32) -> Vec<String> {
    let sf = font.as_scaled(PxScale::from(px));
    let width = |s: &str| -> f32 {
        let mut w = 0.0;
        let mut prev = None;
        for c in s.chars() {
            let id = sf.glyph_id(c);
            if let Some(p) = prev {
                w += sf.kern(p, id);
            }
            w += sf.h_advance(id);
            prev = Some(id);
        }
        w
    };
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split(' ') {
        let cand = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if width(&cand) <= max_w || cur.is_empty() {
            cur = cand;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur = word.to_string();
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Rasterize lines of text at `px` into a strip `width` wide.
fn text_strip<F: Font>(font: &F, px: f32, text: &str, width: usize, caption: String) -> Strip {
    let sf = font.as_scaled(PxScale::from(px));
    let lines = wrap(font, px, text, width as f32 - 4.0);
    let line_h = (sf.ascent() - sf.descent() + sf.line_gap()).max(px * 0.8);
    let height = ((line_h * lines.len().max(1) as f32).ceil() as usize + 4).min(MAX_BLOCK_H);
    let mut rgba = vec![0u8; width * height * 4];
    for (li, line) in lines.iter().enumerate() {
        let baseline = 2.0 + sf.ascent() + li as f32 * line_h;
        let mut x = 2.0;
        let mut prev = None;
        for c in line.chars() {
            let id = sf.glyph_id(c);
            if let Some(p) = prev {
                x += sf.kern(p, id);
            }
            let g: Glyph = id.with_scale_and_position(PxScale::from(px), point(x, baseline));
            x += sf.h_advance(id);
            prev = Some(id);
            blit(font, g, &mut rgba, width, height);
        }
    }
    Strip { caption, width, height, rgba }
}

fn blit<F: Font>(font: &F, g: Glyph, rgba: &mut [u8], width: usize, height: usize) {
    let Some(og) = font.outline_glyph(g) else { return };
    let b = og.px_bounds();
    og.draw(|gx, gy, cov| {
        let x = b.min.x as i64 + gx as i64;
        let y = b.min.y as i64 + gy as i64;
        if x < 0 || y < 0 || x >= width as i64 || y >= height as i64 {
            return;
        }
        let i = (y as usize * width + x as usize) * 4;
        let a = ((cov.clamp(0.0, 1.0) * 255.0) as u8).max(rgba[i + 3]);
        rgba[i..i + 4].copy_from_slice(&[a, a, a, a]);
    });
}

/// Grid of covered glyphs, `cell` px squares.
fn grid_strip<F: Font>(font: &F, chars: &[char], width: usize, cell: usize) -> Strip {
    let cols = (width / cell).max(1);
    let rows = chars.len().div_ceil(cols).max(1);
    let height = (rows * cell).min(MAX_BLOCK_H);
    let mut rgba = vec![0u8; width * height * 4];
    let px = cell as f32 * 0.62;
    let sf = font.as_scaled(PxScale::from(px));
    // Faint cell borders so the grid reads as a grid.
    for r in 0..=rows {
        let y = (r * cell).min(height - 1);
        for x in 0..cols * cell {
            let i = (y * width + x.min(width - 1)) * 4;
            rgba[i..i + 4].copy_from_slice(&[28, 28, 28, 28]);
        }
    }
    for c in 0..=cols {
        let x = (c * cell).min(width - 1);
        for y in 0..height {
            let i = (y * width + x) * 4;
            rgba[i..i + 4].copy_from_slice(&[28, 28, 28, 28]);
        }
    }
    for (n, ch) in chars.iter().enumerate() {
        let (r, c) = (n / cols, n % cols);
        if (r + 1) * cell > height {
            break;
        }
        let id = sf.glyph_id(*ch);
        let adv = sf.h_advance(id);
        let x = c as f32 * cell as f32 + (cell as f32 - adv) / 2.0;
        let baseline = r as f32 * cell as f32 + cell as f32 / 2.0 + (sf.ascent() + sf.descent()) / 2.0;
        blit(font, id.with_scale_and_position(PxScale::from(px), point(x, baseline)), &mut rgba, width, height);
    }
    Strip { caption: tr!("services-font-glyphs-caption", count = chars.len()), width, height, rgba }
}

const PANGRAM: &str = "The quick brown fox jumps over the lazy dog.";

/// Render the specimen strips at `width` pixels; `scale` is pixels per point, so the
/// captioned sizes are true point sizes on screen.
pub fn specimen(data: &[u8], family: &str, width: usize, scale: f32) -> Result<Vec<Strip>, String> {
    let font = FontRef::try_from_slice_and_index(data, 0).map_err(|_| tr!("services-font-cant-render").to_string())?;
    let width = width.clamp(160, 4096);
    let covered: Vec<char> = {
        let mut v: Vec<char> = font.codepoint_ids().filter(|(id, _)| id.0 != 0).map(|(_, c)| c).collect();
        v.sort_unstable();
        v.dedup();
        v.retain(|c| !c.is_control() && !c.is_whitespace());
        v
    };
    let latin = ['a', 'e', 'o', 'T', 'q'].iter().all(|c| covered.binary_search(c).is_ok());
    // The pangram of the UI language when the font covers it, else the English one.
    let local = tr!("services-font-pangram");
    let pangram =
        if local.chars().all(|c| !c.is_alphabetic() || covered.binary_search(&c).is_ok()) { local } else { PANGRAM };
    let sample: String = if latin { pangram.into() } else { covered.iter().take(40).collect() };
    let s = scale.clamp(0.5, 4.0);
    let mut out = Vec::new();
    let title = if latin || family.chars().all(|c| covered.binary_search(&c).is_ok() || c == ' ') {
        family.to_string()
    } else {
        sample.chars().take(12).collect()
    };
    out.push(text_strip(&font, 40.0 * s, &title, width, String::new()));
    for px in [30.0, 20.0, 14.0] {
        out.push(text_strip(&font, px * s, &sample, width, format!("{px:.0} pt")));
    }
    if latin {
        out.push(text_strip(
            &font,
            18.0 * s,
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789 .,;:!?&@#%()[]{}",
            width,
            tr!("services-font-characters").into(),
        ));
    }
    let grid: Vec<char> = covered.iter().copied().take(GRID_GLYPHS).collect();
    if !grid.is_empty() {
        out.push(grid_strip(&font, &grid, width, (44.0 * s).round().max(24.0) as usize));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font bundled with egui (Hack / Ubuntu Light), so the test needs no system files.
    fn system_font() -> Option<Vec<u8>> {
        let defs = egui::FontDefinitions::default();
        let mut keys: Vec<&String> = defs.font_data.keys().collect();
        keys.sort_by_key(|k| !k.contains("Hack"));
        keys.into_iter().map(|k| defs.font_data[k].font.to_vec()).find(|d| format_of(d) == Some("TrueType"))
    }

    #[test]
    fn metadata_and_specimen_from_a_real_font() {
        let Some(data) = system_font() else { return };
        let m = meta(&data).unwrap();
        assert!(!m.family.is_empty());
        assert!(m.glyphs > 100);
        assert!(m.units_per_em >= 16);
        assert_eq!(m.faces, 1);
        assert_eq!(m.format, "TrueType");
        let strips = specimen(&data, &m.family, 640, 1.0).unwrap();
        assert!(strips.len() >= 5);
        for s in &strips {
            assert_eq!(s.rgba.len(), s.width * s.height * 4);
        }
        assert!(strips[1].rgba.chunks(4).any(|p| p[3] > 200), "pangram has ink");
    }

    #[test]
    fn woff_roundtrip_of_a_real_font() {
        let Some(sfnt) = system_font() else { return };
        // Build a WOFF 1 from the sfnt (all tables zlib-compressed).
        let num = u16::from_be_bytes([sfnt[4], sfnt[5]]) as usize;
        let mut dir = Vec::new();
        let mut bodies = Vec::new();
        let mut off = 44 + num * 20;
        for i in 0..num {
            let e = &sfnt[12 + i * 16..28 + i * 16];
            let o = u32::from_be_bytes([e[8], e[9], e[10], e[11]]) as usize;
            let l = u32::from_be_bytes([e[12], e[13], e[14], e[15]]) as usize;
            let raw = &sfnt[o..o + l];
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            std::io::Write::write_all(&mut z, raw).unwrap();
            let comp = z.finish().unwrap();
            let comp = if comp.len() < raw.len() { comp } else { raw.to_vec() };
            dir.extend_from_slice(&e[0..4]);
            dir.extend_from_slice(&(off as u32).to_be_bytes());
            dir.extend_from_slice(&(comp.len() as u32).to_be_bytes());
            dir.extend_from_slice(&(l as u32).to_be_bytes());
            dir.extend_from_slice(&e[4..8]);
            off += comp.len().div_ceil(4) * 4;
            let mut c = comp;
            c.resize(c.len().div_ceil(4) * 4, 0);
            bodies.push(c);
        }
        let mut woff = b"wOFF".to_vec();
        woff.extend_from_slice(&sfnt[0..4]);
        woff.extend_from_slice(&(off as u32).to_be_bytes());
        woff.extend_from_slice(&(num as u16).to_be_bytes());
        woff.extend_from_slice(&[0; 30]);
        woff.extend_from_slice(&dir);
        for b in bodies {
            woff.extend_from_slice(&b);
        }
        assert_eq!(format_of(&woff), Some("WOFF"));
        let back = woff_to_sfnt(&woff, 64 << 20).unwrap();
        assert_eq!(meta(&back).unwrap().family, meta(&sfnt).unwrap().family);
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(meta(b"definitely not a font").is_err());
        assert!(woff_to_sfnt(b"wOFF\0\0\0\0", 1024).is_err());
        assert!(specimen(b"nope", "x", 300, 1.0).is_err());
        assert_eq!(format_of(b"OTTO...."), Some("OpenType (CFF)"));
    }
}
