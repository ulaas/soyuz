//! One decoded picture ([`grab_frame`]): poster frames and thumbnails.

use crate::decode::{Decoder, Demuxer, Frame, Limits, Packet, Quality, Recv, Scaler};
use crate::info::{Source, StreamKind};
use crate::sys::layout::AVRational;
use crate::sys::{codec, types};
use crate::{MediaError, Runtime};

/// Which frame, and how big.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameRequest {
    /// Seconds from the start.
    pub at: f64,
    /// Longest edge of the result (the frame is scaled down to fit, never up).
    pub max_px: u32,
    /// `true`: the first frame at or after `at` (decode from the previous keyframe);
    /// `false`: the nearest keyframe before `at` (`skip_frame = nokey`, one decoder
    /// thread; what thumbnails use).
    pub exact: bool,
}

impl FrameRequest {
    /// A keyframe near `at`, fitted into `max_px` (thumbnails, posters).
    pub fn near(at: f64, max_px: u32) -> FrameRequest {
        FrameRequest { at, max_px, exact: false }
    }
}

/// Decode one frame of the first video stream of `src` to RGBA, rotated as the stream's
/// display matrix says and fitted into `req.max_px`. `stale` is polled between packets
/// (and inside every read of the source); `true` stops the work with
/// [`MediaError::Stale`].
pub fn grab_frame(
    rt: &Runtime,
    src: &Source,
    req: FrameRequest,
    stale: &dyn Fn() -> bool,
) -> Result<image::RgbaImage, MediaError> {
    let av = rt.av();
    let mut d = Demuxer::open(av, src, Some(stale), Limits::default())?;
    let (idx, decoder) = d.best_stream(StreamKind::Video, None)?;
    let s = d.stream(idx).ok_or(MediaError::NoStream(StreamKind::Video))?;
    let tb = s.time_base();
    let rotation = s.side_data(types::AV_PKT_DATA_DISPLAYMATRIX).map_or(0, |m| codec::display_rotation(av, m));
    let sar = d.guess_sar(s);
    let attached = s.disposition() & types::AV_DISPOSITION_ATTACHED_PIC != 0;
    let start = s.start_time();
    let start_secs = tb.secs(start).unwrap_or(0.0);
    let opts: &[(&str, &str)] =
        if req.exact { &[("threads", "auto")] } else { &[("threads", "1"), ("skip_frame", "nokey")] };
    let mut dec = Decoder::open(av, decoder, s.par_ptr(), tb, opts)?;

    let at = if req.at.is_finite() { req.at.max(0.0) } else { 0.0 };
    let target = start_secs + at;
    if at > 0.0 && !attached && tb.num > 0 && tb.den > 0 {
        let ts = (target * f64::from(tb.den) / f64::from(tb.num)).round() as i64;
        if let Err(e) = d.seek_before(Some(idx), ts) {
            if matches!(e, MediaError::Stale) {
                return Err(e);
            }
            // Not seekable from here: decode from the start instead.
            log::debug!("soyuz-media: seek to {at}s failed ({e}); decoding from the start");
        }
    }

    let mut pkt = Packet::new(av)?;
    // Two frames: `receive` fills the one `cur` names while the other keeps the last
    // candidate (the frame just before `target` when the stream ends early).
    let mut frames = [Frame::new(av)?, Frame::new(av)?];
    let mut cur = 0usize;
    let mut have_last = false;
    let mut eof = false;
    let fit = |frame: &Frame| fit_and_rotate(av, frame, sar, req.max_px, rotation);
    while !eof {
        if stale() {
            return Err(MediaError::Stale);
        }
        let more = d.read(&mut pkt)?;
        if more && pkt.stream_index() != idx {
            continue;
        }
        dec.send(if more { Some(&pkt) } else { None })?;
        eof = !more;
        loop {
            match dec.receive(&mut frames[cur])? {
                Recv::Frame => {}
                Recv::Again => break,
                Recv::Eof => {
                    eof = true;
                    break;
                }
            }
            let f = &frames[cur];
            let secs = tb.secs(f.best_effort_timestamp()).or_else(|| tb.secs(f.pts()));
            let take = !req.exact || secs.is_none_or(|s| s + 1e-3 >= target);
            if take {
                return fit(f);
            }
            have_last = true;
            cur ^= 1;
        }
    }
    if have_last {
        return fit(&frames[cur ^ 1]);
    }
    Err(MediaError::Unsupported("no decodable frame".into()))
}

/// Scale `frame` to RGBA so that its display size (sample aspect ratio applied) fits
/// into `max_px`, then rotate it clockwise by `rotation` degrees.
fn fit_and_rotate(
    av: &'static crate::sys::Av,
    frame: &Frame,
    sar: AVRational,
    max_px: u32,
    rotation: i32,
) -> Result<image::RgbaImage, MediaError> {
    let (fw, fh) = (frame.width(), frame.height());
    if fw <= 0 || fh <= 0 {
        return Err(MediaError::Unsupported("empty frame".into()));
    }
    let (mut w, mut h) = (f64::from(fw), f64::from(fh));
    if sar.num > 0 && sar.den > 0 && sar.num != sar.den {
        if sar.num > sar.den {
            w *= f64::from(sar.num) / f64::from(sar.den);
        } else {
            h *= f64::from(sar.den) / f64::from(sar.num);
        }
    }
    let max_px = f64::from(max_px.max(1));
    let scale = (max_px / w.max(h)).min(1.0);
    let dw = ((w * scale).round() as u32).max(1);
    let dh = ((h * scale).round() as u32).max(1);
    let quality = if scale < 0.5 { Quality::Area } else { Quality::Bilinear };
    let mut sc = Scaler::new(av, fw, fh, frame.format(), dw, dh, quality)?;
    let mut rgba = Vec::new();
    sc.scale(frame, &mut rgba)?;
    let img =
        image::RgbaImage::from_raw(dw, dh, rgba).ok_or_else(|| MediaError::Unsupported("rgba buffer size".into()))?;
    Ok(match rotation {
        90 => image::imageops::rotate90(&img),
        180 => image::imageops::rotate180(&img),
        270 => image::imageops::rotate270(&img),
        _ => img,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_helpers() {
        let r = FrameRequest::near(1.5, 256);
        assert_eq!(r, FrameRequest { at: 1.5, max_px: 256, exact: false });
    }
}
