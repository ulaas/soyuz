//! MP4 / MOV "faststart": move the index (`moov`) in front of the samples (`mdat`) of a
//! finished file, in place, so a player can start before it has the whole file.
//!
//! FFmpeg's own `movflags=+faststart` re-opens the output by URL to read it back, which
//! our write AVIO (a Rust writer, no URL) cannot offer; this does the same afterwards in
//! plain Rust: the `moov` box is read into memory, every chunk offset in it (`stco` /
//! `co64` of each track) that points into the moved range grows by the box's size, the
//! range between the first `mdat` and the `moov` slides forward by that size (copied from
//! the end backwards, so the file never grows), and the patched `moov` goes into the gap.
//! The length of the file does not change. Files that are already fast-started,
//! fragmented (`moof`), or whose 32-bit offsets would overflow are left as they are.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// A `moov` bigger than this is left where it is (it is read into memory).
const MAX_MOOV: u64 = 256 << 20;
/// Copy buffer for the shift.
const CHUNK: usize = 1 << 20;

/// A top-level box: type, offset, total size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TopBox {
    kind: [u8; 4],
    offset: u64,
    size: u64,
}

/// The top-level boxes of `f` (`len` bytes); `None` when the structure does not add up.
fn top_boxes(f: &mut File, len: u64) -> std::io::Result<Option<Vec<TopBox>>> {
    let mut out = Vec::new();
    let mut pos = 0u64;
    while pos < len {
        if len - pos < 8 {
            return Ok(None);
        }
        let mut h = [0u8; 16];
        f.seek(SeekFrom::Start(pos))?;
        let n = (len - pos).min(16) as usize;
        f.read_exact(&mut h[..n])?;
        let kind: [u8; 4] = h[4..8].try_into().expect("4 bytes");
        let size = match u32::from_be_bytes(h[..4].try_into().expect("4 bytes")) {
            0 => len - pos,
            1 if n == 16 => u64::from_be_bytes(h[8..16].try_into().expect("8 bytes")),
            1 => return Ok(None),
            s => u64::from(s),
        };
        if size < 8 || size > len - pos {
            return Ok(None);
        }
        out.push(TopBox { kind, offset: pos, size });
        pos += size;
    }
    Ok(Some(out))
}

/// `(header length, total size)` of the box at `b[0..]`, within `b.len()` bytes.
fn box_at(b: &[u8]) -> Option<(usize, usize)> {
    if b.len() < 8 {
        return None;
    }
    let (hdr, size) = match u32::from_be_bytes(b[..4].try_into().ok()?) {
        0 => (8, b.len() as u64),
        1 => (16, u64::from_be_bytes(b.get(8..16)?.try_into().ok()?)),
        s => (8, u64::from(s)),
    };
    let size = usize::try_from(size).ok()?;
    (size >= hdr && size <= b.len()).then_some((hdr, size))
}

/// Why a `moov` cannot be patched.
#[derive(Debug, PartialEq, Eq)]
enum Unpatchable {
    Malformed,
    /// A 32-bit `stco` offset would pass 4 GiB.
    Overflow,
}

/// Add `delta` to every chunk offset in `[from, to)` inside the boxes of `b` (the body
/// of a container box), descending into the boxes on the path to the sample tables.
fn patch(b: &mut [u8], from: u64, to: u64, delta: u64) -> Result<(), Unpatchable> {
    let mut i = 0usize;
    while i < b.len() {
        let (hdr, size) = box_at(&b[i..]).ok_or(Unpatchable::Malformed)?;
        let kind: [u8; 4] = b[i + 4..i + 8].try_into().expect("4 bytes");
        let body = &mut b[i + hdr..i + size];
        match &kind {
            b"trak" | b"mdia" | b"minf" | b"stbl" => patch(body, from, to, delta)?,
            b"stco" | b"co64" => {
                let wide = &kind == b"co64";
                let count = body.get(4..8).ok_or(Unpatchable::Malformed)?;
                let count = u32::from_be_bytes(count.try_into().expect("4 bytes")) as usize;
                let width = if wide { 8 } else { 4 };
                let need = count.checked_mul(width).and_then(|n| n.checked_add(8)).ok_or(Unpatchable::Malformed)?;
                if body.len() < need {
                    return Err(Unpatchable::Malformed);
                }
                for e in body[8..need].chunks_exact_mut(width) {
                    let off = if wide {
                        u64::from_be_bytes(e.try_into().expect("8 bytes"))
                    } else {
                        u64::from(u32::from_be_bytes(e.try_into().expect("4 bytes")))
                    };
                    if off < from || off >= to {
                        continue;
                    }
                    let new = off + delta;
                    if wide {
                        e.copy_from_slice(&new.to_be_bytes());
                    } else {
                        let new = u32::try_from(new).map_err(|_| Unpatchable::Overflow)?;
                        e.copy_from_slice(&new.to_be_bytes());
                    }
                }
            }
            _ => {}
        }
        i += size;
    }
    Ok(())
}

