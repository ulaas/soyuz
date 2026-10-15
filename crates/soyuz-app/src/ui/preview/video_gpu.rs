//! The player's GPU colour conversion (Phase V4b, `[media] gpu_yuv`): video frames
//! arrive as YUV planes ([`YuvImage`], I420 or NV12, 8 or 16 bits) and are
//! drawn by an `egui_wgpu` paint callback. Each plane is uploaded to an integer texture
//! (`R8` / `RG8` / `R16` / `RG16` `Uint`), and a small WGSL shader (`video_gpu.wgsl`)
//! filters them, applies the frame's matrix (BT.601 / 709 / 2020) and range, tone-maps
//! PQ / HLG to SDR, and rotates as the stream says.
//!
//! Only with the wgpu renderer: [`init`] registers the pipeline in egui-wgpu's callback
//! resources when the app starts and records the device's texture limit; [`max_side`] is
//! 0 otherwise (glow renderer, tests), and the player keeps uploading RGBA textures.
//! Textures are kept per view (pane / Quick Look) and re-created only when the picture's
//! size or format changes; views not drawn for a few seconds free theirs.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use eframe::wgpu;
use soyuz_appcore::models::player::yuv::{self, Transfer, YuvImage, YuvLayout};

const SHADER: &str = include_str!("video_gpu.wgsl");
/// Size of the shader's `Params` block.
const PARAMS_BYTES: usize = 96;
/// A view's textures are freed when it has not been drawn for this long.
const IDLE: Duration = Duration::from_secs(5);

/// The device's largest texture side, set by [`init`] (absent: no GPU path).
static MAX_SIDE: OnceLock<u32> = OnceLock::new();

/// Register the pipeline with the wgpu renderer (once, at start-up). Nothing happens
/// with another renderer.
pub fn init(cc: &eframe::CreationContext<'_>) {
    let Some(rs) = cc.wgpu_render_state.as_ref() else { return };
    let pipe = Pipeline::new(&rs.device, rs.target_format);
    rs.renderer.write().callback_resources.insert(Resources { pipe, views: HashMap::new() });
    let _ = MAX_SIDE.set(rs.device.limits().max_texture_dimension_2d);
}

/// The largest picture side the GPU path takes; 0 when it is unavailable.
pub fn max_side() -> u32 {
    MAX_SIDE.get().copied().unwrap_or(0)
}

/// Draw `image` into `rect` (`view` tells the pane and Quick Look apart).
pub fn paint(painter: &egui::Painter, rect: egui::Rect, view: u64, image: Arc<YuvImage>) {
    painter.add(egui_wgpu::Callback::new_paint_callback(rect, YuvCallback { view, image }));
}

struct YuvCallback {
    view: u64,
    image: Arc<YuvImage>,
}

impl CallbackTrait for YuvCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(r) = resources.get_mut::<Resources>() else { return Vec::new() };
        let now = Instant::now();
        r.views.retain(|k, v| *k == self.view || now.duration_since(v.used) < IDLE);
        let planes = match r.views.get_mut(&self.view) {
            Some(p) if p.fits(&self.image) => p,
            _ => {
                let p = Planes::new(device, &r.pipe, &self.image);
                r.views.entry(self.view).insert_entry(p).into_mut()
            }
        };
        planes.used = now;
        planes.upload(queue, &self.image, r.pipe.linear_out);
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(r) = resources.get::<Resources>() else { return };
        let Some(p) = r.views.get(&self.view) else { return };
        pass.set_pipeline(&r.pipe.pipeline);
        pass.set_bind_group(0, &p.bind, &[]);
        pass.draw(0..6, 0..1);
    }
}

/// The pipeline plus every view's textures (egui-wgpu's callback resources).
struct Resources {
    pipe: Pipeline,
    views: HashMap<u64, Planes>,
}

struct Pipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// The target is an sRGB format: the shader writes linear values.
    linear_out: bool,
}

