//! The `Player` suite against a real FFmpeg runtime (`SOYUZ_FFMPEG_DIR`, gated like
//! `tests/with_libs.rs`: skipped without a runtime unless `SOYUZ_MEDIA_TESTS=1`).
//!
//! Frames go to a counting [`FrameSink`]; audio to a [`NullAudioOutput`] whose sink
//! behaves like a device: it accepts up to half a second, and `played_frames` advances
//! with the wall clock while not paused, past what was pushed (underruns play silence).
//! It records how much was queued each time the player let it run (the pre-roll).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use soyuz_media::{
    AudioOutput, AudioReader, AudioSink, DIR_ENV, FramePixels, FrameSink, HwPref, MediaError, Player, PlayerCmd,
    PlayerOptions, PlayerState, Runtime, Source, fixtures, player::seams, preroll_frames, search_dirs,
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

/// A `secs` long, 30 fps Motion-JPEG clip of `w × h` with a colour ramp.
fn clip(w: u32, h: u32, secs: f64) -> Vec<u8> {
    let n = (secs * 30.0).round() as usize;
    let frames: Vec<Vec<u8>> = (0..n)
        .map(|i| fixtures::jpeg_solid(w, h, [(i * 255 / n.max(1)) as u8, 64, 255 - (i * 255 / n.max(1)) as u8]))
        .collect();
    fixtures::avi(w, h, 30, fixtures::AviCodec::Mjpeg, &frames)
}

#[derive(Clone, Copy, Debug)]
struct Shown {
    width: u32,
    height: u32,
    pts: f64,
}

#[derive(Default)]
struct Frames(Arc<Mutex<Vec<Shown>>>);

impl Frames {
    fn sink(&self) -> Box<dyn FrameSink> {
        struct S(Arc<Mutex<Vec<Shown>>>);
        impl FrameSink for S {
            fn present(&mut self, f: FramePixels<'_>) {
                assert_eq!(f.rgba.len(), (f.width * f.height * 4) as usize, "tightly packed RGBA");
                self.0.lock().unwrap().push(Shown { width: f.width, height: f.height, pts: f.pts });
            }
        }
        Box::new(S(Arc::clone(&self.0)))
    }

    fn all(&self) -> Vec<Shown> {
        self.0.lock().unwrap().clone()
    }

    fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

struct NullState {
    rate: u32,
    channels: u32,
    pushed: u64,
    played_at_pause: u64,
    running_since: Option<Instant>,
    volume: f32,
    flushes: u32,
    /// Frames queued (pushed, not played) at every pause → run transition.
    started_with: Vec<u64>,
}

impl NullState {
    fn played(&self) -> u64 {
        self.played_at_pause
            + self.running_since.map_or(0, |s| (s.elapsed().as_secs_f64() * f64::from(self.rate)) as u64)
    }
}

/// An audio "device" that plays silence in real time.
#[derive(Clone)]
struct NullAudioOutput(Arc<Mutex<NullState>>);

impl NullAudioOutput {
    fn new() -> NullAudioOutput {
        NullAudioOutput(Arc::new(Mutex::new(NullState {
            rate: 0,
            channels: 0,
            pushed: 0,
            played_at_pause: 0,
            running_since: None,
            volume: -1.0,
            flushes: 0,
            started_with: Vec::new(),
        })))
    }

    fn started_with(&self) -> Vec<u64> {
        self.0.lock().unwrap().started_with.clone()
    }

    fn pushed(&self) -> u64 {
        self.0.lock().unwrap().pushed
    }

    fn opened_as(&self) -> (u32, u32) {
        let s = self.0.lock().unwrap();
        (s.rate, s.channels)
    }
}

impl AudioOutput for NullAudioOutput {
    fn open(&self, rate: u32, channels: u32) -> Box<dyn AudioSink> {
        let mut s = self.0.lock().unwrap();
        s.rate = rate;
        s.channels = channels;
        drop(s);
        Box::new(NullSink(Arc::clone(&self.0)))
    }
}

struct NullSink(Arc<Mutex<NullState>>);

impl AudioSink for NullSink {
    fn push(&mut self, interleaved: &[f32]) -> usize {
        let mut s = self.0.lock().unwrap();
        let queued = s.pushed.saturating_sub(s.played());
        let room = (u64::from(s.rate) / 2).saturating_sub(queued) as usize;
        let n = (interleaved.len() / s.channels.max(1) as usize).min(room);
        s.pushed += n as u64;
        n
    }

