//! The load-time self-test: decode an in-memory WAV and a 2-frame `I420` AVI (both from
//! `crate::fixtures`) through the custom AVIO layer, the demuxer, a decoder, the scaler
//! and the resampler, and compare with the known values. Guards against a misread
//! struct layout (wrong majors are refused before this runs) and a broken library set.

use std::sync::Arc;

use super::codec::{Decoder, Frame, Recv, pix_fmt_name};
use super::demux::{Demuxer, Limits, Packet};
use super::loader::Av;
use super::resample::Resampler;
use super::scale::{Quality, Scaler};
use super::types::*;
use crate::fixtures;
use crate::info::{Source, StreamKind};

const RATE: u32 = 8000;
const SAMPLES: usize = 800;

/// `Err(reason)` when anything differs from the expectation.
pub(crate) fn self_test(av: &'static Av) -> Result<(), String> {
    audio(av).map_err(|e| format!("audio: {e}"))?;
    video(av).map_err(|e| format!("video: {e}"))?;
    Ok(())
}

fn audio(av: &'static Av) -> Result<(), String> {
    let samples: Vec<i16> = (0..SAMPLES).map(|i| ((i % 100) as i16 - 50) * 300).collect();
    let bytes = fixtures::wav(RATE, 1, &samples);
    let src = Source::Mem(Arc::from(bytes.into_boxed_slice()));
    let mut d = Demuxer::open(av, &src, None, Limits::default()).map_err(|e| format!("open: {e}"))?;
    if d.stream_count() != 1 {
        return Err(format!("{} streams, expected 1", d.stream_count()));
    }
    let s = d.stream(0).ok_or("no stream")?;
    let par = s.par();
    if par.codec_type != AVMEDIA_TYPE_AUDIO || par.codec_id != AV_CODEC_ID_PCM_S16LE {
        return Err(format!("codec_type {} codec_id {}", par.codec_type, par.codec_id));
    }
    if par.sample_rate != RATE as i32 || par.ch_layout.nb_channels != 1 {
        return Err(format!("{} Hz, {} channels", par.sample_rate, par.ch_layout.nb_channels));
    }
    let dur = d.duration().ok_or("no duration")?;
    if (dur - 0.1).abs() > 0.002 {
        return Err(format!("duration {dur}"));
    }
    let (idx, codec) = d.best_stream(StreamKind::Audio, None).map_err(|e| e.to_string())?;
    if idx != 0 {
        return Err(format!("best stream {idx}"));
    }
    let mut dec =
        Decoder::open(av, codec, s.par_ptr(), s.time_base(), &[("threads", "1")]).map_err(|e| e.to_string())?;
    let layout = dec.ch_layout().map_err(|e| e.to_string())?;
    // A WAV without a channel mask has an unspecified-order layout ("1 channels").
    if layout.channels() != 1 || !matches!(layout.describe().as_str(), "mono" | "1 channels") {
        return Err(format!("decoder layout {} ({})", layout.channels(), layout.describe()));
    }
    let mut pkt = Packet::new(av).map_err(|e| e.to_string())?;
    let mut frame = Frame::new(av).map_err(|e| e.to_string())?;
    let mut total = 0usize;
    let mut first: Option<i16> = None;
    let mut floats: Vec<f32> = Vec::new();
    let mut rs: Option<Resampler> = None;
    let mut eof = false;
    while !eof {
        let more = d.read(&mut pkt).map_err(|e| format!("read: {e}"))?;
        if more && pkt.stream_index() != 0 {
            continue;
        }
        dec.send(if more { Some(&pkt) } else { None }).map_err(|e| format!("send: {e}"))?;
        eof = !more;
        loop {
            match dec.receive(&mut frame).map_err(|e| format!("receive: {e}"))? {
                Recv::Frame => {}
                Recv::Again => break,
                Recv::Eof => {
                    eof = true;
                    break;
                }
            }
            let n = usize::try_from(frame.nb_samples()).unwrap_or(0);
            if frame.sample_rate() != RATE as i32 {
                return Err(format!("frame rate {}", frame.sample_rate()));
            }
            if first.is_none() {
                let plane = frame.plane(0, 2).ok_or("no plane")?;
                first = Some(i16::from_le_bytes([plane[0], plane[1]]));
                if frame.pts() != 0 {
                    return Err(format!("first pts {}", frame.pts()));
                }
            }
            if rs.is_none() {
                rs = Some(
                    Resampler::new(av, &layout, frame.format(), frame.sample_rate(), RATE)
                        .map_err(|e| e.to_string())?,
                );
            }
            rs.as_mut().expect("set").convert(Some(&frame), &mut floats).map_err(|e| e.to_string())?;
            total += n;
        }
    }
    if let Some(rs) = rs.as_mut() {
        rs.convert(None, &mut floats).map_err(|e| e.to_string())?;
    }
    if total != SAMPLES {
        return Err(format!("{total} samples decoded, expected {SAMPLES}"));
    }
    if first != Some(samples[0]) {
        return Err(format!("first sample {first:?}, expected {}", samples[0]));
    }
    if floats.len() != SAMPLES {
        return Err(format!("{} resampled samples", floats.len()));
    }
    let want = f32::from(samples[10]) / f32::from(i16::MAX);
    if (floats[10] - want).abs() > 0.001 {
        return Err(format!("resampled sample 10 = {}, expected {want}", floats[10]));
    }
    Ok(())
}

