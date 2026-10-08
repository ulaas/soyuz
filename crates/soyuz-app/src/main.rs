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

Arguments:
  [PATH]                  Folder to open (a file opens its folder with the file selected)

Options:
  --renderer <wgpu|glow>  Rendering backend (default: wgpu; glow needs the `glow` cargo feature)
  --new-window            Secondary window: don't restore or overwrite the saved session
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

/// A small procedural window icon (accent ring with an orbiting dot).
fn window_icon() -> egui::IconData {
    const N: u32 = 64;
    let mut rgba = vec![0u8; (N * N * 4) as usize];
    let c = (N as f32 - 1.0) / 2.0;
    let accent = [0x5B, 0x8C, 0xFF];
    let dot = (c + 17.0 * 0.70, c - 17.0 * 0.70);
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32, y as f32);
            let r = ((fx - c).powi(2) + (fy - c).powi(2)).sqrt();
            let ring = (1.0 - ((r - 17.0).abs() - 2.5).clamp(0.0, 1.0)).max(0.0);
            let core = (1.0 - (r - 7.0).clamp(0.0, 1.0)).max(0.0);
            let dr = ((fx - dot.0).powi(2) + (fy - dot.1).powi(2)).sqrt();
            let sat = (1.0 - (dr - 5.0).clamp(0.0, 1.0)).max(0.0);
            let a = ring.max(core).max(sat);
            let i = ((y * N + x) * 4) as usize;
            let color = if sat > 0.0 { [0xF2, 0xB8, 0x4B] } else { accent };
            rgba[i..i + 3].copy_from_slice(&color);
            rgba[i + 3] = (a * 255.0) as u8;
        }
    }
    egui::IconData { rgba, width: N, height: N }
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu=warn,naga=warn")).init();

    let cli = match parse_args(std::env::args().skip(1)) {
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

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Soyuz")
            .with_app_id("soyuz")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([720.0, 440.0])
            .with_icon(window_icon()),
        renderer,
        centered: true,
        // eframe's own state (window geometry, egui memory) lives with the rest of the
        // configuration, so portable mode keeps it next to the executable as well.
        persistence_path: Some(soyuz_core::config::config_dir().join("app.ron")),
        ..Default::default()
    };
    let result = eframe::run_native(
        "Soyuz",
        options,
        Box::new(move |cc| Ok(Box::new(soyuz_app::SoyuzApp::new(cc, renderer_name, startup)))),
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
    }

    #[test]
    fn icon_is_square_rgba() {
        let i = window_icon();
        assert_eq!(i.rgba.len(), (i.width * i.height * 4) as usize);
        assert!(i.rgba.chunks(4).any(|px| px[3] == 255));
    }
}
