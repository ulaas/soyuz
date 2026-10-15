//! What survives the transcoder besides the streams: chapters, container and stream
//! tags, cover art (attached pictures), rotation (display matrix) and the MP4 / MOV
//! `moov` placement (faststart). Against real libraries (`SOYUZ_FFMPEG_DIR`), on the
//! committed fixtures (`tests/fixtures/README.md`).
//!
//! Without a runtime every test prints `skipped: …` and passes, unless
//! `SOYUZ_MEDIA_TESTS=1` is set: then a missing runtime is a failure (CI).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use soyuz_media::transcode::{AudioCodecChoice, Cancelled, MediaOp, Outcome, Progress, RunOptions, run_with};
use soyuz_media::{DIR_ENV, MediaInfo, PresetId, Runtime, Source, probe, search_dirs};

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

fn go() -> Result<(), Cancelled> {
    Ok(())
}

fn run_opts(rt: &Runtime, src: &Path, dest: &Path, op: &MediaOp, opts: &RunOptions) -> Outcome {
    let out =
        run_with(rt, src, dest, op, opts, &mut |_: Progress| {}, &mut go).unwrap_or_else(|e| panic!("{op:?}: {e}"));
    assert!(dest.is_file(), "{} written", dest.display());
    assert_eq!(out.out_bytes, std::fs::metadata(dest).unwrap().len(), "{op:?}: out_bytes is the file size");
    out
}

fn run_ok(rt: &Runtime, src: &Path, dest: &Path, op: &MediaOp) -> Outcome {
    run_opts(rt, src, dest, op, &RunOptions::default())
}

fn info(rt: &Runtime, p: &Path) -> MediaInfo {
    probe(rt, &Source::File(p.to_path_buf())).unwrap_or_else(|e| panic!("probe {}: {e}", p.display()))
}

fn convert(preset: PresetId) -> MediaOp {
    MediaOp::Convert { preset, video_stream: None, audio_stream: None }
}

/// The top-level box types of an MP4 / MOV file, in file order.
fn top_boxes(p: &Path) -> Vec<String> {
    let b = std::fs::read(p).unwrap();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 8 <= b.len() {
        let mut size = u32::from_be_bytes(b[i..i + 4].try_into().unwrap()) as u64;
        let kind = String::from_utf8_lossy(&b[i + 4..i + 8]).into_owned();
        if size == 1 {
            size = u64::from_be_bytes(b[i + 8..i + 16].try_into().unwrap());
        } else if size == 0 {
            size = (b.len() - i) as u64;
        }
        out.push(kind);
        assert!(size >= 8, "box size in {}", p.display());
        i += size as usize;
    }
    assert_eq!(i, b.len(), "the boxes of {} add up to its size", p.display());
    out
}

fn moov_first(p: &Path) -> bool {
    let boxes = top_boxes(p);
    let moov = boxes.iter().position(|b| b == "moov").expect("moov");
    let mdat = boxes.iter().position(|b| b == "mdat").expect("mdat");
    moov < mdat
}

