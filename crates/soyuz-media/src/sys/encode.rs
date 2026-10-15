//! Encoding (V2c): an open encoder ([`Encoder`]), frames the transcoder fills itself
//! ([`new_video_frame`], [`new_audio_frame`], [`set_pts`]), and conversion of decoded
//! pictures / samples into them ([`PixConv`], [`AudioConv`]). Built only on the symbols
//! wave 1 bound (`symbols.rs` is frozen).
//!
//! Encoder set-up goes through `AVCodecParameters` + `avcodec_parameters_to_context`
//! plus AVOptions (review must-fix 10: there is no `sample_fmt` AVOption, and
//! `AVCodecContext` is never read as a struct). Codec-private options (`crf`, `preset`,
//! …) are set with `AV_OPT_SEARCH_CHILDREN`.
//!
//! The encoder path *writes* a few `AVFrame` fields the read prefix of `layout.rs`
//! covers (`width`, `height`, `format`, `nb_samples`, `sample_rate`, `pts`, `quality`)
//! and two fields past `decode_error_flags`: `ch_layout` (audio frames need a valid
//! layout for `av_frame_get_buffer` and the encoders) and `duration`. Their offsets come
//! from [`FrameTail`], one per major, pinned by `tests::expected_tail_offsets` and
//! compared with `layout_probe.c` output when `SOYUZ_FFMPEG_LAYOUT` is set.

use std::ffi::{c_int, c_void};

use super::codec::{ChLayout, Frame, pix_fmt_name, sample_fmt_name};
use super::demux::Packet;
use super::layout::*;
use super::loader::Av;
use super::types::*;
use super::{av_err, cstr, cstring, ok, opt_set, opt_set_q};
use crate::{Majors, MediaError};

/// `AV_OPT_SEARCH_CHILDREN`: codec-private options live in `priv_data`.
const AV_OPT_SEARCH_CHILDREN: c_int = 1;
/// `AV_CODEC_CONFIG_SAMPLE_RATE` (`AV_CODEC_CONFIG_PIX_FORMAT` and `_SAMPLE_FORMAT` are
/// in `types.rs`).
const AV_CODEC_CONFIG_SAMPLE_RATE: c_int = 2;
/// `AV_CODEC_CAP_VARIABLE_FRAME_SIZE`: the encoder takes audio frames of any size.
const AV_CODEC_CAP_VARIABLE_FRAME_SIZE: c_int = 1 << 16;

/// `AVFrame` through `duration` for 8.x / 9.0 (`AVFrame8` is the read prefix).
#[repr(C)]
struct AVFrame8Full {
    base: AVFrame8,
    hw_frames_ctx: *mut AVBufferRef,
    opaque_ref: *mut AVBufferRef,
    crop_top: usize,
    crop_bottom: usize,
    crop_left: usize,
    crop_right: usize,
    private_ref: *mut c_void,
    ch_layout: AVChannelLayout,
    duration: i64,
}

/// `AVFrame` through `duration` for 7.1 (`pkt_size` ends the read prefix there).
#[repr(C)]
struct AVFrame7Full {
    base: AVFrame7,
    hw_frames_ctx: *mut AVBufferRef,
    opaque_ref: *mut AVBufferRef,
    crop_top: usize,
    crop_bottom: usize,
    crop_left: usize,
    crop_right: usize,
    private_ref: *mut c_void,
    ch_layout: AVChannelLayout,
    duration: i64,
}

/// Byte offsets of the written `AVFrame` fields outside [`FrameOff`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameTail {
    pub quality: usize,
    pub ch_layout: usize,
    pub duration: usize,
}

const TAIL8: FrameTail = FrameTail {
    quality: std::mem::offset_of!(AVFrame8, quality),
    ch_layout: std::mem::offset_of!(AVFrame8Full, ch_layout),
    duration: std::mem::offset_of!(AVFrame8Full, duration),
};
const TAIL7: FrameTail = FrameTail {
    quality: std::mem::offset_of!(AVFrame7, quality),
    ch_layout: std::mem::offset_of!(AVFrame7Full, ch_layout),
    duration: std::mem::offset_of!(AVFrame7Full, duration),
};

pub(crate) fn frame_tail(m: Majors) -> FrameTail {
    if m == Majors::V7 { TAIL7 } else { TAIL8 }
}

