//! The GUI video player, end to end: the real binary previews a generated MJPEG clip
//! (`soyuz_media::fixtures`) in the preview pane with `[media] autoplay`, then in Quick
//! Look, and screenshots its own window (`SOYUZ_SCRIPT` `shot:`; the window opens
//! off-screen with `SOYUZ_WINDOW_POS`). The clip is red for its first second and blue
//! after: blue pixels in the capture prove a decoded frame is on screen, not the poster
//! (which is taken at a tenth of the length, while it is still red).
//!
//! Needs the FFmpeg libraries and a desktop session: runs only with
//! `SOYUZ_MEDIA_TESTS=1` (and `SOYUZ_FFMPEG_DIR` when they are not next to the exe):
//! `SOYUZ_MEDIA_TESTS=1 cargo test -p soyuz-app --test player_smoke -- --nocapture`.
//! Without the variable it passes doing nothing; with it, a GUI that never comes up is
//! a failure.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use soyuz_media::fixtures::{AviCodec, avi, jpeg_solid};

/// Pixels of the capture that are clearly `rgb` (each channel within `tol`).
fn count_near(png: &Path, rgb: [u8; 3], tol: u8) -> usize {
    let img = image::open(png).expect("read the screenshot").to_rgba8();
    img.pixels()
        .filter(|p| {
            p[3] > 200 && p[0].abs_diff(rgb[0]) <= tol && p[1].abs_diff(rgb[1]) <= tol && p[2].abs_diff(rgb[2]) <= tol
        })
        .count()
}

#[test]
fn counts_pixels_near_a_colour() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("x.png");
    let mut img = image::RgbaImage::new(4, 2);
    for (i, px) in img.pixels_mut().enumerate() {
        *px = if i < 3 { image::Rgba([30, 40, 220, 255]) } else { image::Rgba([200, 200, 200, 255]) };
    }
    img.save(&p).unwrap();
    assert_eq!(count_near(&p, [30, 40, 220], 24), 3);
    assert_eq!(count_near(&p, [255, 0, 0], 24), 0);
}

#[test]
fn preview_pane_and_quick_look_play_a_clip() {
    if std::env::var("SOYUZ_MEDIA_TESTS").map_or(true, |v| v != "1") {
        eprintln!("player_smoke: skipped (set SOYUZ_MEDIA_TESTS=1 with the FFmpeg libraries available)");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir(&data).unwrap();
    // 10 fps, 5 s: red for the first second, blue after.
    let red = jpeg_solid(320, 240, [220, 30, 30]);
    let blue = jpeg_solid(320, 240, [30, 40, 220]);
    let frames: Vec<Vec<u8>> = (0..50).map(|i| if i < 10 { red.clone() } else { blue.clone() }).collect();
    std::fs::write(data.join("clip.avi"), avi(320, 240, 10, AviCodec::Mjpeg, &frames)).unwrap();
    std::fs::write(data.join("note.txt"), b"hello").unwrap();
    let shots = tmp.path().join("shots");
    std::fs::create_dir(&shots).unwrap();
    let pane_png = shots.join("pane.png");
    let ql_png = shots.join("quicklook.png");
    let bin = tmp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let exe = bin.join(if cfg!(windows) { "soyuz.exe" } else { "soyuz" });
    std::fs::copy(env!("CARGO_BIN_EXE_soyuz"), &exe).unwrap();
    std::fs::write(bin.join("portable"), b"").unwrap();

    let script = [
        "welcome:off".to_string(),
        "config:media.autoplay=true".into(),
        "config:layout.preview_pane=false".into(),
        "size:1100x760".into(),
        format!("cd:{}", data.display()),
        "wait_for:has=clip.avi".into(),
        "select:clip.avi".into(),
        "toggle_preview_pane".into(),
        // Past the red second (the player opens off-thread first).
        "wait:3000".into(),
        format!("shot:{}", pane_png.display()),
        "wait:200".into(),
        "quick_look".into(),
        "wait:2500".into(),
        format!("shot:{}", ql_png.display()),
        "wait:200".into(),
        "quit".into(),
    ]
    .join(";");
    let mut child = Command::new(&exe)
        .env("SOYUZ_SCRIPT", script)
        .env("SOYUZ_WINDOW_POS", "-4000,-4000")
        .env("XDG_CACHE_HOME", tmp.path().join("cache"))
        .env("RUST_LOG", "info,wgpu=warn,naga=warn,wgpu_hal=error")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch soyuz");
    let stderr = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut log = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            log.push_str(&line);
            log.push('\n');
        }
        log
    });
    let deadline = Instant::now() + Duration::from_secs(90);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("soyuz did not quit within 90 s");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let log = reader.join().unwrap();
    assert!(log.contains("SOYUZ_SCRIPT: welcome:off"), "the GUI did not start:\n{log}");
    assert!(pane_png.exists() && ql_png.exists(), "no screenshots:\n{log}");
    // A decoded (blue) frame is on screen in both; the poster alone would be red.
    let pane_blue = count_near(&pane_png, [30, 40, 220], 28);
    let ql_blue = count_near(&ql_png, [30, 40, 220], 28);
    println!("blue pixels: pane {pane_blue}, Quick Look {ql_blue} ({})", shots.display());
    assert!(pane_blue > 2_000, "the preview pane shows no decoded frame ({pane_blue} blue pixels):\n{log}");
    assert!(ql_blue > 20_000, "Quick Look shows no decoded frame ({ql_blue} blue pixels):\n{log}");
    // Keep the captures for a look when asked.
    if let Ok(keep) = std::env::var("SOYUZ_KEEP_SHOTS") {
        let dir = Path::new(&keep);
        std::fs::create_dir_all(dir).unwrap();
        std::fs::copy(&pane_png, dir.join("player-pane.png")).unwrap();
        std::fs::copy(&ql_png, dir.join("player-quicklook.png")).unwrap();
    }
}
