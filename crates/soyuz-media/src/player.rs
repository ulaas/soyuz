//! Audio / video playback ([`Player`]).
//!
//! Threads per open file (V2a): a demux thread feeds bounded packet queues; the video
//! decode thread scales frames to RGBA at the requested output size and hands them to the
//! [`FrameSink`] when their pts is due; the audio decode thread resamples to `f32` and
//! pushes into the [`AudioSink`], whose `played_frames` is the master clock (wall clock
//! without audio). The UI thread only sends [`PlayerCmd`]s and reads [`PlayerState`].
//!
//! `Drop` stops playback and joins the threads with a bounded wait; a thread stuck in a
//! demuxer leaves its FFmpeg contexts *leaked*, never freed (freeing under a running call
//! would be a use-after-free inside FFmpeg).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::info::{Source, StreamInfo};
use crate::{MediaError, Runtime};

/// An audio device the frontend provides (rodio in `soyuz-appcore`).
pub trait AudioOutput: Send + Sync {
    /// Open a stream of interleaved `f32` frames at `rate` Hz with `channels` channels.
    fn open(&self, rate: u32, channels: u32) -> Box<dyn AudioSink>;
}

/// One open audio stream.
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

/// Hardware decoding preference (`[media] hw_decode`).
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
    /// Presentation cap (frames per second); late frames are dropped.
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
    /// Reached the end (and `loop_` was off).
    pub ended: bool,
    /// Seconds.
    pub pos: f64,
    pub duration: Option<f64>,
    /// Seconds of audio queued ahead of `pos`.
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

/// A playing file. V0 stub: [`open`](Self::open) fails with [`MediaError::NotLoaded`];
/// `send` / `state` work on the snapshot alone (V2a adds the threads).
pub struct Player {
    state: Mutex<PlayerState>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player").field("state", &self.state()).finish_non_exhaustive()
    }
}

impl Player {
    /// Open `src` and start the threads (paused unless `opts.autoplay`). `frames` receives
    /// every presented picture, `audio` (when `Some`) the samples, and `wake` is called
    /// whenever the state or the frame changed, so the UI repaints.
    ///
    /// V0 stub: [`MediaError::NotLoaded`].
    pub fn open(
        rt: Arc<Runtime>,
        src: Source,
        opts: PlayerOptions,
        frames: Box<dyn FrameSink>,
        audio: Option<Arc<dyn AudioOutput>>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Player, MediaError> {
        let _ = (rt, src, opts, frames, audio, wake);
        Err(MediaError::NotLoaded)
    }

    /// Queue a command (never blocks).
    pub fn send(&self, cmd: PlayerCmd) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match cmd {
            PlayerCmd::Play => s.playing = true,
            PlayerCmd::Pause => s.playing = false,
            PlayerCmd::TogglePlay => s.playing = !s.playing,
            PlayerCmd::Seek(at) => s.pos = at.max(0.0),
            PlayerCmd::SeekBy(by) => s.pos = (s.pos + by).max(0.0),
            PlayerCmd::SetVolume(v) => s.volume = v.clamp(0.0, 1.0),
            PlayerCmd::Mute(on) => s.muted = on,
            PlayerCmd::SetOutputSize(_) => {}
            PlayerCmd::Stop => {
                s.playing = false;
                s.ended = true;
            }
        }
        drop(s);
        (self.wake)();
    }

    /// The current snapshot.
    pub fn state(&self) -> PlayerState {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn commands_update_the_snapshot_and_wake() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let w = Arc::clone(&wakes);
        let p = Player {
            state: Mutex::new(PlayerState { duration: Some(10.0), ..Default::default() }),
            wake: Arc::new(move || {
                w.fetch_add(1, Ordering::Relaxed);
            }),
        };
        p.send(PlayerCmd::Play);
        p.send(PlayerCmd::Seek(3.0));
        p.send(PlayerCmd::SeekBy(-5.0));
        p.send(PlayerCmd::SetVolume(1.5));
        p.send(PlayerCmd::Mute(true));
        let s = p.state();
        assert!(s.playing && s.muted);
        assert_eq!(s.pos, 0.0, "never before the start");
        assert_eq!(s.volume, 1.0, "clamped");
        p.send(PlayerCmd::Stop);
        assert!(!p.state().playing && p.state().ended);
        assert_eq!(wakes.load(Ordering::Relaxed), 6);
        assert!(format!("{p:?}").contains("ended: true"));
        assert_eq!(PlayerOptions::default().hw, HwPref::Auto);
    }
}
