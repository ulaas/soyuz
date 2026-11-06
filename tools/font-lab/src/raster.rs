//! Turning one [`Spec`] into pixels: the real egui pipeline rendered offscreen, or a custom
//! coverage path for what egui has no mode for.

use std::sync::Arc;

use eframe::{egui_wgpu, wgpu};
use egui::epaint::text::{FontTweak, HintingTarget, LcdOrder, VariationCoords};
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, ColorImage, FontData, FontDefinitions, FontFamily, FontId, Rect, pos2, vec2};
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, HintingInstance, OutlinePen, Target};
use vello_cpu::kurbo::{Affine, BezPath, Stroke};

use crate::spec::{FontEntry, GAMMAS, HINTS, Kind, Spec, TECHS, THEMES, transfer};

pub const PAD: u32 = 6;

/// What a worker needs from the window's GPU device.
pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: egui_wgpu::Renderer,
}

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl Gpu {
    /// A device of its own on the window's adapter: work on the window's device makes its
    /// surface reconfigure fail ("Failed to wait for GPU to come idle"), and samples are
    /// read back to the CPU anyway.
    pub fn new(adapter: &wgpu::Adapter) -> Result<Self, String> {
        let dual_source = adapter.features() & wgpu::Features::DUAL_SOURCE_BLENDING;
        let desc = wgpu::DeviceDescriptor {
            label: Some("font-lab worker"),
            required_features: dual_source,
            ..Default::default()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&desc)).map_err(|e| e.to_string())?;
        // Options as in eframe's own painter: no MSAA, dithering on (the Soyuz window's defaults).
        let renderer = egui_wgpu::Renderer::new(&device, FORMAT, egui_wgpu::RendererOptions::default());
        Ok(Self { device, queue, renderer })
    }
}

pub fn line_height(spec: &Spec) -> u32 {
    (spec.px() * 1.4).ceil() as u32
}

/// Draw one sample on a `w` × `h` pixel canvas.
pub fn render(
    gpu: &mut Gpu,
    spec: &Spec,
    fonts: &[FontEntry],
    text: &str,
    w: u32,
    h: u32,
) -> Result<ColorImage, String> {
    match TECHS[spec.tech].kind {
        Kind::Stock | Kind::StockLcd => stock(gpu, spec, fonts, text, w, h),
        Kind::Gray { .. } | Kind::Lcd { .. } => custom(spec, fonts, text, w, h),
        Kind::DirectWrite { mode, measuring, gray, linear } => {
            directwrite(spec, fonts, text, w, h, mode, measuring, gray, linear)
        }
    }
}

fn coords_of(spec: &Spec, font: &FontEntry) -> Vec<([u8; 4], f32)> {
    let mut c: Vec<([u8; 4], f32)> = font.coords.iter().map(|(t, v)| (**t, *v)).collect();
    if spec.wght != 0 {
        c.push((*b"wght", f32::from(spec.wght)));
    }
    c
}

