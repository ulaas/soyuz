//! The with-library suite: loads the FFmpeg runtime from `SOYUZ_FFMPEG_DIR` (or the
//! default search folders) and exercises `probe`, `grab_frame`, `AudioReader` and
//! `peaks` on files generated here with `soyuz_media::fixtures` and on the three tiny
//! checked-in fixtures (`tests/fixtures/README.md`).
//!
//! Without a runtime every test prints `skipped: …` and passes, unless
//! `SOYUZ_MEDIA_TESTS=1` is set: then a missing or refused runtime is a failure (CI).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use soyuz_media::{
    AudioReader, DIR_ENV, FrameRequest, Hdr, MediaError, Runtime, Source, StreamKind, fixtures, grab_frame, peaks,
    probe, search_dirs,
};

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

fn fixture(name: &str) -> Source {
    Source::File(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name))
}

fn mem(bytes: Vec<u8>) -> Source {
    Source::Mem(Arc::from(bytes.into_boxed_slice()))
}

/// A 5 fps Motion-JPEG AVI: red, green, blue frames at 0 / 0.2 / 0.4 s.
fn rgb_avi(w: u32, h: u32) -> Vec<u8> {
    let frames = vec![
        fixtures::jpeg_solid(w, h, [255, 0, 0]),
        fixtures::jpeg_solid(w, h, [0, 255, 0]),
        fixtures::jpeg_solid(w, h, [0, 0, 255]),
    ];
    fixtures::avi(w, h, 5, fixtures::AviCodec::Mjpeg, &frames)
}

fn dominant(px: &image::Rgba<u8>) -> char {
    let [r, g, b, _] = px.0;
    if r > 180 && g < 80 && b < 80 {
        'r'
    } else if g > 180 && r < 80 && b < 80 {
        'g'
    } else if b > 180 && r < 80 && g < 80 {
        'b'
    } else {
        '?'
    }
}

fn never() -> bool {
    false
}

#[test]
fn runtime_loads_and_reports_itself() {
    let rt = rt!();
    let v = rt.versions();
    assert!(v.majors().release().is_some(), "{}", v.majors().describe());
    assert!(!v.configuration.is_empty());
    assert!(Runtime::loaded().is_some_and(|l| Arc::ptr_eq(&l, &rt)));
    // A later call with other folders returns the loaded runtime.
    let again = Runtime::get(&[PathBuf::from("/nowhere")]).expect("already loaded");
    assert!(Arc::ptr_eq(&again, &rt));
    assert!(rt.has_decoder("pcm_s16le") && rt.has_decoder("rawvideo") && rt.has_decoder("mjpeg"));
    assert!(!rt.has_decoder("no-such-codec"));
    assert!(rt.has_encoder("pcm_s16le"), "the PCM encoder opens");
    assert!(!rt.has_encoder("no-such-codec"));
    // Hardware encoders answer without panicking whatever the machine has.
    let _ = rt.has_encoder("h264_nvenc");
    let codecs = rt.codecs();
    assert!(codecs.len() > 100, "{}", codecs.len());
    let aac = codecs.iter().find(|c| c.name == "aac").expect("aac listed");
    assert!(aac.decoder && aac.kind == StreamKind::Audio);
    let h264 = codecs.iter().find(|c| c.name == "h264").expect("h264 listed");
    assert_eq!(h264.kind, StreamKind::Video);
}

#[test]
fn probe_generated_wav_from_memory_and_file() {
    let rt = rt!();
    let samples = fixtures::sine(44_100, 440.0, 0.5, 22_050);
    let stereo: Vec<i16> = samples.iter().flat_map(|s| [*s, *s]).collect();
    let bytes = fixtures::wav(44_100, 2, &stereo);
    let len = bytes.len() as u64;
    let info = probe(&rt, &mem(bytes.clone())).expect("probe wav");
    assert_eq!(info.container, "wav");
    assert_eq!(info.size, len);
    assert!((info.duration.expect("duration") - 0.5).abs() < 0.01, "{:?}", info.duration);
    assert_eq!(info.streams.len(), 1);
    let a = info.audio().expect("audio stream");
    assert_eq!(a.codec, "pcm_s16le");
    assert_eq!((a.sample_rate, a.channels), (44_100, 2));
    // No channel mask in a plain WAV header: FFmpeg describes the layout by count.
    assert!(matches!(a.channel_layout.as_str(), "stereo" | "2 channels"), "{}", a.channel_layout);
    assert_eq!(a.sample_fmt.as_deref(), Some("s16"));
    assert!(info.video().is_none());
    // The same file on disk probes the same.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tone.wav");
    std::fs::write(&path, &bytes).unwrap();
    let from_file = probe(&rt, &Source::File(path)).expect("probe file");
    assert_eq!(from_file.streams, info.streams);
    assert_eq!(from_file.duration, info.duration);
}

