//! The transcoder against real libraries (`SOYUZ_FFMPEG_DIR`): remux, trim (stream copy
//! and frame-exact), convert with the presets this build has, extract audio, save a
//! frame, cancel, corrupt input, `plan`. Each preset test is gated on
//! `Runtime::has_encoder` (a development runtime may lack libvpx / libopus, and no CI
//! runner has a hardware H.264 encoder).
//!
//! Without a runtime every test prints `skipped: …` and passes, unless
//! `SOYUZ_MEDIA_TESTS=1` is set: then a missing runtime is a failure (CI).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use soyuz_media::transcode::{AudioCodecChoice, Cancelled, ImageFormat, MediaOp, Outcome, Progress, plan, run};
use soyuz_media::{DIR_ENV, MediaError, PresetId, Runtime, Source, StreamKind, fixtures, presets, probe, search_dirs};

fn runtime() -> Option<Arc<Runtime>> {
    let strict = std::env::var("SOYUZ_MEDIA_TESTS").is_ok_and(|v| v == "1");
    let dirs: Vec<PathBuf> = match std::env::var_os(DIR_ENV) {
        Some(d) => vec![PathBuf::from(d)],
        None => search_dirs("", &std::env::temp_dir().join("soyuz-media-tests-config")),
    };
    match Runtime::get(&dirs) {
        Ok(rt) => Some(rt),
        Err(e) if strict => panic!("SOYUZ_MEDIA_TESTS=1 but no FFmpeg runtime loads: {e}"),
        Err(e) => {
            eprintln!("skipped: no FFmpeg runtime ({e}); set {DIR_ENV} to a folder with the libraries");
            None
        }
    }
}

macro_rules! rt {
    () => {
        match runtime() {
            Some(r) => r,
            None => return,
        }
    };
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
}

/// A Motion-JPEG AVI of `n` frames at `fps` cycling red / green / blue, on disk.
fn rgb_avi(dir: &Path, name: &str, w: u32, h: u32, fps: u32, n: usize) -> PathBuf {
    let colours = [[255u8, 0, 0], [0, 255, 0], [0, 0, 255]];
    let frames: Vec<Vec<u8>> = (0..n).map(|i| fixtures::jpeg_solid(w, h, colours[i % 3])).collect();
    let path = dir.join(name);
    std::fs::write(&path, fixtures::avi(w, h, fps, fixtures::AviCodec::Mjpeg, &frames)).unwrap();
    path
}

/// A mono 16-bit WAV of `secs` seconds of a 440 Hz tone at `rate`, on disk.
fn tone_wav(dir: &Path, name: &str, rate: u32, secs: f64) -> PathBuf {
    let samples = fixtures::sine(rate, 440.0, 0.5, (f64::from(rate) * secs) as usize);
    let path = dir.join(name);
    std::fs::write(&path, fixtures::wav(rate, 1, &samples)).unwrap();
    path
}

fn no_progress(_: Progress) {}

fn go() -> Result<(), Cancelled> {
    Ok(())
}

fn run_ok(rt: &Runtime, src: &Path, dest: &Path, op: &MediaOp) -> Outcome {
    let mut reports: Vec<Progress> = Vec::new();
    let out = run(rt, src, dest, op, &mut |p| reports.push(p), &mut go).unwrap_or_else(|e| panic!("{op:?}: {e}"));
    assert!(dest.is_file(), "{} written", dest.display());
    assert!(!dest.with_file_name(format!("{}.part", dest.file_name().unwrap().to_string_lossy())).exists());
    let last = reports.last().expect("at least the final report");
    assert!(last.out_bytes > 0 || matches!(op, MediaOp::Frame { .. }), "{last:?}");
    assert_eq!(out.out_bytes, std::fs::metadata(dest).unwrap().len(), "{op:?}: out_bytes is the file size");
    out
}

