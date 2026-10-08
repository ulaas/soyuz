//! Job history (`history.jsonl`, one JSON object per line; the engine is the only writer).
//!
//! Entries are created only when a journaled job reaches `Done`. Because undo/redo
//! mutate existing entries (`undone`, `records`), the engine rewrites the whole file
//! atomically (`soyuz_fs::write_atomic`) on every change instead of appending; the file
//! stays valid JSONL at all times and is bounded by `EngineConfig::history_limit`.

use std::path::Path;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use soyuz_core::time::serde_rfc3339;

use crate::job::{JobKind, JobSummary};
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
    use std::path::PathBuf;

    fn entry(id: u64) -> HistoryEntry {
        HistoryEntry {
            id: HistoryId(id),
            job: JobKind::EmptyTrash,
            summary: JobSummary::default(),
            when: SystemTime::UNIX_EPOCH,
            records: vec![UndoRecord::Created(PathBuf::from("/x"))],
            undone: false,
            undoable: true,
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
    fn undoable_rules() {
        assert!(!is_undoable(&[]));
        assert!(is_undoable(&[UndoRecord::Created("/a".into())]));
        assert!(!is_undoable(&[UndoRecord::Created("/a".into()), UndoRecord::Irreversible("/b".into())]));
    }
}
