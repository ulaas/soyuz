//! `AVCodecContext` ([`Decoder`]), `AVFrame` ([`Frame`], read through the per-major
//! offset table) and `AVChannelLayout` ([`ChLayout`]).

use std::ffi::{c_int, c_void};

use super::demux::Packet;
use super::layout::*;
use super::loader::Av;
use super::types::*;
use super::{av_err, cstr, cstring, ok, opt_set, opt_set_int, opt_set_q};
use crate::MediaError;

/// Decoder limits (review must-fix 8b): `max_pixels` and `err_detect`.
pub(crate) const MAX_PIXELS: i64 = 64 << 20;

/// What `Decoder::receive` produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Recv {
    Frame,
    /// Send more input first.
    Again,
    /// Drained after a flush.
    Eof,
}

/// An `AVChannelLayout` that is uninitialised on drop.
pub(crate) struct ChLayout {
    av: &'static Av,
    pub l: AVChannelLayout,
}

#[allow(dead_code)] // `default_for` / `copy_of` serve the encoder set-up (V2c)
impl ChLayout {
    pub(crate) fn zeroed(av: &'static Av) -> ChLayout {
        ChLayout { av, l: AVChannelLayout::zeroed() }
    }

    /// The default layout for `channels` channels.
    pub(crate) fn default_for(av: &'static Av, channels: c_int) -> ChLayout {
        let mut c = ChLayout::zeroed(av);
        // SAFETY: `l` is a zeroed layout; the function fills it.
        unsafe { (av.f.av_channel_layout_default)(&mut c.l, channels) };
        c
    }

    pub(crate) fn copy_of(av: &'static Av, src: &AVChannelLayout) -> Result<ChLayout, MediaError> {
        let mut c = ChLayout::zeroed(av);
        // SAFETY: `src` is a live layout (inside codec parameters or a context), `l` is
        // zeroed and receives an owned copy.
        ok(av, unsafe { (av.f.av_channel_layout_copy)(&mut c.l, src) })?;
        Ok(c)
    }

    pub(crate) fn channels(&self) -> u32 {
        u32::try_from(self.l.nb_channels).unwrap_or(0)
    }

    /// `av_channel_layout_describe` (`"stereo"`, `"5.1(side)"`, …).
    pub(crate) fn describe(&self) -> String {
        describe_layout(self.av, &self.l)
    }
}

impl Drop for ChLayout {
    fn drop(&mut self) {
        // SAFETY: `l` is either zeroed or an owned layout; `uninit` handles both.
        unsafe { (self.av.f.av_channel_layout_uninit)(&mut self.l) };
    }
}