#[test]
fn chapters_survive_remux_convert_and_trim() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    let src = fixture("hdr-chapters.mkv");
    let titles = |i: &MediaInfo| i.chapters.iter().map(|c| c.title.clone().unwrap_or_default()).collect::<Vec<_>>();
    for (name, op) in [
        ("copy.mp4", convert(PresetId::Mp4Copy)),
        ("copy.mkv", MediaOp::Remux { container: "matroska".into() }),
        ("copy.mov", MediaOp::Remux { container: "mov".into() }),
        ("enc.mp4", convert(PresetId::Mp4Mpeg4)),
    ] {
        let dest = dir.path().join(name);
        let out = run_ok(&rt, &src, &dest, &op);
        assert!(!out.warnings.iter().any(|w| w.contains("chapter")), "{name}: {:?}", out.warnings);
        let i = info(&rt, &dest);
        assert_eq!(titles(&i), ["Intro", "Outro"], "{name}");
        assert!((i.chapters[1].start - 0.25).abs() < 0.01, "{name}: {:?}", i.chapters);
        assert_eq!(i.tag("title"), Some("Soyuz HDR"), "{name}: container tags");
    }
    if rt.has_encoder("libvpx-vp9") {
        let dest = dir.path().join("enc.webm");
        run_ok(&rt, &src, &dest, &convert(PresetId::WebmVp9));
        assert_eq!(titles(&info(&rt, &dest)), ["Intro", "Outro"], "webm");
    }
    // A trim keeps the chapters that overlap the window, shifted to the new start.
    let dest = dir.path().join("cut.mkv");
    run_ok(&rt, &src, &dest, &MediaOp::Trim { start: 0.3, end: 0.5, stream_copy: false });
    let i = info(&rt, &dest);
    assert_eq!(titles(&i), ["Outro"], "{:?}", i.chapters);
    assert!(i.chapters[0].start.abs() < 0.01 && (i.chapters[0].end - 0.2).abs() < 0.02, "{:?}", i.chapters);
    // AVI has no chapters: the plan and the run say so.
    let dest = dir.path().join("cut.avi");
    let out = run_ok(&rt, &src, &dest, &MediaOp::Trim { start: 0.0, end: 0.5, stream_copy: true });
    assert!(out.warnings.iter().any(|w| w.contains("chapters")), "{:?}", out.warnings);
    let p = soyuz_media::plan(&rt, &src, &convert(PresetId::Mp4Copy)).unwrap();
    assert!(!p.warnings.iter().any(|w| w.contains("chapter")), "{:?}", p.warnings);
    let p = soyuz_media::plan(&rt, &src, &convert(PresetId::AviMpeg4)).unwrap();
    assert!(p.warnings.iter().any(|w| w.contains("chapters")), "{:?}", p.warnings);
}

#[test]
fn cover_art_and_tags_follow_audio() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    let src = fixture("cover.flac");
    let png = |i: &MediaInfo| i.cover.as_ref().is_some_and(|c| c.starts_with(b"\x89PNG"));
    // Containers with a picture slot keep the cover; the others drop it without a warning.
    let cases: Vec<(&str, MediaOp, bool)> = vec![
        ("a.m4a", convert(PresetId::AudioAac), true),
        ("b.flac", MediaOp::ExtractAudio { codec: AudioCodecChoice::Flac }, true),
        ("c.flac", MediaOp::Trim { start: 0.0, end: 0.2, stream_copy: true }, true),
        ("d.mka", MediaOp::Remux { container: "matroska".into() }, true),
        ("e.wav", convert(PresetId::AudioWav), false),
        ("f.m4a", convert(PresetId::AudioAlac), true),
        ("g.opus", convert(PresetId::AudioOpus), false),
    ];
    for (name, op, cover) in cases {
        if name.ends_with(".opus") && !rt.has_encoder("libopus") {
            continue;
        }
        let dest = dir.path().join(name);
        let out = run_ok(&rt, &src, &dest, &op);
        assert!(out.warnings.is_empty(), "{name}: {:?}", out.warnings);
        let i = info(&rt, &dest);
        assert_eq!(png(&i), cover, "{name}: cover {:?}", i.streams);
        assert!(i.audio().is_some(), "{name}");
        if !name.ends_with(".wav") {
            // Ogg has stream tags only.
            let a = i.audio().unwrap();
            let tag = |k: &str| {
                i.tag(k).or_else(|| a.tags.iter().find(|(t, _)| t.eq_ignore_ascii_case(k)).map(|(_, v)| v.as_str()))
            };
            assert_eq!(tag("artist"), Some("Soyuz"), "{name}: tags {:?} / {:?}", i.tags, a.tags);
            assert_eq!(tag("title"), Some("Cover"), "{name}: tags {:?} / {:?}", i.tags, a.tags);
        }
    }
    // Stream tags of the copied audio (language) and the container title survive an
    // audio extraction from a video file.
    let dest = dir.path().join("rot.m4a");
    run_ok(&rt, &fixture("rotated.mp4"), &dest, &MediaOp::ExtractAudio { codec: AudioCodecChoice::Copy });
    let i = info(&rt, &dest);
    assert_eq!(i.tag("title"), Some("Soyuz rotated"));
    assert_eq!(i.audio().unwrap().language.as_deref(), Some("eng"));
}

