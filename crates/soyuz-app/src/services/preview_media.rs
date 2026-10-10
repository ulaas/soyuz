//! Cheap audio / video metadata for the preview pane ("audio/video metadata
//! only"). Hand-rolled header parsers for the common containers — WAV, FLAC, MP3
//! (ID3v2 + first MPEG frame), Ogg Vorbis/Opus, MP4/MOV/M4A (moov/mvhd/tkhd) and AVI —
//! that read at most a few boxes, never the media data itself. Everything is pure and
//! works on any `Read + Seek`, so archive entries can be probed from memory too.

use std::io::{Read, Seek, SeekFrom};

/// One `(label, value)` row shown in the preview's metadata card.
pub type Field = (String, String);

/// `3:25`, `1:02:03`; sub-second durations show as `0:00`.
pub fn format_duration(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "—".into();
    }
    let total = secs.round() as u64;
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

fn push(out: &mut Vec<Field>, k: &str, v: impl Into<String>) {
    let v = v.into();
    if !v.trim().is_empty() {
        out.push((k.to_string(), v.trim().to_string()));
    }
}

fn u16le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
fn u32be(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
fn u64be(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// Probe a media stream. `ext` (lower-case, no dot) only breaks ties; detection is by
/// magic bytes. Unknown formats return an empty list.
pub fn probe<R: Read + Seek + ?Sized>(r: &mut R, len: u64, ext: &str) -> Vec<Field> {
    let mut head = vec![0u8; 64 * 1024];
    let n = read_at(r, 0, &mut head).unwrap_or(0);
    head.truncate(n);
    let mut out = Vec::new();
    if head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WAVE") {
        wav(&head, &mut out);
    } else if head.starts_with(b"RIFF") && head.get(8..12) == Some(b"AVI ") {
        avi(&head, &mut out);
    } else if head.starts_with(b"fLaC") {
        flac(&head, &mut out);
    } else if head.starts_with(b"OggS") {
        ogg(&head, &mut out);
    } else if head.get(4..8).is_some_and(|t| t == b"ftyp" || t == b"moov" || t == b"mdat" || t == b"wide") {
        mp4(r, len, &mut out);
    } else if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        push(&mut out, "Container", if ext == "webm" { "WebM (Matroska)" } else { "Matroska" });
    } else if head.starts_with(b"ID3") || mpeg_frame(&head, 0).is_some() || ext == "mp3" {
        mp3(&head, len, &mut out);
    }
    out
}

fn read_at<R: Read + Seek + ?Sized>(r: &mut R, pos: u64, buf: &mut [u8]) -> std::io::Result<usize> {
    r.seek(SeekFrom::Start(pos))?;
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

// ---- WAV ----------------------------------------------------------------------------

fn wav(b: &[u8], out: &mut Vec<Field>) {
    push(out, "Format", tr!("services-media-wave"));
    let mut at = 12;
    let mut byte_rate = 0u32;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let size = u32le(b, at + 4).unwrap_or(0) as usize;
        let body = at + 8;
        if id == b"fmt " {
            let fmt = u16le(b, body).unwrap_or(0);
            let channels = u16le(b, body + 2).unwrap_or(0);
            let rate = u32le(b, body + 4).unwrap_or(0);
            byte_rate = u32le(b, body + 8).unwrap_or(0);
            let bits = u16le(b, body + 14).unwrap_or(0);
            push(
                out,
                "Codec",
                if fmt == 1 {
                    "PCM".to_string()
                } else if fmt == 3 {
                    tr!("services-media-ieee-float").into()
                } else {
                    format!("0x{fmt:04X}")
                },
            );
            push(out, "Sample rate", format!("{rate} Hz"));
            push(out, "Channels", channel_label(channels as u32));
            if bits > 0 {
                push(out, "Bit depth", tr!("services-media-bits", bits = bits.to_string()));
            }
        } else if id == b"data" && byte_rate > 0 {
            push(out, "Duration", format_duration(size as f64 / byte_rate as f64));
            break;
        }
        // Checked: a hostile size must end the walk, not wrap / panic.
        match body.checked_add(size).and_then(|n| n.checked_add(size & 1)) {
            Some(next) if next > at => at = next,
            _ => break,
        }
    }
}

