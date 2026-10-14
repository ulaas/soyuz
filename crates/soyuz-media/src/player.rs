//! Audio / video playback ([`Player`]).
//!
//! Threads per open file (V2a):
//!
//! ```text
//!  demux thread ──packets──► bounded queue (video) ──► video decode thread ──► sws_scale →
//!      │                                                 RGBA (≤ max_px, rotated) →
//!      │                                                 FrameSink::present when pts ≤ clock
//!      └────packets──► bounded queue (audio) ──► audio decode thread ──► swr → f32 →
//!                                                     AudioSink::push (never blocks)
//!  clock = AudioSink::played_frames / rate + anchor   (wall clock without audio)
//! ```
//!
//! The demux thread feeds the bounded packet queues ([`crate::queue::Bounded`]) and
//! performs seeks; the video thread scales frames to RGBA at the requested output size,
//! rotates them as the stream's display matrix says and hands them to the [`FrameSink`]
//! when their pts is due (late frames are dropped and counted); the audio thread
//! resamples to interleaved `f32` and pushes into the [`AudioSink`], whose
//! `played_frames` is the master clock ([`crate::clock::Clock`]; the wall clock when the
//! file has no audio or no [`AudioOutput`] was given). The UI thread only sends
//! [`PlayerCmd`]s and reads [`PlayerState`]; `Play` / `Pause` / volume take effect in
//! [`Player::send`] itself, seeks are queued for the demux thread.
//!
//! A seek is accurate: the queues are flushed, the demuxer seeks to the keyframe at or
//! before the target, a flush marker makes both decoders drop their buffers and discard
//! output until the target. After a seek the first video frame is presented even while
//! paused, so scrubbing shows the picture.
//!
//! `Drop` stops playback and joins the threads with a bounded wait ([`JOIN_TIMEOUT`]).
//! A thread that does not stop in time (stuck inside a demuxer or decoder call) is
//! detached and its FFmpeg contexts are *leaked*, never freed: freeing them from here
//! while FFmpeg is still inside them would be a use-after-free, and the thread itself
//! forgets them when it finally returns (`abandoned`). Every thread body runs under
//! `catch_unwind`; a panic or a decoding error surfaces in [`PlayerState::error`] and
//! stops the player, never the process.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::clock::Clock;
use crate::decode::{Decoder, Demuxer, Frame, Limits, Packet, Quality, Recv, Resampler, Scaler};
use crate::info::{Source, StreamInfo, StreamKind, stream_info};
use crate::queue::{Bounded, Pop, PushError, Signal};
use crate::sys::layout::AVRational;
use crate::sys::{Av, types};
use crate::{MediaError, Runtime};

/// An audio device the frontend provides (rodio in `soyuz-appcore`).
pub trait AudioOutput: Send + Sync {
    /// Open a stream of interleaved `f32` frames at `rate` Hz with `channels` channels.
    /// Called on the thread that calls [`Player::open`].
    fn open(&self, rate: u32, channels: u32) -> Box<dyn AudioSink>;
}

/// One open audio stream. `push` runs on the player's audio thread; the other methods
/// also from [`Player::send`] (the UI thread) and from the clock, so they must be cheap
/// and never block on the device.
pub trait AudioSink: Send {
    /// Queue frames without blocking; returns how many were accepted (the rest are
    /// offered again later).
    fn push(&mut self, interleaved: &[f32]) -> usize;
    /// Frames the device has played so far: the clock. Underruns play silence and still
    /// advance it (counted in the device callback, never blocked by the decoder).
    fn played_frames(&self) -> u64;
    fn pause(&mut self, on: bool);
    /// 0.0 … 1.0.
    fn set_volume(&mut self, v: f32);
    /// Drop queued frames (seek).
    fn flush(&mut self);
}

/// One decoded, scaled picture (tightly packed RGBA, `width * height * 4` bytes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FramePixels<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
    /// Presentation time (seconds).
    pub pts: f64,
}

/// Where presented frames go: the egui sink keeps one texture and updates it in place
/// when the size is unchanged; the terminal UI hands the pixels to its image worker.
/// Called on the video thread; must return quickly (copy, then wake the UI).
pub trait FrameSink: Send {
    fn present(&mut self, frame: FramePixels<'_>);
}

/// Hardware decoding preference (`[media] hw_decode`). V2a decodes in software whatever
/// the preference says; V4b (optional) adds the devices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HwPref {
    /// The platform's decoder when the codec has one, software otherwise.
    #[default]
    Auto,
    Off,
    D3d11va,
    Vaapi,
    Videotoolbox,
}

/// How to play.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerOptions {
    /// Longest edge of frames handed to the sink until `SetOutputSize` says otherwise.
    pub max_px: u32,
    /// Presentation cap (frames per second): frames due sooner than `1 / fps_cap` after
    /// the previous one are skipped (not counted as dropped).
    pub fps_cap: f32,
    /// Decoder threads, 0 = auto.
    pub threads: u32,
    pub hw: HwPref,
    pub autoplay: bool,
    /// Restart at the end.
    pub loop_: bool,
    /// 0.0 … 1.0.
    pub volume: f32,
    pub muted: bool,
}

impl Default for PlayerOptions {
    fn default() -> Self {
        PlayerOptions {
            max_px: 1280,
            fps_cap: 60.0,
            threads: 0,
            hw: HwPref::Auto,
            autoplay: false,
            loop_: false,
            volume: 0.8,
            muted: false,
        }
    }
}

/// Commands from the UI.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayerCmd {
    Play,
    Pause,
    TogglePlay,
    /// Seconds from the start.
    Seek(f64),
    /// Relative seconds (negative = back).
    SeekBy(f64),
    /// 0.0 … 1.0.
    SetVolume(f32),
    Mute(bool),
    /// Longest edge of the frames wanted from now on (the pane's pixel size).
    SetOutputSize(u32),
    Stop,
}

