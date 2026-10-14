//! `AVFormatContext` ([`Demuxer`]) over a custom AVIO source, and `AVPacket`
//! ([`Packet`]).

use std::ffi::{c_int, c_void};
use std::io::Cursor;
use std::sync::Arc;

use super::avio::Avio;
use super::layout::*;
use super::loader::Av;
use super::types::*;
use super::{av_err, cstr, dict_entries, dict_get, opt_set_int};
use crate::MediaError;
use crate::info::{Source, StreamKind};

/// Limits applied before a file is opened (file contents are untrusted; review must-fix
/// 8b). The defaults suit probing and single-frame work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limits {
    /// `probesize` (bytes read to detect the format and stream parameters).
    pub probesize: i64,
    /// `analyzeduration` (microseconds of input examined by `find_stream_info`).
    pub analyze_us: i64,
    /// `max_streams`.
    pub max_streams: i64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { probesize: 16 << 20, analyze_us: 5 * AV_TIME_BASE, max_streams: 64 }
    }
}

/// One `AVPacket`.
pub(crate) struct Packet {
    av: &'static Av,
    ptr: *mut AVPacket,
}

// SAFETY: a packet is plain heap data owned by this handle; FFmpeg has no thread
// affinity for packets, so moving the handle to another thread is fine.
unsafe impl Send for Packet {}

#[allow(dead_code)] // the timestamp / position accessors serve the player (V2a) and transcoder (V2c)
impl Packet {
    pub(crate) fn new(av: &'static Av) -> Result<Packet, MediaError> {
        // SAFETY: plain allocation, NULL-checked.
        let ptr = unsafe { (av.f.av_packet_alloc)() };
        if ptr.is_null() {
            return Err(av_err(av, AVERROR_ENOMEM));
        }
        Ok(Packet { av, ptr })
    }

    pub(crate) fn as_ptr(&self) -> *const AVPacket {
        self.ptr
    }

    pub(crate) fn as_mut_ptr(&mut self) -> *mut AVPacket {
        self.ptr
    }

    fn get(&self) -> &AVPacket {
        // SAFETY: `ptr` is a live packet (allocated in `new`, freed only in `drop`), and
        // `AVPacket` matches the library's struct in every supported major.
        unsafe { &*self.ptr }
    }

    pub(crate) fn stream_index(&self) -> usize {
        usize::try_from(self.get().stream_index).unwrap_or(usize::MAX)
    }

    pub(crate) fn pts(&self) -> i64 {
        self.get().pts
    }

    pub(crate) fn dts(&self) -> i64 {
        self.get().dts
    }

    pub(crate) fn duration(&self) -> i64 {
        self.get().duration
    }

    pub(crate) fn pos(&self) -> i64 {
        self.get().pos
    }

    pub(crate) fn is_key(&self) -> bool {
        self.get().flags & AV_PKT_FLAG_KEY != 0
    }

    pub(crate) fn size(&self) -> usize {
        usize::try_from(self.get().size).unwrap_or(0)
    }

    /// Drop the payload (the packet can be reused).
    pub(crate) fn unref(&mut self) {
        // SAFETY: `ptr` is a live packet.
        unsafe { (self.av.f.av_packet_unref)(self.ptr) };
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from `av_packet_alloc`; `av_packet_free` unrefs and frees it.
        unsafe { (self.av.f.av_packet_free)(&mut self.ptr) };
    }
}

/// An open input: the format context plus the AVIO source it reads through. The context
/// is closed before the source is freed (field order).
pub(crate) struct Demuxer<'a> {
    av: &'static Av,
    ctx: *mut AVFormatContext,
    avio: Avio<'a>,
    /// Source size in bytes.
    pub size: u64,
}

// SAFETY: the demuxer is used from one thread at a time (every method takes `&self` or
// `&mut self` and nothing is shared with FFmpeg threads: demuxers are single-threaded);
// its reader is `Send` by the bound on `ReadSeek`.
unsafe impl Send for Demuxer<'_> {}

