//! Directory watching for open panes: one `soyuz_fs::Watcher` (one notify backend) with
//! a non-recursive watch per pane directory.
//!
//! The watcher lives on its own thread: registering a watch can be slow (the notify
//! debouncer's file-id cache scans the directory, which takes seconds for 100k-entry
//! folders), so the UI only sends commands. Events are drained by the UI each frame
//! and mapped back to panes through a shared id map.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::Mutex;
use soyuz_fs::{WatchEvent, WatchId, WatchKind, Watcher};

use super::mailbox::Wake;
use crate::state::pane::PaneId;

enum Command {
    Watch(PaneId, PathBuf),
    Unwatch(PaneId),
}

#[derive(Default)]
struct Maps {
    by_id: HashMap<WatchId, PaneId>,
    by_pane: HashMap<PaneId, WatchId>,
}

pub struct WatchService {
    tx: Sender<Command>,
    rx: Receiver<WatchEvent>,
    maps: Arc<Mutex<Maps>>,
}

impl WatchService {
    pub fn new(debounce: Duration, wake: Wake) -> Self {
        let (ev_tx, ev_rx) = unbounded();
        let (tx, cmd_rx) = unbounded::<Command>();
        let maps: Arc<Mutex<Maps>> = Arc::default();
        let thread_maps = maps.clone();
        let spawned = thread::Builder::new().name("soyuz-watcher".into()).spawn(move || {
            let mut watcher = match Watcher::new(ev_tx, debounce, wake) {
                Ok(w) => w,
                Err(e) => {
                    log::warn!("file watching unavailable: {e}");
                    // Keep draining commands so senders never block.
                    while cmd_rx.recv().is_ok() {}
                    return;
                }
            };
            while let Ok(cmd) = cmd_rx.recv() {
                match cmd {
                    Command::Watch(pane, dir) => {
                        unwatch(&mut watcher, &thread_maps, pane);
                        match watcher.watch(&dir, false) {
                            Ok(id) => {
                                let mut m = thread_maps.lock();
                                m.by_id.insert(id, pane);
                                m.by_pane.insert(pane, id);
                            }
                            Err(e) => log::debug!("not watching {}: {e}", dir.display()),
                        }
                    }
                    Command::Unwatch(pane) => unwatch(&mut watcher, &thread_maps, pane),
                }
            }
        });
        if let Err(e) = spawned {
            log::error!("cannot spawn watcher thread: {e}");
        }
        WatchService { tx, rx: ev_rx, maps }
    }

    /// Watch `dir` for `pane`, replacing the pane's previous watch.
    pub fn watch(&mut self, pane: PaneId, dir: &Path) {
        // Forget the old mapping right away so stale events are dropped.
        self.maps.lock().by_id.retain(|_, p| *p != pane);
        let _ = self.tx.send(Command::Watch(pane, dir.to_path_buf()));
    }

    pub fn unwatch(&mut self, pane: PaneId) {
        self.maps.lock().by_id.retain(|_, p| *p != pane);
        let _ = self.tx.send(Command::Unwatch(pane));
    }

    /// Pending events, mapped to panes (events for forgotten watches are dropped).
    pub fn drain(&mut self) -> Vec<(PaneId, WatchKind)> {
        let mut out = Vec::new();
        let maps = self.maps.lock();
        while let Ok(ev) = self.rx.try_recv() {
            if let Some(p) = maps.by_id.get(&ev.id) {
                out.push((*p, ev.kind));
            }
        }
        out
    }
}

fn unwatch(watcher: &mut Watcher, maps: &Mutex<Maps>, pane: PaneId) {
    let id = {
        let mut m = maps.lock();
        let id = m.by_pane.remove(&pane);
        if let Some(id) = id {
            m.by_id.remove(&id);
        }
        id
    };
    if let Some(id) = id {
        watcher.unwatch(id);
    }
}