/// A snapshot for the UI (cheap to clone).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PlayerState {
    pub path: PathBuf,
    pub playing: bool,
    /// Reached the end (and `loop_` was off), or stopped.
    pub ended: bool,
    /// Seconds.
    pub pos: f64,
    pub duration: Option<f64>,
    /// Seconds decoded ahead of `pos` (audio pushed to the device; demuxed video
    /// without audio).
    pub buffered: f64,
    pub volume: f32,
    pub muted: bool,
    pub video: Option<StreamInfo>,
    pub audio: Option<StreamInfo>,
    /// Frames dropped for being late.
    pub dropped: u64,
    /// The active hardware decoder (`"d3d11va"`), `None` for software.
    pub hw: Option<String>,
    /// A decoding error that stopped playback.
    pub error: Option<String>,
}

/// How long `Drop` waits for the threads before detaching them (and leaking their
/// FFmpeg contexts).
pub const JOIN_TIMEOUT: Duration = Duration::from_millis(500);

/// A frame this much behind the clock is dropped instead of presented.
const LATE_SECS: f64 = 0.05;
/// … unless this many were dropped in a row: a machine too slow for the stream still
/// shows a picture now and then.
const MAX_CONSECUTIVE_DROPS: u32 = 8;
/// Tolerance when discarding output before a seek target.
const SEEK_EPS: f64 = 1e-3;
/// Packets queued ahead of each decoder.
const VIDEO_QUEUE: usize = 64;
const AUDIO_QUEUE: usize = 256;
/// Idle poll of the threads (commands wake them earlier through the signal).
const POLL: Duration = Duration::from_millis(20);
/// Minimum gap between two `wake` calls caused by frames.
const WAKE_GAP: Duration = Duration::from_millis(2);

/// What travels through a packet queue.
enum Item {
    Packet(Packet),
    /// A seek happened: drop buffered output, discard until `target` (`None`: the seek
    /// failed, only resynchronise with `serial`).
    Flush {
        serial: u64,
        target: Option<f64>,
    },
    /// The demuxer reached the end: drain the decoder.
    Eof,
}

type SharedSink = Arc<Mutex<Box<dyn AudioSink>>>;

/// Everything the threads and the handle share.
struct Shared {
    state: Mutex<PlayerState>,
    playing: AtomicBool,
    stop: AtomicBool,
    /// `Drop` gave up waiting: the threads forget their contexts instead of freeing them.
    abandoned: AtomicBool,
    /// Bumped by every seek request; flush markers carry it so the decoders can tell a
    /// stale frame from a fresh one.
    seek_serial: AtomicU64,
    pending_seek: Mutex<Option<f64>>,
    /// One `ended` / loop restart per end of file.
    end_handled: AtomicBool,
    max_px: AtomicU32,
    dropped: AtomicU64,
    /// Seconds decoded ahead (f64 bits): the last audio sample converted, or the last
    /// packet demuxed without audio.
    ahead: AtomicU64,
    video_eof: AtomicBool,
    audio_eof: AtomicBool,
    has_video: bool,
    has_audio: bool,
    loop_: bool,
    fps_cap: f32,
    duration: Option<f64>,
    clock: Clock,
    sink: Option<SharedSink>,
    signal: Signal,
    live: Mutex<usize>,
    live_cv: Condvar,
    wake: Arc<dyn Fn() + Send + Sync>,
    last_wake: Mutex<Instant>,
    vq: Bounded<Item>,
    aq: Bounded<Item>,
}

