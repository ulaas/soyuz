//! Audio previews: a waveform, tags, duration, bitrate and cover art, plus a tiny player
//! (rodio → cpal → WASAPI on Windows; no shell APIs) with play / pause / seek and the
//! `[media]` volume.
//!
//! Two decoders ([`backend`], `[media] audio_backend`): the FFmpeg libraries when they
//! are loaded (`services::media`; Opus, WMA, APE, AC-3, DSD, MKA and everything else they
//! read) and Symphonia (pure Rust; mp3, flac, wav, ogg, aac, alac, …) otherwise or when
//! asked. Both give the same [`AudioInfo`]. The FFmpeg path analyses with
//! `soyuz_media::probe` + `peaks` and plays through an [`AudioReader`] feeding a ring
//! ([`media_player::ring`]) that a `rodio::Source` pulls from: the feeder thread never
//! touches the device callback, which plays silence when the ring runs dry.
//!
//! The player lives on its own thread, which owns the output device. The UI sends
//! commands and reads a shared state snapshot, so a slow device open never blocks a
//! frame. Playback stops by itself shortly after no preview shows the track anymore
//! (the renderer sends a heartbeat every frame it draws the player).

use std::fs::File;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine as _;
use crossbeam_channel::{Receiver, Sender, TryRecvError, unbounded};
use parking_lot::Mutex;
use soyuz_core::config::{AudioBackend, Media as MediaCfg};
use soyuz_media::{AudioReader, AudioSink as _, MediaError, Runtime, Source as FfSource};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey};
use symphonia::core::probe::Hint;

use super::media;
use super::media_player::{self, RingClock, RingSink};

/// Points in a finished waveform.
pub const WAVE_POINTS: usize = 1200;
/// Frames folded into one raw bucket while decoding.
const BUCKET_FRAMES: usize = 256;
/// Decoding stops after this long; the waveform then covers the decoded part only.
const DECODE_BUDGET: Duration = Duration::from_secs(8);

/// Where the audio bytes come from.
#[derive(Clone, Debug)]
pub enum AudioSrc {
    File(PathBuf),
    /// An archive entry read into memory.
    Mem(Arc<[u8]>),
}

impl AudioSrc {
    /// Identity used by the player to tell tracks apart.
    pub fn key(&self) -> String {
        match self {
            AudioSrc::File(p) => p.display().to_string(),
            AudioSrc::Mem(b) => format!("mem:{:p}:{}", b.as_ptr(), b.len()),
        }
    }

    fn media(&self) -> Result<Box<dyn MediaSource>, String> {
        Ok(match self {
            AudioSrc::File(p) => Box::new(File::open(p).map_err(|e| e.to_string())?),
            AudioSrc::Mem(b) => Box::new(Cursor::new(b.clone())),
        })
    }

    /// The same bytes for the FFmpeg libraries.
    pub fn media_source(&self) -> FfSource {
        match self {
            AudioSrc::File(p) => FfSource::File(p.clone()),
            AudioSrc::Mem(b) => FfSource::Mem(b.clone()),
        }
    }
}

/// Which decoder produced an [`AudioInfo`] (shown as a fact).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Decoder {
    #[default]
    Symphonia,
    /// The FFmpeg libraries, with their `avcodec` major.
    Ffmpeg(u32),
}

