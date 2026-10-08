//! Minimal EXIF reader for JPEG and TIFF-based files (TIFF, DNG and most camera raw
//! formats): capture date, camera, exposure and pixel size. Reads only the first
//! [`HEAD_BYTES`] of the file and never decodes image data. Used by the batch rename
//! `{exif:…}` token and the Properties "Details" tab.

use std::io::Read;
use std::path::Path;

use chrono::NaiveDateTime;

/// How much of a file is inspected.
pub const HEAD_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exif {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    pub software: Option<String>,
    /// `DateTimeOriginal`, else `DateTime`.
    pub taken: Option<NaiveDateTime>,
    pub orientation: Option<u16>,
    pub exposure: Option<(u32, u32)>,
    pub f_number: Option<(u32, u32)>,
    pub iso: Option<u32>,
    pub focal_mm: Option<(u32, u32)>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl Exif {
    /// Human-readable `(label, value)` rows.
    pub fn fields(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut push = |k: &str, v: Option<String>| {
            if let Some(v) = v.filter(|v| !v.trim().is_empty()) {
                out.push((k.to_string(), v));
            }
        };
        let camera = match (&self.make, &self.model) {
            (Some(m), Some(md)) if md.starts_with(m.as_str()) => Some(md.clone()),
            (Some(m), Some(md)) => Some(format!("{m} {md}")),
            (a, b) => a.clone().or_else(|| b.clone()),
        };
        let dec = soyuz_core::locale::format_decimal;
        push(tr!("dialogs-exif-camera"), camera);
        push(tr!("dialogs-exif-lens"), self.lens.clone());
        push(tr!("dialogs-exif-taken"), self.taken.map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string()));
        push(
            tr!("dialogs-exif-exposure"),
            self.exposure.filter(|e| e.1 != 0).map(|(n, d)| {
                if n >= d {
                    format!("{} s", dec(n as f64 / d as f64, 1))
                } else {
                    format!("1/{} s", (d as f64 / n.max(1) as f64).round())
                }
            }),
        );
        push(
            tr!("dialogs-exif-aperture"),
            self.f_number.filter(|f| f.1 != 0).map(|(n, d)| format!("f/{}", dec(n as f64 / d as f64, 1))),
        );
        push("ISO", self.iso.map(|i| i.to_string()));
        push(
            tr!("dialogs-exif-focal"),
            self.focal_mm.filter(|f| f.1 != 0).map(|(n, d)| format!("{:.0} mm", n as f64 / d as f64)),
        );
        push(tr!("dialogs-exif-orientation"), self.orientation.filter(|o| *o > 1).map(orientation_label));
        push(tr!("dialogs-exif-software"), self.software.clone());
        out
    }
}

fn orientation_label(o: u16) -> String {
    match o {
        2 => tr!("dialogs-exif-orient-2").into(),
        3 => tr!("dialogs-exif-orient-3").into(),
        4 => tr!("dialogs-exif-orient-4").into(),
        5 => tr!("dialogs-exif-orient-5").into(),
        6 => tr!("dialogs-exif-orient-6").into(),
        7 => tr!("dialogs-exif-orient-7").into(),
        8 => tr!("dialogs-exif-orient-8").into(),
        o => format!("{o}"),
    }
}

/// Read the EXIF block of `path` (`None` when there is none or it is unreadable).
pub fn read(path: &Path) -> Option<Exif> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut head = Vec::with_capacity(HEAD_BYTES);
    f.by_ref().take(HEAD_BYTES as u64).read_to_end(&mut head).ok()?;
    parse(&head)
}

/// Parse EXIF from the start of a JPEG or TIFF file.
pub fn parse(data: &[u8]) -> Option<Exif> {
    if data.starts_with(b"II*\0") || data.starts_with(b"MM\0*") {
        return parse_tiff(data);
    }
    if !data.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut i = 2;
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            return None;
        }
        let marker = data[i + 1];
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            return None; // start of scan: no EXIF before the image data
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        let body = data.get(i + 4..i + 2 + len)?;
        if marker == 0xE1 && body.starts_with(b"Exif\0\0") {
            return parse_tiff(&body[6..]);
        }
        i += 2 + len;
    }
    None
}

