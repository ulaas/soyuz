//! Size, speed and ETA formatting (locale-aware, see [`crate::locale`]).

use serde::{Deserialize, Serialize};

use crate::locale::{self, Locale};

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

/// `format_size(1536, Binary)` → `"1.5 KiB"` (`"1,5 KiB"` in Turkish); values < 100 get
/// one decimal. Uses the current [`locale`](crate::locale).
pub fn format_size(bytes: u64, style: SizeStyle) -> String {
    format_size_in(bytes, style, locale::current())
}

/// [`format_size`] in an explicit locale.
pub fn format_size_in(bytes: u64, style: SizeStyle, l: Locale) -> String {
    let (base, units): (f64, [&str; 7]) = match style {
        SizeStyle::Bytes => return format!("{} B", locale::group_int_in(bytes, l)),
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
        let s = if l.decimal_sep() == '.' { s.to_string() } else { s.replace('.', &l.decimal_sep().to_string()) };
        format!("{s} {}", units[unit])
    } else {
        format!("{v:.0} {}", units[unit])
    }
}

/// `"12.3 MiB/s"` (`"12,3 MiB/sn"` in Turkish).
pub fn format_speed(bps: f64) -> String {
    format_speed_in(bps, locale::current())
}

/// [`format_speed`] in an explicit locale.
pub fn format_speed_in(bps: f64, l: Locale) -> String {
    let per = locale::words(l).per_sec;
    if !bps.is_finite() || bps <= 0.0 {
        return format!("0 B{per}");
    }
    format!("{}{per}", format_size_in(bps as u64, SizeStyle::Binary, l))
}

/// `"45s"`, `"3m 05s"`, `"2h 07m"` (`"3 dk 05 sn"` in Turkish); `"—"` for unknown
/// (non-finite / negative).
pub fn format_eta(secs: f64) -> String {
    format_eta_in(secs, locale::current())
}

/// [`format_eta`] in an explicit locale.
pub fn format_eta_in(secs: f64, l: Locale) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "\u{2014}".into();
    }
    let w = locale::words(l);
    let g = w.unit_gap;
    let s = secs.round() as u64;
    if s < 60 {
        format!("{s}{g}{}", w.sec)
    } else if s < 3600 {
        format!("{}{g}{} {:02}{g}{}", s / 60, w.min, s % 60, w.sec)
    } else if s < 100 * 3600 {
        format!("{}{g}{} {:02}{g}{}", s / 3600, w.hour, (s % 3600) / 60, w.min)
    } else {
        format!("{}{g}{} {:02}{g}{}", s / 86400, w.day, (s % 86400) / 3600, w.hour)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn en(bytes: u64, style: SizeStyle) -> String {
        format_size_in(bytes, style, Locale::EnUs)
    }

    #[test]
    fn sizes() {
        assert_eq!(en(0, SizeStyle::Binary), "0 B");
        assert_eq!(en(1023, SizeStyle::Binary), "1023 B");
        assert_eq!(en(1024, SizeStyle::Binary), "1 KiB");
        assert_eq!(en(1536, SizeStyle::Binary), "1.5 KiB");
        assert_eq!(en(150 * 1024 * 1024, SizeStyle::Binary), "150 MiB");
        assert_eq!(en(1024 * 1024 - 1, SizeStyle::Binary), "1 MiB");
        assert_eq!(en(1500, SizeStyle::Decimal), "1.5 kB");
        assert_eq!(en(1234567, SizeStyle::Bytes), "1,234,567 B");
        assert_eq!(en(999, SizeStyle::Bytes), "999 B");
        assert_eq!(en(u64::MAX, SizeStyle::Binary), "16 EiB");
    }

    #[test]
    fn speed_eta() {
        let l = Locale::EnUs;
        assert_eq!(format_speed_in(2.0 * 1024.0 * 1024.0, l), "2 MiB/s");
        assert_eq!(format_speed_in(f64::NAN, l), "0 B/s");
        assert_eq!(format_eta_in(5.2, l), "5s");
        assert_eq!(format_eta_in(185.0, l), "3m 05s");
        assert_eq!(format_eta_in(7620.0, l), "2h 07m");
        assert_eq!(format_eta_in(f64::INFINITY, l), "\u{2014}");
    }

    #[test]
    fn turkish_sizes_speed_eta() {
        let tr = Locale::TrTr;
        assert_eq!(format_size_in(1536, SizeStyle::Binary, tr), "1,5 KiB");
        assert_eq!(format_size_in(1_200_000, SizeStyle::Decimal, tr), "1,2 MB");
        assert_eq!(format_size_in(1_234_567, SizeStyle::Bytes, tr), "1.234.567 B");
        assert_eq!(format_size_in(150 * 1024 * 1024, SizeStyle::Binary, tr), "150 MiB");
        assert_eq!(format_speed_in(2.5 * 1024.0 * 1024.0, tr), "2,5 MiB/sn");
        assert_eq!(format_speed_in(0.0, tr), "0 B/sn");
        assert_eq!(format_eta_in(185.0, tr), "3 dk 05 sn");
        assert_eq!(format_eta_in(7620.0, tr), "2 sa 07 dk");
        assert_eq!(format_eta_in(5.0, tr), "5 sn");
    }
}
