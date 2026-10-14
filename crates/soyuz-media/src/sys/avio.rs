//! A custom `AVIOContext` over a Rust reader ([`Avio`]): every source, files included,
//! is read through it, so FFmpeg never parses a user path as a protocol, no network
//! protocol can be reached from a playlist, extended-length Windows paths work through
//! `std::fs`, and cancellation (`stale`) is checked on every read.
//!
//! Callback rule (review must-fix 9): the body of every `extern "C"` callback runs inside
//! `catch_unwind`; a panic becomes `AVERROR_EXTERNAL` instead of unwinding into C (which
//! aborts the process).

use std::ffi::{c_int, c_void};
use std::io::{Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};

use super::layout::AVIOContextPrefix;
use super::loader::Av;
use super::types::*;
use crate::MediaError;

/// Bytes FFmpeg reads per callback.
const BUFFER_SIZE: usize = 64 * 1024;

pub(crate) trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

/// What the callbacks see through `opaque`.
pub(crate) struct IoState<'a> {
    reader: Box<dyn ReadSeek + 'a>,
    len: u64,
    stale: Option<&'a (dyn Fn() -> bool + 'a)>,
    /// `stale` returned `true`: FFmpeg got `AVERROR_EXIT`; the caller reports
    /// [`MediaError::Stale`].
    pub stopped: bool,
    /// The last I/O error of the reader (FFmpeg got `AVERROR(EIO)`).
    pub io_error: Option<std::io::Error>,
}

impl<'a> IoState<'a> {
    pub(crate) fn new(reader: Box<dyn ReadSeek + 'a>, len: u64, stale: Option<&'a (dyn Fn() -> bool + 'a)>) -> Self {
        IoState { reader, len, stale, stopped: false, io_error: None }
    }

    fn check_stale(&mut self) -> bool {
        if self.stale.is_some_and(|f| f()) {
            self.stopped = true;
            return true;
        }
        false
    }
}

const AVERROR_EIO: c_int = -5;

pub(crate) unsafe extern "C" fn read_packet(opaque: *mut c_void, buf: *mut u8, buf_size: c_int) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `opaque` is the `Box<IoState>` [`Avio::new`] registered, alive while the
        // context is (the box is dropped after `avio_context_free`); `buf` has `buf_size`
        // writable bytes by FFmpeg's contract.
        let st = unsafe { &mut *opaque.cast::<IoState<'_>>() };
        if st.check_stale() {
            return AVERROR_EXIT;
        }
        let Ok(n) = usize::try_from(buf_size) else { return AVERROR_EINVAL };
        if n == 0 {
            return 0;
        }
        // SAFETY: see above.
        let out = unsafe { std::slice::from_raw_parts_mut(buf, n) };
        loop {
            match st.reader.read(out) {
                Ok(0) => return AVERROR_EOF,
                Ok(read) => return c_int::try_from(read).unwrap_or(c_int::MAX),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    st.io_error = Some(e);
                    return AVERROR_EIO;
                }
            }
        }
    }))
    .unwrap_or(AVERROR_EXTERNAL)
}

pub(crate) unsafe extern "C" fn seek(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as in `read_packet`.
        let st = unsafe { &mut *opaque.cast::<IoState<'_>>() };
        if whence & AVSEEK_SIZE != 0 {
            return i64::try_from(st.len).unwrap_or(i64::MAX);
        }
        if st.check_stale() {
            return i64::from(AVERROR_EXIT);
        }
        let from = match whence & !AVSEEK_FORCE {
            SEEK_SET => match u64::try_from(offset) {
                Ok(o) => SeekFrom::Start(o),
                Err(_) => return i64::from(AVERROR_EINVAL),
            },
            SEEK_CUR => SeekFrom::Current(offset),
            SEEK_END => SeekFrom::End(offset),
            _ => return i64::from(AVERROR_EINVAL),
        };
        match st.reader.seek(from) {
            Ok(pos) => i64::try_from(pos).unwrap_or(i64::MAX),
            Err(e) => {
                st.io_error = Some(e);
                i64::from(AVERROR_EIO)
            }
        }
    }))
    .unwrap_or(i64::from(AVERROR_EXTERNAL))
}

/// An `AVIOContext` reading from a Rust reader; freed (with whatever buffer FFmpeg
/// holds by then) on drop.
pub(crate) struct Avio<'a> {
    av: &'static Av,
    ctx: *mut AVIOContext,
    state: Box<IoState<'a>>,
}

