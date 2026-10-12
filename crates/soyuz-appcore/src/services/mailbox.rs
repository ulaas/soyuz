//! The single mailbox app-owned background services report into. Producers
//! send a [`UiMsg`] and wake the UI; `AppCore::pump_services` drains it at frame start.

use std::path::PathBuf;

use crossbeam_channel::{Receiver, Sender, unbounded};
use soyuz_core::{EntryInfo, FsError, Location};
use soyuz_fs::VolumeInfo;

use crate::models::toasts::Toast;
use crate::state::pane::PaneId;

pub use crate::host::Wake;

#[derive(Debug)]
pub enum UiMsg {
    /// A streamed chunk of a pane listing. `done` marks the last message of generation
    /// `generation`; `err` is set when the listing failed.
    Listing { pane: PaneId, generation: u64, chunk: Vec<EntryInfo>, done: bool, err: Option<FsError> },
    /// Watcher-driven in-place update (re-stat of changed paths).
    Patched { pane: PaneId, generation: u64, updated: Vec<EntryInfo>, removed: Vec<String> },
    /// A one-shot directory listing used by breadcrumb dropdowns, path completion and
    /// Miller columns.
    Aux { path: PathBuf, result: Result<Vec<EntryInfo>, FsError> },
    /// Nearest existing ancestor of a pane's lost directory.
    Resolved { pane: PaneId, generation: u64, location: Location },
    /// Result of a "navigate if the directory exists" probe (synchronized navigation).
    Probed { pane: PaneId, location: Location, exists: bool },
    /// Refreshed volume list (with free space where available).
    Volumes(Vec<VolumeInfo>),
    /// Cloud-storage folders detected in the background.
    CloudPlaces(Vec<(String, PathBuf)>),
    /// A notification from a worker (e.g. a failed launch).
    Notice(Toast),
    /// A shortcut resolved to a folder: navigate the focused pane there.
    OpenDir(PathBuf),
    /// A worker found a file to open with its default application.
    Launch(PathBuf),
}

/// Cloneable sending half handed to worker threads.
#[derive(Clone)]
pub struct Postman {
    tx: Sender<UiMsg>,
    wake: Wake,
}

impl Postman {
    /// Send and wake the UI. Returns `false` when the app is gone.
    pub fn send(&self, msg: UiMsg) -> bool {
        let ok = self.tx.send(msg).is_ok();
        (self.wake)();
        ok
    }

    pub fn wake(&self) -> Wake {
        self.wake.clone()
    }
}

pub struct Mailbox {
    rx: Receiver<UiMsg>,
    postman: Postman,
}

impl Mailbox {
    pub fn new(wake: Wake) -> Self {
        let (tx, rx) = unbounded();
        Mailbox { rx, postman: Postman { tx, wake } }
    }

    pub fn postman(&self) -> Postman {
        self.postman.clone()
    }

    pub fn try_recv(&self) -> Option<UiMsg> {
        self.rx.try_recv().ok()
    }
}