impl Shared {
    fn stopping(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    fn serial(&self) -> u64 {
        self.seek_serial.load(Ordering::Acquire)
    }

    fn ahead(&self) -> f64 {
        f64::from_bits(self.ahead.load(Ordering::Relaxed))
    }

    fn set_ahead(&self, secs: f64) {
        self.ahead.store(secs.to_bits(), Ordering::Relaxed);
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, PlayerState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn with_sink(&self, f: impl FnOnce(&mut dyn AudioSink)) {
        if let Some(s) = &self.sink {
            f(s.lock().unwrap_or_else(|e| e.into_inner()).as_mut());
        }
    }

    fn wake(&self) {
        (self.wake)();
    }

    /// `wake` for a new frame, at most every [`WAKE_GAP`].
    fn wake_frame(&self) {
        let mut last = self.last_wake.lock().unwrap_or_else(|e| e.into_inner());
        if last.elapsed() >= WAKE_GAP {
            *last = Instant::now();
            drop(last);
            self.wake();
        }
    }

    fn set_playing(&self, on: bool) {
        if self.stopping() {
            return;
        }
        if on {
            let ended = std::mem::take(&mut self.lock_state().ended);
            if ended {
                self.request_seek(0.0);
            }
        }
        self.playing.store(on, Ordering::Relaxed);
        if on {
            self.clock.resume();
        } else {
            self.clock.pause();
        }
        self.with_sink(|s| s.pause(!on));
    }

    fn request_seek(&self, at: f64) {
        if self.stopping() {
            return;
        }
        let mut at = if at.is_finite() { at.max(0.0) } else { 0.0 };
        if let Some(d) = self.duration {
            at = at.min(d);
        }
        self.lock_state().ended = false;
        self.end_handled.store(false, Ordering::Relaxed);
        *self.pending_seek.lock().unwrap_or_else(|e| e.into_inner()) = Some(at);
        self.seek_serial.fetch_add(1, Ordering::AcqRel);
        self.vq.interrupt();
        self.aq.interrupt();
        self.signal.notify();
    }

    fn take_pending_seek(&self) -> Option<f64> {
        self.pending_seek.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    fn set_volume(&self, volume: Option<f32>, muted: Option<bool>) {
        let mut s = self.lock_state();
        if let Some(v) = volume {
            s.volume = if v.is_finite() { v.clamp(0.0, 1.0) } else { s.volume };
        }
        if let Some(m) = muted {
            s.muted = m;
        }
        let effective = if s.muted { 0.0 } else { s.volume };
        drop(s);
        self.with_sink(|sink| sink.set_volume(effective));
    }

    /// Stop everything (`Stop`, `Drop`, an error).
    fn stop_all(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.playing.store(false, Ordering::Relaxed);
        self.clock.pause();
        self.with_sink(|s| s.pause(true));
        self.lock_state().ended = true;
        self.vq.close();
        self.aq.close();
        self.signal.notify();
    }

    /// A thread failed: keep the first error, stop the rest.
    fn fail(&self, what: String) {
        {
            let mut s = self.lock_state();
            if s.error.is_none() {
                s.error = Some(what);
            }
        }
        self.stop_all();
        self.wake();
    }

    /// Both decoders consumed their last frame: end, or restart when looping.
    fn maybe_ended(&self) {
        let video_done = !self.has_video || self.video_eof.load(Ordering::Acquire);
        let audio_done = !self.has_audio || self.audio_eof.load(Ordering::Acquire);
        if !(video_done && audio_done) || self.stopping() {
            return;
        }
        if self.end_handled.swap(true, Ordering::AcqRel) {
            return;
        }
        if self.loop_ {
            self.request_seek(0.0);
        } else {
            self.playing.store(false, Ordering::Relaxed);
            self.clock.pause();
            self.with_sink(|s| s.pause(true));
            self.lock_state().ended = true;
        }
        self.signal.notify();
        self.wake();
    }

    /// Called by every thread when it returns.
    fn thread_done(&self) {
        let mut n = self.live.lock().unwrap_or_else(|e| e.into_inner());
        *n = n.saturating_sub(1);
        drop(n);
        self.live_cv.notify_all();
    }
}

/// A playing file: see the module documentation for the threads.
pub struct Player {
    shared: Arc<Shared>,
    handles: Vec<JoinHandle<()>>,
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player").field("state", &self.state()).finish_non_exhaustive()
    }
}

/// The streams chosen at open time.
struct Picked {
    index: usize,
    decoder: Decoder,
    info: StreamInfo,
    time_base: AVRational,
}

impl Player {
    /// Open `src` and start the threads (paused unless `opts.autoplay`). `frames` receives
    /// every presented picture, `audio` (when `Some`) the samples, and `wake` is called
    /// whenever the state or the frame changed, so the UI repaints.
    ///
    /// Errors here are synchronous: no runtime, no decodable video *and* no audio
    /// stream, an unsupported codec, or an unreadable source. Later errors surface in
    /// [`PlayerState::error`].
    pub fn open(
        rt: Arc<Runtime>,
        src: Source,
        opts: PlayerOptions,
        frames: Box<dyn FrameSink>,
        audio: Option<Arc<dyn AudioOutput>>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Player, MediaError> {
        let av = rt.av();
        let d: Demuxer<'static> = Demuxer::open(av, &src, None, Limits::default())?;
        let threads = if opts.threads == 0 { "auto".to_string() } else { opts.threads.to_string() };
        let video = pick(av, &d, StreamKind::Video, &[("threads", threads.as_str())])?;
        let audio_stream = if audio.is_some() { pick(av, &d, StreamKind::Audio, &[("threads", "1")])? } else { None };
        if video.is_none() && audio_stream.is_none() {
            // An audio file without an output: nothing to play.
            return Err(if audio.is_some() {
                MediaError::NoStream(StreamKind::Video)
            } else {
                MediaError::Unsupported("audio playback needs an audio output".into())
            });
        }
        let start = d.start_time();
        let duration = d.duration();

        // The audio device, its sink and the clock.
        let mut sink: Option<SharedSink> = None;
        let mut audio_rate = 0u32;
        let audio_channels = match (&audio_stream, &audio) {
            (Some(a), Some(out)) => {
                let channels = a.decoder.ch_layout().map(|l| l.channels()).unwrap_or(0);
                let channels = if channels > 0 { channels } else { a.info.channels };
                audio_rate = a.info.sample_rate;
                if audio_rate > 0 && channels > 0 {
                    let mut s = out.open(audio_rate, channels);
                    s.set_volume(if opts.muted { 0.0 } else { opts.volume.clamp(0.0, 1.0) });
                    s.pause(!opts.autoplay);
                    sink = Some(Arc::new(Mutex::new(s)));
                }
                Some(channels)
            }
            _ => None,
        };
        let audio_stream = if sink.is_some() { audio_stream } else { None };
        if video.is_none() && audio_stream.is_none() {
            return Err(MediaError::Unsupported("no decodable audio parameters".into()));
        }
        let clock = match &sink {
            Some(s) => {
                let s = Arc::clone(s);
                Clock::audio(audio_rate, Arc::new(move || s.lock().unwrap_or_else(|e| e.into_inner()).played_frames()))
            }
            None => Clock::wall(),
        };
        if opts.autoplay {
            clock.resume();
        }

        let state = PlayerState {
            path: src.path().map(PathBuf::from).unwrap_or_default(),
            playing: opts.autoplay,
            duration,
            volume: opts.volume.clamp(0.0, 1.0),
            muted: opts.muted,
            video: video.as_ref().map(|v| v.info.clone()),
            audio: audio_stream.as_ref().map(|a| a.info.clone()),
            ..Default::default()
        };
        let threads_to_start = 1 + usize::from(video.is_some()) + usize::from(audio_stream.is_some());
        let shared = Arc::new(Shared {
            state: Mutex::new(state),
            playing: AtomicBool::new(opts.autoplay),
            stop: AtomicBool::new(false),
            abandoned: AtomicBool::new(false),
            seek_serial: AtomicU64::new(0),
            pending_seek: Mutex::new(None),
            end_handled: AtomicBool::new(false),
            max_px: AtomicU32::new(opts.max_px.max(1)),
            dropped: AtomicU64::new(0),
            ahead: AtomicU64::new(0f64.to_bits()),
            video_eof: AtomicBool::new(false),
            audio_eof: AtomicBool::new(false),
            has_video: video.is_some(),
            has_audio: audio_stream.is_some(),
            loop_: opts.loop_,
            fps_cap: opts.fps_cap,
            duration,
            clock,
            sink,
            signal: Signal::new(),
            live: Mutex::new(threads_to_start),
            live_cv: Condvar::new(),
            wake,
            last_wake: Mutex::new(Instant::now() - WAKE_GAP),
            vq: Bounded::new(VIDEO_QUEUE),
            aq: Bounded::new(AUDIO_QUEUE),
        });

        let mut handles = Vec::with_capacity(threads_to_start);
        let seek_on =
            video.as_ref().map(|v| (v.index, v.time_base)).or(audio_stream.as_ref().map(|a| (a.index, a.time_base)));
        let demux = DemuxThread {
            av,
            d,
            vidx: video.as_ref().map(|v| v.index),
            aidx: audio_stream.as_ref().map(|a| a.index),
            seek_on,
            start,
        };
        handles.push(spawn("soyuz-media demux", Arc::clone(&shared), demux)?);
        if let Some(v) = video {
            let rotation = v.info.rotation;
            let sar = AVRational::new(v.info.sar.0, v.info.sar.1);
            let worker = VideoThread {
                av,
                dec: v.decoder,
                sink: frames,
                tb: v.time_base,
                start,
                rotation,
                sar,
                scaler: None,
                rgba: Vec::new(),
                rotated: Vec::new(),
                serial: 0,
                skip_until: None,
                present_first: true,
                consecutive_drops: 0,
                last_present: None,
                eof: false,
            };
            handles.push(spawn("soyuz-media video", Arc::clone(&shared), worker)?);
        }
        if let (Some(a), Some(channels)) = (audio_stream, audio_channels) {
            let worker = AudioThread {
                av,
                dec: a.decoder,
                tb: a.time_base,
                start,
                channels,
                rate: audio_rate,
                rs: None,
                pending: Vec::new(),
                pending_pos: 0,
                skip_until: None,
                serial: 0,
                end: 0.0,
                eof: false,
            };
            handles.push(spawn("soyuz-media audio", Arc::clone(&shared), worker)?);
        }
        drop(rt);
        Ok(Player { shared, handles })
    }

    /// Queue a command (never blocks on I/O; `Play` / `Pause` / volume apply at once,
    /// seeks on the demux thread).
    pub fn send(&self, cmd: PlayerCmd) {
        let sh = &self.shared;
        match cmd {
            PlayerCmd::Play => sh.set_playing(true),
            PlayerCmd::Pause => sh.set_playing(false),
            PlayerCmd::TogglePlay => sh.set_playing(!sh.playing()),
            PlayerCmd::Seek(at) => sh.request_seek(at),
            PlayerCmd::SeekBy(by) => sh.request_seek(sh.clock.now() + by),
            PlayerCmd::SetVolume(v) => sh.set_volume(Some(v), None),
            PlayerCmd::Mute(on) => sh.set_volume(None, Some(on)),
            PlayerCmd::SetOutputSize(px) => sh.max_px.store(px.max(1), Ordering::Relaxed),
            PlayerCmd::Stop => sh.stop_all(),
        }
        sh.signal.notify();
        sh.wake();
    }

    /// The current snapshot.
    pub fn state(&self) -> PlayerState {
        let sh = &self.shared;
        let mut pos = sh.clock.now();
        if let Some(d) = sh.duration {
            pos = pos.min(d);
        }
        let ahead = sh.ahead();
        let mut s = sh.lock_state().clone();
        s.playing = sh.playing();
        s.pos = pos;
        s.buffered = (ahead - pos).max(0.0);
        s.dropped = sh.dropped.load(Ordering::Relaxed);
        s
    }

    /// A handle without threads (unit tests of the command semantics).
    #[cfg(test)]
    fn detached(state: PlayerState, wake: Arc<dyn Fn() + Send + Sync>) -> Player {
        let duration = state.duration;
        let shared = Arc::new(Shared {
            state: Mutex::new(state),
            playing: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            abandoned: AtomicBool::new(false),
            seek_serial: AtomicU64::new(0),
            pending_seek: Mutex::new(None),
            end_handled: AtomicBool::new(false),
            max_px: AtomicU32::new(64),
            dropped: AtomicU64::new(0),
            ahead: AtomicU64::new(0f64.to_bits()),
            video_eof: AtomicBool::new(false),
            audio_eof: AtomicBool::new(false),
            has_video: true,
            has_audio: false,
            loop_: false,
            fps_cap: 60.0,
            duration,
            clock: Clock::wall(),
            sink: None,
            signal: Signal::new(),
            live: Mutex::new(0),
            live_cv: Condvar::new(),
            wake,
            last_wake: Mutex::new(Instant::now()),
            vq: Bounded::new(4),
            aq: Bounded::new(4),
        });
        Player { shared, handles: Vec::new() }
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let sh = &self.shared;
        sh.stop_all();
        let deadline = Instant::now() + JOIN_TIMEOUT;
        let mut live = sh.live.lock().unwrap_or_else(|e| e.into_inner());
        while *live > 0 {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            live = sh.live_cv.wait_timeout(live, deadline - now).unwrap_or_else(|e| e.into_inner()).0;
        }
        let stuck = *live;
        drop(live);
        if stuck == 0 {
            for h in self.handles.drain(..) {
                let _ = h.join();
            }
        } else {
            // Detach: the threads are inside FFmpeg (or a sink); when they return they
            // forget their contexts (never freed, see the module documentation).
            sh.abandoned.store(true, Ordering::SeqCst);
            log::warn!(
                "soyuz-media: {stuck} player thread(s) did not stop within {JOIN_TIMEOUT:?}; detached, contexts leaked"
            );
            self.handles.clear();
        }
    }
}

/// The best stream of `kind` with an open decoder; `None` when the file has none (cover
/// art is not a video stream). An unsupported codec is an error.
fn pick(
    av: &'static Av,
    d: &Demuxer<'static>,
    kind: StreamKind,
    options: &[(&str, &str)],
) -> Result<Option<Picked>, MediaError> {
    let (index, decoder) = match d.best_stream(kind, None) {
        Ok(found) => found,
        Err(MediaError::NoStream(_)) => return Ok(None),
        Err(e) => return Err(e),
    };
    let s = d.stream(index).ok_or(MediaError::NoStream(kind))?;
    if s.kind() != Some(kind) || s.disposition() & types::AV_DISPOSITION_ATTACHED_PIC != 0 {
        return Ok(None);
    }
    let info = stream_info(av, d, s);
    let time_base = s.time_base();
    let decoder = Decoder::open(av, decoder, s.par_ptr(), time_base, options)?;
    Ok(Some(Picked { index, decoder, info, time_base }))
}

/// One of the player's threads: `run` does the work, `finish` frees or forgets the
/// FFmpeg contexts depending on whether `Drop` gave up waiting.
trait Worker: Send + 'static {
    fn run(&mut self, sh: &Shared) -> Result<(), MediaError>;
}

fn spawn<W: Worker>(name: &str, sh: Arc<Shared>, mut worker: W) -> Result<JoinHandle<()>, MediaError> {
    let label = name.to_string();
    std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            let r = catch_unwind(AssertUnwindSafe(|| worker.run(&sh)));
            match r {
                Ok(Ok(())) => {}
                Ok(Err(MediaError::Stale | MediaError::Cancelled)) => {}
                Ok(Err(e)) => sh.fail(e.to_string()),
                Err(_) => sh.fail(format!("internal error in the {label} thread")),
            }
            if sh.abandoned.load(Ordering::SeqCst) {
                // The handle gave up on us: FFmpeg may have been mid-call when the stop
                // was requested; the contexts are leaked on purpose.
                std::mem::forget(worker);
            } else {
                drop(worker);
            }
            sh.thread_done();
        })
        .map_err(MediaError::Io)
}

