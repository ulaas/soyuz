//! The media player's audio output (Phase V2b): [`RodioOutput`] implements
//! [`soyuz_media::AudioOutput`] with rodio (cpal → WASAPI / CoreAudio / ALSA, no shell
//! APIs), and the ring between the decoder and the device ([`ring`]) also carries the
//! FFmpeg-decoded tracks of the audio preview (`preview_audio`).
//!
//! **The clock.** The player's master clock is [`AudioSink::played_frames`]: it is counted
//! inside [`RingSource::next`], the pull the device callback makes, never by the decoder.
//! Nothing on that path blocks: samples travel as whole-frame chunks through a bounded
//! channel (`try_send` / `try_recv` only), pause and volume are atomics. When the ring runs
//! dry, the source plays silence and the clock keeps counting, so lip sync does not drift.
//! The exception is the pre-roll after an open or a [`flush`](RingSink::flush): until
//! [`soyuz_media::PREROLL`] of audio is queued (or the producer [finished](RingSink::finish))
//! the source holds the data back and plays silence that does not count, so a slow decoder
//! start neither stutters nor puts the video ahead.
//!
//! **The device** is owned by one thread (`soyuz-media-out`) that opens it on the first
//! stream and releases it after [`DEVICE_IDLE`] without streams. When no device can be
//! opened the stream is pulled by a timer instead ([`pull_silently`]), so playback keeps
//! going (silently) and the clock still advances.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded, unbounded};
use parking_lot::Mutex;
use soyuz_media::{AudioOutput, AudioSink, preroll_frames};

/// Seconds of audio the ring holds ahead of the device.
pub const RING_SECS: f64 = 0.5;
/// Release the device after this long without a stream.
const DEVICE_IDLE: Duration = Duration::from_secs(20);
/// Chunks the channel can hold (the frame cap of [`RING_SECS`] is what limits the ring;
/// this only bounds the channel's memory).
const CHUNKS: usize = 256;

/// One batch of whole frames, stamped with the ring generation it was pushed in.
struct Chunk {
    generation: u32,
    samples: Vec<f32>,
}

/// State both ends of a ring share.
struct Shared {
    rate: u32,
    channels: u32,
    /// Frames handed to the device (data and counted silence).
    played: AtomicU64,
    /// Frames in the channel plus the rest of the consumer's current chunk.
    queued: AtomicU64,
    /// Bumped by `flush`: older chunks are dropped wherever they are.
    generation: AtomicU32,
    /// The pre-roll is queued since the last flush (or the producer finished): the
    /// source plays the data, and silence now counts as played.
    started: AtomicBool,
    /// Frames to queue before `started`.
    preroll: u64,
    paused: AtomicBool,
    /// `f32` bits.
    volume: AtomicU32,
    /// The producer is done: the source ends once the ring is drained.
    finished: AtomicBool,
    /// The sink was dropped before finishing, or the source was dropped: the stream ends.
    closed: AtomicBool,
    /// Silence frames counted (underruns), for diagnostics.
    underruns: AtomicU64,
    /// Position base set by [`RingSink::rebase`]: `(seconds as f64 bits, played then)`.
    epoch_secs: AtomicU64,
    epoch_played: AtomicU64,
}

impl Shared {
    fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed))
    }
}

/// The producer end: what the decoder pushes into (an [`AudioSink`]).
pub struct RingSink {
    shared: Arc<Shared>,
    tx: Sender<Chunk>,
    /// A second receiver so `flush` can drain from the producer side.
    rx: Receiver<Chunk>,
    cap_frames: u64,
}

/// The consumer end: a [`rodio::Source`] the device pulls from.
pub struct RingSource {
    shared: Arc<Shared>,
    rx: Receiver<Chunk>,
    cur: Option<Chunk>,
    pos: usize,
    /// Sample index inside the current frame (`0..channels`); chunks hold whole frames,
    /// so a chunk boundary is always a frame boundary.
    phase: u32,
    /// The frame being emitted counts as played (data, or silence after the start).
    counting: bool,
}

/// A read-only view of a ring's clock (for the preview player's position).
#[derive(Clone)]
pub struct RingClock {
    shared: Arc<Shared>,
}

impl RingClock {
    /// Frames handed to the device so far.
    pub fn played_frames(&self) -> u64 {
        self.shared.played.load(Ordering::Relaxed)
    }

