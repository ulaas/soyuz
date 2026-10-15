//! Decoded audio: a pull reader of interleaved `f32` samples ([`AudioReader`]) for
//! playback, and waveform [`peaks`] for the preview.

use crate::decode::{ChLayout, Decoder, Demuxer, Frame, Limits, Packet, Recv, Resampler};
use crate::info::{Source, StreamInfo, StreamKind, stream_info};
use crate::sys::Av;
use crate::sys::layout::AVRational;
use crate::{MediaError, Runtime};

/// Demuxer + decoder + resampler to interleaved `f32` at a fixed rate.
///
/// `Send` (open it on one thread, read on another), not `Sync`. Every part is a handle
/// from `sys/` that is `Send` because its FFmpeg context is only ever used by the thread
/// that holds the handle (`&mut self` methods, no FFmpeg-internal threads touch the
/// demuxer, the decoder runs with one thread). The demuxer's `stale` callback, the one
/// borrowed piece that could be shared, is always `None` here.
pub struct AudioReader {
    inner: Reader<'static>,
}

// The reader is moved to feeder threads; this stops compiling if a part loses `Send`.
const _: fn() = || {
    fn send<T: Send>() {}
    send::<AudioReader>();
};

impl std::fmt::Debug for AudioReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioReader")
            .field("info", &self.inner.info)
            .field("rate", &self.inner.rate)
            .field("channels", &self.inner.channels)
            .finish_non_exhaustive()
    }
}

impl AudioReader {
    /// Open the audio stream `stream` (`None` = the default one) of `src`, resampled to
    /// `out_rate` (`None` = the stream's own rate). Channel count and sample format are
    /// taken from the decoder (`ch_layout` AVOption, never an `AVFrame` tail field).
    pub fn open(rt: &Runtime, src: &Source, stream: Option<usize>, out_rate: Option<u32>) -> Result<Self, MediaError> {
        Ok(AudioReader { inner: Reader::open(rt.av(), src, stream, out_rate, None)? })
    }

    pub fn info(&self) -> &StreamInfo {
        &self.inner.info
    }

    /// Output sample rate (Hz).
    pub fn rate(&self) -> u32 {
        self.inner.rate
    }

    /// Output channels (samples per frame).
    pub fn channels(&self) -> u32 {
        self.inner.channels
    }

    /// Seek to `secs` (the next [`read`](Self::read) starts there: the decoder restarts
    /// at the keyframe before it and the samples up to `secs` are dropped).
    pub fn seek(&mut self, secs: f64) -> Result<(), MediaError> {
        self.inner.seek(secs)
    }

    /// Append up to `max_frames` frames (`max_frames * channels` samples) to `out`;
    /// returns the frames appended, `0` at the end of the stream.
    pub fn read(&mut self, out: &mut Vec<f32>, max_frames: usize) -> Result<usize, MediaError> {
        self.inner.read(out, max_frames)
    }
}