pub(crate) fn describe_layout(av: &Av, l: &AVChannelLayout) -> String {
    let mut buf = [0u8; 128];
    // SAFETY: `buf` is a writable buffer of the given size; the function NUL-terminates.
    let n = unsafe { (av.f.av_channel_layout_describe)(l, buf.as_mut_ptr().cast(), buf.len()) };
    if n <= 0 {
        return String::new();
    }
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// An open decoder.
pub(crate) struct Decoder {
    av: &'static Av,
    ctx: *mut AVCodecContext,
}

// SAFETY: a decoder context is used from one thread at a time (`&mut self`); FFmpeg's
// own worker threads are internal to the context and joined on close.
unsafe impl Send for Decoder {}

#[allow(dead_code)] // `as_ptr` / `opt_int` serve hardware decoding (V4b)
impl Decoder {
    /// Open `codec` with the parameters of a stream (`par`), its time base and the given
    /// AVOptions (`threads`, `skip_frame`, …) on top of the limits.
    pub(crate) fn open(
        av: &'static Av,
        codec: *const AVCodec,
        par: *const AVCodecParameters,
        time_base: AVRational,
        options: &[(&str, &str)],
    ) -> Result<Decoder, MediaError> {
        Decoder::open_on(av, codec, par, time_base, options, None)
    }

    /// [`open`](Self::open) with a hardware device attached (`hw_device_ctx`): frames
    /// come in the codec's hardware format when FFmpeg's hwaccel accepts the stream, in
    /// a software format otherwise.
    pub(crate) fn open_hw(
        av: &'static Av,
        codec: *const AVCodec,
        par: *const AVCodecParameters,
        time_base: AVRational,
        options: &[(&str, &str)],
        device: &super::hwdec::HwDevice,
    ) -> Result<Decoder, MediaError> {
        Decoder::open_on(av, codec, par, time_base, options, Some(device))
    }

    fn open_on(
        av: &'static Av,
        codec: *const AVCodec,
        par: *const AVCodecParameters,
        time_base: AVRational,
        options: &[(&str, &str)],
        device: Option<&super::hwdec::HwDevice>,
    ) -> Result<Decoder, MediaError> {
        if codec.is_null() {
            return Err(MediaError::Unsupported("no decoder".into()));
        }
        // SAFETY: `codec` is a static codec description; NULL-checked.
        let ctx = unsafe { (av.f.avcodec_alloc_context3)(codec) };
        if ctx.is_null() {
            return Err(av_err(av, AVERROR_ENOMEM));
        }
        let d = Decoder { av, ctx };
        let obj = ctx.cast::<c_void>();
        // SAFETY: a fresh context and live parameters.
        ok(av, unsafe { (av.f.avcodec_parameters_to_context)(ctx, par) })?;
        if time_base.den > 0 {
            opt_set_q(av, obj, "pkt_timebase", time_base)?;
        }
        opt_set_int(av, obj, "max_pixels", MAX_PIXELS)?;
        opt_set(av, obj, "err_detect", "careful")?;
        for (k, v) in options {
            opt_set(av, obj, k, v)?;
        }
        if let Some(device) = device {
            super::hwdec::attach(av, ctx, device)?;
        }
        // SAFETY: a configured context; no options dictionary.
        ok(av, unsafe { (av.f.avcodec_open2)(d.ctx, codec, std::ptr::null_mut()) })?;
        Ok(d)
    }

    pub(crate) fn as_ptr(&self) -> *mut AVCodecContext {
        self.ctx
    }

    /// Feed a packet (`None` = start draining).
    pub(crate) fn send(&mut self, pkt: Option<&Packet>) -> Result<(), MediaError> {
        let p = pkt.map_or(std::ptr::null(), Packet::as_ptr);
        // SAFETY: an open context and a live (or NULL) packet.
        let r = unsafe { (self.av.f.avcodec_send_packet)(self.ctx, p) };
        if r == AVERROR_EAGAIN || r == AVERROR_EOF {
            return Ok(());
        }
        ok(self.av, r).map(drop)
    }

    pub(crate) fn receive(&mut self, frame: &mut Frame) -> Result<Recv, MediaError> {
        frame.unref();
        // SAFETY: an open context and a live frame.
        let r = unsafe { (self.av.f.avcodec_receive_frame)(self.ctx, frame.ptr) };
        match r {
            AVERROR_EAGAIN => Ok(Recv::Again),
            AVERROR_EOF => Ok(Recv::Eof),
            r if r < 0 => Err(av_err(self.av, r)),
            _ => Ok(Recv::Frame),
        }
    }

    /// Discard buffered frames (after a seek).
    pub(crate) fn flush(&mut self) {
        // SAFETY: an open context.
        unsafe { (self.av.f.avcodec_flush_buffers)(self.ctx) };
    }

    /// The decoder's current channel layout (`ch_layout` AVOption; never an `AVFrame`
    /// tail field).
    pub(crate) fn ch_layout(&self) -> Result<ChLayout, MediaError> {
        let mut c = ChLayout::zeroed(self.av);
        // SAFETY: an open context; `l` receives an owned copy.
        ok(self.av, unsafe { (self.av.f.av_opt_get_chlayout)(self.ctx.cast(), c"ch_layout".as_ptr(), 0, &mut c.l) })?;
        Ok(c)
    }

    /// An integer AVOption of the context (`sample_rate`, `width`, …).
    pub(crate) fn opt_int(&self, name: &str) -> Option<i64> {
        let n = cstring(name);
        let mut v = 0i64;
        // SAFETY: an open context; `v` receives the value.
        let r = unsafe { (self.av.f.av_opt_get_int)(self.ctx.cast(), n.as_ptr(), 0, &mut v) };
        (r >= 0).then_some(v)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: `ctx` came from `avcodec_alloc_context3`; `free_context` closes it too.
        unsafe { (self.av.f.avcodec_free_context)(&mut self.ctx) };
    }
}

/// One `AVFrame`; fields are read through the loaded major's offset table, never past
/// `decode_error_flags`.
pub(crate) struct Frame {
    av: &'static Av,
    ptr: *mut c_void,
}

// SAFETY: a frame is heap data owned by this handle (reference-counted buffers are
// thread-safe in FFmpeg).
unsafe impl Send for Frame {}

#[allow(dead_code)] // the colour / key-frame accessors serve the player (V2a)
impl Frame {
    pub(crate) fn new(av: &'static Av) -> Result<Frame, MediaError> {
        // SAFETY: plain allocation, NULL-checked.
        let ptr = unsafe { (av.f.av_frame_alloc)() };
        if ptr.is_null() {
            return Err(av_err(av, AVERROR_ENOMEM));
        }
        Ok(Frame { av, ptr })
    }

    pub(crate) fn as_ptr(&self) -> *mut c_void {
        self.ptr
    }

    fn field<T: Copy>(&self, off: usize) -> T {
        // SAFETY: `ptr` is a live frame of the loaded library; `off` comes from the
        // layout table of that library's major (`Av::layout`), so the field exists at
        // that offset with type `T` (the prefix never goes past `decode_error_flags`).
        unsafe { std::ptr::read_unaligned(self.ptr.cast::<u8>().add(off).cast::<T>()) }
    }

    pub(crate) fn width(&self) -> c_int {
        self.field(self.av.layout.frame.width)
    }

    pub(crate) fn height(&self) -> c_int {
        self.field(self.av.layout.frame.height)
    }

    pub(crate) fn format(&self) -> c_int {
        self.field(self.av.layout.frame.format)
    }

    pub(crate) fn nb_samples(&self) -> c_int {
        self.field(self.av.layout.frame.nb_samples)
    }

    pub(crate) fn sample_rate(&self) -> c_int {
        self.field(self.av.layout.frame.sample_rate)
    }

    pub(crate) fn pts(&self) -> i64 {
        self.field(self.av.layout.frame.pts)
    }

    pub(crate) fn best_effort_timestamp(&self) -> i64 {
        self.field(self.av.layout.frame.best_effort_timestamp)
    }

    pub(crate) fn sample_aspect_ratio(&self) -> AVRational {
        self.field(self.av.layout.frame.sample_aspect_ratio)
    }

    pub(crate) fn is_key(&self) -> bool {
        self.field::<c_int>(self.av.layout.frame.flags) & 2 != 0
    }

    pub(crate) fn color_range(&self) -> c_int {
        self.field(self.av.layout.frame.color_range)
    }

    pub(crate) fn color_trc(&self) -> c_int {
        self.field(self.av.layout.frame.color_trc)
    }

    pub(crate) fn color_primaries(&self) -> c_int {
        self.field(self.av.layout.frame.color_primaries)
    }

    pub(crate) fn colorspace(&self) -> c_int {
        self.field(self.av.layout.frame.colorspace)
    }

    /// Copy a hardware frame (`self`) into system memory: `dst` gets the transfer
    /// format FFmpeg picks first (NV12 / P010 for the supported devices) and this
    /// frame's size.
    pub(crate) fn transfer_to(&self, dst: &mut Frame) -> Result<(), MediaError> {
        dst.unref();
        // SAFETY: two live frames; `dst` is blank (format unset), so FFmpeg allocates it.
        ok(self.av, unsafe { (self.av.f.av_hwframe_transfer_data)(dst.ptr, self.ptr, 0) }).map(drop)
    }

    /// The eight plane pointers.
    pub(crate) fn data(&self) -> [*const u8; 8] {
        let d: [*mut u8; 8] = self.field(self.av.layout.frame.data);
        d.map(|p| p.cast_const())
    }

    /// `extended_data`: the plane pointers of planar audio with more than 8 channels
    /// (equal to `data` otherwise).
    pub(crate) fn extended_data(&self) -> *const *const u8 {
        let p: *mut *mut u8 = self.field(self.av.layout.frame.extended_data);
        p.cast_const().cast()
    }

    pub(crate) fn linesize(&self) -> [c_int; 8] {
        self.field(self.av.layout.frame.linesize)
    }

    /// Plane `i` as a byte slice of `len` bytes (audio: `nb_samples * bytes per
    /// sample [* channels]`).
    pub(crate) fn plane(&self, i: usize, len: usize) -> Option<&[u8]> {
        let p = self.data()[i];
        if p.is_null() || len == 0 {
            return None;
        }
        // SAFETY: FFmpeg guarantees `len` bytes of the plane (the caller computes it from
        // the frame's own dimensions / sample count and format).
        Some(unsafe { std::slice::from_raw_parts(p, len) })
    }

    pub(crate) fn unref(&mut self) {
        // SAFETY: a live frame.
        unsafe { (self.av.f.av_frame_unref)(self.ptr) };
    }

    /// Make this a blank `w × h` picture in `fmt` with its own buffers (the target of a
    /// scaler).
    pub(crate) fn alloc_video(&mut self, w: c_int, h: c_int, fmt: c_int) -> Result<(), MediaError> {
        self.unref();
        let l = self.av.layout.frame;
        // SAFETY: an unreferenced live frame of the loaded library; the offsets come from
        // that major's table (prefix fields), written with their C types.
        unsafe {
            let base = self.ptr.cast::<u8>();
            std::ptr::write_unaligned(base.add(l.width).cast::<c_int>(), w);
            std::ptr::write_unaligned(base.add(l.height).cast::<c_int>(), h);
            std::ptr::write_unaligned(base.add(l.format).cast::<c_int>(), fmt);
        }
        // SAFETY: size and format set above; FFmpeg allocates the planes.
        ok(self.av, unsafe { (self.av.f.av_frame_get_buffer)(self.ptr, 0) }).map(drop)
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from `av_frame_alloc`.
        unsafe { (self.av.f.av_frame_free)(&mut self.ptr) };
    }
}

/// An owned copy of a stream's `AVCodecParameters` (a decoder can be reopened later, on
/// another thread, without touching the demuxer's stream).
pub(crate) struct Params {
    av: &'static Av,
    ptr: *mut AVCodecParameters,
}

// SAFETY: heap data owned by this handle, only read by `avcodec_parameters_to_context`.
unsafe impl Send for Params {}

impl Params {
    pub(crate) fn copy_of(av: &'static Av, src: *const AVCodecParameters) -> Result<Params, MediaError> {
        // SAFETY: plain allocation, NULL-checked.
        let ptr = unsafe { (av.f.avcodec_parameters_alloc)() };
        if ptr.is_null() {
            return Err(av_err(av, AVERROR_ENOMEM));
        }
        let p = Params { av, ptr };
        // SAFETY: a fresh parameter set and a live source (the caller's stream).
        ok(av, unsafe { (av.f.avcodec_parameters_copy)(p.ptr, src) })?;
        Ok(p)
    }

    pub(crate) fn as_ptr(&self) -> *const AVCodecParameters {
        self.ptr
    }

    /// `[color_range, color_primaries, color_trc, color_space]` of the stream.
    pub(crate) fn color(&self) -> [c_int; 4] {
        // SAFETY: an owned, initialised parameter set (prefix struct identical on every
        // supported release).
        let p = unsafe { &*self.ptr };
        [p.color_range, p.color_primaries, p.color_trc, p.color_space]
    }
}

/// A codec description (`AVCodec`): static data inside the library, which is never
/// unloaded, so the pointer may move between threads.
#[derive(Clone, Copy)]
pub(crate) struct CodecRef(pub *const AVCodec);

// SAFETY: points at an immutable static of the loaded library.
unsafe impl Send for CodecRef {}

impl Drop for Params {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from `avcodec_parameters_alloc`.
        unsafe { (self.av.f.avcodec_parameters_free)(&mut self.ptr) };
    }
}

