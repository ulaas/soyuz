//! `JobKind::Media` (Phase V): the transcoder as a job. The with-libraries tests load the
//! FFmpeg runtime from `SOYUZ_FFMPEG_DIR` (or the usual search folders) and skip with a
//! note when none loads, unless `SOYUZ_MEDIA_TESTS=1` makes that a failure (CI). The
//! runtime is process-wide, so the "no runtime" test only runs while none is loaded.

use std::time::Duration;

use soyuz_core::ConflictResolution;
use soyuz_media::transcode::{ImageFormat, MediaOp};
use soyuz_media::{DIR_ENV, PresetId, Runtime, fixtures};

use super::*;
use crate::journal::{self, JournalJob};
use crate::{JobKindBrief, JobOptions, PromptResponse, UndoRecord};

/// The folders a `Media` job may load the libraries from (see `EngineConfig::media_dirs`).
fn media_dirs() -> Vec<PathBuf> {
    match std::env::var_os(DIR_ENV) {
        Some(d) => vec![PathBuf::from(d)],
        None => soyuz_media::search_dirs("", &std::env::temp_dir().join("soyuz-jobs-media-tests-config")),
    }
}

/// A harness whose engine may load the libraries; `None` (with a note) when they are
/// missing and `SOYUZ_MEDIA_TESTS` is not set.
fn harness() -> Option<(H, Arc<Runtime>)> {
    let strict = std::env::var("SOYUZ_MEDIA_TESTS").is_ok_and(|v| v == "1");
    let dirs = media_dirs();
    let rt = match Runtime::get(&dirs) {
        Ok(rt) => rt,
        Err(e) if strict => panic!("SOYUZ_MEDIA_TESTS=1 but no FFmpeg runtime loads: {e}"),
        Err(e) => {
            eprintln!("skipped: no FFmpeg runtime ({e}); set {DIR_ENV} to a folder with the libraries");
            return None;
        }
    };
    let h = H::with(move |c| c.media_dirs = dirs);
    Some((h, rt))
}

macro_rules! harness {
    () => {
        match harness() {
            Some(h) => h,
            None => return,
        }
    };
}

/// A Motion-JPEG AVI of `n` identical frames at `fps`, on disk.
fn mjpeg_avi(h: &H, name: &str, w: u32, h_px: u32, fps: u32, n: usize) -> PathBuf {
    let frame = fixtures::jpeg_solid(w, h_px, [200, 40, 40]);
    let frames: Vec<Vec<u8>> = (0..n).map(|_| frame.clone()).collect();
    let path = h.p(name);
    write(&path, &fixtures::avi(w, h_px, fps, fixtures::AviCodec::Mjpeg, &frames));
    path
}

fn media_kind(src: &Path, dest: &Path, op: MediaOp) -> JobKind {
    JobKind::Media { src: src.to_path_buf(), dest: dest.to_path_buf(), op }
}

fn convert(src: &Path, dest: &Path, preset: PresetId) -> JobKind {
    media_kind(src, dest, MediaOp::Convert { preset, video_stream: None, audio_stream: None })
}

fn part_of(dest: &Path) -> PathBuf {
    dest.with_file_name(format!("{}.part", dest.file_name().unwrap().to_string_lossy()))
}

#[test]
fn without_runtime_the_job_fails_with_a_readable_message() {
    if Runtime::loaded().is_some() {
        eprintln!("skipped: a runtime is loaded in this process");
        return;
    }
    let h = H::new();
    let src = mjpeg_avi(&h, "clip.avi", 32, 24, 5, 5);
    let dest = h.p("clip.mkv");
    let id = h.submit(convert(&src, &dest, PresetId::MkvCopy));
    match h.done(id) {
        JobState::Failed(e) => {
            assert_eq!(e.kind, soyuz_core::FsErrorKind::Unsupported);
            assert!(e.to_string().contains("FFmpeg libraries are not loaded"), "{e}");
            assert!(!dest.exists() && !part_of(&dest).exists());
        }
        // A sibling test loaded the process-wide runtime meanwhile: nothing to check.
        JobState::Done(_) if Runtime::loaded().is_some() => eprintln!("skipped: a runtime loaded meanwhile"),
        other => panic!("{other:?}"),
    }
    assert_eq!(h.snap(id).kind, JobKindBrief::Media);
}

