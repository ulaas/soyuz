//! Soyuz — a keyboard-first, non-blocking file explorer.
#![forbid(unsafe_code)]
// Release builds on Windows are GUI-subsystem binaries (no console window).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::process::ExitCode;

/// `println!` / `eprintln!` that never panic: a GUI-subsystem binary may have no stream,
/// and a caller may close its pipe early (a panic there would write a crash report).
macro_rules! out {
    ($($t:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), $($t)*);
    }};
}
macro_rules! err {
    ($($t:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($t)*);
    }};
}

/// Renderer requested on the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RendererChoice {
    Wgpu,
    Glow,
}

#[derive(Debug, PartialEq, Eq)]
enum Cli {
    Run {
        renderer: RendererChoice,
        startup: soyuz_app::app::Startup,
        /// Paths after the first: opened as extra tabs once the window is up.
        extra: Vec<std::path::PathBuf>,
        /// `--dbus-file-manager`: own `org.freedesktop.FileManager1` regardless of
        /// `[linux] own_file_manager1` (the opt-in D-Bus activation file passes it).
        dbus_file_manager: bool,
    },
    /// `--tui [PATH]`: the terminal UI in this terminal.
    Tui {
        startup: soyuz_app::app::Startup,
        /// `--screen-reader`: screen-reader mode for this run only.
        screen_reader: bool,
    },
    Help,
    Version,
}

const USAGE: &str = "\
Usage: soyuz [OPTIONS] [PATH]...
       soyuz --tui [--safe-mode] [--screen-reader] [PATH]
       soyuz-tui [--safe-mode] [--screen-reader] [PATH]   (the terminal UI as its own console program)
       soyuz <cp|mv|rm|trash|ls|open|reveal|jobs> …   (run `soyuz cp --help`)

Arguments:
  [PATH]...               Folders to open, as tabs (a file opens its folder with the file
                          selected); `file://` URIs are accepted

Options:
  --tui                   Run the terminal UI (in this terminal; on Windows a release build
                          opens a console window of its own: run `soyuz-tui` to stay in
                          the terminal); opens at most one PATH
  --screen-reader         With --tui: screen-reader mode for this run (the setting stays)
  --renderer <wgpu|glow>  Rendering backend (default: wgpu; glow needs the `glow` cargo feature)
  --new-window            A separate Soyuz process, even when one already runs (`[workspace]
                          launch`); it doesn't restore or overwrite the saved session
  --safe-mode             Default settings; no session restore, folder watching, thumbnails or git
  --dbus-file-manager     Linux: serve org.freedesktop.FileManager1 (\"show in folder\" from other apps)
  --mcp                   Run as an MCP server on stdin/stdout (attaches to the running window)
  --mcp-client <NAME>     With --mcp: the client's name in Settings → AI assistant → Connections
  --mcp-headless          With --mcp: never attach to a running window
  -h, --help              Print help
  -V, --version           Print version";

/// A command-line path: `file://` URIs (desktop files pass `%U`) are decoded.
fn arg_path(arg: String) -> std::path::PathBuf {
    if arg.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("file:"))
        && let Some(p) = soyuz_fs::launch::path_from_file_uri(&arg)
    {
        return p;
    }
    std::path::PathBuf::from(arg)
}

/// Why this GUI start must not hand over to a running window (`None`: try the handoff).
/// `has_env` tells whether an environment variable is set; `mode` reads `[workspace] launch`
/// (only when nothing else decided).
fn handoff_bypass(
    args: &[String],
    startup: &soyuz_app::app::Startup,
    dbus_file_manager: bool,
    has_env: impl Fn(&str) -> bool,
    mode: impl FnOnce() -> soyuz_core::config::LaunchMode,
) -> Option<&'static str> {
    let opts: Vec<&str> = args.iter().map(String::as_str).take_while(|a| *a != "--").collect();
    if startup.secondary {
        return Some("--new-window");
    }
    if startup.safe_mode {
        return Some("--safe-mode");
    }
    if opts.iter().any(|a| *a == "--renderer" || a.starts_with("--renderer=")) {
        return Some("--renderer");
    }
    if dbus_file_manager {
        return Some("--dbus-file-manager");
    }
    if !has_env(soyuz_app::services::instance::TEST_ENV) {
        if has_env("SOYUZ_SCRIPT") {
            return Some("scripted run (SOYUZ_SCRIPT)");
        }
        if has_env("SOYUZ_WINDOW_POS") {
            return Some("scripted run (SOYUZ_WINDOW_POS)");
        }
    }
    if mode() == soyuz_core::config::LaunchMode::NewProcess {
        return Some("[workspace] launch = \"new_process\"");
    }
    None
}

