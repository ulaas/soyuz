//! Media tools as one function ([`run`]) plus its dry run ([`plan`]): convert, extract
//! audio, trim, remux, save a frame. Driven by `soyuz-jobs` (`JobKind::Media`), which
//! adapts `progress` / `check` to its own progress and checkpoint calls. Every operation
//! writes `dest` through a `.part` temporary and renames it at the end, so a cancelled or
//! failed run leaves nothing behind.
//!
//! How an operation becomes work: [`resolve`] turns the probed input, the destination's
//! extension and the [`MediaOp`] into a [`Resolved`] plan (container, one
//! copy / encode / drop decision per input stream, an optional time window) without
//! touching the libraries, so [`plan`] and the unit tests share it with [`run`]. The
//! engine then opens the input through the crate's own AVIO, sets the output up
//! (`mux::Muxer` over the `.part` file, one `mux::Encoder` per encoded stream, set up
//! through `AVCodecParameters` + AVOptions) and pumps packets: copied streams are
//! re-timestamped with `av_packet_rescale_ts`, encoded ones go decoder → swscale /
//! swresample → encoder. Progress is the input position (`AVPacket.pos`) over the file
//! size, or the pts over the duration when the demuxer reports no positions; `check`
//! runs between packets.

use std::ffi::c_int;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::decode::{ChLayout, Decoder, Demuxer, Frame, Limits, Packet, Recv, StreamRef};
use crate::frame::{FrameRequest, grab_frame};
use crate::info::{Chapter, MediaInfo, Source, StreamInfo, StreamKind, stream_info};
use crate::mux::{
    self, AudioConv, EncSpec, Encoder, Got, Muxer, OutStream, OwnedPar, PixConv, audio_copy_container,
    container_accepts, container_for_trim, ext_for_container, output_format,
};
use crate::presets::{self, PresetId};
use crate::sys::layout::AVRational;
use crate::sys::{Av, codec, encode, types};
use crate::{MediaError, Runtime};

/// The audio codec of [`MediaOp::ExtractAudio`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCodecChoice {
    /// Keep the stream as is; the container is chosen to fit (`.m4a`, `.opus`, `.mka`,
    /// …: [`plan`] reports the extension in [`PlanInfo::ext`]).
    Copy,
    Flac,
    Aac,
    Opus,
    /// 16-bit PCM.
    Wav,
}

impl AudioCodecChoice {
    /// The preset whose audio encoder and container this choice uses (`None` = copy).
    fn preset(self) -> Option<PresetId> {
        match self {
            AudioCodecChoice::Copy => None,
            AudioCodecChoice::Flac => Some(PresetId::AudioFlac),
            AudioCodecChoice::Aac => Some(PresetId::AudioAac),
            AudioCodecChoice::Opus => Some(PresetId::AudioOpus),
            AudioCodecChoice::Wav => Some(PresetId::AudioWav),
        }
    }
}

/// Picture format of [`MediaOp::Frame`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl ImageFormat {
    pub fn ext(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
        }
    }
}

/// What to do with the input. Serialized inside `soyuz-jobs`' `JobKind::Media` (job
/// history, crash journal): the names are stable once shipped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// otherwise the cut is re-encoded frame-exact (with the input's own codecs when the
    /// build has encoders for them, else the container's usual ones).
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
    /// `input_total` when the demuxer reports no positions, and always for a trim, whose
    /// fraction is the window done).
    pub input_bytes: u64,
    pub input_total: u64,
    /// Media time processed (seconds from the start of the output).
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
    /// Output container (muxer name; `"png"` / `"jpeg"` for a frame).
    pub container: String,
    /// The extension the output should have (without the dot).
    pub ext: String,
    /// `(stream index, action)` rows: `"copy"`, `"encode h264_nvenc"`, `"drop: …"`.
    pub streams: Vec<(usize, String)>,
    /// Rough output size (bytes), when it can be estimated.
    pub estimated_bytes: Option<u64>,
    /// Seconds of output, when known.
    pub duration: Option<f64>,
    pub warnings: Vec<String>,
}

/// What happens to one input stream.
#[derive(Clone, Debug, PartialEq)]
enum Action {
    Copy,
    Encode { encoder: String, options: Vec<(String, String)> },
    Drop(String),
}

impl Action {
    fn describe(&self) -> String {
        match self {
            Action::Copy => "copy".into(),
            Action::Encode { encoder, .. } => format!("encode {encoder}"),
            Action::Drop(why) => format!("drop: {why}"),
        }
    }
}

/// An operation turned into decisions (see the module doc).
#[derive(Clone, Debug, PartialEq)]
struct Resolved {
    container: String,
    ext: String,
    /// One row per input stream, in index order.
    actions: Vec<(usize, Action)>,
    /// `(start, end)` seconds of a trim (file time, before the input's start offset).
    window: Option<(f64, f64)>,
    warnings: Vec<String>,
}

impl Resolved {
    fn kept(&self) -> impl Iterator<Item = (usize, &Action)> + '_ {
        self.actions.iter().filter(|(_, a)| !matches!(a, Action::Drop(_))).map(|(i, a)| (*i, a))
    }
}

fn stream_label(s: &StreamInfo) -> String {
    match s.kind {
        Some(k) => format!("stream {} ({k} {})", s.index, s.codec),
        None => format!("stream {}", s.index),
    }
}

/// Every stream the container accepts is copied; the rest is dropped with a warning.
fn remux_all(info: &MediaInfo, container: &str, ext: &str, window: Option<(f64, f64)>) -> Result<Resolved, MediaError> {
    let mut r =
        Resolved { container: container.into(), ext: ext.into(), actions: Vec::new(), window, warnings: Vec::new() };
    for s in &info.streams {
        let action = match s.kind {
            Some(StreamKind::Video | StreamKind::Audio | StreamKind::Subtitle) => {
                if container_accepts(container, &s.codec) {
                    Action::Copy
                } else {
                    r.warnings.push(format!("{} dropped: {container} cannot hold {}", stream_label(s), s.codec));
                    Action::Drop(format!("{container} cannot hold {}", s.codec))
                }
            }
            Some(StreamKind::Attachment) if container == "matroska" => Action::Copy,
            _ => Action::Drop("not copied".into()),
        };
        r.actions.push((s.index, action));
    }
    if r.kept().next().is_none() {
        return Err(MediaError::Unsupported(format!("none of the streams fits a {container} file")));
    }
    Ok(r)
}

/// `"no H.264 hardware encoder on this machine"` style text for an encoder list none of
/// which is available.
fn no_encoder(encoders: &[&str]) -> String {
    let family = |prefix: &str, what: &str| {
        encoders.iter().all(|e| e.starts_with(prefix)).then(|| format!("no {what} hardware encoder on this machine"))
    };
    family("h264_", "H.264")
        .or_else(|| family("hevc_", "HEVC"))
        .unwrap_or_else(|| format!("none of the encoders {} is available in this FFmpeg build", encoders.join(", ")))
}

/// Pick the stream `wanted` (which must be of `kind`), or the default one.
fn choose_stream(info: &MediaInfo, kind: StreamKind, wanted: Option<usize>) -> Result<Option<&StreamInfo>, MediaError> {
    match wanted {
        Some(i) => match info.streams.iter().find(|s| s.index == i) {
            Some(s) if s.kind == Some(kind) && !s.attached_pic => Ok(Some(s)),
            _ => Err(MediaError::NoStream(kind)),
        },
        None => Ok(match kind {
            StreamKind::Video => info.video(),
            StreamKind::Audio => info.audio(),
            _ => None,
        }),
    }
}

/// A stream with the encoders (in preference order) and options it goes through.
type Choice<'a> = (&'a StreamInfo, &'a [&'a str], &'a [(&'a str, &'a str)]);

