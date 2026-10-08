//! Hex editor model (pure logic): an overwrite-only patch overlay with undo / redo, the
//! data inspector (integers / floats in both byte orders, UTF-8 character) and parsing
//! of "find bytes" and "go to offset" input.

use std::collections::BTreeMap;

/// One undo step: `(offset, previous patch, new patch)` per byte.
type Step = Vec<(u64, Option<u8>, Option<u8>)>;

/// Overwrite patches on top of the file (`offset → new byte`).
#[derive(Debug, Default)]
pub struct Patches {
    map: BTreeMap<u64, u8>,
    undo: Vec<Step>,
    redo: Vec<Step>,
}

impl Patches {
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn get(&self, off: u64) -> Option<u8> {
        self.map.get(&off).copied()
    }

    pub fn map(&self) -> &BTreeMap<u64, u8> {
        &self.map
    }

    /// Overwrite the byte at `off` (whose on-disk value is `original`) with `value`; a
    /// value equal to the original drops the patch. One undo step.
    pub fn set(&mut self, off: u64, original: u8, value: u8) {
        let prev = self.map.get(&off).copied();
        let next = (value != original).then_some(value);
        if prev == next {
            return;
        }
        self.apply(off, next);
        self.redo.clear();
        self.undo.push(vec![(off, prev, next)]);
    }

    fn apply(&mut self, off: u64, v: Option<u8>) {
        match v {
            Some(b) => {
                self.map.insert(off, b);
            }
            None => {
                self.map.remove(&off);
            }
        }
    }

    /// Undo one step; returns the offset it touched.
    pub fn undo(&mut self) -> Option<u64> {
        let step = self.undo.pop()?;
        for &(off, prev, _) in step.iter().rev() {
            self.apply(off, prev);
        }
        let off = step.first().map(|s| s.0);
        self.redo.push(step);
        off
    }

    pub fn redo(&mut self) -> Option<u64> {
        let step = self.redo.pop()?;
        for &(off, _, next) in &step {
            self.apply(off, next);
        }
        let off = step.first().map(|s| s.0);
        self.undo.push(step);
        off
    }

    /// After a save (or reload): no patches, no history.
    pub fn clear(&mut self) {
        self.map.clear();
        self.undo.clear();
        self.redo.clear();
    }

    /// Apply the patches that fall into `buf` (which holds the file bytes at `base`).
    pub fn overlay(&self, base: u64, buf: &mut [u8]) {
        overlay(&self.map, base, buf);
    }
}

pub fn overlay(map: &BTreeMap<u64, u8>, base: u64, buf: &mut [u8]) {
    let end = base + buf.len() as u64;
    for (&off, &b) in map.range(base..end) {
        buf[(off - base) as usize] = b;
    }
}

/// Data inspector rows for the bytes at the cursor (`bytes` may be shorter than 8 at
/// the end of the file).
pub fn inspect(bytes: &[u8], big_endian: bool) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    macro_rules! int {
        ($label:expr, $t:ty, $n:expr) => {
            rows.push((
                $label,
                match bytes.get(..$n) {
                    Some(b) => {
                        let arr: [u8; $n] = b.try_into().unwrap_or([0; $n]);
                        let v = if big_endian { <$t>::from_be_bytes(arr) } else { <$t>::from_le_bytes(arr) };
                        v.to_string()
                    }
                    None => "—".to_string(),
                },
            ));
        };
    }
    int!("u8", u8, 1);
    int!("i8", i8, 1);
    int!("u16", u16, 2);
    int!("i16", i16, 2);
    int!("u32", u32, 4);
    int!("i32", i32, 4);
    int!("u64", u64, 8);
    int!("i64", i64, 8);
    let float = |n: usize| -> String {
        match bytes.get(..n) {
            Some(b) if n == 4 => {
                let arr: [u8; 4] = b.try_into().unwrap_or([0; 4]);
                let v = if big_endian { f32::from_be_bytes(arr) } else { f32::from_le_bytes(arr) };
                format!("{v:e}")
            }
            Some(b) => {
                let arr: [u8; 8] = b.try_into().unwrap_or([0; 8]);
                let v = if big_endian { f64::from_be_bytes(arr) } else { f64::from_le_bytes(arr) };
                format!("{v:e}")
            }
            None => "—".to_string(),
        }
    };
    rows.push(("f32", float(4)));
    rows.push(("f64", float(8)));
    // "binary" is translated where the inspector draws its labels.
    rows.push(("binary", bytes.first().map_or("—".to_string(), |b| format!("{b:08b}"))));
    rows.push(("UTF-8", utf8_char(bytes)));
    rows
}

/// The UTF-8 character starting at `bytes[0]` (`U+XXXX 'c'`), or why there is none.
pub fn utf8_char(bytes: &[u8]) -> String {
    let Some(&b0) = bytes.first() else { return "—".into() };
    let len = match b0 {
        0x00..=0x7F => 1,
        0xC2..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF4 => 4,
        _ => return tr!("editors-hex-invalid").into(),
    };
    match bytes.get(..len).and_then(|b| std::str::from_utf8(b).ok()).and_then(|s| s.chars().next()) {
        Some(c) if c.is_control() => format!("U+{:04X}", c as u32),
        Some(c) => format!("U+{:04X} '{c}'", c as u32),
        None => tr!("editors-hex-invalid").into(),
    }
}

