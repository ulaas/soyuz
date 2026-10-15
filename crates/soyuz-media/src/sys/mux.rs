//! Muxing (V2c): an output `AVFormatContext` ([`Muxer`]) writing through a custom
//! `AVIOContext` over a Rust writer (no FFmpeg protocol parsing of the destination path,
//! extended-length Windows paths through `std::fs`, byte counting for progress), plus
//! the packet field writes the transcoder needs ([`pkt_set_stream_index`], …).
//!
//! Callback rule (review must-fix 9): every `extern "C"` callback body runs inside
//! `catch_unwind`; a panic becomes `AVERROR_EXTERNAL`.

use std::ffi::{c_char, c_int, c_uint, c_void};
use std::io::{Seek, SeekFrom, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};

use super::demux::{Demuxer, Packet};
use super::layout::*;
use super::loader::Av;
use super::types::*;
use super::{av_err, cstr, cstring, ok};
use crate::MediaError;

/// Bytes FFmpeg hands to the write callback at a time.
const BUFFER_SIZE: usize = 256 * 1024;
const AVERROR_EIO: c_int = -5;

/// `AVFMT_*` flags of an output format.
pub(crate) const AVFMT_NOFILE: c_int = 0x0001;
pub(crate) const AVFMT_GLOBALHEADER: c_int = 0x0040;

/// `AVOutputFormat` prefix: `name`, `long_name`, `mime_type`, `extensions`, the three
/// default codec ids and `flags` (identical on 7.1 / 8.x / 9.0; `codec_tag` and
/// `priv_class` after it are never read).
#[repr(C)]
pub(crate) struct AVOutputFormatPrefix {
    pub name: *const c_char,
    pub long_name: *const c_char,
    pub mime_type: *const c_char,
    pub extensions: *const c_char,
    pub audio_codec: c_int,
    pub video_codec: c_int,
    pub subtitle_codec: c_int,
    pub flags: c_int,
}

/// What [`output_format`] reports about a muxer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OutFormat {
    pub name: String,
    pub flags: c_int,
}

/// `av_guess_format(name)`: the muxer called `name`, if the build has it.
pub(crate) fn output_format(av: &Av, name: &str) -> Option<OutFormat> {
    let n = cstring(name);
    // SAFETY: a lookup in the static muxer table.
    let f = unsafe { (av.f.av_guess_format)(n.as_ptr(), std::ptr::null(), std::ptr::null()) };
    if f.is_null() {
        return None;
    }
    // SAFETY: a static format description; the prefix matches every supported major.
    let p = unsafe { &*f.cast::<AVOutputFormatPrefix>() };
    Some(OutFormat { name: cstr(p.name).unwrap_or_default(), flags: p.flags })
}

pub(crate) trait WriteSeek: Write + Seek + Send {}
impl<T: Write + Seek + Send> WriteSeek for T {}

/// What the write callbacks see through `opaque`.
struct WriteState {
    writer: Box<dyn WriteSeek>,
    /// Current position and the highest position written (the `AVSEEK_SIZE` answer).
    pos: u64,
    len: u64,
    io_error: Option<std::io::Error>,
}

unsafe extern "C" fn write_packet(opaque: *mut c_void, buf: *const u8, buf_size: c_int) -> c_int {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `opaque` is the `Box<WriteState>` [`WriteAvio::new`] registered, alive
        // while the context is; `buf` has `buf_size` readable bytes by FFmpeg's contract.
        let st = unsafe { &mut *opaque.cast::<WriteState>() };
        let Ok(n) = usize::try_from(buf_size) else { return AVERROR_EINVAL };
        if n == 0 {
            return 0;
        }
        // SAFETY: see above.
        let data = unsafe { std::slice::from_raw_parts(buf, n) };
        match st.writer.write_all(data) {
            Ok(()) => {
                st.pos += n as u64;
                st.len = st.len.max(st.pos);
                c_int::try_from(n).unwrap_or(c_int::MAX)
            }
            Err(e) => {
                st.io_error = Some(e);
                AVERROR_EIO
            }
        }
    }))
    .unwrap_or(AVERROR_EXTERNAL)
}