struct Tiff<'a> {
    d: &'a [u8],
    le: bool,
}

impl Tiff<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        let b: [u8; 2] = self.d.get(at..at + 2)?.try_into().ok()?;
        Some(if self.le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let b: [u8; 4] = self.d.get(at..at + 4)?.try_into().ok()?;
        Some(if self.le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }
}

#[derive(Clone, Copy)]
struct Field {
    tag: u16,
    typ: u16,
    count: u32,
    /// Offset of the 4-byte value/offset slot inside the entry.
    slot: usize,
}

const TYPE_SIZES: [usize; 13] = [0, 1, 1, 2, 4, 8, 1, 1, 2, 4, 8, 4, 8];

impl Tiff<'_> {
    fn entries(&self, ifd: usize) -> Vec<Field> {
        let Some(n) = self.u16(ifd) else { return Vec::new() };
        (0..n.min(512) as usize)
            .filter_map(|k| {
                let e = ifd + 2 + k * 12;
                Some(Field { tag: self.u16(e)?, typ: self.u16(e + 2)?, count: self.u32(e + 4)?, slot: e + 8 })
            })
            .collect()
    }

    fn data_at(&self, f: &Field) -> Option<usize> {
        let size = TYPE_SIZES.get(f.typ as usize).copied().unwrap_or(0).checked_mul(f.count as usize)?;
        if size <= 4 { Some(f.slot) } else { self.u32(f.slot).map(|o| o as usize) }
    }

    fn ascii(&self, f: &Field) -> Option<String> {
        if f.typ != 2 {
            return None;
        }
        let at = self.data_at(f)?;
        let raw = self.d.get(at..at + f.count as usize)?;
        let s = raw.split(|b| *b == 0).next().unwrap_or(raw);
        let s = String::from_utf8_lossy(s).trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    fn uint(&self, f: &Field) -> Option<u32> {
        let at = self.data_at(f)?;
        match f.typ {
            3 => self.u16(at).map(u32::from),
            4 | 9 => self.u32(at),
            _ => None,
        }
    }

    fn rational(&self, f: &Field) -> Option<(u32, u32)> {
        if f.typ != 5 && f.typ != 10 {
            return None;
        }
        let at = self.data_at(f)?;
        Some((self.u32(at)?, self.u32(at + 4)?))
    }
}

fn parse_date(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s.trim(), "%Y:%m:%d %H:%M:%S").ok()
}

