//! Job thread ↔ UI prompt protocol.

use std::path::PathBuf;

use soyuz_core::{ConflictResolution, EntryInfo, ErrorAction, FsError};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConflictKind {
    FileExists,
    DirExists,
    FileVsDir,
    SameFile,
    MoveIntoSelf,
    CaseOnly,
}

#[derive(Clone, Debug)]
pub struct ConflictInfo {
    pub src: EntryInfo,
    pub dst: EntryInfo,
    pub kind: ConflictKind,
    pub suggested_name: String,
}

/// Kept unboxed (frozen API); prompts are rare and short-lived.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum JobPrompt {
    Conflict(ConflictInfo),
    Error {
        path: PathBuf,
        error: FsError,
        retryable: bool,
    },
    NotEnoughSpace {
        needed: u64,
        free: u64,
        dest: PathBuf,
    },
    /// Offer Permanent / Skip / Cancel.
    TrashUnsupported {
        path: PathBuf,
        reason: String,
    },
    ReadOnlyItem {
        path: PathBuf,
    },
    Password {
        archive: PathBuf,
        wrong: bool,
    },
}

#[derive(Clone, Debug)]
pub enum PromptResponse {
    Conflict {
        resolution: ConflictResolution,
        apply_to_all: bool,
    },
    Error {
        action: ErrorAction,
        apply_to_all: bool,
    },
    Space {
        proceed: bool,
    },
    TrashUnsupported {
        permanent: bool,
        apply_to_all: bool,
    },
    ReadOnly {
        force: bool,
        apply_to_all: bool,
    },
    Password(Option<String>),
    /// Sent by `JobEngine::cancel()` to unblock a waiting job.
    Cancelled,
}