unsafe extern "C" fn seek(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as in `write_packet`.
        let st = unsafe { &mut *opaque.cast::<WriteState>() };
        if whence & AVSEEK_SIZE != 0 {
            return i64::try_from(st.len).unwrap_or(i64::MAX);
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
        match st.writer.seek(from) {
            Ok(pos) => {
                st.pos = pos;
                i64::try_from(pos).unwrap_or(i64::MAX)
            }
            Err(e) => {
                st.io_error = Some(e);
                i64::from(AVERROR_EIO)
            }
        }
    }))
    .unwrap_or(i64::from(AVERROR_EXTERNAL))
}

/// A writable `AVIOContext` over a Rust writer; freed on drop (after the format
/// context that used it).
struct WriteAvio {
    av: &'static Av,
    ctx: *mut AVIOContext,
    state: Box<WriteState>,
}

impl WriteAvio {
    fn new(av: &'static Av, writer: Box<dyn WriteSeek>) -> Result<WriteAvio, MediaError> {
        let mut state = Box::new(WriteState { writer, pos: 0, len: 0, io_error: None });
        // SAFETY: plain allocation; checked for NULL below.
        let buffer = unsafe { (av.f.av_malloc)(BUFFER_SIZE) }.cast::<u8>();
        if buffer.is_null() {
            return Err(av_err(av, AVERROR_ENOMEM));
        }
        let opaque: *mut WriteState = &mut *state;
        // SAFETY: `buffer` is `BUFFER_SIZE` bytes from `av_malloc`, owned by the context
        // from now on; `opaque` points into the heap box that outlives the context (the
        // context is freed first in `drop`).
        let ctx = unsafe {
            (av.f.avio_alloc_context)(
                buffer,
                BUFFER_SIZE as c_int,
                1,
                opaque.cast(),
                None,
                Some(write_packet),
                Some(seek),
            )
        };
        if ctx.is_null() {
            // SAFETY: the buffer was never handed over.
            unsafe { (av.f.av_free)(buffer.cast()) };
            return Err(av_err(av, AVERROR_ENOMEM));
        }
        Ok(WriteAvio { av, ctx, state })
    }

    /// Map an FFmpeg error code to the writer's own error, if it caused it.
    fn explain(&mut self, code: c_int) -> MediaError {
        self.state.io_error.take().map_or_else(|| av_err(self.av, code), MediaError::Io)
    }
}

impl Drop for WriteAvio {
    fn drop(&mut self) {
        // SAFETY: `ctx` came from `avio_alloc_context` and no format context references
        // it any more (the muxer frees its context first). FFmpeg may have replaced the
        // buffer, so the one to free is read from the context.
        unsafe {
            let prefix = self.ctx.cast::<AVIOContextPrefix>();
            let buffer = (*prefix).buffer;
            (*prefix).buffer = std::ptr::null_mut();
            (self.av.f.av_free)(buffer.cast());
            (self.av.f.avio_context_free)(&mut self.ctx);
        }
    }
}

/// A stream to add to a [`Muxer`].
pub(crate) struct OutStream<'a> {
    /// Codec parameters to copy (an input stream's, or an opened encoder's).
    pub par: *const AVCodecParameters,
    pub time_base: AVRational,
    pub avg_frame_rate: AVRational,
    pub sample_aspect_ratio: AVRational,
    pub disposition: c_int,
    /// Stream tags (`language`, `title`, …).
    pub tags: &'a [(String, String)],
    /// Reset `codec_tag` (a FourCC of another container must not survive a remux).
    pub clear_tag: bool,
}

/// A chapter for [`Muxer::set_chapters`]: seconds of the output, every tag (`title`, …).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OutChapter {
    pub start: f64,
    pub end: f64,
    pub tags: Vec<(String, String)>,
}

/// The chapters of an opened input, in seconds of its own clock, with all their tags.
pub(crate) fn input_chapters(av: &Av, d: &Demuxer<'_>) -> Vec<OutChapter> {
    // SAFETY: a live input context (the demuxer owns it for its lifetime).
    let c = unsafe { &*d.as_ptr() };
    let mut out = Vec::new();
    if c.chapters.is_null() {
        return out;
    }
    for i in 0..c.nb_chapters as usize {
        // SAFETY: `i < nb_chapters`; the chapters are owned by the context.
        let ch = unsafe { *c.chapters.add(i) };
        if ch.is_null() {
            continue;
        }
        // SAFETY: a live chapter.
        let ch = unsafe { &*ch };
        let Some(start) = ch.time_base.secs(ch.start) else { continue };
        let end = ch.time_base.secs(ch.end).filter(|e| *e >= start).unwrap_or(start);
        out.push(OutChapter { start, end, tags: super::dict_entries(av, ch.metadata) });
    }
    out
}

