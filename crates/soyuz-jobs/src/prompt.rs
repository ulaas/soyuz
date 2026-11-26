//! Job thread ↔ UI prompt protocol.

use std::path::PathBuf;

use soyuz_core::{ConflictResolution, EntryInfo, ErrorAction, FsError};
pub use soyuz_fs::in_use::Blocker;

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

/// The programs that keep a failed item in use (see `soyuz_fs::in_use`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InUse {
    /// Never empty.
    pub blockers: Vec<Blocker>,
    /// `false` when the item is too big to have been examined completely.
    pub complete: bool,
    /// `PromptResponse::CloseBlockers { force: true }` is allowed
    /// (`QueueSettings::end_blockers`).
    pub may_end: bool,
    /// This prompt follows a close request that left the item in use (the program asks to
    /// save, or ignored the request).
    pub still_open: bool,
}

impl InUse {
    /// The programs a close request acts on.
    pub fn closable(&self) -> impl Iterator<Item = &Blocker> {
        self.blockers.iter().filter(|b| b.closable())
    }
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
        /// Who keeps the item in use, when that is why it failed and it could be found.
        in_use: Option<InUse>,
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
    /// Answer to an `Error` prompt with `in_use`: close the programs that keep the item in
    /// use, then try again. `force` ends them at once (their unsaved work is lost); it is
    /// refused unless `InUse::may_end`.
    CloseBlockers {
        force: bool,
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
