//! The hayro backend: loading, region rendering, the text device and metadata. The only
//! module (with `nav`) that uses hayro's types.

use std::sync::Arc;

use hayro::hayro_interpret::font::{FallbackFontQuery, FontData, FontQuery, Glyph, GlyphRun};
use hayro::hayro_interpret::hayro_cmap::{BfString, CidFamily};
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps, InterpreterCache,
    InterpreterSettings, RenderContent, SoftMask, interpret_page,
};
use hayro::hayro_syntax::object::DateTime;
use hayro::hayro_syntax::page::{Page, Rotation};
use hayro::hayro_syntax::{DecryptionError, LoadPdfError, Pdf, PdfData, PdfVersion};
use hayro::kurbo::{Affine, BezPath, Point, Rect};
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::{PixmapMut, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{RenderCache, RenderSettings};

use super::fonts::{self, Cjk, FontWant};
use super::text::{PageText, RawGlyph};
use super::{Bitmap, DocInfo, EngineKind, MAX_PAGE_PX, PageGeom, PdfError, Region, RenderOpts};
use crate::services::preview_pdf::{clean_text_max, decode_pdf_string};

// The session hands documents between threads only as bytes, but the engine types are
// thread safe (checked here so an upgrade that changes it fails to build).
const _: fn() = || {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Pdf>();
    send_sync::<Page<'static>>();
};

/// Pages a worker renders before it reloads the document: pages cache their decoded
/// content inside the `Pdf`, so recycling it bounds memory.
pub(crate) const RECYCLE_PAGES: usize = 64;

/// The shared file bytes as hayro's input (no copy).
struct Shared(Arc<[u8]>);

impl AsRef<[u8]> for Shared {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Parse a document. `password` is passed as UTF-8 bytes (a non-ASCII password on an
/// RC4 / AES-128 file may not match what the producer hashed).
pub(crate) fn load(bytes: Arc<[u8]>, password: Option<&str>) -> Result<Pdf, PdfError> {
    let data = PdfData::from(Arc::new(Shared(bytes)));
    super::guard::catch(|| Pdf::new_with_password(data, password.unwrap_or(""))).and_then(|r| {
        r.map_err(|e| match e {
            LoadPdfError::Decryption(DecryptionError::PasswordProtected) => {
                PdfError::Password { wrong: password.is_some() }
            }
            LoadPdfError::Decryption(e) => PdfError::Other(format!("{e:?}")),
            LoadPdfError::Invalid => PdfError::Invalid,
        })
    })
}

fn version(v: PdfVersion) -> &'static str {
    match v {
        PdfVersion::Pdf10 => "1.0",
        PdfVersion::Pdf11 => "1.1",
        PdfVersion::Pdf12 => "1.2",
        PdfVersion::Pdf13 => "1.3",
        PdfVersion::Pdf14 => "1.4",
        PdfVersion::Pdf15 => "1.5",
        PdfVersion::Pdf16 => "1.6",
        PdfVersion::Pdf17 => "1.7",
        PdfVersion::Pdf20 => "2.0",
    }
}

fn date(d: &DateTime) -> String {
    let mut s = format!("{:04}-{:02}-{:02}", d.year, d.month, d.day);
    if d.hour != 0 || d.minute != 0 {
        s.push_str(&format!(" {:02}:{:02}", d.hour, d.minute));
    }
    s
}

/// Geometry of one page.
pub(crate) fn geom(page: &Page<'_>) -> PageGeom {
    let (w_pt, h_pt) = page.render_dimensions();
    let rotation = match page.rotation() {
        Rotation::None => 0,
        Rotation::Horizontal => 90,
        Rotation::Flipped => 180,
        Rotation::FlippedHorizontal => 270,
    };
    PageGeom { w_pt, h_pt, rotation }
}

/// Every page's geometry.
pub(crate) fn geoms(pdf: &Pdf) -> Vec<PageGeom> {
    pdf.pages().iter().map(geom).collect()
}

/// Metadata of a loaded document (`bytes` = its file, for the encryption check).
pub(crate) fn info(pdf: &Pdf, bytes: &[u8]) -> DocInfo {
    let m = pdf.metadata();
    let s = |b: &Option<Box<[u8]>>| {
        b.as_deref().map(decode_pdf_string).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    };
    let pages = pdf.pages();
    DocInfo {
        pages: pages.len() as u32,
        version: Some(version(pdf.version()).to_string()),
        title: s(&m.title),
        author: s(&m.author),
        subject: s(&m.subject),
        keywords: s(&m.keywords),
        creator: s(&m.creator),
        producer: s(&m.producer),
        created: m.creation_date.as_ref().map(date),
        modified: m.modification_date.as_ref().map(date),
        // hayro does not expose the trailer; an `/Encrypt` key is what marks it.
        encrypted: memchr::memmem::find(bytes, b"/Encrypt").is_some(),
        first_page: pages.first().map(geom),
        engine: EngineKind::Hayro,
    }
}

/// Interpreter settings for `opts`: standard fonts from hayro's embedded set, missing
/// fonts from the system ([`fonts::find`]) when `opts.system_fonts`.
pub(crate) fn settings(opts: &RenderOpts) -> InterpreterSettings {
    let system = opts.system_fonts;
    let mut s = InterpreterSettings {
        font_resolver: Arc::new(move |q: &FontQuery| match q {
            FontQuery::Standard(f) => Some(f.get_font_data()),
            FontQuery::Fallback(f) => system
                .then(|| fonts::find(&want(f)))
                .flatten()
                .map(|(bytes, index)| (bytes as FontData, index))
                .or_else(|| Some(f.pick_standard_font().get_font_data())),
        }),
        ..Default::default()
    };
    if !opts.annotations {
        s.render_content = RenderContent::PAGE;
    }
    s
}

fn want(f: &FallbackFontQuery) -> FontWant {
    let mut names: Vec<String> = Vec::new();
    for n in [&f.font_family, &f.font_name, &f.post_script_name].into_iter().flatten() {
        // `MSGothic,Bold` → `MSGothic`.
        let base = n.split(',').next().unwrap_or(n).trim().to_string();
        if !base.is_empty() && !names.contains(&base) {
            names.push(base);
        }
    }
    let cjk = f.character_collection.as_ref().and_then(|cc| match &cc.family {
        CidFamily::AdobeJapan1 => Some(Cjk::Japanese),
        CidFamily::AdobeGB1 => Some(Cjk::SimplifiedChinese),
        CidFamily::AdobeCNS1 => Some(Cjk::TraditionalChinese),
        CidFamily::AdobeKorea1 => Some(Cjk::Korean),
        _ => None,
    });
    FontWant { names, cjk, bold: f.is_bold || f.font_weight >= 600, italic: f.is_italic, serif: f.is_serif }
}

/// The device transform for a region: page space → region pixels.
fn region_transform(page: &Page<'_>, r: &Region) -> Affine {
    let (w, h) = page.render_dimensions();
    let s = f64::from(r.scale);
    let (pw, ph) = (f64::from(w) * s, f64::from(h) * s);
    let rot = match r.rot_quarter % 4 {
        0 => Affine::IDENTITY,
        1 => Affine::new([0.0, 1.0, -1.0, 0.0, ph, 0.0]),
        2 => Affine::new([-1.0, 0.0, 0.0, -1.0, pw, ph]),
        _ => Affine::new([0.0, -1.0, 1.0, 0.0, 0.0, pw]),
    };
    Affine::translate((-f64::from(r.x), -f64::from(r.y)))
        * rot
        * Affine::scale(s)
        * page.initial_transform(true).to_kurbo()
}

/// Render one region of `page` on white. Each call interprets the whole page, so keep
/// regions large (`tiles::TILE`).
pub(crate) fn render_region<'a>(
    page: &'a Page<'a>,
    cache: &RenderCache<'a>,
    settings: &InterpreterSettings,
    r: &Region,
    max_side: u32,
) -> Result<Bitmap, PdfError> {
    if r.w == 0 || r.h == 0 || r.w > max_side || r.h > max_side || !r.scale.is_finite() || r.scale <= 0.0 {
        return Err(PdfError::Other(format!("bad region {}×{}", r.w, r.h)));
    }
    let (w, h) = (r.w as u16, r.h as u16);
    let mut ctx = RenderContext::new(w, h);
    hayro::render_into(page, cache, settings, &RenderSettings::default(), &mut ctx, region_transform(page, r));
    ctx.flush();
    let mut rgba = vec![0u8; r.w as usize * r.h as usize * 4];
    let target = PixmapMut::new(w, h, &mut rgba).ok_or_else(|| PdfError::Other("pixmap".into()))?;
    ctx.render_with(
        target,
        &mut Resources::default(),
        RasterizerSettings { target_init: TargetInit::Clear(WHITE), ..Default::default() },
    );
    Ok(Bitmap { w: r.w, h: r.h, rgba })
}

fn no_page(page: u32) -> PdfError {
    PdfError::Other(tr!("services-pdf-no-page", page = (page + 1).to_string(), error = "-"))
}

/// The whole page `page` fitted into `max_w` × `max_h` (view rotation `rot_quarter`).
pub(crate) fn render_fit(
    pdf: &Pdf,
    page: u32,
    max_w: u32,
    max_h: u32,
    rot_quarter: u8,
    opts: &RenderOpts,
) -> Result<Bitmap, PdfError> {
    let pg = pdf.pages().get(page as usize).ok_or_else(|| no_page(page))?;
    let cache = RenderCache::new();
    let mut bmp = render_fit_with(pg, &cache, &settings(opts), max_w, max_h, rot_quarter)?;
    if opts.invert {
        super::invert_lightness(&mut bmp.rgba);
    }
    Ok(bmp)
}

/// [`render_fit`] with the caller's caches.
pub(crate) fn render_fit_with<'a>(
    page: &'a Page<'a>,
    cache: &RenderCache<'a>,
    settings: &InterpreterSettings,
    max_w: u32,
    max_h: u32,
    rot_quarter: u8,
) -> Result<Bitmap, PdfError> {
    let g = geom(page);
    let scale = g.fit_scale(max_w, max_h, rot_quarter);
    let (w, h) = g.px(scale, rot_quarter);
    let (w, h) = (w.min(MAX_PAGE_PX), h.min(MAX_PAGE_PX));
    render_region(page, cache, settings, &Region { page: 0, scale, rot_quarter, x: 0, y: 0, w, h }, MAX_PAGE_PX)
}

