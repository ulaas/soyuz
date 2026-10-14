//! Container and stream metadata ([`probe`]).

use std::path::PathBuf;
use std::sync::Arc;

use crate::decode::{Demuxer, Limits, StreamRef};
use crate::sys::{self, codec, types};
use crate::{MediaError, Runtime};

/// What to open. Every source goes through the crate's own `AVIOContext` (no FFmpeg
/// protocol parsing of user paths); `Mem` serves archive entries and remote downloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    File(PathBuf),
    Mem(Arc<[u8]>),
}

impl Source {
    /// The path of a `File` source (labels, cache keys); `None` for memory.
    pub fn path(&self) -> Option<&std::path::Path> {
        match self {
            Source::File(p) => Some(p),
            Source::Mem(_) => None,
        }
    }
}

/// `AVMediaType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    Video,
    Audio,
    Subtitle,
    Data,
    Attachment,
}

impl std::fmt::Display for StreamKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            StreamKind::Video => "video",
            StreamKind::Audio => "audio",
            StreamKind::Subtitle => "subtitle",
            StreamKind::Data => "data",
            StreamKind::Attachment => "attachment",
        })
    }
}

/// High dynamic range signalling of a video stream (`color_trc` plus the mastering /
/// content-light side data; Dolby Vision from its configuration record).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hdr {
    Hdr10,
    Hdr10Plus,
    Hlg,
    DolbyVision,
}

/// Colour description of a video stream (FFmpeg's names: `tv`, `bt709`, `smpte2084`, …).
#[derive(Clone, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ColorInfo {
    pub range: String,
    pub primaries: String,
    pub transfer: String,
    pub matrix: String,
}

/// One chapter (seconds).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Chapter {
    pub start: f64,
    pub end: f64,
    pub title: Option<String>,
}

/// One stream of a container.
#[derive(Clone, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct StreamInfo {
    pub index: usize,
    pub kind: Option<StreamKind>,
    /// Codec name (`h264`) and long name (`H.264 / AVC / MPEG-4 AVC …`).
    pub codec: String,
    pub codec_long: String,
    pub profile: Option<String>,
    pub bit_rate: Option<u64>,
    // Video.
    pub width: u32,
    pub height: u32,
    /// Sample aspect ratio (num, den); `(0, 0)` = unknown.
    pub sar: (i32, i32),
    /// Clockwise rotation (0 / 90 / 180 / 270) to apply for display, from the stream's
    /// display matrix (what `ffmpeg`'s autorotate applies; a phone's portrait clip says
    /// 90).
    pub rotation: i32,
    pub fps: Option<f64>,
    pub pix_fmt: Option<String>,
    pub hdr: Option<Hdr>,
    pub color: Option<ColorInfo>,
    // Audio.
    pub sample_rate: u32,
    pub channels: u32,
    pub channel_layout: String,
    pub sample_fmt: Option<String>,
    // Common.
    pub language: Option<String>,
    pub title: Option<String>,
    pub default: bool,
    pub forced: bool,
    /// The stream is cover art (`AV_DISPOSITION_ATTACHED_PIC`).
    pub attached_pic: bool,
    pub tags: Vec<(String, String)>,
}

/// Everything [`probe`] reads without decoding.
#[derive(Clone, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct MediaInfo {
    /// Demuxer name (`mov,mp4,m4a,3gp,3g2,mj2`) and its long name.
    pub container: String,
    pub container_long: String,
    /// Seconds; `None` when the container does not say.
    pub duration: Option<f64>,
    /// Start time (seconds).
    pub start: f64,
    pub bit_rate: Option<u64>,
    /// Bytes.
    pub size: u64,
    pub streams: Vec<StreamInfo>,
    pub chapters: Vec<Chapter>,
    /// Container-level tags (`title`, `artist`, `encoder`, …), in file order.
    pub tags: Vec<(String, String)>,
    /// The attached picture (cover art) as stored (PNG / JPEG bytes).
    pub cover: Option<Vec<u8>>,
}

impl MediaInfo {
    /// The first video stream that is not cover art.
    pub fn video(&self) -> Option<&StreamInfo> {
        self.streams.iter().find(|s| s.kind == Some(StreamKind::Video) && !s.attached_pic)
    }

    /// The first (default) audio stream.
    pub fn audio(&self) -> Option<&StreamInfo> {
        let audio = || self.streams.iter().filter(|s| s.kind == Some(StreamKind::Audio));
        audio().find(|s| s.default).or_else(|| audio().next())
    }

    /// A tag by case-insensitive key.
    pub fn tag(&self, key: &str) -> Option<&str> {
        self.tags.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }
}

/// Container, streams, chapters, tags and cover art of `src` (no decoding; `cover` is
/// the stored picture bytes). Limits (`probesize`, `max_analyze_duration`, `max_streams`)
/// are set before the file is opened.
pub fn probe(rt: &Runtime, src: &Source) -> Result<MediaInfo, MediaError> {
    let av = rt.av();
    let d = Demuxer::open(av, src, None, Limits::default())?;
    let (container, container_long) = d.format();
    let mut info = MediaInfo {
        container,
        container_long,
        duration: d.duration(),
        start: d.start_time(),
        bit_rate: d.bit_rate(),
        size: d.size,
        streams: Vec::with_capacity(d.stream_count()),
        chapters: d.chapters().into_iter().map(|(start, end, title)| Chapter { start, end, title }).collect(),
        tags: d.metadata(),
        cover: None,
    };
    for s in d.streams() {
        let si = stream_info(av, &d, s);
        if si.attached_pic && info.cover.is_none() {
            info.cover = s.attached_pic();
        }
        info.streams.push(si);
    }
    Ok(info)
}

