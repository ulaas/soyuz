//! Struct layouts: `#[repr(C)]` *prefix* structs of the FFmpeg objects the crate reads,
//! and the per-major offset tables ([`Layout`]) for the one struct whose prefix differs
//! between the supported releases (`AVFrame`: 7.1 still has the deprecated `key_frame`,
//! `interlaced_frame`, `top_field_first`, `palette_has_changed`, `pkt_pos` and
//! `pkt_size` fields).
//!
//! Every struct here is a prefix: the real objects are allocated by FFmpeg (`av_*_alloc`)
//! and are longer; only the leading fields listed are ever read through these types, and
//! nothing is ever allocated from a Rust `size_of` of them. The declaration order was
//! taken from the 9.0 headers of the development runtime and from the `n8.1` / `n7.1`
//! tags of the FFmpeg repository (2026-10-14); `tests::expected_offsets` pins the
//! 64-bit offsets computed by hand from those headers, and `tests::layout_json` compares
//! the tables with `third_party/ffmpeg/layout_probe.c` output when
//! `SOYUZ_FFMPEG_LAYOUT=<file>` is set (CI).
//!
//! What is read where:
//! - `AVPacket`, `AVPacketSideData`, `AVFrameSideData`, `AVCodecParameters` (through
//!   `seek_preroll`), `AVStream` (through `attached_pic`), `AVFormatContext` (through
//!   `interrupt_callback`), `AVChapter`, `AVCodecDescriptor`, `AVInputFormat` (`name`,
//!   `long_name`), `AVIOContext` (`buffer`), `AVChannelLayout`, `AVRational`,
//!   `AVDictionaryEntry`: identical on 7.1 / 8.x / 9.0, one struct each.
//! - `AVFrame`: [`FrameOff`] per major, never read past `decode_error_flags`
//!   (`ch_layout` comes from the decoder's `ch_layout` AVOption instead).
//! - `AVCodecContext`: never read as a struct (AVOptions only).

use std::ffi::{c_char, c_int, c_uint, c_void};
use std::mem::offset_of;

use super::types::*;
use crate::Majors;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AVRational {
    pub num: c_int,
    pub den: c_int,
}

impl AVRational {
    pub(crate) const fn new(num: c_int, den: c_int) -> AVRational {
        AVRational { num, den }
    }

    /// `num / den` as a float, `None` when zero or invalid.
    pub(crate) fn to_f64(self) -> Option<f64> {
        (self.den != 0 && self.num != 0).then(|| f64::from(self.num) / f64::from(self.den))
    }

    /// `ts * self` in seconds (`None` for `AV_NOPTS_VALUE`).
    pub(crate) fn secs(self, ts: i64) -> Option<f64> {
        (ts != AV_NOPTS_VALUE && self.den != 0).then(|| ts as f64 * f64::from(self.num) / f64::from(self.den))
    }
}

/// `AVChannelLayout` (24 bytes): `order`, `nb_channels`, the `u` union (mask or map
/// pointer), `opaque`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct AVChannelLayout {
    pub order: c_int,
    pub nb_channels: c_int,
    pub u: u64,
    pub opaque: *mut c_void,
}

impl AVChannelLayout {
    pub(crate) const fn zeroed() -> AVChannelLayout {
        AVChannelLayout { order: 0, nb_channels: 0, u: 0, opaque: std::ptr::null_mut() }
    }
}

#[repr(C)]
pub(crate) struct AVDictionaryEntry {
    pub key: *mut c_char,
    pub value: *mut c_char,
}

#[repr(C)]
pub(crate) struct AVBufferRef {
    pub buffer: *mut AVBuffer,
    pub data: *mut u8,
    pub size: usize,
}

