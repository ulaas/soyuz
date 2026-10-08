//! Job history (`history.jsonl`, one JSON object per line; the engine is the only writer).
//!
//! Entries are created only when a journaled job reaches `Done`. Because undo/redo
//! mutate existing entries (`undone`, `records`), the engine rewrites the whole file
//! atomically (`soyuz_fs::write_atomic`) on every change instead of appending; the file
//! stays valid JSONL at all times and is bounded by `EngineConfig::history_limit`.
//!
//! Compatibility: fields added after the first release (`stamps`, `options`) are optional
//! with serde defaults, so older files load unchanged and newer files stay readable.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use soyuz_core::time::serde_rfc3339;

use crate::job::{JobKind, JobOptions, JobSummary};
use crate::undo::UndoRecord;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct HistoryId(pub u64);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: HistoryId,
    pub job: JobKind,
    pub summary: JobSummary,
    #[serde(with = "serde_rfc3339")]
    pub when: SystemTime,
    pub records: Vec<UndoRecord>,
    pub undone: bool,
    /// `false` when any record is `Irreversible` (UI disables Undo with a tooltip).
    pub undoable: bool,
    /// Identity of what `Copied` / `Extracted` / `Created` records produced, taken when the
    /// job finished. Undo refuses to remove a path whose identity no longer matches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stamps: Vec<UndoStamp>,
    /// Options of the original job (reused by redo / retry). `None` in old files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<JobOptions>,
    /// Created (or redone) by this engine instance, i.e. in this session. Not persisted:
    /// entries loaded from `history.jsonl` are `false` (the app's Ctrl+Z skips them).
    #[serde(skip)]
    pub this_session: bool,
}

/// What a path looked like right after the job created it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoStamp {
    pub path: PathBuf,
    /// Device / volume serial.
    pub dev: u64,
    /// File id (inode / 128-bit NTFS id) as lowercase hex.
    pub id: String,
    /// Size of a non-directory (directories change size and time as content changes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// Modification time of a non-directory, nanoseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_ns: Option<u64>,
}

impl UndoStamp {
    /// Stamp of `path` as it is now (links not followed); `None` if it cannot be read.
    pub fn take(path: &Path) -> Option<UndoStamp> {
        let ident = soyuz_fs::file_identity(path, false).ok()?;
        let e = soyuz_fs::stat(path, false).ok()?;
        let dir = matches!(e.kind, soyuz_core::EntryKind::Dir);
        let ns = e
            .modified
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| u64::try_from(d.as_nanos()).ok());
        Some(UndoStamp {
            path: path.to_path_buf(),
            dev: ident.dev,
            id: format!("{:x}", ident.id),
            size: (!dir).then_some(e.size),
            modified_ns: if dir { None } else { ns },
        })
    }

    /// Same object, and (for files) same size and modification time.
    pub fn matches(&self, now: &UndoStamp) -> bool {
        self.dev == now.dev && self.id == now.id && self.size == now.size && self.modified_ns == now.modified_ns
    }
}

/// An entry can be undone when it recorded something and nothing irreversible.
pub(crate) fn is_undoable(records: &[UndoRecord]) -> bool {
    !records.is_empty() && !records.iter().any(|r| matches!(r, UndoRecord::Irreversible(_)))
}

/// Read `history.jsonl`; a missing file is an empty history, bad lines are skipped.
pub(crate) fn load(path: &Path) -> Vec<HistoryEntry> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("soyuz-jobs: cannot read history {}: {e}", path.display());
            }
            return Vec::new();
        }
    };
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .filter_map(|(i, l)| match serde_json::from_str::<HistoryEntry>(l) {
            Ok(e) => Some(e),
            Err(err) => {
                log::warn!("soyuz-jobs: {}:{}: skipping bad history line: {err}", path.display(), i + 1);
                None
            }
        })
        .collect()
}

/// Serialize to JSONL (one object per line, trailing newline).
pub(crate) fn to_jsonl(entries: &[HistoryEntry]) -> String {
    let mut out = String::new();
    for e in entries {
        match serde_json::to_string(e) {
            Ok(line) => {
                out.push_str(&line);
                out.push('\n');
            }
            Err(err) => log::warn!("soyuz-jobs: cannot serialize history entry {:?}: {err}", e.id),
        }
    }
    out
}

