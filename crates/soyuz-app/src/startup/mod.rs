//! CPU-rendered first frames (roadmap R5, `[performance] cpu_first_frame`, Windows).
//!
//! The GPU renderer needs ~260 ms before it can draw anything on the reference machine
//! (the D3D12 user-mode driver loads during adapter enumeration) and eframe creates the
//! window only after that. So `main` instead:
//!
//! 1. starts creating the wgpu instance / adapter / device on a worker thread,
//! 2. opens the startup placeholder (`services::splash`) at the real window's exact
//!    client rectangle,
//! 3. builds the egui context and the real [`SoyuzApp`] on the main thread and runs
//!    ordinary egui passes ([`run`]): the tessellated output is drawn by the CPU
//!    rasterizer in [`soft`] and shown in the placeholder (GDI). Pointer input over the
//!    placeholder is fed to egui, and further frames are drawn when egui asks for a
//!    repaint, until the GPU device is ready,
//! 4. hands the *same* context and app to eframe (`run_native_ext` with
//!    `WgpuSetup::Existing`), re-uploads every texture egui already handed out
//!    ([`Prebuilt::handover`]) and replays the viewport commands of the CPU passes. The
//!    placeholder closes once the GPU window has presented its first frame.
//!
//! During the CPU phase the window has no taskbar button, no keyboard focus, no DWM
//! shadow / rounded corners and no Mica / Acrylic backdrop (an opaque theme background
//! instead); in-process extra windows (F7) and scripted `SOYUZ_SCRIPT` steps start with
//! the GPU window.

pub mod soft;

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use egui::{Color32, Event, Pos2, Rect, ViewportCommand, ViewportId, ViewportInfo, pos2, vec2};

use crate::SoyuzApp;
use crate::devscript::startup_mark;

static CPU_PHASE: AtomicBool = AtomicBool::new(false);

/// Frames are being drawn on the CPU (the GPU window does not exist yet).
pub fn cpu_phase() -> bool {
    CPU_PHASE.load(Ordering::Relaxed)
}

/// The context and app built during the CPU phase, ready for eframe.
pub struct Prebuilt {
    pub ctx: egui::Context,
    pub app: SoyuzApp,
    textures: soft::Textures,
    commands: Vec<ViewportCommand>,
    /// CPU frames drawn.
    pub frames: u32,
}

impl Prebuilt {
    /// Call from eframe's app creator (the GPU window exists): attach the app to the
    /// native window, queue every live texture for upload and replay the viewport
    /// commands of the CPU passes.
    pub fn handover(mut self, cc: &eframe::CreationContext<'_>) -> SoyuzApp {
        CPU_PHASE.store(false, Ordering::Relaxed);
        self.app.attach_window(cc);
        let tex = self.ctx.tex_manager();
        let mut tex = tex.write();
        let mut bytes = 0;
        let deltas = self.textures.full_deltas();
        for (id, delta) in deltas.iter().cloned() {
            bytes += delta.image.width() * delta.image.height() * 4;
            tex.set(id, delta);
        }
        drop(tex);
        for c in self.commands {
            self.ctx.send_viewport_cmd(c);
        }
        log::debug!(
            "cpu-first: handed over {} textures ({:.1} MiB) after {} CPU frames",
            deltas.len(),
            bytes as f64 / 1048576.0,
            self.frames
        );
        self.app
    }
}

/// eframe's saved egui memory (`app.ron`, key `egui`), so the CPU frames use the saved
/// panel sizes etc. (eframe loads it only for contexts it creates itself).
fn saved_memory(app_ron: Option<&str>) -> Option<egui::Memory> {
    let map: std::collections::HashMap<String, String> = ron::from_str(app_ron?).ok()?;
    ron::from_str(map.get("egui")?).ok()
}

/// A fresh context set up the way eframe sets up its own.
pub fn context(app_ron: Option<&str>) -> egui::Context {
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    ctx.options_mut(|o| o.max_passes = NonZeroUsize::new(2).unwrap_or(NonZeroUsize::MIN));
    if let Some(mem) = saved_memory(app_ron) {
        ctx.memory_mut(|m| *m = mem);
    }
    ctx
}

