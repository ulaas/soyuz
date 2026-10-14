//! Decoded audio: a pull reader of interleaved `f32` samples ([`AudioReader`]) for
//! playback, and waveform [`peaks`] for the preview.

use crate::info::{Source, StreamInfo};
use crate::{MediaError, Runtime};

/// Demuxer + decoder + resampler to interleaved `f32` at a fixed rate.
#[derive(Debug)]
pub struct AudioReader {
    info: StreamInfo,
    rate: u32,
    channels: u32,
}

impl AudioReader {
    /// Open the audio stream `stream` (`None` = the default one) of `src`, resampled to
    /// `out_rate` (`None` = the stream's own rate). Channel count and sample format are
    /// taken from the decoder (`ch_layout` AVOption, never an `AVFrame` tail field).
    ///
    /// V0 stub: [`MediaError::NotLoaded`].
    pub fn open(rt: &Runtime, src: &Source, stream: Option<usize>, out_rate: Option<u32>) -> Result<Self, MediaError> {
        let _ = (rt, src, stream, out_rate);
        Err(MediaError::NotLoaded)
    }

    pub fn info(&self) -> &StreamInfo {
        &self.info
    }

    /// Output sample rate (Hz).
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Output channels (samples per frame).
    pub fn channels(&self) -> u32 {
        self.channels
    }

    /// Seek to `secs` (the next [`read`](Self::read) starts at or just before it).
    pub fn seek(&mut self, secs: f64) -> Result<(), MediaError> {
        let _ = secs;
        Err(MediaError::NotLoaded)
    }

    /// Append up to `max_frames` frames (`max_frames * channels` samples) to `out`;
    /// returns the frames appended, `0` at the end of the stream.
    pub fn read(&mut self, out: &mut Vec<f32>, max_frames: usize) -> Result<usize, MediaError> {
        let _ = (out, max_frames);
        Err(MediaError::NotLoaded)
    }
}

/// Waveform peaks: `buckets` pairs of `(min, max)` over the whole stream, all channels
/// mixed. `stale` is polled between packets.
///
/// V0 stub: [`MediaError::NotLoaded`].
pub fn peaks(
    rt: &Runtime,
    src: &Source,
    buckets: usize,
    stale: &dyn Fn() -> bool,
) -> Result<Vec<(f32, f32)>, MediaError> {
    let _ = (rt, src, buckets, stale);
    Err(MediaError::NotLoaded)
}