/// Seconds of player time of a timestamp in `tb` (`start` = the container's start).
fn secs_of(tb: AVRational, ts: i64, start: f64) -> Option<f64> {
    tb.secs(ts).map(|s| s - start)
}

/// Timestamp in `tb` of `secs` of player time.
fn ts_of(tb: AVRational, secs: f64, start: f64) -> Option<i64> {
    (tb.num > 0 && tb.den > 0).then(|| ((start + secs) * f64::from(tb.den) / f64::from(tb.num)).round() as i64)
}

/// `true` when a sleep should stop: stop requested or a newer seek than `serial`.
fn interrupted(sh: &Shared, serial: u64) -> bool {
    sh.stopping() || sh.serial() != serial
}

// ---------------------------------------------------------------- demux thread

struct DemuxThread {
    av: &'static Av,
    d: Demuxer<'static>,
    vidx: Option<usize>,
    aidx: Option<usize>,
    /// The stream seeks are expressed in (video when there is one).
    seek_on: Option<(usize, AVRational)>,
    start: f64,
}

impl DemuxThread {
    /// Seek to the keyframe at or before `at`; `Ok` means the queues were flushed.
    fn seek(&mut self, sh: &Shared, at: f64) -> Result<(), MediaError> {
        let (idx, tb) = self.seek_on.ok_or_else(|| MediaError::Unsupported("nothing to seek".into()))?;
        let ts = ts_of(tb, at, self.start).ok_or_else(|| MediaError::Unsupported("stream has no time base".into()))?;
        self.d.seek_before(Some(idx), ts)?;
        sh.vq.flush();
        sh.aq.flush();
        sh.video_eof.store(false, Ordering::Release);
        sh.audio_eof.store(false, Ordering::Release);
        sh.end_handled.store(false, Ordering::Relaxed);
        if !sh.clock.is_audio() {
            sh.clock.set(at);
        }
        sh.set_ahead(at);
        Ok(())
    }