#[test]
fn rotation_is_kept_where_the_container_can_hold_it() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    let src = fixture("rotated.mp4");
    let mut cases: Vec<(&str, MediaOp)> = vec![
        ("copy.mkv", MediaOp::Remux { container: "matroska".into() }),
        ("copy.mov", MediaOp::Remux { container: "mov".into() }),
        ("enc.mp4", convert(PresetId::Mp4Mpeg4)),
        ("trim.mp4", MediaOp::Trim { start: 0.2, end: 0.8, stream_copy: false }),
        ("trim.mkv", MediaOp::Trim { start: 0.0, end: 0.8, stream_copy: true }),
    ];
    if rt.has_encoder("libvpx-vp9") && rt.has_encoder("libopus") {
        cases.push(("enc.webm", convert(PresetId::WebmVp9)));
    }
    if rt.has_encoder("prores_ks") {
        cases.push(("enc.mov", convert(PresetId::MovProres)));
    }
    for (name, op) in cases {
        let dest = dir.path().join(name);
        let out = run_ok(&rt, &src, &dest, &op);
        assert!(!out.warnings.iter().any(|w| w.contains("rotation")), "{name}: {:?}", out.warnings);
        assert_eq!(info(&rt, &dest).video().unwrap().rotation, 270, "{name}");
    }
    // AVI cannot store it: a warning, in the plan too.
    let dest = dir.path().join("enc.avi");
    let out = run_ok(&rt, &src, &dest, &convert(PresetId::AviMpeg4));
    assert!(out.warnings.iter().any(|w| w.contains("rotation")), "{:?}", out.warnings);
    let p = soyuz_media::plan(&rt, &src, &convert(PresetId::AviMpeg4)).unwrap();
    assert!(p.warnings.iter().any(|w| w.contains("rotation")), "{:?}", p.warnings);
}

#[test]
fn faststart_puts_the_index_first() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    let src = fixture("rotated.mp4");
    assert!(!moov_first(&src), "the fixture has its index at the end");
    for (name, op) in [
        ("copy.mp4", convert(PresetId::Mp4Copy)),
        ("enc.mp4", convert(PresetId::Mp4Mpeg4)),
        ("copy.mov", MediaOp::Remux { container: "mov".into() }),
        ("audio.m4a", MediaOp::ExtractAudio { codec: AudioCodecChoice::Copy }),
        ("trim.mp4", MediaOp::Trim { start: 0.2, end: 0.9, stream_copy: true }),
    ] {
        let dest = dir.path().join(name);
        run_ok(&rt, &src, &dest, &op);
        assert!(moov_first(&dest), "{name}: {:?}", top_boxes(&dest));
        // The moved index still points at the samples: the file decodes to the end.
        let i = info(&rt, &dest);
        assert!(i.duration.is_some_and(|d| d > 0.5), "{name}: {:?}", i.duration);
        let img = soyuz_media::grab_frame(
            &rt,
            &Source::File(dest.clone()),
            soyuz_media::FrameRequest { at: 0.6, max_px: 64, exact: true },
            &|| false,
        );
        assert!(name.ends_with(".m4a") || img.is_ok(), "{name}: {:?}", img.err());
        let mut r = soyuz_media::AudioReader::open(&rt, &Source::File(dest.clone()), None, None).expect("audio");
        let mut buf = Vec::new();
        let mut total = 0;
        while let Ok(n @ 1..) = r.read(&mut buf, 4096) {
            total += n;
        }
        assert!(total > 10_000, "{name}: {total} samples");
    }
    // Off: the muxer's own layout (index at the end), and nothing else changes.
    let dest = dir.path().join("plain.mp4");
    run_opts(&rt, &src, &dest, &convert(PresetId::Mp4Copy), &RunOptions { faststart: false, ..RunOptions::default() });
    assert!(!moov_first(&dest), "{:?}", top_boxes(&dest));
}

#[test]
fn threads_option_reaches_decoder_and_encoder() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    let src = fixture("rotated.mp4");
    for threads in [0, 1, 3] {
        let dest = dir.path().join(format!("t{threads}.mp4"));
        let out =
            run_opts(&rt, &src, &dest, &convert(PresetId::Mp4Mpeg4), &RunOptions { threads, ..RunOptions::default() });
        assert!(out.duration.is_some_and(|d| (d - 1.0).abs() < 0.15), "{threads}: {:?}", out.duration);
    }
}