/// A stream of a [`Demuxer`] (reads go through the `AVStream` prefix struct).
#[derive(Clone, Copy)]
pub(crate) struct StreamRef<'d> {
    av: &'static Av,
    ptr: *mut AVStream,
    _d: std::marker::PhantomData<&'d ()>,
}

#[allow(dead_code)] // `as_ptr`, `nb_frames`, … serve the transcoder (V2c)
impl StreamRef<'_> {
    fn get(&self) -> &AVStream {
        // SAFETY: `ptr` is an element of the live context's `streams` array; the prefix
        // struct matches every supported major through `attached_pic`.
        unsafe { &*self.ptr }
    }

    pub(crate) fn as_ptr(&self) -> *mut AVStream {
        self.ptr
    }

    pub(crate) fn index(&self) -> usize {
        usize::try_from(self.get().index).unwrap_or(0)
    }

    pub(crate) fn time_base(&self) -> AVRational {
        self.get().time_base
    }

    pub(crate) fn start_time(&self) -> i64 {
        self.get().start_time
    }

    pub(crate) fn duration(&self) -> i64 {
        self.get().duration
    }

    pub(crate) fn nb_frames(&self) -> i64 {
        self.get().nb_frames
    }

    pub(crate) fn disposition(&self) -> c_int {
        self.get().disposition
    }

    pub(crate) fn sample_aspect_ratio(&self) -> AVRational {
        self.get().sample_aspect_ratio
    }

    pub(crate) fn avg_frame_rate(&self) -> AVRational {
        self.get().avg_frame_rate
    }

    /// The codec parameters (never null for a stream of an opened context).
    pub(crate) fn par(&self) -> &AVCodecParameters {
        // SAFETY: `codecpar` is allocated with every stream and freed with the context.
        unsafe { &*self.get().codecpar }
    }

    pub(crate) fn par_ptr(&self) -> *const AVCodecParameters {
        self.get().codecpar
    }

    pub(crate) fn kind(&self) -> Option<StreamKind> {
        kind_of(self.par().codec_type)
    }

    pub(crate) fn metadata(&self) -> Vec<(String, String)> {
        dict_entries(self.av, self.get().metadata)
    }

    pub(crate) fn tag(&self, key: &str) -> Option<String> {
        dict_get(self.av, self.get().metadata, key)
    }

    /// The attached picture's bytes (cover art), when the stream is one.
    pub(crate) fn attached_pic(&self) -> Option<Vec<u8>> {
        let s = self.get();
        if s.disposition & AV_DISPOSITION_ATTACHED_PIC == 0 || s.attached_pic.data.is_null() {
            return None;
        }
        let n = usize::try_from(s.attached_pic.size).ok().filter(|n| *n > 0)?;
        // SAFETY: `attached_pic` is a packet owned by the stream with `size` valid bytes.
        Some(unsafe { std::slice::from_raw_parts(s.attached_pic.data, n) }.to_vec())
    }

    /// The side data of `kind` from the codec parameters (`AV_PKT_DATA_*`).
    pub(crate) fn side_data(&self, kind: c_int) -> Option<&[u8]> {
        let par = self.par();
        if par.coded_side_data.is_null() || par.nb_coded_side_data <= 0 {
            return None;
        }
        // SAFETY: `coded_side_data` has `nb_coded_side_data` entries owned by the params.
        let sd = unsafe { (self.av.f.av_packet_side_data_get)(par.coded_side_data, par.nb_coded_side_data, kind) };
        if sd.is_null() {
            return None;
        }
        // SAFETY: a returned entry has `size` valid bytes at `data`.
        unsafe {
            let sd = &*sd;
            if sd.data.is_null() || sd.size == 0 {
                return None;
            }
            Some(std::slice::from_raw_parts(sd.data, sd.size))
        }
    }
}