/// One video and one audio stream through the preset's encoders (`"copy"` keeps the
/// packets when the container accepts them); everything else is dropped.
fn encode_one_each(
    info: &MediaInfo,
    container: &str,
    ext: &str,
    video: Option<Choice<'_>>,
    audio: Option<Choice<'_>>,
    window: Option<(f64, f64)>,
    has_encoder: &dyn Fn(&str) -> bool,
) -> Result<Resolved, MediaError> {
    let mut r =
        Resolved { container: container.into(), ext: ext.into(), actions: Vec::new(), window, warnings: Vec::new() };
    let mut decide = |s: &StreamInfo, encoders: &[&str], options: &[(&str, &str)]| -> Result<Action, MediaError> {
        if encoders == ["copy"] {
            if container_accepts(container, &s.codec) {
                return Ok(Action::Copy);
            }
            r.warnings.push(format!("{} dropped: {container} cannot hold {}", stream_label(s), s.codec));
            return Ok(Action::Drop(format!("{container} cannot hold {}", s.codec)));
        }
        let Some(enc) = presets::pick_encoder(encoders, has_encoder) else {
            return Err(MediaError::Unsupported(no_encoder(encoders)));
        };
        let options =
            options.iter().chain(presets::hw_options(enc)).map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
        Ok(Action::Encode { encoder: enc.to_string(), options })
    };
    let mut chosen: Vec<(usize, Action)> = Vec::new();
    if let Some((s, encoders, options)) = video {
        chosen.push((s.index, decide(s, encoders, options)?));
    }
    if let Some((s, encoders, options)) = audio {
        chosen.push((s.index, decide(s, encoders, options)?));
    }
    for s in &info.streams {
        let action = match chosen.iter().find(|(i, _)| *i == s.index) {
            Some((_, a)) => a.clone(),
            None => Action::Drop("not kept".into()),
        };
        r.actions.push((s.index, action));
    }
    if r.kept().next().is_none() {
        return Err(MediaError::Unsupported("nothing to convert: no stream the preset keeps".into()));
    }
    Ok(r)
}

/// Encoder candidates for re-encoding a stream of `codec` into `container`: the
/// input's own codec first (its default encoder, when the build has one that opens),
/// then the container's usual choice.
fn reencode_candidates(kind: StreamKind, codec: &str, container: &str, own_encoder: Option<&str>) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    if let Some(own) = own_encoder
        && container_accepts(container, codec)
    {
        v.push(own.to_string());
    }
    let defaults: &[&str] = match (kind, container) {
        (StreamKind::Video, "mp4" | "ipod" | "mov" | "mpegts") => {
            &["h264_nvenc", "h264_amf", "h264_qsv", "h264_videotoolbox", "h264_vaapi", "mpeg4"]
        }
        (StreamKind::Video, "webm") => &["libvpx-vp9", "libvpx"],
        (StreamKind::Video, "matroska") => {
            &["h264_nvenc", "h264_amf", "h264_qsv", "h264_videotoolbox", "h264_vaapi", "libvpx-vp9", "mpeg4"]
        }
        (StreamKind::Video, "avi") => &["mpeg4", "mjpeg"],
        (StreamKind::Video, "ogg") => &["libvpx"],
        (StreamKind::Video, _) => &[],
        (StreamKind::Audio, "webm") => &["libopus"],
        (StreamKind::Audio, "ogg") => &["libopus", "flac"],
        (StreamKind::Audio, "avi") => &["mp2", "pcm_s16le"],
        (StreamKind::Audio, "wav") => &["pcm_s16le"],
        (StreamKind::Audio, "flac") => &["flac"],
        (StreamKind::Audio, "mov") => &["aac", "pcm_s16le"],
        (StreamKind::Audio, _) => &["aac"],
        _ => &[],
    };
    for d in defaults {
        if !v.iter().any(|e| e == d) {
            v.push((*d).to_string());
        }
    }
    v
}

/// Default options when re-encoding with `encoder` (a trim): a sensible quality.
fn reencode_options(encoder: &str) -> Vec<(String, String)> {
    let base: &[(&str, &str)] = match encoder {
        "mpeg4" => &[("flags", "+qscale"), ("global_quality", "472")],
        "mjpeg" => &[("flags", "+qscale"), ("global_quality", "354")],
        "libvpx-vp9" => &[("crf", "32"), ("b", "0"), ("row-mt", "1"), ("cpu-used", "2")],
        "libvpx" => &[("crf", "10"), ("b", "2M"), ("cpu-used", "2")],
        "aac" => &[("b", "192k")],
        "libopus" => &[("b", "128k")],
        "mp2" => &[("b", "160k")],
        _ => &[],
    };
    base.iter().chain(presets::hw_options(encoder)).map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect()
}

/// The decisions for `op` on `info` (the destination's extension decides a trim's
/// container; `own_encoder(codec)` names the build's default encoder for a codec, for
/// re-encoded trims; `has_encoder` says which encoders open). Pure: no library call.
fn resolve(
    info: &MediaInfo,
    dest_ext: Option<&str>,
    op: &MediaOp,
    has_encoder: &dyn Fn(&str) -> bool,
    own_encoder: &dyn Fn(&str) -> Option<String>,
) -> Result<Resolved, MediaError> {
    match op {
        MediaOp::Remux { container } => remux_all(info, container, ext_for_container(container), None),
        MediaOp::Convert { preset, video_stream, audio_stream } => {
            let p = preset.preset();
            if p.is_copy() {
                let mut r = remux_all(info, p.container, p.ext, None)?;
                // Explicit picks still apply to a copy preset: other streams of the
                // same kind are dropped.
                for wanted in [(StreamKind::Video, *video_stream), (StreamKind::Audio, *audio_stream)] {
                    if let (kind, Some(i)) = wanted {
                        choose_stream(info, kind, Some(i))?;
                        for (idx, a) in &mut r.actions {
                            let s = &info.streams[*idx];
                            if s.kind == Some(kind) && !s.attached_pic && *idx != i {
                                *a = Action::Drop("another stream was chosen".into());
                            }
                        }
                    }
                }
                return Ok(r);
            }
            let v = match p.video {
                Some(enc) => {
                    choose_stream(info, StreamKind::Video, *video_stream)?.map(|s| (s, enc.encoders, enc.options))
                }
                None => None,
            };
            let a = match p.audio {
                Some(enc) => {
                    choose_stream(info, StreamKind::Audio, *audio_stream)?.map(|s| (s, enc.encoders, enc.options))
                }
                None => None,
            };
            if p.video.is_some() && v.is_none() && a.is_none() {
                return Err(MediaError::NoStream(StreamKind::Video));
            }
            if p.video.is_none() && a.is_none() {
                return Err(MediaError::NoStream(StreamKind::Audio));
            }
            encode_one_each(info, p.container, p.ext, v, a, None, has_encoder)
        }
        MediaOp::ExtractAudio { codec } => {
            let s = info.audio().ok_or(MediaError::NoStream(StreamKind::Audio))?;
            match codec.preset() {
                None => {
                    let (container, ext) = audio_copy_container(&s.codec);
                    encode_one_each(info, container, ext, None, Some((s, &["copy"], &[])), None, has_encoder)
                }
                Some(id) => {
                    let p = id.preset();
                    let enc = p.audio.expect("audio presets have an audio encoder");
                    encode_one_each(
                        info,
                        p.container,
                        p.ext,
                        None,
                        Some((s, enc.encoders, enc.options)),
                        None,
                        has_encoder,
                    )
                }
            }
        }
        MediaOp::Trim { start, end, stream_copy } => {
            if !(start.is_finite() && end.is_finite() && *start >= 0.0 && *end > *start) {
                return Err(MediaError::Unsupported(format!("trim window {start}..{end} s")));
            }
            let container = container_for_trim(&info.container, dest_ext)
                .ok_or_else(|| MediaError::Unsupported(format!("cannot write a {} file", info.container)))?;
            let ext = dest_ext
                .filter(|e| mux::container_for_ext(e) == Some(container))
                .unwrap_or(ext_for_container(container));
            let window = Some((*start, *end));
            if *stream_copy {
                return remux_all(info, container, ext, window);
            }
            let mut r = Resolved {
                container: container.into(),
                ext: ext.into(),
                actions: Vec::new(),
                window,
                warnings: Vec::new(),
            };
            let mut kinds_seen = (false, false);
            for s in &info.streams {
                let kind = match s.kind {
                    Some(StreamKind::Video) if !s.attached_pic && !kinds_seen.0 => StreamKind::Video,
                    Some(StreamKind::Audio) if !kinds_seen.1 => StreamKind::Audio,
                    _ => {
                        r.actions.push((s.index, Action::Drop("not kept".into())));
                        continue;
                    }
                };
                let own = own_encoder(&s.codec);
                let candidates = reencode_candidates(kind, &s.codec, container, own.as_deref());
                let names: Vec<&str> = candidates.iter().map(String::as_str).collect();
                let Some(enc) = presets::pick_encoder(&names, has_encoder) else {
                    return Err(MediaError::Unsupported(format!(
                        "{}: {}",
                        stream_label(s),
                        if names.is_empty() { "no encoder for this container".to_string() } else { no_encoder(&names) }
                    )));
                };
                if kind == StreamKind::Video {
                    kinds_seen.0 = true;
                } else {
                    kinds_seen.1 = true;
                }
                r.actions.push((s.index, Action::Encode { encoder: enc.to_string(), options: reencode_options(enc) }));
            }
            if r.kept().next().is_none() {
                return Err(MediaError::Unsupported("nothing to trim: no video or audio stream".into()));
            }
            Ok(r)
        }
        MediaOp::Frame { format, .. } => {
            let s = info.video().ok_or(MediaError::NoStream(StreamKind::Video))?;
            let name = format.ext();
            let container = if *format == ImageFormat::Png { "png" } else { "jpeg" };
            Ok(Resolved {
                container: container.into(),
                ext: name.into(),
                actions: info
                    .streams
                    .iter()
                    .map(|x| {
                        let a = if x.index == s.index {
                            Action::Encode { encoder: container.into(), options: Vec::new() }
                        } else {
                            Action::Drop("not kept".into())
                        };
                        (x.index, a)
                    })
                    .collect(),
                window: None,
                warnings: Vec::new(),
            })
        }
    }
}

