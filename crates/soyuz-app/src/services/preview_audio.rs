//! Audio previews (roadmap F3): a waveform (decoded with Symphonia, pure Rust), tags,
//! duration, bitrate and cover art, plus a tiny player (rodio → cpal → WASAPI on
//! Windows; no shell APIs) with play / pause / seek.
//!
//! The player lives on its own thread, which owns the output device. The UI sends
//! commands and reads a shared state snapshot, so a slow device open never blocks a
//! frame. Playback stops by itself shortly after no preview shows the track anymore
//! (the renderer sends a heartbeat every frame it draws the player).

use std::fs::File;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::Mutex;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey};
use symphonia::core::probe::Hint;

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

fn read_tags(rev: &MetadataRevision, info: &mut AudioInfo) {
    for t in rev.tags() {
        let Some(label) = t.std_key.and_then(tag_label) else { continue };
        let v = t.value.to_string();
        let v = v.trim();
        if !v.is_empty() && !info.tags.iter().any(|(k, _)| k == label) {
            info.tags.push((label.to_string(), v.chars().take(200).collect()));
        }
    }
    if info.cover.is_none()
        && let Some(v) = rev.visuals().iter().find(|v| v.data.len() > 16)
    {
        info.cover = Some(v.data.to_vec());
    }
}

/// Probe tags / stream info and (when `waveform`) decode the whole track into peaks.
pub fn analyze(src: &AudioSrc, ext: &str, waveform: bool, stale: &dyn Fn() -> bool) -> Result<AudioInfo, String> {
    let mss = MediaSourceStream::new(src.media()?, Default::default());
    let mut hint = Hint::new();
    if !ext.is_empty() {
        hint.with_extension(ext);
    }
    let mut probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions { enable_gapless: true, ..Default::default() }, &MetadataOptions::default())
        .map_err(|e| tr!("services-audio-unsupported", error = e.to_string()))?;
    let mut info = AudioInfo::default();
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
}

enum Cmd {
    Play { src: AudioSrc, ext: String, at: f64 },
    Pause,
    Resume,
    Seek(f64),
    Stop,
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

fn player() -> &'static Player {
    static P: OnceLock<Player> = OnceLock::new();
    P.get_or_init(|| {
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

fn audio_thread(rx: Receiver<Cmd>, state: Arc<Mutex<PlayerState>>, seen: Arc<Mutex<Instant>>) {
    let mut device: Option<rodio::MixerDeviceSink> = None;
    let mut track: Option<rodio::Player> = None;
    let mut idle_since = Instant::now();
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
                let mut s = state.lock();
                if t.empty() {
                    s.playing = false;
                    s.pos = 0.0;
                    track = None;
                } else {
                    s.pos = t.get_pos().as_secs_f64();
                    s.playing = !t.is_paused();
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

fn handle(
    cmd: Cmd,
    device: &mut Option<rodio::MixerDeviceSink>,
    track: &mut Option<rodio::Player>,
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
            let p = rodio::Player::connect_new(dev.mixer());
            let built = match &src {
                AudioSrc::File(path) => match File::open(path) {
                    Ok(f) => {
                        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
                        rodio::Decoder::builder()
                            .with_data(std::io::BufReader::new(f))
                            .with_byte_len(len)
                            .with_hint(&ext)
                            .with_seekable(true)
                            .build()
                            .map(|d| p.append(d))
                            .map_err(|e| e.to_string())
                    }
                    Err(e) => Err(e.to_string()),
                },
                AudioSrc::Mem(b) => rodio::Decoder::builder()
                    .with_data(Cursor::new(b.clone()))
                    .with_byte_len(b.len() as u64)
                    .with_hint(&ext)
                    .with_seekable(true)
                    .build()
                    .map(|d| p.append(d))
                    .map_err(|e| e.to_string()),
            };
            if let Err(e) = built {
                return fail(tr!("services-audio-cant-play", error = e.to_string()));
            }
            if at > 0.0 {
                let _ = p.try_seek(Duration::from_secs_f64(at));
            }
            p.play();
            *track = Some(p);
        }
        Cmd::Pause => {
            if let Some(t) = track {
                t.pause();
            }
        }
        Cmd::Resume => {
            if let Some(t) = track {
                t.play();
            }
        }
        Cmd::Seek(at) => {
            if let Some(t) = track {
                let _ = t.try_seek(Duration::from_secs_f64(at.max(0.0)));
            }
        }
        Cmd::Stop => {
            *track = None;
            let mut s = state.lock();
            s.playing = false;
            s.key = None;
            s.pos = 0.0;
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

    #[test]
    fn player_plays_pauses_and_stops() {
        // Silent track; machines without an audio device report an error instead.
        let src = AudioSrc::Mem(wav(2.0, 8000, 0.0).into());
        play(src.clone(), "wav", 0.0);
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
        if s.error.is_none() {
            assert_eq!(s.key, Some(src.key()));
            assert!(s.pos > 0.0, "position advances");
            pause();
            std::thread::sleep(Duration::from_millis(200));
            assert!(!state().playing);
            seek(1.0);
            stop();
            std::thread::sleep(Duration::from_millis(200));
            assert_eq!(state().key, None);
        }
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
        let info = analyze(&src, "wav", true, &|| false).unwrap();
        assert_eq!(info.sample_rate, Some(8000));
        assert_eq!(info.channels, Some(1));
        assert!((info.duration.unwrap() - 1.0).abs() < 0.01);
        assert!(!info.peaks.is_empty() && info.peaks.len() <= WAVE_POINTS);
        assert_eq!(info.covered, 1.0);
        let first = info.peaks.first().unwrap().1;
        let last = info.peaks.last().unwrap().1;
        assert!(first < 0.1 && last > 0.6, "ramp: {first} → {last}");
        let quick = analyze(&src, "wav", false, &|| false).unwrap();
        assert!(quick.peaks.is_empty());
        assert!(analyze(&AudioSrc::Mem(b"garbage!".to_vec().into()), "mp3", true, &|| false).is_err());
    }
}
