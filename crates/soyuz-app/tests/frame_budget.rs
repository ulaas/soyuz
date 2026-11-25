//! Scripted UI tour with `SOYUZ_FRAME_LOG`: after the initial load no frame may spend
//! more than the budget (16 ms by default, `SOYUZ_FRAME_BUDGET_MS` to change) in `ui()`.
//!
//! Launches the real binary on a temporary folder, in portable mode (a copy of the exe
//! next to a `portable` marker, so no user settings are read or written). Needs a
//! desktop session; meant for release builds:
//! `cargo test -p soyuz-app --release --test frame_budget -- --ignored --nocapture`.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// `frame <n> ui <ms> ms` lines after the `mark` step → frame times in ms.
fn frame_times(log: &str, mark: &str) -> Vec<f64> {
    frame_times_until(log, mark, None)
}

/// [`frame_times`] from `mark` up to the `until` mark (exclusive).
fn frame_times_until(log: &str, mark: &str, until: Option<&str>) -> Vec<f64> {
    let mut counting = false;
    let mut out = Vec::new();
    for line in log.lines() {
        if line.contains(&format!("SOYUZ_SCRIPT: mark {mark}")) {
            counting = true;
            continue;
        }
        if let Some(u) = until
            && counting
            && line.contains(&format!("SOYUZ_SCRIPT: mark {u}"))
        {
            break;
        }
        if !counting {
            continue;
        }
        if let Some(rest) = line.split("frame ").nth(1)
            && let Some(ms) = rest.split(" ui ").nth(1).and_then(|s| s.split(" ms").next())
            && let Ok(v) = ms.trim().parse::<f64>()
        {
            out.push(v);
        }
    }
    out
}

#[test]
fn parses_frame_log() {
    let log = "frame 1 ui 40.00 ms\nINFO SOYUZ_SCRIPT: mark tour\nINFO frame 2 ui 3.25 ms; repaint: \nx frame 3 ui 12.00 ms; repaint: a.rs:1 x\n";
    assert_eq!(frame_times(log, "tour"), vec![3.25, 12.0]);
    let log = "INFO SOYUZ_SCRIPT: mark a\nframe 1 ui 1.00 ms\nINFO SOYUZ_SCRIPT: mark b\nframe 2 ui 2.00 ms\n";
    assert_eq!(frame_times_until(log, "a", Some("b")), vec![1.0]);
    assert_eq!(frame_times_until(log, "b", None), vec![2.0]);
    assert_eq!(stats(&[1.0, 3.0, 2.0]), (3.0, 3.0));
}

/// `(worst, p95)` of frame times in ms.
/// A text PDF of `pages` A4 pages (Helvetica, no dependencies), for the viewer step.
fn text_pdf(pages: usize) -> Vec<u8> {
    let mut objs: Vec<Vec<u8>> = vec![Vec::new(); 3 + 2 * pages];
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    objs[0] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
    objs[1] = format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" ")).into_bytes();
    objs[2] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec();
    for i in 0..pages {
        let mut text = format!(
            "BT /F1 24 Tf 72 760 Td (Page {}) Tj ET
",
            i + 1
        );
        for k in 0..30 {
            let word = if k % 5 == 0 { "Hello" } else { "Lorem" };
            text.push_str(&format!(
                "BT /F1 11 Tf 72 {} Td ({word} ipsum dolor sit amet, line {k}.) Tj ET
",
                720 - k * 20
            ));
        }
        objs[3 + 2 * i] = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            5 + 2 * i
        )
        .into_bytes();
        let mut c = format!(
            "<< /Length {} >>
stream
",
            text.len()
        )
        .into_bytes();
        c.extend_from_slice(text.as_bytes());
        c.extend_from_slice(
            b"
endstream",
        );
        objs[4 + 2 * i] = c;
    }
    let mut out = b"%PDF-1.7