/// Collects glyphs for [`PageText`]. Draws nothing (images are never decoded).
struct TextDevice {
    glyphs: Vec<RawGlyph>,
    /// Fill-and-stroke text reaches the device twice: (count, first, last origin) of the
    /// previous run, to skip the repeat.
    last_run: Option<(usize, Point, Point)>,
}

/// Ascender / descender used for every char box, in glyph units (1000 per em).
const ASCENT: f64 = 800.0;
const DESCENT: f64 = -200.0;

impl<'a> Device<'a> for TextDevice {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        let glyphs = run.glyphs();
        if glyphs.is_empty() {
            return;
        }
        let at = |g: &hayro::hayro_interpret::font::PositionedGlyph<'_>| props.transform * g.transform();
        let first = at(&glyphs[0]) * Point::ZERO;
        let last = at(&glyphs[glyphs.len() - 1]) * Point::ZERO;
        let sig = (glyphs.len(), first, last);
        if self.last_run == Some(sig) {
            return;
        }
        self.last_run = Some(sig);
        let pt = |p: Point| [p.x as f32, p.y as f32];
        for (i, g) in glyphs.iter().enumerate() {
            let Some(text) = g.as_unicode() else { continue };
            let text = match text {
                BfString::Char(c) => c.to_string(),
                BfString::String(s) => s,
            };
            // Private-use / control output carries no readable text.
            let text: String = text.chars().filter(|c| !c.is_control() || *c == '\t').collect();
            if text.is_empty() {
                continue;
            }
            let t = at(g);
            let origin = t * Point::ZERO;
            let up = t * Point::new(0.0, ASCENT) - origin;
            let down = t * Point::new(0.0, DESCENT) - origin;
            // The advance (PDF widths, 1000 per em); the glyph transform's hidden
            // horizontal stretch makes this approximate, so the next origin wins when it is
            // on the same baseline and not far away.
            let adv = match &**g {
                Glyph::Outline(o) => o.advance_width().map(f64::from),
                Glyph::Type3(_) => None,
            }
            .unwrap_or(500.0);
            let by_adv = t * Point::new(adv, 0.0);
            let end = glyphs
                .get(i + 1)
                .map(|n| at(n) * Point::ZERO)
                .filter(|n| {
                    let step = *n - origin;
                    let want = by_adv - origin;
                    let l = want.hypot().max(1e-6);
                    let along = step.dot(want) / l;
                    let across = (step - want * (along / l)).hypot();
                    along > 0.0 && along <= l * 1.5 + 0.1 * up.hypot() && across < 0.2 * up.hypot().max(1e-6)
                })
                .unwrap_or(by_adv);
            self.glyphs.push(RawGlyph {
                text,
                origin: pt(origin),
                end: pt(end),
                up: [up.x as f32, up.y as f32],
                down: [down.x as f32, down.y as f32],
            });
        }
    }
}

