//! Soyuz — a keyboard-first, non-blocking file explorer.
#![forbid(unsafe_code)]
// Release builds on Windows are GUI-subsystem binaries (no console window).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::process::ExitCode;

/// Renderer requested on the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RendererChoice {
    Wgpu,
    Glow,
}

#[derive(Debug, PartialEq, Eq)]
enum Cli {
    Run { renderer: RendererChoice, startup: soyuz_app::app::Startup },
    Help,
    Version,
}

const USAGE: &str = "\
Usage: soyuz [OPTIONS] [PATH]
       soyuz <cp|mv|rm|trash|ls|open|reveal|jobs> …   (run `soyuz cp --help`)

Arguments:
  [PATH]                  Folder to open (a file opens its folder with the file selected)

Options:
  --renderer <wgpu|glow>  Rendering backend (default: wgpu; glow needs the `glow` cargo feature)
  --new-window            Secondary window: don't restore or overwrite the saved session
  --safe-mode             Default settings; no session restore, folder watching, thumbnails or git
  --mcp                   Run as an MCP server on stdin/stdout (attaches to the running window)
  --mcp-client <NAME>     With --mcp: the client's name in Settings → AI assistant → Connections
  --mcp-headless          With --mcp: never attach to a running window
  -h, --help              Print help
  -V, --version           Print version";

fn set_path(startup: &mut soyuz_app::app::Startup, path: String) -> Result<(), String> {
    if startup.open.is_some() {
        return Err(format!("unexpected extra path '{path}'"));
    }
    startup.open = Some(std::path::PathBuf::from(path));
    Ok(())
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Cli, String> {
    let mut renderer = RendererChoice::Wgpu;
    let mut startup = soyuz_app::app::Startup::default();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let value = if arg == "--renderer" {
            Some(it.next().ok_or("--renderer needs a value (wgpu or glow)")?)
        } else {
            arg.strip_prefix("--renderer=").map(str::to_string)
        };
        if let Some(v) = value {
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
            "--safe-mode" => startup.safe_mode = true,
            "--" => {
                let path = it.next().ok_or("`--` needs a path after it")?;
                set_path(&mut startup, path)?;
            }
            other if other.starts_with('-') => return Err(format!("unexpected argument '{other}'")),
            _ => set_path(&mut startup, arg)?,
        }
    }
    Ok(Cli::Run { renderer, startup })
}