#[test]
fn remux_mjpeg_avi_to_mkv_keeps_streams_and_duration() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    let src = rgb_avi(dir.path(), "rgb.avi", 32, 24, 5, 10);
    let dest = dir.path().join("rgb.mkv");
    let out = run_ok(&rt, &src, &dest, &MediaOp::Remux { container: "matroska".into() });
    assert!((out.duration.expect("duration") - 2.0).abs() < 0.25, "{:?}", out.duration);
    let info = probe(&rt, &Source::File(dest.clone())).expect("probe output");
    assert!(info.container.starts_with("matroska"), "{}", info.container);
    assert_eq!(info.streams.len(), 1);
    assert_eq!(info.video().expect("video").codec, "mjpeg");
    assert!((info.duration.expect("duration") - 2.0).abs() < 0.25, "{:?}", info.duration);
    // The same into MP4 (a copy preset): the muxer picks its own time base.
    let dest = dir.path().join("rgb.mp4");
    run_ok(&rt, &src, &dest, &MediaOp::Convert { preset: PresetId::Mp4Copy, video_stream: None, audio_stream: None });
    let info = probe(&rt, &Source::File(dest)).expect("probe mp4");
    assert_eq!(info.video().expect("video").codec, "mjpeg");
    assert!((info.duration.expect("duration") - 2.0).abs() < 0.25, "{:?}", info.duration);
    // Both streams of the MP4 fixture survive a remux, with their tags.
    let dest = dir.path().join("rotated.mkv");
    let out = run_ok(&rt, &fixture("rotated.mp4"), &dest, &MediaOp::Remux { container: "matroska".into() });
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let info = probe(&rt, &Source::File(dest)).expect("probe mkv");
    assert_eq!(info.streams.len(), 2);
    assert_eq!(info.tag("title"), Some("Soyuz rotated"));
    assert_eq!(info.audio().expect("audio").language.as_deref(), Some("eng"));
    assert_eq!(info.video().expect("video").codec, "mpeg4");
    assert!((info.duration.expect("duration") - 1.0).abs() < 0.1, "{:?}", info.duration);
}

#[test]
fn trim_by_stream_copy_lands_within_a_frame() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    // 25 frames at 5 fps = 5 s, every frame a keyframe.
    let src = rgb_avi(dir.path(), "long.avi", 32, 24, 5, 25);
    let dest = dir.path().join("cut.avi");
    let out = run_ok(&rt, &src, &dest, &MediaOp::Trim { start: 1.0, end: 3.0, stream_copy: true });
    assert!((out.duration.expect("duration") - 2.0).abs() <= 0.2 + 1e-6, "{:?}", out.duration);
    let info = probe(&rt, &Source::File(dest)).expect("probe cut");
    assert!((info.duration.expect("duration") - 2.0).abs() <= 0.2 + 1e-6, "{:?}", info.duration);
    assert!(info.start.abs() < 0.05, "starts at zero: {}", info.start);
    // The window beyond the end is clamped, the destination extension picks the container.
    let dest = dir.path().join("tail.mkv");
    let out = run_ok(&rt, &src, &dest, &MediaOp::Trim { start: 4.0, end: 9.0, stream_copy: true });
    assert!((out.duration.expect("duration") - 1.0).abs() <= 0.2 + 1e-6, "{:?}", out.duration);
    let info = probe(&rt, &Source::File(dest)).expect("probe tail");
    assert!(info.container.starts_with("matroska"), "{}", info.container);
    // A clip with audio: the cut keeps both streams.
    let dest = dir.path().join("rotated-cut.mp4");
    let out = run_ok(&rt, &fixture("rotated.mp4"), &dest, &MediaOp::Trim { start: 0.0, end: 0.5, stream_copy: true });
    let info = probe(&rt, &Source::File(dest)).expect("probe rotated cut");
    assert_eq!(info.streams.len(), 2);
    assert!((info.duration.expect("duration") - 0.5).abs() <= 0.15, "{:?} / {:?}", info.duration, out.duration);
}