/// Names of a pixel / sample format, codec and profile (empty when unknown).
pub(crate) fn pix_fmt_name(av: &Av, fmt: c_int) -> Option<String> {
    if fmt < 0 {
        return None;
    }
    // SAFETY: returns a static string or NULL.
    cstr(unsafe { (av.f.av_get_pix_fmt_name)(fmt) })
}

pub(crate) fn sample_fmt_name(av: &Av, fmt: c_int) -> Option<String> {
    if fmt < 0 {
        return None;
    }
    // SAFETY: returns a static string or NULL.
    cstr(unsafe { (av.f.av_get_sample_fmt_name)(fmt) })
}

/// `av_get_pix_fmt("rgba")` / `av_get_sample_fmt("flt")`: a format by name.
pub(crate) fn pix_fmt(av: &Av, name: &str) -> Option<c_int> {
    let n = cstring(name);
    // SAFETY: a lookup in a static table.
    let f = unsafe { (av.f.av_get_pix_fmt)(n.as_ptr()) };
    (f != AV_PIX_FMT_NONE).then_some(f)
}

pub(crate) fn sample_fmt(av: &Av, name: &str) -> Option<c_int> {
    let n = cstring(name);
    // SAFETY: a lookup in a static table.
    let f = unsafe { (av.f.av_get_sample_fmt)(n.as_ptr()) };
    (f != AV_SAMPLE_FMT_NONE).then_some(f)
}