/// `AVPacket`: the whole struct is identical on 7.1 / 8.x / 9.0 (its size is still not
/// public ABI: packets are allocated with `av_packet_alloc`; `AVStream::attached_pic`
/// is embedded and only read).
#[repr(C)]
pub(crate) struct AVPacket {
    pub buf: *mut AVBufferRef,
    pub pts: i64,
    pub dts: i64,
    pub data: *mut u8,
    pub size: c_int,
    pub stream_index: c_int,
    pub flags: c_int,
    pub side_data: *mut AVPacketSideData,
    pub side_data_elems: c_int,
    pub duration: i64,
    pub pos: i64,
    pub opaque: *mut c_void,
    pub opaque_ref: *mut AVBufferRef,
    pub time_base: AVRational,
}

#[repr(C)]
pub(crate) struct AVPacketSideData {
    pub data: *mut u8,
    pub size: usize,
    pub r#type: c_int,
}

#[repr(C)]
pub(crate) struct AVFrameSideData {
    pub r#type: c_int,
    pub data: *mut u8,
    pub size: usize,
    pub metadata: *mut AVDictionary,
    pub buf: *mut AVBufferRef,
}

/// `AVCodecParameters` through `seek_preroll` (8.x / 9.0 append `alpha_mode`).
#[repr(C)]
pub(crate) struct AVCodecParameters {
    pub codec_type: c_int,
    pub codec_id: c_int,
    pub codec_tag: u32,
    pub extradata: *mut u8,
    pub extradata_size: c_int,
    pub coded_side_data: *mut AVPacketSideData,
    pub nb_coded_side_data: c_int,
    pub format: c_int,
    pub bit_rate: i64,
    pub bits_per_coded_sample: c_int,
    pub bits_per_raw_sample: c_int,
    pub profile: c_int,
    pub level: c_int,
    pub width: c_int,
    pub height: c_int,
    pub sample_aspect_ratio: AVRational,
    pub framerate: AVRational,
    pub field_order: c_int,
    pub color_range: c_int,
    pub color_primaries: c_int,
    pub color_trc: c_int,
    pub color_space: c_int,
    pub chroma_location: c_int,
    pub video_delay: c_int,
    pub ch_layout: AVChannelLayout,
    pub sample_rate: c_int,
    pub block_align: c_int,
    pub frame_size: c_int,
    pub initial_padding: c_int,
    pub trailing_padding: c_int,
    pub seek_preroll: c_int,
}

/// `AVStream` through `attached_pic` (7.1 has deprecated side-data fields after it, so
/// `event_flags`, `r_frame_rate` and `pts_wrap_bits` are never read; frame rates come
/// from `av_guess_frame_rate`).
#[repr(C)]
pub(crate) struct AVStream {
    pub av_class: *const AVClass,
    pub index: c_int,
    pub id: c_int,
    pub codecpar: *mut AVCodecParameters,
    pub priv_data: *mut c_void,
    pub time_base: AVRational,
    pub start_time: i64,
    pub duration: i64,
    pub nb_frames: i64,
    pub disposition: c_int,
    pub discard: c_int,
    pub sample_aspect_ratio: AVRational,
    pub metadata: *mut AVDictionary,
    pub avg_frame_rate: AVRational,
    pub attached_pic: AVPacket,
}

#[repr(C)]
pub(crate) struct AVIOInterruptCB {
    pub callback: Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
    pub opaque: *mut c_void,
}

/// `AVFormatContext` through `interrupt_callback` (identical on 7.1 / 8.x / 9.0).
/// Written: `pb` (before `avformat_open_input`). Everything else is read only;
/// `probesize`, `max_analyze_duration` and `max_streams` are set through AVOptions.
#[repr(C)]
pub(crate) struct AVFormatContext {
    pub av_class: *const AVClass,
    pub iformat: *const AVInputFormat,
    pub oformat: *const AVOutputFormat,
    pub priv_data: *mut c_void,
    pub pb: *mut AVIOContext,
    pub ctx_flags: c_int,
    pub nb_streams: c_uint,
    pub streams: *mut *mut AVStream,
    pub nb_stream_groups: c_uint,
    pub stream_groups: *mut *mut AVStreamGroup,
    pub nb_chapters: c_uint,
    pub chapters: *mut *mut AVChapter,
    pub url: *mut c_char,
    pub start_time: i64,
    pub duration: i64,
    pub bit_rate: i64,
    pub packet_size: c_uint,
    pub max_delay: c_int,
    pub flags: c_int,
    pub probesize: i64,
    pub max_analyze_duration: i64,
    pub key: *const u8,
    pub keylen: c_int,
    pub nb_programs: c_uint,
    pub programs: *mut *mut AVProgram,
    pub video_codec_id: c_int,
    pub audio_codec_id: c_int,
    pub subtitle_codec_id: c_int,
    pub data_codec_id: c_int,
    pub metadata: *mut AVDictionary,
    pub start_time_realtime: i64,
    pub fps_probe_size: c_int,
    pub error_recognition: c_int,
    pub interrupt_callback: AVIOInterruptCB,
}