impl Pipeline {
    fn new(device: &wgpu::Device, target: wgpu::TextureFormat) -> Pipeline {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("soyuz_video_yuv"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Uint,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("soyuz_video_yuv"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(PARAMS_BYTES as u64),
                    },
                    count: None,
                },
                tex_entry(1),
                tex_entry(2),
                tex_entry(3),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("soyuz_video_yuv"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("soyuz_video_yuv"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        Pipeline { pipeline, layout, linear_out: target.is_srgb() }
    }
}

/// One view's textures, uniform buffer and bind group.
struct Planes {
    /// `(width, height, layout, bytes per sample)` they were made for.
    shape: (u32, u32, YuvLayout, u8),
    textures: Vec<wgpu::Texture>,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    /// The picture last uploaded (kept, so a new picture never shares its address).
    last: Option<Arc<YuvImage>>,
    used: Instant,
}

/// The integer texture format of plane `i`.
fn plane_format(img: &YuvImage, i: usize) -> wgpu::TextureFormat {
    let pair = i == 1 && img.layout == YuvLayout::Nv12;
    match (img.bytes, pair) {
        (2, true) => wgpu::TextureFormat::Rg16Uint,
        (2, false) => wgpu::TextureFormat::R16Uint,
        (_, true) => wgpu::TextureFormat::Rg8Uint,
        (_, false) => wgpu::TextureFormat::R8Uint,
    }
}

impl Planes {
    fn new(device: &wgpu::Device, pipe: &Pipeline, img: &YuvImage) -> Planes {
        let mut textures: Vec<wgpu::Texture> = (0..img.plane_count())
            .map(|i| {
                let (w, h) = img.plane_size(i);
                texture(device, w.max(1), h.max(1), plane_format(img, i))
            })
            .collect();
        if textures.len() < 3 {
            // NV12 has no third plane; the shader never reads it, but the slot is bound.
            textures.push(texture(device, 1, 1, wgpu::TextureFormat::R8Uint));
        }
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("soyuz_video_yuv_params"),
            size: PARAMS_BYTES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let views: Vec<wgpu::TextureView> =
            textures.iter().map(|t| t.create_view(&wgpu::TextureViewDescriptor::default())).collect();
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("soyuz_video_yuv"),
            layout: &pipe.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&views[0]) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&views[1]) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&views[2]) },
            ],
        });
        Planes {
            shape: (img.width, img.height, img.layout, img.bytes),
            textures,
            uniform,
            bind,
            last: None,
            used: Instant::now(),
        }
    }

    fn fits(&self, img: &YuvImage) -> bool {
        self.shape == (img.width, img.height, img.layout, img.bytes)
    }

    /// Upload the planes and the parameters of `img` unless it is already on the GPU.
    fn upload(&mut self, queue: &wgpu::Queue, img: &Arc<YuvImage>, linear_out: bool) {
        if self.last.as_ref().is_some_and(|l| Arc::ptr_eq(l, img)) {
            return;
        }
        for i in 0..img.plane_count() {
            let (w, h) = img.plane_size(i);
            let row = w * img.texel_bytes(i) as u32;
            if img.planes[i].len() < (row * h) as usize {
                return;
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.textures[i],
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &img.planes[i],
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
        }
        queue.write_buffer(&self.uniform, 0, &params(img, linear_out));
        self.last = Some(Arc::clone(img));
    }
}