/// Viewport commands that only make sense while a mouse button is held (or for the
/// placeholder itself) are not replayed on the GPU window.
fn replay(c: &ViewportCommand) -> bool {
    !matches!(c, ViewportCommand::StartDrag | ViewportCommand::BeginResize(_) | ViewportCommand::Screenshot(_))
}

/// The placeholder the CPU frames go to (the abstraction keeps [`run`] testable).
pub trait Surface {
    /// Client size in physical pixels.
    fn size(&self) -> (u32, u32);
    /// Monitor scale factor.
    fn scale(&self) -> f32;
    /// Show a BGRA frame; `painted` runs once it is on screen.
    fn present(&self, bgra: Vec<u8>, w: u32, h: u32, painted: Option<Box<dyn FnOnce() + Send>>);
    fn take_input(&self) -> Vec<soyuz_fs::splash::Input>;
}

impl Surface for soyuz_fs::splash::Splash {
    fn size(&self) -> (u32, u32) {
        soyuz_fs::splash::Splash::size(self)
    }
    fn scale(&self) -> f32 {
        soyuz_fs::splash::Splash::scale(self)
    }
    fn present(&self, bgra: Vec<u8>, w: u32, h: u32, painted: Option<Box<dyn FnOnce() + Send>>) {
        soyuz_fs::splash::Splash::present(self, bgra, w, h, painted);
    }
    fn take_input(&self) -> Vec<soyuz_fs::splash::Input> {
        soyuz_fs::splash::Splash::take_input(self)
    }
}

/// Options of one CPU phase.
pub struct Phase<'a> {
    pub surface: &'a dyn Surface,
    /// Whether the window restores maximized (reported to egui).
    pub maximized: bool,
    /// Background behind translucent panels (no system backdrop yet).
    pub clear: Color32,
    /// Blocks up to the given time; `true` once the GPU device is ready (or failed).
    pub gpu_ready: &'a dyn Fn(Duration) -> bool,
    /// Stop after this long even if the GPU is not ready (eframe then creates its own).
    pub max: Duration,
}

