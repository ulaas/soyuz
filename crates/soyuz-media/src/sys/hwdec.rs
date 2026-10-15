//! Hardware decoding (V4b): a device context ([`HwDevice`], `av_hwdevice_ctx_create`),
//! the codec's matching hardware pixel format (`avcodec_get_hw_config`), and the one
//! `AVCodecContext` write of the crate: `hw_device_ctx`, set before `avcodec_open2`
//! ([`attach`]). With a device attached, FFmpeg's default `get_format` picks the
//! hardware format itself (no callback of ours); frames in that format are copied to
//! system memory with `av_hwframe_transfer_data` ([`super::codec::Frame::transfer_to`]).
//!
//! The `hw_device_ctx` offset comes from the layout table of the loaded major and is
//! used only when FFmpeg's own option table agrees: the `hwaccel_flags` AVOption (the
//! field right after it in every supported release) must sit one pointer later, and the
//! field must read NULL in a fresh context. Anything else turns hardware decoding off
//! for the process (software decoding is always the fallback).

use std::ffi::{c_int, c_void};
use std::sync::OnceLock;

use super::layout::*;
use super::loader::Av;
use super::types::*;
use super::{av_err, ok};
use crate::MediaError;

/// A hardware device context (`AVHWDeviceContext` inside an `AVBufferRef`); decoders
/// hold their own references, so dropping this never pulls a device from under one.
pub(crate) struct HwDevice {
    av: &'static Av,
    buf: *mut AVBufferRef,
}

// SAFETY: the buffer reference count is atomic and the device context is only read by
// FFmpeg after creation (decoders take their own reference on open).
unsafe impl Send for HwDevice {}
// SAFETY: as above; `&HwDevice` only hands out new references (`av_buffer_ref`).
unsafe impl Sync for HwDevice {}

impl HwDevice {
    /// `av_hwdevice_ctx_create(type, NULL, NULL, 0)`: the default device of that API
    /// (fails cleanly on machines without the driver or GPU).
    pub(crate) fn create(av: &'static Av, kind: c_int) -> Result<HwDevice, MediaError> {
        let mut buf: *mut AVBufferRef = std::ptr::null_mut();
        // SAFETY: `buf` receives a new reference on success; no device string or options.
        let r = unsafe { (av.f.av_hwdevice_ctx_create)(&mut buf, kind, std::ptr::null(), std::ptr::null_mut(), 0) };
        ok(av, r)?;
        if buf.is_null() {
            return Err(av_err(av, AVERROR_ENOMEM));
        }
        Ok(HwDevice { av, buf })
    }
}

impl Drop for HwDevice {
    fn drop(&mut self) {
        // SAFETY: `buf` is our own reference from `av_hwdevice_ctx_create`.
        unsafe { (self.av.f.av_buffer_unref)(&mut self.buf) };
    }
}

/// The hardware pixel format `codec` decodes to with a device of type `kind` attached
/// through `hw_device_ctx` (`None`: the codec has no such hardware path).
pub(crate) fn hw_pix_fmt(av: &Av, codec: *const AVCodec, kind: c_int) -> Option<c_int> {
    if codec.is_null() {
        return None;
    }
    for i in 0..64 {
        // SAFETY: `codec` is a static codec description; the function returns NULL past
        // the last configuration.
        let cfg = unsafe { (av.f.avcodec_get_hw_config)(codec, i) };
        if cfg.is_null() {
            return None;
        }
        // SAFETY: a static `AVCodecHWConfig` (identical on every supported release).
        let c = unsafe { &*cfg };
        if c.methods & AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX != 0 && c.device_type == kind {
            return Some(c.pix_fmt);
        }
    }
    None
}

/// The validated `hw_device_ctx` offset (checked once per process on the first context).
fn device_offset(av: &Av, ctx: *mut AVCodecContext) -> Option<usize> {
    static OFFSET: OnceLock<Option<usize>> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        let off = av.layout.codec_ctx.hw_device_ctx;
        let find = av.opt.av_opt_find?;
        if off == 0 {
            return None;
        }
        // SAFETY: `ctx` is a live codec context (its first field is the AVClass pointer
        // `av_opt_find` reads); the result is a static option description or NULL.
        let o = unsafe { find(ctx.cast::<c_void>(), c"hwaccel_flags".as_ptr(), std::ptr::null(), 0, 0) };
        if o.is_null() {
            log::info!("soyuz-media: no hwaccel_flags option; hardware decoding off");
            return None;
        }
        // SAFETY: a non-null static `AVOption`.
        let next = unsafe { (*o).offset };
        let agrees = usize::try_from(next).ok() == Some(off + size_of::<*mut c_void>());
        if !agrees {
            log::warn!(
                "soyuz-media: hw_device_ctx offset {off} disagrees with hwaccel_flags at {next}; hardware decoding off"
            );
        }
        agrees.then_some(off)
    })
}

/// Attach a new reference to `device` as the `hw_device_ctx` of a configured, not yet
/// opened decoder context. Errors leave the context as it was (software decoding).
pub(crate) fn attach(av: &Av, ctx: *mut AVCodecContext, device: &HwDevice) -> Result<(), MediaError> {
    let off = device_offset(av, ctx)
        .ok_or_else(|| MediaError::Unsupported("hardware decoding is not available with these libraries".into()))?;
    // SAFETY: `off` is the `hw_device_ctx` field of the loaded release's AVCodecContext
    // (layout table, confirmed by the option table above); the context is live and
    // much larger than `off + 8`.
    let field = unsafe { ctx.cast::<u8>().add(off).cast::<*mut AVBufferRef>() };
    // SAFETY: reading the pointer field of a live context.
    if !unsafe { std::ptr::read_unaligned(field) }.is_null() {
        return Err(MediaError::Unsupported("hw_device_ctx is already set".into()));
    }
    // SAFETY: a live device reference; the new reference belongs to the context, which
    // unrefs it in `avcodec_free_context`.
    let r = unsafe { (av.f.av_buffer_ref)(device.buf) };
    if r.is_null() {
        return Err(av_err(av, AVERROR_ENOMEM));
    }
    // SAFETY: as for the read above.
    unsafe { std::ptr::write_unaligned(field, r) };
    Ok(())
}