impl Decoder {
    /// A label for the facts list.
    pub fn label(self) -> String {
        match self {
            Decoder::Symphonia => tr!("services-audio-decoder-symphonia").into(),
            Decoder::Ffmpeg(major) => tr!("services-audio-decoder-ffmpeg", major = major.to_string()),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AudioInfo {
    /// `(min, max)` sample pairs in -1..1, at most [`WAVE_POINTS`].
    pub peaks: Vec<(f32, f32)>,
    /// Share of the track the waveform covers (1 = all of it).
    pub covered: f32,
    pub duration: Option<f64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<usize>,
    pub bits: Option<u32>,
    pub codec: Option<String>,
    /// `(label, value)` tags in display order.
    pub tags: Vec<(String, String)>,
    /// Embedded cover art (encoded image bytes).
    pub cover: Option<Vec<u8>>,
    /// Which decoder read the file.
    pub decoder: Decoder,
}

/// Fold `src` into exactly `n` buckets (or fewer when `src` is shorter), keeping the
/// extremes of each bucket.
pub fn downsample(src: &[(f32, f32)], n: usize) -> Vec<(f32, f32)> {
    if n == 0 || src.is_empty() {
        return Vec::new();
    }
    if src.len() <= n {
        return src.to_vec();
    }
    (0..n)
        .map(|i| {
            let a = i * src.len() / n;
            let b = ((i + 1) * src.len() / n).max(a + 1);
            src[a..b].iter().fold((f32::MAX, f32::MIN), |(lo, hi), &(l, h)| (lo.min(l), hi.max(h)))
        })
        .collect()
}

// ---- backend choice and the `[media]` player keys --------------------------------------

/// `[media] audio_backend` (0 auto, 1 ffmpeg, 2 symphonia).
static BACKEND: AtomicU8 = AtomicU8::new(0);
/// `[media] volume`, 0–100.
static VOLUME: AtomicU8 = AtomicU8::new(80);
/// `[media] muted`.
static MUTED: AtomicBool = AtomicBool::new(false);

/// Apply the `[media]` keys this module reads (`AppCore::apply_core_settings`).
pub fn configure(m: &MediaCfg) {
    set_backend(m.audio_backend);
    set_volume(m.volume, m.muted);
}

pub fn set_backend(b: AudioBackend) {
    let v = match b {
        AudioBackend::Auto => 0,
        AudioBackend::Ffmpeg => 1,
        AudioBackend::Symphonia => 2,
    };
    BACKEND.store(v, Ordering::Relaxed);
}

/// The configured backend.
pub fn backend() -> AudioBackend {
    match BACKEND.load(Ordering::Relaxed) {
        1 => AudioBackend::Ffmpeg,
        2 => AudioBackend::Symphonia,
        _ => AudioBackend::Auto,
    }
}

/// The player's volume (0–100) and mute; applied to the playing track at once. The
/// values are the config's (`models::player` writes changes back to `config.toml`).
pub fn set_volume(percent: u8, muted: bool) {
    VOLUME.store(percent.min(100), Ordering::Relaxed);
    MUTED.store(muted, Ordering::Relaxed);
    if let Some(p) = PLAYER.get() {
        let _ = p.tx.send(Cmd::Volume);
    }
}

/// `(volume 0–100, muted)` as last set.
pub fn volume() -> (u8, bool) {
    (VOLUME.load(Ordering::Relaxed), MUTED.load(Ordering::Relaxed))
}

/// The gain the device gets (0 when muted).
fn gain() -> f32 {
    let (v, muted) = volume();
    if muted { 0.0 } else { f32::from(v) / 100.0 }
}

/// The decoder to use right now.
enum Backend {
    Symphonia,
    Ffmpeg(Arc<Runtime>),
}

/// Pick by `[media] audio_backend`; the flag says whether Symphonia may take over when
/// the FFmpeg path fails on a file (`auto` only).
fn pick() -> Result<(Backend, bool), String> {
    match backend() {
        AudioBackend::Symphonia => Ok((Backend::Symphonia, false)),
        AudioBackend::Ffmpeg => media::runtime()
            .map(|rt| (Backend::Ffmpeg(rt), false))
            .ok_or_else(|| tr!("services-audio-no-ffmpeg").into()),
        AudioBackend::Auto => Ok(match media::runtime() {
            Some(rt) => (Backend::Ffmpeg(rt), true),
            None => (Backend::Symphonia, false),
        }),
    }
}

// ---- analysis ----------------------------------------------------------------------------

fn tag_label(k: StandardTagKey) -> Option<&'static str> {
    use StandardTagKey as K;
    Some(match k {
        K::TrackTitle => "Title",
        K::Artist => "Artist",
        K::Album => "Album",
        K::AlbumArtist => "Album artist",
        K::Date | K::ReleaseDate | K::OriginalDate => "Year",
        K::Genre => "Genre",
        K::TrackNumber => "Track",
        K::Composer => "Composer",
        _ => return None,
    })
}

/// The label of an FFmpeg metadata key (`artist`, `ALBUM_ARTIST`, `TIT2`-less names only:
/// the libraries already map ID3 / Vorbis / MP4 atoms to these).
fn ffmpeg_tag_label(key: &str) -> Option<&'static str> {
    Some(match key.to_ascii_lowercase().as_str() {
        "title" => "Title",
        "artist" => "Artist",
        "album" => "Album",
        "album_artist" | "albumartist" => "Album artist",
        "date" | "year" | "originaldate" | "original_date" => "Year",
        "genre" => "Genre",
        "track" | "tracknumber" => "Track",
        "composer" => "Composer",
        _ => return None,
    })
}

fn push_tag(info: &mut AudioInfo, label: &str, v: &str) {
    let v = v.trim();
    if !v.is_empty() && !info.tags.iter().any(|(k, _)| k == label) {
        info.tags.push((label.to_string(), v.chars().take(200).collect()));
    }
}

fn read_tags(rev: &MetadataRevision, info: &mut AudioInfo) {
    for t in rev.tags() {
        let Some(label) = t.std_key.and_then(tag_label) else { continue };
        push_tag(info, label, &t.value.to_string());
    }
    if info.cover.is_none()
        && let Some(v) = rev.visuals().iter().find(|v| v.data.len() > 16)
    {
        info.cover = Some(v.data.to_vec());
    }
}

/// Probe tags / stream info and (when `waveform`) decode the whole track into peaks,
/// with the configured [`backend`].
pub fn analyze(src: &AudioSrc, ext: &str, waveform: bool, stale: &dyn Fn() -> bool) -> Result<AudioInfo, String> {
    match pick()? {
        (Backend::Ffmpeg(rt), fallback) => match analyze_ffmpeg(&rt, src, waveform, stale) {
            Ok(info) => Ok(info),
            Err(e) if e == "stale" || !fallback => Err(e),
            Err(e) => {
                log::debug!("preview_audio: FFmpeg could not read {}: {e}; trying Symphonia", src.key());
                analyze_symphonia(src, ext, waveform, stale)
            }
        },
        (Backend::Symphonia, _) => analyze_symphonia(src, ext, waveform, stale),
    }
}

/// Bits per sample from FFmpeg's sample format name (`s16p` → 16; floats have none).
fn sample_fmt_bits(fmt: Option<&str>) -> Option<u32> {
    match fmt?.trim_end_matches('p') {
        "u8" => Some(8),
        "s16" => Some(16),
        "s32" => Some(32),
        "s64" => Some(64),
        _ => None,
    }
}

/// The picture bytes of a Vorbis-comment `METADATA_BLOCK_PICTURE` (a base64 FLAC
/// `PICTURE` block: type, MIME, description, size fields, then the data), which Ogg
/// Vorbis / Opus files use for cover art that no stream carries.
pub fn metadata_block_picture(b64: &str) -> Option<Vec<u8>> {
    let b = base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()?;
    let u32_at = |at: usize| b.get(at..at + 4).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]) as usize);
    let mut at = 4; // picture type
    let mime_len = u32_at(at)?;
    at += 4 + mime_len;
    let desc_len = u32_at(at)?;
    at += 4 + desc_len;
    at += 16; // width, height, depth, colours
    let data_len = u32_at(at)?;
    at += 4;
    let data = b.get(at..at.checked_add(data_len)?)?;
    (data.len() > 16).then(|| data.to_vec())
}