fn write_field<T: Copy>(frame: &mut Frame, off: usize, v: T) {
    // SAFETY: `frame` is a live frame of the loaded library and `off` is the offset of a
    // field of type `T` in that library's `AVFrame` (the read prefix table or
    // `frame_tail` of its major); plain data fields are written in place, never the
    // reference-counted buffers.
    unsafe { std::ptr::write_unaligned(frame.as_ptr().cast::<u8>().add(off).cast::<T>(), v) }
}

pub(crate) fn set_pts(av: &Av, frame: &mut Frame, pts: i64) {
    write_field(frame, av.layout.frame.pts, pts);
}

/// `frame->quality` (what `ffmpeg` sets from `global_quality` for `-q:v` encoders).
pub(crate) fn set_quality(av: &Av, frame: &mut Frame, q: c_int) {
    write_field(frame, frame_tail(av.layout.majors).quality, q);
}

/// `frame->nb_samples` (the last, shorter audio frame of a stream).
pub(crate) fn set_nb_samples(av: &Av, frame: &mut Frame, n: c_int) {
    write_field(frame, av.layout.frame.nb_samples, n);
}

/// The codec id of a codec name (`"aac"` → `AV_CODEC_ID_AAC`), through the descriptor
/// table.
pub(crate) fn codec_id_by_name(av: &Av, name: &str) -> Option<c_int> {
    let mut prev: *const AVCodecDescriptor = std::ptr::null();
    loop {
        // SAFETY: iteration over the static descriptor table.
        let d = unsafe { (av.f.avcodec_descriptor_next)(prev) };
        if d.is_null() {
            return None;
        }
        // SAFETY: a static descriptor with a NUL-terminated name.
        let (desc_name, id) = unsafe { (cstr((*d).name), (*d).id) };
        if desc_name.as_deref() == Some(name) {
            return Some(id);
        }
        prev = d;
    }
}

/// A frame with its own buffer for a `w`×`h` picture of `fmt` (`av_frame_get_buffer`).
pub(crate) fn new_video_frame(av: &'static Av, w: c_int, h: c_int, fmt: c_int) -> Result<Frame, MediaError> {
    let mut f = Frame::new(av)?;
    let l = &av.layout.frame;
    write_field(&mut f, l.width, w);
    write_field(&mut f, l.height, h);
    write_field(&mut f, l.format, fmt);
    // SAFETY: the dimensions and format were just written; the frame has no buffer yet.
    ok(av, unsafe { (av.f.av_frame_get_buffer)(f.as_ptr(), 0) })?;
    Ok(f)
}

/// A frame with its own buffer for `nb_samples` samples of `fmt` at `rate` Hz in `layout`.
pub(crate) fn new_audio_frame(
    av: &'static Av,
    nb_samples: c_int,
    fmt: c_int,
    rate: c_int,
    layout: &AVChannelLayout,
) -> Result<Frame, MediaError> {
    let mut f = Frame::new(av)?;
    let l = &av.layout.frame;
    write_field(&mut f, l.nb_samples, nb_samples);
    write_field(&mut f, l.format, fmt);
    write_field(&mut f, l.sample_rate, rate);
    let off = frame_tail(av.layout.majors).ch_layout;
    // SAFETY: a fresh frame's `ch_layout` is zeroed (a valid empty layout);
    // `av_channel_layout_copy` fills it from the live `layout` and the frame's own
    // `unref` / free uninitialise it again.
    ok(av, unsafe { (av.f.av_channel_layout_copy)(f.as_ptr().cast::<u8>().add(off).cast(), layout) })?;
    // SAFETY: sample count, format, rate and layout were just written; no buffer yet.
    ok(av, unsafe { (av.f.av_frame_get_buffer)(f.as_ptr(), 0) })?;
    Ok(f)
}

/// `av_frame_make_writable`: give the frame a private buffer when an encoder still
/// holds a reference to the current one.
pub(crate) fn make_writable(av: &Av, frame: &mut Frame) -> Result<(), MediaError> {
    // SAFETY: a live frame.
    ok(av, unsafe { (av.f.av_frame_make_writable)(frame.as_ptr()) }).map(drop)
}

/// The plane pointers of `frame` for writing (`extended_data`, which equals `data` for
/// up to eight planes).
fn planes_mut(frame: &Frame) -> *const *mut u8 {
    frame.extended_data().cast_mut().cast()
}

/// An owned `AVCodecParameters` (what an opened encoder describes, for the muxer).
pub(crate) struct OwnedPar {
    av: &'static Av,
    ptr: *mut AVCodecParameters,
}

