//! NTFS change journal records (`USN_RECORD_V2` / `V3`), parsed from the raw buffers
//! `soyuz_fs::usn` returns, and the catch-up planner that turns a run of records into
//! index updates. Pure byte parsing: no OS calls, so it is tested with fixtures on any
//! platform.

use std::collections::HashMap;
use std::path::PathBuf;

/// `USN_REASON_*` bits we act on.
pub mod reason {
    pub const DATA_OVERWRITE: u32 = 0x0000_0001;
    pub const DATA_EXTEND: u32 = 0x0000_0002;
    pub const DATA_TRUNCATION: u32 = 0x0000_0004;
    pub const FILE_CREATE: u32 = 0x0000_0100;
    pub const FILE_DELETE: u32 = 0x0000_0200;
    pub const RENAME_OLD_NAME: u32 = 0x0000_1000;
    pub const RENAME_NEW_NAME: u32 = 0x0000_2000;
    pub const BASIC_INFO_CHANGE: u32 = 0x0000_8000;
    pub const CLOSE: u32 = 0x8000_0000;
}

/// `FILE_ATTRIBUTE_DIRECTORY`.
pub const ATTR_DIRECTORY: u32 = 0x10;

/// One change-journal (or MFT enumeration) record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsnRecord {
    /// File reference number (low 64 bits for V3's 128-bit ids).
    pub frn: u64,
    pub parent_frn: u64,
    pub usn: i64,
    /// FILETIME of the change.
    pub timestamp: i64,
    pub reason: u32,
    pub attributes: u32,
    pub name: String,
}

impl UsnRecord {
    pub fn is_dir(&self) -> bool {
        self.attributes & ATTR_DIRECTORY != 0
    }
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

/// Parse one record at the start of `b`; returns it (or `None` for an unsupported
/// version such as V4 range records) and its length.
pub fn parse_record(b: &[u8]) -> Result<(Option<UsnRecord>, usize), String> {
    let len = u32_at(b, 0).ok_or("truncated record header")? as usize;
    if len < 8 || len > b.len() {
        return Err(format!("bad record length {len} (buffer {})", b.len()));
    }
    let rec = &b[..len];
    let major = u16_at(rec, 4).ok_or("truncated")?;
    let parsed = match major {
        2 => {
            let name_len = u16_at(rec, 56).ok_or("truncated V2")? as usize;
            let name_off = u16_at(rec, 58).ok_or("truncated V2")? as usize;
            Some(UsnRecord {
                frn: u64_at(rec, 8).ok_or("truncated V2")?,
                parent_frn: u64_at(rec, 16).ok_or("truncated V2")?,
                usn: u64_at(rec, 24).ok_or("truncated V2")? as i64,
                timestamp: u64_at(rec, 32).ok_or("truncated V2")? as i64,
                reason: u32_at(rec, 40).ok_or("truncated V2")?,
                attributes: u32_at(rec, 52).ok_or("truncated V2")?,
                name: utf16_name(rec, name_off, name_len)?,
            })
        }
        3 => {
            let name_len = u16_at(rec, 72).ok_or("truncated V3")? as usize;
            let name_off = u16_at(rec, 74).ok_or("truncated V3")? as usize;
            Some(UsnRecord {
                frn: u64_at(rec, 8).ok_or("truncated V3")?,
                parent_frn: u64_at(rec, 24).ok_or("truncated V3")?,
                usn: u64_at(rec, 40).ok_or("truncated V3")? as i64,
                timestamp: u64_at(rec, 48).ok_or("truncated V3")? as i64,
                reason: u32_at(rec, 56).ok_or("truncated V3")?,
                attributes: u32_at(rec, 68).ok_or("truncated V3")?,
                name: utf16_name(rec, name_off, name_len)?,
            })
        }
        _ => None,
    };
    Ok((parsed, len))
}

fn utf16_name(rec: &[u8], off: usize, len: usize) -> Result<String, String> {
    let bytes = rec.get(off..off + len).ok_or("file name outside the record")?;
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    Ok(String::from_utf16_lossy(&units))
}

/// Parse an `FSCTL_READ_USN_JOURNAL` / `FSCTL_ENUM_USN_DATA` output buffer: a leading
/// 8-byte cursor (next USN / next file reference number) followed by records.
pub fn parse_buffer(b: &[u8]) -> Result<(u64, Vec<UsnRecord>), String> {
    let next = u64_at(b, 0).ok_or("buffer shorter than its 8-byte header")?;
    let mut out = Vec::new();
    let mut off = 8;
    while off + 4 <= b.len() {
        let (rec, len) = parse_record(&b[off..])?;
        out.extend(rec);
        // Records are 8-byte aligned.
        off += len.next_multiple_of(8).max(8);
    }
    Ok((next, out))
}

/// What the journal says happened to one file since the last checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub frn: u64,
    /// OR of every record's reason.
    pub reasons: u32,
    pub is_dir: bool,
    /// Last name seen.
    pub name: String,
    pub parent_frn: u64,
}