/// Text of one page, positioned in rendered page points.
pub(crate) fn text<'a>(page: &'a Page<'a>, settings: &InterpreterSettings) -> PageText {
    let (w, h) = page.render_dimensions();
    let mut s = settings.clone();
    s.render_content = RenderContent::PAGE;
    let cache = InterpreterCache::new();
    let mut ctx = Context::new(
        page.initial_transform(true).to_kurbo(),
        Rect::new(0.0, 0.0, f64::from(w), f64::from(h)),
        &cache,
        page.xref(),
        s,
    );
    let mut dev = TextDevice { glyphs: Vec::new(), last_run: None };
    interpret_page(page, &mut ctx, &mut dev);
    PageText::build(&dev.glyphs)
}

/// [`text`] of page `page` of `pdf`.
pub(crate) fn page_text(pdf: &Pdf, page: u32) -> Result<PageText, PdfError> {
    let pg = pdf.pages().get(page as usize).ok_or_else(|| no_page(page))?;
    Ok(text(pg, &settings(&RenderOpts::default())))
}

/// Plain text of the first `max_pages` pages (at most `max_chars` chars), reloading the
/// document every [`RECYCLE_PAGES`] pages to bound memory.
pub(crate) fn document_text(
    bytes: &Arc<[u8]>,
    password: Option<&str>,
    max_pages: u32,
    max_chars: usize,
) -> Result<String, PdfError> {
    let is = settings(&RenderOpts::default());
    let mut out = String::new();
    let mut pdf = load(bytes.clone(), password)?;
    let n = (pdf.pages().len() as u32).min(max_pages);
    for i in 0..n {
        if i > 0 && (i as usize).is_multiple_of(RECYCLE_PAGES) {
            pdf = load(bytes.clone(), password)?;
        }
        let Some(page) = pdf.pages().get(i as usize) else { break };
        out.push_str(&text(page, &is).text);
        out.push_str("\n\n");
        if out.len() > max_chars.saturating_mul(2) {
            break;
        }
    }
    Ok(clean_text_max(&out, max_chars))
}