    fn push(&self, sh: &Shared, to: Option<usize>, item: Item) -> Result<bool, PushError> {
        match to {
            Some(i) if Some(i) == self.vidx => sh.vq.push(item).map(|()| true),
            Some(i) if Some(i) == self.aidx => sh.aq.push(item).map(|()| true),
            _ => Ok(false),
        }
    }
}

impl Worker for DemuxThread {
    fn run(&mut self, sh: &Shared) -> Result<(), MediaError> {
        let mut at_eof = false;
        loop {
            if sh.stopping() {
                return Ok(());
            }
            if let Some(at) = sh.take_pending_seek() {
                let serial = sh.serial();
                sh.vq.clear_interrupt();
                sh.aq.clear_interrupt();
                let target = match self.seek(sh, at) {
                    Ok(()) => {
                        at_eof = false;
                        Some(at)
                    }
                    Err(e) => {
                        log::debug!("soyuz-media: seek to {at:.3}s failed: {e}");
                        None
                    }
                };
                for idx in [self.vidx, self.aidx] {
                    match self.push(sh, idx, Item::Flush { serial, target }) {
                        Ok(_) | Err(PushError::Interrupted) => {}
                        Err(PushError::Closed) => return Ok(()),
                    }
                }
                sh.signal.notify();
                sh.wake();
                continue;
            }
            if at_eof {
                sh.signal.wait_timeout(POLL);
                continue;
            }
            let mut pkt = Packet::new(self.av)?;
            if !self.d.read(&mut pkt)? {
                let mut all = true;
                for idx in [self.vidx, self.aidx] {
                    match self.push(sh, idx, Item::Eof) {
                        Ok(_) => {}
                        Err(PushError::Interrupted) => all = false,
                        Err(PushError::Closed) => return Ok(()),
                    }
                }
                // Interrupted: a seek is pending; otherwise the next read is EOF again.
                at_eof = all;
                continue;
            }
            if self.d.stopped() {
                return Err(MediaError::Stale);
            }
            let idx = pkt.stream_index();
            let tb = self.d.stream(idx).map(|s| s.time_base());
            let end = tb.and_then(|tb| {
                let ts = if pkt.pts() != types::AV_NOPTS_VALUE { pkt.pts() } else { pkt.dts() };
                secs_of(tb, ts, self.start).map(|s| s + tb.secs(pkt.duration().max(0)).unwrap_or(0.0))
            });
            match self.push(sh, Some(idx), Item::Packet(pkt)) {
                Ok(true) => {
                    if let Some(end) = end
                        && !sh.has_audio
                        && end > sh.ahead()
                    {
                        sh.set_ahead(end);
                    }
                }
                Ok(false) | Err(PushError::Interrupted) => {}
                Err(PushError::Closed) => return Ok(()),
            }
        }
    }
}

// ---------------------------------------------------------------- video thread

struct VideoThread {
    av: &'static Av,
    dec: Decoder,
    sink: Box<dyn FrameSink>,
    tb: AVRational,
    start: f64,
    rotation: i32,
    sar: AVRational,
    scaler: Option<Scaler>,
    rgba: Vec<u8>,
    rotated: Vec<u8>,
    /// The seek serial of the last flush marker.
    serial: u64,
    skip_until: Option<f64>,
    /// Present the next frame at once (first picture, after a seek), even when paused.
    present_first: bool,
    consecutive_drops: u32,
    last_present: Option<Instant>,
    eof: bool,
}

/// What to do after a frame.
enum Flow {
    Continue,
    /// A seek or stop arrived while waiting: drop everything decoded so far.
    Abandon,
}