/// Chapter times are written in microseconds.
const CHAPTER_TB: AVRational = AVRational { num: 1, den: 1_000_000 };

/// An output context writing through [`WriteAvio`]. Field order: the context is freed
/// before the AVIO it wrote through.
pub(crate) struct Muxer {
    av: &'static Av,
    ctx: *mut AVFormatContext,
    avio: WriteAvio,
    format: OutFormat,
    header: bool,
    trailer: bool,
}

// SAFETY: used from one thread at a time (`&mut self`); muxers are single-threaded and
// the writer is `Send` by the bound on `WriteSeek`.
unsafe impl Send for Muxer {}

impl Muxer {
    /// A context for the muxer `format` writing into `writer` (which must seek: MP4 /
    /// MOV write their index at the end by seeking back).
    pub(crate) fn new(av: &'static Av, format: &str, writer: Box<dyn WriteSeek>) -> Result<Muxer, MediaError> {
        let of = output_format(av, format).ok_or_else(|| MediaError::Unsupported(format!("no muxer {format}")))?;
        if of.flags & AVFMT_NOFILE != 0 {
            return Err(MediaError::Unsupported(format!("{format} does not write a single file")));
        }
        let avio = WriteAvio::new(av, writer)?;
        let name = cstring(format);
        let mut ctx: *mut AVFormatContext = std::ptr::null_mut();
        // SAFETY: `ctx` receives a new output context for the named format (NULL
        // filename: the custom `pb` is the only output).
        let r = unsafe {
            (av.f.avformat_alloc_output_context2)(&mut ctx, std::ptr::null(), name.as_ptr(), std::ptr::null())
        };
        if r < 0 || ctx.is_null() {
            return Err(av_err(av, if r < 0 { r } else { AVERROR_ENOMEM }));
        }
        // SAFETY: a fresh output context; `pb` is the one field written before the header
        // (the prefix struct matches every supported major).
        unsafe { (*ctx).pb = avio.ctx };
        Ok(Muxer { av, ctx, avio, format: of, header: false, trailer: false })
    }

    /// `AVFMT_GLOBALHEADER`: encoders must put their headers in `extradata`.
    pub(crate) fn needs_global_header(&self) -> bool {
        self.format.flags & AVFMT_GLOBALHEADER != 0
    }

    fn get(&self) -> &AVFormatContext {
        // SAFETY: `ctx` is a live output context (freed only in `drop`).
        unsafe { &*self.ctx }
    }

    fn stream_ptr(&self, i: usize) -> Option<*mut AVStream> {
        let c = self.get();
        if i >= c.nb_streams as usize || c.streams.is_null() {
            return None;
        }
        // SAFETY: `i < nb_streams`, the array is owned by the context.
        let p = unsafe { *c.streams.add(i) };
        (!p.is_null()).then_some(p)
    }

    /// Add a stream before the header is written; returns its output index.
    pub(crate) fn add_stream(&mut self, s: &OutStream<'_>) -> Result<usize, MediaError> {
        if self.header {
            return Err(MediaError::Unsupported("stream added after the header".into()));
        }
        // SAFETY: a live output context; NULL codec (the parameters say everything).
        let st = unsafe { (self.av.f.avformat_new_stream)(self.ctx, std::ptr::null()) };
        if st.is_null() {
            return Err(av_err(self.av, AVERROR_ENOMEM));
        }
        // SAFETY: a stream the context just allocated (prefix struct through
        // `attached_pic`); `codecpar` was allocated with it; the muxer reads these
        // fields in `write_header`.
        unsafe {
            let stream = &mut *st;
            ok(self.av, (self.av.f.avcodec_parameters_copy)(stream.codecpar, s.par))?;
            if s.clear_tag {
                (*stream.codecpar).codec_tag = 0;
            }
            stream.time_base = s.time_base;
            stream.avg_frame_rate = s.avg_frame_rate;
            stream.sample_aspect_ratio = s.sample_aspect_ratio;
            stream.disposition = s.disposition;
            for (k, v) in s.tags {
                let (kc, vc) = (cstring(k), cstring(v));
                (self.av.f.av_dict_set)(&mut stream.metadata, kc.as_ptr(), vc.as_ptr(), 0);
            }
            Ok(usize::try_from(stream.index).unwrap_or(0))
        }
    }

