//! `SwrContext` ([`Resampler`]): decoded audio frames to interleaved `f32`.

use std::ffi::c_int;

use super::codec::{ChLayout, Frame, sample_fmt};
use super::loader::Av;
use super::ok;
use crate::MediaError;

pub(crate) struct Resampler {
    av: &'static Av,
    ctx: *mut super::types::SwrContext,
    /// Source description this context was built for.
    src: (c_int, c_int, u32),
    pub out_channels: u32,
    out: Vec<f32>,
}

// SAFETY: used from one thread at a time (`&mut self`); swresample contexts have no
// thread affinity.
unsafe impl Send for Resampler {}

impl Resampler {
    /// From the decoder's layout at `in_fmt` / `in_rate` to interleaved `f32` at
    /// `out_rate` with the same channels.
    pub(crate) fn new(
        av: &'static Av,
        layout: &ChLayout,
        in_fmt: c_int,
        in_rate: c_int,
        out_rate: u32,
    ) -> Result<Resampler, MediaError> {
        let flt = sample_fmt(av, "flt").ok_or_else(|| MediaError::Unsupported("no flt sample format".into()))?;
        if in_rate <= 0 || in_fmt < 0 || layout.l.nb_channels <= 0 || out_rate == 0 {
            return Err(MediaError::Unsupported(format!(
                "cannot resample {} channels at {in_rate} Hz (format {in_fmt})",
                layout.l.nb_channels
            )));
        }
        let mut ctx: *mut super::types::SwrContext = std::ptr::null_mut();
        let out_rate_i = c_int::try_from(out_rate).unwrap_or(c_int::MAX);
        // SAFETY: `ctx` receives a new context; both layouts are live and owned by the
        // caller for the duration of the call (swr copies them).
        let r = unsafe {
            (av.f.swr_alloc_set_opts2)(
                &mut ctx,
                &layout.l,
                flt,
                out_rate_i,
                &layout.l,
                in_fmt,
                in_rate,
                0,
                std::ptr::null_mut(),
            )
        };
        ok(av, r)?;
        let rs = Resampler {
            av,
            ctx,
            src: (in_fmt, in_rate, layout.channels()),
            out_channels: layout.channels(),
            out: Vec::new(),
        };
        // SAFETY: a configured context.
        ok(av, unsafe { (av.f.swr_init)(rs.ctx) })?;
        Ok(rs)
    }

    /// Whether the context fits a frame of this format / rate / channel count.
    pub(crate) fn matches(&self, in_fmt: c_int, in_rate: c_int, channels: u32) -> bool {
        self.src == (in_fmt, in_rate, channels)
    }

    /// Convert `frame` (or drain the delay line when `None`), appending interleaved
    /// `f32` frames to `out`; returns the frames appended.
    pub(crate) fn convert(&mut self, frame: Option<&Frame>, out: &mut Vec<f32>) -> Result<usize, MediaError> {
        let (input, in_count) = match frame {
            Some(f) => (f.extended_data(), f.nb_samples()),
            None => (std::ptr::null(), 0),
        };
        // SAFETY: a live context; a pure query.
        let want = unsafe { (self.av.f.swr_get_out_samples)(self.ctx, in_count) };
        let want = usize::try_from(want).unwrap_or(0).max(if frame.is_none() { 1024 } else { 0 });
        if want == 0 {
            return Ok(0);
        }
        let ch = self.out_channels as usize;
        self.out.clear();
        self.out.resize(want * ch, 0.0);
        let dst: [*mut u8; 1] = [self.out.as_mut_ptr().cast()];
        // SAFETY: `dst[0]` holds `want` interleaved frames of `ch` f32 samples; `input`
        // is the frame's plane array (`extended_data`, valid for `nb_samples`) or NULL.
        let got = unsafe {
            (self.av.f.swr_convert)(
                self.ctx,
                dst.as_ptr(),
                c_int::try_from(want).unwrap_or(c_int::MAX),
                input,
                in_count,
            )
        };
        let got = usize::try_from(ok(self.av, got)?).unwrap_or(0);
        out.extend_from_slice(&self.out[..got * ch]);
        Ok(got)
    }
}

impl Drop for Resampler {
    fn drop(&mut self) {
        // SAFETY: `ctx` came from `swr_alloc_set_opts2`.
        unsafe { (self.av.f.swr_free)(&mut self.ctx) };
    }
}