#[test]
fn convert_writes_the_output_and_undo_trashes_it_while_unchanged() {
    let (h, _rt) = harness!();
    let src = mjpeg_avi(&h, "clip.avi", 32, 24, 5, 10);
    let dest = h.p("out/clip.mkv");
    std::fs::create_dir_all(h.p("out")).unwrap();
    let mut spec = JobSpec::new(convert(&src, &dest, PresetId::MkvCopy));
    spec.options.priority = crate::Priority::Low;
    let id = h.engine.submit(spec);
    let s = h.ok(id);
    assert_eq!((s.items_ok, s.items_failed, s.items_skipped), (1, 0, 0), "{s:?}");
    assert!(dest.is_file(), "output written");
    assert!(!part_of(&dest).exists(), "no temporary left");
    let snap = h.snap(id);
    assert_eq!(snap.kind, JobKindBrief::Media);
    assert!(
        snap.progress.bytes_done > 0 && snap.progress.bytes_done <= snap.progress.bytes_total,
        "{:?}",
        snap.progress
    );
    assert!(snap.label.starts_with("Convert \u{201c}clip.avi\u{201d}"), "{}", snap.label);
    let hist = h.engine.history();
    assert_eq!(hist.len(), 1);
    assert!(hist[0].undoable);
    assert_eq!(hist[0].records, vec![UndoRecord::Copied { dst: dest.clone() }]);
    assert_eq!(hist[0].stamps.len(), 1, "the output is identity-stamped");
    // Undo removes the output (the test volume has no trash: confirm the permanent delete).
    let u = h.engine.undo(hist[0].id).expect("undoable");
    let s = settle(&h, u, true);
    assert_eq!(s.items_failed, 0, "{s:?}");
    assert!(!dest.exists(), "output gone after undo");
    assert!(src.is_file(), "the source is untouched");
    assert!(h.engine.history()[0].undone);
}

#[test]
fn undo_leaves_an_output_that_changed_since() {
    let (h, _rt) = harness!();
    let src = mjpeg_avi(&h, "clip.avi", 32, 24, 5, 6);
    let dest = h.p("clip.mkv");
    let id = h.submit(convert(&src, &dest, PresetId::MkvCopy));
    h.ok(id);
    // Someone appended to the output: its identity no longer matches the stamp.
    std::thread::sleep(Duration::from_millis(20));
    {
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new().append(true).open(&dest).unwrap();
        f.write_all(b"changed").unwrap();
    }
    let u = h.engine.undo(h.engine.history()[0].id).expect("undoable");
    let s = settle(&h, u, true);
    assert_eq!(s.items_failed, 1, "{s:?}");
    assert!(dest.is_file(), "a changed output is left alone");
    assert!(s.errors[0].1.to_string().contains("changed after the operation"), "{:?}", s.errors);
}

#[test]
fn conflicts_on_an_existing_output_follow_the_answer() {
    let (h, _rt) = harness!();
    let src = mjpeg_avi(&h, "clip.avi", 32, 24, 5, 6);
    let dest = h.p("clip.mkv");
    write(&dest, b"old output");
    // Skip keeps the old file.
    let id = h.submit(convert(&src, &dest, PresetId::MkvCopy));
    assert!(matches!(h.prompt(id), JobPrompt::Conflict(c) if c.dst.path == dest), "asks about the output");
    h.engine.respond(id, PromptResponse::Conflict { resolution: ConflictResolution::Skip, apply_to_all: false });
    let s = h.ok(id);
    assert_eq!((s.items_ok, s.items_skipped), (0, 1), "{s:?}");
    assert_eq!(read(&dest), b"old output");
    assert!(!h.engine.history().iter().any(|e| e.undoable), "nothing to undo");
    // Keep both writes next to it.
    let id = h.submit(convert(&src, &dest, PresetId::MkvCopy));
    h.prompt(id);
    h.engine.respond(id, PromptResponse::Conflict { resolution: ConflictResolution::KeepBoth, apply_to_all: false });
    let s = h.ok(id);
    assert_eq!(s.items_ok, 1, "{s:?}");
    assert_eq!(read(&dest), b"old output");
    let kept: Vec<PathBuf> = std::fs::read_dir(h.dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "mkv") && *p != dest)
        .collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert!(std::fs::metadata(&kept[0]).unwrap().len() > 100);
    assert_eq!(h.engine.history().last().unwrap().records, vec![UndoRecord::Copied { dst: kept[0].clone() }]);
    // Overwrite without a trash asks, then replaces in place (not undoable).
    let id = h.submit(convert(&src, &dest, PresetId::MkvCopy));
    h.prompt(id);
    h.engine.respond(id, PromptResponse::Conflict { resolution: ConflictResolution::Overwrite, apply_to_all: false });
    assert!(matches!(h.prompt(id), JobPrompt::TrashUnsupported { path, .. } if path == dest));
    h.engine.respond(id, PromptResponse::TrashUnsupported { permanent: true, apply_to_all: false });
    let s = h.ok(id);
    assert_eq!(s.items_ok, 1, "{s:?}");
    assert_ne!(read(&dest), b"old output", "replaced");
    assert!(!h.engine.history().last().unwrap().undoable, "no backup, no undo");
    let leftovers: Vec<PathBuf> = std::fs::read_dir(h.dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| journal::is_leftover_name(&p.file_name().unwrap().to_string_lossy()))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    // Overwrite declined keeps the old file.
    write(&dest, b"old again");
    let id = h.submit(convert(&src, &dest, PresetId::MkvCopy));
    h.prompt(id);
    h.engine.respond(id, PromptResponse::Conflict { resolution: ConflictResolution::Overwrite, apply_to_all: false });
    h.prompt(id);
    h.engine.respond(id, PromptResponse::TrashUnsupported { permanent: false, apply_to_all: false });
    let s = h.ok(id);
    assert_eq!(s.items_skipped, 1, "{s:?}");
    assert_eq!(read(&dest), b"old again");
}