/// Waveform peaks: `buckets` pairs of `(min, max)` over the whole stream, all channels
/// mixed. `stale` is polled between packets.
pub fn peaks(
    rt: &Runtime,
    src: &Source,
    buckets: usize,
    stale: &dyn Fn() -> bool,
) -> Result<Vec<(f32, f32)>, MediaError> {
    if buckets == 0 {
        return Ok(Vec::new());
    }
    let mut r = Reader::open(rt.av(), src, None, None, Some(stale))?;
    let ch = r.channels.max(1) as usize;
    // Per group of `group` frames, the (min, max) over every sample, reduced to `buckets`
    // at the end: at least four groups per bucket when the duration is known (short
    // clips keep their resolution), at most one pair per 256 frames (bounded memory for
    // long files).
    let est_frames = r.duration().map_or(0.0, |d| d * f64::from(r.rate));
    let group = ((est_frames / (buckets as f64 * 4.0)) as usize).clamp(1, 256);
    let mut groups: Vec<(f32, f32)> = Vec::new();
    let mut buf: Vec<f32> = Vec::new();
    let mut current = (f32::MAX, f32::MIN);
    let mut in_group = 0usize;
    loop {
        if stale() {
            return Err(MediaError::Stale);
        }
        buf.clear();
        let n = r.read(&mut buf, 4096)?;
        if n == 0 {
            break;
        }
        for frame in buf.chunks(ch) {
            for s in frame {
                current.0 = current.0.min(*s);
                current.1 = current.1.max(*s);
            }
            in_group += 1;
            if in_group == group {
                groups.push(current);
                current = (f32::MAX, f32::MIN);
                in_group = 0;
            }
        }
    }
    if in_group > 0 {
        groups.push(current);
    }
    if groups.is_empty() {
        return Ok(Vec::new());
    }
    let buckets = buckets.min(groups.len()).max(1);
    let mut out = Vec::with_capacity(buckets);
    for b in 0..buckets {
        let from = b * groups.len() / buckets;
        let to = ((b + 1) * groups.len() / buckets).max(from + 1).min(groups.len());
        let mut acc = (f32::MAX, f32::MIN);
        for g in &groups[from..to] {
            acc.0 = acc.0.min(g.0);
            acc.1 = acc.1.max(g.1);
        }
        out.push(acc);
    }
    Ok(out)
}

/// The reader behind [`AudioReader`] and [`peaks`] (`'a` = the `stale` closure).
pub(crate) struct Reader<'a> {
    av: &'static Av,
    d: Demuxer<'a>,
    dec: Decoder,
    pkt: Packet,
    frame: Frame,
    idx: usize,
    tb: AVRational,
    start: i64,
    info: StreamInfo,
    rate: u32,
    channels: u32,
    out_rate: Option<u32>,
    layout: ChLayout,
    rs: Option<Resampler>,
    /// Converted samples not yet handed out.
    pending: Vec<f32>,
    pending_pos: usize,
    /// Drop output before this position (seconds, stream clock) after a seek.
    skip_until: Option<f64>,
    eof: bool,
}