/// A private egui context for `spec`, and the layout job of `text`.
fn setup(spec: &Spec, fonts: &[FontEntry], text: &str) -> (egui::Context, LayoutJob) {
    let font = &fonts[spec.font];
    let hint = &HINTS[spec.hint];
    let tweak = |coords: &[([u8; 4], f32)]| {
        let mut t = FontTweak {
            hinting: Some(hint.target.is_some()),
            hinting_target: match (hint.target.unwrap_or_default(), TECHS[spec.tech].kind) {
                (HintingTarget::Smooth(smooth), Kind::StockLcd) => HintingTarget::SmoothLcd(smooth, LcdOrder::Rgb),
                (target, _) => target,
            },
            subpixel_binning: Some(spec.bin),
            ..FontTweak::default()
        };
        if !coords.is_empty() {
            let pairs: Vec<(&[u8; 4], f32)> = coords.iter().map(|(t, v)| (t, *v)).collect();
            t.coords = VariationCoords::new(pairs);
        }
        t
    };
    let mut defs = FontDefinitions::empty();
    defs.font_data.insert("v".into(), Arc::new(FontData::from_static(font.bytes).tweak(tweak(&coords_of(spec, font)))));
    // What a partial font leaves out (Inter Tabular has no letters).
    defs.font_data.insert("fb".into(), Arc::new(FontData::from_static(fonts[0].bytes).tweak(tweak(&[]))));
    let list = vec!["v".to_string(), "fb".to_string()];
    defs.families.insert(FontFamily::Proportional, list.clone());
    defs.families.insert(FontFamily::Monospace, list);

    let ctx = egui::Context::default();
    ctx.set_fonts(defs);
    let curve = match GAMMAS[spec.gamma].1 {
        Some(g) => egui::epaint::FontColorTransferFunction::Gamma(g),
        None => egui::epaint::FontColorTransferFunction::TwoCoverageMinusCoverageSq,
    };
    ctx.all_styles_mut(|s| s.visuals.text_options.color_transfer_function = curve);
    let theme = &THEMES[spec.theme];
    ctx.set_theme(if theme.dark { egui::Theme::Dark } else { egui::Theme::Light });

    let ppp = f32::from(spec.scale) / 100.0;
    let fg = Color32::from_rgb(theme.fg[0], theme.fg[1], theme.fg[2]);
    let size_pt = crate::spec::SIZES[spec.size];
    let lh = line_height(spec) as f32 / ppp;
    let mut job = LayoutJob::default();
    job.append(
        text,
        0.0,
        TextFormat { font_id: FontId::proportional(size_pt), color: fg, line_height: Some(lh), ..Default::default() },
    );
    (ctx, job)
}

fn input(ppp: f32, screen: Rect) -> egui::RawInput {
    let mut raw = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
    raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
    raw
}

/// The pixel row of each line's baseline, from egui's own layout, so every technique
/// sits on the same lines as the real pipeline.
fn baselines(spec: &Spec, fonts: &[FontEntry], text: &str) -> Vec<f32> {
    let (ctx, job) = setup(spec, fonts, text);
    let ppp = f32::from(spec.scale) / 100.0;
    let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(4000.0, 4000.0));
    let mut out = Vec::new();
    let _ = ctx.run_pass(input(ppp, screen), |ctx| {
        let galley = ctx.fonts_mut(|f| f.layout_job(job.clone()));
        out = galley
            .rows
            .iter()
            .map(|row| {
                let y = row.glyphs.first().map_or(0.0, |g| g.pos.y);
                ((row.pos.y + y) * ppp + PAD as f32).round()
            })
            .collect();
    });
    out
}