/// `"192k"` / `"2M"` / `"128000"` to bits per second.
fn parse_bitrate(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num, mult) = match s.chars().last() {
        Some('k' | 'K') => (&s[..s.len() - 1], 1_000),
        Some('M') => (&s[..s.len() - 1], 1_000_000),
        _ => (s, 1),
    };
    num.parse::<f64>().ok().filter(|v| *v >= 0.0).map(|v| (v * mult as f64) as u64)
}

/// A rough output size: copied streams keep their share of the input, encoded streams
/// take their option bitrate (video without one is assumed as big as the input).
fn estimate_bytes(info: &MediaInfo, r: &Resolved) -> Option<u64> {
    let duration = info.duration.filter(|d| *d > 0.0)?;
    let window = r.window.map_or(duration, |(s, e)| (e.min(duration) - s).max(0.0));
    if window <= 0.0 {
        return Some(0);
    }
    // Bytes of each input stream: its bit rate, or an equal share of what the known
    // ones leave of the file.
    let known: u64 = info.streams.iter().filter_map(|s| s.bit_rate).map(|b| b * duration as u64 / 8).sum();
    let unknown = info.streams.iter().filter(|s| s.bit_rate.is_none()).count() as u64;
    let share = info.size.saturating_sub(known).checked_div(unknown).unwrap_or(0);
    let input_bytes = |s: &StreamInfo| s.bit_rate.map_or(share, |b| b * duration as u64 / 8);
    let mut total = 0u64;
    for (idx, action) in &r.actions {
        let Some(s) = info.streams.iter().find(|s| s.index == *idx) else { continue };
        let bytes = match action {
            Action::Copy => input_bytes(s),
            Action::Encode { options, .. } => {
                match options.iter().find(|(k, _)| k == "b").and_then(|(_, v)| parse_bitrate(v)) {
                    Some(bps) if bps > 0 => bps * duration as u64 / 8,
                    _ if s.kind == Some(StreamKind::Audio) => input_bytes(s).max(16_000 * duration as u64),
                    _ => input_bytes(s),
                }
            }
            Action::Drop(_) => 0,
        };
        total += bytes;
    }
    Some((total as f64 * window / duration) as u64)
}

fn dest_ext(dest: &Path) -> Option<String> {
    dest.extension().map(|e| e.to_string_lossy().to_ascii_lowercase())
}

/// The temporary next to `dest` (`video.mp4.part`).
fn part_path(dest: &Path) -> PathBuf {
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dest.with_file_name(format!("{name}.part"))
}

/// The `Resolved` plan of `op` on an opened input.
fn resolve_with(rt: &Runtime, info: &MediaInfo, dest_ext: Option<&str>, op: &MediaOp) -> Result<Resolved, MediaError> {
    let av = rt.av();
    let own = |codec: &str| {
        info.streams
            .iter()
            .find(|s| s.codec == codec)
            .and_then(|s| codec_id_of(av, &s.codec))
            .and_then(|id| Encoder::name_for_id(av, id))
    };
    let r = resolve(info, dest_ext, op, &|name| rt.has_encoder(name), &own)?;
    if !matches!(op, MediaOp::Frame { .. }) && output_format(av, &r.container).is_none() {
        return Err(MediaError::Unsupported(format!("this FFmpeg build has no {} muxer", r.container)));
    }
    Ok(r)
}

/// The codec id of a codec name (through its descriptor list).
fn codec_id_of(av: &Av, name: &str) -> Option<c_int> {
    encode::codec_id_by_name(av, name)
}

/// The [`MediaInfo`] of an opened demuxer (what [`crate::probe`] builds, without the
/// cover bytes).
fn info_of(av: &'static Av, d: &Demuxer<'_>) -> MediaInfo {
    let (container, container_long) = d.format();
    MediaInfo {
        container,
        container_long,
        duration: d.duration(),
        start: d.start_time(),
        bit_rate: d.bit_rate(),
        size: d.size,
        streams: d.streams().map(|s| stream_info(av, d, s)).collect(),
        chapters: d.chapters().into_iter().map(|(start, end, title)| Chapter { start, end, title }).collect(),
        tags: d.metadata(),
        cover: None,
    }
}

/// The dry run of `op` on `src`: container, what happens to each stream, estimated size,
/// warnings (an encoder the machine lacks is an error, like in [`run`]).
pub fn plan(rt: &Runtime, src: &Path, op: &MediaOp) -> Result<PlanInfo, MediaError> {
    let av = rt.av();
    let d = Demuxer::open(av, &Source::File(src.to_path_buf()), None, Limits::default())?;
    let info = info_of(av, &d);
    let r = resolve_with(rt, &info, None, op)?;
    let duration = match (op, info.duration) {
        (MediaOp::Frame { .. }, _) => None,
        (MediaOp::Trim { start, end, .. }, d) => Some(d.map_or(end - start, |d| (end.min(d) - start).max(0.0))),
        (_, d) => d,
    };
    let mut warnings = r.warnings.clone();
    if !info.chapters.is_empty() && !matches!(op, MediaOp::Frame { .. }) {
        warnings.push("chapters are not copied".into());
    }
    Ok(PlanInfo {
        estimated_bytes: if matches!(op, MediaOp::Frame { .. }) { None } else { estimate_bytes(&info, &r) },
        streams: r.actions.iter().map(|(i, a)| (*i, a.describe())).collect(),
        container: r.container,
        ext: r.ext,
        duration,
        warnings,
    })
}