/// The FFmpeg path: `probe` for tags, cover and stream facts, `peaks` for the waveform.
fn analyze_ffmpeg(rt: &Runtime, src: &AudioSrc, waveform: bool, stale: &dyn Fn() -> bool) -> Result<AudioInfo, String> {
    let msrc = src.media_source();
    let probed = soyuz_media::probe(rt, &msrc).map_err(|e| tr!("services-audio-unsupported", error = e.to_string()))?;
    let a = probed.audio().ok_or_else(|| tr!("services-audio-no-track").to_string())?;
    let mut info = AudioInfo { decoder: Decoder::Ffmpeg(rt.versions().majors().avcodec), ..Default::default() };
    for (k, v) in probed.tags.iter().chain(a.tags.iter()) {
        if let Some(label) = ffmpeg_tag_label(k) {
            push_tag(&mut info, label, v);
        }
    }
    info.cover = probed.cover.clone().or_else(|| {
        probed
            .tags
            .iter()
            .chain(a.tags.iter())
            .find(|(k, _)| k.eq_ignore_ascii_case("METADATA_BLOCK_PICTURE"))
            .and_then(|(_, v)| metadata_block_picture(v))
    });
    info.sample_rate = Some(a.sample_rate).filter(|r| *r > 0);
    info.channels = Some(a.channels as usize).filter(|c| *c > 0);
    info.bits = sample_fmt_bits(a.sample_fmt.as_deref());
    info.codec =
        Some(if a.codec_long.is_empty() { a.codec.clone() } else { a.codec_long.clone() }).filter(|c| !c.is_empty());
    info.duration = probed.duration.filter(|d| d.is_finite() && *d >= 0.0);
    if !waveform {
        return Ok(info);
    }
    let peaks = match soyuz_media::peaks(rt, &msrc, WAVE_POINTS, stale) {
        Ok(p) => p,
        Err(MediaError::Stale) => return Err("stale".into()),
        Err(e) => return Err(tr!("services-audio-unsupported-codec", error = e.to_string())),
    };
    info.peaks = peaks.into_iter().map(|(lo, hi)| (lo.clamp(-1.0, 1.0), hi.clamp(-1.0, 1.0))).collect();
    info.covered = 1.0;
    Ok(info)
}

/// The pure-Rust path.
fn analyze_symphonia(src: &AudioSrc, ext: &str, waveform: bool, stale: &dyn Fn() -> bool) -> Result<AudioInfo, String> {
    let mss = MediaSourceStream::new(src.media()?, Default::default());
    let mut hint = Hint::new();
    if !ext.is_empty() {
        hint.with_extension(ext);
    }
    let mut probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions { enable_gapless: true, ..Default::default() }, &MetadataOptions::default())
        .map_err(|e| tr!("services-audio-unsupported", error = e.to_string()))?;
    let mut info = AudioInfo { decoder: Decoder::Symphonia, ..Default::default() };
    if let Some(m) = probed.metadata.get()
        && let Some(rev) = m.current()
    {
        read_tags(rev, &mut info);
    }
    let mut format = probed.format;
    if let Some(rev) = format.metadata().current() {
        read_tags(rev, &mut info);
    }
    let track = format
        .default_track()
        .filter(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .or_else(|| format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL))
        .ok_or(tr!("services-audio-no-track"))?
        .clone();
    let cp = &track.codec_params;
    info.sample_rate = cp.sample_rate;
    info.channels = cp.channels.map(|c| c.count());
    info.bits = cp.bits_per_sample;
    info.codec = symphonia::default::get_codecs().get_codec(cp.codec).map(|d| d.long_name.to_string());
    info.duration = match (cp.n_frames, cp.time_base, cp.sample_rate) {
        (Some(n), Some(tb), _) => {
            let t = tb.calc_time(n);
            Some(t.seconds as f64 + t.frac)
        }
        (Some(n), None, Some(sr)) if sr > 0 => Some(n as f64 / sr as f64),
        _ => None,
    };
    if !waveform {
        return Ok(info);
    }
    let mut decoder = symphonia::default::get_codecs()
        .make(cp, &DecoderOptions::default())
        .map_err(|e| tr!("services-audio-unsupported-codec", error = e.to_string()))?;
    let started = Instant::now();
    let mut raw: Vec<(f32, f32)> = Vec::new();
    let mut cur = (f32::MAX, f32::MIN);
    let mut in_bucket = 0usize;
    let mut frames: u64 = 0;
    let mut sbuf: Option<SampleBuffer<f32>> = None;
    let mut complete = true;
    loop {
        if stale() {
            return Err("stale".into());
        }
        if started.elapsed() > DECODE_BUDGET {
            complete = false;
            break;
        }
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::ResetRequired) | Err(SymError::IoError(_)) => break,
            Err(_) => break,
        };
        if packet.track_id() != track.id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymError::DecodeError(_)) => continue,
            Err(_) => break,
        };
        let spec = *decoded.spec();
        let cap = decoded.capacity() as u64;
        if sbuf.as_ref().is_none_or(|b| (b.capacity() as u64) < cap * spec.channels.count() as u64) {
            sbuf = Some(SampleBuffer::<f32>::new(cap, spec));
        }
        let Some(sb) = sbuf.as_mut() else { break };
        sb.copy_interleaved_ref(decoded);
        let ch = spec.channels.count().max(1);
        for frame in sb.samples().chunks(ch) {
            for &s in frame {
                cur.0 = cur.0.min(s);
                cur.1 = cur.1.max(s);
            }
            in_bucket += 1;
            if in_bucket == BUCKET_FRAMES {
                raw.push(cur);
                cur = (f32::MAX, f32::MIN);
                in_bucket = 0;
            }
        }
        frames += (sb.samples().len() / ch) as u64;
    }
    if in_bucket > 0 {
        raw.push(cur);
    }
    let total = cp.n_frames.filter(|&n| n > 0);
    info.covered = match total {
        Some(n) if !complete => (frames as f64 / n as f64).clamp(0.0, 1.0) as f32,
        _ => 1.0,
    };
    if info.duration.is_none()
        && complete
        && let Some(sr) = info.sample_rate.filter(|&s| s > 0)
    {
        info.duration = Some(frames as f64 / sr as f64);
    }
    info.peaks =
        downsample(&raw, WAVE_POINTS).into_iter().map(|(lo, hi)| (lo.clamp(-1.0, 1.0), hi.clamp(-1.0, 1.0))).collect();
    Ok(info)
}