#[test]
fn cancel_leaves_no_output_behind() {
    let (h, rt) = harness!();
    if !rt.has_encoder("mpeg4") {
        eprintln!("skipped: no mpeg4 encoder");
        return;
    }
    // A software encode of 1,500 frames takes long enough to be interrupted.
    let src = mjpeg_avi(&h, "long.avi", 320, 240, 25, 1500);
    let dest = h.p("long.mp4");
    let id = h.submit(convert(&src, &dest, PresetId::Mp4Mpeg4));
    h.wait_state(id, |s| *s == JobState::Running || s.is_terminal());
    h.engine.cancel(id);
    assert_eq!(h.done(id), JobState::Cancelled);
    assert!(!dest.exists(), "no output after cancel");
    assert!(!part_of(&dest).exists(), "no temporary after cancel");
    assert!(h.engine.history().is_empty());
}

#[test]
fn pause_holds_the_transcoder_between_packets() {
    let (h, rt) = harness!();
    if !rt.has_encoder("mpeg4") {
        eprintln!("skipped: no mpeg4 encoder");
        return;
    }
    let src = mjpeg_avi(&h, "long.avi", 320, 240, 25, 800);
    let dest = h.p("long.mp4");
    let id = h.submit(convert(&src, &dest, PresetId::Mp4Mpeg4));
    h.wait_state(id, |s| *s == JobState::Running || s.is_terminal());
    h.engine.pause(id);
    let paused = h.wait_state(id, |s| *s == JobState::Paused || s.is_terminal());
    if paused.state.is_terminal() {
        eprintln!("note: the encode finished before it could be paused");
        return;
    }
    let done_at_pause = paused.progress.bytes_done;
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(h.snap(id).progress.bytes_done, done_at_pause, "no progress while paused");
    h.engine.resume(id);
    let s = h.ok(id);
    assert_eq!(s.items_ok, 1, "{s:?}");
    assert!(dest.is_file());
}

#[test]
fn frame_and_audio_tools_run_as_jobs() {
    let (h, rt) = harness!();
    let src = mjpeg_avi(&h, "clip.avi", 32, 24, 5, 10);
    let png = h.p("clip.png");
    let op = MediaOp::Frame { at: 0.5, format: ImageFormat::Png, max_px: Some(64) };
    let id = h.submit(media_kind(&src, &png, op));
    let s = h.ok(id);
    assert_eq!(s.items_ok, 1, "{s:?}");
    assert!(png.is_file());
    assert!(read(&png).starts_with(b"\x89PNG"));
    assert!(h.snap(id).label.starts_with("Save a frame of"), "{}", h.snap(id).label);
    // A WAV through the transcoder: WAV (16-bit PCM) needs only the pcm encoder.
    let samples = fixtures::sine(22_050, 440.0, 0.5, 22_050);
    let wav = h.p("tone.wav");
    write(&wav, &fixtures::wav(22_050, 1, &samples));
    if rt.has_encoder("flac") {
        let flac = h.p("tone.flac");
        let id =
            h.submit(media_kind(&wav, &flac, MediaOp::ExtractAudio { codec: soyuz_media::AudioCodecChoice::Flac }));
        let s = h.ok(id);
        assert_eq!(s.items_ok, 1, "{s:?}");
        assert!(read(&flac).starts_with(b"fLaC"));
        assert!(h.snap(id).label.starts_with("Extract audio from"), "{}", h.snap(id).label);
    }
    // A trim by stream copy.
    let cut = h.p("cut.avi");
    let id = h.submit(media_kind(&src, &cut, MediaOp::Trim { start: 0.4, end: 1.4, stream_copy: true }));
    let s = h.ok(id);
    assert_eq!(s.items_ok, 1, "{s:?}");
    let info = soyuz_media::probe(&rt, &soyuz_media::Source::File(cut.clone())).expect("probe the cut");
    assert!((info.duration.expect("duration") - 1.0).abs() <= 0.25, "{:?}", info.duration);
    // An input without the stream the tool needs fails as an item, readably.
    let no_video = h.p("tone.png");
    let id = h.submit(media_kind(&wav, &no_video, MediaOp::Frame { at: 0.0, format: ImageFormat::Png, max_px: None }));
    match h.done(id) {
        JobState::Failed(e) => assert!(e.to_string().contains("no video stream"), "{e}"),
        other => panic!("{other:?}"),
    }
    assert!(!no_video.exists());
}

