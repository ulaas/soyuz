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
    Run { renderer: RendererChoice },
    Help,
    Version,
}

const USAGE: &str = "\
Usage: soyuz [OPTIONS]

Options:
  --renderer <wgpu|glow>  Rendering backend (default: wgpu; glow needs the `glow` cargo feature)
  -h, --help              Print help
  -V, --version           Print version";

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Cli, String> {
    let mut renderer = RendererChoice::Wgpu;
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
            other => return Err(format!("unexpected argument '{other}'")),
        }
    }
    Ok(Cli::Run { renderer })
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
        Cli::Run { renderer } => renderer,
    };
    let (renderer, renderer_name) = resolve_renderer(renderer);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Soyuz")
            .with_app_id("soyuz")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([720.0, 440.0])
            .with_icon(soyuz_app::app_icon::icon_data(128)),
        renderer,
        centered: true,
        ..Default::default()
    };
    let result = eframe::run_native(
        "Soyuz",
        options,
        Box::new(move |cc| Ok(Box::new(soyuz_app::SoyuzApp::new(cc, renderer_name)))),
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
        assert_eq!(parse(&[]), Ok(Cli::Run { renderer: RendererChoice::Wgpu }));
        assert_eq!(parse(&["--renderer", "glow"]), Ok(Cli::Run { renderer: RendererChoice::Glow }));
        assert_eq!(parse(&["--renderer=WGPU"]), Ok(Cli::Run { renderer: RendererChoice::Wgpu }));
        assert_eq!(parse(&["--help"]), Ok(Cli::Help));
        assert_eq!(parse(&["-V"]), Ok(Cli::Version));
        assert!(parse(&["--renderer"]).is_err());
        assert!(parse(&["--renderer", "vulkan"]).is_err());
        assert!(parse(&["--bogus"]).is_err());
    }

    #[test]
    fn icon_is_square_rgba() {
        let i = soyuz_app::app_icon::icon_data(32);
        assert_eq!(i.rgba.len(), (i.width * i.height * 4) as usize);
        assert!(i.rgba.chunks(4).any(|px| px[3] == 255));
    }
}