    /// An attachment stream (Matroska): `data` becomes its `extradata`; `tags` must name
    /// a `filename` and should give a `mimetype`. Returns its output index.
    pub(crate) fn add_attachment(
        &mut self,
        codec_id: c_int,
        data: &[u8],
        tags: &[(String, String)],
    ) -> Result<usize, MediaError> {
        if self.header {
            return Err(MediaError::Unsupported("stream added after the header".into()));
        }
        let pad = super::encode::AV_INPUT_BUFFER_PADDING_SIZE;
        // SAFETY: a live output context; NULL codec.
        let st = unsafe { (self.av.f.avformat_new_stream)(self.ctx, std::ptr::null()) };
        if st.is_null() {
            return Err(av_err(self.av, AVERROR_ENOMEM));
        }
        // SAFETY: plain allocation of the bytes plus FFmpeg's zeroed padding.
        let buf = unsafe { (self.av.f.av_malloc)(data.len() + pad) }.cast::<u8>();
        if buf.is_null() {
            return Err(av_err(self.av, AVERROR_ENOMEM));
        }
        // SAFETY: `buf` has `data.len() + pad` bytes; the stream was just allocated with
        // its `codecpar`, which owns `buf` from now on (`avcodec_parameters_free` frees
        // `extradata` when the context is freed).
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), buf, data.len());
            std::ptr::write_bytes(buf.add(data.len()), 0, pad);
            let stream = &mut *st;
            let par = &mut *stream.codecpar;
            par.codec_type = AVMEDIA_TYPE_ATTACHMENT;
            par.codec_id = codec_id;
            par.extradata = buf;
            par.extradata_size = c_int::try_from(data.len()).unwrap_or(c_int::MAX);
            for (k, v) in tags {
                let (kc, vc) = (cstring(k), cstring(v));
                (self.av.f.av_dict_set)(&mut stream.metadata, kc.as_ptr(), vc.as_ptr(), 0);
            }
            Ok(usize::try_from(stream.index).unwrap_or(0))
        }
    }

    /// Container-level tags (`title`, `artist`, …).
    pub(crate) fn set_metadata(&mut self, tags: &[(String, String)]) {
        for (k, v) in tags {
            let (kc, vc) = (cstring(k), cstring(v));
            // SAFETY: a live output context; `metadata` is its dictionary (NULL = empty).
            unsafe { (self.av.f.av_dict_set)(&mut (*self.ctx).metadata, kc.as_ptr(), vc.as_ptr(), 0) };
        }
    }

    /// The output's chapters (before the header; muxers without chapters ignore them).
    /// The array and each `AVChapter` are allocated with `av_malloc`, so
    /// `avformat_free_context` frees them like a demuxer's own.
    pub(crate) fn set_chapters(&mut self, chapters: &[OutChapter]) -> Result<(), MediaError> {
        if self.header {
            return Err(MediaError::Unsupported("chapters set after the header".into()));
        }
        if chapters.is_empty() || self.get().nb_chapters != 0 {
            return Ok(());
        }
        let n = chapters.len().min(c_uint::MAX as usize);
        let av = self.av;
        // SAFETY: plain allocation of `n` pointers; checked for NULL.
        let array = unsafe { (av.f.av_malloc)(n * std::mem::size_of::<*mut AVChapter>()) }.cast::<*mut AVChapter>();
        if array.is_null() {
            return Err(av_err(av, AVERROR_ENOMEM));
        }
        let mut filled = 0usize;
        for (i, c) in chapters.iter().take(n).enumerate() {
            // SAFETY: plain allocation of one chapter; checked for NULL.
            let ch = unsafe { (av.f.av_malloc)(std::mem::size_of::<AVChapter>()) }.cast::<AVChapter>();
            if ch.is_null() {
                break;
            }
            let secs = |s: f64| (s * f64::from(CHAPTER_TB.den)).round() as i64;
            // SAFETY: `ch` is a fresh allocation of the chapter's size; `array` has room
            // for `n` pointers and `i < n`.
            unsafe {
                ch.write(AVChapter {
                    id: i as i64 + 1,
                    time_base: CHAPTER_TB,
                    start: secs(c.start),
                    end: secs(c.end),
                    metadata: std::ptr::null_mut(),
                });
                for (k, v) in &c.tags {
                    let (kc, vc) = (cstring(k), cstring(v));
                    (av.f.av_dict_set)(&mut (*ch).metadata, kc.as_ptr(), vc.as_ptr(), 0);
                }
                *array.add(i) = ch;
            }
            filled = i + 1;
        }
        // SAFETY: a fresh output context without chapters; it owns the array from now on
        // (`avformat_free_context` frees `filled` chapters, their tags and the array).
        unsafe {
            (*self.ctx).chapters = array;
            (*self.ctx).nb_chapters = filled as c_uint;
        }
        Ok(())
    }

    /// `avformat_write_header` with muxer options (`movflags`, …); unknown options are
    /// returned as warnings.
    pub(crate) fn write_header(&mut self, options: &[(&str, &str)]) -> Result<Vec<String>, MediaError> {
        let mut dict: *mut AVDictionary = std::ptr::null_mut();
        for (k, v) in options {
            let (kc, vc) = (cstring(k), cstring(v));
            // SAFETY: `dict` is ours (NULL = new); strings are copied.
            unsafe { (self.av.f.av_dict_set)(&mut dict, kc.as_ptr(), vc.as_ptr(), 0) };
        }
        // SAFETY: a configured output context with streams; `dict` is consumed (unknown
        // entries are left in it) and freed below.
        let r = unsafe { (self.av.f.avformat_write_header)(self.ctx, &mut dict) };
        let mut warnings = Vec::new();
        for (k, v) in super::dict_entries(self.av, dict) {
            warnings.push(format!("{}: option {k}={v} not applied", self.format.name));
        }
        // SAFETY: freeing our dictionary (NULL is fine).
        unsafe { (self.av.f.av_dict_free)(&mut dict) };
        if r < 0 {
            return Err(self.avio.explain(r));
        }
        self.header = true;
        Ok(warnings)
    }

    /// The time base the muxer chose for stream `i` (valid after the header).
    pub(crate) fn stream_time_base(&self, i: usize) -> AVRational {
        // SAFETY: an element of the live context's `streams` array.
        self.stream_ptr(i).map_or(AVRational::new(0, 1), |p| unsafe { (*p).time_base })
    }

    /// `av_interleaved_write_frame`: the packet's timestamps must already be in the
    /// stream's time base; its payload is taken (the packet is blank afterwards).
    pub(crate) fn write(&mut self, pkt: &mut Packet) -> Result<(), MediaError> {
        // SAFETY: a context with the header written and a live packet.
        let r = unsafe { (self.av.f.av_interleaved_write_frame)(self.ctx, pkt.as_mut_ptr()) };
        if r < 0 {
            return Err(self.avio.explain(r));
        }
        Ok(())
    }

    /// `av_write_trailer` and flush.
    pub(crate) fn finish(&mut self) -> Result<(), MediaError> {
        if !self.header || self.trailer {
            return Ok(());
        }
        self.trailer = true;
        // SAFETY: a context with the header written.
        let r = unsafe { (self.av.f.av_write_trailer)(self.ctx) };
        if r < 0 {
            return Err(self.avio.explain(r));
        }
        // SAFETY: a live AVIO context.
        unsafe { (self.av.f.avio_flush)(self.avio.ctx) };
        if let Some(e) = self.avio.state.io_error.take() {
            return Err(MediaError::Io(e));
        }
        self.avio.state.writer.flush()?;
        Ok(())
    }

    /// The output's size so far (the furthest byte written; muxers that seek back to
    /// rewrite a header do not count twice).
    pub(crate) fn bytes_written(&self) -> u64 {
        self.avio.state.len
    }
}

