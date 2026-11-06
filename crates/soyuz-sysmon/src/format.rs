//! Locale-free text helpers. Sizes go through `soyuz_core::format_size` in the callers;
//! translated labels are the app core's job.

const UNITS: [&str; 6] = ["B/s", "KiB/s", "MiB/s", "GiB/s", "TiB/s", "PiB/s"];

/// `1.5 MiB/s`; a rate below one kibibyte has no decimals (`512 B/s`).
pub fn format_rate(bytes_per_sec: f64) -> String {
    let mut value = if bytes_per_sec.is_finite() { bytes_per_sec.max(0.0) } else { 0.0 };
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 || value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// `59s`, `4:05`, `3:04:05`, `2d 03:04:05`.
pub fn format_duration(secs: u64) -> String {
    let (d, h, m, s) = (secs / 86_400, secs / 3_600 % 24, secs / 60 % 60, secs % 60);
    if d > 0 {
        format!("{d}d {h:02}:{m:02}:{s:02}")
    } else if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else if m > 0 {
        format!("{m}:{s:02}")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates() {
        assert_eq!(format_rate(0.0), "0 B/s");
        assert_eq!(format_rate(512.0), "512 B/s");
        assert_eq!(format_rate(1536.0), "1.5 KiB/s");
        assert_eq!(format_rate(150.0 * 1024.0 * 1024.0), "150 MiB/s");
        assert_eq!(format_rate(f64::NAN), "0 B/s");
        assert_eq!(format_rate(-5.0), "0 B/s");
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration(59), "59s");
        assert_eq!(format_duration(245), "4:05");
        assert_eq!(format_duration(3 * 3600 + 245), "3:04:05");
        assert_eq!(format_duration(2 * 86_400 + 3 * 3600 + 245), "2d 03:04:05");
    }
}