#[test]
fn probe_generated_mjpeg_avi() {
    let rt = rt!();
    let info = probe(&rt, &mem(rgb_avi(32, 24))).expect("probe avi");
    assert_eq!(info.container, "avi");
    assert!((info.duration.expect("duration") - 0.6).abs() < 0.01, "{:?}", info.duration);
    let v = info.video().expect("video stream");
    assert_eq!(v.codec, "mjpeg");
    assert_eq!((v.width, v.height), (32, 24));
    assert_eq!(v.fps, Some(5.0));
    assert!(v.pix_fmt.is_some());
    assert_eq!(v.rotation, 0);
    assert!(v.hdr.is_none());
}

#[test]
fn grab_frame_fits_and_picks_the_right_frame() {
    let rt = rt!();
    let src = mem(rgb_avi(32, 24));
    let img = grab_frame(&rt, &src, FrameRequest::near(0.0, 16), &never).expect("first frame");
    assert_eq!((img.width(), img.height()), (16, 12), "fitted into 16 px");
    assert_eq!(dominant(img.get_pixel(8, 6)), 'r');
    // Never upscaled.
    let img = grab_frame(&rt, &src, FrameRequest::near(0.0, 4096), &never).expect("first frame");
    assert_eq!((img.width(), img.height()), (32, 24));
    // Exact: the first frame at or after 0.25 s is the blue one (0.4 s).
    let img = grab_frame(&rt, &src, FrameRequest { at: 0.25, max_px: 32, exact: true }, &never).expect("exact");
    assert_eq!(dominant(img.get_pixel(16, 12)), 'b');
    // Exact at a frame's own time.
    let img = grab_frame(&rt, &src, FrameRequest { at: 0.2, max_px: 32, exact: true }, &never).expect("exact 0.2");
    assert_eq!(dominant(img.get_pixel(16, 12)), 'g');
    // Nearest keyframe before 0.25 s: every MJPEG frame is a keyframe, so the green one.
    let img = grab_frame(&rt, &src, FrameRequest::near(0.25, 32), &never).expect("near");
    assert_eq!(dominant(img.get_pixel(16, 12)), 'g');
    // Past the end: the last frame.
    let img = grab_frame(&rt, &src, FrameRequest { at: 9.0, max_px: 32, exact: true }, &never).expect("past end");
    assert_eq!(dominant(img.get_pixel(16, 12)), 'b');
}

#[test]
fn rotated_fixture_probe_and_frame() {
    let rt = rt!();
    let src = fixture("rotated.mp4");
    let info = probe(&rt, &src).expect("probe rotated.mp4");
    assert!(info.container.contains("mp4"), "{}", info.container);
    assert_eq!(info.tag("title"), Some("Soyuz rotated"));
    assert!((info.duration.expect("duration") - 1.0).abs() < 0.05);
    let v = info.video().expect("video");
    assert_eq!(v.codec, "mpeg4");
    assert_eq!((v.width, v.height), (64, 48));
    assert_eq!(v.rotation, 270, "`-display_rotation 90` (counter-clockwise) = 270 clockwise");
    assert_eq!(v.fps, Some(10.0));
    assert!(v.profile.is_some(), "mpeg4 reports a profile");
    let a = info.audio().expect("audio");
    assert_eq!(a.codec, "aac");
    assert_eq!((a.sample_rate, a.channels), (22_050, 1));
    assert_eq!(a.language.as_deref(), Some("eng"));
    assert!(a.default);
    // The frame comes out rotated: 48 wide, 64 tall, fitted into 64.
    let img = grab_frame(&rt, &src, FrameRequest::near(0.5, 64), &never).expect("frame");
    assert_eq!((img.width(), img.height()), (48, 64));
    let img = grab_frame(&rt, &src, FrameRequest { at: 0.5, max_px: 32, exact: true }, &never).expect("exact frame");
    assert_eq!((img.width(), img.height()), (24, 32));
    // Its AAC track reads as audio (priming samples trimmed by the decoder).
    let mut r = AudioReader::open(&rt, &src, None, None).expect("aac reader");
    assert_eq!((r.rate(), r.channels()), (22_050, 1));
    assert_eq!(r.info().codec, "aac");
    let mut out = Vec::new();
    let mut total = 0;
    loop {
        let n = r.read(&mut out, 4096).expect("read aac");
        if n == 0 {
            break;
        }
        total += n;
    }
    assert!((20_000..=23_000).contains(&total), "{total} frames of a 1 s clip");
    assert!(out.iter().any(|s| s.abs() > 0.1), "not silence");
}