    /// Position in seconds: the last [`RingSink::rebase`] plus what played since.
    pub fn secs(&self) -> f64 {
        let base = f64::from_bits(self.shared.epoch_secs.load(Ordering::Relaxed));
        let since = self.played_frames().saturating_sub(self.shared.epoch_played.load(Ordering::Relaxed));
        base + since as f64 / f64::from(self.shared.rate.max(1))
    }

    /// Seconds of audio queued ahead of the device.
    pub fn buffered_secs(&self) -> f64 {
        self.shared.queued.load(Ordering::Relaxed) as f64 / f64::from(self.shared.rate.max(1))
    }

    /// Silence frames played because the ring was empty.
    pub fn underrun_frames(&self) -> u64 {
        self.shared.underruns.load(Ordering::Relaxed)
    }

    /// The stream ended (finished and drained, or closed).
    pub fn ended(&self) -> bool {
        self.shared.closed.load(Ordering::Relaxed)
            || (self.shared.finished.load(Ordering::Relaxed) && self.shared.queued.load(Ordering::Relaxed) == 0)
    }
}

/// A ring of `secs` seconds of interleaved `f32` frames at `rate` Hz with `channels`,
/// with the player's pre-roll ([`preroll_frames`], at most the whole ring).
pub fn ring(rate: u32, channels: u32, secs: f64) -> (RingSink, RingSource) {
    ring_with_preroll(rate, channels, secs, preroll_frames(rate))
}

/// [`ring`] that starts once `preroll` frames are queued (0: with the first push).
pub fn ring_with_preroll(rate: u32, channels: u32, secs: f64, preroll: u64) -> (RingSink, RingSource) {
    let rate = rate.max(1);
    let channels = channels.clamp(1, u32::from(u16::MAX));
    let cap_frames = ((secs.max(0.01) * f64::from(rate)).ceil() as u64).max(1);
    let shared = Arc::new(Shared {
        rate,
        channels,
        played: AtomicU64::new(0),
        queued: AtomicU64::new(0),
        generation: AtomicU32::new(0),
        started: AtomicBool::new(false),
        preroll: preroll.min(cap_frames),
        paused: AtomicBool::new(false),
        volume: AtomicU32::new(1.0f32.to_bits()),
        finished: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        underruns: AtomicU64::new(0),
        epoch_secs: AtomicU64::new(0.0f64.to_bits()),
        epoch_played: AtomicU64::new(0),
    });
    let (tx, rx) = bounded(CHUNKS);
    let sink = RingSink { shared: Arc::clone(&shared), tx, rx: rx.clone(), cap_frames };
    let source = RingSource { shared, rx, cur: None, pos: 0, phase: 0, counting: false };
    (sink, source)
}

impl RingSink {
    /// The clock of this ring.
    pub fn clock(&self) -> RingClock {
        RingClock { shared: Arc::clone(&self.shared) }
    }

    pub fn rate(&self) -> u32 {
        self.shared.rate
    }

    pub fn channels(&self) -> u32 {
        self.shared.channels
    }

    /// Frames the ring can take right now.
    pub fn free_frames(&self) -> u64 {
        self.cap_frames.saturating_sub(self.shared.queued.load(Ordering::Relaxed))
    }

    /// The consumer is gone or the stream was closed: pushing is pointless.
    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::Relaxed)
    }

    /// No more frames will come; the source ends once the ring is drained (a pre-roll
    /// ends here: the tail plays even when it is shorter).
    pub fn finish(&mut self) {
        self.shared.finished.store(true, Ordering::Release);
        self.shared.started.store(true, Ordering::Release);
    }

    /// Drop queued frames and restart the position at `secs` (a seek).
    pub fn rebase(&mut self, secs: f64) {
        self.flush();
        self.shared.epoch_secs.store(secs.max(0.0).to_bits(), Ordering::Relaxed);
        self.shared.epoch_played.store(self.shared.played.load(Ordering::Relaxed), Ordering::Relaxed);
    }
}