/// Run `op` from `src` to `dest`. `progress` is called between packets, `check` too
/// (returning `Err(Cancelled)` stops the work, removes the temporary and yields
/// [`MediaError::Cancelled`]). `dest` must not exist (the caller's conflict policy
/// decides what to do with an existing file); the result is written to `<dest>.part`
/// and renamed into place at the end.
pub fn run(
    rt: &Runtime,
    src: &Path,
    dest: &Path,
    op: &MediaOp,
    progress: &mut dyn FnMut(Progress),
    check: &mut dyn FnMut() -> Result<(), Cancelled>,
) -> Result<Outcome, MediaError> {
    if dest.exists() {
        return Err(MediaError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} exists", dest.display()),
        )));
    }
    let part = part_path(dest);
    let result = match op {
        MediaOp::Frame { .. } => save_frame(rt, src, &part, op, progress, check),
        _ => run_inner(rt, src, &part, dest, op, progress, check),
    };
    match result {
        Ok(outcome) => {
            if let Err(e) = std::fs::rename(&part, dest) {
                let _ = std::fs::remove_file(&part);
                return Err(MediaError::Io(e));
            }
            Ok(outcome)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

fn save_frame(
    rt: &Runtime,
    src: &Path,
    part: &Path,
    op: &MediaOp,
    progress: &mut dyn FnMut(Progress),
    check: &mut dyn FnMut() -> Result<(), Cancelled>,
) -> Result<Outcome, MediaError> {
    let MediaOp::Frame { at, format, max_px } = *op else {
        return Err(MediaError::Unsupported("not a frame operation".into()));
    };
    check().map_err(|_| MediaError::Cancelled)?;
    let input_total = std::fs::metadata(src)?.len();
    let req = FrameRequest { at, max_px: max_px.unwrap_or(u32::MAX), exact: true };
    let img = grab_frame(rt, &Source::File(src.to_path_buf()), req, &|| false)?;
    check().map_err(|_| MediaError::Cancelled)?;
    let mut bytes = Vec::new();
    match format {
        ImageFormat::Png => img
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .map_err(|e| MediaError::Unsupported(format!("PNG encode: {e}")))?,
        ImageFormat::Jpeg => {
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 92);
            image::DynamicImage::ImageRgba8(img)
                .to_rgb8()
                .write_with_encoder(enc)
                .map_err(|e| MediaError::Unsupported(format!("JPEG encode: {e}")))?;
        }
    }
    std::fs::write(part, &bytes)?;
    let out_bytes = bytes.len() as u64;
    progress(Progress { input_bytes: input_total, input_total, secs: at.max(0.0), out_bytes });
    Ok(Outcome { out_bytes, duration: None, warnings: Vec::new() })
}

/// Progress bookkeeping with throttled reports.
struct Prog {
    input_total: u64,
    /// Seconds the fraction fallback divides by (the window length for a trim).
    span: Option<f64>,
    /// Always report by time (a trim: the window done, not the file position).
    by_time: bool,
    max_pos: u64,
    max_secs: f64,
    /// The last report (`None` before the first).
    last: Option<Instant>,
}

impl Prog {
    fn tick(&mut self, pos: i64, secs: Option<f64>, out_bytes: u64, progress: &mut dyn FnMut(Progress), force: bool) {
        if let Ok(p) = u64::try_from(pos) {
            self.max_pos = self.max_pos.max(p);
        }
        if let Some(s) = secs
            && s.is_finite()
        {
            self.max_secs = self.max_secs.max(s);
        }
        if !force && self.last.is_some_and(|l| l.elapsed() < Duration::from_millis(50)) {
            return;
        }
        self.last = Some(Instant::now());
        let by_time = || {
            self.span
                .filter(|d| *d > 0.0)
                .map_or(0, |d| ((self.max_secs / d).clamp(0.0, 1.0) * self.input_total as f64) as u64)
        };
        let input_bytes =
            if self.by_time || self.max_pos == 0 { by_time() } else { self.max_pos.min(self.input_total) };
        progress(Progress { input_bytes, input_total: self.input_total, secs: self.max_secs, out_bytes });
    }
}

/// A copied stream.
struct CopyState {
    out: usize,
    tb_in: AVRational,
    tb_out: AVRational,
    video: bool,
    last_dts: Option<i64>,
    /// Non-monotonic timestamps were fixed up (reported once).
    fixed: bool,
}

struct VideoState {
    dst: (c_int, c_int, c_int),
    conv: Option<(PixConv, Frame)>,
    last_pts: Option<i64>,
}

struct AudioState {
    layout: ChLayout,
    out_fmt: c_int,
    out_rate: c_int,
    frame_size: usize,
    conv: Option<AudioConv>,
    cur: Option<Frame>,
    filled: usize,
    next_pts: i64,
    started: bool,
}

/// A re-encoded stream.
struct EncState {
    out: usize,
    dec: Decoder,
    enc: Encoder,
    tb_in: AVRational,
    tb_out: AVRational,
    video: Option<VideoState>,
    audio: Option<AudioState>,
}

enum State {
    Copy(CopyState),
    Enc(Box<EncState>),
}

/// Everything the packet loop touches.
struct Engine<'a> {
    av: &'static Av,
    muxer: Muxer,
    states: Vec<Option<State>>,
    /// Input index of the stream that defines the cut and the time origin.
    main: usize,
    /// Absolute (input clock) window, when trimming.
    window: Option<(f64, f64)>,
    /// Absolute time of output zero; `None` until the first kept main packet of a trim.
    origin: Option<f64>,
    /// Streams whose packets passed `end`.
    done: Vec<bool>,
    out_secs: f64,
    warnings: Vec<String>,
    prog: Prog,
    progress: &'a mut dyn FnMut(Progress),
    opkt: Packet,
}

fn secs_to_ts(secs: f64, tb: AVRational) -> i64 {
    if tb.num <= 0 || tb.den <= 0 {
        return 0;
    }
    (secs * f64::from(tb.den) / f64::from(tb.num)).round() as i64
}