/// Atomically replace `history.jsonl`.
pub(crate) fn save(path: &Path, entries: &[HistoryEntry]) -> soyuz_core::FsResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        soyuz_fs::create_dir_all(parent)?;
    }
    soyuz_fs::write_atomic(path, to_jsonl(entries).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64) -> HistoryEntry {
        HistoryEntry {
            id: HistoryId(id),
            job: JobKind::EmptyTrash,
            summary: JobSummary::default(),
            when: SystemTime::UNIX_EPOCH,
            records: vec![UndoRecord::Created(PathBuf::from("/x"))],
            undone: false,
            undoable: true,
            stamps: Vec::new(),
            options: None,
            this_session: true,
        }
    }

    #[test]
    fn jsonl_round_trip() {
        let e = entry(7);
        let line = serde_json::to_string(&e).unwrap();
        assert!(!line.contains('\n'));
        let back: HistoryEntry = serde_json::from_str(&line).unwrap();
        assert_eq!(back.id, e.id);
        assert_eq!(back.records, e.records);
        assert_eq!(back.when, e.when);
    }

    #[test]
    fn file_round_trip_skips_bad_lines() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub").join("history.jsonl");
        save(&p, &[entry(1), entry(2)]).unwrap();
        let mut text = std::fs::read_to_string(&p).unwrap();
        assert_eq!(text.lines().count(), 2);
        text.push_str("{not json}\n\n");
        std::fs::write(&p, text).unwrap();
        let back = load(&p);
        assert_eq!(back.iter().map(|e| e.id.0).collect::<Vec<_>>(), vec![1, 2]);
        assert!(load(&d.path().join("missing.jsonl")).is_empty());
    }

    #[test]
    fn old_lines_without_new_fields_still_load() {
        // A line written before `stamps` / `options` existed.
        let line = r#"{"id":3,"job":"empty_trash","summary":{"items_ok":0,"items_skipped":0,"items_failed":0,"bytes":0,"elapsed":{"secs":0,"nanos":0},"errors":[],"notes":[]},"when":"2026-01-02T03:04:05Z","records":[{"copied":{"dst":"/d/f"}}],"undone":false,"undoable":true}"#;
        let e: HistoryEntry = serde_json::from_str(line).unwrap();
        assert!(e.stamps.is_empty() && e.options.is_none() && !e.this_session);
        assert_eq!(e.records, vec![UndoRecord::Copied { dst: "/d/f".into() }]);
        let mut e2 = entry(4);
        e2.stamps.push(UndoStamp { path: "/x".into(), dev: 1, id: "ff".into(), size: Some(2), modified_ns: None });
        e2.options = Some(JobOptions { journal: true, ..JobOptions::default() });
        let back: HistoryEntry = serde_json::from_str(&serde_json::to_string(&e2).unwrap()).unwrap();
        assert_eq!(back.stamps, e2.stamps);
        assert!(back.options.unwrap().journal);
        assert!(!back.this_session, "session flag is not persisted");
    }

    #[test]
    fn stamp_detects_changes() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f");
        std::fs::write(&f, b"one").unwrap();
        let s = UndoStamp::take(&f).unwrap();
        assert!(s.matches(&UndoStamp::take(&f).unwrap()));
        std::fs::write(&f, b"changed!").unwrap();
        assert!(!s.matches(&UndoStamp::take(&f).unwrap()), "size changed");
        let g = d.path().join("g");
        std::fs::write(&g, b"one").unwrap();
        let other = UndoStamp::take(&g).unwrap();
        assert_ne!(s.id, other.id, "a different file with the same content");
        assert!(UndoStamp::take(&d.path().join("missing")).is_none());
    }

    #[test]
    fn undoable_rules() {
        assert!(!is_undoable(&[]));
        assert!(is_undoable(&[UndoRecord::Created("/a".into())]));
        assert!(!is_undoable(&[UndoRecord::Created("/a".into()), UndoRecord::Irreversible("/b".into())]));
    }
}
