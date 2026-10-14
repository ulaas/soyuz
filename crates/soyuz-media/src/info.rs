//! Container and stream metadata ([`probe`]).

use std::path::PathBuf;
use std::sync::Arc;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hdr {
    Hdr10,
    Hdr10Plus,
    Hlg,
    DolbyVision,
}

/// Colour description of a video stream (FFmpeg's names: `tv`, `bt709`, `smpte2084`, …).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ColorInfo {
    pub range: String,
    pub primaries: String,
    pub transfer: String,
    pub matrix: String,
}

/// One chapter (seconds).
#[derive(Clone, Debug, PartialEq)]
pub struct Chapter {
    pub start: f64,
    pub end: f64,
    pub title: Option<String>,
}

/// One stream of a container.
#[derive(Clone, Debug, PartialEq, Default)]
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
    /// Display rotation from the stream's display matrix: 0 / 90 / 180 / 270.
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
#[derive(Clone, Debug, PartialEq, Default)]
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
///
/// V0 stub: [`MediaError::NotLoaded`].
pub fn probe(rt: &Runtime, src: &Source) -> Result<MediaInfo, MediaError> {
    let _ = (rt, src);
    Err(MediaError::NotLoaded)
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
