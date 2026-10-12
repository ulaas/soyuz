//! Directory watching for open panes: one `soyuz_fs::Watcher` (one notify backend) with
//! a non-recursive watch per pane directory.
//!
//! The watcher lives on its own thread: registering a watch opens the directory, which
//! can block on a slow or dead network share, so the UI only sends commands. Events are
//! drained by the UI each frame and mapped back to panes through a shared id map.
//! `soyuz_fs::Watcher` re-arms watches that go silent and reports them as `Rescan`; on
//! top of that, finished jobs and manual refreshes relist panes (`ops::refresh_affected`,
//! `ops::refresh`), so nothing depends on the watcher alone.
//!
//! When the OS refuses more watches (Linux: `fs.inotify.max_user_watches`), folders are
//! polled every `[behaviour] watch_poll_interval_ms` instead; [`WatchService::take_degraded`]
//! reports that once per session so the UI can say so (one toast, not per folder).

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
    /// Scan interval of polled watches (applies to the polling backend, which is created
    /// when the first watch has to be polled).
    PollInterval(Duration),
}

/// Watch-degradation state shared with the watcher thread.
#[derive(Default)]
struct Degraded {
    /// Why some watches are polled (`soyuz_fs::Watcher::degraded`).
    reason: Option<String>,
    /// The UI was told already (once per session).
    reported: bool,
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
    degraded: Arc<Mutex<Degraded>>,
    poll_interval: Option<Duration>,
}

impl WatchService {
    pub fn new(debounce: Duration, wake: Wake) -> Self {
        let (ev_tx, ev_rx) = unbounded();
        let (tx, cmd_rx) = unbounded::<Command>();
        let maps: Arc<Mutex<Maps>> = Arc::default();
        let thread_maps = maps.clone();
        let degraded: Arc<Mutex<Degraded>> = Arc::default();
        let thread_degraded = degraded.clone();
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
                    Command::PollInterval(d) => watcher.set_poll_interval(d),
                }
                if let Some(why) = watcher.degraded() {
                    let mut d = thread_degraded.lock();
                    if d.reason.is_none() {
                        log::warn!("file watching degraded: {why}");
                        d.reason = Some(why.to_string());
                    }
                }
            }
        });
        if let Err(e) = spawned {
            log::error!("cannot spawn watcher thread: {e}");
        }
        WatchService { tx, rx: ev_rx, maps, degraded, poll_interval: None }
    }

    /// `[behaviour] watch_poll_interval_ms` (cheap; only sends when it changed). Polled
    /// watches pick it up when the polling backend starts, i.e. the first time a folder
    /// has to be polled in this session.
    pub fn set_poll_interval(&mut self, every: Duration) {
        if self.poll_interval != Some(every) {
            self.poll_interval = Some(every);
            let _ = self.tx.send(Command::PollInterval(every));
        }
    }

    /// Some folders are polled instead of watched (the OS watch limit is reached):
    /// `Some(reason)` the first time this is seen, then `None` for the rest of the
    /// session (the UI shows one hint, not one per folder).
    pub fn take_degraded(&mut self) -> Option<String> {
        let mut d = self.degraded.lock();
        if d.reported {
            return None;
        }
        let r = d.reason.clone()?;
        d.reported = true;
        Some(r)
    }

    /// Why some folders are polled instead of watched, if any are (stays set).
    pub fn degraded(&self) -> Option<String> {
        self.degraded.lock().reason.clone()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn degraded_is_reported_once() {
        let mut w = WatchService::new(Duration::from_millis(50), Arc::new(|| {}));
        assert_eq!(w.take_degraded(), None);
        w.degraded.lock().reason = Some("limit".into());
        assert_eq!(w.take_degraded().as_deref(), Some("limit"));
        assert_eq!(w.take_degraded(), None, "once per session");
        assert_eq!(w.degraded().as_deref(), Some("limit"));
    }

    #[test]
    fn poll_interval_is_sent_only_on_change() {
        let mut w = WatchService::new(Duration::from_millis(50), Arc::new(|| {}));
        w.set_poll_interval(Duration::from_secs(2));
        w.set_poll_interval(Duration::from_secs(2));
        assert_eq!(w.poll_interval, Some(Duration::from_secs(2)));
        w.set_poll_interval(Duration::from_secs(5));
        assert_eq!(w.poll_interval, Some(Duration::from_secs(5)));
    }
}