/// The [`StreamInfo`] of one stream of an open demuxer.
pub(crate) fn stream_info(av: &'static sys::Av, d: &Demuxer<'_>, s: StreamRef<'_>) -> StreamInfo {
    let par = s.par();
    let kind = s.kind();
    let disposition = s.disposition();
    let mut si = StreamInfo {
        index: s.index(),
        kind,
        codec: codec::codec_name(av, par.codec_id),
        codec_long: codec::codec_long_name(av, par.codec_id),
        profile: codec::profile_name(av, par.codec_id, par.profile),
        bit_rate: u64::try_from(par.bit_rate).ok().filter(|b| *b > 0),
        language: s.tag("language"),
        title: s.tag("title"),
        default: disposition & types::AV_DISPOSITION_DEFAULT != 0,
        forced: disposition & types::AV_DISPOSITION_FORCED != 0,
        attached_pic: disposition & types::AV_DISPOSITION_ATTACHED_PIC != 0,
        tags: s.metadata(),
        ..Default::default()
    };
    match kind {
        Some(StreamKind::Video) => {
            si.width = u32::try_from(par.width).unwrap_or(0);
            si.height = u32::try_from(par.height).unwrap_or(0);
            let sar = d.guess_sar(s);
            let sar = if sar.num > 0 && sar.den > 0 { sar } else { par.sample_aspect_ratio };
            si.sar = if sar.num > 0 && sar.den > 0 { (sar.num, sar.den) } else { (0, 0) };
            si.rotation = s.side_data(types::AV_PKT_DATA_DISPLAYMATRIX).map_or(0, |m| codec::display_rotation(av, m));
            si.fps = d.guess_frame_rate(s).to_f64().filter(|f| *f > 0.0);
            si.pix_fmt = codec::pix_fmt_name(av, par.format);
            let [range, primaries, transfer, matrix] =
                codec::color_names(av, par.color_range, par.color_primaries, par.color_trc, par.color_space);
            let known = par.color_range != types::AVCOL_RANGE_UNSPECIFIED
                || par.color_primaries != types::AVCOL_UNSPECIFIED
                || par.color_trc != types::AVCOL_UNSPECIFIED
                || par.color_space != types::AVCOL_UNSPECIFIED;
            if known {
                let name = |v: Option<String>, unspecified: bool| {
                    if unspecified { String::new() } else { v.unwrap_or_default() }
                };
                si.color = Some(ColorInfo {
                    range: name(range, par.color_range == types::AVCOL_RANGE_UNSPECIFIED),
                    primaries: name(primaries, par.color_primaries == types::AVCOL_UNSPECIFIED),
                    transfer: name(transfer, par.color_trc == types::AVCOL_UNSPECIFIED),
                    matrix: name(matrix, par.color_space == types::AVCOL_UNSPECIFIED),
                });
            }
            si.hdr = hdr_of(s, par.color_trc);
        }
        Some(StreamKind::Audio) => {
            si.sample_rate = u32::try_from(par.sample_rate).unwrap_or(0);
            si.channels = u32::try_from(par.ch_layout.nb_channels).unwrap_or(0);
            si.channel_layout = codec::describe_layout(av, &par.ch_layout);
            si.sample_fmt = codec::sample_fmt_name(av, par.format);
        }
        _ => {}
    }
    si
}

/// HDR signalling: Dolby Vision configuration first, then the transfer function
/// (PQ = HDR10, HDR10+ when its dynamic metadata is present; HLG), then the static
/// mastering / content-light metadata alone.
fn hdr_of(s: StreamRef<'_>, trc: std::ffi::c_int) -> Option<Hdr> {
    if s.side_data(types::AV_PKT_DATA_DOVI_CONF).is_some() {
        return Some(Hdr::DolbyVision);
    }
    match trc {
        types::AVCOL_TRC_SMPTE2084 => {
            Some(if s.side_data(types::AV_PKT_DATA_DYNAMIC_HDR10_PLUS).is_some() { Hdr::Hdr10Plus } else { Hdr::Hdr10 })
        }
        types::AVCOL_TRC_ARIB_STD_B67 => Some(Hdr::Hlg),
        _ => (s.side_data(types::AV_PKT_DATA_MASTERING_DISPLAY_METADATA).is_some()
            && s.side_data(types::AV_PKT_DATA_CONTENT_LIGHT_LEVEL).is_some())
        .then_some(Hdr::Hdr10),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_pickers() {
        let v = |index, attached_pic| StreamInfo {
            index,
            kind: Some(StreamKind::Video),
            attached_pic,
            ..Default::default()
        };
        let a = |index, default| StreamInfo { index, kind: Some(StreamKind::Audio), default, ..Default::default() };
        let info = MediaInfo {
            streams: vec![v(0, true), a(1, false), v(2, false), a(3, true)],
            tags: vec![("Title".into(), "x".into())],
            ..Default::default()
        };
        assert_eq!(info.video().map(|s| s.index), Some(2), "cover art is not the video");
        assert_eq!(info.audio().map(|s| s.index), Some(3), "the default audio stream wins");
        assert_eq!(info.tag("title"), Some("x"));
        assert_eq!(MediaInfo::default().video(), None);
        assert_eq!(Source::File("a.mp4".into()).path(), Some(std::path::Path::new("a.mp4")));
        assert_eq!(Source::Mem(Arc::from(&b"x"[..])).path(), None);
        assert_eq!(StreamKind::Subtitle.to_string(), "subtitle");
    }
}