#[test]
fn hdr_and_chapters_fixture() {
    let rt = rt!();
    let info = probe(&rt, &fixture("hdr-chapters.mkv")).expect("probe hdr-chapters.mkv");
    assert!(info.container.starts_with("matroska"), "{}", info.container);
    assert_eq!(info.tag("title"), Some("Soyuz HDR"));
    let v = info.video().expect("video");
    assert_eq!(v.hdr, Some(Hdr::Hdr10));
    let color = v.color.as_ref().expect("colour info");
    assert_eq!(color.transfer, "smpte2084");
    assert_eq!(color.primaries, "bt2020");
    assert_eq!(color.matrix, "bt2020nc");
    assert_eq!(color.range, "tv");
    assert_eq!(info.chapters.len(), 2);
    assert_eq!(info.chapters[0].title.as_deref(), Some("Intro"));
    assert!((info.chapters[0].end - 0.25).abs() < 1e-6);
    assert!((info.chapters[1].start - 0.25).abs() < 1e-6 && (info.chapters[1].end - 0.5).abs() < 1e-6);
    assert_eq!(info.chapters[1].title.as_deref(), Some("Outro"));
}

#[test]
fn cover_art_fixture() {
    let rt = rt!();
    let info = probe(&rt, &fixture("cover.flac")).expect("probe cover.flac");
    assert_eq!(info.container, "flac");
    assert_eq!(info.tag("artist"), Some("Soyuz"));
    assert_eq!(info.tag("TITLE"), Some("Cover"), "tags are matched case-insensitively");
    let cover = info.cover.as_ref().expect("cover bytes");
    assert_eq!(&cover[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(info.streams.len(), 2);
    assert!(info.streams[1].attached_pic);
    assert_eq!(info.streams[1].codec, "png");
    assert!(info.video().is_none(), "cover art is not a video stream");
    let a = info.audio().expect("audio");
    assert_eq!((a.codec.as_str(), a.sample_rate, a.channels), ("flac", 8000, 1));
    // grab_frame on the cover art decodes the attached picture.
    let img = grab_frame(&rt, &fixture("cover.flac"), FrameRequest::near(0.0, 8), &never).expect("cover frame");
    assert_eq!((img.width(), img.height()), (8, 8));
    assert_eq!(dominant(img.get_pixel(4, 4)), 'r');
}

#[test]
fn audio_reader_counts_samples_resamples_and_seeks() {
    let rt = rt!();
    let rate = 8000;
    let samples = fixtures::sine(rate, 100.0, 0.5, rate as usize);
    let src = mem(fixtures::wav(rate, 1, &samples));
    let mut r = AudioReader::open(&rt, &src, None, None).expect("open");
    assert_eq!((r.rate(), r.channels()), (rate, 1));
    assert_eq!(r.info().codec, "pcm_s16le");
    let mut out = Vec::new();
    let mut total = 0;
    loop {
        let n = r.read(&mut out, 1000).expect("read");
        if n == 0 {
            break;
        }
        total += n;
    }
    assert_eq!(total, rate as usize, "one second of samples");
    assert_eq!(out.len(), total);
    for (i, want) in samples.iter().take(200).enumerate() {
        let w = f32::from(*want) / f32::from(i16::MAX);
        assert!((out[i] - w).abs() < 2e-4, "sample {i}: {} vs {w}", out[i]);
    }
    assert_eq!(r.read(&mut out, 10).expect("eof"), 0, "stays at the end");
    // Seek to 0.5 s: the next sample is the one at index 4000.
    r.seek(0.5).expect("seek");
    out.clear();
    let n = r.read(&mut out, 100).expect("read after seek");
    assert_eq!(n, 100);
    for i in 0..100 {
        let w = f32::from(samples[4000 + i]) / f32::from(i16::MAX);
        assert!((out[i] - w).abs() < 2e-4, "sample {i} after seek: {} vs {w}", out[i]);
    }
    let mut rest = 100;
    loop {
        let n = r.read(&mut out, 4096).expect("read rest");
        if n == 0 {
            break;
        }
        rest += n;
    }
    assert!((3998..=4002).contains(&rest), "{rest} samples after seeking to 0.5 s");
    // Resampling to 16 kHz doubles the count (± the resampler's edge).
    let mut r = AudioReader::open(&rt, &src, None, Some(16_000)).expect("open 16k");
    assert_eq!(r.rate(), 16_000);
    let mut total = 0;
    out.clear();
    loop {
        let n = r.read(&mut out, 4096).expect("read 16k");
        if n == 0 {
            break;
        }
        total += n;
    }
    assert!((15_900..=16_100).contains(&total), "{total} resampled samples");
    // An explicit stream index that is not audio is refused.
    assert!(matches!(AudioReader::open(&rt, &src, Some(7), None), Err(MediaError::NoStream(StreamKind::Audio))));
}

#[test]
fn peaks_cover_the_waveform() {
    let rt = rt!();
    let samples = fixtures::sine(8000, 50.0, 0.5, 8000);
    let src = mem(fixtures::wav(8000, 1, &samples));
    let p = peaks(&rt, &src, 50, &never).expect("peaks");
    assert_eq!(p.len(), 50);
    for (i, (lo, hi)) in p.iter().enumerate() {
        assert!(*hi > 0.4 && *lo < -0.4, "bucket {i}: {lo} .. {hi}");
    }
    assert!(peaks(&rt, &src, 0, &never).expect("no buckets").is_empty());
    // More buckets than groups: as many as there are.
    let p = peaks(&rt, &src, 100_000, &never).expect("many buckets");
    assert!(!p.is_empty() && p.len() <= 8000, "{}", p.len());
}

#[test]
fn stale_stops_the_work() {
    let rt = rt!();
    let src = mem(rgb_avi(32, 24));
    let always = || true;
    assert!(matches!(grab_frame(&rt, &src, FrameRequest::near(0.0, 16), &always), Err(MediaError::Stale)));
    // Stale after a few reads: the audio decode stops part-way.
    let calls = AtomicUsize::new(0);
    let after_three = || calls.fetch_add(1, Ordering::Relaxed) >= 3;
    let wav = mem(fixtures::wav(8000, 1, &fixtures::sine(8000, 50.0, 0.5, 80_000)));
    assert!(matches!(peaks(&rt, &wav, 10, &after_three), Err(MediaError::Stale)));
    assert!(calls.load(Ordering::Relaxed) < 40, "stopped early: {} polls", calls.load(Ordering::Relaxed));
}

#[test]
fn corrupt_input_is_an_error_not_a_crash() {
    let rt = rt!();
    let junk: Vec<u8> = (0..4096u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    assert!(probe(&rt, &mem(junk.clone())).is_err());
    assert!(probe(&rt, &mem(Vec::new())).is_err(), "empty input");
    assert!(grab_frame(&rt, &mem(junk), FrameRequest::near(0.0, 16), &never).is_err());
    // A WAV header that promises more data than exists still probes (FFmpeg clamps).
    let mut short = fixtures::wav(8000, 1, &[0; 800]);
    short.truncate(44 + 100);
    let _ = probe(&rt, &mem(short.clone()));
    // A WAV has no video stream.
    assert!(matches!(
        grab_frame(&rt, &mem(fixtures::wav(8000, 1, &[0; 80])), FrameRequest::near(0.0, 16), &never),
        Err(MediaError::NoStream(StreamKind::Video))
    ));
    // Truncated JPEG frames inside an AVI: an error or a partial picture, never a panic.
    let mut frames = vec![fixtures::jpeg_solid(16, 16, [0, 255, 0])];
    let half = frames[0].len() / 2;
    frames[0].truncate(half);
    let avi = fixtures::avi(16, 16, 5, fixtures::AviCodec::Mjpeg, &frames);
    let _ = grab_frame(&rt, &mem(avi), FrameRequest::near(0.0, 16), &never);
    // A missing file is an I/O error.
    let missing = Source::File(std::env::temp_dir().join("soyuz-media-no-such-file.mp4"));
    assert!(matches!(probe(&rt, &missing), Err(MediaError::Io(_))));
}
