//! Small helpers shared by readers, writers and the extraction engine.

use std::io::{self, Read};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use soyuz_core::location::normalize_inner;
use soyuz_core::{ByteProgress, FsError, FsErrorKind, OpControl, ProgressSink};

use crate::ArchiveError;

/// Copy buffer size for streaming entry data (never buffers a whole entry).
pub(crate) const CHUNK: usize = 256 * 1024;

/// Per-entry data that the readers keep next to the public [`crate::ArchiveEntry`].
#[derive(Clone, Debug, Default)]
pub(crate) struct EntryExtra {
    /// Raw name as stored in the archive when it differs from the normalized path
    /// (used for zip-slip detection at extraction time).
    pub raw: Option<Box<str>>,
    /// tar hard link (`link_target` is then an archive path, not a symlink target).
    pub hardlink: bool,
    /// GNU sparse tar entry (data is not contiguous at `data_pos`).
    pub sparse: bool,
    /// Byte offset of the entry data in the (decompressed) tar stream.
    pub data_pos: u64,
    /// tar ordinal / zip index / 7z file index of this entry in the container.
    pub ordinal: usize,
}

/// Normalize a raw entry name; returns the normalized path and the raw name when it differs.
pub(crate) fn normalize_name(raw: &str) -> (String, Option<Box<str>>) {
    let norm = normalize_inner(raw);
    let trimmed = raw.trim_end_matches(['/', '\\']);
    let raw_keep = if trimmed == norm { None } else { Some(raw.into()) };
    (norm, raw_keep)
}

/// `true` when `raw` is absolute (`/x`, `\x`, `C:\x`, `C:x`, `\\server`) or escapes its
/// root through `..` segments.
pub(crate) fn path_escapes(raw: &str) -> bool {
    if raw.starts_with('/') || raw.starts_with('\\') {
        return true;
    }
    let b = raw.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return true;
    }
    let mut depth: i64 = 0;
    for seg in raw.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            _ => depth += 1,
        }
    }
    false
}

/// Resolve a symlink `target` relative to the directory containing `link_rel` (both
/// `/`-separated, relative to the extraction root). `None` when it is absolute or escapes.
#[cfg(test)]
pub(crate) fn resolve_link_target(link_rel: &str, target: &str) -> Option<String> {
    resolve_link_target_via(link_rel, target, &mut |_| false)
}

/// [`resolve_link_target`], but also `None` when resolving would *traverse* a path for
/// which `is_link` is true (another symlink in the archive or on disk). A lexical check
/// alone is beaten by chains: `d/l1 -> ..` is fine on its own, but `l2 -> d/l1/..`
/// lexically stays at `d` while on disk it resolves to the parent of the destination.
/// Ending *at* a link is allowed (it is checked on its own); passing through one is not.
pub(crate) fn resolve_link_target_via(
    link_rel: &str,
    target: &str,
    is_link: &mut dyn FnMut(&str) -> bool,
) -> Option<String> {
    if target.is_empty() || path_escapes_absolute(target) {
        return None;
    }
    let mut parts: Vec<&str> = link_rel.split('/').filter(|s| !s.is_empty()).collect();
    parts.pop(); // the link itself
    for seg in target.split(['/', '\\']) {
        if matches!(seg, "" | ".") {
            continue;
        }
        if !parts.is_empty() && is_link(&parts.join("/")) {
            return None;
        }
        match seg {
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

fn path_escapes_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    p.starts_with('/') || p.starts_with('\\') || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

pub(crate) fn unix_secs_to_time(secs: i64) -> Option<SystemTime> {
    if secs >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(secs as u64))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs(secs.unsigned_abs()))
    }
}

pub(crate) fn time_to_unix_secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// Error used to abort a library-driven copy when the operation is cancelled.
pub(crate) fn cancelled_io() -> io::Error {
    // Not `Interrupted`: `io::copy` and friends retry on that kind forever.
    io::Error::other("soyuz: cancelled")
}

/// Map an `io::Error` raised while *reading archive data*.
pub(crate) fn read_err(e: io::Error, ctl: Option<&OpControl>, encrypted: bool) -> ArchiveError {
    if ctl.is_some_and(|c| c.is_cancelled()) {
        return ArchiveError::Cancelled;
    }
    if let Some(inner) = e.get_ref().and_then(|x| x.downcast_ref::<ArchiveError>()) {
        return inner.clone();
    }
    match e.kind() {
        io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput | io::ErrorKind::UnexpectedEof => {
            let msg = e.to_string();
            if encrypted && (msg.contains("checksum") || msg.contains("password") || msg.contains("CRC")) {
                ArchiveError::WrongPassword
            } else {
                ArchiveError::Corrupt(msg)
            }
        }
        _ => ArchiveError::from(e),
    }
}