/// `--tui` is on the command line (before a `--`): decided before logging starts, so
/// the terminal UI logs to a file instead of the screen.
fn tui_requested(args: &[String]) -> bool {
    args.iter().take_while(|a| a.as_str() != "--").any(|a| a == "--tui")
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Cli, String> {
    let mut renderer = RendererChoice::Wgpu;
    let mut renderer_given = false;
    let mut tui = false;
    let mut screen_reader = false;
    let mut startup = soyuz_app::app::Startup::default();
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    let mut dbus_file_manager = false;
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let value = if arg == "--renderer" {
            Some(it.next().ok_or("--renderer needs a value (wgpu or glow)")?)
        } else {
            arg.strip_prefix("--renderer=").map(str::to_string)
        };
        if let Some(v) = value {
            renderer_given = true;
            renderer = match v.to_ascii_lowercase().as_str() {
                "wgpu" => RendererChoice::Wgpu,
                "glow" | "gl" | "opengl" => RendererChoice::Glow,
                other => return Err(format!("unknown renderer '{other}' (expected wgpu or glow)")),
            };
            continue;
        }
        match arg.as_str() {
            "-h" | "--help" => return Ok(Cli::Help),
            "-V" | "--version" => return Ok(Cli::Version),
            "--new-window" => startup.secondary = true,
            "--tui" => tui = true,
            "--screen-reader" => screen_reader = true,
            "--safe-mode" => startup.safe_mode = true,
            "--dbus-file-manager" => dbus_file_manager = true,
            "--" => {
                let rest: Vec<String> = it.by_ref().collect();
                if rest.is_empty() {
                    return Err("`--` needs a path after it".into());
                }
                paths.extend(rest.into_iter().map(arg_path));
            }
            other if other.starts_with('-') => return Err(format!("unexpected argument '{other}'")),
            _ => paths.push(arg_path(arg)),
        }
    }
    if tui {
        if renderer_given {
            return Err("--renderer cannot be combined with --tui".into());
        }
        if dbus_file_manager {
            return Err("--dbus-file-manager cannot be combined with --tui".into());
        }
        if paths.len() > 1 {
            return Err("--tui opens one folder (got several paths)".into());
        }
        startup.open = paths.into_iter().next();
        return Ok(Cli::Tui { startup, screen_reader });
    }
    if screen_reader {
        return Err("--screen-reader needs --tui (the GUI follows the system's screen reader)".into());
    }
    let mut paths = paths.into_iter();
    startup.open = paths.next();
    Ok(Cli::Run { renderer, startup, extra: paths.collect(), dbus_file_manager })
}

/// Map the CLI choice onto an eframe renderer; returns the renderer and its display name.
fn resolve_renderer(choice: RendererChoice) -> (eframe::Renderer, &'static str) {
    match choice {
        RendererChoice::Wgpu => (eframe::Renderer::Wgpu, "wgpu"),
        #[cfg(feature = "glow")]
        RendererChoice::Glow => (eframe::Renderer::Glow, "glow"),
        #[cfg(not(feature = "glow"))]
        RendererChoice::Glow => {
            err!(
                "soyuz: this build has no OpenGL renderer; rebuild with `cargo build --features glow`. \
                 Falling back to wgpu."
            );
            (eframe::Renderer::Wgpu, "wgpu")
        }
    }
}

