//! Tiny media files written by hand (no encoder needed): a PCM WAV and an AVI holding
//! raw `I420` or Motion-JPEG frames. The load-time self-test decodes them in memory,
//! and the crate's tests generate their inputs with them; `#[doc(hidden)]` because they
//! are not part of the media API.

#![doc(hidden)]

/// A 16-bit PCM WAV of interleaved `samples` (`channels` per frame) at `rate` Hz.
pub fn wav(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// The video codec of an [`avi`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AviCodec {
    /// Raw planar YUV 4:2:0 (`rawvideo`, pixel format `yuv420p`): each frame is
    /// `width * height * 3 / 2` bytes (Y plane, then U, then V).
    I420,
    /// Motion-JPEG: each frame is a baseline JPEG file.
    Mjpeg,
}

/// An AVI with one video stream of `frames` at `fps` frames per second.
pub fn avi(width: u32, height: u32, fps: u32, codec: AviCodec, frames: &[Vec<u8>]) -> Vec<u8> {
    let fourcc: &[u8; 4] = match codec {
        AviCodec::I420 => b"I420",
        AviCodec::Mjpeg => b"MJPG",
    };
    let bit_count: u16 = match codec {
        AviCodec::I420 => 12,
        AviCodec::Mjpeg => 24,
    };
    let max_frame = frames.iter().map(Vec::len).max().unwrap_or(0) as u32;
    let n = frames.len() as u32;
    let fps = fps.max(1);

    let mut avih = Vec::new();
    push_u32s(
        &mut avih,
        &[
            1_000_000 / fps, // microseconds per frame
            max_frame * fps, // max bytes per second
            0,               // padding granularity
            0x10,            // AVIF_HASINDEX
            n,
            0, // initial frames
            1, // streams
            max_frame,
            width,
            height,
            0,
            0,
            0,
            0,
        ],
    );

    let mut strh = Vec::new();
    strh.extend_from_slice(b"vids");
    strh.extend_from_slice(fourcc);
    push_u32s(&mut strh, &[0, 0]); // flags; priority + language
    push_u32s(&mut strh, &[0, 1, fps, 0, n, max_frame]); // initial frames, scale, rate, start, length, buffer
    strh.extend_from_slice(&(-1i32).to_le_bytes()); // quality
    push_u32s(&mut strh, &[0]); // sample size
    for v in [0u16, 0, width as u16, height as u16] {
        strh.extend_from_slice(&v.to_le_bytes());
    }

    let mut strf = Vec::new();
    push_u32s(&mut strf, &[40, width, height]);
    strf.extend_from_slice(&1u16.to_le_bytes());
    strf.extend_from_slice(&bit_count.to_le_bytes());
    strf.extend_from_slice(fourcc);
    push_u32s(&mut strf, &[width * height * u32::from(bit_count) / 8, 0, 0, 0, 0]);

    let strl = list(b"strl", &[chunk(b"strh", &strh), chunk(b"strf", &strf)].concat());
    let hdrl = list(b"hdrl", &[chunk(b"avih", &avih), strl].concat());

    let mut movi_body = Vec::new();
    let mut idx = Vec::new();
    for f in frames {
        let offset = (movi_body.len() + 4) as u32; // from the 'movi' fourcc
        movi_body.extend_from_slice(&chunk(b"00dc", f));
        idx.extend_from_slice(b"00dc");
        push_u32s(&mut idx, &[0x10, offset, f.len() as u32]); // AVIIF_KEYFRAME
    }
    let movi = list(b"movi", &movi_body);
    let body = [b"AVI ".to_vec(), hdrl, movi, chunk(b"idx1", &idx)].concat();
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// One `I420` frame of a solid colour (`y`, `u`, `v`).
pub fn i420_solid(width: u32, height: u32, y: u8, u: u8, v: u8) -> Vec<u8> {
    let luma = (width * height) as usize;
    let chroma = (width.div_ceil(2) * height.div_ceil(2)) as usize;
    let mut f = vec![y; luma];
    f.extend(std::iter::repeat_n(u, chroma));
    f.extend(std::iter::repeat_n(v, chroma));
    f
}

/// A baseline JPEG of a solid RGB colour (for [`AviCodec::Mjpeg`]).
pub fn jpeg_solid(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(width, height, image::Rgb(rgb));
    let mut out = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90);
    img.write_with_encoder(enc).expect("jpeg encode of a solid image");
    out
}

/// `samples` of a sine at `hz` (amplitude `amp`), mono.
pub fn sine(rate: u32, hz: f64, amp: f64, samples: usize) -> Vec<i16> {
    (0..samples)
        .map(|i| {
            let t = i as f64 / f64::from(rate);
            ((t * hz * std::f64::consts::TAU).sin() * amp * f64::from(i16::MAX)) as i16
        })
        .collect()
}

fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 9);
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    if body.len() % 2 == 1 {
        out.push(0);
    }
    out
}

fn list(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut inner = Vec::with_capacity(body.len() + 4);
    inner.extend_from_slice(kind);
    inner.extend_from_slice(body);
    chunk(b"LIST", &inner)
}

fn push_u32s(out: &mut Vec<u8>, values: &[u32]) {
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_well_formed() {
        let w = wav(8000, 1, &[1, -1, 0]);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes([w[4], w[5], w[6], w[7]]), 36 + 6);
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([w[22], w[23]]), 1, "mono");
        assert_eq!(u32::from_le_bytes([w[24], w[25], w[26], w[27]]), 8000);
        assert_eq!(&w[36..40], b"data");
        assert_eq!(w.len(), 44 + 6);
        assert_eq!(&w[44..46], &1i16.to_le_bytes());
    }

    #[test]
    fn avi_structure() {
        let frames = vec![i420_solid(4, 4, 235, 128, 128), i420_solid(4, 4, 16, 128, 128)];
        assert_eq!(frames[0].len(), 24);
        let a = avi(4, 4, 10, AviCodec::I420, &frames);
        assert_eq!(&a[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes([a[4], a[5], a[6], a[7]]) as usize, a.len() - 8);
        assert_eq!(&a[8..12], b"AVI ");
        assert_eq!(&a[12..16], b"LIST");
        assert_eq!(&a[20..24], b"hdrl");
        let movi = a.windows(4).position(|w| w == b"movi").expect("movi list");
        assert_eq!(&a[movi + 4..movi + 8], b"00dc");
        let idx = a.windows(4).position(|w| w == b"idx1").expect("index");
        let first_offset = u32::from_le_bytes([a[idx + 16], a[idx + 17], a[idx + 18], a[idx + 19]]);
        assert_eq!(first_offset, 4, "first chunk right after the movi fourcc");
        let m = avi(8, 8, 25, AviCodec::Mjpeg, &[jpeg_solid(8, 8, [255, 0, 0])]);
        assert!(m.windows(4).any(|w| w == b"MJPG"));
        assert!(m.windows(2).any(|w| w == [0xFF, 0xD8]), "a JPEG SOI inside");
        assert_eq!(sine(8000, 440.0, 0.5, 4).len(), 4);
    }
}