impl Engine<'_> {
    fn origin_ts(&self, tb: AVRational) -> i64 {
        secs_to_ts(self.origin.unwrap_or(0.0), tb)
    }

    fn note_written(&mut self, pkt: &Packet, tb_out: AVRational) {
        if let Some(s) = tb_out.secs(pkt.pts()) {
            let end = s + tb_out.secs(pkt.duration()).unwrap_or(0.0);
            if end.is_finite() {
                self.out_secs = self.out_secs.max(end);
            }
        }
    }

    /// Stream copy of one packet (timestamps shifted to the origin and rescaled).
    fn copy_packet(&mut self, idx: usize, pkt: &mut Packet) -> Result<(), MediaError> {
        let Some(State::Copy(c)) = self.states[idx].as_mut() else { return Ok(()) };
        let (tb_in, tb_out, out, video) = (c.tb_in, c.tb_out, c.out, c.video);
        let ts = if pkt.pts() == types::AV_NOPTS_VALUE { pkt.dts() } else { pkt.pts() };
        let secs = tb_in.secs(ts);
        if let Some((start, end)) = self.window {
            if self.origin.is_none() {
                if idx != self.main {
                    return Ok(());
                }
                let Some(s) = secs else { return Ok(()) };
                let dur = tb_in.secs(pkt.duration()).unwrap_or(0.0);
                let keep = if video { pkt.is_key() } else { s + dur > start };
                if !keep {
                    return Ok(());
                }
                self.origin = Some(s);
                if video && (s - start).abs() > 1e-3 {
                    self.warnings.push(format!("the cut starts at the keyframe at {s:.3} s (asked for {start:.3} s)"));
                }
            }
            if let Some(s) = secs {
                if s < self.origin.unwrap_or(start) - 1e-6 {
                    return Ok(());
                }
                if s >= end {
                    self.done[idx] = true;
                    return Ok(());
                }
            }
        }
        let shift = self.origin_ts(tb_in);
        let shifted = |t: i64| if t == types::AV_NOPTS_VALUE { t } else { t - shift };
        mux::pkt_set_ts(pkt, shifted(pkt.pts()), shifted(pkt.dts()));
        mux::pkt_rescale_ts(self.av, pkt, tb_in, tb_out);
        let Some(State::Copy(c)) = self.states[idx].as_mut() else { return Ok(()) };
        let mut dts = pkt.dts();
        let mut pts = pkt.pts();
        if dts != types::AV_NOPTS_VALUE {
            if let Some(last) = c.last_dts
                && dts <= last
            {
                dts = last + 1;
                if pts != types::AV_NOPTS_VALUE && pts < dts {
                    pts = dts;
                }
                mux::pkt_set_ts(pkt, pts, dts);
                if !c.fixed {
                    c.fixed = true;
                    self.warnings.push(format!("stream {idx}: non-monotonic timestamps were fixed up"));
                }
            }
            c.last_dts = Some(dts);
        }
        mux::pkt_set_stream_index(pkt, out);
        mux::pkt_clear_pos(pkt);
        self.note_written(pkt, tb_out);
        self.muxer.write(pkt)
    }

    /// Decode one packet (`None` = flush) and encode what comes out.
    fn decode_packet(&mut self, idx: usize, pkt: Option<&Packet>, dframe: &mut Frame) -> Result<(), MediaError> {
        {
            let Some(State::Enc(e)) = self.states[idx].as_mut() else { return Ok(()) };
            e.dec.send(pkt)?;
        }
        loop {
            let got = {
                let Some(State::Enc(e)) = self.states[idx].as_mut() else { return Ok(()) };
                e.dec.receive(dframe)?
            };
            match got {
                Recv::Frame => self.process_frame(idx, dframe)?,
                Recv::Again | Recv::Eof => return Ok(()),
            }
        }
    }

    fn process_frame(&mut self, idx: usize, frame: &mut Frame) -> Result<(), MediaError> {
        let (tb_in, is_video) = {
            let Some(State::Enc(e)) = self.states[idx].as_ref() else { return Ok(()) };
            (e.tb_in, e.video.is_some())
        };
        let ts = {
            let b = frame.best_effort_timestamp();
            if b == types::AV_NOPTS_VALUE { frame.pts() } else { b }
        };
        let secs = tb_in.secs(ts);
        if let Some((start, end)) = self.window
            && let Some(s) = secs
        {
            if s < start - 1e-6 {
                return Ok(());
            }
            if s >= end {
                self.done[idx] = true;
                return Ok(());
            }
        }
        if self.origin.is_none() {
            self.origin = Some(self.window.map_or(0.0, |(s, _)| s));
        }
        let rel = if ts == types::AV_NOPTS_VALUE { 0 } else { ts - self.origin_ts(tb_in) };
        if is_video { self.push_video(idx, frame, rel) } else { self.push_audio(idx, Some(&*frame), rel) }
    }

    fn push_video(&mut self, idx: usize, frame: &mut Frame, rel: i64) -> Result<(), MediaError> {
        let av = self.av;
        let Some(State::Enc(e)) = self.states[idx].as_mut() else { return Ok(()) };
        let Some(v) = e.video.as_mut() else { return Ok(()) };
        let mut pts = mux::rescale(av, rel, e.tb_in, e.enc.time_base());
        if let Some(last) = v.last_pts
            && pts <= last
        {
            pts = last + 1;
        }
        v.last_pts = Some(pts);
        let quality = e.enc.global_quality();
        if (frame.width(), frame.height(), frame.format()) == v.dst {
            encode::set_pts(av, frame, pts);
            encode::set_quality(av, frame, quality);
            return self.send_frame(idx, Some(frame));
        }
        if v.conv.as_ref().is_none_or(|(c, _)| !c.matches(frame)) {
            let conv = PixConv::new(av, (frame.width(), frame.height(), frame.format()), v.dst)?;
            let dst = encode::new_video_frame(av, v.dst.0, v.dst.1, v.dst.2)?;
            v.conv = Some((conv, dst));
        }
        let (conv, dst) = v.conv.as_mut().expect("set above");
        encode::make_writable(av, dst)?;
        conv.convert(frame, dst)?;
        encode::set_pts(av, dst, pts);
        encode::set_quality(av, dst, quality);
        // Take the frame out for the call (the encoder borrows the state).
        let (conv, dst) = v.conv.take().expect("set above");
        let r = self.send_frame(idx, Some(&dst));
        if let Some(State::Enc(e)) = self.states[idx].as_mut()
            && let Some(v) = e.video.as_mut()
        {
            v.conv = Some((conv, dst));
        }
        r
    }

    /// Resample `input` (`None` = drain at the end) into encoder-sized frames.
    fn push_audio(&mut self, idx: usize, input: Option<&Frame>, rel: i64) -> Result<(), MediaError> {
        let av = self.av;
        let Some(State::Enc(e)) = self.states[idx].as_mut() else { return Ok(()) };
        let Some(a) = e.audio.as_mut() else { return Ok(()) };
        if let Some(f) = input {
            let (fmt, rate, n) = (f.format(), f.sample_rate(), f.nb_samples());
            if n <= 0 || rate <= 0 {
                return Ok(());
            }
            let in_layout = e.dec.ch_layout()?;
            if a.conv.as_ref().is_none_or(|c| !c.matches(fmt, rate, in_layout.channels())) {
                a.conv = Some(AudioConv::new(av, &in_layout, fmt, rate, &a.layout.l, a.out_fmt, a.out_rate)?);
            }
            if !a.started {
                a.started = true;
                a.next_pts = mux::rescale(av, rel, e.tb_in, AVRational::new(1, a.out_rate)).max(0);
            }
        }
        if a.conv.is_none() {
            return Ok(());
        }
        let mut input = input;
        let mut ready: Vec<Frame> = Vec::new();
        loop {
            if a.cur.is_none() {
                a.cur = Some(encode::new_audio_frame(av, a.frame_size as c_int, a.out_fmt, a.out_rate, &a.layout.l)?);
                a.filled = 0;
            }
            let conv = a.conv.as_mut().expect("checked above");
            let cur = a.cur.as_mut().expect("set above");
            let n = conv.convert_into(input.take(), cur, a.filled, a.frame_size)?;
            a.filled += n;
            if a.filled < a.frame_size {
                break;
            }
            encode::set_pts(av, cur, a.next_pts);
            a.next_pts += a.frame_size as i64;
            ready.push(a.cur.take().expect("set above"));
        }
        for f in &ready {
            self.send_frame(idx, Some(f))?;
        }
        Ok(())
    }

    /// The partially filled last audio frame, at the end of the input.
    fn flush_audio(&mut self, idx: usize) -> Result<(), MediaError> {
        self.push_audio(idx, None, 0)?;
        let av = self.av;
        let last = {
            let Some(State::Enc(e)) = self.states[idx].as_mut() else { return Ok(()) };
            let Some(a) = e.audio.as_mut() else { return Ok(()) };
            match a.cur.take() {
                Some(mut f) if a.filled > 0 => {
                    encode::set_nb_samples(av, &mut f, a.filled as c_int);
                    encode::set_pts(av, &mut f, a.next_pts);
                    a.next_pts += a.filled as i64;
                    Some(f)
                }
                _ => None,
            }
        };
        if let Some(f) = last {
            self.send_frame(idx, Some(&f))?;
        }
        Ok(())
    }

    /// Hand a frame (or the flush, `None`) to the encoder and write what it produces.
    fn send_frame(&mut self, idx: usize, frame: Option<&Frame>) -> Result<(), MediaError> {
        loop {
            let accepted = {
                let Some(State::Enc(e)) = self.states[idx].as_mut() else { return Ok(()) };
                e.enc.send(frame)?
            };
            self.drain_encoder(idx)?;
            if accepted {
                return Ok(());
            }
        }
    }

    fn drain_encoder(&mut self, idx: usize) -> Result<(), MediaError> {
        loop {
            let (got, tb_enc, tb_out, out) = {
                let Some(State::Enc(e)) = self.states[idx].as_mut() else { return Ok(()) };
                (e.enc.receive(&mut self.opkt)?, e.enc.time_base(), e.tb_out, e.out)
            };
            match got {
                Got::Packet => {
                    mux::pkt_rescale_ts(self.av, &mut self.opkt, tb_enc, tb_out);
                    mux::pkt_set_stream_index(&mut self.opkt, out);
                    mux::pkt_clear_pos(&mut self.opkt);
                    let mut pkt = std::mem::replace(&mut self.opkt, Packet::new(self.av)?);
                    self.note_written(&pkt, tb_out);
                    let r = self.muxer.write(&mut pkt);
                    self.opkt = pkt;
                    r?;
                }
                Got::Again | Got::Eof => return Ok(()),
            }
        }
    }
}