impl VideoThread {
    fn run_frame(&mut self, sh: &Shared, frame: &Frame) -> Result<Flow, MediaError> {
        let ts = if frame.best_effort_timestamp() != types::AV_NOPTS_VALUE {
            frame.best_effort_timestamp()
        } else {
            frame.pts()
        };
        let pts = secs_of(self.tb, ts, self.start);
        if let (Some(until), Some(p)) = (self.skip_until, pts) {
            if p + SEEK_EPS < until {
                return Ok(Flow::Continue);
            }
            self.skip_until = None;
        }
        // Wait for the frame's time (or present at once after a seek / at the start).
        let now = loop {
            if interrupted(sh, self.serial) {
                return Ok(Flow::Abandon);
            }
            let now = sh.clock.now();
            if self.present_first {
                break now;
            }
            if !sh.playing() {
                sh.signal.wait_timeout(POLL * 5);
                continue;
            }
            match pts {
                Some(p) if p > now + 0.001 => {
                    let wait = Duration::from_secs_f64((p - now).min(POLL.as_secs_f64()));
                    sh.signal.wait_timeout(wait);
                }
                _ => break now,
            }
        };
        if let Some(p) = pts
            && !self.present_first
            && now - p > LATE_SECS
            && self.consecutive_drops < MAX_CONSECUTIVE_DROPS
        {
            self.consecutive_drops += 1;
            sh.dropped.fetch_add(1, Ordering::Relaxed);
            return Ok(Flow::Continue);
        }
        self.consecutive_drops = 0;
        if !self.present_first
            && sh.fps_cap > 0.0
            && let Some(last) = self.last_present
            && last.elapsed().as_secs_f64() < 1.0 / f64::from(sh.fps_cap)
        {
            return Ok(Flow::Continue);
        }
        self.present_first = false;
        self.present(sh, frame, pts.unwrap_or(now))?;
        Ok(Flow::Continue)
    }

    fn present(&mut self, sh: &Shared, frame: &Frame, pts: f64) -> Result<(), MediaError> {
        let (fw, fh) = (frame.width(), frame.height());
        if fw <= 0 || fh <= 0 {
            return Ok(());
        }
        let max_px = sh.max_px.load(Ordering::Relaxed);
        let (dw, dh) = fit_dims(
            u32::try_from(fw).unwrap_or(1),
            u32::try_from(fh).unwrap_or(1),
            (self.sar.num, self.sar.den),
            max_px,
        );
        let fresh = self.scaler.as_ref().is_none_or(|s| !s.matches(frame) || s.dst_w != dw || s.dst_h != dh);
        if fresh {
            self.scaler = Some(Scaler::new(self.av, fw, fh, frame.format(), dw, dh, Quality::Fast)?);
        }
        let sc = self.scaler.as_mut().expect("set above");
        sc.scale(frame, &mut self.rgba)?;
        let (w, h, pixels) = if self.rotation == 0 {
            (dw, dh, self.rgba.as_slice())
        } else {
            let (w, h) = rotate_rgba(&self.rgba, dw, dh, self.rotation, &mut self.rotated);
            (w, h, self.rotated.as_slice())
        };
        self.sink.present(FramePixels { width: w, height: h, rgba: pixels, pts });
        self.last_present = Some(Instant::now());
        sh.wake_frame();
        Ok(())
    }

    /// Pull every frame the decoder has; `Abandon` when a seek / stop interrupted.
    fn drain(&mut self, sh: &Shared, frame: &mut Frame) -> Result<(Flow, bool), MediaError> {
        loop {
            match self.dec.receive(frame)? {
                Recv::Frame => {}
                Recv::Again => return Ok((Flow::Continue, false)),
                Recv::Eof => return Ok((Flow::Continue, true)),
            }
            if let Flow::Abandon = self.run_frame(sh, frame)? {
                return Ok((Flow::Abandon, false));
            }
        }
    }
}

impl Worker for VideoThread {
    fn run(&mut self, sh: &Shared) -> Result<(), MediaError> {
        let mut frame = Frame::new(self.av)?;
        loop {
            if sh.stopping() {
                return Ok(());
            }
            match sh.vq.pop_timeout(POLL) {
                Pop::Closed => return Ok(()),
                Pop::Empty => {}
                Pop::Item(Item::Flush { serial, target }) => {
                    if target.is_some() {
                        self.dec.flush();
                        self.skip_until = target;
                        self.present_first = true;
                        self.consecutive_drops = 0;
                        self.eof = false;
                    }
                    self.serial = serial;
                }
                Pop::Item(Item::Packet(pkt)) => {
                    if self.eof {
                        continue;
                    }
                    self.dec.send(Some(&pkt))?;
                    drop(pkt);
                    let _ = self.drain(sh, &mut frame)?;
                }
                Pop::Item(Item::Eof) => {
                    if self.eof {
                        continue;
                    }
                    self.dec.send(None)?;
                    let (flow, _) = self.drain(sh, &mut frame)?;
                    if let Flow::Abandon = flow {
                        continue;
                    }
                    self.eof = true;
                    sh.video_eof.store(true, Ordering::Release);
                    sh.maybe_ended();
                }
            }
        }
    }
}

// ---------------------------------------------------------------- audio thread

struct AudioThread {
    av: &'static Av,
    dec: Decoder,
    tb: AVRational,
    start: f64,
    /// The sink's channel count (the decoder's at open time).
    channels: u32,
    /// The sink's rate.
    rate: u32,
    rs: Option<Resampler>,
    pending: Vec<f32>,
    pending_pos: usize,
    skip_until: Option<f64>,
    serial: u64,
    /// Player time of the end of the last converted frame.
    end: f64,
    eof: bool,
}