#[test]
fn trim_by_reencoding_is_frame_exact() {
    let rt = rt!();
    if !rt.has_encoder("mjpeg") {
        eprintln!("skipped: no mjpeg encoder");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = rgb_avi(dir.path(), "long.avi", 32, 24, 5, 25);
    let dest = dir.path().join("exact.avi");
    let op = MediaOp::Trim { start: 1.0, end: 3.0, stream_copy: false };
    let p = plan(&rt, &src, &op).expect("plan");
    assert_eq!(p.streams, vec![(0, "encode mjpeg".to_string())]);
    assert_eq!((p.container.as_str(), p.ext.as_str()), ("avi", "avi"));
    let out = run_ok(&rt, &src, &dest, &op);
    assert!((out.duration.expect("duration") - 2.0).abs() <= 0.2 + 1e-6, "{:?}", out.duration);
    let info = probe(&rt, &Source::File(dest.clone())).expect("probe exact");
    assert_eq!(info.video().expect("video").codec, "mjpeg");
    assert!((info.duration.expect("duration") - 2.0).abs() <= 0.2 + 1e-6, "{:?}", info.duration);
    // Frame 5 (1.0 s) is blue, frame 10 is green: the first output frame is blue.
    let img = soyuz_media::grab_frame(&rt, &Source::File(dest), soyuz_media::FrameRequest::near(0.0, 32), &|| false)
        .expect("first frame");
    let [r, g, b, _] = img.get_pixel(16, 12).0;
    assert!(b > 150 && r < 100 && g < 100, "first frame is blue: {r} {g} {b}");
}

#[test]
fn convert_with_available_presets_round_trips() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    let src = fixture("rotated.mp4");
    let available = presets(&rt);
    let names: Vec<&str> = available.iter().map(|p| p.id.name()).collect();
    eprintln!("presets on this machine: {names:?}");
    assert!(names.contains(&"mkv-copy") && names.contains(&"mp4-copy"));
    for p in &available {
        if p.id == PresetId::Mp4H264Hw || p.id == PresetId::Mp4HevcHw {
            // Hardware encoders are exercised below when present.
            continue;
        }
        let dest = dir.path().join(format!("{}.{}", p.id.name(), p.ext));
        let op = MediaOp::Convert { preset: p.id, video_stream: None, audio_stream: None };
        let planned = plan(&rt, &src, &op).unwrap_or_else(|e| panic!("plan {}: {e}", p.id.name()));
        assert_eq!(planned.container, p.container);
        assert_eq!(planned.ext, p.ext);
        let out = run_ok(&rt, &src, &dest, &op);
        let info = probe(&rt, &Source::File(dest)).unwrap_or_else(|e| panic!("probe {}: {e}", p.id.name()));
        let dur = info.duration.or(out.duration).unwrap_or(0.0);
        assert!((dur - 1.0).abs() < 0.2, "{}: duration {dur} ({:?} / {:?})", p.id.name(), info.duration, out.duration);
        let v = info.video();
        let a = info.audio();
        match p.id {
            PresetId::MkvCopy | PresetId::Mp4Copy => {
                assert_eq!(v.map(|s| s.codec.as_str()), Some("mpeg4"), "{}", p.id.name());
                assert_eq!(a.map(|s| s.codec.as_str()), Some("aac"), "{}", p.id.name());
            }
            PresetId::WebmVp9 => {
                assert_eq!(v.map(|s| s.codec.as_str()), Some("vp9"));
                assert_eq!(a.map(|s| s.codec.as_str()), Some("opus"));
                assert_eq!(a.map(|s| s.sample_rate), Some(48_000), "22.05 kHz is not an Opus rate");
            }
            PresetId::WebmVp8 => {
                assert_eq!(v.map(|s| s.codec.as_str()), Some("vp8"));
                assert_eq!(a.map(|s| s.codec.as_str()), Some("opus"));
            }
            PresetId::MovProres => {
                assert_eq!(v.map(|s| s.codec.as_str()), Some("prores"));
                assert_eq!(a.map(|s| s.codec.as_str()), Some("pcm_s16le"));
            }
            PresetId::Mp4Mpeg4 => {
                assert_eq!(v.map(|s| s.codec.as_str()), Some("mpeg4"));
                assert_eq!(a.map(|s| s.codec.as_str()), Some("aac"));
            }
            PresetId::AviMpeg4 => {
                assert_eq!(v.map(|s| s.codec.as_str()), Some("mpeg4"));
                assert!(matches!(a.map(|s| s.codec.as_str()), Some("mp2" | "pcm_s16le")), "{a:?}");
            }
            PresetId::AudioFlac => {
                assert!(v.is_none());
                assert_eq!(a.map(|s| s.codec.as_str()), Some("flac"));
            }
            PresetId::AudioAac => {
                assert!(v.is_none());
                assert_eq!(a.map(|s| s.codec.as_str()), Some("aac"));
            }
            PresetId::AudioOpus => {
                assert!(v.is_none());
                assert_eq!(a.map(|s| s.codec.as_str()), Some("opus"));
            }
            PresetId::AudioWav => {
                assert!(v.is_none());
                assert_eq!(a.map(|s| s.codec.as_str()), Some("pcm_s16le"));
                assert_eq!(a.map(|s| s.sample_rate), Some(22_050), "WAV keeps the input rate");
            }
            PresetId::AudioAlac => {
                assert!(v.is_none());
                assert_eq!(a.map(|s| s.codec.as_str()), Some("alac"));
            }
            PresetId::Mp4H264Hw | PresetId::Mp4HevcHw => unreachable!(),
        }
        if let Some(v) = v {
            assert_eq!((v.width, v.height), (64, 48), "{}", p.id.name());
            assert!(
                v.rotation == 270 || p.container == "avi" || p.container == "webm",
                "{}: rotation {}",
                p.id.name(),
                v.rotation
            );
        }
        if let Some(a) = a {
            assert_eq!(a.channels, 1, "{}", p.id.name());
        }
    }
    // Hardware H.264 / HEVC: only on machines with the driver, and on a picture big
    // enough for them (NVENC refuses the 64x48 fixture).
    let big = rgb_avi(dir.path(), "big.avi", 320, 240, 5, 10);
    for (id, codec) in [(PresetId::Mp4H264Hw, "h264"), (PresetId::Mp4HevcHw, "hevc")] {
        if !available.iter().any(|p| p.id == id) {
            eprintln!("skipped: {} (no hardware encoder opens)", id.name());
            continue;
        }
        let dest = dir.path().join(format!("{}.mp4", id.name()));
        let op = MediaOp::Convert { preset: id, video_stream: None, audio_stream: None };
        let planned = plan(&rt, &big, &op).expect("plan hw");
        assert!(planned.streams[0].1.starts_with("encode "), "{:?}", planned.streams);
        let out = run_ok(&rt, &big, &dest, &op);
        let info = probe(&rt, &Source::File(dest)).expect("probe hw");
        let v = info.video().expect("video");
        assert_eq!(v.codec, codec);
        assert_eq!((v.width, v.height), (320, 240));
        let dur = info.duration.or(out.duration).unwrap_or(0.0);
        assert!((dur - 2.0).abs() < 0.3, "{}: {dur}", id.name());
        // A picture too small for the hardware encoder is a clear error, not a crash.
        let tiny = dir.path().join(format!("{}-tiny.mp4", id.name()));
        match run(&rt, &src, &tiny, &op, &mut no_progress, &mut go) {
            Ok(_) => {}
            Err(e) => assert!(matches!(e, MediaError::Unsupported(_)) && !tiny.exists(), "{e}"),
        }
    }
    // A preset without its encoders is refused with a clear message.
    if !rt.has_encoder("h264_nvenc")
        && !rt.has_encoder("h264_amf")
        && !rt.has_encoder("h264_qsv")
        && !rt.has_encoder("h264_videotoolbox")
        && !rt.has_encoder("h264_vaapi")
    {
        let op = MediaOp::Convert { preset: PresetId::Mp4H264Hw, video_stream: None, audio_stream: None };
        let e = plan(&rt, &src, &op).unwrap_err();
        assert_eq!(e.to_string(), "unsupported: no H.264 hardware encoder on this machine");
        let dest = dir.path().join("never.mp4");
        assert!(run(&rt, &src, &dest, &op, &mut no_progress, &mut go).is_err());
        assert!(!dest.exists() && !dir.path().join("never.mp4.part").exists());
    }
}