/// Pick the encoder's sample format / rate for an input stream.
fn audio_spec(
    av: &'static Av,
    codec_ptr: *const types::AVCodec,
    s: StreamRef<'_>,
) -> Result<(c_int, c_int, ChLayout), MediaError> {
    let par = s.par();
    let fmts = Encoder::sample_fmts(av, codec_ptr);
    let in_name = codec::sample_fmt_name(av, par.format);
    let fmt = fmts
        .iter()
        .find(|(_, n)| Some(n) == in_name.as_ref())
        .or_else(|| fmts.first())
        .map(|(f, _)| *f)
        .or(if par.format >= 0 { Some(par.format) } else { None })
        .or_else(|| codec::sample_fmt(av, "fltp"))
        .ok_or_else(|| MediaError::Unsupported("no sample format".into()))?;
    let rates = Encoder::sample_rates(av, codec_ptr);
    let in_rate = par.sample_rate.max(1);
    let rate = if rates.is_empty() || rates.contains(&in_rate) {
        in_rate
    } else {
        rates.iter().copied().filter(|r| *r >= in_rate).min().or_else(|| rates.iter().copied().max()).unwrap_or(48_000)
    };
    let channels = par.ch_layout.nb_channels.max(1);
    // An unspecified order (a plain WAV) is refused by most encoders: use the default
    // layout of that channel count.
    let layout = if par.ch_layout.order == 0 {
        ChLayout::default_for(av, channels)
    } else {
        ChLayout::copy_of(av, &par.ch_layout)?
    };
    Ok((fmt, rate, layout))
}

/// Pick the encoder's pixel format and time base for an input stream.
fn video_spec(
    av: &Av,
    codec_ptr: *const types::AVCodec,
    d: &Demuxer<'_>,
    s: StreamRef<'_>,
) -> Result<(c_int, AVRational, AVRational), MediaError> {
    let par = s.par();
    let fmts = Encoder::pix_fmts(av, codec_ptr);
    let in_name = codec::pix_fmt_name(av, par.format);
    let fmt = fmts
        .iter()
        .find(|(_, n)| Some(n) == in_name.as_ref())
        .or_else(|| fmts.first())
        .map(|(f, _)| *f)
        .or(if par.format >= 0 { Some(par.format) } else { None })
        .or_else(|| codec::pix_fmt(av, "yuv420p"))
        .ok_or_else(|| MediaError::Unsupported("no pixel format".into()))?;
    let fps = d.guess_frame_rate(s);
    let tb_in = s.time_base();
    // The frame rate's inverse keeps encoders with a bounded time base (MPEG-4 part 2)
    // happy; variable-rate input keeps its own clock.
    let tb = if fps.num > 0 && fps.den > 0 && fps.num <= 65_535 { AVRational::new(fps.den, fps.num) } else { tb_in };
    let framerate = if fps.num > 0 && fps.den > 0 { fps } else { AVRational::new(0, 1) };
    Ok((fmt, tb, framerate))
}