#[allow(dead_code)] // V2c
pub(crate) fn bytes_per_sample(av: &Av, fmt: c_int) -> usize {
    // SAFETY: a lookup in a static table.
    usize::try_from(unsafe { (av.f.av_get_bytes_per_sample)(fmt) }).unwrap_or(0)
}

#[allow(dead_code)] // V2c
pub(crate) fn sample_fmt_is_planar(av: &Av, fmt: c_int) -> bool {
    // SAFETY: a lookup in a static table.
    unsafe { (av.f.av_sample_fmt_is_planar)(fmt) != 0 }
}

pub(crate) fn codec_name(av: &Av, id: c_int) -> String {
    // SAFETY: returns a static string ("unknown_codec" for bad ids).
    cstr(unsafe { (av.f.avcodec_get_name)(id) }).unwrap_or_default()
}

pub(crate) fn codec_long_name(av: &Av, id: c_int) -> String {
    // SAFETY: returns a static descriptor or NULL.
    let d = unsafe { (av.f.avcodec_descriptor_get)(id) };
    if d.is_null() {
        return String::new();
    }
    // SAFETY: a static descriptor.
    cstr(unsafe { (*d).long_name }).unwrap_or_default()
}

pub(crate) fn profile_name(av: &Av, id: c_int, profile: c_int) -> Option<String> {
    // SAFETY: returns a static string or NULL.
    cstr(unsafe { (av.f.avcodec_profile_name)(id, profile) })
}