#[test]
fn journal_round_trips_the_kind() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(journal::JOURNAL_FILE);
    let kind = media_kind(
        Path::new("/v/clip.mov"),
        Path::new("/v/clip.webm"),
        MediaOp::Convert { preset: PresetId::WebmVp9, video_stream: Some(0), audio_stream: None },
    );
    let j = JournalJob {
        id: 7,
        label: crate::job::default_label(&kind),
        kind: kind.clone(),
        options: JobOptions { priority: crate::Priority::Low, journal: true, ..JobOptions::default() },
        started: std::time::SystemTime::UNIX_EPOCH,
        records: Vec::new(),
        stamps: Vec::new(),
        items_done: 0,
        items_total: 1,
        bytes_done: 10,
        bytes_total: 100,
    };
    std::fs::write(&p, journal::to_json(std::slice::from_ref(&j))).unwrap();
    let back = journal::read_journal(&p).unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].kind, kind);
    assert_eq!(back[0].options.priority, crate::Priority::Low);
    assert!(journal::journals(&kind));
    assert_eq!(journal::scrub(&kind), kind, "nothing secret in a media job");
    assert_eq!(journal::resume_kind(&kind), Some(kind.clone()), "re-run as a whole");
    assert_eq!(crate::kinds::retry_kind(&kind, &crate::JobSummary::default()), Some(kind));
}

#[test]
fn a_resumed_trim_keeps_its_op_label_and_options() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(journal::JOURNAL_FILE);
    let kind = media_kind(
        Path::new("/v/clip.mp4"),
        Path::new("/v/clip.cut.mp4"),
        MediaOp::Trim { start: 1.0, end: 2.5, stream_copy: true },
    );
    let media = crate::MediaOptions { threads: 2, faststart: false };
    let j = JournalJob {
        id: 9,
        label: crate::job::default_label(&kind),
        kind: kind.clone(),
        options: JobOptions { journal: true, media, ..JobOptions::default() },
        started: std::time::SystemTime::UNIX_EPOCH,
        records: Vec::new(),
        stamps: Vec::new(),
        items_done: 0,
        items_total: 1,
        bytes_done: 0,
        bytes_total: 0,
    };
    assert_eq!(j.label, "Trim \u{201c}clip.mp4\u{201d} to \u{201c}clip.cut.mp4\u{201d}");
    std::fs::write(&p, journal::to_json(std::slice::from_ref(&j))).unwrap();
    let back = journal::read_journal(&p).unwrap().remove(0);
    assert_eq!(back.kind, kind, "the op survives the journal");
    assert_eq!(back.options.media, media, "and so do the transcoder options");
    // Journals written before the options existed read as the defaults.
    let old = journal::to_json(std::slice::from_ref(&j)).replace(",\"media\":{\"threads\":2,\"faststart\":false}", "");
    assert!(!old.contains("faststart"), "{old}");
    std::fs::write(&p, old).unwrap();
    assert_eq!(journal::read_journal(&p).unwrap()[0].options.media, crate::MediaOptions::default());
    // The resumed job is still a trim (its label names the op, not "Convert").
    let h = H::new();
    let id = h.engine.resume_journaled(&back).expect("a media job is re-run as a whole");
    let snap = h.snap(id);
    assert_eq!(snap.kind, JobKindBrief::Media);
    assert_eq!(snap.label, "Resume: Trim \u{201c}clip.mp4\u{201d} to \u{201c}clip.cut.mp4\u{201d}");
    h.engine.cancel(id);
    h.done(id);
}