    fn played_frames(&self) -> u64 {
        self.0.lock().unwrap().played()
    }

    fn queued_frames(&self) -> u64 {
        let s = self.0.lock().unwrap();
        s.pushed.saturating_sub(s.played())
    }

    fn pause(&mut self, on: bool) {
        let mut s = self.0.lock().unwrap();
        if on {
            s.played_at_pause = s.played();
            s.running_since = None;
        } else if s.running_since.is_none() {
            let queued = s.pushed.saturating_sub(s.played());
            s.started_with.push(queued);
            s.running_since = Some(Instant::now());
        }
    }

    fn set_volume(&mut self, v: f32) {
        self.0.lock().unwrap().volume = v;
    }

    fn flush(&mut self) {
        let mut s = self.0.lock().unwrap();
        s.pushed = s.played();
        s.flushes += 1;
    }
}

fn wait_for(p: &Player, what: &str, timeout: Duration, mut ok: impl FnMut(&PlayerState) -> bool) -> PlayerState {
    let deadline = Instant::now() + timeout;
    loop {
        let s = p.state();
        if ok(&s) {
            return s;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}: {s:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn no_wake() -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(|| {})
}

fn assert_monotonic(frames: &[Shown]) {
    for w in frames.windows(2) {
        assert!(w[1].pts > w[0].pts, "pts not increasing: {:?} then {:?}", w[0], w[1]);
    }
}

#[test]
fn video_only_plays_pauses_seeks_resizes_and_ends() {
    let rt = rt!();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ramp.avi");
    std::fs::write(&path, clip(64, 48, 2.0)).unwrap();
    let frames = Frames::default();
    let wakes = Arc::new(Mutex::new(0u32));
    let w = Arc::clone(&wakes);
    let opts = PlayerOptions { max_px: 64, autoplay: true, ..Default::default() };
    let p = Player::open(
        Arc::clone(&rt),
        Source::File(path.clone()),
        opts,
        frames.sink(),
        None,
        Arc::new(move || *w.lock().unwrap() += 1),
    )
    .expect("open");
    let s = p.state();
    assert_eq!(s.path, path);
    assert!(s.playing && !s.ended && s.error.is_none());
    assert!((s.duration.expect("duration") - 2.0).abs() < 0.05);
    assert_eq!(s.video.as_ref().map(|v| (v.codec.as_str(), v.fps)), Some(("mjpeg", Some(30.0))));
    assert!(s.audio.is_none(), "no audio stream: the wall clock drives it");

    // About a second of playback: ~30 frames, in order, at the requested size.
    std::thread::sleep(Duration::from_millis(1000));
    let shown = frames.all();
    assert!(shown.len() >= 25, "{} frames presented in 1 s", shown.len());
    assert!(shown.len() <= 36, "{} frames: more than the clip has for 1 s", shown.len());
    assert_monotonic(&shown);
    assert!(shown.iter().all(|f| (f.width, f.height) == (64, 48)), "{shown:?}");
    assert_eq!(shown[0].pts, 0.0, "the first picture is shown at once");
    let s = p.state();
    assert!((0.85..1.4).contains(&s.pos), "pos {}", s.pos);
    assert!(s.buffered >= 0.0);
    assert!(*wakes.lock().unwrap() >= 20, "the UI was woken per frame");

    // Pause freezes the position and the frames.
    p.send(PlayerCmd::Pause);
    let s1 = wait_for(&p, "pause", Duration::from_secs(1), |s| !s.playing);
    std::thread::sleep(Duration::from_millis(120));
    let n = frames.len();
    let s2 = p.state();
    assert_eq!(s1.pos, s2.pos, "paused position moves");
    std::thread::sleep(Duration::from_millis(120));
    assert_eq!(frames.len(), n, "frames keep coming while paused");

    // A seek while paused shows the target frame at the new output size.
    p.send(PlayerCmd::SetOutputSize(32));
    p.send(PlayerCmd::Seek(1.0));
    let deadline = Instant::now() + Duration::from_secs(2);
    let shown = loop {
        let all = frames.all();
        if all.len() > n {
            break all;
        }
        assert!(Instant::now() < deadline, "no frame after the seek");
        std::thread::sleep(Duration::from_millis(5));
    };
    let f = shown[n];
    assert!((f.pts - 1.0).abs() <= 1.0 / 30.0 + 1e-6, "seek landed at {}", f.pts);
    assert_eq!((f.width, f.height), (32, 24), "the new output size");
    let s = p.state();
    assert!((s.pos - 1.0).abs() < 0.01, "pos {} after seeking to 1.0 while paused", s.pos);
    assert!(!s.playing && !s.ended);
    std::thread::sleep(Duration::from_millis(60));
    assert_eq!(frames.len(), n + 1, "one frame for a seek while paused");

    // Resume: it plays to the end and reports it.
    p.send(PlayerCmd::Play);
    let s = wait_for(&p, "the end", Duration::from_secs(3), |s| s.ended);
    assert!(!s.playing && s.error.is_none());
    assert!((1.9..=2.0).contains(&s.pos), "pos {} at the end", s.pos);
    let shown = frames.all();
    assert_monotonic(&shown[n..]);
    let last = shown.last().unwrap();
    assert!((last.pts - 59.0 / 30.0).abs() < 1e-6, "the last frame was shown: {last:?}");
    assert!(shown[n..].iter().all(|f| (f.width, f.height) == (32, 24)));
    assert_eq!(s.dropped, 0, "nothing was late");

    // Play again after the end starts over.
    p.send(PlayerCmd::Play);
    let s = wait_for(&p, "restart", Duration::from_secs(2), |s| s.playing && !s.ended && s.pos < 0.5);
    assert!(s.error.is_none());

    let started = Instant::now();
    drop(p);
    assert!(started.elapsed() < Duration::from_millis(500), "drop joined in {:?}", started.elapsed());
}

#[test]
fn loop_restarts_at_the_end() {
    let rt = rt!();
    let frames = Frames::default();
    let opts = PlayerOptions { max_px: 16, autoplay: true, loop_: true, ..Default::default() };
    let p = Player::open(rt, mem(clip(16, 16, 0.5)), opts, frames.sink(), None, no_wake()).expect("open");
    std::thread::sleep(Duration::from_millis(1300));
    let s = p.state();
    assert!(s.playing && !s.ended, "{s:?}");
    let shown = frames.all();
    assert!(shown.len() >= 25, "{} frames in 1.3 s of a looping 0.5 s clip", shown.len());
    let restarts = shown.windows(2).filter(|w| w[1].pts < w[0].pts).count();
    assert!((1..=3).contains(&restarts), "{restarts} restarts: {shown:?}");
    assert!(shown.iter().any(|f| (f.pts - 14.0 / 30.0).abs() < 1e-6), "the last frame is shown before the loop");
    // The position wrapped as well.
    assert!(s.pos < 0.6, "pos {}", s.pos);
    p.send(PlayerCmd::Stop);
    let s = wait_for(&p, "stop", Duration::from_secs(1), |s| s.ended);
    assert!(!s.playing);
}

#[test]
fn audio_and_video_follow_the_audio_clock() {
    let rt = rt!();
    let frames = Frames::default();
    let out = NullAudioOutput::new();
    let opts = PlayerOptions { max_px: 32, autoplay: true, volume: 0.5, ..Default::default() };
    let p = Player::open(rt, fixture("rotated.mp4"), opts, frames.sink(), Some(Arc::new(out.clone())), no_wake())
        .expect("open rotated.mp4");
    let s = p.state();
    assert_eq!(s.video.as_ref().map(|v| v.codec.as_str()), Some("mpeg4"));
    assert_eq!(s.audio.as_ref().map(|a| (a.codec.as_str(), a.sample_rate, a.channels)), Some(("aac", 22_050, 1)));
    assert_eq!(out.opened_as(), (22_050, 1), "the device was opened for the stream");
    assert_eq!(out.0.lock().unwrap().volume, 0.5);
    let s = wait_for(&p, "the end", Duration::from_secs(4), |s| s.ended);
    assert!(s.error.is_none(), "{s:?}");
    assert!((0.9..=1.05).contains(&s.pos), "pos {} at the end of a 1 s clip", s.pos);
    let pushed = out.pushed();
    assert!((20_000..=23_000).contains(&pushed), "{pushed} audio frames pushed");
    let shown = frames.all();
    assert!((8..=11).contains(&shown.len()), "{} of 10 frames shown", shown.len());
    assert_monotonic(&shown);
    assert!(shown.iter().all(|f| (f.width, f.height) == (24, 32)), "rotated 270° and fitted into 32: {shown:?}");
    // Mute and volume reach the device.
    p.send(PlayerCmd::Mute(true));
    assert_eq!(out.0.lock().unwrap().volume, 0.0);
    p.send(PlayerCmd::SetVolume(0.25));
    assert_eq!(out.0.lock().unwrap().volume, 0.0, "still muted");
    p.send(PlayerCmd::Mute(false));
    assert_eq!(out.0.lock().unwrap().volume, 0.25);
    let s = p.state();
    assert!(!s.muted && s.volume == 0.25);
    // Seeking back and playing again: the device is flushed and the clock re-anchored.
    p.send(PlayerCmd::Seek(0.5));
    p.send(PlayerCmd::Play);
    let s = wait_for(&p, "playing from 0.5", Duration::from_secs(2), |s| s.playing && (0.45..0.8).contains(&s.pos));
    assert!(!s.ended);
    assert!(out.0.lock().unwrap().flushes >= 1);
    // The device ran only with a full pre-roll queued: at the open, and after the seek.
    let starts = out.started_with();
    assert!(starts.len() >= 2, "{starts:?}");
    assert!(starts.iter().all(|q| *q >= preroll_frames(22_050)), "started with too little audio: {starts:?}");
    let started = Instant::now();
    drop(p);
    assert!(started.elapsed() < Duration::from_millis(500), "drop joined in {:?}", started.elapsed());
}

#[test]
fn the_clock_waits_for_the_preroll() {
    let rt = rt!();
    let rate = 8000;
    let wav = fixtures::wav(rate, 1, &fixtures::sine(rate, 220.0, 0.5, 2 * rate as usize));
    let frames = Frames::default();
    let out = NullAudioOutput::new();
    let opts = PlayerOptions { autoplay: true, ..Default::default() };
    let p = Player::open(rt, mem(wav), opts, frames.sink(), Some(Arc::new(out.clone())), no_wake()).expect("open");
    let need = preroll_frames(rate);
    let s = wait_for(&p, "the clock to run", Duration::from_secs(2), |s| s.pos > 0.05);
    assert!(s.playing && s.error.is_none());
    let starts = out.started_with();
    assert_eq!(starts.len(), 1, "one start: {starts:?}");
    assert!(starts[0] >= need, "the device started with {} of {need} frames", starts[0]);
    // A seek pauses the device until the new position is queued again; the position
    // stands at the target meanwhile and then runs from there.
    p.send(PlayerCmd::Seek(1.0));
    let s = wait_for(&p, "the clock to run after the seek", Duration::from_secs(2), |s| s.pos > 1.02);
    assert!(s.pos < 1.5, "pos {}", s.pos);
    let starts = out.started_with();
    assert_eq!(starts.len(), 2, "{starts:?}");
    assert!(starts[1] >= need, "after the seek the device started with {} of {need} frames", starts[1]);
    // Pause and resume with the device full: it runs again at once.
    p.send(PlayerCmd::Pause);
    std::thread::sleep(Duration::from_millis(50));
    p.send(PlayerCmd::Play);
    let starts = out.started_with();
    assert_eq!(starts.len(), 3, "resumed at once: {starts:?}");
    assert!(starts[2] >= need);
    // A file shorter than the pre-roll starts with what it has and still ends.
    let short = fixtures::wav(rate, 1, &fixtures::sine(rate, 220.0, 0.5, (need / 2) as usize));
    let out = NullAudioOutput::new();
    let p = Player::open(
        runtime().unwrap(),
        mem(short),
        PlayerOptions { autoplay: true, ..Default::default() },
        frames.sink(),
        Some(Arc::new(out.clone())),
        no_wake(),
    )
    .expect("open short");
    let s = wait_for(&p, "the end of a short file", Duration::from_secs(2), |s| s.ended);
    assert!(s.error.is_none());
    let starts = out.started_with();
    assert_eq!(starts.len(), 1, "{starts:?}");
    assert!((need / 2..need).contains(&starts[0]), "started with the whole file: {starts:?}");
}

#[test]
fn audio_reader_moves_between_threads() {
    let rt = rt!();
    let rate = 8000;
    let wav = fixtures::wav(rate, 1, &fixtures::sine(rate, 220.0, 0.5, rate as usize));
    // Opened here, read on another thread (as the preview's feeder does).
    let mut r = AudioReader::open(&rt, &mem(wav), None, None).expect("open");
    let read = std::thread::spawn(move || {
        let mut out = Vec::new();
        while r.read(&mut out, 1024).expect("read") > 0 {}
        out.len()
    });
    assert_eq!(read.join().unwrap(), rate as usize);
}

#[test]
fn audio_only_file_plays_to_the_device() {
    let rt = rt!();
    let rate = 8000;
    let wav = fixtures::wav(rate, 1, &fixtures::sine(rate, 220.0, 0.5, rate as usize));
    let frames = Frames::default();
    let out = NullAudioOutput::new();
    let opts = PlayerOptions { autoplay: false, ..Default::default() };
    let p =
        Player::open(Arc::clone(&rt), mem(wav.clone()), opts, frames.sink(), Some(Arc::new(out.clone())), no_wake())
            .expect("open wav");
    let s = p.state();
    assert!(!s.playing && s.video.is_none() && s.audio.is_some());
    assert_eq!(out.opened_as(), (rate, 1));
    std::thread::sleep(Duration::from_millis(150));
    let s = p.state();
    assert_eq!(s.pos, 0.0, "paused: the device does not count");
    assert!(
        out.pushed() > 0 && out.pushed() <= u64::from(rate) / 2,
        "pre-buffered up to the device's room: {}",
        out.pushed()
    );
    assert!(s.buffered > 0.0, "buffered {}", s.buffered);
    p.send(PlayerCmd::Play);
    let s = wait_for(&p, "the end", Duration::from_secs(3), |s| s.ended);
    assert!(s.error.is_none());
    assert!((0.95..=1.05).contains(&s.pos), "pos {}", s.pos);
    let pushed = out.pushed();
    assert!((7_990..=8_010).contains(&pushed), "{pushed} frames of a 1 s file");
    assert_eq!(frames.len(), 0, "no pictures");
    // Without an output an audio file cannot be played.
    assert!(matches!(
        Player::open(rt, mem(wav), PlayerOptions::default(), frames.sink(), None, no_wake()),
        Err(MediaError::Unsupported(_))
    ));
}

#[test]
fn corrupt_input_is_an_error_not_a_crash() {
    let rt = rt!();
    let frames = Frames::default();
    let junk: Vec<u8> = (0..4096u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    assert!(
        Player::open(Arc::clone(&rt), mem(junk.clone()), PlayerOptions::default(), frames.sink(), None, no_wake())
            .is_err()
    );
    assert!(
        Player::open(Arc::clone(&rt), mem(Vec::new()), PlayerOptions::default(), frames.sink(), None, no_wake())
            .is_err()
    );
    let missing = Source::File(std::env::temp_dir().join("soyuz-media-no-such-file.mp4"));
    assert!(matches!(
        Player::open(Arc::clone(&rt), missing, PlayerOptions::default(), frames.sink(), None, no_wake()),
        Err(MediaError::Io(_))
    ));
    // A well-formed container whose frames are garbage: the decoder fails or produces
    // nothing; the player reports an error or the end, and the process survives.
    let avi = fixtures::avi(16, 16, 5, fixtures::AviCodec::Mjpeg, &[junk.clone(), junk.clone(), junk]);
    let opts = PlayerOptions { autoplay: true, ..Default::default() };
    match Player::open(rt, mem(avi), opts, frames.sink(), None, no_wake()) {
        Ok(p) => {
            let s = wait_for(&p, "error or end", Duration::from_secs(3), |s| s.error.is_some() || s.ended);
            assert!(!s.playing);
            let started = Instant::now();
            drop(p);
            assert!(started.elapsed() < Duration::from_millis(500));
        }
        Err(e) => eprintln!("refused at open: {e}"),
    }
}

#[test]
fn drop_while_playing_joins_quickly() {
    let rt = rt!();
    let frames = Frames::default();
    let out = NullAudioOutput::new();
    let opts = PlayerOptions { autoplay: true, ..Default::default() };
    let p =
        Player::open(rt, fixture("rotated.mp4"), opts, frames.sink(), Some(Arc::new(out)), no_wake()).expect("open");
    std::thread::sleep(Duration::from_millis(80));
    let started = Instant::now();
    drop(p);
    assert!(started.elapsed() < Duration::from_millis(500), "drop joined in {:?}", started.elapsed());
}

/// Records the pts of every presented picture.
struct Pts(Arc<Mutex<Vec<f64>>>);

impl FrameSink for Pts {
    fn present(&mut self, f: FramePixels<'_>) {
        self.0.lock().unwrap().push(f.pts);
    }
}

#[test]
fn a_hardware_decoding_error_reopens_in_software_and_plays_on() {
    let rt = rt!();
    // Two seconds of 30 fps Motion JPEG; the decoder fails once at packet 21 (0.67 s).
    let frames: Vec<Vec<u8>> = (0..60u8).map(|i| fixtures::jpeg_solid(32, 16, [i * 4, 64, 255 - i * 4])).collect();
    let clip = fixtures::avi(32, 16, 30, fixtures::AviCodec::Mjpeg, &frames);
    let shown = Arc::new(Mutex::new(Vec::new()));
    seams::inject_hw_fault(20);
    let opts = PlayerOptions { max_px: 32, autoplay: true, hw: HwPref::Auto, ..Default::default() };
    let p = Player::open(rt, mem(clip), opts, Box::new(Pts(Arc::clone(&shown))), None, no_wake()).expect("open");
    let s = wait_for(&p, "the end", Duration::from_secs(5), |s| s.ended);
    assert!(s.error.is_none(), "the error was absorbed: {s:?}");
    assert_eq!(s.hw, None, "software after the fallback");
    assert_eq!(p.seek_count(), 1, "the fallback seeked once, to where playback was");
    let pts = shown.lock().unwrap().clone();
    assert_eq!(pts.first().copied(), Some(0.0));
    assert!((pts.last().unwrap() - 59.0 / 30.0).abs() < 1e-6, "played to the end: {pts:?}");
    // Continued from the current position: never back to the start, no jump ahead.
    for w in pts.windows(2) {
        assert!(w[1] >= w[0], "went back: {pts:?}");
        assert!(w[1] - w[0] < 0.2, "skipped from {} to {}: {pts:?}", w[0], w[1]);
    }
    assert!(pts.len() + s.dropped as usize >= 55, "{} shown, {} dropped", pts.len(), s.dropped);
}

#[test]
fn large_packets_stay_under_the_byte_budget() {
    let rt = rt!();
    // Raw 1080p frames: 3 MB packets. The count cap (64) alone would queue all 16
    // (50 MB) while the player is paused.
    let (w, h) = (1920u32, 1080u32);
    let frame = vec![128u8; (w * h * 3 / 2) as usize];
    let clip = fixtures::avi(w, h, 30, fixtures::AviCodec::I420, &vec![frame; 16]);
    let shown = Arc::new(Mutex::new(Vec::new()));
    let opts = PlayerOptions { max_px: 32, ..Default::default() };
    let p = Player::open(rt, mem(clip), opts, Box::new(Pts(Arc::clone(&shown))), None, no_wake()).expect("open");
    // Paused: the first picture is shown, then the demuxer fills the queue until the
    // budget stops it.
    let deadline = Instant::now() + Duration::from_secs(5);
    while !p.video_queue().2 {
        assert!(Instant::now() < deadline, "the demuxer never blocked: {:?}", p.video_queue());
        std::thread::sleep(Duration::from_millis(5));
    }
    let (n, bytes, _) = p.video_queue();
    assert!(bytes <= seams::VIDEO_QUEUE_BYTES, "{bytes} bytes in {n} packets");
    assert!((2..14).contains(&n), "{n} packets queued");
    // Playing drains it to the end.
    p.send(PlayerCmd::Play);
    let s = wait_for(&p, "the end", Duration::from_secs(5), |s| s.ended);
    assert!(s.error.is_none(), "{s:?}");
    assert_eq!(shown.lock().unwrap().len() + s.dropped as usize, 16);
    assert_eq!(p.video_queue().1, 0);
}