// ---- player -------------------------------------------------------------------------

/// Snapshot of the player.
#[derive(Clone, Debug, Default)]
pub struct PlayerState {
    /// Track currently loaded (see [`AudioSrc::key`]).
    pub key: Option<String>,
    pub playing: bool,
    /// Position in seconds.
    pub pos: f64,
    pub error: Option<String>,
    /// Which decoder plays the track.
    pub decoder: Decoder,
}

enum Cmd {
    Play {
        src: AudioSrc,
        ext: String,
        at: f64,
    },
    Pause,
    Resume,
    Seek(f64),
    Stop,
    /// The volume / mute statics changed.
    Volume,
}

struct Player {
    tx: Sender<Cmd>,
    state: Arc<Mutex<PlayerState>>,
    seen: Arc<Mutex<Instant>>,
}

/// Stop playback this long after the last frame that showed the player.
const ORPHAN_AFTER: Duration = Duration::from_millis(1500);
/// Release the audio device after this much idle time.
const DEVICE_IDLE: Duration = Duration::from_secs(20);
/// Frames the FFmpeg feeder decodes per `read`.
const FEED_FRAMES: usize = 2048;

static PLAYER: OnceLock<Player> = OnceLock::new();

fn player() -> &'static Player {
    PLAYER.get_or_init(|| {
        let (tx, rx) = unbounded();
        let state = Arc::new(Mutex::new(PlayerState::default()));
        let seen = Arc::new(Mutex::new(Instant::now()));
        let (s2, seen2) = (state.clone(), seen.clone());
        let _ = std::thread::Builder::new().name("soyuz-audio".into()).spawn(move || audio_thread(rx, s2, seen2));
        Player { tx, state, seen }
    })
}

/// Current player state.
pub fn state() -> PlayerState {
    player().state.lock().clone()
}

/// The renderer shows the player this frame (keeps playback alive).
pub fn heartbeat() {
    *player().seen.lock() = Instant::now();
}

pub fn play(src: AudioSrc, ext: &str, at: f64) {
    heartbeat();
    let p = player();
    {
        let mut s = p.state.lock();
        s.key = Some(src.key());
        s.playing = true;
        s.pos = at;
        s.error = None;
    }
    let _ = p.tx.send(Cmd::Play { src, ext: ext.to_string(), at });
}

pub fn pause() {
    player().state.lock().playing = false;
    let _ = player().tx.send(Cmd::Pause);
}

pub fn resume() {
    heartbeat();
    player().state.lock().playing = true;
    let _ = player().tx.send(Cmd::Resume);
}

pub fn seek(at: f64) {
    player().state.lock().pos = at;
    let _ = player().tx.send(Cmd::Seek(at));
}

pub fn stop() {
    let _ = player().tx.send(Cmd::Stop);
}

/// A playing track: rodio's own decoders, or the FFmpeg feeder behind a ring.
enum Track {
    Rodio(rodio::Player),
    Ffmpeg { player: rodio::Player, feed: Sender<FeedCmd>, clock: RingClock },
}

impl Track {
    fn player(&self) -> &rodio::Player {
        match self {
            Track::Rodio(p) | Track::Ffmpeg { player: p, .. } => p,
        }
    }

    fn pos(&self) -> f64 {
        match self {
            Track::Rodio(p) => p.get_pos().as_secs_f64(),
            Track::Ffmpeg { clock, .. } => clock.secs(),
        }
    }

    fn seek(&self, at: f64) {
        match self {
            Track::Rodio(p) => {
                let _ = p.try_seek(Duration::from_secs_f64(at.max(0.0)));
            }
            Track::Ffmpeg { feed, .. } => {
                let _ = feed.send(FeedCmd::Seek(at.max(0.0)));
            }
        }
    }
}