impl Drop for Muxer {
    fn drop(&mut self) {
        // SAFETY: `ctx` came from `avformat_alloc_output_context2`; freeing it does not
        // touch the custom `pb`, which `avio` frees right after (field order).
        unsafe { (self.av.f.avformat_free_context)(self.ctx) };
    }
}

/// `pkt->stream_index = i`.
pub(crate) fn pkt_set_stream_index(pkt: &mut Packet, i: usize) {
    // SAFETY: a live packet; `AVPacket` is a complete struct in every supported major.
    unsafe { (*pkt.as_mut_ptr()).stream_index = c_int::try_from(i).unwrap_or(0) };
}

/// `pkt->pts`, `pkt->dts` (`AV_NOPTS_VALUE` allowed).
pub(crate) fn pkt_set_ts(pkt: &mut Packet, pts: i64, dts: i64) {
    // SAFETY: as above.
    unsafe {
        let p = &mut *pkt.as_mut_ptr();
        p.pts = pts;
        p.dts = dts;
    }
}

/// `pkt->pos = -1` (a position from the input means nothing in the output).
pub(crate) fn pkt_clear_pos(pkt: &mut Packet) {
    // SAFETY: as above.
    unsafe { (*pkt.as_mut_ptr()).pos = -1 };
}

/// `av_packet_rescale_ts`: pts, dts and duration from `from` to `to`.
pub(crate) fn pkt_rescale_ts(av: &Av, pkt: &mut Packet, from: AVRational, to: AVRational) {
    // SAFETY: a live packet and two valid time bases.
    unsafe { (av.f.av_packet_rescale_ts)(pkt.as_mut_ptr(), from, to) };
}