"
    .to_vec();
    let mut offsets = Vec::new();
    for (k, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(
            format!(
                "{} 0 obj
",
                k + 1
            )
            .as_bytes(),
        );
        out.extend_from_slice(o);
        out.extend_from_slice(
            b"
endobj
",
        );
    }
    let xref = out.len();
    out.extend_from_slice(
        format!(
            "xref
0 {}
0000000000 65535 f 
",
            objs.len() + 1
        )
        .as_bytes(),
    );
    for o in offsets {
        out.extend_from_slice(
            format!(
                "{o:010} 00000 n 
"
            )
            .as_bytes(),
        );
    }
    out.extend_from_slice(
        format!(
            "trailer
<< /Size {} /Root 1 0 R >>
startxref
{xref}
%%EOF
",
            objs.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[test]
fn text_pdf_is_well_formed() {
    let pdf = text_pdf(3);
    assert!(pdf.starts_with(b"%PDF-1.7"));
    let s = String::from_utf8_lossy(&pdf);
    assert!(
        s.contains("/Count 3")
            && s.ends_with(
                "%%EOF
"
            )
    );
    // The xref offsets point at the objects.
    let at = s
        .find(
            "0000000000 65535 f 
",
        )
        .unwrap()
        + 20;
    let first: usize = s[at..at + 10].parse().unwrap();
    assert!(s[first..].starts_with("1 0 obj"));
}

fn stats(times: &[f64]) -> (f64, f64) {
    let worst = times.iter().copied().fold(0.0, f64::max);
    let mut sorted = times.to_vec();
    sorted.sort_by(f64::total_cmp);
    let p95 = sorted.get((sorted.len() * 95 / 100).min(sorted.len().saturating_sub(1))).copied().unwrap_or(0.0);
    (worst, p95)
}

#[test]
#[ignore = "launches the GUI; run manually or in a desktop CI job"]
fn scripted_tour_stays_within_frame_budget() {
    let budget: f64 = std::env::var("SOYUZ_FRAME_BUDGET_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(16.0);
    let tmp = tempfile::tempdir().unwrap();
    // A folder worth browsing: 5 000 files and a few subfolders.
    let data = tmp.path().join("data");
    std::fs::create_dir(&data).unwrap();
    for d in ["alpha", "beta", "gamma"] {
        std::fs::create_dir(data.join(d)).unwrap();
    }
    for i in 0..5_000 {
        std::fs::write(data.join(format!("file_{i:05}.txt")), b"x").unwrap();
    }
    // A clip for the "preview a video" step (plays when the FFmpeg libraries load;
    // the poster card otherwise): 10 fps, 6 s of alternating colours.
    {
        use soyuz_media::fixtures::{AviCodec, avi, jpeg_solid};
        let a = jpeg_solid(640, 360, [220, 30, 30]);
        let b = jpeg_solid(640, 360, [30, 40, 220]);
        let frames: Vec<Vec<u8>> = (0..60).map(|i| if (i / 10) % 2 == 0 { a.clone() } else { b.clone() }).collect();
        std::fs::write(data.join("clip.avi"), avi(640, 360, 10, AviCodec::Mjpeg, &frames)).unwrap();
        // 720p 4:2:0 (raw I420, 24 fps, 2 s) for the CPU-RGBA vs GPU-YUV comparison
        // (`[media] gpu_yuv`): two copies, so each opens fresh and autoplays.
        let plane = |w: usize, h: usize, v: u8| vec![v; w * h];
        let a = [plane(1280, 720, 81), plane(640, 360, 90), plane(640, 360, 240)].concat();
        let b = [plane(1280, 720, 41), plane(640, 360, 240), plane(640, 360, 110)].concat();
        let frames: Vec<Vec<u8>> = (0..48).map(|i| if (i / 6) % 2 == 0 { a.clone() } else { b.clone() }).collect();
        let clip = avi(1280, 720, 24, AviCodec::I420, &frames);
        std::fs::write(data.join("hd_cpu.avi"), &clip).unwrap();
        std::fs::write(data.join("hd_gpu.avi"), &clip).unwrap();
    }
    // A 200-page PDF for the viewer step.
    let book = data.join("book.pdf");
    std::fs::write(&book, text_pdf(200)).unwrap();
    // Portable copy of the binary.
    let bin = tmp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let exe = bin.join(if cfg!(windows) { "soyuz.exe" } else { "soyuz" });
    std::fs::copy(env!("CARGO_BIN_EXE_soyuz"), &exe).unwrap();
    std::fs::write(bin.join("portable"), b"").unwrap();

    let d = data.display();
    let script = [
        "welcome:off".to_string(),
        format!("cd:{d}"),
        "wait:2500".into(),
        "mark:tour".into(),
        "select:file_00010.txt".into(),
        "wait:200".into(),
        "view_grid".into(),
        "wait:400".into(),
        "scroll:20000".into(),
        "wait:300".into(),
        "view_details".into(),
        "wait:300".into(),
        "toggle_dual_pane".into(),
        "wait:400".into(),
        format!("cd:{}", data.join("alpha").display()),
        "wait:300".into(),
        "toggle_dual_pane".into(),
        "wait:300".into(),
        "command_palette".into(),
        "wait:300".into(),
        "command_palette".into(),
        "wait:300".into(),
        // Preview a video: the pane's player (autoplay), then Quick Look on it.
        "config:media.autoplay=true".into(),
        "select:clip.avi".into(),
        "toggle_preview_pane".into(),
        "wait:2000".into(),
        "quick_look".into(),
        "wait:1500".into(),
        "quick_look".into(),
        "toggle_preview_pane".into(),
        "wait:300".into(),
        // 720p through the CPU (RGBA textures), then through the GPU YUV path.
        "config:media.gpu_yuv=false".into(),
        "select:hd_cpu.avi".into(),
        "mark:cpu".into(),
        "toggle_preview_pane".into(),
        "wait:2000".into(),
        "quick_look".into(),
        "wait:1500".into(),
        "quick_look".into(),
        "toggle_preview_pane".into(),
        "wait:300".into(),
        "config:media.gpu_yuv=true".into(),
        "select:hd_gpu.avi".into(),
        "mark:gpu".into(),
        "toggle_preview_pane".into(),
        "wait:2000".into(),
        "quick_look".into(),
        "wait:1500".into(),
        "quick_look".into(),
        "toggle_preview_pane".into(),
        "wait:300".into(),
        // The PDF viewer: open, zoom, find, jump, thumbnails, close.
        format!("edit:{}", book.display()),
        "editor:maximize".into(),
        "mark:pdf".into(),
        "wait:1500".into(),
        "editor:pdf=zoom=200".into(),
        "wait:500".into(),
        "editor:pdf=find=Hello".into(),
        "wait:800".into(),
        "editor:pdf=page=120".into(),
        "wait:500".into(),
        "editor:pdf=sidebar=thumbnails".into(),
        "wait:800".into(),
        "editor:pdf=fit=page".into(),
        "wait:500".into(),
        "editor:close".into(),
        "wait:300".into(),
        "mark:end".into(),
        "quit".into(),
    ]
    .join(";");
    let mut child = Command::new(&exe)
        .env("SOYUZ_SCRIPT", script)
        .env("SOYUZ_FRAME_LOG", "1")
        // Linux: keep freedesktop thumbnail writes out of the real ~/.cache.
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
            panic!("soyuz did not quit within 90 s");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let log = reader.join().unwrap();
    let times = frame_times(&log, "tour");
    assert!(times.len() > 10, "too few frames logged:\n{log}");
    let (worst, p95) = stats(&times);
    println!("{} frames after load: worst {worst:.2} ms, p95 {p95:.2} ms (budget {budget} ms)", times.len());
    for (name, from, to) in
        [("720p CPU RGBA", "cpu", "gpu"), ("720p GPU YUV", "gpu", "pdf"), ("PDF viewer", "pdf", "end")]
    {
        let seg = frame_times_until(&log, from, Some(to));
        let (w, p) = stats(&seg);
        println!("  {name}: {} frames, worst {w:.2} ms, p95 {p:.2} ms", seg.len());
    }
    assert!(worst <= budget, "a frame took {worst:.2} ms (> {budget} ms)");
}