/// Rank of an adapter type for a power preference (lower = better); software last.
fn adapter_rank(t: eframe::wgpu::DeviceType, power: eframe::wgpu::PowerPreference) -> u8 {
    use eframe::wgpu::{DeviceType, PowerPreference};
    match (t, power) {
        (DeviceType::DiscreteGpu, PowerPreference::LowPower) => 1,
        (DeviceType::IntegratedGpu, PowerPreference::LowPower) => 0,
        (DeviceType::DiscreteGpu, _) => 0,
        (DeviceType::IntegratedGpu, _) => 1,
        (DeviceType::VirtualGpu, _) => 2,
        (DeviceType::Other, _) => 3,
        (DeviceType::Cpu, _) => 4,
    }
}

/// Shader-visible CBV / SRV / UAV descriptors under `[performance] gpu_memory = "low"`
/// (Direct3D 12 sizes that heap from this limit: wgpu's default of 1 000 000 commits
/// ~30 MB; 131 072 is ~4 MB, far above the one bind group per egui texture we use).
const LOW_MEMORY_BINDINGS: u32 = 131_072;

/// `[performance] gpu_memory = "low"`: start the allocator with small blocks instead of
/// egui-wgpu's `MemoryHints::Performance` (128 MB device + 64 MB host reserved up front,
/// ~3 MB of it used by an idle window) and cap the descriptor heap. Lowering a required
/// limit never exceeds what the adapter supports.
fn low_gpu_memory(d: &mut eframe::wgpu::DeviceDescriptor<'static>, mode: soyuz_core::config::GpuMemory) {
    if mode == soyuz_core::config::GpuMemory::Low {
        d.memory_hints = eframe::wgpu::MemoryHints::MemoryUsage;
        d.required_limits.max_non_sampler_bindings =
            d.required_limits.max_non_sampler_bindings.min(LOW_MEMORY_BINDINGS);
    }
}

