//! The player's clock: the position every thread agrees on. With audio it is the device's
//! `played_frames` (the samples the device has actually pulled, so underruns, which play
//! silence, still advance it); without audio it is the wall clock. Positions are seconds
//! of *player time*: 0 at the start of the file.

use std::sync::{Arc, Mutex};
use std::time::Instant;

/// How many frames the audio device has played so far (the sink's `played_frames`).
pub(crate) type PlayedFrames = Arc<dyn Fn() -> u64 + Send + Sync>;

struct Anchor {
    /// Player time at the anchor.
    pos: f64,
    /// Wall clock: when the anchor was taken (`None` while paused).
    since: Option<Instant>,
    /// Audio clock: `played_frames()` at the anchor.
    played: u64,
}

enum Source {
    Wall,
    Audio { played: PlayedFrames, rate: f64 },
}

pub(crate) struct Clock {
    anchor: Mutex<Anchor>,
    source: Source,
}

impl Clock {
    /// A wall clock starting at 0, paused.
    pub(crate) fn wall() -> Clock {
        Clock { anchor: Mutex::new(Anchor { pos: 0.0, since: None, played: 0 }), source: Source::Wall }
    }

    /// An audio clock: `played()` frames at `rate` Hz since the last [`set`](Self::set).
    /// Pausing is the device's business (a paused device stops counting).
    pub(crate) fn audio(rate: u32, played: PlayedFrames) -> Clock {
        let at = played();
        Clock {
            anchor: Mutex::new(Anchor { pos: 0.0, since: None, played: at }),
            source: Source::Audio { played, rate: f64::from(rate.max(1)) },
        }
    }

    pub(crate) fn is_audio(&self) -> bool {
        matches!(self.source, Source::Audio { .. })
    }

    /// The current position (seconds, never negative).
    pub(crate) fn now(&self) -> f64 {
        let a = self.anchor.lock().unwrap_or_else(|e| e.into_inner());
        let pos = match &self.source {
            Source::Wall => a.pos + a.since.map_or(0.0, |s| s.elapsed().as_secs_f64()),
            Source::Audio { played, rate } => a.pos + played().saturating_sub(a.played) as f64 / rate,
        };
        pos.max(0.0)
    }

    /// Re-anchor at `pos` (after a seek); a running wall clock keeps running.
    pub(crate) fn set(&self, pos: f64) {
        let mut a = self.anchor.lock().unwrap_or_else(|e| e.into_inner());
        a.pos = pos.max(0.0);
        if a.since.is_some() {
            a.since = Some(Instant::now());
        }
        if let Source::Audio { played, .. } = &self.source {
            a.played = played();
        }
    }

    /// Freeze a wall clock (no-op for the audio clock: pause the device instead).
    pub(crate) fn pause(&self) {
        let mut a = self.anchor.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = a.since.take() {
            a.pos += s.elapsed().as_secs_f64();
        }
    }

    /// Let a wall clock run.
    pub(crate) fn resume(&self) {
        if matches!(self.source, Source::Wall) {
            let mut a = self.anchor.lock().unwrap_or_else(|e| e.into_inner());
            if a.since.is_none() {
                a.since = Some(Instant::now());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    #[test]
    fn wall_clock_runs_pauses_and_seeks() {
        let c = Clock::wall();
        assert!(!c.is_audio());
        assert_eq!(c.now(), 0.0, "paused at 0 until resumed");
        std::thread::sleep(Duration::from_millis(15));
        assert_eq!(c.now(), 0.0);
        c.resume();
        std::thread::sleep(Duration::from_millis(30));
        let t = c.now();
        assert!((0.03..1.0).contains(&t), "{t}");
        c.pause();
        let frozen = c.now();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(c.now(), frozen, "paused");
        c.set(5.0);
        assert_eq!(c.now(), 5.0, "a seek while paused lands exactly");
        c.resume();
        c.resume();
        std::thread::sleep(Duration::from_millis(10));
        assert!(c.now() > 5.0);
        c.set(-3.0);
        assert!(c.now() >= 0.0 && c.now() < 1.0, "clamped at 0, still running");
    }

    #[test]
    fn audio_clock_follows_played_frames() {
        let played = Arc::new(AtomicU64::new(1000));
        let p = Arc::clone(&played);
        let c = Clock::audio(100, Arc::new(move || p.load(Ordering::Relaxed)));
        assert!(c.is_audio());
        assert_eq!(c.now(), 0.0, "anchored at the frames already played");
        played.store(1050, Ordering::Relaxed);
        assert!((c.now() - 0.5).abs() < 1e-9);
        // Pause / resume do nothing: the device stops counting by itself.
        c.pause();
        c.resume();
        assert!((c.now() - 0.5).abs() < 1e-9);
        c.set(10.0);
        assert_eq!(c.now(), 10.0);
        played.store(1150, Ordering::Relaxed);
        assert!((c.now() - 11.0).abs() < 1e-9);
        // A device whose counter went backwards (reset) never yields a negative delta.
        played.store(0, Ordering::Relaxed);
        assert_eq!(c.now(), 10.0);
    }
}