pub(crate) fn kind_of(codec_type: c_int) -> Option<StreamKind> {
    match codec_type {
        AVMEDIA_TYPE_VIDEO => Some(StreamKind::Video),
        AVMEDIA_TYPE_AUDIO => Some(StreamKind::Audio),
        AVMEDIA_TYPE_SUBTITLE => Some(StreamKind::Subtitle),
        AVMEDIA_TYPE_DATA => Some(StreamKind::Data),
        AVMEDIA_TYPE_ATTACHMENT => Some(StreamKind::Attachment),
        _ => None,
    }
}

pub(crate) fn media_type(kind: StreamKind) -> c_int {
    match kind {
        StreamKind::Video => AVMEDIA_TYPE_VIDEO,
        StreamKind::Audio => AVMEDIA_TYPE_AUDIO,
        StreamKind::Subtitle => AVMEDIA_TYPE_SUBTITLE,
        StreamKind::Data => AVMEDIA_TYPE_DATA,
        StreamKind::Attachment => AVMEDIA_TYPE_ATTACHMENT,
    }
}

impl<'a> Demuxer<'a> {
    /// Open `src` through the custom AVIO layer and read the stream information.
    pub(crate) fn open(
        av: &'static Av,
        src: &Source,
        stale: Option<&'a (dyn Fn() -> bool + 'a)>,
        limits: Limits,
    ) -> Result<Demuxer<'a>, MediaError> {
        let (reader, size): (Box<dyn super::avio::ReadSeek + 'a>, u64) = match src {
            Source::File(path) => {
                let f = std::fs::File::open(path)?;
                let len = f.metadata()?.len();
                (Box::new(f), len)
            }
            Source::Mem(bytes) => (Box::new(Cursor::new(Arc::clone(bytes))), bytes.len() as u64),
        };
        let mut avio = Avio::new(av, reader, size, stale)?;
        // SAFETY: plain allocation, NULL-checked.
        let mut ctx = unsafe { (av.f.avformat_alloc_context)() };
        if ctx.is_null() {
            return Err(av_err(av, AVERROR_ENOMEM));
        }
        // SAFETY: `ctx` is a fresh context; `pb` is the one field written before the open
        // (`avformat_open_input` then sets AVFMT_FLAG_CUSTOM_IO itself).
        unsafe { (*ctx).pb = avio.ctx() };
        let obj = ctx.cast::<c_void>();
        let set = |name: &str, val: i64| opt_set_int(av, obj, name, val);
        if let Err(e) = set("probesize", limits.probesize)
            .and_then(|()| set("analyzeduration", limits.analyze_us))
            .and_then(|()| set("max_streams", limits.max_streams))
        {
            // SAFETY: the context was never opened; `pb` is ours (freed by `avio`).
            unsafe {
                (*ctx).pb = std::ptr::null_mut();
                (av.f.avformat_free_context)(ctx);
            }
            return Err(e);
        }
        // SAFETY: `ctx` is ours; an empty URL with a custom `pb` reads through our
        // callbacks only. On failure FFmpeg frees the context and NULLs `ctx` (the custom
        // AVIOContext is left to us, as documented for AVFMT_FLAG_CUSTOM_IO).
        let r = unsafe { (av.f.avformat_open_input)(&mut ctx, c"".as_ptr(), std::ptr::null(), std::ptr::null_mut()) };
        if r < 0 {
            return Err(avio.explain(r).unwrap_or_else(|| av_err(av, r)));
        }
        let mut d = Demuxer { av, ctx, avio, size };
        // SAFETY: an opened context.
        let r = unsafe { (av.f.avformat_find_stream_info)(d.ctx, std::ptr::null_mut()) };
        if r < 0 {
            return Err(d.avio.explain(r).unwrap_or_else(|| av_err(av, r)));
        }
        Ok(d)
    }

    #[allow(dead_code)] // the transcoder (V2c)
    pub(crate) fn as_ptr(&self) -> *mut AVFormatContext {
        self.ctx
    }

    fn get(&self) -> &AVFormatContext {
        // SAFETY: `ctx` is an opened context (closed only in `drop`); the prefix struct
        // matches every supported major through `interrupt_callback`.
        unsafe { &*self.ctx }
    }

    pub(crate) fn stream_count(&self) -> usize {
        self.get().nb_streams as usize
    }

    pub(crate) fn stream(&self, i: usize) -> Option<StreamRef<'_>> {
        let c = self.get();
        if i >= c.nb_streams as usize || c.streams.is_null() {
            return None;
        }
        // SAFETY: `i < nb_streams`, the array is owned by the context.
        let ptr = unsafe { *c.streams.add(i) };
        (!ptr.is_null()).then_some(StreamRef { av: self.av, ptr, _d: std::marker::PhantomData })
    }

    pub(crate) fn streams(&self) -> impl Iterator<Item = StreamRef<'_>> + '_ {
        (0..self.stream_count()).filter_map(move |i| self.stream(i))
    }

    /// `(name, long_name)` of the input format.
    pub(crate) fn format(&self) -> (String, String) {
        let f = self.get().iformat;
        if f.is_null() {
            return (String::new(), String::new());
        }
        // SAFETY: `iformat` points at a static format description.
        unsafe { (cstr((*f).name).unwrap_or_default(), cstr((*f).long_name).unwrap_or_default()) }
    }

    /// Duration in seconds, from the container or (failing that) the longest stream.
    pub(crate) fn duration(&self) -> Option<f64> {
        let c = self.get();
        if c.duration != AV_NOPTS_VALUE && c.duration > 0 {
            return Some(c.duration as f64 / AV_TIME_BASE as f64);
        }
        self.streams().filter_map(|s| s.time_base().secs(s.duration())).filter(|d| *d > 0.0).reduce(f64::max)
    }

    pub(crate) fn start_time(&self) -> f64 {
        let c = self.get();
        if c.start_time == AV_NOPTS_VALUE { 0.0 } else { c.start_time as f64 / AV_TIME_BASE as f64 }
    }

    pub(crate) fn bit_rate(&self) -> Option<u64> {
        u64::try_from(self.get().bit_rate).ok().filter(|b| *b > 0)
    }

    pub(crate) fn metadata(&self) -> Vec<(String, String)> {
        dict_entries(self.av, self.get().metadata)
    }

    /// `(start, end, title)` of every chapter, in seconds.
    pub(crate) fn chapters(&self) -> Vec<(f64, f64, Option<String>)> {
        let c = self.get();
        let mut out = Vec::new();
        if c.chapters.is_null() {
            return out;
        }
        for i in 0..c.nb_chapters as usize {
            // SAFETY: `i < nb_chapters`; chapters are owned by the context.
            let ch = unsafe { *c.chapters.add(i) };
            if ch.is_null() {
                continue;
            }
            // SAFETY: a live chapter.
            let ch = unsafe { &*ch };
            let tb = ch.time_base;
            let start = tb.secs(ch.start).unwrap_or(0.0);
            let end = tb.secs(ch.end).unwrap_or(start);
            out.push((start, end, dict_get(self.av, ch.metadata, "title")));
        }
        out
    }

    /// `av_find_best_stream`: the index and decoder of the best stream of `kind`
    /// (`wanted` = a specific index).
    pub(crate) fn best_stream(
        &self,
        kind: StreamKind,
        wanted: Option<usize>,
    ) -> Result<(usize, *const AVCodec), MediaError> {
        let mut dec: *const AVCodec = std::ptr::null();
        let want = wanted.and_then(|w| c_int::try_from(w).ok()).unwrap_or(-1);
        // SAFETY: an opened context; `dec` receives the decoder pointer (static).
        let r = unsafe { (self.av.f.av_find_best_stream)(self.ctx, media_type(kind), want, -1, &mut dec, 0) };
        if r < 0 {
            return Err(if r == AVERROR_DECODER_NOT_FOUND {
                MediaError::Unsupported(format!("no decoder for the {kind} stream"))
            } else {
                MediaError::NoStream(kind)
            });
        }
        Ok((r as usize, dec))
    }

    /// `av_guess_frame_rate` of stream `i`.
    pub(crate) fn guess_frame_rate(&self, s: StreamRef<'_>) -> AVRational {
        // SAFETY: an opened context and one of its streams.
        unsafe { (self.av.f.av_guess_frame_rate)(self.ctx, s.ptr, std::ptr::null_mut()) }
    }

    /// `av_guess_sample_aspect_ratio` of stream `i`.
    pub(crate) fn guess_sar(&self, s: StreamRef<'_>) -> AVRational {
        // SAFETY: as above.
        unsafe { (self.av.f.av_guess_sample_aspect_ratio)(self.ctx, s.ptr, std::ptr::null_mut()) }
    }

    /// Read the next packet into `pkt`; `Ok(false)` at the end of the input.
    pub(crate) fn read(&mut self, pkt: &mut Packet) -> Result<bool, MediaError> {
        pkt.unref();
        // SAFETY: an opened context and a live packet.
        let r = unsafe { (self.av.f.av_read_frame)(self.ctx, pkt.as_mut_ptr()) };
        if r == AVERROR_EOF {
            return Ok(false);
        }
        if r < 0 {
            return Err(self.avio.explain(r).unwrap_or_else(|| av_err(self.av, r)));
        }
        Ok(true)
    }

    /// Seek so that the next packet of `stream` is the keyframe at or before `ts`
    /// (stream time base; `AV_TIME_BASE` when `stream` is `None`).
    pub(crate) fn seek_before(&mut self, stream: Option<usize>, ts: i64) -> Result<(), MediaError> {
        let idx = stream.and_then(|s| c_int::try_from(s).ok()).unwrap_or(-1);
        // SAFETY: an opened context.
        let r = unsafe { (self.av.f.avformat_seek_file)(self.ctx, idx, i64::MIN, ts, ts, 0) };
        if r < 0 {
            // Some demuxers cannot seek backwards from here; retry with the byte-free
            // legacy call before giving up.
            // SAFETY: as above.
            let r2 = unsafe { (self.av.f.av_seek_frame)(self.ctx, idx, ts, AVSEEK_FLAG_BACKWARD) };
            if r2 < 0 {
                return Err(self.avio.explain(r).unwrap_or_else(|| av_err(self.av, r)));
            }
        }
        Ok(())
    }

    /// Whether the source said "stale" during the last call.
    pub(crate) fn stopped(&self) -> bool {
        self.avio.state().stopped
    }
}

impl Drop for Demuxer<'_> {
    fn drop(&mut self) {
        // SAFETY: `ctx` is an opened context. `avformat_close_input` leaves a custom
        // `pb` alone (AVFMT_FLAG_CUSTOM_IO); `avio` is freed right after by its own drop.
        unsafe { (self.av.f.avformat_close_input)(&mut self.ctx) };
    }
}

/// `AVERROR_DECODER_NOT_FOUND` (`FFERRTAG(0xF8, 'D', 'E', 'C')`).
const AVERROR_DECODER_NOT_FOUND: c_int = -(0xF8 | (b'D' as c_int) << 8 | (b'E' as c_int) << 16 | (b'C' as c_int) << 24);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip() {
        for k in [StreamKind::Video, StreamKind::Audio, StreamKind::Subtitle, StreamKind::Data, StreamKind::Attachment]
        {
            assert_eq!(kind_of(media_type(k)), Some(k));
        }
        assert_eq!(kind_of(-1), None);
        assert_eq!(Limits::default().max_streams, 64);
        assert_eq!(AVERROR_DECODER_NOT_FOUND, -0x4345_44F8);
    }
}