impl OwnedPar {
    pub(crate) fn new(av: &'static Av) -> Result<OwnedPar, MediaError> {
        // SAFETY: plain allocation, NULL-checked.
        let ptr = unsafe { (av.f.avcodec_parameters_alloc)() };
        if ptr.is_null() {
            return Err(av_err(av, AVERROR_ENOMEM));
        }
        Ok(OwnedPar { av, ptr })
    }

    pub(crate) fn as_ptr(&self) -> *const AVCodecParameters {
        self.ptr
    }

    /// Append one entry to `coded_side_data` (the display matrix of a re-encoded
    /// stream, so the container keeps the rotation). Allocated the way FFmpeg does
    /// (`av_malloc`), since `avcodec_parameters_free` releases it with `av_free`.
    pub(crate) fn add_side_data(&mut self, kind: c_int, bytes: &[u8]) -> Result<(), MediaError> {
        let av = self.av;
        // SAFETY: `ptr` is a live parameter set; its `coded_side_data` array (NULL when
        // empty) has `nb_coded_side_data` entries, each with an `av_malloc`ed `data`.
        // The new entry's buffer carries FFmpeg's input padding; the old array is freed
        // after its entries were copied into the new one.
        unsafe {
            let p = &mut *self.ptr;
            let nb = usize::try_from(p.nb_coded_side_data).unwrap_or(0);
            let data = (av.f.av_malloc)(bytes.len() + AV_INPUT_BUFFER_PADDING_SIZE).cast::<u8>();
            if data.is_null() {
                return Err(av_err(av, AVERROR_ENOMEM));
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
            std::ptr::write_bytes(data.add(bytes.len()), 0, AV_INPUT_BUFFER_PADDING_SIZE);
            let arr = (av.f.av_malloc)(std::mem::size_of::<AVPacketSideData>() * (nb + 1)).cast::<AVPacketSideData>();
            if arr.is_null() {
                (av.f.av_free)(data.cast());
                return Err(av_err(av, AVERROR_ENOMEM));
            }
            if nb > 0 {
                std::ptr::copy_nonoverlapping(p.coded_side_data.cast_const(), arr, nb);
            }
            std::ptr::write(arr.add(nb), AVPacketSideData { data, size: bytes.len(), r#type: kind });
            let old = p.coded_side_data;
            p.coded_side_data = arr;
            p.nb_coded_side_data = c_int::try_from(nb + 1).unwrap_or(c_int::MAX);
            (av.f.av_free)(old.cast());
        }
        Ok(())
    }
}

/// `AV_INPUT_BUFFER_PADDING_SIZE`.
pub(crate) const AV_INPUT_BUFFER_PADDING_SIZE: usize = 64;

impl Drop for OwnedPar {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from `avcodec_parameters_alloc`.
        unsafe { (self.av.f.avcodec_parameters_free)(&mut self.ptr) };
    }
}

/// How to set an encoder up.
pub(crate) enum EncSpec<'a> {
    Video {
        width: c_int,
        height: c_int,
        pix_fmt: c_int,
        time_base: AVRational,
        framerate: AVRational,
        sar: AVRational,
        /// `color_range`, `color_primaries`, `color_trc`, `color_space`.
        color: [c_int; 4],
    },
    Audio {
        sample_rate: c_int,
        sample_fmt: c_int,
        layout: &'a AVChannelLayout,
    },
}

/// What `Encoder::receive` produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Got {
    Packet,
    /// Send more frames first.
    Again,
    /// Drained after a flush.
    Eof,
}

/// An open encoder.
pub(crate) struct Encoder {
    av: &'static Av,
    ctx: *mut AVCodecContext,
    time_base: AVRational,
    frame_size: usize,
    variable_frame_size: bool,
    global_quality: c_int,
}

// SAFETY: an encoder context is used from one thread at a time (`&mut self`); FFmpeg's
// worker threads are internal to the context and joined on close.
unsafe impl Send for Encoder {}

