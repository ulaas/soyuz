//! Pure parsers of `/proc` and `/sys` text and other OS blobs: no I/O, no unsafe, compiled on
//! every OS so their tests run everywhere (like `sys/reparse.rs`). Malformed input yields
//! `None` or an empty result; nothing here panics on what a file contains.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

pub(crate) mod hw;
pub(crate) mod misc;
pub(crate) mod net;
pub(crate) mod pid;
pub(crate) mod system;

use super::ProcPriority;

/// The value after `key` in `key: value` / `key=value` style text, trimmed.
fn field<'a>(line: &'a str, key: &str, sep: char) -> Option<&'a str> {
    let rest = line.strip_prefix(key)?.trim_start();
    Some(rest.strip_prefix(sep)?.trim())
}

/// `1234 kB` (or a bare number, taken as kB as the kernel prints it) to bytes.
fn kb_value(v: &str) -> Option<u64> {
    v.split_ascii_whitespace().next()?.parse::<u64>().ok()?.checked_mul(1024)
}

/// Shared six-step scale: niceness 19, 10, 0, -5, -15, -20 and everything between.
pub(crate) fn nice_to_priority(nice: i32) -> ProcPriority {
    match nice {
        15.. => ProcPriority::Idle,
        5..=14 => ProcPriority::BelowNormal,
        -2..=4 => ProcPriority::Normal,
        -10..=-3 => ProcPriority::AboveNormal,
        -17..=-11 => ProcPriority::High,
        _ => ProcPriority::Realtime,
    }
}

/// The niceness a priority step sets.
pub(crate) fn priority_to_nice(p: ProcPriority) -> i32 {
    match p {
        ProcPriority::Idle => 19,
        ProcPriority::BelowNormal => 10,
        ProcPriority::Normal => 0,
        ProcPriority::AboveNormal => -5,
        ProcPriority::High => -15,
        ProcPriority::Realtime => -20,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_scale_round_trips() {
        for p in [
            ProcPriority::Idle,
            ProcPriority::BelowNormal,
            ProcPriority::Normal,
            ProcPriority::AboveNormal,
            ProcPriority::High,
            ProcPriority::Realtime,
        ] {
            assert_eq!(nice_to_priority(priority_to_nice(p)), p);
        }
        assert_eq!(nice_to_priority(1), ProcPriority::Normal);
        assert_eq!(nice_to_priority(i32::MAX), ProcPriority::Idle);
        assert_eq!(nice_to_priority(i32::MIN), ProcPriority::Realtime);
    }

    #[test]
    fn helpers_tolerate_junk() {
        assert_eq!(field("MemTotal:   12 kB", "MemTotal", ':'), Some("12 kB"));
        assert_eq!(field("MemTotalX: 12", "MemTotal", ':'), None);
        assert_eq!(kb_value("2 kB"), Some(2048));
        assert_eq!(kb_value("x kB"), None);
        assert_eq!(kb_value(""), None);
        assert_eq!(kb_value("99999999999999999999 kB"), None);
    }
}