/// The real thing: a private egui context with its own atlas (the coverage curve applies to a
/// whole atlas), tessellated and drawn by `egui_wgpu::Renderer`, exactly the calls eframe makes.
fn stock(gpu: &mut Gpu, spec: &Spec, fonts: &[FontEntry], text: &str, w: u32, h: u32) -> Result<ColorImage, String> {
    let (ctx, job) = setup(spec, fonts, text);
    let theme = &THEMES[spec.theme];
    let ppp = f32::from(spec.scale) / 100.0;
    let bg = Color32::from_rgb(theme.bg[0], theme.bg[1], theme.bg[2]);
    let fg = Color32::from_rgb(theme.fg[0], theme.fg[1], theme.fg[2]);
    let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(w as f32 / ppp, h as f32 / ppp));
    let origin = pos2(PAD as f32 / ppp, PAD as f32 / ppp);

    let mut deltas = Vec::new();
    let mut last = None;
    for _ in 0..2 {
        let job = job.clone();
        let out = ctx.run_pass(input(ppp, screen), move |ctx| {
            let painter = ctx.layer_painter(egui::LayerId::background());
            painter.rect_filled(screen, 0.0, bg);
            let galley = ctx.fonts_mut(|f| f.layout_job(job.clone()));
            painter.galley(origin, galley, fg);
        });
        deltas.push(out.textures_delta.clone());
        last = Some(out);
    }
    let out = last.ok_or("no pass")?;
    let prims = ctx.tessellate(out.shapes, out.pixels_per_point);

    let (device, queue) = (gpu.device.clone(), gpu.queue.clone());
    for d in &deltas {
        for (id, set) in &d.set {
            for delta in set {
                gpu.renderer.update_texture(&device, &queue, *id, delta);
            }
        }
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("font-lab cell"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let screen_desc = egui_wgpu::ScreenDescriptor { size_in_pixels: [w, h], pixels_per_point: out.pixels_per_point };
    let mut encoder = device.create_command_encoder(&Default::default());
    let extra = gpu.renderer.update_buffers(&device, &queue, &mut encoder, &prims, &screen_desc);
    {
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("font-lab cell"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        let mut pass = pass.forget_lifetime();
        gpu.renderer.render(&mut pass, &prims, &screen_desc);
    }
    let stride = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("font-lab readback"),
        size: u64::from(stride * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: Some(h) },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit(extra.into_iter().chain(std::iter::once(encoder.finish())));
    let (tx, rx) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    let data = buffer.slice(..).get_mapped_range().map_err(|e| e.to_string())?;
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for row in data.chunks_exact(stride as usize) {
        for px in row[..(w * 4) as usize].chunks_exact(4) {
            pixels.push(Color32::from_rgb(px[0], px[1], px[2]));
        }
    }
    drop(data);
    buffer.unmap();
    texture.destroy();
    Ok(ColorImage::new([w as usize, h as usize], pixels))
}

/// One positioned glyph of a line.
struct Placed {
    id: u32,
    x: f32,
    y: f32,
}

/// Shape with harfrust, as epaint does: `(glyph, advance, x offset, y offset)` in pixels.
fn shape(font: &FontEntry, spec: &Spec, line: &str) -> Result<Vec<(u32, f32, f32, f32)>, String> {
    let fref = skrifa::FontRef::new(font.bytes).map_err(|e| e.to_string())?;
    let data = harfrust::ShaperData::new(&fref);
    let vars: Vec<harfrust::Variation> = coords_of(spec, font)
        .into_iter()
        .map(|(t, value)| harfrust::Variation { tag: harfrust::Tag::new(&t), value })
        .collect();
    let instance = (!vars.is_empty()).then(|| harfrust::ShaperInstance::from_variations(&fref, vars));
    let shaper = data.shaper(&fref).instance(instance.as_ref()).build();
    let mut buffer = harfrust::UnicodeBuffer::new();
    buffer.push_str(line);
    buffer.guess_segment_properties();
    let glyphs = shaper.shape(buffer, harfrust::ShapeOptions::new());
    let upem = f32::from(fref.metrics(Size::unscaled(), LocationRef::default()).units_per_em);
    let k = spec.px() / upem;
    Ok(glyphs
        .glyph_infos()
        .iter()
        .zip(glyphs.glyph_positions())
        .map(|(i, p)| (i.glyph_id, p.x_advance as f32 * k, p.x_offset as f32 * k, -(p.y_offset as f32) * k))
        .collect())
}

fn layout(font: &FontEntry, spec: &Spec, text: &str, bases: &[f32]) -> Result<Vec<Placed>, String> {
    let mut out = Vec::new();
    for (line, &y) in text.lines().zip(bases) {
        let mut cursor = PAD as f32;
        for (id, adv, dx, dy) in shape(font, spec, line)? {
            let x = cursor + dx;
            // Whole pixels, or epaint's four sub-pixel bins.
            let x = if spec.bin { (x * 4.0).round() / 4.0 } else { x.round() };
            out.push(Placed { id, x, y: (y + dy).round() });
            cursor += adv;
        }
    }
    Ok(out)
}

struct Pen<'a> {
    path: &'a mut BezPath,
    x: f64,
    y: f64,
}

impl OutlinePen for Pen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to((f64::from(x) + self.x, self.y - f64::from(y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to((f64::from(x) + self.x, self.y - f64::from(y)));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.path.quad_to(
            (f64::from(cx0) + self.x, self.y - f64::from(cy0)),
            (f64::from(x) + self.x, self.y - f64::from(y)),
        );
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.path.curve_to(
            (f64::from(cx0) + self.x, self.y - f64::from(cy0)),
            (f64::from(cx1) + self.x, self.y - f64::from(cy1)),
            (f64::from(x) + self.x, self.y - f64::from(y)),
        );
    }
    fn close(&mut self) {
        self.path.close_path();
    }
}

/// The whole text as one outline path, with epaint's hinting.
fn outline(font: &FontEntry, spec: &Spec, text: &str, bases: &[f32]) -> Result<BezPath, String> {
    let fref = skrifa::FontRef::new(font.bytes).map_err(|e| e.to_string())?;
    let glyphs = fref.outline_glyphs();
    let settings: Vec<(skrifa::Tag, f32)> =
        coords_of(spec, font).into_iter().map(|(t, v)| (skrifa::Tag::new(&t), v)).collect();
    let location = fref.axes().location(settings.iter().copied());
    let loc: LocationRef<'_> = (&location).into();
    let size = Size::new(spec.px());
    let mut hinting = match HINTS[spec.hint].target {
        Some(t) => {
            let target: Target = t.into();
            Some(HintingInstance::new(&glyphs, size, loc, target).map_err(|e| e.to_string())?)
        }
        None => None,
    };
    let mut path = BezPath::new();
    for g in layout(font, spec, text, bases)? {
        let Some(glyph) = glyphs.get(skrifa::GlyphId::new(g.id)) else { continue };
        let mut pen = Pen { path: &mut path, x: f64::from(g.x), y: f64::from(g.y) };
        let settings = match &mut hinting {
            Some(h) => DrawSettings::hinted(h, false),
            None => DrawSettings::unhinted(size, loc),
        };
        glyph.draw(settings, &mut pen).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

/// Coverage of `path` on a canvas `sx` times wider than `w`, by epaint's rasterizer.
fn coverage(path: &BezPath, w: u32, h: u32, sx: u32, embolden: f32) -> Vec<u8> {
    let (cw, ch) = ((w * sx) as u16, h as u16);
    let mut ctx = vello_cpu::RenderContext::new(cw, ch);
    ctx.set_transform(Affine::scale_non_uniform(f64::from(sx), 1.0));
    ctx.set_paint(vello_cpu::color::OpaqueColor::<vello_cpu::color::Srgb>::WHITE);
    ctx.fill_path(path);
    if embolden > 0.0 {
        // A stroke of width 2e grows every edge by e.
        ctx.set_stroke(Stroke::new(f64::from(embolden) * 2.0));
        ctx.stroke_path(path);
    }
    let mut pix = vello_cpu::Pixmap::new(cw, ch);
    let mut res = vello_cpu::Resources::new();
    ctx.render(&mut pix, &mut res);
    pix.data_as_u8_slice().chunks_exact(4).map(|p| p[3]).collect()
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}
fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// One channel of `fg` over `bg`, `a` opaque: in gamma space like egui, or in linear light.
fn mix(bg: u8, fg: u8, a: f32, linear: bool) -> u8 {
    let (b, f) = (f32::from(bg) / 255.0, f32::from(fg) / 255.0);
    let v = if linear {
        linear_to_srgb(srgb_to_linear(b) + (srgb_to_linear(f) - srgb_to_linear(b)) * a)
    } else {
        b + (f - b) * a
    };
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

fn compose_gray(spec: &Spec, cov: &[f32], w: u32, h: u32, linear: bool) -> ColorImage {
    let t = &THEMES[spec.theme];
    let g = GAMMAS[spec.gamma].1;
    let pixels = cov
        .iter()
        .map(|&c| {
            let a = transfer(g, c);
            Color32::from_rgb(
                mix(t.bg[0], t.fg[0], a, linear),
                mix(t.bg[1], t.fg[1], a, linear),
                mix(t.bg[2], t.fg[2], a, linear),
            )
        })
        .collect();
    ColorImage::new([w as usize, h as usize], pixels)
}

/// `cov` holds three coverages per pixel, in panel order red, green, blue.
fn compose_rgb(spec: &Spec, cov: &[f32], w: u32, h: u32, linear: bool) -> ColorImage {
    let t = &THEMES[spec.theme];
    let g = GAMMAS[spec.gamma].1;
    let pixels = cov
        .chunks_exact(3)
        .map(|c| {
            Color32::from_rgb(
                mix(t.bg[0], t.fg[0], transfer(g, c[0]), linear),
                mix(t.bg[1], t.fg[1], transfer(g, c[1]), linear),
                mix(t.bg[2], t.fg[2], transfer(g, c[2]), linear),
            )
        })
        .collect();
    ColorImage::new([w as usize, h as usize], pixels)
}

fn custom(spec: &Spec, fonts: &[FontEntry], text: &str, w: u32, h: u32) -> Result<ColorImage, String> {
    let font = &fonts[spec.font];
    let path = outline(font, spec, text, &baselines(spec, fonts, text))?;
    match TECHS[spec.tech].kind {
        Kind::Gray { linear, embolden, sharpen } => {
            let raw = coverage(&path, w, h, 1, embolden);
            let mut cov: Vec<f32> = raw.iter().map(|&c| f32::from(c) / 255.0).collect();
            if sharpen > 0.0 {
                let src = cov.clone();
                for (y, row) in cov.chunks_exact_mut(w as usize).enumerate() {
                    let s = &src[y * w as usize..(y + 1) * w as usize];
                    for x in 1..w as usize - 1 {
                        row[x] = (s[x] * (1.0 + 2.0 * sharpen) - (s[x - 1] + s[x + 1]) * sharpen).clamp(0.0, 1.0);
                    }
                }
            }
            Ok(compose_gray(spec, &cov, w, h, linear))
        }
        Kind::Lcd { fir, bgr, linear } => {
            let sub = coverage(&path, w, h, 3, 0.0);
            let sw = (w * 3) as usize;
            let mut cov = vec![0.0f32; sw * h as usize];
            for y in 0..h as usize {
                let row = &sub[y * sw..(y + 1) * sw];
                for x in 0..sw {
                    let mut v = 0.0;
                    for (k, weight) in fir.iter().enumerate() {
                        let i = x as i64 + k as i64 - 2;
                        if (0..sw as i64).contains(&i) {
                            v += f32::from(row[i as usize]) / 255.0 * weight;
                        }
                    }
                    cov[y * sw + x] = v.min(1.0);
                }
            }
            if bgr {
                for px in cov.chunks_exact_mut(3) {
                    px.swap(0, 2);
                }
            }
            Ok(compose_rgb(spec, &cov, w, h, linear))
        }
        _ => Err("not a custom technique".into()),
    }
}

#[allow(clippy::too_many_arguments)]
fn directwrite(
    spec: &Spec,
    fonts: &[FontEntry],
    text: &str,
    w: u32,
    h: u32,
    mode: i32,
    measuring: i32,
    gray: bool,
    linear: bool,
) -> Result<ColorImage, String> {
    #[cfg(windows)]
    {
        let font = &fonts[spec.font];
        let mut lines = Vec::new();
        for (line, base) in text.lines().zip(baselines(spec, fonts, text)) {
            lines.push((base, shape(font, spec, line)?));
        }
        let path = font.path.as_ref().ok_or("no font file")?;
        let cov = crate::dwrite::draw(path, spec.px(), &lines, PAD as f32, w, h, mode, measuring)
            .map_err(|e| e.to_string())?;
        if gray {
            let avg: Vec<f32> = cov.chunks_exact(3).map(|c| (c[0] + c[1] + c[2]) / 3.0).collect();
            Ok(compose_gray(spec, &avg, w, h, linear))
        } else {
            Ok(compose_rgb(spec, &cov, w, h, linear))
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (spec, fonts, text, w, h, mode, measuring, gray, linear);
        Err("DirectWrite exists only on Windows".into())
    }
}
