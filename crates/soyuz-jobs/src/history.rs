//! Job history (`history.jsonl`, one JSON object per line; the engine is the only writer).

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn jsonl_round_trip() {
        let e = HistoryEntry {
            id: HistoryId(7),
            job: JobKind::EmptyTrash,
            summary: JobSummary::default(),
            when: SystemTime::UNIX_EPOCH,
            records: vec![UndoRecord::Created(PathBuf::from("/x"))],
            undone: false,
            undoable: true,
        };
        let line = serde_json::to_string(&e).unwrap();
        assert!(!line.contains('\n'));
        let back: HistoryEntry = serde_json::from_str(&line).unwrap();
        assert_eq!(back.id, e.id);
        assert_eq!(back.records, e.records);
        assert_eq!(back.when, e.when);
    }
}