/// The adapter to render with: the one named by `WGPU_ADAPTER_NAME` (substring, any case),
/// else the best type for `power` that can present to the window.
fn pick_adapter(
    adapters: &[eframe::wgpu::Adapter],
    surface: Option<&eframe::wgpu::Surface<'_>>,
    power: eframe::wgpu::PowerPreference,
) -> Result<eframe::wgpu::Adapter, String> {
    let usable: Vec<&eframe::wgpu::Adapter> =
        adapters.iter().filter(|a| surface.is_none_or(|s| a.is_surface_supported(s))).collect();
    let infos: Vec<(String, eframe::wgpu::DeviceType)> =
        usable.iter().map(|a| a.get_info()).map(|i| (i.name, i.device_type)).collect();
    let wanted = std::env::var("WGPU_ADAPTER_NAME").ok().filter(|s| !s.trim().is_empty());
    match choose_adapter(&infos, wanted.as_deref(), power) {
        Some(i) => Ok(usable[i].clone()),
        None => Err(format!(
            "no usable GPU adapter (found: {})",
            adapters.iter().map(|a| a.get_info().name).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// Index into `infos` (`(name, type)`) of the adapter to use.
fn choose_adapter(
    infos: &[(String, eframe::wgpu::DeviceType)],
    wanted: Option<&str>,
    power: eframe::wgpu::PowerPreference,
) -> Option<usize> {
    if let Some(w) = wanted {
        let w = w.trim().to_lowercase();
        if let Some(i) = infos.iter().position(|(n, _)| n.to_lowercase().contains(&w)) {
            return Some(i);
        }
        log::warn!("WGPU_ADAPTER_NAME={w}: no such adapter; choosing automatically");
    }
    (0..infos.len()).min_by_key(|&i| adapter_rank(infos[i].1, power))
}

/// `[linux] window_backend`: force winit's X11 or Wayland backend (`auto` lets winit
/// pick Wayland when `WAYLAND_DISPLAY` is set). Falls back to the other backend when the
/// forced one has no display.
#[cfg(target_os = "linux")]
fn with_window_backend(
    mut options: eframe::NativeOptions,
    backend: soyuz_core::config::WindowBackend,
) -> eframe::NativeOptions {
    use soyuz_core::config::WindowBackend;
    use winit::platform::{wayland::EventLoopBuilderExtWayland, x11::EventLoopBuilderExtX11};
    let has = |var: &str| std::env::var_os(var).is_some_and(|v| !v.is_empty());
    let wayland = has("WAYLAND_DISPLAY") || has("WAYLAND_SOCKET");
    let x11 = has("DISPLAY");
    match backend {
        WindowBackend::X11 if x11 => {
            log::info!("window: X11 backend ([linux] window_backend = \"x11\")");
            options.event_loop_builder = Some(Box::new(|b| {
                b.with_x11();
            }));
        }
        WindowBackend::Wayland if wayland => {
            log::info!("window: Wayland backend ([linux] window_backend = \"wayland\")");
            options.event_loop_builder = Some(Box::new(|b| {
                b.with_wayland();
            }));
        }
        WindowBackend::Auto => {}
        other => log::warn!("window: [linux] window_backend = {other:?} has no display; choosing automatically"),
    }
    options
}

/// `soyuz --tui`: the terminal UI. Returns before any window / GPU code runs. The
/// session is the one `soyuz-tui` runs (`soyuz_tui::launch`); here a GUI-subsystem
/// release build on Windows opens a console window of its own.
#[cfg(feature = "tui")]
fn run_tui(startup: soyuz_app::app::Startup, screen_reader: bool) -> ExitCode {
    soyuz_tui::launch::launch_with(startup, soyuz_tui::launch::RunOpts { screen_reader })
}

/// Logging for `--tui`: a capped log file and the crash-report panic hook.
#[cfg(feature = "tui")]
fn init_tui_logging() {
    soyuz_tui::launch::init_logging();
}

#[cfg(not(feature = "tui"))]
fn init_tui_logging() {
    soyuz_app::services::crash::init_logging();
    soyuz_app::services::crash::install_panic_hook();
}

#[cfg(not(feature = "tui"))]
fn run_tui(_startup: soyuz_app::app::Startup, _screen_reader: bool) -> ExitCode {
    soyuz_fs::console::attach_parent();
    err!("soyuz: built without the terminal UI (cargo feature `tui`)");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    soyuz_app::devscript::mark_process_start();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if tui_requested(&args) {
        // The screen belongs to the terminal UI: log to a capped file instead of stderr
        // (`soyuz-tui.log`, shared with the `soyuz-tui` binary).
        init_tui_logging();
    } else {
        soyuz_app::services::crash::init_logging();
        soyuz_app::services::crash::install_panic_hook();
    }

    // `soyuz cp|mv|rm|trash|ls|open|reveal|jobs …`: the CLI companion; no GUI.
    let cwd = std::env::current_dir().unwrap_or_default();
    match soyuz_script::cli::parse(&args, &cwd) {
        Ok(Some(cmd)) => {
            soyuz_fs::console::attach_parent();
            return ExitCode::from(soyuz_script::cli::run(cmd).clamp(0, 255) as u8);
        }
        Ok(None) => {}
        Err(e) => {
            soyuz_fs::console::attach_parent();
            err!(
                "soyuz: {e}

Run `soyuz <command> --help` for usage."
            );
            return ExitCode::from(soyuz_script::cli::exit::USAGE as u8);
        }
    }
    // `soyuz --mcp …`: MCP over stdio (attach to the running window or headless); no GUI.
    match soyuz_mcp::cli::parse(&args) {
        Ok(Some(opts)) => return ExitCode::from(soyuz_mcp::cli::run(opts).clamp(0, 255) as u8),
        Ok(None) => {}
        Err(e) => {
            err!(
                "soyuz: {e}

{USAGE}"
            );
            return ExitCode::from(2);
        }
    }
    let args_for_bypass = args.clone();
    let cli = match parse_args(args) {
        Ok(c) => c,
        Err(e) => {
            err!("soyuz: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let renderer = match cli {
        Cli::Help => {
            soyuz_fs::console::attach_parent();
            out!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Cli::Version => {
            soyuz_fs::console::attach_parent();
            out!("soyuz {} ({})", env!("CARGO_PKG_VERSION"), soyuz_app::GIT_SHA);
            return ExitCode::SUCCESS;
        }
        Cli::Tui { startup, screen_reader } => return run_tui(startup, screen_reader),
        Cli::Run { renderer, startup, extra, dbus_file_manager } => (renderer, startup, extra, dbus_file_manager),
    };
    let (renderer, mut startup, extra, dbus_file_manager) = renderer;
    // Single instance: hand the launch to a running window, before anything here starts
    // threads, claims a crash-journal slot or writes the config.
    let has_env = |k: &str| std::env::var_os(k).is_some();
    let mode = soyuz_app::services::instance::configured_mode;
    match handoff_bypass(&args_for_bypass, &startup, dbus_file_manager, has_env, mode) {
        Some(why) => log::info!("instance: {why}; not handing over"),
        None => {
            use soyuz_app::services::instance;
            let req = instance::LaunchRequest::from_args(startup.open.as_deref(), &extra);
            let name = instance::endpoint_name();
            match instance::claim_or_hand_off(&name, &req) {
                instance::Handoff::Handed => {
                    log::info!("instance: handed the launch to the running window");
                    return ExitCode::SUCCESS;
                }
                instance::Handoff::Primary(l) => {
                    log::info!("instance: serving {name}");
                    instance::install(l);
                }
                instance::Handoff::Own(why) => {
                    log::warn!("instance: {why}; starting a separate window that leaves the session alone");
                    startup.secondary = true;
                }
            }
        }
    }
    soyuz_app::services::filemanager1::set_forced(dbus_file_manager);
    // Further command-line paths open as tabs once the window runs.
    soyuz_app::services::filemanager1::open_paths_later(extra);
    let (renderer, renderer_name) = resolve_renderer(renderer);
    // Before the job engine exists: claim the crash-journal slot, collect what to recover.
    soyuz_app::services::crash::startup(startup.safe_mode);
    let config =
        if startup.safe_mode { soyuz_app::services::crash::safe_config() } else { soyuz_app::app::load_config() };
    soyuz_app::devscript::startup_mark("config loaded");
    let chrome = soyuz_app::ui::titlebar::Chrome::for_config(config.appearance.custom_titlebar);
    // Windows 11 system backdrop: a transparent window presented through a
    // DirectComposition visual (DX12), read once at startup.
    let backdrop = soyuz_app::ui::native_chrome::startup_backdrop(&config).is_some();
    soyuz_app::ui::native_chrome::set_backdrop_active(backdrop);
    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        // Windows: DX12 only unless `WGPU_BACKEND` says otherwise. Initializing the Vulkan
        // loader (and any overlay layers injected into it) costs ~250 ms of startup.
        // `[performance] gpu_backend` picks another API (next start).
        if std::env::var_os("WGPU_BACKEND").is_none() {
            use soyuz_core::config::GpuBackend;
            match config.performance.gpu_backend {
                GpuBackend::Auto if cfg!(windows) => setup.instance_descriptor.backends = eframe::wgpu::Backends::DX12,
                GpuBackend::Auto => {}
                GpuBackend::Dx12 => setup.instance_descriptor.backends = eframe::wgpu::Backends::DX12,
                GpuBackend::Vulkan => setup.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN,
                GpuBackend::Gl => setup.instance_descriptor.backends = eframe::wgpu::Backends::GL,
            }
        }
        if backdrop {
            setup.instance_descriptor.backends = eframe::wgpu::Backends::DX12;
            setup.instance_descriptor.backend_options.dx12.presentation_system =
                eframe::wgpu::Dx12SwapchainKind::DxgiFromVisual;
        }
        // Pick from the adapters egui-wgpu already enumerated instead of letting
        // `request_adapter` enumerate them all again, honouring `WGPU_ADAPTER_NAME`
        // (CI forces the WARP software adapter with it) and `WGPU_POWER_PREF`.
        let power = setup.power_preference;
        setup.native_adapter_selector = Some(std::sync::Arc::new(move |adapters, surface| {
            soyuz_app::devscript::startup_mark("adapters enumerated");
            pick_adapter(adapters, surface, power)
        }));
        let device_descriptor = setup.device_descriptor.clone();
        let gpu_memory = config.performance.gpu_memory;
        setup.device_descriptor = std::sync::Arc::new(move |adapter| {
            soyuz_app::devscript::startup_mark("adapter chosen");
            let mut d = device_descriptor(adapter);
            low_gpu_memory(&mut d, gpu_memory);
            d
        });
    }
    // Debug hook (screenshot sessions): `SOYUZ_WINDOW_POS=x,y` opens the window at that
    // spot without activating it, e.g. off-screen so scripted captures do not flash.
    let dev_pos = std::env::var("SOYUZ_WINDOW_POS").ok().and_then(|s| {
        let (x, y) = s.split_once(',')?;
        Some(egui::pos2(x.trim().parse().ok()?, y.trim().parse().ok()?))
    });

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Soyuz")
        .with_app_id("soyuz")
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([720.0, 440.0])
        .with_icon(soyuz_app::app_icon::icon_data(128))
        .with_transparent(backdrop);
    if let Some(pos) = dev_pos {
        viewport = viewport.with_position(pos).with_active(false);
    }
    let options = eframe::NativeOptions {
        viewport: soyuz_app::ui::titlebar::configure_viewport(viewport, chrome),
        renderer,
        wgpu_options,
        centered: dev_pos.is_none(),
        // Scripted / off-screen runs neither restore nor save the window geometry: a run
        // at -4000,-4000 would save a clamped on-screen position the next run restores.
        persist_window: dev_pos.is_none(),
        // eframe's own state (window geometry, egui memory) lives with the rest of the
        // configuration, so portable mode keeps it next to the executable as well.
        persistence_path: Some(soyuz_core::config::config_dir().join("app.ron")),
        // Windows: a window saved maximized is created restored and maximized while still
        // hidden (`native_chrome::set_restore_maximized`), so startup shows one window
        // once instead of winit's show / hide flashes.
        window_builder: Some(Box::new(move |builder| {
            if cfg!(windows) && builder.maximized == Some(true) {
                soyuz_app::ui::native_chrome::set_restore_maximized(dev_pos.is_none());
                builder.with_maximized(false)
            } else {
                builder
            }
        })),
        ..Default::default()
    };
    #[cfg(target_os = "linux")]
    let options = with_window_backend(options, config.linux.window_backend);
    let result = eframe::run_native(
        "Soyuz",
        options,
        Box::new(move |cc| {
            soyuz_app::devscript::startup_mark("window + renderer ready");
            let app = soyuz_app::SoyuzApp::new(cc, config, renderer_name, startup);
            soyuz_app::devscript::startup_mark("app state ready");
            Ok(Box::new(app))
        }),
    );
    // Normally gone already (quitting closes it): never hand a launch to a dying process.
    soyuz_app::services::instance::shutdown();
    // "Install & restart" after an in-place update: start the new binary.
    if let Some(exe) = soyuz_app::services::updates::take_restart() {
        let e = soyuz_app::services::updates::exec_restart(&exe);
        log::error!("soyuz: restart failed: {e}");
        err!("soyuz: restart failed: {e}");
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("soyuz: {e}");
            err!("soyuz: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_choice() {
        use eframe::wgpu::{DeviceType, PowerPreference};
        let l = |v: &[(&str, DeviceType)]| v.iter().map(|(n, t)| (n.to_string(), *t)).collect::<Vec<_>>();
        let set = l(&[
            ("Microsoft Basic Render Driver", DeviceType::Cpu),
            ("Intel UHD", DeviceType::IntegratedGpu),
            ("NVIDIA RTX", DeviceType::DiscreteGpu),
        ]);
        assert_eq!(choose_adapter(&set, None, PowerPreference::HighPerformance), Some(2));
        assert_eq!(choose_adapter(&set, None, PowerPreference::LowPower), Some(1));
        assert_eq!(choose_adapter(&set, Some("basic render"), PowerPreference::HighPerformance), Some(0));
        assert_eq!(choose_adapter(&set, Some("missing"), PowerPreference::None), Some(2));
        let warp_only = l(&[("Microsoft Basic Render Driver", DeviceType::Cpu)]);
        assert_eq!(choose_adapter(&warp_only, None, PowerPreference::HighPerformance), Some(0));
        assert_eq!(choose_adapter(&[], None, PowerPreference::HighPerformance), None);
    }

    fn parse(v: &[&str]) -> Result<Cli, String> {
        parse_args(v.iter().map(|s| s.to_string()))
    }

    #[test]
    fn cli_parsing() {
        let run = |renderer| {
            Ok(Cli::Run { renderer, startup: Default::default(), extra: Vec::new(), dbus_file_manager: false })
        };
        assert_eq!(parse(&[]), run(RendererChoice::Wgpu));
        assert_eq!(parse(&["--renderer", "glow"]), run(RendererChoice::Glow));
        assert_eq!(parse(&["--renderer=WGPU"]), run(RendererChoice::Wgpu));
        let Ok(Cli::Run { startup, .. }) = parse(&["--new-window", "C:\\data"]) else { panic!("parse") };
        assert!(startup.secondary);
        assert_eq!(startup.open, Some(std::path::PathBuf::from("C:\\data")));
        let Ok(Cli::Run { startup, .. }) = parse(&["--", "-dash"]) else { panic!("parse") };
        assert_eq!(startup.open, Some(std::path::PathBuf::from("-dash")));
        let Ok(Cli::Run { startup, extra, .. }) = parse(&["a", "b", "c"]) else { panic!("parse") };
        assert_eq!(startup.open, Some(std::path::PathBuf::from("a")));
        assert_eq!(extra, vec![std::path::PathBuf::from("b"), std::path::PathBuf::from("c")]);
        let Ok(Cli::Run { dbus_file_manager, .. }) = parse(&["--dbus-file-manager"]) else { panic!("parse") };
        assert!(dbus_file_manager);
        assert_eq!(parse(&["--help"]), Ok(Cli::Help));
        assert_eq!(parse(&["-V"]), Ok(Cli::Version));
        assert!(parse(&["--renderer"]).is_err());
        assert!(parse(&["--renderer", "vulkan"]).is_err());
        assert!(parse(&["--bogus"]).is_err());
        let Ok(Cli::Run { startup, .. }) = parse(&["--safe-mode"]) else { panic!("parse") };
        assert!(startup.safe_mode && !startup.secondary);
    }

    #[test]
    fn tui_parsing() {
        let tui = |open: Option<&str>| {
            Ok(Cli::Tui {
                startup: soyuz_app::app::Startup { open: open.map(std::path::PathBuf::from), ..Default::default() },
                screen_reader: false,
            })
        };
        assert_eq!(parse(&["--tui"]), tui(None));
        assert_eq!(parse(&["--tui", "dir"]), tui(Some("dir")));
        assert_eq!(parse(&["dir", "--tui"]), tui(Some("dir")));
        assert_eq!(parse(&["--tui", "--", "-dash"]), tui(Some("-dash")));
        let Ok(Cli::Tui { startup, .. }) = parse(&["--tui", "--safe-mode"]) else { panic!("parse") };
        assert!(startup.safe_mode);
        let Ok(Cli::Tui { screen_reader, .. }) = parse(&["--tui", "--screen-reader"]) else { panic!("parse") };
        assert!(screen_reader);
        assert!(parse(&["--screen-reader"]).is_err());
        // After `--` it is a path: the GUI opens a folder named `--tui`.
        let Ok(Cli::Run { startup, .. }) = parse(&["--", "--tui"]) else { panic!("parse") };
        assert_eq!(startup.open, Some(std::path::PathBuf::from("--tui")));
        assert!(parse(&["--tui", "a", "b"]).is_err());
        assert!(parse(&["--tui", "--renderer", "glow"]).is_err());
        assert!(parse(&["--tui", "--dbus-file-manager"]).is_err());
        assert_eq!(parse(&["--tui", "--help"]), Ok(Cli::Help));
    }

    #[test]
    fn handoff_bypass_flags() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        use soyuz_core::config::LaunchMode;
        let no_env = |_: &str| false;
        let window = || LaunchMode::NewWindow;
        let plain = soyuz_app::app::Startup::default();
        let secondary = soyuz_app::app::Startup { secondary: true, ..Default::default() };
        let safe = soyuz_app::app::Startup { safe_mode: true, ..Default::default() };
        assert_eq!(handoff_bypass(&a(&["--new-window"]), &secondary, false, no_env, window), Some("--new-window"));
        assert_eq!(handoff_bypass(&a(&["--safe-mode"]), &safe, false, no_env, window), Some("--safe-mode"));
        assert_eq!(handoff_bypass(&a(&["--renderer", "glow"]), &plain, false, no_env, window), Some("--renderer"));
        assert_eq!(handoff_bypass(&a(&["--renderer=wgpu"]), &plain, false, no_env, window), Some("--renderer"));
        assert_eq!(handoff_bypass(&a(&[]), &plain, true, no_env, window), Some("--dbus-file-manager"));
        let script = |k: &str| k == "SOYUZ_SCRIPT";
        assert_eq!(handoff_bypass(&a(&[]), &plain, false, script, window), Some("scripted run (SOYUZ_SCRIPT)"));
        let pos = |k: &str| k == "SOYUZ_WINDOW_POS";
        assert_eq!(handoff_bypass(&a(&[]), &plain, false, pos, window), Some("scripted run (SOYUZ_WINDOW_POS)"));
        // The dev hook allows the handoff in scripted runs.
        let test = |k: &str| k == "SOYUZ_WINDOW_POS" || k == soyuz_app::services::instance::TEST_ENV;
        assert_eq!(handoff_bypass(&a(&[]), &plain, false, test, window), None);
        assert_eq!(handoff_bypass(&a(&["dir"]), &plain, false, no_env, || LaunchMode::NewTab), None);
        assert!(handoff_bypass(&a(&[]), &plain, false, no_env, || LaunchMode::NewProcess).is_some());
        // A folder named `--renderer` after `--` is a path, not the flag.
        assert_eq!(handoff_bypass(&a(&["--", "--renderer"]), &plain, false, no_env, window), None);
    }

    #[test]
    fn tui_is_detected_before_logging() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(tui_requested(&a(&["--tui"])));
        assert!(tui_requested(&a(&["--safe-mode", "--tui", "x"])));
        assert!(!tui_requested(&a(&["--", "--tui"])));
        assert!(!tui_requested(&a(&["x"])));
    }

    #[test]
    fn file_uris_on_the_command_line() {
        let Ok(Cli::Run { startup, extra, .. }) = parse(&["file:///tmp/a%20b", "FILE:///x"]) else { panic!("parse") };
        if cfg!(windows) {
            assert!(startup.open.is_some());
        } else {
            assert_eq!(startup.open, Some(std::path::PathBuf::from("/tmp/a b")));
            assert_eq!(extra, vec![std::path::PathBuf::from("/x")]);
        }
        // Not a file URI: kept as a path.
        let Ok(Cli::Run { startup, .. }) = parse(&["sftp://host/x"]) else { panic!("parse") };
        assert_eq!(startup.open, Some(std::path::PathBuf::from("sftp://host/x")));
    }

    #[test]
    fn icon_is_square_rgba() {
        let i = soyuz_app::app_icon::icon_data(32);
        assert_eq!(i.rgba.len(), (i.width * i.height * 4) as usize);
        assert!(i.rgba.chunks(4).any(|px| px[3] == 255));
    }
}
