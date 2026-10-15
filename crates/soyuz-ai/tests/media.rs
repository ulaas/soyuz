//! Media tools through `dispatch` (Phase V): `media_info` and the dry-run plans of
//! `media_convert` / `media_extract_audio` / `media_trim` / `media_frame`, and one
//! approved plan run as a job on a real engine. The with-libraries tests load the FFmpeg
//! runtime from `SOYUZ_FFMPEG_DIR` (or the usual search folders) and skip with a note when
//! none loads, unless `SOYUZ_MEDIA_TESTS=1` makes that a failure. Everything happens in a
//! temp dir.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use soyuz_ai::tools::{
    CallContext, FsToolHost, MediaAccess, Plan, PlanConflict, PlanOp, Risk, Scope, ToolError, ToolResponse, dispatch,
};
use soyuz_core::config::AiPrivacy;
use soyuz_jobs::{EngineConfig, JobEngine, JobKind, JobState, Priority};
use soyuz_media::{DIR_ENV, MediaOp, Runtime, fixtures};

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    host: FsToolHost,
    dirs: Vec<PathBuf>,
}

fn media_dirs() -> Vec<PathBuf> {
    match std::env::var_os(DIR_ENV) {
        Some(d) => vec![PathBuf::from(d)],
        None => soyuz_media::search_dirs("", &std::env::temp_dir().join("soyuz-ai-media-tests-config")),
    }
}

fn fixture(media: MediaAccess) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    let frame = fixtures::jpeg_solid(32, 24, [200, 40, 40]);
    let frames: Vec<Vec<u8>> = (0..10).map(|_| frame.clone()).collect();
    std::fs::write(root.join("clip.avi"), fixtures::avi(32, 24, 5, fixtures::AviCodec::Mjpeg, &frames)).unwrap();
    let samples = fixtures::sine(8000, 440.0, 0.3, 8000);
    std::fs::write(root.join("tone.wav"), fixtures::wav(8000, 1, &samples)).unwrap();
    let host = FsToolHost::new(Scope::new(vec![root.clone()], &[], false)).with_media(media);
    Fixture { _dir: dir, root, host, dirs: media_dirs() }
}

/// A fixture whose host may load the libraries; `None` (with a note) when they are
/// missing and `SOYUZ_MEDIA_TESTS` is not set.
fn with_libs() -> Option<Fixture> {
    let strict = std::env::var("SOYUZ_MEDIA_TESTS").is_ok_and(|v| v == "1");
    let dirs = media_dirs();
    if let Err(e) = Runtime::get(&dirs) {
        assert!(!strict, "SOYUZ_MEDIA_TESTS=1 but no FFmpeg runtime loads: {e}");
        eprintln!("skipped: no FFmpeg runtime ({e}); set {DIR_ENV} to a folder with the libraries");
        return None;
    }
    Some(fixture(MediaAccess { dirs, ..MediaAccess::default() }))
}

macro_rules! with_libs {
    () => {
        match with_libs() {
            Some(f) => f,
            None => return,
        }
    };
}