impl AudioThread {
    /// Resample `frame` into `pending`, honouring `skip_until`.
    fn convert(&mut self, sh: &Shared, frame: &Frame) -> Result<(), MediaError> {
        let (fmt, in_rate, n) = (frame.format(), frame.sample_rate(), frame.nb_samples());
        if n <= 0 || in_rate <= 0 {
            return Ok(());
        }
        let fresh = self.rs.as_ref().is_none_or(|rs| !rs.matches(fmt, in_rate, self.channels));
        if fresh {
            // The decoder's layout can change with the stream (parameter changes); the
            // device was opened for `channels`, so another count is an error.
            let layout = self.dec.ch_layout()?;
            if layout.channels() != self.channels {
                return Err(MediaError::Unsupported(format!(
                    "the audio stream changed from {} to {} channels",
                    self.channels,
                    layout.channels()
                )));
            }
            self.rs = Some(Resampler::new(self.av, &layout, fmt, in_rate, self.rate)?);
        }
        let ch = self.channels.max(1) as usize;
        let before = self.pending.len();
        let rs = self.rs.as_mut().expect("set above");
        rs.convert(Some(frame), &mut self.pending)?;
        let ts = if frame.best_effort_timestamp() != types::AV_NOPTS_VALUE {
            frame.best_effort_timestamp()
        } else {
            frame.pts()
        };
        let secs = secs_of(self.tb, ts, self.start);
        if let Some(skip) = self.skip_until {
            match secs {
                Some(secs) if secs < skip => {
                    let produced = (self.pending.len() - before) / ch;
                    let drop = (((skip - secs) * f64::from(self.rate)).round() as usize).min(produced);
                    self.pending.drain(before..before + drop * ch);
                    if drop < produced {
                        self.skip_until = None;
                    }
                }
                _ => self.skip_until = None,
            }
        }
        let produced = (self.pending.len() - before) / ch;
        let base = secs.unwrap_or(self.end);
        self.end = base.max(self.end) + produced as f64 / f64::from(self.rate);
        sh.set_ahead(self.end);
        Ok(())
    }

    /// Offer `pending` to the sink; `true` when everything went.
    fn push_pending(&mut self, sh: &Shared) -> bool {
        if self.pending_pos >= self.pending.len() {
            self.pending.clear();
            self.pending_pos = 0;
            return true;
        }
        let ch = self.channels.max(1) as usize;
        let mut accepted = 0;
        sh.with_sink(|s| accepted = s.push(&self.pending[self.pending_pos..]));
        self.pending_pos += (accepted * ch).min(self.pending.len() - self.pending_pos);
        if self.pending_pos >= self.pending.len() {
            self.pending.clear();
            self.pending_pos = 0;
            true
        } else {
            false
        }
    }

    fn drain(&mut self, sh: &Shared, frame: &mut Frame) -> Result<bool, MediaError> {
        loop {
            match self.dec.receive(frame)? {
                Recv::Frame => self.convert(sh, frame)?,
                Recv::Again => return Ok(false),
                Recv::Eof => return Ok(true),
            }
        }
    }
}

impl Worker for AudioThread {
    fn run(&mut self, sh: &Shared) -> Result<(), MediaError> {
        let mut frame = Frame::new(self.av)?;
        loop {
            if sh.stopping() {
                return Ok(());
            }
            if sh.serial() != self.serial {
                // A seek is on its way: whatever is pending belongs to the old position.
                self.pending.clear();
                self.pending_pos = 0;
            } else if !self.push_pending(sh) {
                // The device is full (or paused): try again shortly.
                sh.signal.wait_timeout(Duration::from_millis(5));
                continue;
            }
            match sh.aq.pop_timeout(POLL) {
                Pop::Closed => return Ok(()),
                Pop::Empty => {}
                Pop::Item(Item::Flush { serial, target }) => {
                    if let Some(target) = target {
                        self.dec.flush();
                        self.rs = None;
                        self.pending.clear();
                        self.pending_pos = 0;
                        self.skip_until = Some(target);
                        self.eof = false;
                        self.end = target;
                        sh.set_ahead(target);
                        sh.with_sink(|s| s.flush());
                        sh.clock.set(target);
                    }
                    self.serial = serial;
                }
                Pop::Item(Item::Packet(pkt)) => {
                    if self.eof {
                        continue;
                    }
                    self.dec.send(Some(&pkt))?;
                    drop(pkt);
                    self.drain(sh, &mut frame)?;
                }
                Pop::Item(Item::Eof) => {
                    if self.eof {
                        continue;
                    }
                    self.dec.send(None)?;
                    self.drain(sh, &mut frame)?;
                    if let Some(rs) = self.rs.as_mut() {
                        let before = self.pending.len();
                        rs.convert(None, &mut self.pending)?;
                        let tail = (self.pending.len() - before) / self.channels.max(1) as usize;
                        self.end += tail as f64 / f64::from(self.rate);
                        sh.set_ahead(self.end);
                    }
                    // Hand the device the rest, then let it play out before `ended`.
                    let serial = self.serial;
                    while !interrupted(sh, serial) && !self.push_pending(sh) {
                        sh.signal.wait_timeout(Duration::from_millis(5));
                    }
                    while !interrupted(sh, serial) && sh.clock.now() + 0.005 < self.end {
                        sh.signal.wait_timeout(POLL);
                    }
                    if interrupted(sh, serial) {
                        continue;
                    }
                    self.eof = true;
                    sh.audio_eof.store(true, Ordering::Release);
                    sh.maybe_ended();
                }
            }
        }
    }
}

// ---------------------------------------------------------------- pixels

/// The output size for a `w × h` picture with sample aspect ratio `sar` fitted into
/// `max_px` on its longest edge (never upscaled).
pub(crate) fn fit_dims(w: u32, h: u32, sar: (i32, i32), max_px: u32) -> (u32, u32) {
    let (mut fw, mut fh) = (f64::from(w.max(1)), f64::from(h.max(1)));
    if sar.0 > 0 && sar.1 > 0 && sar.0 != sar.1 {
        if sar.0 > sar.1 {
            fw *= f64::from(sar.0) / f64::from(sar.1);
        } else {
            fh *= f64::from(sar.1) / f64::from(sar.0);
        }
    }
    let scale = (f64::from(max_px.max(1)) / fw.max(fh)).min(1.0);
    (((fw * scale).round() as u32).max(1), ((fh * scale).round() as u32).max(1))
}