fn channel_label(n: u32) -> String {
    match n {
        1 => tr!("services-media-mono").into(),
        2 => tr!("services-media-stereo").into(),
        6 => "5.1".into(),
        8 => "7.1".into(),
        n => n.to_string(),
    }
}

// ---- AVI ----------------------------------------------------------------------------

fn avi(b: &[u8], out: &mut Vec<Field>) {
    push(out, "Container", "AVI");
    let Some(pos) = b.windows(4).position(|w| w == b"avih") else { return };
    let s = pos + 8;
    let usec = u32le(b, s).unwrap_or(0);
    let frames = u32le(b, s + 16).unwrap_or(0);
    let (w, h) = (u32le(b, s + 32).unwrap_or(0), u32le(b, s + 36).unwrap_or(0));
    if w > 0 && h > 0 {
        push(out, "Dimensions", format!("{w} × {h}"));
    }
    if usec > 0 {
        push(out, "Frame rate", format!("{:.2} fps", 1_000_000.0 / usec as f64));
        push(out, "Duration", format_duration(frames as f64 * usec as f64 / 1_000_000.0));
    }
}

// ---- FLAC ---------------------------------------------------------------------------

fn flac(b: &[u8], out: &mut Vec<Field>) {
    push(out, "Format", tr!("services-media-flac"));
    // First metadata block must be STREAMINFO (type 0), 34 bytes, right after "fLaC".
    if b.get(4).map(|t| t & 0x7F) != Some(0) || b.len() < 8 + 18 {
        return;
    }
    let s = &b[8..];
    let rate = (u32::from(s[10]) << 12) | (u32::from(s[11]) << 4) | (u32::from(s[12]) >> 4);
    let channels = ((s[12] >> 1) & 0x07) as u32 + 1;
    let bits = ((((s[12] & 1) << 4) | (s[13] >> 4)) as u32) + 1;
    let samples = (u64::from(s[13] & 0x0F) << 32) | u64::from(u32::from_be_bytes([s[14], s[15], s[16], s[17]]));
    push(out, "Sample rate", format!("{rate} Hz"));
    push(out, "Channels", channel_label(channels));
    push(out, "Bit depth", tr!("services-media-bits", bits = bits.to_string()));
    if rate > 0 && samples > 0 {
        push(out, "Duration", format_duration(samples as f64 / rate as f64));
    }
}

// ---- Ogg ----------------------------------------------------------------------------

fn ogg(b: &[u8], out: &mut Vec<Field>) {
    if let Some(p) = b.windows(7).position(|w| w == b"\x01vorbis") {
        push(out, "Format", "Ogg Vorbis");
        let ch = b.get(p + 11).copied().unwrap_or(0);
        let rate = u32le(b, p + 12).unwrap_or(0);
        push(out, "Channels", channel_label(ch as u32));
        push(out, "Sample rate", format!("{rate} Hz"));
    } else if let Some(p) = b.windows(8).position(|w| w == b"OpusHead") {
        push(out, "Format", "Ogg Opus");
        let ch = b.get(p + 9).copied().unwrap_or(0);
        push(out, "Channels", channel_label(ch as u32));
        push(out, "Sample rate", format!("{} Hz", u32le(b, p + 12).unwrap_or(48_000)));
    } else {
        push(out, "Container", "Ogg");
    }
}

// ---- MP3 ----------------------------------------------------------------------------