impl Encoder {
    /// `avcodec_find_encoder_by_name`, with the codec's media type and id.
    pub(crate) fn find(av: &Av, name: &str) -> Option<(*const AVCodec, c_int, c_int)> {
        let n = cstring(name);
        // SAFETY: a lookup in a static table.
        let codec = unsafe { (av.f.avcodec_find_encoder_by_name)(n.as_ptr()) };
        if codec.is_null() {
            return None;
        }
        // SAFETY: the prefix struct matches every supported major (layout.rs).
        let c = unsafe { &*codec.cast::<AVCodecPrefix>() };
        Some((codec, c.r#type, c.id))
    }

    /// The name of the default encoder for a codec id (`avcodec_find_encoder`), if any.
    pub(crate) fn name_for_id(av: &Av, id: c_int) -> Option<String> {
        // SAFETY: a lookup in a static table.
        let codec = unsafe { (av.f.avcodec_find_encoder)(id) };
        if codec.is_null() {
            return None;
        }
        // SAFETY: the prefix struct matches every supported major (layout.rs).
        cstr(unsafe { (*codec.cast::<AVCodecPrefix>()).name })
    }

    /// The encoder's supported list of `config` (`AV_CODEC_CONFIG_*` of `int` values);
    /// `None` = anything goes (or the query is unavailable).
    pub(crate) fn supported_ints(av: &Av, codec: *const AVCodec, config: c_int) -> Option<Vec<c_int>> {
        let f = av.opt.avcodec_get_supported_config?;
        let mut list: *const c_void = std::ptr::null();
        let mut n: c_int = 0;
        // SAFETY: a NULL context is allowed (the codec's static list is returned); `list`
        // receives a static array of `n` `c_int` values, or NULL for "everything".
        let r = unsafe { f(std::ptr::null(), codec, config, 0, &mut list, &mut n) };
        if r < 0 || list.is_null() || n <= 0 {
            return None;
        }
        // SAFETY: `n > 0` entries of `c_int`, static.
        let s = unsafe { std::slice::from_raw_parts(list.cast::<c_int>(), n as usize) };
        Some(s.to_vec())
    }

    /// Pixel formats the encoder takes, by name (hardware-only formats excluded), in the
    /// encoder's preference order; empty when unknown.
    pub(crate) fn pix_fmts(av: &Av, codec: *const AVCodec) -> Vec<(c_int, String)> {
        Self::supported_ints(av, codec, AV_CODEC_CONFIG_PIX_FORMAT)
            .unwrap_or_default()
            .into_iter()
            .filter(|f| *f != AV_PIX_FMT_NONE)
            .filter_map(|f| pix_fmt_name(av, f).map(|n| (f, n)))
            .filter(|(_, n)| !is_hw_pix_fmt(n))
            .collect()
    }

    pub(crate) fn sample_fmts(av: &Av, codec: *const AVCodec) -> Vec<(c_int, String)> {
        Self::supported_ints(av, codec, AV_CODEC_CONFIG_SAMPLE_FORMAT)
            .unwrap_or_default()
            .into_iter()
            .filter(|f| *f != AV_SAMPLE_FMT_NONE)
            .filter_map(|f| sample_fmt_name(av, f).map(|n| (f, n)))
            .collect()
    }

    /// Sample rates the encoder takes; empty = any.
    pub(crate) fn sample_rates(av: &Av, codec: *const AVCodec) -> Vec<c_int> {
        Self::supported_ints(av, codec, AV_CODEC_CONFIG_SAMPLE_RATE)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| *r > 0)
            .collect()
    }

    /// Open `name` for `spec`. `options` are AVOptions of the context or the codec
    /// (`b`, `crf`, `preset`, …); one the build does not know is skipped and reported in
    /// the returned warnings, never fatal. `global_header` sets
    /// `AV_CODEC_FLAG_GLOBAL_HEADER` (MP4 / MOV / Matroska).
    pub(crate) fn open(
        av: &'static Av,
        name: &str,
        spec: &EncSpec<'_>,
        global_header: bool,
        options: &[(&str, &str)],
    ) -> Result<(Encoder, Vec<String>), MediaError> {
        let (codec, kind, id) =
            Self::find(av, name).ok_or_else(|| MediaError::Unsupported(format!("no encoder {name}")))?;
        let want = match spec {
            EncSpec::Video { .. } => AVMEDIA_TYPE_VIDEO,
            EncSpec::Audio { .. } => AVMEDIA_TYPE_AUDIO,
        };
        if kind != want {
            return Err(MediaError::Unsupported(format!("{name} is not a {} encoder", kind_name(want))));
        }
        // SAFETY: `codec` is a static codec description; NULL-checked.
        let ctx = unsafe { (av.f.avcodec_alloc_context3)(codec) };
        if ctx.is_null() {
            return Err(av_err(av, AVERROR_ENOMEM));
        }
        let mut enc = Encoder {
            av,
            ctx,
            time_base: AVRational::new(0, 1),
            frame_size: 0,
            variable_frame_size: false,
            global_quality: 0,
        };
        let par = OwnedPar::new(av)?;
        // SAFETY: `par` is a fresh parameter set; the prefix struct is written in place
        // (the same `par` table `probe` reads).
        unsafe {
            let p = &mut *par.ptr;
            p.codec_type = kind;
            p.codec_id = id;
            match *spec {
                EncSpec::Video { width, height, pix_fmt, framerate, sar, color, .. } => {
                    p.width = width;
                    p.height = height;
                    p.format = pix_fmt;
                    p.framerate = framerate;
                    p.sample_aspect_ratio = sar;
                    p.color_range = color[0];
                    p.color_primaries = color[1];
                    p.color_trc = color[2];
                    p.color_space = color[3];
                }
                EncSpec::Audio { sample_rate, sample_fmt, layout } => {
                    p.sample_rate = sample_rate;
                    p.format = sample_fmt;
                    ok(av, (av.f.av_channel_layout_copy)(&mut p.ch_layout, layout))?;
                }
            }
            ok(av, (av.f.avcodec_parameters_to_context)(ctx, par.ptr))?;
        }
        let obj = ctx.cast::<c_void>();
        let time_base = match *spec {
            EncSpec::Video { time_base, .. } => time_base,
            EncSpec::Audio { sample_rate, .. } => AVRational::new(1, sample_rate.max(1)),
        };
        opt_set_q(av, obj, "time_base", time_base)?;
        enc.time_base = time_base;
        if kind == AVMEDIA_TYPE_VIDEO {
            opt_set(av, obj, "threads", "auto")?;
        }
        if global_header {
            opt_set(av, obj, "flags", "+global_header")?;
        }
        let mut warnings = Vec::new();
        for (k, v) in options {
            let (kc, vc) = (cstring(k), cstring(v));
            // SAFETY: `ctx` is an AVOptions-enabled object we own; the search includes
            // the codec's private class.
            let r = unsafe { (av.f.av_opt_set)(obj, kc.as_ptr(), vc.as_ptr(), AV_OPT_SEARCH_CHILDREN) };
            if r < 0 {
                warnings.push(format!("{name}: option {k}={v} not applied ({})", av_err(av, r)));
            }
        }
        // SAFETY: a configured context; no options dictionary.
        let r = unsafe { (av.f.avcodec_open2)(ctx, codec, std::ptr::null_mut()) };
        if r < 0 {
            let e = av_err(av, r);
            let hint = if is_hw_encoder(name) {
                " (hardware encoders need a picture of at least about 160x64 and a pixel format they support)"
            } else {
                ""
            };
            return Err(MediaError::Unsupported(format!("encoder {name} refused the stream: {e}{hint}")));
        }
        enc.frame_size = usize::try_from(enc.opt_int("frame_size").unwrap_or(0)).unwrap_or(0);
        enc.global_quality = c_int::try_from(enc.opt_int("global_quality").unwrap_or(0)).unwrap_or(0);
        // SAFETY: the prefix struct matches every supported major (layout.rs).
        let caps = unsafe { (*codec.cast::<AVCodecPrefix>()).capabilities };
        enc.variable_frame_size = caps & AV_CODEC_CAP_VARIABLE_FRAME_SIZE != 0;
        Ok((enc, warnings))
    }

    /// The time base frame pts are expressed in (`1 / sample_rate` for audio).
    pub(crate) fn time_base(&self) -> AVRational {
        self.time_base
    }

    /// Samples per audio frame the encoder wants (`0` = any, see
    /// [`variable_frame_size`](Self::variable_frame_size)).
    pub(crate) fn frame_size(&self) -> usize {
        self.frame_size
    }

    pub(crate) fn variable_frame_size(&self) -> bool {
        self.variable_frame_size || self.frame_size == 0
    }

    /// `global_quality` of the context (`frame->quality` for `-q:v` style encoders).
    pub(crate) fn global_quality(&self) -> c_int {
        self.global_quality
    }

    /// The stream parameters of the opened encoder (`avcodec_parameters_from_context`),
    /// for the muxer's stream; `extradata` (global headers) included.
    pub(crate) fn par(&self) -> Result<OwnedPar, MediaError> {
        let par = OwnedPar::new(self.av)?;
        // SAFETY: an open context and a fresh parameter set.
        ok(self.av, unsafe { (self.av.f.avcodec_parameters_from_context)(par.ptr, self.ctx) })?;
        Ok(par)
    }

    /// An integer AVOption of the context.
    fn opt_int(&self, name: &str) -> Option<i64> {
        let n = cstring(name);
        let mut v = 0i64;
        // SAFETY: an open context; `v` receives the value.
        let r = unsafe { (self.av.f.av_opt_get_int)(self.ctx.cast(), n.as_ptr(), 0, &mut v) };
        (r >= 0).then_some(v)
    }

    /// Feed a frame (`None` = start draining). `Ok(false)`: the encoder is full; receive
    /// its packets and send the same frame again.
    pub(crate) fn send(&mut self, frame: Option<&Frame>) -> Result<bool, MediaError> {
        let p: *const c_void = frame.map_or(std::ptr::null(), |f| f.as_ptr().cast_const());
        // SAFETY: an open context and a live (or NULL) frame.
        let r = unsafe { (self.av.f.avcodec_send_frame)(self.ctx, p) };
        match r {
            AVERROR_EAGAIN => Ok(false),
            AVERROR_EOF => Ok(true),
            r if r < 0 => Err(av_err(self.av, r)),
            _ => Ok(true),
        }
    }

    pub(crate) fn receive(&mut self, pkt: &mut Packet) -> Result<Got, MediaError> {
        pkt.unref();
        // SAFETY: an open context and a live packet.
        let r = unsafe { (self.av.f.avcodec_receive_packet)(self.ctx, pkt.as_mut_ptr()) };
        match r {
            AVERROR_EAGAIN => Ok(Got::Again),
            AVERROR_EOF => Ok(Got::Eof),
            r if r < 0 => Err(av_err(self.av, r)),
            _ => Ok(Got::Packet),
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: `ctx` came from `avcodec_alloc_context3`; `free_context` closes it too.
        unsafe { (self.av.f.avcodec_free_context)(&mut self.ctx) };
    }
}

fn kind_name(kind: c_int) -> &'static str {
    if kind == AVMEDIA_TYPE_VIDEO { "video" } else { "audio" }
}

/// A vendor hardware encoder (`h264_nvenc`, `hevc_amf`, …).
pub(crate) fn is_hw_encoder(name: &str) -> bool {
    ["_nvenc", "_amf", "_qsv", "_videotoolbox", "_vaapi", "_mf", "_v4l2m2m"].iter().any(|s| name.ends_with(s))
}

/// Pixel formats that name GPU surfaces (an encoder lists them after its software
/// formats; the CPU path never produces them).
pub(crate) fn is_hw_pix_fmt(name: &str) -> bool {
    matches!(
        name,
        "cuda"
            | "d3d11"
            | "d3d11va"
            | "d3d12"
            | "dxva2_vld"
            | "qsv"
            | "vaapi"
            | "vdpau"
            | "videotoolbox_vld"
            | "mediacodec"
            | "vulkan"
            | "opencl"
            | "drm_prime"
            | "amf_surface"
            | "mmal"
            | "xvmc"
    )
}

/// A scaler from one picture size / format to another ([`new_video_frame`] targets).
pub(crate) struct PixConv {
    av: &'static Av,
    ctx: *mut SwsContext,
    src: (c_int, c_int, c_int),
    dst: (c_int, c_int, c_int),
}

// SAFETY: used from one thread at a time (`&mut self`); swscale contexts have no thread
// affinity.
unsafe impl Send for PixConv {}

impl PixConv {
    pub(crate) fn new(
        av: &'static Av,
        src: (c_int, c_int, c_int),
        dst: (c_int, c_int, c_int),
    ) -> Result<PixConv, MediaError> {
        if src.0 <= 0 || src.1 <= 0 || src.2 < 0 || dst.0 <= 0 || dst.1 <= 0 || dst.2 < 0 {
            return Err(MediaError::Unsupported(format!("cannot convert {}x{} (format {})", src.0, src.1, src.2)));
        }
        // Bicubic keeps detail when the size changes; a pure format change is exact anyway.
        let flags = if (src.0, src.1) == (dst.0, dst.1) { SWS_BILINEAR } else { SWS_BICUBIC };
        // SAFETY: plain construction from validated sizes; NULL-checked.
        let ctx = unsafe {
            (av.f.sws_getContext)(
                src.0,
                src.1,
                src.2,
                dst.0,
                dst.1,
                dst.2,
                flags,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if ctx.is_null() {
            return Err(MediaError::Unsupported(format!(
                "swscale refuses {}x{} format {} to {}x{} format {}",
                src.0, src.1, src.2, dst.0, dst.1, dst.2
            )));
        }
        Ok(PixConv { av, ctx, src, dst })
    }

    pub(crate) fn matches(&self, frame: &Frame) -> bool {
        self.src == (frame.width(), frame.height(), frame.format())
    }

    /// Convert `src` into `dst` (a writable frame of the destination size and format).
    pub(crate) fn convert(&mut self, src: &Frame, dst: &mut Frame) -> Result<(), MediaError> {
        if !self.matches(src) || (dst.width(), dst.height(), dst.format()) != self.dst {
            return Err(MediaError::Unsupported("frame does not match the converter".into()));
        }
        let data = src.data();
        let linesize = src.linesize();
        let dst_data = planes_mut(dst);
        let dst_linesize = dst.linesize();
        // SAFETY: `src` matches the context's source description (checked above) and
        // `dst` its destination; both frames own their planes (`dst` from
        // `av_frame_get_buffer`, made writable by the caller).
        let rows = unsafe {
            (self.av.f.sws_scale)(
                self.ctx,
                data.as_ptr(),
                linesize.as_ptr(),
                0,
                self.src.1,
                dst_data,
                dst_linesize.as_ptr(),
            )
        };
        if rows <= 0 {
            return Err(av_err(self.av, if rows < 0 { rows } else { AVERROR_INVALIDDATA }));
        }
        Ok(())
    }
}

impl Drop for PixConv {
    fn drop(&mut self) {
        // SAFETY: `ctx` came from `sws_getContext`.
        unsafe { (self.av.f.sws_freeContext)(self.ctx) };
    }
}

/// `SWS_BICUBIC`.
const SWS_BICUBIC: c_int = 4;

/// A resampler from decoded frames to an encoder's sample format / rate / layout,
/// writing straight into [`new_audio_frame`] frames (swresample buffers the rest).
pub(crate) struct AudioConv {
    av: &'static Av,
    ctx: *mut SwrContext,
    src: (c_int, c_int, u32),
    out_channels: usize,
    /// Bytes per sample of the output format; one plane per channel when planar.
    bps: usize,
    planar: bool,
}

// SAFETY: used from one thread at a time (`&mut self`); swresample contexts have no
// thread affinity.
unsafe impl Send for AudioConv {}

impl AudioConv {
    pub(crate) fn new(
        av: &'static Av,
        in_layout: &ChLayout,
        in_fmt: c_int,
        in_rate: c_int,
        out_layout: &AVChannelLayout,
        out_fmt: c_int,
        out_rate: c_int,
    ) -> Result<AudioConv, MediaError> {
        if in_rate <= 0 || in_fmt < 0 || in_layout.l.nb_channels <= 0 || out_rate <= 0 || out_fmt < 0 {
            return Err(MediaError::Unsupported(format!(
                "cannot resample {} channels at {in_rate} Hz (format {in_fmt})",
                in_layout.l.nb_channels
            )));
        }
        let mut ctx: *mut SwrContext = std::ptr::null_mut();
        // SAFETY: `ctx` receives a new context; both layouts are live for the call (swr
        // copies them).
        let r = unsafe {
            (av.f.swr_alloc_set_opts2)(
                &mut ctx,
                out_layout,
                out_fmt,
                out_rate,
                &in_layout.l,
                in_fmt,
                in_rate,
                0,
                std::ptr::null_mut(),
            )
        };
        ok(av, r)?;
        let c = AudioConv {
            av,
            ctx,
            src: (in_fmt, in_rate, in_layout.channels()),
            out_channels: usize::try_from(out_layout.nb_channels).unwrap_or(0),
            bps: super::codec::bytes_per_sample(av, out_fmt),
            planar: super::codec::sample_fmt_is_planar(av, out_fmt),
        };
        // SAFETY: a configured context.
        ok(av, unsafe { (av.f.swr_init)(c.ctx) })?;
        Ok(c)
    }

    pub(crate) fn matches(&self, in_fmt: c_int, in_rate: c_int, channels: u32) -> bool {
        self.src == (in_fmt, in_rate, channels)
    }

    /// Convert `input` (`None` = drain what swresample buffered) into `dst` from sample
    /// `filled` on, at most `capacity - filled` samples; returns the samples written.
    /// Input that does not fit stays buffered inside swresample: call again with `None`
    /// until this returns less than the space offered.
    pub(crate) fn convert_into(
        &mut self,
        input: Option<&Frame>,
        dst: &mut Frame,
        filled: usize,
        capacity: usize,
    ) -> Result<usize, MediaError> {
        let space = capacity.saturating_sub(filled);
        if space == 0 {
            return Ok(0);
        }
        let planes = if self.planar { self.out_channels } else { 1 };
        let step = if self.planar { self.bps } else { self.bps * self.out_channels };
        let base = planes_mut(dst);
        let mut out: Vec<*mut u8> = Vec::with_capacity(planes.max(1));
        for i in 0..planes {
            // SAFETY: `dst` has `out_channels` planes (packed: one) of `capacity`
            // samples from `av_frame_get_buffer`; `filled * step` stays inside them.
            let p = unsafe { (*base.add(i)).add(filled * step) };
            out.push(p);
        }
        let (in_ptr, in_count) = match input {
            Some(f) => (f.extended_data(), f.nb_samples()),
            None => (std::ptr::null(), 0),
        };
        // SAFETY: `out` holds `planes` writable pointers with `space` samples of room
        // each; `in_ptr` is the input frame's plane array valid for `in_count` samples,
        // or NULL to drain.
        let got = unsafe {
            (self.av.f.swr_convert)(
                self.ctx,
                out.as_ptr(),
                c_int::try_from(space).unwrap_or(c_int::MAX),
                in_ptr,
                in_count,
            )
        };
        usize::try_from(ok(self.av, got)?).map_err(|_| MediaError::Unsupported("swr_convert".into()))
    }
}

impl Drop for AudioConv {
    fn drop(&mut self) {
        // SAFETY: `ctx` came from `swr_alloc_set_opts2`.
        unsafe { (self.av.f.swr_free)(&mut self.ctx) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-computed from the headers (LP64 / LLP64), continuing `layout::tests`: 8.x /
    /// 9.0 pad `decode_error_flags` (320) to 328, then five pointers and four `size_t`
    /// bring `ch_layout` to 384 and `duration` to 408; 7.1 has `pkt_size` after
    /// `decode_error_flags` (344), so its tail sits 24 bytes later.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn expected_tail_offsets() {
        assert_eq!(TAIL8, FrameTail { quality: 160, ch_layout: 384, duration: 408 });
        assert_eq!(TAIL7, FrameTail { quality: 160, ch_layout: 408, duration: 432 });
        assert_eq!(frame_tail(Majors::V9), TAIL8);
        assert_eq!(frame_tail(Majors::V8), TAIL8);
        assert_eq!(frame_tail(Majors::V7), TAIL7);
        assert_eq!(std::mem::offset_of!(AVFrame8Full, hw_frames_ctx), 328);
        assert_eq!(std::mem::offset_of!(AVFrame7Full, hw_frames_ctx), 352);
        assert_eq!(AV_CODEC_CAP_VARIABLE_FRAME_SIZE, 0x10000);
        assert!(is_hw_pix_fmt("cuda") && !is_hw_pix_fmt("yuv420p"));
        assert!(is_hw_encoder("h264_nvenc") && !is_hw_encoder("mpeg4"));
    }

    /// The tail rows of `layout_probe.c` output (`SOYUZ_FFMPEG_LAYOUT`, CI), same file
    /// and line format as `layout::tests::layout_json`.
    #[test]
    fn tail_json() {
        let Some(path) = std::env::var_os("SOYUZ_FFMPEG_LAYOUT") else {
            eprintln!("skipped: SOYUZ_FFMPEG_LAYOUT not set");
            return;
        };
        let text = std::fs::read_to_string(&path).expect("read SOYUZ_FFMPEG_LAYOUT");
        let get = |line: &str, key: &str| -> Option<String> {
            let needle = format!("\"{key}\":");
            let rest = &line[line.find(&needle)? + needle.len()..];
            let end = rest.find([',', '}']).unwrap_or(rest.len());
            Some(rest[..end].trim().trim_matches('"').to_string())
        };
        let mut majors: Option<Majors> = None;
        let mut rows: Vec<(String, usize)> = Vec::new();
        for line in text.lines() {
            if line.contains("\"majors\"") {
                let num = |k: &str| get(line, k).and_then(|v| v.parse::<u32>().ok()).expect("major");
                majors = Some(Majors {
                    avutil: num("avutil"),
                    swresample: num("swresample"),
                    swscale: num("swscale"),
                    avcodec: num("avcodec"),
                    avformat: num("avformat"),
                });
            } else if get(line, "struct").as_deref() == Some("AVFrame")
                && let (Some(f), Some(o)) = (get(line, "field"), get(line, "offset"))
            {
                rows.push((f, o.parse().expect("offset")));
            }
        }
        let tail = frame_tail(majors.expect("a majors line"));
        let probed =
            |f: &str| rows.iter().find(|(pf, _)| pf == f).map(|r| r.1).unwrap_or_else(|| panic!("AVFrame.{f}"));
        assert_eq!(tail.quality, probed("quality"));
        assert_eq!(tail.ch_layout, probed("ch_layout"));
        assert_eq!(tail.duration, probed("duration"));
    }
}