fn ctx() -> CallContext {
    CallContext::new("test", Risk::Write, AiPrivacy::MetadataOnly)
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn plan_of(r: Result<ToolResponse, ToolError>) -> Plan {
    match r {
        Ok(ToolResponse::Plan(p) | ToolResponse::NeedsApproval(p)) => p,
        Ok(ToolResponse::Data(d)) => panic!("expected a plan, got {d}"),
        Err(e) => panic!("expected a plan, got {e}"),
    }
}

fn err_of(r: Result<ToolResponse, ToolError>) -> ToolError {
    match r {
        Err(e) => e,
        Ok(r) => panic!("expected an error, got {}", r.payload()),
    }
}

#[test]
fn settings_refuse_the_tools_clearly() {
    let f = fixture(MediaAccess { ffmpeg: false, ..MediaAccess::default() });
    let clip = s(&f.root.join("clip.avi"));
    let e = err_of(dispatch(&f.host, "media_info", json!({"path": clip}), &ctx()));
    assert!(e.to_string().contains("[media] ffmpeg = false"), "{e}");
    let f = fixture(MediaAccess { tools: false, ..MediaAccess::default() });
    let clip = s(&f.root.join("clip.avi"));
    let e = err_of(dispatch(&f.host, "media_frame", json!({"path": clip, "at": 0, "dry_run": true}), &ctx()));
    assert!(e.to_string().contains("[media] tools = false"), "{e}");
    // Argument errors come first, without the libraries.
    let e = err_of(dispatch(&f.host, "media_trim", json!({"path": clip, "start": 3, "end": 1}), &ctx()));
    assert!(matches!(e, ToolError::InvalidArgs(_)), "{e}");
    // Scope: an output outside the allowed folders is refused before planning.
    let e = err_of(dispatch(
        &f.host,
        "media_convert",
        json!({"path": clip, "preset": "mkv-copy", "dest": s(&std::env::temp_dir().join("x.mkv"))}),
        &ctx(),
    ));
    assert!(matches!(e, ToolError::Scope(_)), "{e}");
}

#[test]
fn without_runtime_the_error_is_readable() {
    if Runtime::loaded().is_some() {
        eprintln!("skipped: a runtime is loaded in this process");
        return;
    }
    let f = fixture(MediaAccess::default());
    let clip = s(&f.root.join("clip.avi"));
    let e = err_of(dispatch(&f.host, "media_info", json!({"path": clip}), &ctx()));
    assert!(e.to_string().contains("FFmpeg libraries not loaded"), "{e}");
    let e = err_of(dispatch(&f.host, "media_convert", json!({"path": clip, "preset": "mkv-copy"}), &ctx()));
    assert!(e.to_string().contains("FFmpeg libraries not loaded"), "{e}");
}

#[test]
fn media_info_reports_probe_facts() {
    let f = with_libs!();
    let r = dispatch(&f.host, "media_info", json!({"path": s(&f.root.join("clip.avi"))}), &ctx()).unwrap();
    let ToolResponse::Data(v) = r else { panic!("expected data") };
    assert_eq!(v["container"], "avi", "{v}");
    let video = &v["streams"][0];
    assert_eq!(video["kind"], "video", "{v}");
    assert_eq!(video["codec"], "mjpeg");
    assert_eq!((video["width"].as_u64(), video["height"].as_u64()), (Some(32), Some(24)));
    assert!(v["duration"].as_f64().is_some_and(|d| (d - 2.0).abs() < 0.3), "{v}");
    assert!(v["presets"].as_array().unwrap().iter().any(|p| p == "mkv-copy"), "{v}");
    assert!(v["size"].as_u64().unwrap() > 0);
    let text = ToolResponse::Data(v).to_model_text("media_info");
    assert!(text.contains("trust=\"untrusted\""));
    let r = dispatch(&f.host, "media_info", json!({"path": s(&f.root.join("tone.wav"))}), &ctx()).unwrap();
    let ToolResponse::Data(v) = r else { panic!("expected data") };
    assert_eq!(v["streams"][0]["kind"], "audio", "{v}");
    assert_eq!(v["streams"][0]["sample_rate"], 8000);
    // Not a media file.
    std::fs::write(f.root.join("notes.txt"), "hello").unwrap();
    let e = err_of(dispatch(&f.host, "media_info", json!({"path": s(&f.root.join("notes.txt"))}), &ctx()));
    assert!(matches!(e, ToolError::Failed(_)), "{e}");
}

fn job_of(plan: &Plan) -> (&Path, &Path, &MediaOp) {
    match &plan.job().expect("a job").kind {
        JobKind::Media { src, dest, op } => (src, dest, op),
        k => panic!("expected a media job, got {k:?}"),
    }
}

#[test]
fn write_tools_plan_media_jobs() {
    let f = with_libs!();
    let clip = f.root.join("clip.avi");
    let c = ctx();
    // Convert: default name beside the input, the dialog's options.
    let plan = plan_of(dispatch(
        &f.host,
        "media_convert",
        json!({"path": s(&clip), "preset": "mkv-copy", "dry_run": true}),
        &c,
    ));
    assert!(plan.executable, "{:#?}", plan);
    assert_eq!(plan.tool, "media_convert");
    assert_eq!(plan.items[0].op, PlanOp::MediaConvert);
    let (src, dest, op) = job_of(&plan);
    assert_eq!((src, dest), (clip.as_path(), f.root.join("clip.mkv").as_path()));
    assert!(matches!(op, MediaOp::Convert { .. }));
    let spec = plan.job().unwrap();
    assert_eq!(spec.options.priority, Priority::Low);
    assert!(spec.options.journal && spec.options.conflict.is_none());
    assert!(plan.summary.contains("clip.mkv") && plan.summary.contains("stream 0"), "{}", plan.summary);
    // A taken name gets the next free one; an explicit taken output is a conflict.
    std::fs::write(f.root.join("clip.mkv"), b"old").unwrap();
    let plan = plan_of(dispatch(&f.host, "media_convert", json!({"path": s(&clip), "preset": "mkv-copy"}), &c));
    assert_eq!(job_of(&plan).1, f.root.join("clip (2).mkv"));
    let plan = plan_of(dispatch(
        &f.host,
        "media_convert",
        json!({"path": s(&clip), "preset": "mkv-copy", "dest": s(&f.root.join("clip.mkv")), "conflict": "overwrite"}),
        &c,
    ));
    assert!(
        matches!(&plan.items[0].conflict, Some(PlanConflict::Exists { resolution, .. }) if resolution == "overwrite")
    );
    assert!(plan.executable);
    assert_eq!(plan.job().unwrap().options.conflict, Some(soyuz_core::ConflictResolution::Overwrite));
    // The output can never be the input.
    let plan = plan_of(dispatch(
        &f.host,
        "media_convert",
        json!({"path": s(&clip), "preset": "mkv-copy", "dest": s(&clip)}),
        &c,
    ));
    assert!(!plan.executable);
    // Trim: the input's container, the window checked against the duration.
    let plan = plan_of(dispatch(&f.host, "media_trim", json!({"path": s(&clip), "start": 0.4, "end": 1.2}), &c));
    let (_, dest, op) = job_of(&plan);
    assert_eq!(dest.extension().unwrap(), "avi");
    assert_ne!(dest, clip);
    assert_eq!(op, &MediaOp::Trim { start: 0.4, end: 1.2, stream_copy: true });
    let e = err_of(dispatch(&f.host, "media_trim", json!({"path": s(&clip), "start": 30, "end": 40}), &c));
    assert!(matches!(e, ToolError::InvalidArgs(_)) && e.to_string().contains("beyond"), "{e}");
    // Frame: a picture beside the video; past the end is refused.
    let plan = plan_of(dispatch(&f.host, "media_frame", json!({"path": s(&clip), "at": 1, "format": "jpeg"}), &c));
    assert_eq!(job_of(&plan).1, f.root.join("clip.jpg"));
    let e = err_of(dispatch(&f.host, "media_frame", json!({"path": s(&clip), "at": 99}), &c));
    assert!(e.to_string().contains("beyond"), "{e}");
    // Extract audio: no audio in the clip; the tone gets the codec's extension.
    let e = err_of(dispatch(&f.host, "media_extract_audio", json!({"path": s(&clip)}), &c));
    assert!(e.to_string().contains("no audio"), "{e}");
    let tone = f.root.join("tone.wav");
    let plan = plan_of(dispatch(&f.host, "media_extract_audio", json!({"path": s(&tone), "codec": "flac"}), &c));
    assert_eq!(job_of(&plan).1, f.root.join("tone.flac"));
    // A preset this build cannot encode names the ones it can.
    let rt = Runtime::loaded().unwrap();
    let available: Vec<&str> = soyuz_media::presets(&rt).iter().map(|p| p.id.name()).collect();
    if !available.contains(&"mp4-hevc-hw") {
        let e = err_of(dispatch(&f.host, "media_convert", json!({"path": s(&clip), "preset": "mp4-hevc-hw"}), &c));
        assert!(e.to_string().contains("mkv-copy"), "{e}");
    }
    // Model-facing plan: no job spec, the summary and the item.
    let v: Value = ToolResponse::Plan(plan).payload();
    assert_eq!(v["plan"]["items"][0]["op"], "media_extract_audio", "{v}");
}

#[test]
fn an_approved_plan_runs_as_a_job() {
    let f = with_libs!();
    let clip = f.root.join("clip.avi");
    let plan = plan_of(dispatch(&f.host, "media_frame", json!({"path": s(&clip), "at": 0.5}), &ctx()));
    let spec = plan.job().unwrap().clone();
    let cfg = EngineConfig { media_dirs: f.dirs.clone(), ..EngineConfig::default() };
    let engine = JobEngine::new(cfg, Arc::new(soyuz_fs::RealProbe), Arc::new(|| {}));
    let id = engine.submit(spec);
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match engine.snapshot(id).map(|s| s.state) {
            Some(JobState::Done(s)) => {
                assert_eq!(s.items_failed, 0, "{s:?}");
                break;
            }
            Some(JobState::Failed(e)) => panic!("job failed: {e}"),
            Some(JobState::Cancelled) => panic!("job cancelled"),
            _ => {}
        }
        assert!(Instant::now() < deadline, "the job did not finish");
        std::thread::sleep(Duration::from_millis(20));
    }
    let png = f.root.join("clip.png");
    let bytes = std::fs::read(&png).unwrap();
    assert!(bytes.starts_with(b"\x89PNG"), "a PNG was written");
    // Undoable: the history has the entry.
    assert!(engine.history().iter().any(|h| h.undoable), "undoable history entry");
    engine.shutdown(Duration::from_secs(5));
}
