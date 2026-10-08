//! Size, speed and ETA formatting.

use serde::{Deserialize, Serialize};

/// How byte sizes are displayed.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SizeStyle {
    /// 1024-based: KiB, MiB, ...
    #[default]
    Binary,
    /// 1000-based: kB, MB, ...
    Decimal,
    /// Exact byte count with thousands separators.
    Bytes,
}

/// `format_size(1536, Binary)` → `"1.5 KiB"`; values < 100 get one decimal.
pub fn format_size(bytes: u64, style: SizeStyle) -> String {
    let (base, units): (f64, [&str; 7]) = match style {
        SizeStyle::Bytes => return format!("{} B", group_thousands(bytes)),
        SizeStyle::Binary => (1024.0, ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"]),
        SizeStyle::Decimal => (1000.0, ["B", "kB", "MB", "GB", "TB", "PB", "EB"]),
    };
    if (bytes as f64) < base {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= base && unit < units.len() - 1 {
        v /= base;
        unit += 1;
    }
    // Avoid "1024.0 KiB" after rounding: bump to the next unit.
    if v >= base - 0.05 && unit < units.len() - 1 {
        v /= base;
        unit += 1;
    }
    if v < 100.0 {
        let s = format!("{v:.1}");
        let s = s.strip_suffix(".0").unwrap_or(&s);
        format!("{s} {}", units[unit])
    } else {
        format!("{v:.0} {}", units[unit])
    }
}

/// `"12.3 MiB/s"`.
pub fn format_speed(bps: f64) -> String {
    if !bps.is_finite() || bps <= 0.0 {
        return "0 B/s".into();
    }
    format!("{}/s", format_size(bps as u64, SizeStyle::Binary))
}

/// `"45s"`, `"3m 05s"`, `"2h 07m"`; `"—"` for unknown (non-finite / negative).
pub fn format_eta(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "\u{2014}".into();
    }
    let s = secs.round() as u64;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else if s < 100 * 3600 {
        format!("{}h {:02}m", s / 3600, (s % 3600) / 60)
    } else {
        format!("{}d {:02}h", s / 86400, (s % 86400) / 3600)
    }
}

fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(format_size(0, SizeStyle::Binary), "0 B");
        assert_eq!(format_size(1023, SizeStyle::Binary), "1023 B");
        assert_eq!(format_size(1024, SizeStyle::Binary), "1 KiB");
        assert_eq!(format_size(1536, SizeStyle::Binary), "1.5 KiB");
        assert_eq!(format_size(150 * 1024 * 1024, SizeStyle::Binary), "150 MiB");
        assert_eq!(format_size(1024 * 1024 - 1, SizeStyle::Binary), "1 MiB");
        assert_eq!(format_size(1500, SizeStyle::Decimal), "1.5 kB");
        assert_eq!(format_size(1234567, SizeStyle::Bytes), "1,234,567 B");
        assert_eq!(format_size(999, SizeStyle::Bytes), "999 B");
        assert_eq!(format_size(u64::MAX, SizeStyle::Binary), "16 EiB");
    }

    #[test]
    fn speed_eta() {
        assert_eq!(format_speed(2.0 * 1024.0 * 1024.0), "2 MiB/s");
        assert_eq!(format_speed(f64::NAN), "0 B/s");
        assert_eq!(format_eta(5.2), "5s");
        assert_eq!(format_eta(185.0), "3m 05s");
        assert_eq!(format_eta(7620.0), "2h 07m");
        assert_eq!(format_eta(f64::INFINITY), "\u{2014}");
    }
}