/// Move the `moov` of the MP4 / MOV file at `path` in front of its first `mdat`.
/// `Ok(true)`: moved; `Ok(false)`: nothing to do or not possible (the file is untouched);
/// `Err`: an I/O error (the file may be damaged: the caller discards it).
pub(crate) fn relocate(path: &Path) -> std::io::Result<bool> {
    let mut f = File::options().read(true).write(true).open(path)?;
    let len = f.metadata()?.len();
    let Some(boxes) = top_boxes(&mut f, len)? else { return Ok(false) };
    if boxes.iter().any(|b| &b.kind == b"moof") {
        return Ok(false);
    }
    let (Some(mdat), Some(moov)) =
        (boxes.iter().find(|b| &b.kind == b"mdat"), boxes.iter().find(|b| &b.kind == b"moov"))
    else {
        return Ok(false);
    };
    if moov.offset < mdat.offset || moov.size > MAX_MOOV {
        return Ok(false);
    }
    let ins = mdat.offset;
    let mut buf = vec![0u8; moov.size as usize];
    f.seek(SeekFrom::Start(moov.offset))?;
    f.read_exact(&mut buf)?;
    let Some((hdr, _)) = box_at(&buf) else { return Ok(false) };
    if patch(&mut buf[hdr..], ins, moov.offset, moov.size).is_err() {
        return Ok(false);
    }
    // Slide [ins, moov.offset) forward by the moov's size, from the end backwards.
    let mut chunk = vec![0u8; CHUNK];
    let mut end = moov.offset;
    while end > ins {
        let n = (end - ins).min(CHUNK as u64);
        let start = end - n;
        let part = &mut chunk[..n as usize];
        f.seek(SeekFrom::Start(start))?;
        f.read_exact(part)?;
        f.seek(SeekFrom::Start(start + moov.size))?;
        f.write_all(part)?;
        end = start;
    }
    f.seek(SeekFrom::Start(ins))?;
    f.write_all(&buf)?;
    f.flush()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    fn stco(offsets: &[u32]) -> Vec<u8> {
        let mut b = vec![0, 0, 0, 0];
        b.extend_from_slice(&(offsets.len() as u32).to_be_bytes());
        for o in offsets {
            b.extend_from_slice(&o.to_be_bytes());
        }
        bx(b"stco", &b)
    }

    fn co64(offsets: &[u64]) -> Vec<u8> {
        let mut b = vec![0, 0, 0, 0];
        b.extend_from_slice(&(offsets.len() as u32).to_be_bytes());
        for o in offsets {
            b.extend_from_slice(&o.to_be_bytes());
        }
        bx(b"co64", &b)
    }

    fn trak(table: Vec<u8>) -> Vec<u8> {
        bx(b"trak", &bx(b"mdia", &bx(b"minf", &bx(b"stbl", &table))))
    }

    /// `ftyp`, `free`, `mdat` with two 4-byte samples per track, then `moov` (two tracks,
    /// one `stco`, one `co64`) and a trailing `udta`-like box the move must not touch.
    fn sample_file() -> (Vec<u8>, [u8; 8]) {
        let ftyp = bx(b"ftyp", b"isom\0\0\0\0");
        let free = bx(b"free", b"");
        let payload = *b"AAAABBBB";
        let mdat_at = (ftyp.len() + free.len()) as u32;
        let first = mdat_at + 8;
        let mdat = bx(b"mdat", &payload);
        let moov =
            bx(b"moov", &[bx(b"mvhd", &[0; 8]), trak(stco(&[first])), trak(co64(&[u64::from(first) + 4]))].concat());
        let tail = bx(b"tail", b"xyz");
        ([ftyp, free, mdat, moov, tail].concat(), payload)
    }

    /// The chunk offsets of every track, read back.
    fn offsets(b: &[u8]) -> Vec<u64> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 8 <= b.len() {
            let kind = &b[i + 4..i + 8];
            if kind == b"stco" || kind == b"co64" {
                let w = if kind == b"co64" { 8 } else { 4 };
                let n = u32::from_be_bytes(b[i + 12..i + 16].try_into().unwrap()) as usize;
                for k in 0..n {
                    let e = &b[i + 16 + k * w..i + 16 + (k + 1) * w];
                    out.push(if w == 8 {
                        u64::from_be_bytes(e.try_into().unwrap())
                    } else {
                        u64::from(u32::from_be_bytes(e.try_into().unwrap()))
                    });
                }
            }
            i += 1;
        }
        out
    }

    #[test]
    fn moves_the_index_and_its_offsets_follow() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.mp4");
        let (bytes, payload) = sample_file();
        std::fs::write(&p, &bytes).unwrap();
        assert!(relocate(&p).unwrap());
        let out = std::fs::read(&p).unwrap();
        assert_eq!(out.len(), bytes.len(), "same length");
        let mut f = File::open(&p).unwrap();
        let kinds: Vec<[u8; 4]> =
            top_boxes(&mut f, out.len() as u64).unwrap().unwrap().iter().map(|b| b.kind).collect();
        assert_eq!(kinds, [*b"ftyp", *b"free", *b"moov", *b"mdat", *b"tail"]);
        // Each offset still points at its sample.
        let offs = offsets(&out);
        assert_eq!(offs.len(), 2);
        assert_eq!(&out[offs[0] as usize..offs[0] as usize + 4], &payload[..4]);
        assert_eq!(&out[offs[1] as usize..offs[1] as usize + 4], &payload[4..]);
        assert!(out.ends_with(&bx(b"tail", b"xyz")));
        // A second run has nothing to do.
        assert!(!relocate(&p).unwrap());
        assert_eq!(std::fs::read(&p).unwrap(), out);
    }

    #[test]
    fn leaves_odd_files_alone() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.mp4");
        // Fragmented.
        let frag = [bx(b"ftyp", b"iso5"), bx(b"moov", &[]), bx(b"moof", &[]), bx(b"mdat", b"x")].concat();
        std::fs::write(&p, &frag).unwrap();
        assert!(!relocate(&p).unwrap());
        // Truncated box sizes.
        let mut bad = sample_file().0;
        bad.truncate(bad.len() - 2);
        std::fs::write(&p, &bad).unwrap();
        assert!(!relocate(&p).unwrap());
        assert_eq!(std::fs::read(&p).unwrap(), bad);
        // No mdat.
        std::fs::write(&p, bx(b"moov", &[])).unwrap();
        assert!(!relocate(&p).unwrap());
    }

    #[test]
    fn a_32_bit_offset_that_would_overflow_is_not_patched() {
        let mut moov_body = trak(stco(&[u32::MAX - 2]));
        assert_eq!(patch(&mut moov_body, 0, u64::MAX, 16), Err(Unpatchable::Overflow));
        let mut ok_body = trak(co64(&[u64::from(u32::MAX) - 2]));
        assert_eq!(patch(&mut ok_body, 0, u64::MAX, 16), Ok(()));
        assert_eq!(offsets(&ok_body), [u64::from(u32::MAX) + 14]);
        // Outside the moved range: unchanged.
        let mut body = trak(stco(&[10, 100]));
        patch(&mut body, 50, 200, 7).unwrap();
        assert_eq!(offsets(&body), [10, 107]);
    }
}