impl<'a> Reader<'a> {
    pub(crate) fn open(
        av: &'static Av,
        src: &Source,
        stream: Option<usize>,
        out_rate: Option<u32>,
        stale: Option<&'a (dyn Fn() -> bool + 'a)>,
    ) -> Result<Reader<'a>, MediaError> {
        let d = Demuxer::open(av, src, stale, Limits::default())?;
        let (idx, decoder) = d.best_stream(StreamKind::Audio, stream)?;
        let s = d.stream(idx).ok_or(MediaError::NoStream(StreamKind::Audio))?;
        if s.kind() != Some(StreamKind::Audio) {
            return Err(MediaError::NoStream(StreamKind::Audio));
        }
        let info = stream_info(av, &d, s);
        let tb = s.time_base();
        let start = s.start_time();
        let dec = Decoder::open(av, decoder, s.par_ptr(), tb, &[("threads", "1")])?;
        let layout = dec.ch_layout()?;
        let mut r = Reader {
            av,
            d,
            dec,
            pkt: Packet::new(av)?,
            frame: Frame::new(av)?,
            idx,
            tb,
            start,
            rate: out_rate.unwrap_or(info.sample_rate),
            channels: layout.channels(),
            info,
            out_rate,
            layout,
            rs: None,
            pending: Vec::new(),
            pending_pos: 0,
            skip_until: None,
            eof: false,
        };
        // Decode the first frame now: the rate and channel count must be final before
        // the caller opens an audio device.
        while r.pending.is_empty() && !r.eof {
            r.fill()?;
        }
        if r.rate == 0 || r.channels == 0 {
            return Err(MediaError::Unsupported("no decodable audio frames".into()));
        }
        Ok(r)
    }

    /// The container's duration (seconds), when known.
    fn duration(&self) -> Option<f64> {
        self.d.duration()
    }

    fn seek(&mut self, secs: f64) -> Result<(), MediaError> {
        let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
        let start_secs = self.tb.secs(self.start).unwrap_or(0.0);
        let target = start_secs + secs;
        if self.tb.num <= 0 || self.tb.den <= 0 {
            return Err(MediaError::Unsupported("stream has no time base".into()));
        }
        let ts = (target * f64::from(self.tb.den) / f64::from(self.tb.num)).round() as i64;
        self.d.seek_before(Some(self.idx), ts)?;
        self.dec.flush();
        self.rs = None;
        self.pending.clear();
        self.pending_pos = 0;
        self.skip_until = Some(target);
        self.eof = false;
        Ok(())
    }

    fn read(&mut self, out: &mut Vec<f32>, max_frames: usize) -> Result<usize, MediaError> {
        let ch = self.channels.max(1) as usize;
        let mut got = 0usize;
        while got < max_frames {
            let avail = (self.pending.len() - self.pending_pos) / ch;
            if avail == 0 {
                if self.eof {
                    break;
                }
                self.pending.clear();
                self.pending_pos = 0;
                self.fill()?;
                continue;
            }
            let take = avail.min(max_frames - got);
            out.extend_from_slice(&self.pending[self.pending_pos..self.pending_pos + take * ch]);
            self.pending_pos += take * ch;
            got += take;
        }
        Ok(got)
    }

    /// Decode until at least one frame was converted into `pending` or the stream ends.
    fn fill(&mut self) -> Result<(), MediaError> {
        while self.pending.is_empty() && !self.eof {
            if self.d.stopped() {
                return Err(MediaError::Stale);
            }
            let more = self.d.read(&mut self.pkt)?;
            if more && self.pkt.stream_index() != self.idx {
                continue;
            }
            self.dec.send(if more { Some(&self.pkt) } else { None })?;
            let mut drained = !more;
            loop {
                match self.dec.receive(&mut self.frame)? {
                    Recv::Frame => {}
                    Recv::Again => break,
                    Recv::Eof => {
                        drained = true;
                        break;
                    }
                }
                self.convert_frame()?;
            }
            if drained {
                self.eof = true;
                if let Some(rs) = self.rs.as_mut() {
                    rs.convert(None, &mut self.pending)?;
                }
            }
        }
        Ok(())
    }

    /// Resample `self.frame` into `pending`, honouring `skip_until`.
    fn convert_frame(&mut self) -> Result<(), MediaError> {
        let (fmt, in_rate, n) = (self.frame.format(), self.frame.sample_rate(), self.frame.nb_samples());
        if n <= 0 || in_rate <= 0 {
            return Ok(());
        }
        if self.rate == 0 {
            self.rate = self.out_rate.unwrap_or(u32::try_from(in_rate).unwrap_or(0));
        }
        let fresh = self.rs.as_ref().is_none_or(|rs| !rs.matches(fmt, in_rate, self.layout.channels()));
        if fresh {
            // The decoder's layout can change with the stream (parameter changes).
            self.layout = self.dec.ch_layout()?;
            if self.channels == 0 {
                self.channels = self.layout.channels();
            }
            self.rs = Some(Resampler::new(self.av, &self.layout, fmt, in_rate, self.rate)?);
        }
        let before = self.pending.len();
        let rs = self.rs.as_mut().expect("set above");
        rs.convert(Some(&self.frame), &mut self.pending)?;
        if let Some(skip) = self.skip_until {
            let secs = self.tb.secs(self.frame.best_effort_timestamp()).or_else(|| self.tb.secs(self.frame.pts()));
            let ch = self.channels.max(1) as usize;
            match secs {
                Some(secs) if secs < skip => {
                    let produced = (self.pending.len() - before) / ch;
                    let drop = (((skip - secs) * f64::from(self.rate)).round() as usize).min(produced);
                    let keep_from = before + drop * ch;
                    self.pending.drain(before..keep_from);
                    if drop < produced {
                        self.skip_until = None;
                    }
                }
                _ => self.skip_until = None,
            }
        }
        Ok(())
    }
}