#[repr(C)]
pub(crate) struct AVChapter {
    pub id: i64,
    pub time_base: AVRational,
    pub start: i64,
    pub end: i64,
    pub metadata: *mut AVDictionary,
}

#[repr(C)]
pub(crate) struct AVCodecDescriptor {
    pub id: c_int,
    pub r#type: c_int,
    pub name: *const c_char,
    pub long_name: *const c_char,
    pub props: c_int,
    pub mime_types: *const *const c_char,
    pub profiles: *const AVProfile,
}

/// `AVCodec` prefix: `name`, `long_name`, `type`, `id`, `capabilities` (identical on
/// 7.1 / 8.x / 9.0; the deprecated format lists after `max_lowres` are never read).
#[repr(C)]
pub(crate) struct AVCodecPrefix {
    pub name: *const c_char,
    pub long_name: *const c_char,
    pub r#type: c_int,
    pub id: c_int,
    pub capabilities: c_int,
    pub max_lowres: u8,
}

/// `AVInputFormat`: `name`, `long_name`, `flags` (the rest is private to lavf).
#[repr(C)]
pub(crate) struct AVInputFormat {
    pub name: *const c_char,
    pub long_name: *const c_char,
    pub flags: c_int,
}

/// `AVIOContext` prefix: `buffer` is read when freeing a custom context (FFmpeg may
/// have replaced the buffer we allocated).
#[repr(C)]
pub(crate) struct AVIOContextPrefix {
    pub av_class: *const AVClass,
    pub buffer: *mut u8,
    pub buffer_size: c_int,
}

/// `AVFrame` prefix of 8.x / 9.0 (through `decode_error_flags`).
#[repr(C)]
pub(crate) struct AVFrame8 {
    pub data: [*mut u8; 8],
    pub linesize: [c_int; 8],
    pub extended_data: *mut *mut u8,
    pub width: c_int,
    pub height: c_int,
    pub nb_samples: c_int,
    pub format: c_int,
    pub pict_type: c_int,
    pub sample_aspect_ratio: AVRational,
    pub pts: i64,
    pub pkt_dts: i64,
    pub time_base: AVRational,
    pub quality: c_int,
    pub opaque: *mut c_void,
    pub repeat_pict: c_int,
    pub sample_rate: c_int,
    pub buf: [*mut AVBufferRef; 8],
    pub extended_buf: *mut *mut AVBufferRef,
    pub nb_extended_buf: c_int,
    pub side_data: *mut *mut AVFrameSideData,
    pub nb_side_data: c_int,
    pub flags: c_int,
    pub color_range: c_int,
    pub color_primaries: c_int,
    pub color_trc: c_int,
    pub colorspace: c_int,
    pub chroma_location: c_int,
    pub best_effort_timestamp: i64,
    pub metadata: *mut AVDictionary,
    pub decode_error_flags: c_int,
}