#[test]
fn audio_conversions_from_a_wav() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    let src = tone_wav(dir.path(), "tone.wav", 44_100, 1.5);
    for (choice, encoder, codec, ext) in [
        (AudioCodecChoice::Flac, "flac", "flac", "flac"),
        (AudioCodecChoice::Aac, "aac", "aac", "m4a"),
        (AudioCodecChoice::Opus, "libopus", "opus", "opus"),
        (AudioCodecChoice::Wav, "pcm_s16le", "pcm_s16le", "wav"),
        (AudioCodecChoice::Copy, "copy", "pcm_s16le", "wav"),
    ] {
        if encoder != "copy" && !rt.has_encoder(encoder) {
            eprintln!("skipped: no {encoder} encoder");
            continue;
        }
        let op = MediaOp::ExtractAudio { codec: choice };
        let p = plan(&rt, &src, &op).expect("plan");
        assert_eq!(p.ext, ext, "{choice:?}");
        assert_eq!(
            p.streams,
            vec![(0, if encoder == "copy" { "copy".to_string() } else { format!("encode {encoder}") })]
        );
        let dest = dir.path().join(format!("tone-{choice:?}.{ext}"));
        let out = run_ok(&rt, &src, &dest, &op);
        let info = probe(&rt, &Source::File(dest.clone())).expect("probe");
        let a = info.audio().expect("audio");
        assert_eq!(a.codec, codec, "{choice:?}");
        assert_eq!(a.channels, 1);
        let dur = info.duration.or(out.duration).unwrap_or(0.0);
        assert!((dur - 1.5).abs() < 0.1, "{choice:?}: {dur}");
        // The tone survives: decoded samples are not silence.
        let mut r = soyuz_media::AudioReader::open(&rt, &Source::File(dest), None, None).expect("reader");
        let mut buf = Vec::new();
        let mut total = 0;
        loop {
            let n = r.read(&mut buf, 4096).expect("read");
            if n == 0 {
                break;
            }
            total += n;
        }
        let expected = 1.5 * f64::from(r.rate());
        assert!((total as f64 - expected).abs() < expected * 0.05, "{choice:?}: {total} frames at {} Hz", r.rate());
        assert!(buf.iter().any(|s| s.abs() > 0.2), "{choice:?}: not silence");
    }
}