/// Fold records into one [`Change`] per file, in first-seen order.
pub fn coalesce(records: &[UsnRecord]) -> Vec<Change> {
    let mut order = Vec::new();
    let mut map: HashMap<u64, Change> = HashMap::new();
    for r in records {
        let c = map.entry(r.frn).or_insert_with(|| {
            order.push(r.frn);
            Change { frn: r.frn, reasons: 0, is_dir: r.is_dir(), name: r.name.clone(), parent_frn: r.parent_frn }
        });
        c.reasons |= r.reason;
        c.is_dir |= r.is_dir();
        c.name = r.name.clone();
        c.parent_frn = r.parent_frn;
    }
    order.into_iter().filter_map(|f| map.remove(&f)).collect()
}

/// An index update derived from the journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Re-read this path (created / modified / renamed to). `crawl`: a folder that
    /// appeared or moved, whose contents must be (re)indexed too.
    Refresh { path: PathBuf, frn: u64, crawl: bool },
    /// The file with this id is gone (or moved out of reach): drop its rows.
    Forget { frn: u64 },
}

/// Plan the updates for a run of changes. `resolve` maps a file id to its current path
/// (`None` when it no longer exists). Only changes that matter for the index (create,
/// delete, rename, data or attribute change) are kept.
pub fn plan(changes: &[Change], mut resolve: impl FnMut(u64) -> Option<PathBuf>) -> Vec<Action> {
    use reason::*;
    let relevant = FILE_CREATE
        | FILE_DELETE
        | RENAME_OLD_NAME
        | RENAME_NEW_NAME
        | DATA_OVERWRITE
        | DATA_EXTEND
        | DATA_TRUNCATION
        | BASIC_INFO_CHANGE;
    let mut out = Vec::new();
    for c in changes {
        if c.reasons & relevant == 0 {
            continue;
        }
        match resolve(c.frn) {
            Some(path) => {
                let crawl = c.is_dir && c.reasons & (FILE_CREATE | RENAME_NEW_NAME) != 0;
                if c.reasons & (RENAME_OLD_NAME | RENAME_NEW_NAME) != 0 {
                    // Drop the rows under the old name first; the refresh re-adds them.
                    out.push(Action::Forget { frn: c.frn });
                }
                out.push(Action::Refresh { path, frn: c.frn, crawl });
            }
            None => out.push(Action::Forget { frn: c.frn }),
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod fixtures {
    /// Build a `USN_RECORD_V2` (8-byte aligned) like the kernel does.
    pub fn v2(frn: u64, parent: u64, usn: i64, reason: u32, attrs: u32, name: &str) -> Vec<u8> {
        let name16: Vec<u8> = name.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let len = (60 + name16.len()).next_multiple_of(8);
        let mut b = vec![0u8; len];
        b[0..4].copy_from_slice(&(len as u32).to_le_bytes());
        b[4..6].copy_from_slice(&2u16.to_le_bytes());
        b[8..16].copy_from_slice(&frn.to_le_bytes());
        b[16..24].copy_from_slice(&parent.to_le_bytes());
        b[24..32].copy_from_slice(&usn.to_le_bytes());
        b[32..40].copy_from_slice(&133_000_000_000_000_000i64.to_le_bytes());
        b[40..44].copy_from_slice(&reason.to_le_bytes());
        b[52..56].copy_from_slice(&attrs.to_le_bytes());
        b[56..58].copy_from_slice(&(name16.len() as u16).to_le_bytes());
        b[58..60].copy_from_slice(&60u16.to_le_bytes());
        b[60..60 + name16.len()].copy_from_slice(&name16);
        b
    }

    /// Build a `USN_RECORD_V3` with 128-bit ids (high halves set to junk).
    pub fn v3(frn: u64, parent: u64, usn: i64, reason: u32, attrs: u32, name: &str) -> Vec<u8> {
        let name16: Vec<u8> = name.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let len = (76 + name16.len()).next_multiple_of(8);
        let mut b = vec![0u8; len];
        b[0..4].copy_from_slice(&(len as u32).to_le_bytes());
        b[4..6].copy_from_slice(&3u16.to_le_bytes());
        b[8..16].copy_from_slice(&frn.to_le_bytes());
        b[16..24].copy_from_slice(&0xDEAD_u64.to_le_bytes());
        b[24..32].copy_from_slice(&parent.to_le_bytes());
        b[32..40].copy_from_slice(&0xBEEF_u64.to_le_bytes());
        b[40..48].copy_from_slice(&usn.to_le_bytes());
        b[56..60].copy_from_slice(&reason.to_le_bytes());
        b[68..72].copy_from_slice(&attrs.to_le_bytes());
        b[72..74].copy_from_slice(&(name16.len() as u16).to_le_bytes());
        b[74..76].copy_from_slice(&76u16.to_le_bytes());
        b[76..76 + name16.len()].copy_from_slice(&name16);
        b
    }

    pub fn buffer(next: u64, records: &[Vec<u8>]) -> Vec<u8> {
        let mut b = next.to_le_bytes().to_vec();
        for r in records {
            b.extend_from_slice(r);
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::reason::*;
    use super::*;

    #[test]
    fn parses_v2_and_v3_records() {
        let buf = buffer(
            4242,
            &[
                v2(10, 5, 100, FILE_CREATE | CLOSE, 0x20, "report.pdf"),
                v3(11, 5, 101, RENAME_NEW_NAME, ATTR_DIRECTORY, "Ünïcødé dir"),
                v2(12, 11, 102, FILE_DELETE | CLOSE, 0x20, "a"),
            ],
        );
        let (next, recs) = parse_buffer(&buf).unwrap();
        assert_eq!(next, 4242);
        assert_eq!(recs.len(), 3);
        assert_eq!(recs[0].frn, 10);
        assert_eq!(recs[0].parent_frn, 5);
        assert_eq!(recs[0].usn, 100);
        assert_eq!(recs[0].name, "report.pdf");
        assert_eq!(recs[0].reason, FILE_CREATE | CLOSE);
        assert!(!recs[0].is_dir());
        assert_eq!(recs[1].frn, 11, "low 64 bits of a 128-bit id");
        assert_eq!(recs[1].parent_frn, 5);
        assert_eq!(recs[1].name, "Ünïcødé dir");
        assert!(recs[1].is_dir());
        assert_eq!(recs[2].name, "a");
    }

    #[test]
    fn empty_and_bad_buffers() {
        let (next, recs) = parse_buffer(&7u64.to_le_bytes()).unwrap();
        assert_eq!((next, recs.len()), (7, 0));
        assert!(parse_buffer(&[1, 2, 3]).is_err());
        let mut bad = buffer(1, &[v2(1, 2, 3, 0, 0, "x")]);
        bad[8..12].copy_from_slice(&999u32.to_le_bytes());
        assert!(parse_buffer(&bad).is_err(), "length past the end");
        let mut name_oob = v2(1, 2, 3, 0, 0, "xy");
        name_oob[58..60].copy_from_slice(&200u16.to_le_bytes());
        assert!(parse_record(&name_oob).is_err());
        // Unknown versions are skipped, not errors.
        let mut v4 = v2(1, 2, 3, 0, 0, "x");
        v4[4..6].copy_from_slice(&4u16.to_le_bytes());
        let (rec, len) = parse_record(&v4).unwrap();
        assert!(rec.is_none());
        assert_eq!(len, v4.len());
    }

    #[test]
    fn coalesces_per_file() {
        let (_, recs) = parse_buffer(&buffer(
            0,
            &[
                v2(1, 9, 1, FILE_CREATE, 0, "tmp"),
                v2(2, 9, 2, DATA_EXTEND, 0, "b"),
                v2(1, 9, 3, RENAME_OLD_NAME, 0, "tmp"),
                v2(1, 9, 4, RENAME_NEW_NAME | CLOSE, 0, "final"),
            ],
        ))
        .unwrap();
        let c = coalesce(&recs);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].frn, 1);
        assert_eq!(c[0].name, "final");
        assert_eq!(c[0].reasons, FILE_CREATE | RENAME_OLD_NAME | RENAME_NEW_NAME | CLOSE);
        assert_eq!(c[1].frn, 2);
    }

    #[test]
    fn plans_updates() {
        let changes = coalesce(
            &parse_buffer(&buffer(
                0,
                &[
                    v2(1, 9, 1, FILE_CREATE | CLOSE, 0x20, "new.txt"),
                    v2(2, 9, 2, FILE_DELETE | CLOSE, 0x20, "gone.txt"),
                    v2(3, 9, 3, RENAME_OLD_NAME, ATTR_DIRECTORY, "old"),
                    v2(3, 9, 4, RENAME_NEW_NAME, ATTR_DIRECTORY, "moved"),
                    v2(4, 9, 5, CLOSE, 0x20, "touched"),
                    v2(5, 9, 6, DATA_EXTEND, 0x20, "deleted later"),
                ],
            ))
            .unwrap()
            .1,
        );
        let resolve = |frn: u64| match frn {
            1 => Some(PathBuf::from("/r/new.txt")),
            3 => Some(PathBuf::from("/r/moved")),
            4 => Some(PathBuf::from("/r/touched")),
            _ => None,
        };
        let plan = plan(&changes, resolve);
        assert_eq!(
            plan,
            vec![
                Action::Refresh { path: "/r/new.txt".into(), frn: 1, crawl: false },
                Action::Forget { frn: 2 },
                Action::Forget { frn: 3 },
                Action::Refresh { path: "/r/moved".into(), frn: 3, crawl: true },
                Action::Forget { frn: 5 },
            ],
            "close-only records are ignored"
        );
    }
}