fn audio_thread(rx: Receiver<Cmd>, state: Arc<Mutex<PlayerState>>, seen: Arc<Mutex<Instant>>) {
    let mut device: Option<rodio::MixerDeviceSink> = None;
    let mut track: Option<Track> = None;
    let mut idle_since = Instant::now();
    let mut applied_gain = f32::NAN;
    loop {
        let cmd = match rx.recv_timeout(Duration::from_millis(80)) {
            Ok(c) => Some(c),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => None,
            Err(_) => return,
        };
        // A decoder / device bug must not kill the only audio thread.
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Some(cmd) = cmd {
                handle(cmd, &mut device, &mut track, &state);
            }
            if let Some(t) = &track {
                let gain = gain();
                if gain != applied_gain {
                    t.player().set_volume(gain);
                    applied_gain = gain;
                }
                let mut s = state.lock();
                if t.player().empty() {
                    s.playing = false;
                    s.pos = 0.0;
                    track = None;
                } else {
                    s.pos = t.pos();
                    s.playing = !t.player().is_paused();
                }
            }
            if track.is_some() && seen.lock().elapsed() > ORPHAN_AFTER {
                track = None;
                let mut s = state.lock();
                s.playing = false;
                s.key = None;
                s.pos = 0.0;
            }
            if track.is_some() {
                idle_since = Instant::now();
            } else if device.is_some() && idle_since.elapsed() > DEVICE_IDLE {
                device = None;
            }
        }));
        if r.is_err() {
            track = None;
            let mut s = state.lock();
            s.playing = false;
            s.error = Some(tr!("services-audio-player-failed").into());
        }
    }
}