#[test]
fn extract_audio_from_the_mp4_fixture() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    let src = fixture("rotated.mp4");
    // Copy: AAC goes into an M4A.
    let op = MediaOp::ExtractAudio { codec: AudioCodecChoice::Copy };
    let p = plan(&rt, &src, &op).expect("plan");
    assert_eq!((p.container.as_str(), p.ext.as_str()), ("ipod", "m4a"));
    assert_eq!(p.streams, vec![(0, "drop: not kept".to_string()), (1, "copy".to_string())]);
    assert!(p.estimated_bytes.is_some_and(|b| b > 0 && b < 20_000), "{:?}", p.estimated_bytes);
    let dest = dir.path().join("audio.m4a");
    run_ok(&rt, &src, &dest, &op);
    let info = probe(&rt, &Source::File(dest)).expect("probe m4a");
    assert_eq!(info.streams.len(), 1);
    let a = info.audio().expect("audio");
    assert_eq!((a.codec.as_str(), a.sample_rate, a.channels), ("aac", 22_050, 1));
    assert_eq!(a.language.as_deref(), Some("eng"), "stream tags are copied");
    assert!((info.duration.expect("duration") - 1.0).abs() < 0.1);
    // Encode to FLAC.
    if rt.has_encoder("flac") {
        let dest = dir.path().join("audio.flac");
        run_ok(&rt, &src, &dest, &MediaOp::ExtractAudio { codec: AudioCodecChoice::Flac });
        let info = probe(&rt, &Source::File(dest)).expect("probe flac");
        let a = info.audio().expect("audio");
        assert_eq!((a.codec.as_str(), a.sample_rate, a.channels), ("flac", 22_050, 1));
        assert!((info.duration.expect("duration") - 1.0).abs() < 0.1);
    }
}