/// Parse "find" input: hex bytes (`DE AD be ef`, `0xDEADBEEF`, `de,ad`) when `hex`,
/// otherwise the UTF-8 bytes of the text.
pub fn parse_needle(input: &str, hex: bool) -> Result<Vec<u8>, String> {
    if !hex {
        return if input.is_empty() { Err(tr!("editors-hex-need-text").into()) } else { Ok(input.as_bytes().to_vec()) };
    }
    let digits: String = input
        .split(|c: char| c.is_whitespace() || c == ',')
        .map(|t| t.trim_start_matches("0x").trim_start_matches("0X"))
        .collect();
    if digits.is_empty() {
        return Err(tr!("editors-hex-need-bytes").into());
    }
    if let Some(c) = digits.chars().find(|c| !c.is_ascii_hexdigit()) {
        return Err(tr!("editors-hex-bad-digit", char = c.to_string()));
    }
    if digits.len() % 2 == 1 {
        return Err(tr!("editors-hex-odd-digits").into());
    }
    Ok((0..digits.len()).step_by(2).map(|i| u8::from_str_radix(&digits[i..i + 2], 16).unwrap_or(0)).collect())
}

/// Parse "go to offset": `0x1F`, `1Fh`, `$1F` (hex) or decimal; `+n` / `-n` relative
/// to `current`.
pub fn parse_offset(input: &str, current: u64) -> Option<u64> {
    let s = input.trim().replace(['_', ' '], "");
    let (rel, s) = match s.chars().next()? {
        '+' => (1i8, &s[1..]),
        '-' => (-1i8, &s[1..]),
        _ => (0, s.as_str()),
    };
    let v = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).or_else(|| s.strip_prefix('$')) {
        u64::from_str_radix(h, 16).ok()?
    } else if let Some(h) = s.strip_suffix('h').or_else(|| s.strip_suffix('H')) {
        u64::from_str_radix(h, 16).ok()?
    } else {
        s.parse::<u64>().ok()?
    };
    Some(match rel {
        1 => current.saturating_add(v),
        -1 => current.saturating_sub(v),
        _ => v,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_overlay_undo_redo() {
        let mut p = Patches::default();
        p.set(3, 0x10, 0xAB);
        p.set(4, 0x20, 0x20); // equal to the original: no patch, no step
        p.set(5, 0x30, 0xCD);
        assert_eq!(p.len(), 2);
        let mut buf = [0u8; 4];
        p.overlay(2, &mut buf);
        assert_eq!(buf, [0, 0xAB, 0, 0xCD]);
        assert_eq!(p.undo(), Some(5));
        assert_eq!(p.get(5), None);
        assert_eq!(p.redo(), Some(5));
        assert_eq!(p.get(5), Some(0xCD));
        // Writing the original value back removes the patch (file is clean again).
        p.set(3, 0x10, 0x10);
        p.set(5, 0x30, 0x30);
        assert!(p.is_empty());
        p.undo();
        assert_eq!(p.get(5), Some(0xCD));
        p.clear();
        assert!(p.is_empty() && p.undo().is_none());
    }

    #[test]
    fn inspector_decodes_both_orders() {
        let b = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let le = inspect(&b, false);
        let get = |rows: &[(&str, String)], k: &str| rows.iter().find(|r| r.0 == k).unwrap().1.clone();
        assert_eq!(get(&le, "u8"), "1");
        assert_eq!(get(&le, "u16"), "513");
        assert_eq!(get(&le, "u32"), "67305985");
        assert_eq!(get(&le, "u64"), "578437695752307201");
        let be = inspect(&b, true);
        assert_eq!(get(&be, "u16"), "258");
        assert_eq!(get(&be, "u32"), "16909060");
        let neg = inspect(&[0xFF, 0xFF, 0xFF, 0xFF], false);
        assert_eq!(get(&neg, "i8"), "-1");
        assert_eq!(get(&neg, "i32"), "-1");
        assert_eq!(get(&neg, "u64"), "—", "not enough bytes");
        let one = inspect(&1.5f32.to_le_bytes(), false);
        assert_eq!(get(&one, "f32"), "1.5e0");
        let d = inspect(&2.0f64.to_be_bytes(), true);
        assert_eq!(get(&d, "f64"), "2e0");
        assert_eq!(get(&inspect(&[0b1010_0101], false), "binary"), "10100101");
    }

    #[test]
    fn utf8_chars() {
        assert_eq!(utf8_char("é".as_bytes()), "U+00E9 'é'");
        assert_eq!(utf8_char("€x".as_bytes()), "U+20AC '€'");
        assert_eq!(utf8_char(b"A"), "U+0041 'A'");
        assert_eq!(utf8_char(&[0x0A]), "U+000A");
        assert_eq!(utf8_char(&[0xE2, 0x82]), "invalid");
        assert_eq!(utf8_char(&[0xFF]), "invalid");
        assert_eq!(utf8_char(&[]), "—");
    }

    #[test]
    fn needles_and_offsets() {
        assert_eq!(parse_needle("DE AD be ef", true).unwrap(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(parse_needle("0xCAFE", true).unwrap(), vec![0xCA, 0xFE]);
        assert_eq!(parse_needle("01,02", true).unwrap(), vec![1, 2]);
        assert!(parse_needle("ABC", true).is_err());
        assert!(parse_needle("zz", true).is_err());
        assert_eq!(parse_needle("hé", false).unwrap(), "hé".as_bytes());
        assert!(parse_needle("", false).is_err());
        assert_eq!(parse_offset("0x10", 0), Some(16));
        assert_eq!(parse_offset("10h", 0), Some(16));
        assert_eq!(parse_offset("$ff", 0), Some(255));
        assert_eq!(parse_offset("1_000", 0), Some(1000));
        assert_eq!(parse_offset("+16", 100), Some(116));
        assert_eq!(parse_offset("-200", 100), Some(0));
        assert_eq!(parse_offset("nope", 0), None);
    }
}