/// `AVFrame` prefix of 7.1 (`FF_API_FRAME_KEY`, `FF_API_INTERLACED_FRAME`,
/// `FF_API_PALETTE_HAS_CHANGED`, `FF_API_FRAME_PKT` are all on while avutil < 60).
#[repr(C)]
pub(crate) struct AVFrame7 {
    pub data: [*mut u8; 8],
    pub linesize: [c_int; 8],
    pub extended_data: *mut *mut u8,
    pub width: c_int,
    pub height: c_int,
    pub nb_samples: c_int,
    pub format: c_int,
    pub key_frame: c_int,
    pub pict_type: c_int,
    pub sample_aspect_ratio: AVRational,
    pub pts: i64,
    pub pkt_dts: i64,
    pub time_base: AVRational,
    pub quality: c_int,
    pub opaque: *mut c_void,
    pub repeat_pict: c_int,
    pub interlaced_frame: c_int,
    pub top_field_first: c_int,
    pub palette_has_changed: c_int,
    pub sample_rate: c_int,
    pub buf: [*mut AVBufferRef; 8],
    pub extended_buf: *mut *mut AVBufferRef,
    pub nb_extended_buf: c_int,
    pub side_data: *mut *mut AVFrameSideData,
    pub nb_side_data: c_int,
    pub flags: c_int,
    pub color_range: c_int,
    pub color_primaries: c_int,
    pub color_trc: c_int,
    pub colorspace: c_int,
    pub chroma_location: c_int,
    pub best_effort_timestamp: i64,
    pub pkt_pos: i64,
    pub metadata: *mut AVDictionary,
    pub decode_error_flags: c_int,
    pub pkt_size: c_int,
}

/// Byte offsets of the `AVFrame` fields the crate reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameOff {
    pub data: usize,
    pub linesize: usize,
    pub extended_data: usize,
    pub width: usize,
    pub height: usize,
    pub nb_samples: usize,
    pub format: usize,
    pub pict_type: usize,
    pub sample_aspect_ratio: usize,
    pub pts: usize,
    pub pkt_dts: usize,
    pub time_base: usize,
    pub sample_rate: usize,
    pub flags: usize,
    pub color_range: usize,
    pub color_primaries: usize,
    pub color_trc: usize,
    pub colorspace: usize,
    pub chroma_location: usize,
    pub best_effort_timestamp: usize,
    pub metadata: usize,
    pub decode_error_flags: usize,
}

macro_rules! frame_off {
    ($t:ty) => {
        FrameOff {
            data: offset_of!($t, data),
            linesize: offset_of!($t, linesize),
            extended_data: offset_of!($t, extended_data),
            width: offset_of!($t, width),
            height: offset_of!($t, height),
            nb_samples: offset_of!($t, nb_samples),
            format: offset_of!($t, format),
            pict_type: offset_of!($t, pict_type),
            sample_aspect_ratio: offset_of!($t, sample_aspect_ratio),
            pts: offset_of!($t, pts),
            pkt_dts: offset_of!($t, pkt_dts),
            time_base: offset_of!($t, time_base),
            sample_rate: offset_of!($t, sample_rate),
            flags: offset_of!($t, flags),
            color_range: offset_of!($t, color_range),
            color_primaries: offset_of!($t, color_primaries),
            color_trc: offset_of!($t, color_trc),
            colorspace: offset_of!($t, colorspace),
            chroma_location: offset_of!($t, chroma_location),
            best_effort_timestamp: offset_of!($t, best_effort_timestamp),
            metadata: offset_of!($t, metadata),
            decode_error_flags: offset_of!($t, decode_error_flags),
        }
    };
}

/// The layout of one supported release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    pub majors: Majors,
    pub frame: FrameOff,
}

pub(crate) const V9: Layout = Layout { majors: Majors::V9, frame: frame_off!(AVFrame8) };
pub(crate) const V8: Layout = Layout { majors: Majors::V8, frame: frame_off!(AVFrame8) };
pub(crate) const V7: Layout = Layout { majors: Majors::V7, frame: frame_off!(AVFrame7) };

/// The layout of a supported row of majors.
pub(crate) fn for_majors(m: Majors) -> Option<&'static Layout> {
    match m {
        Majors::V9 => Some(&V9),
        Majors::V8 => Some(&V8),
        Majors::V7 => Some(&V7),
        _ => None,
    }
}