fn run_inner(
    rt: &Runtime,
    src: &Path,
    part: &Path,
    dest: &Path,
    op: &MediaOp,
    progress: &mut dyn FnMut(Progress),
    check: &mut dyn FnMut() -> Result<(), Cancelled>,
) -> Result<Outcome, MediaError> {
    let av = rt.av();
    let mut d = Demuxer::open(av, &Source::File(src.to_path_buf()), None, Limits::default())?;
    let info = info_of(av, &d);
    let r = resolve_with(rt, &info, dest_ext(dest).as_deref(), op)?;
    let start_time = d.start_time();
    let file = std::fs::File::create(part)?;
    let muxer = Muxer::new(av, &r.container, Box::new(file))?;
    let global_header = muxer.needs_global_header();
    let window = r.window.map(|(s, e)| (start_time + s, start_time + e));
    let mut engine = Engine {
        av,
        muxer,
        states: (0..d.stream_count()).map(|_| None).collect(),
        main: 0,
        window,
        origin: if window.is_some() { None } else { Some(start_time) },
        done: vec![false; d.stream_count()],
        out_secs: 0.0,
        warnings: r.warnings.clone(),
        prog: Prog {
            input_total: d.size,
            span: r.window.map_or(info.duration, |(s, e)| Some(e - s)),
            by_time: window.is_some(),
            max_pos: 0,
            max_secs: 0.0,
            last: None,
        },
        progress,
        opkt: Packet::new(av)?,
    };
    if !info.chapters.is_empty() {
        engine.warnings.push("chapters are not copied".into());
    }
    // The main stream: the first kept video stream, else the first kept stream.
    let kept: Vec<(usize, &Action)> = r.kept().collect();
    engine.main = kept
        .iter()
        .find(|(i, _)| info.streams[*i].kind == Some(StreamKind::Video) && !info.streams[*i].attached_pic)
        .or_else(|| kept.first())
        .map(|(i, _)| *i)
        .ok_or_else(|| MediaError::Unsupported("no stream to write".into()))?;

    // Output streams, in the order of the plan.
    let mut enc_pars: Vec<OwnedPar> = Vec::new();
    for (idx, action) in &kept {
        let idx = *idx;
        let s = d.stream(idx).ok_or(MediaError::Unsupported(format!("stream {idx} vanished")))?;
        let tags = s.metadata();
        match action {
            Action::Copy => {
                let out = engine.muxer.add_stream(&OutStream {
                    par: s.par_ptr(),
                    time_base: s.time_base(),
                    avg_frame_rate: s.avg_frame_rate(),
                    sample_aspect_ratio: s.sample_aspect_ratio(),
                    disposition: s.disposition(),
                    tags: &tags,
                    clear_tag: true,
                })?;
                engine.states[idx] = Some(State::Copy(CopyState {
                    out,
                    tb_in: s.time_base(),
                    tb_out: AVRational::new(0, 1),
                    video: s.kind() == Some(StreamKind::Video),
                    last_dts: None,
                    fixed: false,
                }));
            }
            Action::Encode { encoder, options } => {
                let kind = s.kind().ok_or_else(|| MediaError::Unsupported(format!("stream {idx} has no type")))?;
                let (_, decoder) = d.best_stream(kind, Some(idx))?;
                let dec = Decoder::open(av, decoder, s.par_ptr(), s.time_base(), &[("threads", "auto")])?;
                let (codec_ptr, _, _) = Encoder::find(av, encoder)
                    .ok_or_else(|| MediaError::Unsupported(format!("no encoder {encoder}")))?;
                let opts: Vec<(&str, &str)> = options.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                let par = s.par();
                let (enc, warnings, video, audio) = if kind == StreamKind::Video {
                    if par.width <= 0 || par.height <= 0 {
                        return Err(MediaError::Unsupported(format!("stream {idx}: unknown picture size")));
                    }
                    let (fmt, tb, framerate) = video_spec(av, codec_ptr, &d, s)?;
                    let sar = d.guess_sar(s);
                    let sar = if sar.num > 0 && sar.den > 0 { sar } else { AVRational::new(0, 1) };
                    let spec = EncSpec::Video {
                        width: par.width,
                        height: par.height,
                        pix_fmt: fmt,
                        time_base: tb,
                        framerate,
                        sar,
                        color: [par.color_range, par.color_primaries, par.color_trc, par.color_space],
                    };
                    let (enc, w) = Encoder::open(av, encoder, &spec, global_header, &opts)?;
                    (enc, w, Some(VideoState { dst: (par.width, par.height, fmt), conv: None, last_pts: None }), None)
                } else {
                    let (fmt, rate, layout) = audio_spec(av, codec_ptr, s)?;
                    let spec = EncSpec::Audio { sample_rate: rate, sample_fmt: fmt, layout: &layout.l };
                    let (enc, w) = Encoder::open(av, encoder, &spec, global_header, &opts)?;
                    let frame_size = if enc.variable_frame_size() { 4096 } else { enc.frame_size() };
                    let audio = AudioState {
                        layout,
                        out_fmt: fmt,
                        out_rate: rate,
                        frame_size,
                        conv: None,
                        cur: None,
                        filled: 0,
                        next_pts: 0,
                        started: false,
                    };
                    (enc, w, None, Some(audio))
                };
                engine.warnings.extend(warnings);
                let mut enc_par = enc.par()?;
                // The rotation travels as side data of the parameters; an encoder's own
                // parameters start without it.
                if let Some(m) = s.side_data(types::AV_PKT_DATA_DISPLAYMATRIX)
                    && kind == StreamKind::Video
                {
                    enc_par.add_side_data(types::AV_PKT_DATA_DISPLAYMATRIX, m)?;
                }
                let out = engine.muxer.add_stream(&OutStream {
                    par: enc_par.as_ptr(),
                    time_base: enc.time_base(),
                    avg_frame_rate: if kind == StreamKind::Video {
                        d.guess_frame_rate(s)
                    } else {
                        AVRational::new(0, 1)
                    },
                    sample_aspect_ratio: if kind == StreamKind::Video { d.guess_sar(s) } else { AVRational::new(0, 1) },
                    disposition: s.disposition() & !types::AV_DISPOSITION_ATTACHED_PIC,
                    tags: &tags,
                    clear_tag: false,
                })?;
                enc_pars.push(enc_par);
                engine.states[idx] = Some(State::Enc(Box::new(EncState {
                    out,
                    dec,
                    enc,
                    tb_in: s.time_base(),
                    tb_out: AVRational::new(0, 1),
                    video,
                    audio,
                })));
            }
            Action::Drop(_) => {}
        }
    }
    drop(enc_pars);
    engine.muxer.set_metadata(&info.tags);
    // No `movflags=+faststart`: the MOV muxer re-opens the file by URL for that, which
    // a custom AVIO cannot do.
    let header_warnings = engine.muxer.write_header(&[])?;
    engine.warnings.extend(header_warnings);
    for st in engine.states.iter_mut().flatten() {
        match st {
            State::Copy(c) => c.tb_out = engine.muxer.stream_time_base(c.out),
            State::Enc(e) => e.tb_out = engine.muxer.stream_time_base(e.out),
        }
    }
    if let Some((start, _)) = window
        && start > 0.0
    {
        let ts = (start * types::AV_TIME_BASE as f64).round() as i64;
        if let Err(e) = d.seek_before(None, ts) {
            engine.warnings.push(format!("seek to {start:.3} s failed ({e}); reading from the start"));
        }
    }

    let mut pkt = Packet::new(av)?;
    let mut dframe = Frame::new(av)?;
    let mapped: Vec<usize> = (0..engine.states.len()).filter(|i| engine.states[*i].is_some()).collect();
    loop {
        check().map_err(|_| MediaError::Cancelled)?;
        if !d.read(&mut pkt)? {
            break;
        }
        let idx = pkt.stream_index();
        if idx >= engine.states.len() || engine.states[idx].is_none() || engine.done[idx] {
            if window.is_some() && mapped.iter().all(|i| engine.done[*i]) {
                break;
            }
            continue;
        }
        let tb_in = match engine.states[idx].as_ref() {
            Some(State::Copy(c)) => c.tb_in,
            Some(State::Enc(e)) => e.tb_in,
            None => AVRational::new(0, 1),
        };
        let pos = pkt.pos();
        let secs = tb_in.secs(pkt.pts()).map(|s| s - engine.origin.unwrap_or(start_time));
        match engine.states[idx].as_ref() {
            Some(State::Copy(_)) => engine.copy_packet(idx, &mut pkt)?,
            Some(State::Enc(_)) => engine.decode_packet(idx, Some(&pkt), &mut dframe)?,
            None => {}
        }
        let out_bytes = engine.muxer.bytes_written();
        engine.prog.tick(pos, secs, out_bytes, engine.progress, false);
        if window.is_some() && mapped.iter().all(|i| engine.done[*i]) {
            break;
        }
    }
    // Flush: decoders, the audio resamplers, then the encoders.
    for idx in mapped {
        check().map_err(|_| MediaError::Cancelled)?;
        if matches!(engine.states[idx], Some(State::Enc(_))) {
            engine.decode_packet(idx, None, &mut dframe)?;
            engine.flush_audio(idx)?;
            engine.send_frame(idx, None)?;
        }
    }
    engine.muxer.finish()?;
    let out_bytes = engine.muxer.bytes_written();
    engine.prog.tick(-1, Some(engine.out_secs), out_bytes, engine.progress, true);
    let duration = (engine.out_secs > 0.0).then_some(engine.out_secs);
    Ok(Outcome { out_bytes, duration, warnings: engine.warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(index: usize, codec: &str) -> StreamInfo {
        StreamInfo {
            index,
            kind: Some(StreamKind::Video),
            codec: codec.into(),
            width: 1920,
            height: 1080,
            bit_rate: Some(4_000_000),
            ..Default::default()
        }
    }

    fn audio(index: usize, codec: &str) -> StreamInfo {
        StreamInfo {
            index,
            kind: Some(StreamKind::Audio),
            codec: codec.into(),
            sample_rate: 48_000,
            channels: 2,
            bit_rate: Some(128_000),
            default: true,
            ..Default::default()
        }
    }

    fn sub(index: usize, codec: &str) -> StreamInfo {
        StreamInfo { index, kind: Some(StreamKind::Subtitle), codec: codec.into(), ..Default::default() }
    }

    fn mkv() -> MediaInfo {
        MediaInfo {
            container: "matroska,webm".into(),
            duration: Some(60.0),
            size: 40_000_000,
            streams: vec![video(0, "h264"), audio(1, "aac"), sub(2, "subrip")],
            ..Default::default()
        }
    }

    fn none(_: &str) -> bool {
        false
    }

    fn software(n: &str) -> bool {
        matches!(
            n,
            "aac"
                | "flac"
                | "alac"
                | "pcm_s16le"
                | "mpeg4"
                | "prores_ks"
                | "libopus"
                | "libvpx-vp9"
                | "libvpx"
                | "mjpeg"
        )
    }

    fn no_own(_: &str) -> Option<String> {
        None
    }

    fn actions(r: &Resolved) -> Vec<String> {
        r.actions.iter().map(|(i, a)| format!("{i}:{}", a.describe())).collect()
    }

    #[test]
    fn remux_keeps_what_fits() {
        let r = resolve(&mkv(), None, &MediaOp::Remux { container: "mp4".into() }, &none, &no_own).unwrap();
        assert_eq!(r.container, "mp4");
        assert_eq!(r.ext, "mp4");
        assert_eq!(actions(&r), ["0:copy", "1:copy", "2:drop: mp4 cannot hold subrip"]);
        assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
        let r = resolve(&mkv(), None, &MediaOp::Remux { container: "matroska".into() }, &none, &no_own).unwrap();
        assert_eq!(actions(&r), ["0:copy", "1:copy", "2:copy"]);
        assert_eq!(r.ext, "mkv");
        // Nothing fits: an error, not an empty file.
        let e = resolve(&mkv(), None, &MediaOp::Remux { container: "webm".into() }, &none, &no_own).unwrap_err();
        assert!(matches!(e, MediaError::Unsupported(_)), "{e}");
    }

    #[test]
    fn convert_picks_encoders_or_fails_clearly() {
        let op = |preset| MediaOp::Convert { preset, video_stream: None, audio_stream: None };
        let r = resolve(&mkv(), None, &op(PresetId::WebmVp9), &software, &no_own).unwrap();
        assert_eq!(actions(&r), ["0:encode libvpx-vp9", "1:encode libopus", "2:drop: not kept"]);
        assert_eq!((r.container.as_str(), r.ext.as_str()), ("webm", "webm"));
        let e = resolve(&mkv(), None, &op(PresetId::Mp4H264Hw), &software, &no_own).unwrap_err();
        assert_eq!(e.to_string(), "unsupported: no H.264 hardware encoder on this machine");
        let hw = |n: &str| n == "h264_qsv" || n == "aac";
        let r = resolve(&mkv(), None, &op(PresetId::Mp4H264Hw), &hw, &no_own).unwrap();
        assert_eq!(actions(&r)[0], "0:encode h264_qsv");
        if let Action::Encode { options, .. } = &r.actions[0].1 {
            assert!(options.iter().any(|(k, _)| k == "global_quality"), "QSV quality options: {options:?}");
        }
        // Copy presets are remuxes; an explicit stream pick drops its siblings.
        let two_audio = MediaInfo { streams: vec![video(0, "h264"), audio(1, "aac"), audio(2, "ac3")], ..mkv() };
        let r = resolve(
            &two_audio,
            None,
            &MediaOp::Convert { preset: PresetId::Mp4Copy, video_stream: None, audio_stream: Some(2) },
            &none,
            &no_own,
        )
        .unwrap();
        assert_eq!(actions(&r), ["0:copy", "1:drop: another stream was chosen", "2:copy"]);
        assert!(matches!(
            resolve(
                &mkv(),
                None,
                &MediaOp::Convert { preset: PresetId::Mp4Copy, video_stream: Some(1), audio_stream: None },
                &none,
                &no_own
            ),
            Err(MediaError::NoStream(StreamKind::Video))
        ));
        // Audio presets keep the audio only.
        let r = resolve(&mkv(), None, &op(PresetId::AudioFlac), &software, &no_own).unwrap();
        assert_eq!(actions(&r), ["0:drop: not kept", "1:encode flac", "2:drop: not kept"]);
        assert_eq!(r.ext, "flac");
        // A video preset on an audio-only file converts the audio alone.
        let wav = MediaInfo { container: "wav".into(), streams: vec![audio(0, "pcm_s16le")], ..mkv() };
        let r = resolve(&wav, None, &op(PresetId::Mp4Mpeg4), &software, &no_own).unwrap();
        assert_eq!(actions(&r), ["0:encode aac"]);
        assert!(matches!(resolve(&wav, None, &op(PresetId::WebmVp8), &none, &no_own), Err(MediaError::Unsupported(_))));
    }

    #[test]
    fn extract_audio_copy_picks_the_container() {
        let r =
            resolve(&mkv(), None, &MediaOp::ExtractAudio { codec: AudioCodecChoice::Copy }, &none, &no_own).unwrap();
        assert_eq!((r.container.as_str(), r.ext.as_str()), ("ipod", "m4a"));
        assert_eq!(actions(&r), ["0:drop: not kept", "1:copy", "2:drop: not kept"]);
        let opus = MediaInfo { streams: vec![video(0, "vp9"), audio(1, "opus")], ..mkv() };
        let r = resolve(&opus, None, &MediaOp::ExtractAudio { codec: AudioCodecChoice::Copy }, &none, &no_own).unwrap();
        assert_eq!((r.container.as_str(), r.ext.as_str()), ("ogg", "opus"));
        let r =
            resolve(&mkv(), None, &MediaOp::ExtractAudio { codec: AudioCodecChoice::Wav }, &software, &no_own).unwrap();
        assert_eq!((r.container.as_str(), r.ext.as_str()), ("wav", "wav"));
        assert_eq!(actions(&r)[1], "1:encode pcm_s16le");
        let silent = MediaInfo { streams: vec![video(0, "h264")], ..mkv() };
        assert!(matches!(
            resolve(&silent, None, &MediaOp::ExtractAudio { codec: AudioCodecChoice::Aac }, &software, &no_own),
            Err(MediaError::NoStream(StreamKind::Audio))
        ));
    }

    #[test]
    fn trim_follows_the_input_or_the_destination() {
        let copy = MediaOp::Trim { start: 1.0, end: 5.0, stream_copy: true };
        let r = resolve(&mkv(), None, &copy, &none, &no_own).unwrap();
        assert_eq!((r.container.as_str(), r.ext.as_str()), ("matroska", "mkv"));
        assert_eq!(r.window, Some((1.0, 5.0)));
        assert_eq!(actions(&r), ["0:copy", "1:copy", "2:copy"]);
        let r = resolve(&mkv(), Some("mp4"), &copy, &none, &no_own).unwrap();
        assert_eq!((r.container.as_str(), r.ext.as_str()), ("mp4", "mp4"));
        for (start, end) in [(5.0, 1.0), (-1.0, 2.0), (f64::NAN, 1.0), (1.0, 1.0)] {
            assert!(resolve(&mkv(), None, &MediaOp::Trim { start, end, stream_copy: true }, &none, &no_own).is_err());
        }
        // Frame-exact: the input's own codec when the build encodes it, else the
        // container's usual choice; no software H.264 → MPEG-4 in MP4.
        let exact = MediaOp::Trim { start: 1.0, end: 5.0, stream_copy: false };
        let own = |c: &str| (c == "aac").then(|| "aac".to_string());
        let r = resolve(&mkv(), Some("mp4"), &exact, &software, &own).unwrap();
        assert_eq!(actions(&r), ["0:encode mpeg4", "1:encode aac", "2:drop: not kept"]);
        let own_mjpeg = |c: &str| (c == "mjpeg").then(|| "mjpeg".to_string());
        let avi = MediaInfo { container: "avi".into(), streams: vec![video(0, "mjpeg")], ..mkv() };
        let r = resolve(&avi, None, &exact, &software, &own_mjpeg).unwrap();
        assert_eq!(actions(&r), ["0:encode mjpeg"]);
        let e = resolve(&mkv(), Some("mp4"), &exact, &none, &no_own).unwrap_err();
        assert!(e.to_string().contains("stream 0") && e.to_string().contains("h264_nvenc"), "{e}");
    }

    #[test]
    fn frame_and_estimates() {
        let op = MediaOp::Frame { at: 1.0, format: ImageFormat::Jpeg, max_px: None };
        let r = resolve(&mkv(), None, &op, &none, &no_own).unwrap();
        assert_eq!((r.container.as_str(), r.ext.as_str()), ("jpeg", "jpg"));
        assert_eq!(actions(&r)[0], "0:encode jpeg");
        assert_eq!(ImageFormat::Png.ext(), "png");
        // Copy: the streams' own bytes; a trim scales by the window.
        let r = resolve(&mkv(), None, &MediaOp::Remux { container: "mp4".into() }, &none, &no_own).unwrap();
        let full = estimate_bytes(&mkv(), &r).unwrap();
        assert_eq!(full, (4_000_000 + 128_000) * 60 / 8);
        let r =
            resolve(&mkv(), None, &MediaOp::Trim { start: 0.0, end: 30.0, stream_copy: true }, &none, &no_own).unwrap();
        // Matroska keeps the subtitle too; its unknown bit rate takes what the known
        // streams leave of the file, so the whole file halves.
        let half = estimate_bytes(&mkv(), &r).unwrap();
        assert_eq!(half, 40_000_000 / 2);
        // Encode: the option bitrate.
        let r =
            resolve(&mkv(), None, &MediaOp::ExtractAudio { codec: AudioCodecChoice::Aac }, &software, &no_own).unwrap();
        assert_eq!(estimate_bytes(&mkv(), &r), Some(192_000 * 60 / 8));
        assert_eq!(parse_bitrate("192k"), Some(192_000));
        assert_eq!(parse_bitrate("2M"), Some(2_000_000));
        assert_eq!(parse_bitrate("0"), Some(0));
        assert_eq!(parse_bitrate("x"), None);
        // Unknown bit rates share the file size.
        let unknown = MediaInfo {
            streams: vec![
                StreamInfo { bit_rate: None, ..video(0, "h264") },
                StreamInfo { bit_rate: None, ..audio(1, "aac") },
            ],
            ..mkv()
        };
        let r = resolve(&unknown, None, &MediaOp::Remux { container: "mp4".into() }, &none, &no_own).unwrap();
        assert_eq!(estimate_bytes(&unknown, &r), Some(40_000_000));
        assert_eq!(part_path(Path::new("/x/a.mp4")), PathBuf::from("/x/a.mp4.part"));
        assert_eq!(dest_ext(Path::new("/x/a.MKV")).as_deref(), Some("mkv"));
        assert_eq!(secs_to_ts(1.5, AVRational::new(1, 1000)), 1500);
    }
}