pub(crate) fn color_names(av: &Av, range: c_int, primaries: c_int, trc: c_int, space: c_int) -> [Option<String>; 4] {
    // SAFETY: static strings or NULL.
    unsafe {
        [
            cstr((av.f.av_color_range_name)(range)),
            cstr((av.f.av_color_primaries_name)(primaries)),
            cstr((av.f.av_color_transfer_name)(trc)),
            cstr((av.f.av_color_space_name)(space)),
        ]
    }
}

/// `av_display_rotation_get` of a display matrix (9 × `int32_t`), as the clockwise
/// rotation to apply for display (0 / 90 / 180 / 270), like `ffmpeg`'s autorotate.
pub(crate) fn display_rotation(av: &Av, matrix: &[u8]) -> i32 {
    if matrix.len() < 36 {
        return 0;
    }
    let mut m = [0i32; 9];
    for (i, v) in m.iter_mut().enumerate() {
        *v = i32::from_le_bytes([matrix[i * 4], matrix[i * 4 + 1], matrix[i * 4 + 2], matrix[i * 4 + 3]]);
    }
    // SAFETY: `m` is the 9-element matrix the function reads.
    let theta = unsafe { (av.f.av_display_rotation_get)(m.as_ptr()) };
    if !theta.is_finite() {
        return 0;
    }
    // FFmpeg reports the counter-clockwise angle; display applies the opposite.
    let clockwise = (-theta).round() as i64;
    let r = clockwise.rem_euclid(360);
    // Snap to the four orientations the renderers apply.
    (((r + 45) / 90) % 4 * 90) as i32
}

/// `avcodec_find_decoder_by_name(name)` exists.
pub(crate) fn has_decoder(av: &Av, name: &str) -> bool {
    let n = cstring(name);
    // SAFETY: a lookup in a static table.
    !unsafe { (av.f.avcodec_find_decoder_by_name)(n.as_ptr()) }.is_null()
}