#[cfg(test)]
mod tests {
    use super::super::{MAX_REGION_PX, fixtures};
    use super::*;

    fn dark(b: &Bitmap, x: u32, y: u32) -> bool {
        let i = ((y * b.w + x) * 4) as usize;
        b.rgba[i] < 128 && b.rgba[i + 1] < 128 && b.rgba[i + 2] < 128
    }

    fn load_mem(b: Vec<u8>) -> Pdf {
        load(Arc::from(b), None).unwrap()
    }

    #[test]
    fn letter_page_renders_on_white() {
        let pdf = load_mem(fixtures::text_pages());
        assert_eq!(geom(&pdf.pages()[0]), PageGeom { w_pt: 612.0, h_pt: 792.0, rotation: 0 });
        let page = &pdf.pages()[0];
        let cache = RenderCache::new();
        let r = Region { page: 0, scale: 1.0, rot_quarter: 0, x: 0, y: 0, w: 612, h: 792 };
        let b = render_region(page, &cache, &settings(&RenderOpts::default()), &r, MAX_PAGE_PX).unwrap();
        assert_eq!((b.w, b.h), (612, 792));
        assert_eq!(&b.rgba[0..4], &[255, 255, 255, 255], "opaque white corner, not transparent");
        let inked = b.rgba.as_chunks::<4>().0.iter().filter(|p| p[0] < 128).count();
        assert!(inked > 200, "text drew ink: {inked}");
        assert!(b.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255), "opaque everywhere");
    }

    #[test]
    fn region_equals_the_crop_of_a_full_render() {
        for (bytes, rot) in [
            (fixtures::text_pages(), 0u8),
            (fixtures::rotated(), 0),
            (fixtures::rotated(), 1),
            (fixtures::text_pages(), 3),
        ] {
            let pdf = load_mem(bytes);
            let page = &pdf.pages()[0];
            let is = settings(&RenderOpts::default());
            let cache = RenderCache::new();
            let g = geom(page);
            let (fw, fh) = g.px(1.5, rot);
            let full = render_region(
                page,
                &cache,
                &is,
                &Region { page: 0, scale: 1.5, rot_quarter: rot, x: 0, y: 0, w: fw, h: fh },
                MAX_PAGE_PX,
            )
            .unwrap();
            let (x, y, w, h) = (fw / 4, fh / 5, 300, 260);
            let part = render_region(
                page,
                &cache,
                &is,
                &Region { page: 0, scale: 1.5, rot_quarter: rot, x, y, w, h },
                MAX_REGION_PX,
            )
            .unwrap();
            let mut max = 0i32;
            for yy in 0..h {
                for xx in 0..w {
                    for c in 0..4 {
                        let a = full.rgba[(((y + yy) * fw + x + xx) * 4 + c) as usize] as i32;
                        let b = part.rgba[((yy * w + xx) * 4 + c) as usize] as i32;
                        max = max.max((a - b).abs());
                    }
                }
            }
            assert!(max <= 1, "rot {rot}: region differs from the full render by {max}");
        }
    }

    #[test]
    fn rotate_90_page_geometry() {
        let pdf = load_mem(fixtures::rotated());
        let page = &pdf.pages()[0];
        let g = geom(page);
        assert_eq!((g.w_pt, g.h_pt, g.rotation), (792.0, 612.0, 90));
        let b = render_fit(&pdf, 0, 792, 612, 0, &RenderOpts::default()).unwrap();
        assert_eq!((b.w, b.h), (792, 612));
        // The blue block at user (100..300, 600..700) on a /Rotate 90 page: user x → down,
        // user y → right from the left edge at y = 0 … rendered x = y_user, y = x_user.
        let px = |x: u32, y: u32| {
            let i = ((y * b.w + x) * 4) as usize;
            [b.rgba[i], b.rgba[i + 1], b.rgba[i + 2]]
        };
        assert_eq!(px(650, 200), [0, 0, 255], "inside the block");
        assert_eq!(px(150, 200), [255, 255, 255], "outside it");
        // View rotation on top: 90° more turns the 792 × 612 page back to portrait.
        let b2 = render_fit(&pdf, 0, 612, 792, 1, &RenderOpts::default()).unwrap();
        assert_eq!((b2.w, b2.h), (612, 792));
        // Inverted: white turns black.
        let inv = render_fit(&pdf, 0, 100, 100, 0, &RenderOpts { invert: true, ..Default::default() }).unwrap();
        assert_eq!(&inv.rgba[0..3], &[0, 0, 0]);
    }

    #[test]
    fn page_text_order_and_find_rects_cover_ink() {
        let pdf = load_mem(fixtures::text_pages());
        let t = page_text(&pdf, 0).unwrap();
        assert_eq!(t.text, "Hello world\nagain here, Hello", "{:?}", t.text);
        assert_eq!(t.lines.len(), 2);
        let m = super::super::text::find(&t, &super::super::Query { text: "hello".into(), ..Default::default() });
        assert_eq!(m.len(), 2);
        // Each match rect (in points, scale 1) holds dark pixels of the rendered page.
        let b = render_fit(&pdf, 0, 612, 792, 0, &RenderOpts::default()).unwrap();
        for hit in &m {
            let r = hit.rects[0];
            assert!(r[2] - r[0] > 40.0 && r[3] - r[1] > 15.0, "box size {r:?}");
            let mut ink = 0;
            for y in r[1].max(0.0) as u32..(r[3] as u32).min(b.h) {
                for x in r[0].max(0.0) as u32..(r[2] as u32).min(b.w) {
                    ink += usize::from(dark(&b, x, y));
                }
            }
            assert!(ink > 30, "match {r:?} covers ink ({ink})");
        }
        // Baseline of "Hello world" sits at y = 792 - 700 = 92.
        let c = t.chars[0].quad;
        assert!((c[1] - (92.0 + 24.0 * 0.2)).abs() < 0.5, "descent corner {c:?}");
        // Rotated page: text runs downwards; its boxes are inside the rotated page.
        let pdf = load_mem(fixtures::rotated());
        let t = page_text(&pdf, 0).unwrap();
        assert_eq!(t.text, "Rotated text");
        let r = t.chars[0].rect();
        assert!(r[0] >= 0.0 && r[2] <= 792.0 && r[1] >= 0.0 && r[3] <= 612.0, "{r:?}");
        assert!(t.chars[5].rect()[1] > t.chars[0].rect()[1], "later chars are lower: text runs down");
    }

    #[test]
    fn metadata_and_errors() {
        let bytes: Arc<[u8]> = Arc::from(fixtures::text_pages());
        let pdf = load(bytes.clone(), None).unwrap();
        let i = info(&pdf, &bytes);
        assert_eq!(i.pages, 3);
        assert_eq!(i.version.as_deref(), Some("1.7"));
        assert_eq!(i.author.as_deref(), Some("Ay"));
        assert!(!i.encrypted);
        assert_eq!(
            geoms(&load_mem(fixtures::mixed_sizes())).iter().map(|g| (g.w_pt, g.h_pt)).collect::<Vec<_>>(),
            vec![(612.0, 792.0), (842.0, 595.0), (300.0, 300.0)]
        );
        assert_eq!(load(Arc::from(&b"not a pdf"[..]), None).err().unwrap(), PdfError::Invalid);
        let enc: Arc<[u8]> = Arc::from(fixtures::encrypted("pw"));
        assert_eq!(load(enc.clone(), None).err().unwrap(), PdfError::Password { wrong: false });
        assert_eq!(load(enc.clone(), Some("bad")).err().unwrap(), PdfError::Password { wrong: true });
        assert!(load(enc, Some("pw")).is_ok());
        let r = Region { page: 0, scale: 1.0, rot_quarter: 0, x: 0, y: 0, w: 4000, h: 10 };
        let cache = RenderCache::new();
        let is = settings(&RenderOpts::default());
        assert!(render_region(&pdf.pages()[0], &cache, &is, &r, MAX_REGION_PX).is_err(), "over the region cap");
        assert!(page_text(&pdf, 7).is_err());
    }

    /// A non-embedded CJK font renders through an installed face (skipped without one).
    #[test]
    fn cjk_text_uses_a_system_font() {
        let want = FontWant { cjk: Some(Cjk::Japanese), ..Default::default() };
        if fonts::find(&want).is_none() {
            eprintln!("no Japanese system font installed; skipping");
            return;
        }
        let pdf = load_mem(fixtures::cjk_unembedded());
        let ink = |system_fonts| {
            let b = render_fit(&pdf, 0, 612, 792, 0, &RenderOpts { system_fonts, ..Default::default() }).unwrap();
            b.rgba.as_chunks::<4>().0.iter().filter(|p| p[0] < 128).count()
        };
        let (with, without) = (ink(true), ink(false));
        assert!(with > 500 && with > without, "glyphs drawn: {with} (without system fonts: {without})");
        let t = page_text(&pdf, 0).unwrap();
        assert!(t.text.contains("日本語"), "{:?}", t.text);
    }

    #[test]
    fn document_text_recycles() {
        let bytes: Arc<[u8]> = Arc::from(fixtures::many_pages(RECYCLE_PAGES + 6));
        let t = document_text(&bytes, None, 1000, 1_000_000).unwrap();
        assert!(t.contains("Page 0 of the sweep") && t.contains(&format!("Page {} of the sweep", RECYCLE_PAGES + 5)));
        let short = document_text(&bytes, None, 2, 1_000_000).unwrap();
        assert!(!short.contains("Page 2 "), "{short:?}");
    }
}