#[test]
fn frame_to_png_and_jpeg() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    let src = fixture("rotated.mp4");
    let dest = dir.path().join("frame.png");
    let op = MediaOp::Frame { at: 0.5, format: ImageFormat::Png, max_px: None };
    let p = plan(&rt, &src, &op).expect("plan");
    assert_eq!((p.container.as_str(), p.ext.as_str()), ("png", "png"));
    assert_eq!(p.estimated_bytes, None);
    let out = run_ok(&rt, &src, &dest, &op);
    assert_eq!(out.duration, None);
    let img = image::open(&dest).expect("png decodes");
    assert_eq!((img.width(), img.height()), (48, 64), "rotation applied");
    assert_eq!(std::fs::read(&dest).unwrap()[..4], [0x89, b'P', b'N', b'G']);
    let dest = dir.path().join("frame.jpg");
    run_ok(&rt, &src, &dest, &MediaOp::Frame { at: 0.5, format: ImageFormat::Jpeg, max_px: Some(32) });
    let img = image::open(&dest).expect("jpeg decodes");
    assert_eq!((img.width(), img.height()), (24, 32), "fitted into 32 px");
    assert_eq!(std::fs::read(&dest).unwrap()[..2], [0xFF, 0xD8]);
    // No video stream: an error and no file.
    let wav = tone_wav(dir.path(), "tone.wav", 8000, 0.2);
    let dest = dir.path().join("none.png");
    let e = run(&rt, &wav, &dest, &op, &mut no_progress, &mut go).unwrap_err();
    assert!(matches!(e, MediaError::NoStream(StreamKind::Video)), "{e}");
    assert!(!dest.exists() && !dir.path().join("none.png.part").exists());
}

#[test]
fn cancel_leaves_nothing_behind() {
    let rt = rt!();
    if !rt.has_encoder("flac") {
        eprintln!("skipped: no flac encoder");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = tone_wav(dir.path(), "long.wav", 44_100, 4.0);
    let dest = dir.path().join("long.flac");
    let part = dir.path().join("long.flac.part");
    let calls = AtomicUsize::new(0);
    let mut after_three = || if calls.fetch_add(1, Ordering::Relaxed) >= 3 { Err(Cancelled) } else { Ok(()) };
    let mut reports = 0usize;
    let e = run(
        &rt,
        &src,
        &dest,
        &MediaOp::ExtractAudio { codec: AudioCodecChoice::Flac },
        &mut |_| reports += 1,
        &mut after_three,
    )
    .unwrap_err();
    assert!(matches!(e, MediaError::Cancelled), "{e}");
    assert!(!dest.exists(), "no destination");
    assert!(!part.exists(), "no .part left");
    assert!(calls.load(Ordering::Relaxed) <= 8, "stopped early: {} checks", calls.load(Ordering::Relaxed));
    // An existing destination is refused before anything is written.
    std::fs::write(&dest, b"keep").unwrap();
    let e = run(&rt, &src, &dest, &MediaOp::ExtractAudio { codec: AudioCodecChoice::Wav }, &mut no_progress, &mut go)
        .unwrap_err();
    assert!(matches!(e, MediaError::Io(ref io) if io.kind() == std::io::ErrorKind::AlreadyExists), "{e}");
    assert_eq!(std::fs::read(&dest).unwrap(), b"keep");
    assert!(!part.exists());
}

#[test]
fn progress_reports_bytes_or_time() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    let src = rgb_avi(dir.path(), "rgb.avi", 32, 24, 5, 25);
    let total = std::fs::metadata(&src).unwrap().len();
    let mut reports: Vec<Progress> = Vec::new();
    let dest = dir.path().join("rgb.mkv");
    run(&rt, &src, &dest, &MediaOp::Remux { container: "matroska".into() }, &mut |p| reports.push(p), &mut go)
        .expect("remux");
    let last = reports.last().expect("reports");
    assert_eq!(last.input_total, total);
    assert!(last.input_bytes > 0 && last.input_bytes <= total, "{last:?}");
    assert!((last.secs - 5.0).abs() < 0.3, "{last:?}");
    assert!(reports.windows(2).all(|w| w[1].input_bytes >= w[0].input_bytes), "monotonic");
    // A trim reports the window done.
    reports.clear();
    let dest = dir.path().join("cut.mkv");
    run(
        &rt,
        &src,
        &dest,
        &MediaOp::Trim { start: 1.0, end: 3.0, stream_copy: true },
        &mut |p| reports.push(p),
        &mut go,
    )
    .expect("trim");
    let last = reports.last().expect("reports");
    assert!((last.secs - 2.0).abs() <= 0.25, "{last:?}");
    assert!(last.input_bytes >= total * 9 / 10, "the window is done: {last:?}");
}