/// Map an `io::Error` raised while writing/reading the filesystem at `path`.
pub(crate) fn fs_err(op: &'static str, path: &Path, e: &io::Error) -> ArchiveError {
    ArchiveError::Io(FsError::from_io(op, Some(path), e))
}

pub(crate) fn fs_kind(kind: FsErrorKind, op: &'static str, path: &Path, msg: impl Into<String>) -> ArchiveError {
    ArchiveError::Io(FsError::with_message(kind, op, Some(path), msg))
}

/// Reader adapter used by the writers: checks for cancel/pause and reports progress.
pub(crate) struct ProgressReader<'a, R> {
    pub inner: R,
    pub ctl: &'a OpControl,
    pub sink: &'a mut dyn ProgressSink,
    pub done: u64,
    pub total: u64,
}

impl<'a, R> ProgressReader<'a, R> {
    pub fn new(inner: R, total: u64, ctl: &'a OpControl, sink: &'a mut dyn ProgressSink) -> Self {
        ProgressReader { inner, ctl, sink, done: 0, total }
    }
}

impl<R: Read> Read for ProgressReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.ctl.checkpoint().is_err() {
            return Err(cancelled_io());
        }
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.done += n as u64;
            self.sink.on_progress(ByteProgress { done: self.done, total: self.total, delta: n as u64 });
        }
        Ok(n)
    }
}

/// Map a DOS-style local date/time to `SystemTime` (zip local headers store local time).
pub(crate) fn local_to_system(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Option<SystemTime> {
    use chrono::TimeZone;
    let ndt = chrono::NaiveDate::from_ymd_opt(y, mo, d)?.and_hms_opt(h, mi, s.min(59))?;
    let local = chrono::Local.from_local_datetime(&ndt).earliest()?;
    Some(SystemTime::from(local))
}

/// `SystemTime` → local `(year, month, day, hour, minute, second)`.
pub(crate) fn system_to_local(t: SystemTime) -> (i32, u32, u32, u32, u32, u32) {
    use chrono::{Datelike, Timelike};
    let dt: chrono::DateTime<chrono::Local> = t.into();
    (dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute(), dt.second())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes() {
        assert!(path_escapes("../x"));
        assert!(path_escapes("a/../../x"));
        assert!(path_escapes("/etc/passwd"));
        assert!(path_escapes("\\windows\\x"));
        assert!(path_escapes("C:\\x"));
        assert!(path_escapes("c:x"));
        assert!(path_escapes("a\\..\\..\\x"));
        assert!(!path_escapes("a/../b"));
        assert!(!path_escapes("./a/b/"));
        assert!(!path_escapes("a..b/c"));
    }

    #[test]
    fn link_targets() {
        assert_eq!(resolve_link_target("a/b/l", "../c").as_deref(), Some("a/c"));
        assert_eq!(resolve_link_target("a/l", "../c").as_deref(), Some("c"));
        assert_eq!(resolve_link_target("l", "../c"), None);
        assert_eq!(resolve_link_target("a/l", "/etc/passwd"), None);
        assert_eq!(resolve_link_target("a/l", "x/./y").as_deref(), Some("a/x/y"));
    }

    #[test]
    fn link_chains_are_rejected() {
        let links = ["d/l1", "up"];
        let mut is_link = |p: &str| links.contains(&p);
        // Lexically `d`, but `d/l1` is a link: traversing it is refused.
        assert_eq!(resolve_link_target_via("l2", "d/l1/..", &mut is_link), None);
        assert_eq!(resolve_link_target_via("x/l3", "../up/../../etc", &mut is_link), None);
        assert_eq!(resolve_link_target_via("x/l4", "../up/file", &mut is_link), None);
        // Pointing *at* a link, or around one, is fine.
        assert_eq!(resolve_link_target_via("l5", "d/l1", &mut is_link).as_deref(), Some("d/l1"));
        assert_eq!(resolve_link_target_via("d/l6", "../d/real", &mut is_link).as_deref(), Some("d/real"));
    }

    #[test]
    fn names() {
        assert_eq!(normalize_name("a/b/"), ("a/b".to_string(), None));
        let (n, raw) = normalize_name("../a");
        assert_eq!(n, "a");
        assert_eq!(raw.as_deref(), Some("../a"));
    }

    #[test]
    fn local_time_round_trip() {
        let t = local_to_system(2024, 5, 6, 7, 8, 10).unwrap();
        assert_eq!(system_to_local(t), (2024, 5, 6, 7, 8, 10));
        assert_eq!(unix_secs_to_time(time_to_unix_secs(t)), Some(t));
    }
}