fn texture(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("soyuz_video_plane"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// The shader's `Params` block for `img` (layout in `video_gpu.wgsl`).
fn params(img: &YuvImage, linear_out: bool) -> [u8; PARAMS_BYTES] {
    let c = img.conversion();
    let white = yuv::HDR_REFERENCE_WHITE;
    let peak = match c.transfer {
        Transfer::Pq => yuv::PQ_ASSUMED_PEAK / white,
        Transfer::Hlg => 1000.0 / white,
        Transfer::Sdr => 1.0,
    };
    let transfer = match c.transfer {
        Transfer::Sdr => 0u32,
        Transfer::Pq => 1,
        Transfer::Hlg => 2,
    };
    let rotation = (img.rotation.rem_euclid(360) / 90) as u32;
    let (cw, ch) = img.plane_size(1);
    let mut out = [0u8; PARAMS_BYTES];
    let mut at = 0;
    let mut put = |b: [u8; 4]| {
        out[at..at + 4].copy_from_slice(&b);
        at += 4;
    };
    for row in c.matrix {
        for v in row {
            put(v.to_le_bytes());
        }
    }
    put(c.scale.to_le_bytes());
    put(transfer.to_le_bytes());
    put(u32::from(c.gamut).to_le_bytes());
    put(u32::from(img.layout == YuvLayout::Nv12).to_le_bytes());
    put(rotation.to_le_bytes());
    put(u32::from(linear_out).to_le_bytes());
    put(peak.to_le_bytes());
    put(white.to_le_bytes());
    for v in [img.width as f32, img.height as f32, cw as f32, ch as f32] {
        put(v.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use soyuz_appcore::models::player::yuv::{YuvColor, YuvMatrix};

    fn image(layout: YuvLayout, bytes: u8, w: u32, h: u32, color: YuvColor, rotation: i32) -> YuvImage {
        YuvImage {
            width: w,
            height: h,
            layout,
            bytes,
            depth: if bytes == 2 { 10 } else { 8 },
            msb: false,
            planes: [Vec::new(), Vec::new(), Vec::new()],
            color,
            display: (w, h),
            rotation,
            pts: 0.0,
        }
    }

    #[test]
    fn shader_is_valid_wgsl() {
        let module = wgpu::naga::front::wgsl::parse_str(SHADER).expect("parses");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .expect("validates");
        assert!(module.entry_points.iter().any(|e| e.name == "vs_main"));
        assert!(module.entry_points.iter().any(|e| e.name == "fs_main"));
    }

    #[test]
    fn params_follow_the_shader_layout() {
        let color = YuvColor { matrix: YuvMatrix::Bt709, ..Default::default() };
        let img = image(YuvLayout::Nv12, 2, 64, 48, color, 270);
        let b = params(&img, true);
        let f = |at: usize| f32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let u = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let c = img.conversion();
        assert_eq!(f(0), c.matrix[0][0]);
        assert_eq!(f(44), c.matrix[2][3]);
        assert_eq!(f(48), 0.25, "10-bit scale");
        assert_eq!((u(52), u(56), u(60), u(64), u(68)), (0, 0, 1, 3, 1));
        assert_eq!(f(72), 1.0);
        assert_eq!(f(76), yuv::HDR_REFERENCE_WHITE);
        assert_eq!((f(80), f(84), f(88), f(92)), (64.0, 48.0, 32.0, 24.0));
        let pq = image(YuvLayout::I420, 1, 4, 4, YuvColor { transfer: Transfer::Pq, ..Default::default() }, 0);
        let b = params(&pq, false);
        let u = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        assert_eq!((u(52), u(60), u(64), u(68)), (1, 0, 0, 0));
        assert_eq!(plane_format(&img, 0), wgpu::TextureFormat::R16Uint);
        assert_eq!(plane_format(&img, 1), wgpu::TextureFormat::Rg16Uint);
        assert_eq!(plane_format(&pq, 2), wgpu::TextureFormat::R8Uint);
    }

    /// Poll a future to completion (wgpu's native futures are ready at once).
    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        let mut f = std::pin::pin!(f);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        loop {
            if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
            std::thread::yield_now();
        }
    }

    /// Render `img` offscreen into `w × h` RGBA (`None` without a GPU adapter).
    fn render(img: &Arc<YuvImage>, w: u32, h: u32) -> Option<Vec<u8>> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let pipe = Pipeline::new(&device, format);
        let mut planes = Planes::new(&device, &pipe, img);
        planes.upload(&queue, img, false);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let row = (w * 4).div_ceil(256) * 256;
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(row * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipe.pipeline);
            pass.set_bind_group(0, &planes.bind, &[]);
            pass.draw(0..6, 0..1);
        }
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        let data = buf.slice(..).get_mapped_range().ok()?;
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            out.extend_from_slice(&data[y * row as usize..y * row as usize + (w * 4) as usize]);
        }
        Some(out)
    }

    /// The shader against the CPU reference (`YuvImage::to_rgba`) on a GPU adapter
    /// (hardware, WARP or llvmpipe); skipped when the machine has none.
    #[test]
    fn gpu_conversion_matches_the_cpu_reference() {
        // 8 × 4 I420, BT.709 limited: a luma ramp over flat chroma (bilinear and nearest
        // chroma agree), a 2-pixel red block in 10-bit NV12, a PQ grey.
        let mut i420 = image(YuvLayout::I420, 1, 8, 4, YuvColor { matrix: YuvMatrix::Bt709, ..Default::default() }, 0);
        i420.planes[0] = (0..32).map(|i| 16 + (i % 8) as u8 * 31).collect();
        i420.planes[1] = vec![100; 8];
        i420.planes[2] = vec![170; 8];
        let mut nv12 = image(YuvLayout::Nv12, 2, 8, 4, YuvColor { matrix: YuvMatrix::Bt709, ..Default::default() }, 0);
        nv12.planes[0] = [252u16; 32].iter().flat_map(|v| v.to_le_bytes()).collect();
        nv12.planes[1] = [408u16, 960].repeat(8).iter().flat_map(|v| v.to_le_bytes()).collect();
        let pq_color = YuvColor {
            matrix: YuvMatrix::Bt2020,
            transfer: Transfer::Pq,
            bt2020_primaries: true,
            ..Default::default()
        };
        let mut pq = image(YuvLayout::I420, 2, 8, 4, pq_color, 0);
        pq.planes[0] = [573u16; 32].iter().flat_map(|v| v.to_le_bytes()).collect();
        pq.planes[1] = [512u16; 8].iter().flat_map(|v| v.to_le_bytes()).collect();
        pq.planes[2] = pq.planes[1].clone();
        for img in [i420, nv12, pq] {
            let want = img.to_rgba();
            let img = Arc::new(img);
            let Some(got) = render(&img, 8, 4) else {
                eprintln!("skipped: no GPU adapter");
                return;
            };
            // Rendered at the picture's size, every pixel samples a texel centre.
            for y in 0..4 {
                for x in 0..8 {
                    let at = (y * 8 + x) * 4;
                    for c in 0..3 {
                        let d = (i32::from(got[at + c]) - i32::from(want[at + c])).abs();
                        assert!(d <= 3, "{:?} {x},{y} c{c}: gpu {} cpu {}", img.layout, got[at + c], want[at + c]);
                    }
                }
            }
        }
    }

    #[test]
    fn rotated_pictures_turn_clockwise() {
        // 2 × 2 I420 full range: left column black, right column white.
        let mut img = image(YuvLayout::I420, 1, 2, 2, YuvColor { full_range: true, ..Default::default() }, 90);
        img.planes[0] = vec![0, 255, 0, 255];
        img.planes[1] = vec![128];
        img.planes[2] = vec![128];
        let img = Arc::new(img);
        let Some(got) = render(&img, 2, 2) else {
            eprintln!("skipped: no GPU adapter");
            return;
        };
        // A quarter turn clockwise puts the white column at the bottom.
        let px = |x: usize, y: usize| got[(y * 2 + x) * 4];
        assert!(px(0, 0) < 64 && px(1, 0) < 64, "{got:?}");
        assert!(px(0, 1) > 190 && px(1, 1) > 190, "{got:?}");
    }
}