/// Run egui passes for `app` on the CPU until the GPU is ready (see the module docs).
pub fn run(ctx: egui::Context, mut app: SoyuzApp, phase: Phase<'_>) -> Prebuilt {
    CPU_PHASE.store(true, Ordering::Relaxed);
    let started = Instant::now();
    let (w, h) = phase.surface.size();
    let native = phase.surface.scale().max(0.5);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
    let mut canvas = soft::Canvas::new(w as usize, h as usize);
    let mut textures = soft::Textures::default();
    let mut commands = Vec::new();
    let mut frames = 0u32;
    let mut pointer: Option<Pos2> = None;
    let mut queued: Vec<soyuz_fs::splash::Input> = Vec::new();
    let system_theme = soyuz_fs::os_theme::apps_use_light_theme()
        .map(|light| if light { egui::Theme::Light } else { egui::Theme::Dark });
    loop {
        let ppp = native * ctx.zoom_factor();
        let size = vec2(w as f32, h as f32) / ppp;
        let mut events = Vec::new();
        queued.extend(phase.surface.take_input());
        for input in queued.drain(..) {
            use soyuz_fs::splash::Input;
            let at = |x: i32, y: i32| pos2(x as f32 / ppp, y as f32 / ppp);
            match input {
                Input::Move { x, y } => {
                    pointer = Some(at(x, y));
                    events.push(Event::PointerMoved(at(x, y)));
                }
                Input::Button { x, y, button, down } => {
                    pointer = Some(at(x, y));
                    let button = match button {
                        1 => egui::PointerButton::Secondary,
                        2 => egui::PointerButton::Middle,
                        _ => egui::PointerButton::Primary,
                    };
                    events.push(Event::PointerButton {
                        pos: at(x, y),
                        button,
                        pressed: down,
                        modifiers: Default::default(),
                    });
                    if frames > 0 && down {
                        startup_mark("cpu-first: input handled");
                    }
                }
                Input::Wheel { notches } if pointer.is_some() => events.push(Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: vec2(0.0, notches),
                    phase: egui::TouchPhase::Move,
                    modifiers: Default::default(),
                }),
                Input::Wheel { .. } => {}
            }
        }
        let rect = Rect::from_min_size(Pos2::ZERO, size);
        let info = ViewportInfo {
            native_pixels_per_point: Some(native),
            inner_rect: Some(rect),
            outer_rect: Some(rect),
            maximized: Some(phase.maximized),
            minimized: Some(false),
            fullscreen: Some(false),
            focused: Some(true),
            ..Default::default()
        };
        let raw = egui::RawInput {
            viewport_id: ViewportId::ROOT,
            viewports: std::iter::once((ViewportId::ROOT, info)).collect(),
            screen_rect: Some(rect),
            max_texture_side: Some(8192),
            time: Some(crate::devscript::since_start_ms() / 1000.0),
            predicted_dt: 1.0 / 60.0,
            events,
            focused: true,
            system_theme,
            ..Default::default()
        };
        let t = Instant::now();
        let out = ctx.run_ui(raw, |ui| app.frame(ui));
        let t_ui = t.elapsed();
        textures.set(&out.textures_delta);
        let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
        let t_tess = t.elapsed();
        soft::paint(&mut canvas, &prims, &textures, out.pixels_per_point, phase.clear, threads);
        textures.free(out.textures_delta);
        let t_paint = t.elapsed();
        let mut repaint = Duration::MAX;
        for (id, vo) in out.viewport_output {
            if id == ViewportId::ROOT {
                repaint = vo.repaint_delay;
                commands.extend(vo.commands.into_iter().filter(replay));
            }
        }
        let first = frames == 0;
        let painted: Option<Box<dyn FnOnce() + Send>> =
            first.then(|| Box::new(|| startup_mark("cpu-first: first UI frame presented")) as Box<dyn FnOnce() + Send>);
        phase.surface.present(canvas.bgra.clone(), w, h, painted);
        frames += 1;
        log::debug!(
            "cpu-first: frame {frames}: ui {:.1} ms, tessellate {:.1} ms, raster {:.1} ms ({} meshes, {w}x{h} px)",
            t_ui.as_secs_f64() * 1000.0,
            (t_tess - t_ui).as_secs_f64() * 1000.0,
            (t_paint - t_tess).as_secs_f64() * 1000.0,
            prims.len()
        );
        if first {
            startup_mark("cpu-first: first UI frame drawn");
        }
        // Debug hook: `SOYUZ_CPU_FRAME_DUMP=<file.png>` saves the last CPU frame.
        if let Ok(path) = std::env::var("SOYUZ_CPU_FRAME_DUMP") {
            dump(&canvas, std::path::Path::new(&path));
        }
        // Wait for the GPU, new input (next frame at once) or a requested repaint (at
        // most ~30 fps, so a frame in progress rarely delays the handover).
        let drawn = Instant::now();
        let done = loop {
            if (phase.gpu_ready)(Duration::from_millis(2)) || started.elapsed() > phase.max {
                break true;
            }
            let since = drawn.elapsed();
            queued.extend(phase.surface.take_input());
            if !queued.is_empty() || (since >= Duration::from_millis(33) && since >= repaint) {
                break false;
            }
        };
        if done {
            break;
        }
    }
    startup_mark("cpu-first: GPU ready, handing over");
    Prebuilt { ctx, app, textures, commands, frames }
}

/// Save `canvas` as an opaque RGB PNG (debug hook).
fn dump(canvas: &soft::Canvas, path: &std::path::Path) {
    let rgb: Vec<u8> = canvas.bgra.as_chunks::<4>().0.iter().flat_map(|p| [p[2], p[1], p[0]]).collect();
    match image::RgbImage::from_raw(canvas.w as u32, canvas.h as u32, rgb).map(|img| img.save(path)) {
        Some(Ok(())) => log::info!("cpu-first: saved {}", path.display()),
        other => log::warn!("cpu-first: cannot save {}: {other:?}", path.display()),
    }
}