/// `av_rescale_q_rnd(a, from, to)` with `AV_ROUND_NEAR_INF | AV_ROUND_PASS_MINMAX`.
pub(crate) fn rescale(av: &Av, a: i64, from: AVRational, to: AVRational) -> i64 {
    if a == AV_NOPTS_VALUE || from.den == 0 || to.den == 0 || from.num == 0 || to.num == 0 {
        return a;
    }
    // SAFETY: pure arithmetic.
    unsafe { (av.f.av_rescale_q_rnd)(a, from, to, AV_ROUND_NEAR_INF | AV_ROUND_PASS_MINMAX) }
}

const AV_ROUND_NEAR_INF: c_int = 5;
const AV_ROUND_PASS_MINMAX: c_int = 8192;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn output_format_prefix_offsets() {
        assert_eq!(std::mem::offset_of!(AVOutputFormatPrefix, extensions), 24);
        assert_eq!(std::mem::offset_of!(AVOutputFormatPrefix, audio_codec), 32);
        assert_eq!(std::mem::offset_of!(AVOutputFormatPrefix, video_codec), 36);
        assert_eq!(std::mem::offset_of!(AVOutputFormatPrefix, flags), 44);
        assert_eq!(AVFMT_GLOBALHEADER | AVFMT_NOFILE, 0x41);
    }

    struct Panicky;
    impl Write for Panicky {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            panic!("write panic");
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Seek for Panicky {
        fn seek(&mut self, _: SeekFrom) -> std::io::Result<u64> {
            panic!("seek panic");
        }
    }

    /// The callbacks are called directly (no library needed): a panicking writer yields
    /// an error code, a cursor records what was written and where.
    #[test]
    fn write_callbacks_without_libraries() {
        let mut st = WriteState { writer: Box::new(Panicky), pos: 0, len: 0, io_error: None };
        let opaque: *mut WriteState = &mut st;
        let buf = [1u8, 2, 3];
        // SAFETY: a valid state and buffer, exactly what FFmpeg would pass.
        let r = unsafe { write_packet(opaque.cast(), buf.as_ptr(), 3) };
        assert_eq!(r, AVERROR_EXTERNAL);
        // SAFETY: as above.
        assert_eq!(unsafe { seek(opaque.cast(), 0, SEEK_SET) }, i64::from(AVERROR_EXTERNAL));

        let mut st = WriteState { writer: Box::new(Cursor::new(Vec::new())), pos: 0, len: 0, io_error: None };
        let opaque: *mut WriteState = &mut st;
        // SAFETY: as above.
        assert_eq!(unsafe { write_packet(opaque.cast(), buf.as_ptr(), 3) }, 3);
        // SAFETY: as above.
        assert_eq!(unsafe { seek(opaque.cast(), 1, SEEK_SET) }, 1);
        let two = [9u8];
        // SAFETY: as above.
        assert_eq!(unsafe { write_packet(opaque.cast(), two.as_ptr(), 1) }, 1);
        // SAFETY: as above.
        assert_eq!(unsafe { seek(opaque.cast(), 0, AVSEEK_SIZE) }, 3, "the size is the furthest byte written");
        // SAFETY: as above.
        assert_eq!(unsafe { seek(opaque.cast(), 0, SEEK_END | AVSEEK_FORCE) }, 3);
        assert_eq!(st.len, 3, "rewritten bytes do not count twice");
        assert_eq!(st.pos, 3);
        // SAFETY: as above.
        assert_eq!(unsafe { seek(opaque.cast(), 0, 99) }, i64::from(AVERROR_EINVAL));
        // SAFETY: as above.
        assert_eq!(unsafe { write_packet(opaque.cast(), buf.as_ptr(), 0) }, 0);
    }
}