#[test]
fn corrupt_and_unfit_inputs_are_errors() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    let junk = dir.path().join("junk.mp4");
    std::fs::write(&junk, (0..8192u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect::<Vec<u8>>())
        .unwrap();
    let dest = dir.path().join("junk.mkv");
    assert!(plan(&rt, &junk, &MediaOp::Remux { container: "matroska".into() }).is_err());
    assert!(
        run(&rt, &junk, &dest, &MediaOp::Remux { container: "matroska".into() }, &mut no_progress, &mut go).is_err()
    );
    assert!(!dest.exists() && !dir.path().join("junk.mkv.part").exists());
    // A truncated AVI still converts what it has, or fails cleanly; never a panic.
    let mut avi = std::fs::read(rgb_avi(dir.path(), "rgb.avi", 16, 16, 5, 6)).unwrap();
    avi.truncate(avi.len() * 2 / 3);
    let cut = dir.path().join("cut.avi");
    std::fs::write(&cut, &avi).unwrap();
    let dest = dir.path().join("cut.mkv");
    let _ = run(&rt, &cut, &dest, &MediaOp::Remux { container: "matroska".into() }, &mut no_progress, &mut go);
    assert!(!dir.path().join("cut.mkv.part").exists());
    // Nothing fits the container.
    let e = plan(&rt, &fixture("rotated.mp4"), &MediaOp::Remux { container: "webm".into() }).unwrap_err();
    assert!(matches!(e, MediaError::Unsupported(_)), "{e}");
    let e = plan(&rt, &fixture("rotated.mp4"), &MediaOp::Remux { container: "no-such-muxer".into() }).unwrap_err();
    assert!(matches!(e, MediaError::Unsupported(_)), "{e}");
    // A missing input.
    let e = plan(&rt, &dir.path().join("missing.mp4"), &MediaOp::Remux { container: "mp4".into() }).unwrap_err();
    assert!(matches!(e, MediaError::Io(_)), "{e}");
}

#[test]
fn plan_lists_copy_versus_encode() {
    let rt = rt!();
    let src = fixture("rotated.mp4");
    let p = plan(&rt, &src, &MediaOp::Convert { preset: PresetId::MkvCopy, video_stream: None, audio_stream: None })
        .expect("plan");
    assert_eq!(p.container, "matroska");
    assert_eq!(p.streams, vec![(0, "copy".to_string()), (1, "copy".to_string())]);
    assert!((p.duration.expect("duration") - 1.0).abs() < 0.1);
    assert!(p.estimated_bytes.is_some_and(|b| b > 1000), "{:?}", p.estimated_bytes);
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    // Chapters are not carried over: the plan says so.
    let p = plan(&rt, &fixture("hdr-chapters.mkv"), &MediaOp::Remux { container: "mp4".into() }).expect("plan");
    assert_eq!(p.streams, vec![(0, "copy".to_string())]);
    assert!(p.warnings.iter().any(|w| w.contains("chapters")), "{:?}", p.warnings);
    if rt.has_encoder("libvpx-vp9") && rt.has_encoder("libopus") {
        let p =
            plan(&rt, &src, &MediaOp::Convert { preset: PresetId::WebmVp9, video_stream: None, audio_stream: None })
                .expect("plan");
        assert_eq!(p.streams, vec![(0, "encode libvpx-vp9".to_string()), (1, "encode libopus".to_string())]);
        assert_eq!(p.ext, "webm");
    }
    // Trim plans carry the window's duration.
    let p = plan(&rt, &src, &MediaOp::Trim { start: 0.2, end: 0.7, stream_copy: true }).expect("plan");
    assert!((p.duration.expect("duration") - 0.5).abs() < 1e-9);
    assert_eq!(p.container, "mp4");
    // Explicit stream picks are validated.
    let e =
        plan(&rt, &src, &MediaOp::Convert { preset: PresetId::AudioWav, video_stream: None, audio_stream: Some(0) })
            .unwrap_err();
    assert!(matches!(e, MediaError::NoStream(StreamKind::Audio)), "{e}");
}