fn parse_tiff(d: &[u8]) -> Option<Exif> {
    let le = match d.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let t = Tiff { d, le };
    if t.u16(2)? != 42 {
        return None;
    }
    let ifd0 = t.u32(4)? as usize;
    let mut x = Exif::default();
    let mut date_time = None;
    let mut sub = None;
    for f in t.entries(ifd0) {
        match f.tag {
            0x010F => x.make = t.ascii(&f),
            0x0110 => x.model = t.ascii(&f),
            0x0112 => x.orientation = t.uint(&f).map(|v| v as u16),
            0x0131 => x.software = t.ascii(&f),
            0x0132 => date_time = t.ascii(&f).as_deref().and_then(parse_date),
            0x0100 => x.width = t.uint(&f),
            0x0101 => x.height = t.uint(&f),
            0x8769 => sub = t.uint(&f).map(|o| o as usize),
            _ => {}
        }
    }
    if let Some(sub) = sub.filter(|o| *o < d.len()) {
        for f in t.entries(sub) {
            match f.tag {
                0x9003 => x.taken = t.ascii(&f).as_deref().and_then(parse_date),
                0x829A => x.exposure = t.rational(&f),
                0x829D => x.f_number = t.rational(&f),
                0x8827 => x.iso = t.uint(&f),
                0x920A => x.focal_mm = t.rational(&f),
                0xA002 => x.width = t.uint(&f).or(x.width),
                0xA003 => x.height = t.uint(&f).or(x.height),
                0xA434 => x.lens = t.ascii(&f),
                _ => {}
            }
        }
    }
    x.taken = x.taken.or(date_time);
    Some(x)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build a little-endian TIFF/EXIF block: IFD0 (Make, Model, ExifIFD pointer) and an
    /// EXIF IFD (DateTimeOriginal, ISO, FNumber).
    pub(crate) fn sample_tiff() -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(b"II");
        d.extend_from_slice(&42u16.to_le_bytes());
        d.extend_from_slice(&8u32.to_le_bytes());
        // IFD0 at 8: 3 entries → 2 + 36 + 4 = 42 bytes, data area at 50.
        let make = b"Canon\0";
        let model = b"Canon EOS R5\0";
        let ifd0_data = 8 + 2 + 3 * 12 + 4;
        let make_off = ifd0_data;
        let model_off = make_off + make.len();
        let exif_ifd = model_off + model.len();
        let entry = |d: &mut Vec<u8>, tag: u16, typ: u16, count: u32, val: u32| {
            d.extend_from_slice(&tag.to_le_bytes());
            d.extend_from_slice(&typ.to_le_bytes());
            d.extend_from_slice(&count.to_le_bytes());
            d.extend_from_slice(&val.to_le_bytes());
        };
        d.extend_from_slice(&3u16.to_le_bytes());
        entry(&mut d, 0x010F, 2, make.len() as u32, make_off as u32);
        entry(&mut d, 0x0110, 2, model.len() as u32, model_off as u32);
        entry(&mut d, 0x8769, 4, 1, exif_ifd as u32);
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(make);
        d.extend_from_slice(model);
        // EXIF IFD: 3 entries.
        let date = b"2023:07:14 18:30:05\0";
        let data = exif_ifd + 2 + 3 * 12 + 4;
        let date_off = data;
        let fnum_off = date_off + date.len();
        d.extend_from_slice(&3u16.to_le_bytes());
        entry(&mut d, 0x9003, 2, date.len() as u32, date_off as u32);
        entry(&mut d, 0x8827, 3, 1, 400);
        entry(&mut d, 0x829D, 5, 1, fnum_off as u32);
        d.extend_from_slice(&0u32.to_le_bytes());
        d.extend_from_slice(date);
        d.extend_from_slice(&28u32.to_le_bytes());
        d.extend_from_slice(&10u32.to_le_bytes());
        d
    }

    pub(crate) fn sample_jpeg() -> Vec<u8> {
        let tiff = sample_tiff();
        let mut j = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00]; // tiny APP0
        j.extend_from_slice(&[0xFF, 0xE1]);
        j.extend_from_slice(&((tiff.len() + 8) as u16).to_be_bytes());
        j.extend_from_slice(b"Exif\0\0");
        j.extend_from_slice(&tiff);
        j.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]);
        j
    }

    #[test]
    fn parses_jpeg_app1() {
        let x = parse(&sample_jpeg()).expect("exif");
        assert_eq!(x.make.as_deref(), Some("Canon"));
        assert_eq!(x.model.as_deref(), Some("Canon EOS R5"));
        assert_eq!(x.taken.unwrap().to_string(), "2023-07-14 18:30:05");
        assert_eq!(x.iso, Some(400));
        assert_eq!(x.f_number, Some((28, 10)));
        let f = x.fields();
        assert!(f.contains(&("Camera".into(), "Canon EOS R5".into())), "{f:?}");
        assert!(f.contains(&("Aperture".into(), "f/2.8".into())));
    }

    #[test]
    fn parses_raw_tiff_and_rejects_garbage() {
        assert!(parse(&sample_tiff()).is_some());
        assert!(parse(b"not an image").is_none());
        assert!(parse(&[0xFF, 0xD8, 0xFF, 0xDA, 0, 2]).is_none());
        let mut t = sample_tiff();
        t.truncate(20); // truncated IFD: no panic
        let _ = parse(&t);
    }
}
