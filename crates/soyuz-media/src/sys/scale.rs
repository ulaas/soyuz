//! `SwsContext` ([`Scaler`]): one decoded picture to tightly packed RGBA.

use std::ffi::c_int;

use super::codec::{Frame, pix_fmt};
use super::loader::Av;
use super::types::*;
use crate::MediaError;

/// A scaler from one source size / format to an RGBA destination size; re-created by
/// the caller when the source changes.
pub(crate) struct Scaler {
    av: &'static Av,
    ctx: *mut SwsContext,
    src: (c_int, c_int, c_int),
    pub dst_w: u32,
    pub dst_h: u32,
}

// SAFETY: used from one thread at a time (`&mut self`); swscale contexts have no
// thread affinity.
unsafe impl Send for Scaler {}

/// How to scale (`SWS_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // `Fast` is the player's choice (V2a)
pub(crate) enum Quality {
    /// `SWS_FAST_BILINEAR` (players).
    Fast,
    /// `SWS_BILINEAR`.
    Bilinear,
    /// `SWS_AREA` (thumbnails: best for downscaling).
    Area,
}

impl Scaler {
    pub(crate) fn new(
        av: &'static Av,
        src_w: c_int,
        src_h: c_int,
        src_fmt: c_int,
        dst_w: u32,
        dst_h: u32,
        quality: Quality,
    ) -> Result<Scaler, MediaError> {
        let rgba = pix_fmt(av, "rgba").ok_or_else(|| MediaError::Unsupported("no rgba pixel format".into()))?;
        if src_w <= 0 || src_h <= 0 || src_fmt < 0 || dst_w == 0 || dst_h == 0 {
            return Err(MediaError::Unsupported(format!("cannot scale {src_w}x{src_h} (format {src_fmt})")));
        }
        let flags = match quality {
            Quality::Fast => SWS_FAST_BILINEAR,
            Quality::Bilinear => SWS_BILINEAR,
            Quality::Area => SWS_AREA,
        };
        let (dw, dh) = (c_int::try_from(dst_w).unwrap_or(c_int::MAX), c_int::try_from(dst_h).unwrap_or(c_int::MAX));
        // SAFETY: plain construction from validated sizes; NULL-checked.
        let ctx = unsafe {
            (av.f.sws_getContext)(
                src_w,
                src_h,
                src_fmt,
                dw,
                dh,
                rgba,
                flags,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if ctx.is_null() {
            return Err(MediaError::Unsupported(format!("swscale refuses {src_w}x{src_h} format {src_fmt} to rgba")));
        }
        Ok(Scaler { av, ctx, src: (src_w, src_h, src_fmt), dst_w, dst_h })
    }

    /// Whether this scaler was built for the frame's size and format.
    pub(crate) fn matches(&self, frame: &Frame) -> bool {
        self.src == (frame.width(), frame.height(), frame.format())
    }

    /// Scale `frame` into `out` (resized to `dst_w * dst_h * 4`, stride `dst_w * 4`).
    pub(crate) fn scale(&mut self, frame: &Frame, out: &mut Vec<u8>) -> Result<(), MediaError> {
        if !self.matches(frame) {
            return Err(MediaError::Unsupported("frame size changed".into()));
        }
        let stride = self.dst_w as usize * 4;
        out.clear();
        out.resize(stride * self.dst_h as usize, 0);
        let data = frame.data();
        let linesize = frame.linesize();
        let dst: [*mut u8; 4] = [out.as_mut_ptr(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()];
        let dst_stride: [c_int; 4] = [c_int::try_from(stride).unwrap_or(c_int::MAX), 0, 0, 0];
        // SAFETY: `frame` matches the context's source description (checked above), its
        // planes are owned by the frame; `out` holds exactly `dst_h` rows of `stride`.
        let rows = unsafe {
            (self.av.f.sws_scale)(
                self.ctx,
                data.as_ptr(),
                linesize.as_ptr(),
                0,
                self.src.1,
                dst.as_ptr(),
                dst_stride.as_ptr(),
            )
        };
        if rows <= 0 {
            return Err(super::av_err(self.av, if rows < 0 { rows } else { AVERROR_INVALIDDATA }));
        }
        Ok(())
    }
}

impl Drop for Scaler {
    fn drop(&mut self) {
        // SAFETY: `ctx` came from `sws_getContext`.
        unsafe { (self.av.f.sws_freeContext)(self.ctx) };
    }
}