/// Rotate a tightly packed RGBA picture clockwise by `rotation` degrees (90 / 180 /
/// 270; anything else copies) into `out`; returns the rotated size.
pub(crate) fn rotate_rgba(src: &[u8], w: u32, h: u32, rotation: i32, out: &mut Vec<u8>) -> (u32, u32) {
    let (w, h) = (w as usize, h as usize);
    out.clear();
    out.resize(w * h * 4, 0);
    let px = |x: usize, y: usize| &src[(y * w + x) * 4..(y * w + x) * 4 + 4];
    match rotation {
        90 => {
            // (x, y) → (h - 1 - y, x) in an h-wide picture.
            for y in 0..h {
                for x in 0..w {
                    let i = (x * h + (h - 1 - y)) * 4;
                    out[i..i + 4].copy_from_slice(px(x, y));
                }
            }
            (h as u32, w as u32)
        }
        180 => {
            for y in 0..h {
                for x in 0..w {
                    let i = ((h - 1 - y) * w + (w - 1 - x)) * 4;
                    out[i..i + 4].copy_from_slice(px(x, y));
                }
            }
            (w as u32, h as u32)
        }
        270 => {
            // (x, y) → (y, w - 1 - x) in an h-wide picture.
            for y in 0..h {
                for x in 0..w {
                    let i = ((w - 1 - x) * h + y) * 4;
                    out[i..i + 4].copy_from_slice(px(x, y));
                }
            }
            (h as u32, w as u32)
        }
        _ => {
            out.copy_from_slice(&src[..w * h * 4]);
            (w as u32, h as u32)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn commands_update_the_snapshot_and_wake() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = Arc::clone(&wakes);
        let p = Player::detached(
            PlayerState { duration: Some(10.0), ..Default::default() },
            Arc::new(move || {
                w.fetch_add(1, Ordering::Relaxed);
            }),
        );
        p.send(PlayerCmd::Play);
        assert!(p.state().playing);
        p.send(PlayerCmd::Pause);
        p.send(PlayerCmd::Seek(3.0));
        assert_eq!(p.shared.take_pending_seek(), Some(3.0), "queued for the demux thread");
        p.send(PlayerCmd::Seek(-5.0));
        assert_eq!(p.shared.take_pending_seek(), Some(0.0), "never before the start");
        p.send(PlayerCmd::Seek(50.0));
        assert_eq!(p.shared.take_pending_seek(), Some(10.0), "never past the end");
        p.send(PlayerCmd::SetVolume(1.5));
        p.send(PlayerCmd::Mute(true));
        p.send(PlayerCmd::SetOutputSize(0));
        assert_eq!(p.shared.max_px.load(Ordering::Relaxed), 1, "at least one pixel");
        let s = p.state();
        assert!(!s.playing && s.muted);
        assert!(s.pos < 0.01, "the wall clock ran only between Play and Pause: {}", s.pos);
        assert_eq!(s.volume, 1.0, "clamped");
        p.send(PlayerCmd::TogglePlay);
        assert!(p.state().playing);
        p.send(PlayerCmd::Stop);
        assert!(!p.state().playing && p.state().ended);
        p.send(PlayerCmd::Play);
        assert!(!p.state().playing, "stopped stays stopped");
        assert!(wakes.load(Ordering::Relaxed) >= 10);
        assert!(format!("{p:?}").contains("ended: true"));
        assert_eq!(PlayerOptions::default().hw, HwPref::Auto);
    }

    #[test]
    fn play_after_the_end_restarts() {
        let p = Player::detached(PlayerState { duration: Some(2.0), ..Default::default() }, Arc::new(|| {}));
        p.shared.lock_state().ended = true;
        p.send(PlayerCmd::Play);
        let s = p.state();
        assert!(s.playing && !s.ended);
        assert_eq!(p.shared.take_pending_seek(), Some(0.0));
        assert_eq!(p.shared.serial(), 1);
        // Errors stop the player and are kept (the first one).
        p.shared.fail("boom".into());
        p.shared.fail("later".into());
        let s = p.state();
        assert_eq!(s.error.as_deref(), Some("boom"));
        assert!(s.ended && !s.playing);
    }

    #[test]
    fn fit_dims_matches_grab_frame() {
        assert_eq!(fit_dims(32, 24, (0, 0), 16), (16, 12));
        assert_eq!(fit_dims(32, 24, (1, 1), 4096), (32, 24), "never upscaled");
        assert_eq!(fit_dims(720, 576, (16, 11), 2000), (1047, 576), "anamorphic widens");
        assert_eq!(fit_dims(720, 576, (11, 16), 2000), (720, 838), "… or heightens");
        assert_eq!(fit_dims(0, 0, (0, 0), 0), (1, 1));
    }

    #[test]
    fn rotation_of_a_two_by_three_picture() {
        // 2 wide, 3 tall; pixel value = its index.
        let src: Vec<u8> = (0..6u8).flat_map(|i| [i, i, i, 255]).collect();
        let reds = |v: &[u8]| v.chunks(4).map(|p| p[0]).collect::<Vec<_>>();
        let mut out = Vec::new();
        assert_eq!(rotate_rgba(&src, 2, 3, 90, &mut out), (3, 2));
        assert_eq!(reds(&out), [4, 2, 0, 5, 3, 1]);
        assert_eq!(rotate_rgba(&src, 2, 3, 180, &mut out), (2, 3));
        assert_eq!(reds(&out), [5, 4, 3, 2, 1, 0]);
        assert_eq!(rotate_rgba(&src, 2, 3, 270, &mut out), (3, 2));
        assert_eq!(reds(&out), [1, 3, 5, 0, 2, 4]);
        assert_eq!(rotate_rgba(&src, 2, 3, 0, &mut out), (2, 3));
        assert_eq!(out, src);
        // Agrees with the `image` crate's rotations.
        let img = image::RgbaImage::from_raw(2, 3, src.clone()).unwrap();
        rotate_rgba(&src, 2, 3, 90, &mut out);
        assert_eq!(out, image::imageops::rotate90(&img).into_raw());
        rotate_rgba(&src, 2, 3, 270, &mut out);
        assert_eq!(out, image::imageops::rotate270(&img).into_raw());
    }

    #[test]
    fn timestamps_round_trip() {
        let tb = AVRational::new(1, 30);
        assert_eq!(ts_of(tb, 1.0, 0.5), Some(45));
        assert_eq!(secs_of(tb, 45, 0.5), Some(1.0));
        assert_eq!(secs_of(tb, types::AV_NOPTS_VALUE, 0.0), None);
        assert_eq!(ts_of(AVRational::new(0, 0), 1.0, 0.0), None);
    }
}