/// `(bitrate kbps, sample rate Hz, channels, version label)` of an MPEG audio frame at `at`.
fn mpeg_frame(b: &[u8], at: usize) -> Option<(u32, u32, u32, &'static str)> {
    let h = b.get(at..at + 4)?;
    if h[0] != 0xFF || h[1] & 0xE0 != 0xE0 {
        return None;
    }
    let version = (h[1] >> 3) & 0x03; // 3 = MPEG1, 2 = MPEG2, 0 = MPEG2.5
    let layer = (h[1] >> 1) & 0x03; // 1 = Layer III
    if version == 1 || layer != 1 {
        return None;
    }
    let br_idx = (h[2] >> 4) as usize;
    let sr_idx = ((h[2] >> 2) & 0x03) as usize;
    if br_idx == 0 || br_idx == 15 || sr_idx == 3 {
        return None;
    }
    const BR_V1: [u32; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
    const BR_V2: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
    let (br, sr, label) = match version {
        3 => (BR_V1[br_idx], [44_100, 48_000, 32_000][sr_idx], "MPEG-1 Layer III"),
        2 => (BR_V2[br_idx], [22_050, 24_000, 16_000][sr_idx], "MPEG-2 Layer III"),
        _ => (BR_V2[br_idx], [11_025, 12_000, 8_000][sr_idx], "MPEG-2.5 Layer III"),
    };
    let channels = if h[3] >> 6 == 3 { 1 } else { 2 };
    Some((br, sr, channels, label))
}

fn syncsafe(b: &[u8]) -> usize {
    b.iter().take(4).fold(0usize, |acc, &x| (acc << 7) | (x & 0x7F) as usize)
}

fn mp3(b: &[u8], len: u64, out: &mut Vec<Field>) {
    let mut audio_start = 0usize;
    let mut tags: Vec<Field> = Vec::new();
    if b.starts_with(b"ID3") && b.len() >= 10 {
        let major = b[3];
        let size = syncsafe(&b[6..10]);
        audio_start = size.saturating_add(10);
        let end = audio_start.min(b.len());
        let mut at = 10;
        while at + 10 <= end {
            let id = &b[at..at + 4];
            if id[0] == 0 {
                break;
            }
            let fsize = if major >= 4 { syncsafe(&b[at + 4..at + 8]) } else { u32be(b, at + 4).unwrap_or(0) as usize };
            let body = at + 10;
            if fsize == 0 || body.saturating_add(fsize) > end {
                break;
            }
            let label = match id {
                b"TIT2" => Some("Title"),
                b"TPE1" => Some("Artist"),
                b"TALB" => Some("Album"),
                b"TYER" | b"TDRC" => Some("Year"),
                b"TCON" => Some("Genre"),
                _ => None,
            };
            if let Some(l) = label {
                push(&mut tags, l, id3_text(&b[body..body + fsize]));
            }
            at = body + fsize;
        }
    }
    // First frame after the tag (search a little in case of padding / junk).
    let frame = (audio_start..b.len().saturating_sub(4)).take(16 * 1024).find_map(|i| mpeg_frame(b, i));
    push(out, "Format", frame.map_or(tr!("services-media-mp3"), |f| f.3));
    if let Some((br, sr, ch, _)) = frame {
        push(out, "Bitrate", format!("{br} kbps"));
        push(out, "Sample rate", format!("{sr} Hz"));
        push(out, "Channels", channel_label(ch));
        if br > 0 {
            let audio = len.saturating_sub(audio_start as u64);
            push(out, "Duration", format!("≈ {}", format_duration(audio as f64 * 8.0 / (br as f64 * 1000.0))));
        }
    }
    out.extend(tags);
}

/// Decode an ID3v2 text frame body (encoding byte + text).
pub fn id3_text(body: &[u8]) -> String {
    let Some((&enc, rest)) = body.split_first() else { return String::new() };
    let s = match enc {
        1 | 2 => {
            let (mut bytes, mut be) = (rest, enc == 2);
            if bytes.len() >= 2 && (bytes[..2] == [0xFF, 0xFE] || bytes[..2] == [0xFE, 0xFF]) {
                be = bytes[0] == 0xFE;
                bytes = &bytes[2..];
            }
            let units: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
                .collect();
            String::from_utf16_lossy(&units)
        }
        3 => String::from_utf8_lossy(rest).into_owned(),
        _ => rest.iter().map(|&c| c as char).collect(),
    };
    s.trim_matches(char::from(0)).replace('\0', " / ")
}

// ---- MP4 / MOV ----------------------------------------------------------------------

fn mp4<R: Read + Seek + ?Sized>(r: &mut R, len: u64, out: &mut Vec<Field>) {
    let mut hdr = [0u8; 16];
    let mut pos = 0u64;
    let mut brand = String::new();
    let mut steps = 0;
    while pos + 8 <= len && steps < 64 {
        steps += 1;
        if read_at(r, pos, &mut hdr).unwrap_or(0) < 8 {
            break;
        }
        let mut size = u32be(&hdr, 0).unwrap_or(0) as u64;
        let kind = [hdr[4], hdr[5], hdr[6], hdr[7]];
        let mut header = 8u64;
        if size == 1 {
            size = u64be(&hdr, 8).unwrap_or(0);
            header = 16;
        } else if size == 0 {
            size = len.saturating_sub(pos);
        }
        if size < header {
            break;
        }
        if &kind == b"ftyp" {
            brand = String::from_utf8_lossy(&hdr[8..12]).trim().to_string();
        }
        if &kind == b"moov" {
            let n = (size - header).min(16 * 1024 * 1024) as usize;
            let mut moov = vec![0u8; n];
            let got = pos.checked_add(header).map_or(0, |at| read_at(r, at, &mut moov).unwrap_or(0));
            moov.truncate(got);
            moov_fields(&moov, out);
            break;
        }
        // Box sizes come from the file: a 64-bit size near u64::MAX must not overflow.
        match pos.checked_add(size) {
            Some(next) => pos = next,
            None => break,
        }
    }
    let container = match brand.as_str() {
        "qt" => tr!("services-media-quicktime").to_string(),
        "M4A" | "M4B" => tr!("services-media-mpeg4-audio").into(),
        "" => "MPEG-4".into(),
        b => format!("MPEG-4 ({b})"),
    };
    out.insert(0, ("Container".into(), container));
}

/// Iterate child boxes of a box body: `(type, body)`.
fn boxes(b: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut at = 0usize;
    std::iter::from_fn(move || {
        if at + 8 > b.len() {
            return None;
        }
        let mut size = u32be(b, at)? as usize;
        let kind: [u8; 4] = b[at + 4..at + 8].try_into().ok()?;
        let mut header = 8;
        if size == 1 {
            size = usize::try_from(u64be(b, at + 8)?).ok()?;
            header = 16;
        } else if size == 0 {
            size = b.len() - at;
        }
        let end = at.checked_add(size)?;
        if size < header || end > b.len() {
            return None;
        }
        let body = &b[at + header..end];
        at = end;
        Some((kind, body))
    })
}

fn moov_fields(moov: &[u8], out: &mut Vec<Field>) {
    let mut dims: Option<(u32, u32)> = None;
    let mut audio = false;
    let mut video = false;
    for (kind, body) in boxes(moov) {
        match &kind {
            b"mvhd" => {
                let v1 = body.first() == Some(&1);
                let (ts, dur) = if v1 {
                    (u32be(body, 20).unwrap_or(0) as u64, u64be(body, 24).unwrap_or(0))
                } else {
                    (u32be(body, 12).unwrap_or(0) as u64, u32be(body, 16).unwrap_or(0) as u64)
                };
                if ts > 0 {
                    push(out, "Duration", format_duration(dur as f64 / ts as f64));
                }
            }
            b"trak" => {
                for (k, tb) in boxes(body) {
                    if &k == b"tkhd" {
                        let off = if tb.first() == Some(&1) { 88 } else { 76 };
                        let (w, h) = (u32be(tb, off).unwrap_or(0) >> 16, u32be(tb, off + 4).unwrap_or(0) >> 16);
                        if w > 0 && h > 0 && dims.is_none() {
                            dims = Some((w, h));
                        }
                    }
                    if &k == b"mdia" {
                        for (mk, mb) in boxes(tb) {
                            if &mk == b"hdlr" {
                                match mb.get(8..12) {
                                    Some(b"vide") => video = true,
                                    Some(b"soun") => audio = true,
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if let Some((w, h)) = dims {
        push(out, "Dimensions", format!("{w} × {h}"));
    }
    let tracks = match (video, audio) {
        (true, true) => tr!("services-media-tracks-av"),
        (true, false) => tr!("services-media-tracks-video"),
        (false, true) => tr!("services-media-tracks-audio"),
        _ => "",
    };
    push(out, "Tracks", tracks);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn get<'a>(f: &'a [Field], k: &str) -> Option<&'a str> {
        f.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration(0.4), "0:00");
        assert_eq!(format_duration(205.0), "3:25");
        assert_eq!(format_duration(3723.0), "1:02:03");
        assert_eq!(format_duration(f64::NAN), "—");
    }

    fn wav_bytes(rate: u32, channels: u16, bits: u16, seconds: u32) -> Vec<u8> {
        let byte_rate = rate * channels as u32 * bits as u32 / 8;
        let data = byte_rate * seconds;
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&byte_rate.to_le_bytes());
        v.extend_from_slice(&(channels * bits / 8).to_le_bytes());
        v.extend_from_slice(&bits.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data.to_le_bytes());
        v.resize(v.len() + 64, 0);
        v
    }

    #[test]
    fn wav_header() {
        let b = wav_bytes(44_100, 2, 16, 125);
        let f = probe(&mut Cursor::new(&b), b.len() as u64, "wav");
        assert_eq!(get(&f, "Sample rate"), Some("44100 Hz"));
        assert_eq!(get(&f, "Channels"), Some("Stereo"));
        assert_eq!(get(&f, "Bit depth"), Some("16-bit"));
        assert_eq!(get(&f, "Duration"), Some("2:05"));
        assert_eq!(get(&f, "Codec"), Some("PCM"));
    }

    #[test]
    fn id3_and_mpeg_frame() {
        let mut b = Vec::new();
        let mut frames = Vec::new();
        for (id, text) in [(b"TIT2", "Song"), (b"TPE1", "Band")] {
            frames.extend_from_slice(id);
            frames.extend_from_slice(&((text.len() + 1) as u32).to_be_bytes());
            frames.extend_from_slice(&[0, 0, 3]);
            frames.extend_from_slice(text.as_bytes());
        }
        b.extend_from_slice(b"ID3\x03\x00\x00");
        let n = frames.len();
        b.extend_from_slice(&[(n >> 21) as u8 & 0x7F, (n >> 14) as u8 & 0x7F, (n >> 7) as u8 & 0x7F, n as u8 & 0x7F]);
        b.extend_from_slice(&frames);
        // MPEG-1 Layer III, 128 kbps, 44.1 kHz, joint stereo.
        b.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x44]);
        b.resize(b.len() + 16_000, 0);
        let f = probe(&mut Cursor::new(&b), b.len() as u64, "mp3");
        assert_eq!(get(&f, "Title"), Some("Song"));
        assert_eq!(get(&f, "Artist"), Some("Band"));
        assert_eq!(get(&f, "Bitrate"), Some("128 kbps"));
        assert_eq!(get(&f, "Sample rate"), Some("44100 Hz"));
        assert_eq!(get(&f, "Format"), Some("MPEG-1 Layer III"));
        assert!(get(&f, "Duration").unwrap().starts_with('≈'));
    }

    #[test]
    fn id3_utf16_text() {
        let mut body = vec![1u8, 0xFF, 0xFE];
        for u in "Ünï".encode_utf16() {
            body.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(id3_text(&body), "Ünï");
        assert_eq!(id3_text(&[0, b'a', b'b', 0]), "ab");
    }

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn mp4_moov_at_end() {
        let ftyp = mp4_box(b"ftyp", b"isom\0\0\0\0isom");
        let mdat = mp4_box(b"mdat", &[0u8; 1000]);
        let mut mvhd = vec![0u8; 100];
        mvhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
        mvhd[16..20].copy_from_slice(&90_500u32.to_be_bytes());
        let mut tkhd = vec![0u8; 84];
        tkhd[76..80].copy_from_slice(&(1920u32 << 16).to_be_bytes());
        tkhd[80..84].copy_from_slice(&(1080u32 << 16).to_be_bytes());
        let mut hdlr = vec![0u8; 24];
        hdlr[8..12].copy_from_slice(b"vide");
        let trak = mp4_box(b"trak", &[mp4_box(b"tkhd", &tkhd), mp4_box(b"mdia", &mp4_box(b"hdlr", &hdlr))].concat());
        let moov = mp4_box(b"moov", &[mp4_box(b"mvhd", &mvhd), trak].concat());
        let file = [ftyp, mdat, moov].concat();
        let f = probe(&mut Cursor::new(&file), file.len() as u64, "mp4");
        assert_eq!(get(&f, "Container"), Some("MPEG-4 (isom)"));
        assert_eq!(get(&f, "Duration"), Some("1:31"));
        assert_eq!(get(&f, "Dimensions"), Some("1920 × 1080"));
        assert_eq!(get(&f, "Tracks"), Some("Video"));
    }

    #[test]
    fn mp4_hostile_box_sizes_do_not_panic() {
        // A 64-bit largesize near u64::MAX used to overflow `pos += size` (debug panic).
        let ftyp = mp4_box(b"ftyp", b"isom\0\0\0\0isom");
        let mut huge = 1u32.to_be_bytes().to_vec();
        huge.extend_from_slice(b"mdat");
        huge.extend_from_slice(&(u64::MAX - 4).to_be_bytes());
        huge.extend_from_slice(&[0u8; 32]);
        let file = [ftyp.clone(), huge].concat();
        let f = probe(&mut Cursor::new(&file), file.len() as u64, "mp4");
        assert_eq!(get(&f, "Container"), Some("MPEG-4 (isom)"));
        // Same inside moov: a child box with an absurd largesize.
        let mut child = 1u32.to_be_bytes().to_vec();
        child.extend_from_slice(b"trak");
        child.extend_from_slice(&u64::MAX.to_be_bytes());
        let moov = mp4_box(b"moov", &child);
        let file = [ftyp.clone(), moov].concat();
        let _ = probe(&mut Cursor::new(&file), file.len() as u64, "mp4");
        // size == 0 ("to end of file") with a len smaller than pos must not underflow.
        let mut zero = 0u32.to_be_bytes().to_vec();
        zero.extend_from_slice(b"mdat");
        let file = [ftyp, zero].concat();
        let _ = probe(&mut Cursor::new(&file), 4, "mp4");
        // Hostile WAV chunk size.
        let mut wav = b"RIFF\0\0\0\0WAVEdata".to_vec();
        wav.extend_from_slice(&u32::MAX.to_le_bytes());
        wav.extend_from_slice(&[0u8; 16]);
        let _ = probe(&mut Cursor::new(&wav), wav.len() as u64, "wav");
    }

    #[test]
    fn flac_streaminfo() {
        let mut b = b"fLaC".to_vec();
        b.extend_from_slice(&[0x80, 0, 0, 34]);
        let mut si = [0u8; 34];
        // 44100 Hz (0x0AC44), 2 channels, 16 bit, 441000 samples.
        let rate: u32 = 44_100;
        si[10] = (rate >> 12) as u8;
        si[11] = (rate >> 4) as u8;
        // channels-1 = 1 (stereo); the top bit of bps-1 (15) is 0.
        si[12] = ((rate & 0x0F) << 4) as u8 | (1 << 1);
        si[13] = (((16 - 1) & 0x0F) << 4) as u8;
        si[14..18].copy_from_slice(&441_000u32.to_be_bytes());
        b.extend_from_slice(&si);
        let f = probe(&mut Cursor::new(&b), b.len() as u64, "flac");
        assert_eq!(get(&f, "Sample rate"), Some("44100 Hz"));
        assert_eq!(get(&f, "Channels"), Some("Stereo"));
        assert_eq!(get(&f, "Bit depth"), Some("16-bit"));
        assert_eq!(get(&f, "Duration"), Some("0:10"));
    }

    #[test]
    fn unknown_is_empty() {
        let b = b"hello world".to_vec();
        assert!(probe(&mut Cursor::new(&b), b.len() as u64, "bin").is_empty());
    }
}