/// Map the CLI choice onto an eframe renderer; returns the renderer and its display name.
fn resolve_renderer(choice: RendererChoice) -> (eframe::Renderer, &'static str) {
    match choice {
        RendererChoice::Wgpu => (eframe::Renderer::Wgpu, "wgpu"),
        #[cfg(feature = "glow")]
        RendererChoice::Glow => (eframe::Renderer::Glow, "glow"),
        #[cfg(not(feature = "glow"))]
        RendererChoice::Glow => {
            eprintln!(
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

fn main() -> ExitCode {
    soyuz_app::devscript::mark_process_start();
    soyuz_app::services::crash::init_logging();
    soyuz_app::services::crash::install_panic_hook();

    let args: Vec<String> = std::env::args().skip(1).collect();
    // `soyuz cp|mv|rm|trash|ls|open|reveal|jobs …`: the CLI companion (F6); no GUI.
    let cwd = std::env::current_dir().unwrap_or_default();
    match soyuz_script::cli::parse(&args, &cwd) {
        Ok(Some(cmd)) => {
            soyuz_fs::console::attach_parent();
            return ExitCode::from(soyuz_script::cli::run(cmd).clamp(0, 255) as u8);
        }
        Ok(None) => {}
        Err(e) => {
            soyuz_fs::console::attach_parent();
            eprintln!(
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
            eprintln!(
                "soyuz: {e}

{USAGE}"
            );
            return ExitCode::from(2);
        }
    }
    let cli = match parse_args(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("soyuz: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let renderer = match cli {
        Cli::Help => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Cli::Version => {
            println!("soyuz {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Cli::Run { renderer, startup } => (renderer, startup),
    };
    let (renderer, startup) = renderer;
    let (renderer, renderer_name) = resolve_renderer(renderer);
    // Before the job engine exists: claim the crash-journal slot, collect what to recover.
    soyuz_app::services::crash::startup(startup.safe_mode);
    let config =
        if startup.safe_mode { soyuz_app::services::crash::safe_config() } else { soyuz_app::app::load_config() };
    soyuz_app::devscript::startup_mark("config loaded");
    // Windows: a native placeholder window right away while the GPU renderer starts (R5).
    soyuz_app::services::splash::open(&config, startup.secondary, (1280.0, 800.0));
    let chrome = soyuz_app::ui::titlebar::Chrome::for_config(config.appearance.custom_titlebar);
    // Windows 11 system backdrop: a transparent window presented through a
    // DirectComposition visual (DX12), read once at startup.
    let backdrop = soyuz_app::ui::native_chrome::startup_backdrop(&config).is_some();
    soyuz_app::ui::native_chrome::set_backdrop_active(backdrop);
    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup {
        // Windows: DX12 only unless `WGPU_BACKEND` says otherwise. Initializing the Vulkan
        // loader (and any overlay layers injected into it) costs ~250 ms of startup.
        if cfg!(windows) && std::env::var_os("WGPU_BACKEND").is_none() {
            setup.instance_descriptor.backends = eframe::wgpu::Backends::DX12;
        }
        if backdrop {
            setup.instance_descriptor.backends = eframe::wgpu::Backends::DX12;
            setup.instance_descriptor.backend_options.dx12.presentation_system =
                eframe::wgpu::Dx12SwapchainKind::DxgiFromVisual;
        }
        // Pick from the adapters egui-wgpu already enumerated instead of letting
        // `request_adapter` enumerate them all again (R5), honouring `WGPU_ADAPTER_NAME`
        // (CI forces the WARP software adapter with it) and `WGPU_POWER_PREF`.
        let power = setup.power_preference;
        setup.native_adapter_selector = Some(std::sync::Arc::new(move |adapters, surface| {
            soyuz_app::devscript::startup_mark("adapters enumerated");
            pick_adapter(adapters, surface, power)
        }));
        let device_descriptor = setup.device_descriptor.clone();
        setup.device_descriptor = std::sync::Arc::new(move |adapter| {
            soyuz_app::devscript::startup_mark("adapter chosen");
            device_descriptor(adapter)
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
        // eframe's own state (window geometry, egui memory) lives with the rest of the
        // configuration, so portable mode keeps it next to the executable as well.
        persistence_path: Some(soyuz_core::config::config_dir().join("app.ron")),
        ..Default::default()
    };
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
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            log::error!("soyuz: {e}");
            eprintln!("soyuz: {e}");
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
        let run = |renderer| Ok(Cli::Run { renderer, startup: Default::default() });
        assert_eq!(parse(&[]), run(RendererChoice::Wgpu));
        assert_eq!(parse(&["--renderer", "glow"]), run(RendererChoice::Glow));
        assert_eq!(parse(&["--renderer=WGPU"]), run(RendererChoice::Wgpu));
        let Ok(Cli::Run { startup, .. }) = parse(&["--new-window", "C:\\data"]) else { panic!("parse") };
        assert!(startup.secondary);
        assert_eq!(startup.open, Some(std::path::PathBuf::from("C:\\data")));
        let Ok(Cli::Run { startup, .. }) = parse(&["--", "-dash"]) else { panic!("parse") };
        assert_eq!(startup.open, Some(std::path::PathBuf::from("-dash")));
        assert!(parse(&["a", "b"]).is_err());
        assert_eq!(parse(&["--help"]), Ok(Cli::Help));
        assert_eq!(parse(&["-V"]), Ok(Cli::Version));
        assert!(parse(&["--renderer"]).is_err());
        assert!(parse(&["--renderer", "vulkan"]).is_err());
        assert!(parse(&["--bogus"]).is_err());
        let Ok(Cli::Run { startup, .. }) = parse(&["--safe-mode"]) else { panic!("parse") };
        assert!(startup.safe_mode && !startup.secondary);
    }

    #[test]
    fn icon_is_square_rgba() {
        let i = soyuz_app::app_icon::icon_data(32);
        assert_eq!(i.rgba.len(), (i.width * i.height * 4) as usize);
        assert!(i.rgba.chunks(4).any(|px| px[3] == 255));
    }
}