/// Open a rodio decoder track (Symphonia and rodio's own decoders).
fn open_rodio_track(dev: &rodio::MixerDeviceSink, src: &AudioSrc, ext: &str) -> Result<rodio::Player, String> {
    let p = rodio::Player::connect_new(dev.mixer());
    match src {
        AudioSrc::File(path) => {
            let f = File::open(path).map_err(|e| e.to_string())?;
            let len = f.metadata().map(|m| m.len()).unwrap_or(0);
            rodio::Decoder::builder()
                .with_data(std::io::BufReader::new(f))
                .with_byte_len(len)
                .with_hint(ext)
                .with_seekable(true)
                .build()
                .map(|d| p.append(d))
                .map_err(|e| e.to_string())?;
        }
        AudioSrc::Mem(b) => {
            rodio::Decoder::builder()
                .with_data(Cursor::new(b.clone()))
                .with_byte_len(b.len() as u64)
                .with_hint(ext)
                .with_seekable(true)
                .build()
                .map(|d| p.append(d))
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(p)
}

/// How long opening a file may take before the player gives up on it.
const OPEN_TIMEOUT: Duration = Duration::from_secs(20);

/// Open an FFmpeg track: the reader lives on its feeder thread (it is not `Send`), which
/// reports the ring's device end back once the stream's rate and channels are known.
fn open_ffmpeg_track(rt: Arc<Runtime>, dev: &rodio::MixerDeviceSink, src: &AudioSrc, at: f64) -> Result<Track, String> {
    let at = if at.is_finite() { at.max(0.0) } else { 0.0 };
    let msrc = src.media_source();
    let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
    let (feed, frx) = unbounded();
    std::thread::Builder::new()
        .name("soyuz-audio-feed".into())
        .spawn(move || {
            let mut reader = match AudioReader::open(&rt, &msrc, None, None) {
                Ok(r) => r,
                Err(e) => {
                    let _ = reply_tx.send(Err(e.to_string()));
                    return;
                }
            };
            if at > 0.0
                && let Err(e) = reader.seek(at)
            {
                log::debug!("preview_audio: seek to {at}s failed ({e}); playing from the start");
            }
            let (mut sink, source) = media_player::ring(reader.rate(), reader.channels(), media_player::RING_SECS);
            sink.rebase(at);
            let clock = sink.clock();
            if reply_tx.send(Ok((source, clock))).is_err() {
                return; // The player gave up waiting.
            }
            feeder(reader, sink, frx);
        })
        .map_err(|e| e.to_string())?;
    let (source, clock) = match reply_rx.recv_timeout(OPEN_TIMEOUT) {
        Ok(r) => r?,
        Err(_) => return Err(tr!("services-audio-open-timeout").into()),
    };
    let player = rodio::Player::connect_new(dev.mixer());
    player.append(source);
    Ok(Track::Ffmpeg { player, feed, clock })
}

enum FeedCmd {
    Seek(f64),
}

/// The FFmpeg feeder: decodes ahead into the ring, seeks on request, finishes at the
/// end, and quits when the track is dropped (the command channel closes) or the device
/// side is gone.
fn feeder(mut reader: AudioReader, mut sink: RingSink, rx: Receiver<FeedCmd>) {
    let ch = reader.channels().max(1) as usize;
    let mut buf: Vec<f32> = Vec::new();
    let mut off = 0usize;
    let mut eof = false;
    fn seek(
        secs: f64,
        reader: &mut AudioReader,
        sink: &mut RingSink,
        buf: &mut Vec<f32>,
        off: &mut usize,
        eof: &mut bool,
    ) {
        sink.rebase(secs);
        if let Err(e) = reader.seek(secs) {
            log::debug!("preview_audio: seek to {secs}s failed ({e})");
        }
        buf.clear();
        *off = 0;
        *eof = false;
    }
    loop {
        loop {
            match rx.try_recv() {
                Ok(FeedCmd::Seek(secs)) => seek(secs, &mut reader, &mut sink, &mut buf, &mut off, &mut eof),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if sink.is_closed() {
            return;
        }
        if off >= buf.len() {
            if eof {
                // Done: stay for a late seek until the track is dropped.
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(FeedCmd::Seek(secs)) => seek(secs, &mut reader, &mut sink, &mut buf, &mut off, &mut eof),
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
                }
                continue;
            }
            buf.clear();
            off = 0;
            match reader.read(&mut buf, FEED_FRAMES) {
                Ok(0) => {
                    eof = true;
                    sink.finish();
                    continue;
                }
                Ok(_) => {}
                Err(e) => {
                    log::debug!("preview_audio: decoding stopped ({e})");
                    eof = true;
                    sink.finish();
                    continue;
                }
            }
        }
        let n = sink.push(&buf[off..]);
        off += n * ch;
        if n == 0 {
            // Ring full: wait for the device to drain a little (or a command).
            match rx.recv_timeout(Duration::from_millis(5)) {
                Ok(FeedCmd::Seek(secs)) => seek(secs, &mut reader, &mut sink, &mut buf, &mut off, &mut eof),
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

fn handle(
    cmd: Cmd,
    device: &mut Option<rodio::MixerDeviceSink>,
    track: &mut Option<Track>,
    state: &Mutex<PlayerState>,
) {
    let fail = |msg: String| {
        let mut s = state.lock();
        s.playing = false;
        s.error = Some(msg);
    };
    match cmd {
        Cmd::Play { src, ext, at } => {
            *track = None;
            if device.is_none() {
                match rodio::DeviceSinkBuilder::open_default_sink() {
                    Ok(mut d) => {
                        d.log_on_drop(false);
                        *device = Some(d);
                    }
                    Err(e) => return fail(tr!("services-audio-no-device", error = e.to_string())),
                }
            }
            let Some(dev) = device.as_ref() else { return };
            let (backend, fallback) = match pick() {
                Ok(b) => b,
                Err(e) => return fail(e),
            };
            let opened = match backend {
                Backend::Ffmpeg(rt) => match open_ffmpeg_track(rt, dev, &src, at) {
                    Ok(t) => Ok(t),
                    Err(e) if fallback => {
                        log::debug!("preview_audio: FFmpeg could not play {}: {e}; trying rodio", src.key());
                        open_rodio_track(dev, &src, &ext).map(Track::Rodio)
                    }
                    Err(e) => Err(e),
                },
                Backend::Symphonia => open_rodio_track(dev, &src, &ext).map(Track::Rodio),
            };
            let t = match opened {
                Ok(t) => t,
                Err(e) => return fail(tr!("services-audio-cant-play", error = e)),
            };
            if at > 0.0 && matches!(t, Track::Rodio(_)) {
                t.seek(at);
            }
            state.lock().decoder = match &t {
                Track::Rodio(_) => Decoder::Symphonia,
                Track::Ffmpeg { .. } => {
                    Decoder::Ffmpeg(media::runtime().map_or(0, |rt| rt.versions().majors().avcodec))
                }
            };
            t.player().set_volume(gain());
            t.player().play();
            *track = Some(t);
        }
        Cmd::Pause => {
            if let Some(t) = track {
                t.player().pause();
            }
        }
        Cmd::Resume => {
            if let Some(t) = track {
                t.player().play();
            }
        }
        Cmd::Seek(at) => {
            if let Some(t) = track {
                t.seek(at);
            }
        }
        Cmd::Stop => {
            *track = None;
            let mut s = state.lock();
            s.playing = false;
            s.key = None;
            s.pos = 0.0;
        }
        Cmd::Volume => {
            if let Some(t) = track {
                t.player().set_volume(gain());
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A 16-bit PCM WAV: `secs` of a sine whose amplitude ramps up from 0 to `peak`.
    pub fn wav(secs: f32, rate: u32, peak: f32) -> Vec<u8> {
        let n = (secs * rate as f32) as usize;
        let mut data = Vec::with_capacity(n * 2);
        for i in 0..n {
            let amp = peak * i as f32 / n as f32;
            let s = (amp * (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 32767.0) as i16;
            data.extend_from_slice(&s.to_le_bytes());
        }
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data.len() as u32).to_le_bytes());
        b.extend_from_slice(&data);
        b
    }

    /// The FFmpeg runtime for tests: `SOYUZ_FFMPEG_DIR` or the libraries next to the test
    /// binary. `SOYUZ_MEDIA_TESTS=1` turns a missing runtime into a failure.
    pub(crate) fn test_runtime() -> Option<Arc<Runtime>> {
        let strict = std::env::var("SOYUZ_MEDIA_TESTS").is_ok_and(|v| v == "1");
        let dirs: Vec<PathBuf> = match std::env::var_os("SOYUZ_FFMPEG_DIR") {
            Some(d) => vec![PathBuf::from(d)],
            None => soyuz_media::search_dirs("", &std::env::temp_dir().join("soyuz-appcore-media-tests")),
        };
        match Runtime::get(&dirs) {
            Ok(rt) => Some(rt),
            Err(e) if strict => panic!("SOYUZ_MEDIA_TESTS=1 but no FFmpeg runtime loads: {e}"),
            Err(e) => {
                eprintln!("skipped: no FFmpeg runtime ({e})");
                None
            }
        }
    }

    /// The player and the backend setting are process-wide: tests driving them run one
    /// at a time.
    static SERIAL: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../soyuz-media/tests/fixtures").join(name)
    }

    /// Play `src`, wait for the position to move (or an error: no device on CI), check
    /// pause / seek / stop. Returns the final state when the machine has a device.
    fn play_and_check(src: AudioSrc, ext: &str, want: Decoder) -> Option<PlayerState> {
        play(src.clone(), ext, 0.0);
        let start = Instant::now();
        let mut s = state();
        while start.elapsed() < Duration::from_secs(5) {
            s = state();
            if s.error.is_some() || s.pos > 0.05 {
                break;
            }
            heartbeat();
            std::thread::sleep(Duration::from_millis(20));
        }
        if s.error.is_some() {
            eprintln!("skipped playback checks: {:?}", s.error);
            stop();
            return None;
        }
        assert_eq!(s.key, Some(src.key()));
        assert!(s.pos > 0.0, "position advances");
        assert_eq!(s.decoder, want);
        pause();
        std::thread::sleep(Duration::from_millis(200));
        assert!(!state().playing);
        seek(1.0);
        heartbeat();
        resume();
        std::thread::sleep(Duration::from_millis(250));
        let after = state();
        assert!(after.pos >= 0.9, "seek landed: {}", after.pos);
        stop();
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(state().key, None);
        Some(after)
    }

    #[test]
    fn player_plays_pauses_and_stops() {
        let _serial = SERIAL.lock();
        // Silent track; machines without an audio device report an error instead.
        set_backend(AudioBackend::Symphonia);
        let src = AudioSrc::Mem(wav(2.0, 8000, 0.0).into());
        play_and_check(src, "wav", Decoder::Symphonia);
        set_backend(AudioBackend::Auto);
    }

    #[test]
    fn downsampling_keeps_extremes() {
        let src: Vec<(f32, f32)> = (0..1000).map(|i| (-(i as f32) / 1000.0, i as f32 / 1000.0)).collect();
        let d = downsample(&src, 10);
        assert_eq!(d.len(), 10);
        assert_eq!(d[0], (-0.099, 0.099));
        assert_eq!(d[9].1, 0.999);
        assert_eq!(downsample(&src[..5], 10).len(), 5, "short input passes through");
        assert!(downsample(&[], 10).is_empty());
        assert!(downsample(&src, 0).is_empty());
        // Uneven split: every source bucket lands somewhere.
        let d = downsample(&src[..7], 3);
        assert_eq!(d.len(), 3);
        assert_eq!(d[2].1, 0.006);
    }

    #[test]
    fn wav_waveform_and_info() {
        let src = AudioSrc::Mem(wav(1.0, 8000, 0.8).into());
        let info = analyze_symphonia(&src, "wav", true, &|| false).unwrap();
        assert_eq!(info.sample_rate, Some(8000));
        assert_eq!(info.channels, Some(1));
        assert!((info.duration.unwrap() - 1.0).abs() < 0.01);
        assert!(!info.peaks.is_empty() && info.peaks.len() <= WAVE_POINTS);
        assert_eq!(info.covered, 1.0);
        assert_eq!(info.decoder, Decoder::Symphonia);
        let first = info.peaks.first().unwrap().1;
        let last = info.peaks.last().unwrap().1;
        assert!(first < 0.1 && last > 0.6, "ramp: {first} → {last}");
        let quick = analyze_symphonia(&src, "wav", false, &|| false).unwrap();
        assert!(quick.peaks.is_empty());
        assert!(analyze(&AudioSrc::Mem(b"garbage!".to_vec().into()), "mp3", true, &|| false).is_err());
    }

    #[test]
    fn backend_setting_is_read() {
        let _serial = SERIAL.lock();
        set_backend(AudioBackend::Symphonia);
        assert_eq!(backend(), AudioBackend::Symphonia);
        assert!(matches!(pick(), Ok((Backend::Symphonia, false))));
        set_backend(AudioBackend::Ffmpeg);
        assert_eq!(backend(), AudioBackend::Ffmpeg);
        // Without libraries `ffmpeg` is an error (another test may load them meanwhile).
        if let Err(err) = pick() {
            assert_eq!(err, tr!("services-audio-no-ffmpeg"));
            let src = AudioSrc::Mem(wav(0.2, 8000, 0.5).into());
            if let Err(e) = analyze(&src, "wav", true, &|| false) {
                assert_eq!(e, err);
            }
        }
        set_backend(AudioBackend::Auto);
        assert_eq!(backend(), AudioBackend::Auto);
        configure(&MediaCfg { audio_backend: AudioBackend::Symphonia, volume: 150, muted: true, ..Default::default() });
        assert_eq!(backend(), AudioBackend::Symphonia);
        assert_eq!(volume(), (100, true));
        assert_eq!(gain(), 0.0);
        configure(&MediaCfg::default());
        assert_eq!(volume(), (80, false));
        assert!((gain() - 0.8).abs() < 1e-6);
        assert_eq!(backend(), AudioBackend::Auto);
    }

    #[test]
    fn helpers_map_ffmpeg_names() {
        assert_eq!(ffmpeg_tag_label("ARTIST"), Some("Artist"));
        assert_eq!(ffmpeg_tag_label("album_artist"), Some("Album artist"));
        assert_eq!(ffmpeg_tag_label("encoder"), None);
        assert_eq!(sample_fmt_bits(Some("s16p")), Some(16));
        assert_eq!(sample_fmt_bits(Some("fltp")), None);
        assert_eq!(sample_fmt_bits(None), None);
        assert!(Decoder::Ffmpeg(63).label().contains("63"));
        assert!(!Decoder::Symphonia.label().is_empty());
        let src = AudioSrc::Mem(Arc::from(&b"abc"[..]));
        assert!(matches!(src.media_source(), FfSource::Mem(b) if &b[..] == b"abc"));
        assert!(
            matches!(AudioSrc::File("x.ogg".into()).media_source(), FfSource::File(p) if p == std::path::Path::new("x.ogg"))
        );
    }

    #[test]
    fn metadata_block_picture_is_parsed() {
        // A FLAC PICTURE block: type 3 (front cover), MIME "image/png", empty description,
        // 16x16x24, 0 colours, 20 bytes of "data".
        let mut block = Vec::new();
        block.extend_from_slice(&3u32.to_be_bytes());
        block.extend_from_slice(&9u32.to_be_bytes());
        block.extend_from_slice(b"image/png");
        block.extend_from_slice(&0u32.to_be_bytes());
        for v in [16u32, 16, 24, 0] {
            block.extend_from_slice(&v.to_be_bytes());
        }
        let data: Vec<u8> = (0..20u8).collect();
        block.extend_from_slice(&(data.len() as u32).to_be_bytes());
        block.extend_from_slice(&data);
        let b64 = base64::engine::general_purpose::STANDARD.encode(&block);
        assert_eq!(metadata_block_picture(&b64), Some(data));
        assert_eq!(metadata_block_picture("not base64!"), None);
        assert_eq!(metadata_block_picture(&base64::engine::general_purpose::STANDARD.encode(b"short")), None);
        // A truncated data field is rejected, not read past the end.
        block.truncate(block.len() - 5);
        assert_eq!(metadata_block_picture(&base64::engine::general_purpose::STANDARD.encode(&block)), None);
    }

    /// Both backends read the same generated WAV: same duration (± 1 ms), facts and a
    /// waveform. Needs the FFmpeg runtime (`SOYUZ_FFMPEG_DIR`).
    #[test]
    fn with_runtime_both_backends_agree_on_a_wav() {
        let Some(rt) = test_runtime() else { return };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ramp.wav");
        std::fs::write(&path, wav(1.0, 8000, 0.8)).unwrap();
        for src in [AudioSrc::File(path.clone()), AudioSrc::Mem(wav(1.0, 8000, 0.8).into())] {
            let sym = analyze_symphonia(&src, "wav", true, &|| false).unwrap();
            let ff = analyze_ffmpeg(&rt, &src, true, &|| false).unwrap();
            assert_eq!(ff.decoder, Decoder::Ffmpeg(rt.versions().majors().avcodec));
            assert!(
                (ff.duration.unwrap() - sym.duration.unwrap()).abs() <= 0.001,
                "{:?} vs {:?}",
                ff.duration,
                sym.duration
            );
            assert_eq!(ff.sample_rate, sym.sample_rate);
            assert_eq!(ff.channels, sym.channels);
            assert_eq!(ff.bits, Some(16));
            assert!(ff.codec.as_deref().is_some_and(|c| c.to_ascii_lowercase().contains("pcm")), "{:?}", ff.codec);
            assert!(!ff.peaks.is_empty() && ff.peaks.len() <= WAVE_POINTS);
            assert_eq!(ff.covered, 1.0);
            // Buckets are shorter than a period of the sine, so compare the envelope of
            // the first and last 5 % of the waveform.
            let n = ff.peaks.len();
            let env = |r: std::ops::Range<usize>| ff.peaks[r].iter().map(|p| p.1).fold(f32::MIN, f32::max);
            let (first, last) = (env(0..n / 20), env(n - n / 20..n));
            assert!(first < 0.1 && last > 0.6, "ramp: {first} → {last}");
            let quick = analyze_ffmpeg(&rt, &src, false, &|| false).unwrap();
            assert!(quick.peaks.is_empty() && quick.duration.is_some());
            assert!(matches!(analyze_ffmpeg(&rt, &src, true, &|| true), Err(e) if e == "stale"));
        }
        assert!(analyze_ffmpeg(&rt, &AudioSrc::Mem(b"garbage!".to_vec().into()), true, &|| false).is_err());
    }

    /// The FLAC fixture with an attached picture: tags and cover through both backends.
    #[test]
    fn with_runtime_cover_and_tags_match() {
        let Some(rt) = test_runtime() else { return };
        let src = AudioSrc::File(fixture("cover.flac"));
        let ff = analyze_ffmpeg(&rt, &src, true, &|| false).unwrap();
        let sym = analyze_symphonia(&src, "flac", true, &|| false).unwrap();
        let tag = |i: &AudioInfo, k: &str| i.tags.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
        assert_eq!(tag(&ff, "Title"), Some("Cover".into()));
        assert_eq!(tag(&ff, "Artist"), Some("Soyuz".into()));
        assert_eq!(tag(&ff, "Title"), tag(&sym, "Title"));
        let cover = ff.cover.as_deref().expect("cover from the attached picture");
        assert!(cover.starts_with(b"\x89PNG"));
        assert_eq!(ff.cover, sym.cover);
        assert!(
            (ff.duration.unwrap() - sym.duration.unwrap()).abs() <= 0.001,
            "{:?} vs {:?}",
            ff.duration,
            sym.duration
        );
        assert_eq!(ff.sample_rate, Some(8000));
        assert!(!ff.peaks.is_empty());
    }

    /// Playback through the FFmpeg feeder (skips its checks without an audio device).
    #[test]
    fn with_runtime_ffmpeg_track_plays() {
        let _serial = SERIAL.lock();
        let Some(rt) = test_runtime() else { return };
        let dir = tempfile::tempdir().unwrap();
        media::configure_in(true, "", dir.path().to_path_buf());
        let start = Instant::now();
        while media::runtime().is_none() && start.elapsed() < Duration::from_secs(60) {
            std::thread::sleep(Duration::from_millis(20));
        }
        if media::runtime().is_none() {
            eprintln!("skipped: services::media did not load the runtime");
            return;
        }
        let path = dir.path().join("silence.wav");
        std::fs::write(&path, wav(3.0, 8000, 0.0)).unwrap();
        set_backend(AudioBackend::Ffmpeg);
        let want = Decoder::Ffmpeg(rt.versions().majors().avcodec);
        let r = play_and_check(AudioSrc::File(path.clone()), "wav", want);
        set_backend(AudioBackend::Auto);
        if r.is_some() {
            // Also from memory (an archive entry), with the automatic choice.
            let r = play_and_check(AudioSrc::Mem(wav(3.0, 8000, 0.0).into()), "wav", want);
            assert!(r.is_some());
        }
    }
}