impl<'a> Avio<'a> {
    pub(crate) fn new(
        av: &'static Av,
        reader: Box<dyn ReadSeek + 'a>,
        len: u64,
        stale: Option<&'a (dyn Fn() -> bool + 'a)>,
    ) -> Result<Avio<'a>, MediaError> {
        let mut state = Box::new(IoState::new(reader, len, stale));
        // SAFETY: plain allocation; checked for NULL below.
        let buffer = unsafe { (av.f.av_malloc)(BUFFER_SIZE) }.cast::<u8>();
        if buffer.is_null() {
            return Err(super::av_err(av, AVERROR_ENOMEM));
        }
        let opaque: *mut IoState<'a> = &mut *state;
        // SAFETY: `buffer` is `BUFFER_SIZE` bytes from `av_malloc`, owned by the context
        // from now on; `opaque` points into the heap box that outlives the context (both
        // live in this struct, the context is freed first in `drop`).
        let ctx = unsafe {
            (av.f.avio_alloc_context)(
                buffer,
                BUFFER_SIZE as c_int,
                0,
                opaque.cast(),
                Some(read_packet),
                None,
                Some(seek),
            )
        };
        if ctx.is_null() {
            // SAFETY: the buffer was never handed over.
            unsafe { (av.f.av_free)(buffer.cast()) };
            return Err(super::av_err(av, AVERROR_ENOMEM));
        }
        Ok(Avio { av, ctx, state })
    }

    pub(crate) fn ctx(&self) -> *mut AVIOContext {
        self.ctx
    }

    pub(crate) fn state(&self) -> &IoState<'a> {
        &self.state
    }

    /// Map an FFmpeg error code to the reason our callbacks produced it, if they did.
    pub(crate) fn explain(&mut self, code: c_int) -> Option<MediaError> {
        if code == AVERROR_EXIT && self.state.stopped {
            return Some(MediaError::Stale);
        }
        if let Some(e) = self.state.io_error.take() {
            return Some(MediaError::Io(e));
        }
        None
    }
}

impl Drop for Avio<'_> {
    fn drop(&mut self) {
        // SAFETY: `ctx` came from `avio_alloc_context` and is not referenced by any
        // open AVFormatContext any more (the demuxer drops its context first). FFmpeg may
        // have replaced the buffer, so the one to free is read from the context.
        unsafe {
            let prefix = self.ctx.cast::<AVIOContextPrefix>();
            let buffer = (*prefix).buffer;
            (*prefix).buffer = std::ptr::null_mut();
            (self.av.f.av_free)(buffer.cast());
            (self.av.f.avio_context_free)(&mut self.ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct Panicky;
    impl Read for Panicky {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            panic!("reader panic");
        }
    }
    impl Seek for Panicky {
        fn seek(&mut self, _: SeekFrom) -> std::io::Result<u64> {
            panic!("seek panic");
        }
    }

    /// Review must-fix 9: a panicking reader yields an error code, never an unwind
    /// across the C boundary (the callbacks are called directly here, no library needed).
    #[test]
    fn panics_become_averror_external() {
        let mut st = IoState::new(Box::new(Panicky), 0, None);
        let opaque: *mut IoState<'_> = &mut st;
        let mut buf = [0u8; 16];
        // SAFETY: a valid state and buffer, exactly what FFmpeg would pass.
        let r = unsafe { read_packet(opaque.cast(), buf.as_mut_ptr(), buf.len() as c_int) };
        assert_eq!(r, AVERROR_EXTERNAL);
        // SAFETY: as above.
        let s = unsafe { seek(opaque.cast(), 0, SEEK_SET) };
        assert_eq!(s, i64::from(AVERROR_EXTERNAL));
    }

    #[test]
    fn reads_seeks_and_stops_when_stale() {
        let data: Vec<u8> = (0..100u8).collect();
        let stale = std::cell::Cell::new(false);
        let is_stale = || stale.get();
        let mut st = IoState::new(Box::new(Cursor::new(data)), 100, Some(&is_stale));
        let opaque: *mut IoState<'_> = &mut st;
        let mut buf = [0u8; 8];
        // SAFETY: a valid state and buffer.
        let r = unsafe { read_packet(opaque.cast(), buf.as_mut_ptr(), 8) };
        assert_eq!(r, 8);
        assert_eq!(buf, [0, 1, 2, 3, 4, 5, 6, 7]);
        // SAFETY: as above.
        let size = unsafe { seek(opaque.cast(), 0, AVSEEK_SIZE) };
        assert_eq!(size, 100);
        // SAFETY: as above.
        let pos = unsafe { seek(opaque.cast(), -4, SEEK_END | AVSEEK_FORCE) };
        assert_eq!(pos, 96);
        // SAFETY: as above.
        let r = unsafe { read_packet(opaque.cast(), buf.as_mut_ptr(), 8) };
        assert_eq!(r, 4, "four bytes left");
        // SAFETY: as above.
        let r = unsafe { read_packet(opaque.cast(), buf.as_mut_ptr(), 8) };
        assert_eq!(r, AVERROR_EOF, "EOF is an error code, never 0");
        stale.set(true);
        // SAFETY: as above.
        let r = unsafe { read_packet(opaque.cast(), buf.as_mut_ptr(), 8) };
        assert_eq!(r, AVERROR_EXIT);
        assert!(st.stopped);
        // SAFETY: as above.
        let r = unsafe { seek(opaque.cast(), 0, 99) };
        assert_eq!(r, i64::from(AVERROR_EXIT), "stale wins over the bad whence");
    }
}