/// The encoder `name` exists *and opens* with a small default configuration (hardware
/// encoders fail here on machines without the driver).
pub(crate) fn encoder_opens(av: &'static Av, name: &str) -> bool {
    let n = cstring(name);
    // SAFETY: a lookup in a static table.
    let codec = unsafe { (av.f.avcodec_find_encoder_by_name)(n.as_ptr()) };
    if codec.is_null() {
        return false;
    }
    // SAFETY: the prefix struct matches every supported major (layout.rs).
    let (kind, id) = unsafe {
        let c = &*codec.cast::<AVCodecPrefix>();
        (c.r#type, c.id)
    };
    // SAFETY: plain allocations, NULL-checked; freed below.
    let (ctx, par) = unsafe { ((av.f.avcodec_alloc_context3)(codec), (av.f.avcodec_parameters_alloc)()) };
    if ctx.is_null() || par.is_null() {
        // SAFETY: freeing what was allocated (NULL is accepted).
        unsafe {
            let (mut c, mut p) = (ctx, par);
            (av.f.avcodec_free_context)(&mut c);
            (av.f.avcodec_parameters_free)(&mut p);
        }
        return false;
    }
    let first_format = |config: c_int, none: c_int| -> Option<c_int> {
        let f = av.opt.avcodec_get_supported_config?;
        let mut list: *const c_void = std::ptr::null();
        let mut n: c_int = 0;
        // SAFETY: a fresh context and its codec; `list` receives a static array of
        // `n` `c_int` values (or NULL = everything accepted).
        let r = unsafe { f(ctx, codec, config, 0, &mut list, &mut n) };
        if r < 0 || list.is_null() || n <= 0 {
            return None;
        }
        // SAFETY: `n > 0` entries of `c_int`.
        let v = unsafe { *list.cast::<c_int>() };
        (v != none).then_some(v)
    };
    let opened = (|| -> Result<(), MediaError> {
        // SAFETY: `par` is a fresh parameter set; the prefix struct is written in place.
        unsafe {
            let p = &mut *par;
            p.codec_type = kind;
            p.codec_id = id;
            if kind == AVMEDIA_TYPE_VIDEO {
                // Large enough for every hardware encoder's minimum.
                p.width = 320;
                p.height = 240;
                p.format = first_format(AV_CODEC_CONFIG_PIX_FORMAT, AV_PIX_FMT_NONE)
                    .or_else(|| pix_fmt(av, "yuv420p"))
                    .unwrap_or(0);
                p.framerate = AVRational::new(25, 1);
            } else if kind == AVMEDIA_TYPE_AUDIO {
                p.sample_rate = 48_000;
                p.format = first_format(AV_CODEC_CONFIG_SAMPLE_FORMAT, AV_SAMPLE_FMT_NONE)
                    .or_else(|| sample_fmt(av, "fltp"))
                    .unwrap_or(0);
                (av.f.av_channel_layout_default)(&mut p.ch_layout, 2);
            }
            ok(av, (av.f.avcodec_parameters_to_context)(ctx, par))?;
        }
        opt_set_q(av, ctx.cast(), "time_base", AVRational::new(1, 25))?;
        if kind == AVMEDIA_TYPE_AUDIO {
            opt_set_int(av, ctx.cast(), "b", 128_000)?;
        }
        // SAFETY: a configured context.
        ok(av, unsafe { (av.f.avcodec_open2)(ctx, codec, std::ptr::null_mut()) })?;
        Ok(())
    })();
    // SAFETY: freeing our allocations (the context closes the encoder first).
    unsafe {
        let (mut c, mut p) = (ctx, par);
        (av.f.avcodec_free_context)(&mut c);
        (av.f.avcodec_parameters_free)(&mut p);
    }
    opened.is_ok()
}

/// Every codec descriptor as `(name, long_name, kind, decoder, encoder)`.
pub(crate) fn codec_list(av: &Av) -> Vec<crate::CodecDesc> {
    let mut out = Vec::new();
    let mut prev: *const AVCodecDescriptor = std::ptr::null();
    loop {
        // SAFETY: iteration over the static descriptor table.
        let d = unsafe { (av.f.avcodec_descriptor_next)(prev) };
        if d.is_null() {
            return out;
        }
        // SAFETY: a static descriptor; `find_*` are lookups.
        let (name, long_name, kind, decoder, encoder) = unsafe {
            let desc = &*d;
            (
                cstr(desc.name).unwrap_or_default(),
                cstr(desc.long_name).unwrap_or_default(),
                super::demux::kind_of(desc.r#type),
                !(av.f.avcodec_find_decoder)(desc.id).is_null(),
                !(av.f.avcodec_find_encoder)(desc.id).is_null(),
            )
        };
        if let Some(kind) = kind {
            out.push(crate::CodecDesc { name, long_name, kind, decoder, encoder });
        }
        prev = d;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rotation_snaps_to_quarter_turns() {
        // Pure arithmetic of the post-processing (the FFI call needs the library):
        let cases: [(f64, i32); 7] =
            [(0.0, 0), (-90.0, 90), (90.0, 270), (180.0, 180), (-180.0, 180), (-89.6, 90), (-44.0, 0)];
        for (theta, want) in cases {
            let clockwise = (-theta).round() as i64;
            let r = clockwise.rem_euclid(360);
            let snapped = (((r + 45) / 90) % 4 * 90) as i32;
            assert_eq!(snapped, want, "theta {theta}");
        }
    }
}