#[cfg(test)]
impl FrameOff {
    /// Every `(field, offset)` pair, for the layout tests.
    pub(crate) fn fields(&self) -> Vec<(&'static str, usize)> {
        vec![
            ("data", self.data),
            ("linesize", self.linesize),
            ("extended_data", self.extended_data),
            ("width", self.width),
            ("height", self.height),
            ("nb_samples", self.nb_samples),
            ("format", self.format),
            ("pict_type", self.pict_type),
            ("sample_aspect_ratio", self.sample_aspect_ratio),
            ("pts", self.pts),
            ("pkt_dts", self.pkt_dts),
            ("time_base", self.time_base),
            ("sample_rate", self.sample_rate),
            ("flags", self.flags),
            ("color_range", self.color_range),
            ("color_primaries", self.color_primaries),
            ("color_trc", self.color_trc),
            ("colorspace", self.colorspace),
            ("chroma_location", self.chroma_location),
            ("best_effort_timestamp", self.best_effort_timestamp),
            ("metadata", self.metadata),
            ("decode_error_flags", self.decode_error_flags),
        ]
    }
}

/// `(struct, field, offset)` of every field of the version-independent prefix structs,
/// for the layout tests (`tests::expected_offsets` and the CI probe comparison).
#[cfg(test)]
pub(crate) fn common_fields() -> Vec<(&'static str, &'static str, usize)> {
    macro_rules! rows {
        ($($st:ident { $($f:ident),* $(,)? })*) => {
            vec![$($((stringify!($st), stringify!($f), offset_of!($st, $f)),)*)*]
        };
    }
    let mut v = rows! {
        AVPacket { buf, pts, dts, data, size, stream_index, flags, side_data, side_data_elems, duration, pos, opaque, opaque_ref, time_base }
        AVPacketSideData { data, size, r#type }
        AVFrameSideData { r#type, data, size, metadata, buf }
        AVCodecParameters { codec_type, codec_id, codec_tag, extradata, extradata_size, coded_side_data, nb_coded_side_data, format, bit_rate, bits_per_coded_sample, bits_per_raw_sample, profile, level, width, height, sample_aspect_ratio, framerate, field_order, color_range, color_primaries, color_trc, color_space, chroma_location, video_delay, ch_layout, sample_rate, block_align, frame_size, initial_padding, trailing_padding, seek_preroll }
        AVStream { av_class, index, id, codecpar, priv_data, time_base, start_time, duration, nb_frames, disposition, discard, sample_aspect_ratio, metadata, avg_frame_rate, attached_pic }
        AVFormatContext { av_class, iformat, oformat, priv_data, pb, ctx_flags, nb_streams, streams, nb_stream_groups, stream_groups, nb_chapters, chapters, url, start_time, duration, bit_rate, packet_size, max_delay, flags, probesize, max_analyze_duration, key, keylen, nb_programs, programs, video_codec_id, audio_codec_id, subtitle_codec_id, data_codec_id, metadata, start_time_realtime, fps_probe_size, error_recognition, interrupt_callback }
        AVChapter { id, time_base, start, end, metadata }
        AVCodecPrefix { name, long_name, r#type, id, capabilities }
    };
    // `r#type` must compare as `type`; the probe calls the AVCodec prefix "AVCodec".
    for row in &mut v {
        if row.1 == "r#type" {
            row.1 = "type";
        }
        if row.0 == "AVCodecPrefix" {
            row.0 = "AVCodec";
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Offsets computed by hand from the headers (LP64 / LLP64: pointers 8, `int` 4,
    /// `int64_t` 8, enums 4, `size_t` 8, `AVRational` 8 aligned 4, `AVChannelLayout`
    /// 24 aligned 8, `AVPacket` 104 aligned 8).
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn expected_offsets() {
        let frame8: &[(&str, usize)] = &[
            ("data", 0),
            ("linesize", 64),
            ("extended_data", 96),
            ("width", 104),
            ("height", 108),
            ("nb_samples", 112),
            ("format", 116),
            ("pict_type", 120),
            ("sample_aspect_ratio", 124),
            ("pts", 136),
            ("pkt_dts", 144),
            ("time_base", 152),
            ("sample_rate", 180),
            ("flags", 276),
            ("color_range", 280),
            ("color_primaries", 284),
            ("color_trc", 288),
            ("colorspace", 292),
            ("chroma_location", 296),
            ("best_effort_timestamp", 304),
            ("metadata", 312),
            ("decode_error_flags", 320),
        ];
        let frame7: &[(&str, usize)] = &[
            ("data", 0),
            ("linesize", 64),
            ("extended_data", 96),
            ("width", 104),
            ("height", 108),
            ("nb_samples", 112),
            ("format", 116),
            ("pict_type", 124),
            ("sample_aspect_ratio", 128),
            ("pts", 136),
            ("pkt_dts", 144),
            ("time_base", 152),
            ("sample_rate", 192),
            ("flags", 292),
            ("color_range", 296),
            ("color_primaries", 300),
            ("color_trc", 304),
            ("colorspace", 308),
            ("chroma_location", 312),
            ("best_effort_timestamp", 320),
            ("metadata", 336),
            ("decode_error_flags", 344),
        ];
        assert_eq!(V9.frame.fields(), frame8, "AVFrame 9.0");
        assert_eq!(V8.frame.fields(), frame8, "AVFrame 8.x");
        assert_eq!(V7.frame.fields(), frame7, "AVFrame 7.1");
        // The 7.1-only fields sit where the headers put them.
        assert_eq!(offset_of!(AVFrame7, key_frame), 120);
        assert_eq!(offset_of!(AVFrame7, interlaced_frame), 180);
        assert_eq!(offset_of!(AVFrame7, palette_has_changed), 188);
        assert_eq!(offset_of!(AVFrame7, pkt_pos), 328);
        assert_eq!(offset_of!(AVFrame7, pkt_size), 348);
        // 7.1's tail (`hw_frames_ctx` onwards) is 8 bytes later than 8.x: never read it.
        assert_eq!(size_of::<AVFrame8>(), 328);
        assert_eq!(size_of::<AVFrame7>(), 352);

        let common: &[(&str, &str, usize)] = &[
            ("AVPacket", "buf", 0),
            ("AVPacket", "pts", 8),
            ("AVPacket", "dts", 16),
            ("AVPacket", "data", 24),
            ("AVPacket", "size", 32),
            ("AVPacket", "stream_index", 36),
            ("AVPacket", "flags", 40),
            ("AVPacket", "side_data", 48),
            ("AVPacket", "side_data_elems", 56),
            ("AVPacket", "duration", 64),
            ("AVPacket", "pos", 72),
            ("AVPacket", "opaque", 80),
            ("AVPacket", "opaque_ref", 88),
            ("AVPacket", "time_base", 96),
            ("AVPacketSideData", "data", 0),
            ("AVPacketSideData", "size", 8),
            ("AVPacketSideData", "type", 16),
            ("AVFrameSideData", "type", 0),
            ("AVFrameSideData", "data", 8),
            ("AVFrameSideData", "size", 16),
            ("AVFrameSideData", "metadata", 24),
            ("AVFrameSideData", "buf", 32),
            ("AVCodecParameters", "codec_type", 0),
            ("AVCodecParameters", "codec_id", 4),
            ("AVCodecParameters", "codec_tag", 8),
            ("AVCodecParameters", "extradata", 16),
            ("AVCodecParameters", "extradata_size", 24),
            ("AVCodecParameters", "coded_side_data", 32),
            ("AVCodecParameters", "nb_coded_side_data", 40),
            ("AVCodecParameters", "format", 44),
            ("AVCodecParameters", "bit_rate", 48),
            ("AVCodecParameters", "bits_per_coded_sample", 56),
            ("AVCodecParameters", "bits_per_raw_sample", 60),
            ("AVCodecParameters", "profile", 64),
            ("AVCodecParameters", "level", 68),
            ("AVCodecParameters", "width", 72),
            ("AVCodecParameters", "height", 76),
            ("AVCodecParameters", "sample_aspect_ratio", 80),
            ("AVCodecParameters", "framerate", 88),
            ("AVCodecParameters", "field_order", 96),
            ("AVCodecParameters", "color_range", 100),
            ("AVCodecParameters", "color_primaries", 104),
            ("AVCodecParameters", "color_trc", 108),
            ("AVCodecParameters", "color_space", 112),
            ("AVCodecParameters", "chroma_location", 116),
            ("AVCodecParameters", "video_delay", 120),
            ("AVCodecParameters", "ch_layout", 128),
            ("AVCodecParameters", "sample_rate", 152),
            ("AVCodecParameters", "block_align", 156),
            ("AVCodecParameters", "frame_size", 160),
            ("AVCodecParameters", "initial_padding", 164),
            ("AVCodecParameters", "trailing_padding", 168),
            ("AVCodecParameters", "seek_preroll", 172),
            ("AVStream", "av_class", 0),
            ("AVStream", "index", 8),
            ("AVStream", "id", 12),
            ("AVStream", "codecpar", 16),
            ("AVStream", "priv_data", 24),
            ("AVStream", "time_base", 32),
            ("AVStream", "start_time", 40),
            ("AVStream", "duration", 48),
            ("AVStream", "nb_frames", 56),
            ("AVStream", "disposition", 64),
            ("AVStream", "discard", 68),
            ("AVStream", "sample_aspect_ratio", 72),
            ("AVStream", "metadata", 80),
            ("AVStream", "avg_frame_rate", 88),
            ("AVStream", "attached_pic", 96),
            ("AVFormatContext", "av_class", 0),
            ("AVFormatContext", "iformat", 8),
            ("AVFormatContext", "oformat", 16),
            ("AVFormatContext", "priv_data", 24),
            ("AVFormatContext", "pb", 32),
            ("AVFormatContext", "ctx_flags", 40),
            ("AVFormatContext", "nb_streams", 44),
            ("AVFormatContext", "streams", 48),
            ("AVFormatContext", "nb_stream_groups", 56),
            ("AVFormatContext", "stream_groups", 64),
            ("AVFormatContext", "nb_chapters", 72),
            ("AVFormatContext", "chapters", 80),
            ("AVFormatContext", "url", 88),
            ("AVFormatContext", "start_time", 96),
            ("AVFormatContext", "duration", 104),
            ("AVFormatContext", "bit_rate", 112),
            ("AVFormatContext", "packet_size", 120),
            ("AVFormatContext", "max_delay", 124),
            ("AVFormatContext", "flags", 128),
            ("AVFormatContext", "probesize", 136),
            ("AVFormatContext", "max_analyze_duration", 144),
            ("AVFormatContext", "key", 152),
            ("AVFormatContext", "keylen", 160),
            ("AVFormatContext", "nb_programs", 164),
            ("AVFormatContext", "programs", 168),
            ("AVFormatContext", "video_codec_id", 176),
            ("AVFormatContext", "audio_codec_id", 180),
            ("AVFormatContext", "subtitle_codec_id", 184),
            ("AVFormatContext", "data_codec_id", 188),
            ("AVFormatContext", "metadata", 192),
            ("AVFormatContext", "start_time_realtime", 200),
            ("AVFormatContext", "fps_probe_size", 208),
            ("AVFormatContext", "error_recognition", 212),
            ("AVFormatContext", "interrupt_callback", 216),
            ("AVChapter", "id", 0),
            ("AVChapter", "time_base", 8),
            ("AVChapter", "start", 16),
            ("AVChapter", "end", 24),
            ("AVChapter", "metadata", 32),
            ("AVCodec", "name", 0),
            ("AVCodec", "long_name", 8),
            ("AVCodec", "type", 16),
            ("AVCodec", "id", 20),
            ("AVCodec", "capabilities", 24),
        ];
        assert_eq!(common_fields(), common);
        assert_eq!(size_of::<AVPacket>(), 104);
        assert_eq!(size_of::<AVChannelLayout>(), 24);
        assert_eq!(size_of::<AVStream>(), 200, "attached_pic ends the prefix");
        assert_eq!(size_of::<AVFormatContext>(), 232);
        assert_eq!(offset_of!(AVCodecDescriptor, long_name), 16);
        assert_eq!(offset_of!(AVCodecDescriptor, profiles), 40);
        assert_eq!(offset_of!(AVInputFormat, long_name), 8);
        assert_eq!(offset_of!(AVCodecPrefix, r#type), 16);
        assert_eq!(offset_of!(AVCodecPrefix, id), 20);
        assert_eq!(offset_of!(AVCodecPrefix, capabilities), 24);
        assert_eq!(offset_of!(AVCodecPrefix, max_lowres), 28);
        assert_eq!(offset_of!(AVIOContextPrefix, buffer), 8);
        assert_eq!(offset_of!(AVIOContextPrefix, buffer_size), 16);
    }

    #[test]
    fn tables_are_increasing_and_aligned() {
        for l in [&V9, &V8, &V7] {
            let f = l.frame.fields();
            for w in f.windows(2) {
                assert!(w[1].1 > w[0].1, "{:?}: {} after {}", l.majors, w[1].0, w[0].0);
            }
            assert_eq!(l.frame.pts % 8, 0);
            assert_eq!(l.frame.best_effort_timestamp % 8, 0);
            assert_eq!(l.frame.metadata % size_of::<usize>(), 0);
        }
        assert_eq!(V9.frame, V8.frame, "8.x and 9.0 share the AVFrame prefix");
        assert_ne!(V7.frame, V8.frame);
        assert_eq!(for_majors(Majors::V7).map(|l| l.majors), Some(Majors::V7));
        assert!(for_majors(Majors { avcodec: 60, ..Majors::V8 }).is_none());
    }

    /// Compare with `third_party/ffmpeg/layout_probe.c` output (one JSON object per
    /// line) when `SOYUZ_FFMPEG_LAYOUT` names the file; the file's own `majors` line
    /// selects the table.
    #[test]
    fn layout_json() {
        let Some(path) = std::env::var_os("SOYUZ_FFMPEG_LAYOUT") else {
            eprintln!("skipped: SOYUZ_FFMPEG_LAYOUT not set");
            return;
        };
        let text = std::fs::read_to_string(&path).expect("read SOYUZ_FFMPEG_LAYOUT");
        let mut rows: Vec<(String, String, usize)> = Vec::new();
        let mut majors: Option<Majors> = None;
        for line in text.lines() {
            let get = |key: &str| -> Option<String> {
                let needle = format!("\"{key}\":");
                let rest = &line[line.find(&needle)? + needle.len()..];
                let rest = rest.trim_start();
                let end = rest.find([',', '}']).unwrap_or(rest.len());
                Some(rest[..end].trim().trim_matches('"').to_string())
            };
            if line.contains("\"majors\"") {
                let num = |k: &str| get(k).and_then(|v| v.parse::<u32>().ok()).expect("major");
                majors = Some(Majors {
                    avutil: num("avutil"),
                    swresample: num("swresample"),
                    swscale: num("swscale"),
                    avcodec: num("avcodec"),
                    avformat: num("avformat"),
                });
            } else if let (Some(s), Some(f), Some(o)) = (get("struct"), get("field"), get("offset")) {
                rows.push((s, f, o.parse().expect("offset")));
            }
        }
        let majors = majors.expect("a majors line");
        let layout = for_majors(majors).unwrap_or_else(|| panic!("unsupported majors {majors:?}"));
        let probed = |s: &str, f: &str| -> usize {
            rows.iter().find(|(ps, pf, _)| ps == s && pf == f).map(|r| r.2).unwrap_or_else(|| panic!("{s}.{f} missing"))
        };
        for (f, off) in layout.frame.fields() {
            assert_eq!(off, probed("AVFrame", f), "AVFrame.{f} for {majors:?}");
        }
        // The AVCodec rows are optional in the probe (older layout_probe.c builds print
        // none): checked only when present.
        let has_codec_rows = rows.iter().any(|(s, _, _)| s == "AVCodec");
        for (s, f, off) in common_fields() {
            if s == "AVCodec" && !has_codec_rows {
                continue;
            }
            assert_eq!(off, probed(s, f), "{s}.{f} for {majors:?}");
        }
    }
}