impl AudioSink for RingSink {
    fn push(&mut self, interleaved: &[f32]) -> usize {
        let ch = self.shared.channels as usize;
        let frames = (interleaved.len() / ch) as u64;
        let take = frames.min(self.free_frames());
        if take == 0 {
            return if self.is_closed() { frames as usize } else { 0 };
        }
        let take = take as usize;
        let chunk = Chunk {
            generation: self.shared.generation.load(Ordering::Relaxed),
            samples: interleaved[..take * ch].to_vec(),
        };
        match self.tx.try_send(chunk) {
            Ok(()) => {
                let queued = self.shared.queued.fetch_add(take as u64, Ordering::Relaxed) + take as u64;
                if queued >= self.shared.preroll {
                    self.shared.started.store(true, Ordering::Release);
                }
                take
            }
            // The channel is full of unconsumed chunks (the device stalled): offer again
            // later. Disconnected: the source is gone, swallow so the producer finishes.
            Err(crossbeam_channel::TrySendError::Full(_)) => 0,
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                self.shared.closed.store(true, Ordering::Relaxed);
                frames as usize
            }
        }
    }

    fn played_frames(&self) -> u64 {
        self.shared.played.load(Ordering::Relaxed)
    }

    fn queued_frames(&self) -> u64 {
        self.shared.queued.load(Ordering::Relaxed)
    }

    fn pause(&mut self, on: bool) {
        self.shared.paused.store(on, Ordering::Relaxed);
    }

    fn set_volume(&mut self, v: f32) {
        let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 };
        self.shared.volume.store(v.to_bits(), Ordering::Relaxed);
    }

    fn flush(&mut self) {
        self.shared.generation.fetch_add(1, Ordering::Relaxed);
        self.shared.started.store(false, Ordering::Relaxed);
        // Drain what the device has not pulled yet; the consumer drops the rest of its
        // current chunk itself when it sees the new generation.
        let ch = self.shared.channels as usize;
        while let Ok(c) = self.rx.try_recv() {
            self.shared.queued.fetch_sub((c.samples.len() / ch) as u64, Ordering::Relaxed);
        }
    }
}

impl Drop for RingSink {
    fn drop(&mut self) {
        // Dropped without `finish`: the player closed, stop the stream now. After
        // `finish` the source still drains the tail.
        if !self.shared.finished.load(Ordering::Relaxed) {
            self.shared.closed.store(true, Ordering::Relaxed);
        }
    }
}

impl RingSource {
    /// The clock of this ring.
    pub fn clock(&self) -> RingClock {
        RingClock { shared: Arc::clone(&self.shared) }
    }

    /// Drop the current chunk if it belongs to an older generation (only at a frame
    /// boundary, so channels stay aligned).
    fn drop_stale(&mut self) {
        if self.phase != 0 {
            return;
        }
        let generation = self.shared.generation.load(Ordering::Relaxed);
        if let Some(c) = &self.cur
            && c.generation != generation
        {
            let ch = self.shared.channels as usize;
            let left = (c.samples.len() - self.pos) / ch;
            self.shared.queued.fetch_sub(left as u64, Ordering::Relaxed);
            self.cur = None;
            self.pos = 0;
        }
    }

    /// Pull the next chunk of the current generation; `false` when the channel is empty.
    fn refill(&mut self) -> Result<bool, ()> {
        let generation = self.shared.generation.load(Ordering::Relaxed);
        let ch = self.shared.channels as usize;
        loop {
            match self.rx.try_recv() {
                Ok(c) if c.generation == generation && !c.samples.is_empty() => {
                    self.cur = Some(c);
                    self.pos = 0;
                    return Ok(true);
                }
                Ok(c) => {
                    self.shared.queued.fetch_sub((c.samples.len() / ch) as u64, Ordering::Relaxed);
                }
                Err(TryRecvError::Empty) => return Ok(false),
                Err(TryRecvError::Disconnected) => return Err(()),
            }
        }
    }

