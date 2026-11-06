//! Font rendering lab: every sample carries an id; pick the ones you like.
//! `--preset <name>`, `--sample <name>`, `--scroll <points>`, `--size WxH`,
//! `--window-pos x,y`, `--shot <file.png>` (save the window once drawn, then quit),
//! `--verify` (real window text over the second card, to compare with the first),
//! `--picks-only`, `--hover <id>` (fill the inspector as if the pointer were on that sample).

#[cfg(windows)]
mod dwrite;
mod raster;
mod spec;
mod ui;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let preset = arg("--preset");
    let shot = arg("--shot");
    let sample = arg("--sample");
    let scroll = arg("--scroll").and_then(|s| s.parse::<f32>().ok());
    let verify = args.iter().any(|a| a == "--verify");
    let only_picks = args.iter().any(|a| a == "--picks-only");
    let hover = arg("--hover");
    let size = arg("--size")
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some([w.parse().ok()?, h.parse().ok()?])
        })
        .unwrap_or([1680.0, 1000.0]);
    let pos = arg("--window-pos").and_then(|s| {
        let (x, y) = s.split_once(',')?;
        Some(egui::pos2(x.parse().ok()?, y.parse().ok()?))
    });

    // Soyuz's renderer setup (crates/soyuz-app/src/main.rs): wgpu, DX12 on Windows unless
    // `WGPU_BACKEND` says otherwise.
    let mut wgpu_options = eframe::egui_wgpu::WgpuConfiguration::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup
        && cfg!(windows)
        && std::env::var_os("WGPU_BACKEND").is_none()
    {
        setup.instance_descriptor.backends = eframe::wgpu::Backends::DX12;
    }
    let mut viewport = egui::ViewportBuilder::default().with_title("Soyuz font lab").with_inner_size(size);
    if let Some(p) = pos {
        viewport = viewport.with_position(p).with_active(false);
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options,
        centered: pos.is_none(),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "Soyuz font lab",
        options,
        Box::new(move |cc| Ok(Box::new(ui::Lab::new(cc, preset, shot, sample, scroll, verify, only_picks, hover)))),
    )
}
