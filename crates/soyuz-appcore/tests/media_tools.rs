//! The media tools dialog model (`models::media_tools`, Phase V) against real FFmpeg
//! libraries: the probe and the dry-run plan arrive from their workers, the automatic
//! output name follows the operation, validation (trim window, times, names), preset
//! filtering and the job specs. Skips with a note without a runtime unless
//! `SOYUZ_MEDIA_TESTS=1` (then a missing runtime is a failure).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::models::media_tools::{
    AudioCodecChoice, ConflictChoice, ImageFormat, InputState, MediaTool, MediaToolsModel, PlanState,
};
use soyuz_core::ConflictResolution;
use soyuz_jobs::{JobKind, Priority};
use soyuz_media::transcode::MediaOp;
use soyuz_media::{DIR_ENV, PresetId, Runtime, fixtures, search_dirs};

fn runtime() -> Option<Arc<Runtime>> {
    let strict = std::env::var("SOYUZ_MEDIA_TESTS").is_ok_and(|v| v == "1");
    let dirs: Vec<PathBuf> = match std::env::var_os(DIR_ENV) {
        Some(d) => vec![PathBuf::from(d)],
        None => search_dirs("", &std::env::temp_dir().join("soyuz-appcore-media-tools-config")),
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

/// A Motion-JPEG AVI of `n` frames at `fps`, on disk.
fn mjpeg_avi(dir: &Path, name: &str, fps: u32, n: usize) -> PathBuf {
    let frame = fixtures::jpeg_solid(32, 24, [40, 200, 40]);
    let frames: Vec<Vec<u8>> = (0..n).map(|_| frame.clone()).collect();
    let path = dir.join(name);
    std::fs::write(&path, fixtures::avi(32, 24, fps, fixtures::AviCodec::Mjpeg, &frames)).unwrap();
    path
}

fn tone_wav(dir: &Path, name: &str) -> PathBuf {
    let samples = fixtures::sine(22_050, 440.0, 0.5, 22_050);
    let path = dir.join(name);
    std::fs::write(&path, fixtures::wav(22_050, 1, &samples)).unwrap();
    path
}

fn model(tool: MediaTool, sources: Vec<PathBuf>, existing: &[&str], rt: &Arc<Runtime>) -> MediaToolsModel {
    let existing = existing.iter().map(|s| s.to_string()).collect();
    MediaToolsModel::new(tool, sources, existing, Arc::clone(rt), Arc::new(|| {}))
}

/// Pump `sync` until `pred` holds (the workers are quick; 10 s is generous).
fn settle(m: &mut MediaToolsModel, pred: impl Fn(&MediaToolsModel) -> bool) {
    let start = Instant::now();
    loop {
        m.sync();
        if pred(m) {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "timed out; input {:?} plan {:?}", m.input, m.plan);
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn ready(m: &MediaToolsModel) -> bool {
    matches!(m.input, InputState::Ready(_)) && matches!(m.plan, PlanState::Ready(_) | PlanState::Failed(_))
}

fn single_spec(m: &MediaToolsModel) -> soyuz_jobs::JobSpec {
    let mut specs = m.specs().unwrap_or_else(|e| panic!("specs: {e}"));
    assert_eq!(specs.len(), 1);
    specs.remove(0)
}

#[test]
fn convert_plans_names_and_submits_per_file() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    let clip = mjpeg_avi(dir.path(), "clip.avi", 5, 10);
    let other = mjpeg_avi(dir.path(), "other.avi", 5, 4);
    let mut m = model(MediaTool::Convert, vec![clip.clone(), other.clone()], &["clip.avi", "other.avi"], &rt);
    assert_eq!(
        m.presets.iter().map(|p| p.id).collect::<Vec<_>>(),
        soyuz_media::presets(&rt).iter().map(|p| p.id).collect::<Vec<_>>(),
        "presets are the runtime's"
    );
    assert_eq!(m.presets[0].id, PresetId::MkvCopy);
    assert_eq!(m.name, "clip.mkv", "the automatic name before the probe");
    assert!(m.multi());
    assert!(m.source_line().contains("clip.avi"), "{}", m.source_line());
    settle(&mut m, ready);
    let InputState::Ready(i) = &m.input else { panic!("{:?}", m.input) };
    assert!(i.has_video && !i.has_audio);
    assert!((i.duration.unwrap() - 2.0).abs() < 0.3, "{:?}", i.duration);
    assert_eq!(i.streams.len(), 1);
    assert!(i.streams[0].1.starts_with("video mjpeg 32\u{d7}24"), "{}", i.streams[0].1);
    let PlanState::Ready(p) = &m.plan else { panic!("{:?}", m.plan) };
    assert_eq!(p.ext, "mkv");
    assert_eq!(p.streams, vec![(0, "copy".to_string())]);
    let lines = m.plan_lines();
    assert!(lines.iter().any(|(_, l)| l.contains("copy")), "{lines:?}");
    assert!(lines.iter().any(|(_, l)| l.contains("2 in all")), "one job per file: {lines:?}");
    assert!(m.error().is_none(), "{:?}", m.error());
    // Another preset: the name follows, the plan is refreshed.
    let mp4 = m.presets.iter().position(|p| p.id == PresetId::Mp4Copy).expect("mp4-copy is always available");
    m.set_preset(mp4);
    assert_eq!(m.name, "clip.mp4");
    assert!(matches!(m.plan, PlanState::Pending));
    settle(&mut m, |m| matches!(m.plan, PlanState::Ready(ref p) if p.ext == "mp4"));
    let specs = m.specs().unwrap();
    assert_eq!(specs.len(), 2);
    for (spec, (src, dest)) in specs.iter().zip([(&clip, "clip.mp4"), (&other, "other.mp4")]) {
        assert_eq!(spec.options.priority, Priority::Low);
        assert!(spec.options.journal);
        assert_eq!(spec.options.conflict, None, "ask");
        match &spec.kind {
            JobKind::Media { src: s, dest: d, op } => {
                assert_eq!(s, src);
                assert_eq!(d, &dir.path().join(dest));
                assert_eq!(op, &MediaOp::Convert { preset: PresetId::Mp4Copy, video_stream: None, audio_stream: None });
            }
            other => panic!("{other:?}"),
        }
    }
    assert!(matches!(m.submit().unwrap(), Cmd::Batch(v) if v.len() == 2));
    // A typed name stays; the second file still gets the typed extension.
    m.set_name("movie.mkv");
    let mkv = m.presets.iter().position(|p| p.id == PresetId::MkvCopy).unwrap();
    m.set_preset(mkv);
    assert_eq!(m.name, "movie.mkv");
    let specs = m.specs().unwrap();
    assert!(matches!(&specs[1].kind, JobKind::Media { dest, .. } if dest.ends_with("other.mkv")));
}

#[test]
fn trim_validates_the_window_against_the_duration() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    let clip = mjpeg_avi(dir.path(), "clip.avi", 5, 10);
    let mut m = model(MediaTool::Trim, vec![clip.clone()], &["clip.avi"], &rt);
    assert_eq!(m.start, "0");
    assert!(m.end.is_empty(), "the end waits for the probe");
    assert!(m.error().is_some(), "no window yet: {:?}", m.error());
    settle(&mut m, |m| matches!(m.input, InputState::Ready(_)));
    assert_eq!(m.end, "0:02.0", "the end follows the probed duration");
    assert_eq!(m.name, "clip (2).avi", "a trim keeps the container, never the source's name");
    settle(&mut m, ready);
    assert!(m.error().is_none(), "{:?}", m.error());
    m.set_start("5");
    assert!(m.error().unwrap().contains("only"), "{:?}", m.error());
    m.set_start("1.5");
    m.set_end("1");
    assert!(m.error().unwrap().contains("after the start"), "{:?}", m.error());
    m.set_start("abc");
    assert!(m.error().unwrap().contains("seconds"), "{:?}", m.error());
    m.set_start("0:00.5");
    m.set_end("1.5");
    m.set_stream_copy(false);
    settle(&mut m, ready);
    let spec = single_spec(&m);
    assert!(
        matches!(spec.kind, JobKind::Media { op: MediaOp::Trim { start, end, stream_copy: false }, .. } if start == 0.5 && end == 1.5)
    );
    // Naming rules.
    m.set_name("clip.avi");
    assert!(m.error().unwrap().contains("replace the source"), "{:?}", m.error());
    m.set_name("");
    assert!(m.error().unwrap().contains("Type"), "{:?}", m.error());
    m.set_name("bad/name.avi");
    assert!(m.error().unwrap().contains("valid"), "{:?}", m.error());
    m.set_name("cut.avi");
    assert!(m.error().is_none(), "{:?}", m.error());
    assert!(matches!(m.submit().unwrap(), Cmd::SubmitJob(_)));
}

#[test]
fn frame_and_audio_tools_follow_the_streams() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    let wav = tone_wav(dir.path(), "tone.wav");
    // Save frame needs video.
    let mut m = model(MediaTool::SaveFrame, vec![wav.clone()], &["tone.wav"], &rt);
    assert_eq!(m.name, "tone.png");
    settle(&mut m, |m| matches!(m.input, InputState::Ready(_)));
    assert!(m.error().unwrap().contains("no video"), "{:?}", m.error());
    m.set_format(ImageFormat::Jpeg);
    assert_eq!(m.name, "tone.jpg");
    // Extract audio: the copy keeps the codec's container, a codec picks its own.
    m.set_tool(MediaTool::ExtractAudio);
    settle(&mut m, ready);
    assert!(m.error().is_none(), "{:?}", m.error());
    let copy_ext = soyuz_media::audio_copy_container("pcm_s16le").1;
    assert_eq!(m.name, format!("tone (2).{copy_ext}"));
    m.set_codec(AudioCodecChoice::Flac);
    assert_eq!(m.name, "tone.flac");
    settle(&mut m, ready);
    if rt.has_encoder("flac") {
        assert!(m.error().is_none(), "{:?}", m.error());
        let spec = single_spec(&m);
        assert!(matches!(
            spec.kind,
            JobKind::Media { op: MediaOp::ExtractAudio { codec: AudioCodecChoice::Flac }, .. }
        ));
    } else {
        assert!(m.error().is_some(), "no flac encoder: the plan says so");
    }
    // A video clip: the frame tool plans a PNG.
    let clip = mjpeg_avi(dir.path(), "clip.avi", 5, 10);
    let mut m = model(MediaTool::SaveFrame, vec![clip], &["clip.avi"], &rt);
    m.set_at("1.5");
    settle(&mut m, ready);
    assert!(m.error().is_none(), "{:?}", m.error());
    m.set_at("9");
    assert!(m.error().unwrap().contains("only"), "{:?}", m.error());
    m.set_at("0.2");
    let spec = single_spec(&m);
    assert!(
        matches!(spec.kind, JobKind::Media { op: MediaOp::Frame { at, format: ImageFormat::Png, .. }, .. } if at == 0.2)
    );
}

#[test]
fn taken_names_offer_the_conflict_choice() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    let clip = mjpeg_avi(dir.path(), "clip.avi", 5, 4);
    let mut m = model(MediaTool::Convert, vec![clip], &["clip.avi", "clip.mkv"], &rt);
    assert_eq!(m.name, "clip (2).mkv", "the default never collides");
    assert!(!m.name_exists());
    m.set_name("clip.mkv");
    assert!(m.name_exists());
    assert!(!m.name_exists() || m.error().is_none() || matches!(m.input, InputState::Pending));
    settle(&mut m, ready);
    m.set_conflict(ConflictChoice::Overwrite);
    let spec = single_spec(&m);
    assert_eq!(spec.options.conflict, Some(ConflictResolution::Overwrite));
    assert!(matches!(spec.kind, JobKind::Media { ref dest, .. } if dest.ends_with("clip.mkv")));
    m.set_conflict(ConflictChoice::Skip);
    assert_eq!(single_spec(&m).options.conflict, Some(ConflictResolution::Skip));
}