    /// Advance the frame phase after one emitted sample; counts the frame when complete.
    fn step(&mut self, from_data: bool) {
        self.phase += 1;
        if self.phase >= self.shared.channels {
            self.phase = 0;
            if self.counting {
                self.shared.played.fetch_add(1, Ordering::Relaxed);
            }
            if from_data {
                self.shared.queued.fetch_sub(1, Ordering::Relaxed);
            } else if self.counting {
                self.shared.underruns.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl Iterator for RingSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        // Decisions only at frame boundaries: pause, end and generation changes.
        if self.phase == 0 {
            if self.shared.closed.load(Ordering::Relaxed) {
                return None;
            }
            if self.shared.paused.load(Ordering::Relaxed) {
                return Some(0.0);
            }
            self.drop_stale();
            if !self.shared.started.load(Ordering::Acquire) {
                // Pre-roll: leave the data queued until there is enough; this frame is
                // silence that does not count.
                self.counting = false;
            } else if self.cur.is_none() {
                match self.refill() {
                    Ok(true) => {}
                    Ok(false) => {
                        if self.shared.finished.load(Ordering::Relaxed) {
                            self.shared.closed.store(true, Ordering::Relaxed);
                            return None;
                        }
                    }
                    Err(()) => return None,
                }
                self.counting = true;
            } else {
                self.counting = true;
            }
        }
        match self.cur.as_ref() {
            Some(c) => {
                let s = c.samples[self.pos] * self.shared.volume();
                self.pos += 1;
                if self.pos >= c.samples.len() {
                    self.cur = None;
                    self.pos = 0;
                }
                self.step(true);
                Some(s)
            }
            None => {
                // Underrun (or pre-roll): silence, counted only once the pre-roll is done.

                self.step(false);
                Some(0.0)
            }
        }
    }
}

impl rodio::Source for RingSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        rodio::ChannelCount::new(u16::try_from(self.shared.channels).unwrap_or(u16::MAX))
            .unwrap_or(rodio::ChannelCount::MIN)
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        rodio::SampleRate::new(self.shared.rate).unwrap_or(rodio::SampleRate::MIN)
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

impl Drop for RingSource {
    fn drop(&mut self) {
        self.shared.closed.store(true, Ordering::Relaxed);
    }
}

/// Pull `src` at its nominal rate from a timer until it ends: the stand-in for a device
/// when none can be opened (silent playback whose clock still runs), and the test
/// harness. Blocks the calling thread.
pub fn pull_silently(mut src: RingSource) {
    let per_tick = (src.shared.rate as usize / 100).max(1) * src.shared.channels as usize;
    let tick = Duration::from_millis(10);
    let mut next_at = Instant::now();
    loop {
        for _ in 0..per_tick {
            if src.next().is_none() {
                return;
            }
        }
        next_at += tick;
        let now = Instant::now();
        if next_at > now {
            std::thread::sleep(next_at - now);
        } else if now - next_at > Duration::from_secs(1) {
            // Far behind (the machine slept): do not try to catch up a whole second.
            next_at = now;
        }
    }
}

// ---- the device thread -----------------------------------------------------------------

enum DevCmd {
    Add(RingSource),
}

struct Out {
    tx: Sender<DevCmd>,
    error: Mutex<Option<String>>,
}

fn out() -> &'static Out {
    static OUT: OnceLock<Out> = OnceLock::new();
    OUT.get_or_init(|| {
        let (tx, rx) = unbounded();
        let _ = std::thread::Builder::new().name("soyuz-media-out".into()).spawn(move || device_thread(rx));
        Out { tx, error: Mutex::new(None) }
    })
}

/// Why the last stream could not reach a device (`None` when it did); the player shows
/// it as a hint, playback itself continues silently.
pub fn device_error() -> Option<String> {
    out().error.lock().clone()
}

fn device_thread(rx: Receiver<DevCmd>) {
    let mut device: Option<rodio::MixerDeviceSink> = None;
    let mut live: Vec<Weak<Shared>> = Vec::new();
    let mut idle_since = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(DevCmd::Add(src)) => {
                if device.is_none() {
                    // A device bug must not kill the thread: a failed open means silent
                    // playback for this stream.
                    let opened = std::panic::catch_unwind(rodio::DeviceSinkBuilder::open_default_sink);
                    match opened {
                        Ok(Ok(mut d)) => {
                            d.log_on_drop(false);
                            device = Some(d);
                            *out().error.lock() = None;
                        }
                        Ok(Err(e)) => {
                            *out().error.lock() = Some(tr!("services-audio-no-device", error = e.to_string()))
                        }
                        Err(_) => *out().error.lock() = Some(tr!("services-audio-player-failed").into()),
                    }
                }
                live.push(Arc::downgrade(&src.shared));
                match &device {
                    Some(d) => d.mixer().add(src),
                    None => {
                        let _ = std::thread::Builder::new().name("soyuz-media-out-silent".into()).spawn(move || {
                            pull_silently(src);
                        });
                    }
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
        }
        live.retain(|w| w.upgrade().is_some_and(|s| !s.closed.load(Ordering::Relaxed)));
        if !live.is_empty() {
            idle_since = Instant::now();
        } else if device.is_some() && idle_since.elapsed() > DEVICE_IDLE {
            device = None;
        }
    }
}

/// The frontend-independent audio output of [`soyuz_media::Player`]: every stream goes
/// through a [`ring`] into the mixer of the shared device.
#[derive(Clone, Copy, Debug, Default)]
pub struct RodioOutput;

impl AudioOutput for RodioOutput {
    fn open(&self, rate: u32, channels: u32) -> Box<dyn AudioSink> {
        let (sink, source) = ring(rate, channels, RING_SECS);
        let _ = out().tx.send(DevCmd::Add(source));
        Box::new(sink)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pull `n` frames from `src` (the device callback's work).
    fn pull(src: &mut RingSource, n: usize) -> Vec<f32> {
        let ch = src.shared.channels as usize;
        (0..n * ch).map_while(|_| src.next()).collect()
    }

    fn frames(n: usize, ch: usize, value: f32) -> Vec<f32> {
        vec![value; n * ch]
    }

    #[test]
    fn clock_counts_data_and_underruns_but_not_the_preroll() {
        let (mut sink, mut src) = ring_with_preroll(1000, 2, 0.1, 0);
        assert_eq!(sink.free_frames(), 100);
        // Pre-roll: silence, clock still at 0.
        assert_eq!(pull(&mut src, 10), vec![0.0; 20]);
        assert_eq!(sink.played_frames(), 0);
        assert_eq!(sink.push(&frames(30, 2, 0.5)), 30);
        assert_eq!(sink.queued_frames(), 30);
        assert_eq!(sink.clock().buffered_secs(), 0.03);
        let out = pull(&mut src, 30);
        assert!(out.iter().all(|s| *s == 0.5));
        assert_eq!(sink.played_frames(), 30);
        assert_eq!(sink.clock().buffered_secs(), 0.0);
        // Underrun: silence that counts.
        assert_eq!(pull(&mut src, 5), vec![0.0; 10]);
        assert_eq!(sink.played_frames(), 35);
        assert_eq!(sink.clock().underrun_frames(), 5);
        assert!((sink.clock().secs() - 0.035).abs() < 1e-9);
    }

    #[test]
    fn preroll_holds_the_data_back_until_enough_is_queued() {
        // 8 kHz: the pre-roll is 1200 frames (150 ms).
        let (mut sink, mut src) = ring(8000, 1, 0.5);
        assert_eq!(soyuz_media::preroll_frames(8000), 1200);
        assert_eq!(sink.push(&frames(1000, 1, 0.5)), 1000);
        // Not enough yet: silence that does not count, and the data stays queued.
        assert_eq!(pull(&mut src, 50), vec![0.0; 50]);
        assert_eq!((sink.played_frames(), sink.queued_frames()), (0, 1000));
        assert_eq!(sink.clock().underrun_frames(), 0);
        sink.push(&frames(200, 1, 0.5));
        assert_eq!(pull(&mut src, 1200), vec![0.5; 1200], "the whole pre-roll plays, from the start");
        assert_eq!(sink.played_frames(), 1200);
        // An underrun after the start counts.
        pull(&mut src, 10);
        assert_eq!(sink.played_frames(), 1210);
        // A flush (seek) starts a new pre-roll …
        sink.rebase(3.0);
        sink.push(&frames(600, 1, 0.25));
        assert_eq!(pull(&mut src, 20), vec![0.0; 20]);
        assert_eq!(sink.played_frames(), 1210);
        // … which a finished producer ends early: a short tail still plays.
        sink.finish();
        assert_eq!(pull(&mut src, 600), vec![0.25; 600]);
        assert_eq!(src.next(), None);
        assert!((sink.clock().secs() - 3.075).abs() < 1e-9, "{}", sink.clock().secs());
        // A ring shorter than the pre-roll starts when it is full.
        let (mut sink, mut src) = ring(8000, 1, 0.1);
        assert_eq!(sink.push(&frames(2000, 1, 1.0)), 800);
        assert_eq!(pull(&mut src, 1), vec![1.0]);
    }

    #[test]
    fn push_accepts_what_fits_and_the_rest_later() {
        let (mut sink, mut src) = ring(1000, 1, 0.1);
        assert_eq!(sink.push(&frames(150, 1, 1.0)), 100, "the cap");
        assert_eq!(sink.push(&frames(10, 1, 1.0)), 0, "full");
        pull(&mut src, 40);
        assert_eq!(sink.push(&frames(50, 1, 1.0)), 40);
        assert_eq!(sink.clock().buffered_secs(), 0.1);
    }

    #[test]
    fn pause_and_volume_apply_on_the_pull_side() {
        let (mut sink, mut src) = ring_with_preroll(1000, 2, 0.1, 0);
        sink.push(&frames(20, 2, 1.0));
        sink.set_volume(0.25);
        assert_eq!(pull(&mut src, 1), vec![0.25, 0.25]);
        sink.pause(true);
        assert_eq!(pull(&mut src, 5), vec![0.0; 10]);
        assert_eq!(sink.played_frames(), 1, "paused: no clock");
        sink.pause(false);
        assert_eq!(pull(&mut src, 1), vec![0.25, 0.25]);
        assert_eq!(sink.played_frames(), 2);
        sink.set_volume(f32::NAN);
        assert_eq!(pull(&mut src, 1), vec![1.0, 1.0], "NaN volume is ignored");
    }

    #[test]
    fn flush_drops_queued_frames_and_rebase_moves_the_position() {
        let (mut sink, mut src) = ring_with_preroll(1000, 2, 0.5, 0);
        sink.push(&frames(100, 2, 1.0));
        // The consumer holds a chunk and is mid-frame when the flush lands.
        let got = (0..3).map_while(|_| src.next()).collect::<Vec<_>>();
        assert_eq!(got, vec![1.0; 3]);
        sink.push(&frames(50, 2, 1.0));
        sink.rebase(7.0);
        // The rest of the open frame still comes from the old chunk (alignment), then
        // the stale chunk is dropped and silence follows that does not count (pre-roll
        // again).
        assert_eq!(src.next(), Some(1.0));
        assert_eq!(pull(&mut src, 10), vec![0.0; 20]);
        assert_eq!(sink.clock().buffered_secs(), 0.0);
        assert_eq!(sink.played_frames(), 2);
        // The frame open during the rebase may land on either side of the epoch.
        assert!((sink.clock().secs() - 7.0).abs() < 0.0015, "{}", sink.clock().secs());
        sink.push(&frames(10, 2, 0.5));
        pull(&mut src, 10);
        assert!((sink.clock().secs() - 7.01).abs() < 0.0015, "{}", sink.clock().secs());
    }

    #[test]
    fn finish_ends_the_source_after_the_tail_and_drop_closes_it() {
        let (mut sink, mut src) = ring(1000, 1, 0.1);
        sink.push(&frames(3, 1, 1.0));
        sink.finish();
        assert_eq!(pull(&mut src, 3), vec![1.0; 3]);
        assert!(!sink.clock().ended() || sink.clock().buffered_secs() == 0.0);
        assert_eq!(src.next(), None);
        assert!(sink.clock().ended());
        drop(sink);

        let (mut sink, mut src) = ring(1000, 1, 0.1);
        sink.push(&frames(3, 1, 1.0));
        let clock = sink.clock();
        drop(sink);
        assert_eq!(src.next(), None, "dropped without finish: closed");
        assert!(clock.ended());

        let (mut sink, src) = ring(1000, 1, 0.1);
        drop(src);
        assert_eq!(sink.push(&frames(3, 1, 1.0)), 3, "no consumer: swallowed");
        assert!(sink.is_closed());
    }

    #[test]
    fn silent_pull_runs_the_clock_at_the_rate() {
        let (mut sink, src) = ring_with_preroll(8000, 2, 0.5, 0);

        let clock = sink.clock();
        sink.push(&frames(400, 2, 0.1));
        let t = std::thread::spawn(move || pull_silently(src));
        let start = Instant::now();
        while clock.played_frames() < 400 && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let took = start.elapsed();
        assert!(clock.played_frames() >= 400);
        assert!(took >= Duration::from_millis(30), "400 frames at 8 kHz take ~50 ms, took {took:?}");
        sink.finish();
        t.join().unwrap();
        assert!(clock.ended());
    }

    #[test]
    fn rodio_output_streams_play_with_or_without_a_device() {
        let out = RodioOutput;
        let mut sink = out.open(8000, 1);
        sink.set_volume(0.0);
        let start = Instant::now();
        let mut pushed = 0usize;
        while start.elapsed() < Duration::from_secs(5) {
            if pushed < 1600 {
                pushed += sink.push(&frames(400, 1, 0.0));
            }
            if sink.played_frames() >= 800 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            sink.played_frames() >= 800,
            "the clock runs ({}), device error: {:?}",
            sink.played_frames(),
            device_error()
        );
        sink.flush();
        sink.pause(true);
    }
}