fn video(av: &'static Av) -> Result<(), String> {
    const W: u32 = 8;
    const H: u32 = 8;
    let frames = vec![fixtures::i420_solid(W, H, 235, 128, 128), fixtures::i420_solid(W, H, 16, 128, 128)];
    let bytes = fixtures::avi(W, H, 10, fixtures::AviCodec::I420, &frames);
    let src = Source::Mem(Arc::from(bytes.into_boxed_slice()));
    let mut d = Demuxer::open(av, &src, None, Limits::default()).map_err(|e| format!("open: {e}"))?;
    if d.stream_count() != 1 {
        return Err(format!("{} streams, expected 1", d.stream_count()));
    }
    let s = d.stream(0).ok_or("no stream")?;
    let par = s.par();
    if par.codec_type != AVMEDIA_TYPE_VIDEO || par.codec_id != AV_CODEC_ID_RAWVIDEO {
        return Err(format!("codec_type {} codec_id {}", par.codec_type, par.codec_id));
    }
    if par.width != W as i32 || par.height != H as i32 {
        return Err(format!("{}x{}", par.width, par.height));
    }
    if pix_fmt_name(av, par.format).as_deref() != Some("yuv420p") {
        return Err(format!("pixel format {:?}", pix_fmt_name(av, par.format)));
    }
    let fps = d.guess_frame_rate(s);
    if fps.to_f64().map(|f| (f - 10.0).abs() > 0.01).unwrap_or(true) {
        return Err(format!("frame rate {fps:?}"));
    }
    let tb = s.time_base();
    let (name, _) = d.format();
    if name != "avi" {
        return Err(format!("format {name}"));
    }
    let (idx, codec) = d.best_stream(StreamKind::Video, None).map_err(|e| e.to_string())?;
    if idx != 0 {
        return Err(format!("best stream {idx}"));
    }
    let mut dec = Decoder::open(av, codec, s.par_ptr(), tb, &[("threads", "1")]).map_err(|e| e.to_string())?;
    let mut pkt = Packet::new(av).map_err(|e| e.to_string())?;
    let mut frame = Frame::new(av).map_err(|e| e.to_string())?;
    let mut rgba = Vec::new();
    let mut seen: Vec<(f64, u8)> = Vec::new();
    let mut decode_all = |d: &mut Demuxer<'_>, dec: &mut Decoder, seen: &mut Vec<(f64, u8)>| -> Result<(), String> {
        let mut eof = false;
        while !eof {
            let more = d.read(&mut pkt).map_err(|e| format!("read: {e}"))?;
            dec.send(if more { Some(&pkt) } else { None }).map_err(|e| format!("send: {e}"))?;
            eof = !more;
            loop {
                match dec.receive(&mut frame).map_err(|e| format!("receive: {e}"))? {
                    Recv::Frame => {}
                    Recv::Again => break,
                    Recv::Eof => {
                        eof = true;
                        break;
                    }
                }
                if frame.width() != W as i32 || frame.height() != H as i32 {
                    return Err(format!("frame {}x{}", frame.width(), frame.height()));
                }
                let mut sc = Scaler::new(av, frame.width(), frame.height(), frame.format(), 4, 4, Quality::Area)
                    .map_err(|e| e.to_string())?;
                sc.scale(&frame, &mut rgba).map_err(|e| e.to_string())?;
                if rgba.len() != 4 * 4 * 4 {
                    return Err(format!("{} rgba bytes", rgba.len()));
                }
                let secs = tb.secs(frame.best_effort_timestamp()).ok_or("no timestamp")?;
                seen.push((secs, rgba[0]));
            }
        }
        Ok(())
    };
    decode_all(&mut d, &mut dec, &mut seen)?;
    if seen.len() != 2 {
        return Err(format!("{} frames decoded, expected 2", seen.len()));
    }
    if (seen[0].0 - 0.0).abs() > 1e-6 || (seen[1].0 - 0.1).abs() > 1e-6 {
        return Err(format!("timestamps {:?}", seen));
    }
    // Limited-range white (Y 235) scales to ~255, black (Y 16) to ~0.
    if seen[0].1 < 240 || seen[1].1 > 15 {
        return Err(format!("pixels {:?} (white, black expected)", seen.iter().map(|s| s.1).collect::<Vec<_>>()));
    }
    // Seek back through our AVIO seek callback and decode again.
    d.seek_before(Some(0), 0).map_err(|e| format!("seek: {e}"))?;
    dec.flush();
    seen.clear();
    decode_all(&mut d, &mut dec, &mut seen)?;
    if seen.len() != 2 || seen[0].1 < 240 {
        return Err(format!("after seek: {seen:?}"));
    }
    Ok(())
}
