//! Media tools as one function ([`run`]) plus its dry run ([`plan`]): convert, extract
//! audio, trim, remux, save a frame. Driven by `soyuz-jobs` (`JobKind::Media`), which
//! adapts `progress` / `check` to its own progress and checkpoint calls. Every operation
//! writes `dest` through a `.part` temporary and renames it at the end, so a cancelled or
//! failed run leaves nothing behind.

use std::path::Path;

use crate::presets::PresetId;
use crate::{MediaError, Runtime};

/// The audio codec of [`MediaOp::ExtractAudio`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioCodecChoice {
    /// Keep the stream as is (the container is chosen to fit: `.m4a`, `.opus`, `.mka`, …).
    Copy,
    Flac,
    Aac,
    Opus,
    /// 16-bit PCM.
    Wav,
}

/// Picture format of [`MediaOp::Frame`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
}

/// What to do with the input.
#[derive(Clone, Debug, PartialEq)]
pub enum MediaOp {
    /// Re-encode (or remux, for copy presets) with a built-in preset; `video_stream` /
    /// `audio_stream` pick streams by index (`None` = the defaults).
    Convert {
        preset: PresetId,
        video_stream: Option<usize>,
        audio_stream: Option<usize>,
    },
    ExtractAudio {
        codec: AudioCodecChoice,
    },
    /// Cut `start..end` seconds; `stream_copy` keeps the packets (cuts land on keyframes),
    /// otherwise the cut is re-encoded frame-exact.
    Trim {
        start: f64,
        end: f64,
        stream_copy: bool,
    },
    /// Change the container only (`"matroska"`, `"mp4"`).
    Remux {
        container: String,
    },
    /// Save one frame at `at` seconds; `max_px` scales the picture down to fit.
    Frame {
        at: f64,
        format: ImageFormat,
        max_px: Option<u32>,
    },
}

/// Progress of a running operation.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Progress {
    /// Bytes of input consumed (`AVPacket.pos`; falls back to `secs / duration` scaled to
    /// `input_total` when the demuxer reports no positions).
    pub input_bytes: u64,
    pub input_total: u64,
    /// Media time processed (seconds).
    pub secs: f64,
    pub out_bytes: u64,
}

/// The `check` closure asked to stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cancelled;

/// What [`run`] produced.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Outcome {
    pub out_bytes: u64,
    /// Seconds of output.
    pub duration: Option<f64>,
    /// Things the user should know (a stream dropped, a cut moved to a keyframe, …).
    pub warnings: Vec<String>,
}

/// The dry run of an operation: what would happen, for the approval card.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PlanInfo {
    /// Output container (muxer name).
    pub container: String,
    /// `(stream index, action)` rows: `"copy"`, `"encode h264_nvenc"`, `"drop"`.
    pub streams: Vec<(usize, String)>,
    /// Rough output size (bytes), when it can be estimated.
    pub estimated_bytes: Option<u64>,
    pub warnings: Vec<String>,
}

/// Run `op` from `src` to `dest`. `progress` is called between packets, `check` too
/// (returning `Err(Cancelled)` stops the work, removes the temporary and yields
/// [`MediaError::Cancelled`]).
///
/// V0 stub: [`MediaError::NotLoaded`].
pub fn run(
    rt: &Runtime,
    src: &Path,
    dest: &Path,
    op: &MediaOp,
    progress: &mut dyn FnMut(Progress),
    check: &mut dyn FnMut() -> Result<(), Cancelled>,
) -> Result<Outcome, MediaError> {
    let _ = (rt, src, dest, op, progress, check);
    Err(MediaError::NotLoaded)
}

/// The dry run of `op` on `src`: streams, codecs, estimated size, warnings.
///
/// V0 stub: [`MediaError::NotLoaded`].
pub fn plan(rt: &Runtime, src: &Path, op: &MediaOp) -> Result<PlanInfo, MediaError> {
    let _ = (rt, src, op);
    Err(MediaError::NotLoaded)
}
