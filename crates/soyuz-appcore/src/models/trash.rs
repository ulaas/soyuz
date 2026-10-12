//! The Trash location's state (frontend-agnostic): items of every bin listed by
//! `soyuz_fs::trash::list` on a worker thread, the selection, loading / error state.
//! `ui::views::trash` draws it; restore, empty and purge run as jobs.

use std::collections::BTreeSet;
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, unbounded};
use soyuz_fs::TrashItem;

use crate::host::Wake;
use crate::models::toasts::Toast;

enum TrashMsg {
    Listed(Result<Vec<TrashItem>, String>),
}

pub struct TrashState {
    pub items: Vec<TrashItem>,
    /// Selected items by [`key`].
    pub selected: BTreeSet<String>,
    anchor: Option<String>,
    pub loading: bool,
    pub loaded: bool,
    stale: bool,
    pub error: Option<String>,
    tx: Sender<TrashMsg>,
    rx: Receiver<TrashMsg>,
    wake: Wake,
    /// A message for the toast stack (set by `poll`, taken by the view).
    notice: Option<Toast>,
    last_load: Option<Instant>,
}

/// Selection key of a trashed item (unique across bins).
pub fn key(i: &TrashItem) -> String {
    format!("{}|{}", i.trash_root.display(), i.id)
}

impl TrashState {
    pub fn new(wake: Wake) -> Self {
        let (tx, rx) = unbounded();
        TrashState {
            items: Vec::new(),
            selected: BTreeSet::new(),
            anchor: None,
            loading: false,
            loaded: false,
            stale: true,
            error: None,
            tx,
            rx,
            wake,
            notice: None,
            last_load: None,
        }
    }

    pub fn items(&self) -> &[TrashItem] {
        &self.items
    }

    pub fn mark_stale(&mut self) {
        self.stale = true;
    }

    /// Load when shown for the first time or after a trash-related job.
    pub fn ensure_loaded(&mut self) {
        if (self.stale || !self.loaded) && !self.loading {
            self.reload();
        }
    }

    pub fn reload(&mut self) {
        self.loading = true;
        self.stale = false;
        self.last_load = Some(Instant::now());
        let tx = self.tx.clone();
        let wake = self.wake.clone();
        let spawned = std::thread::Builder::new().name("soyuz-trash-list".into()).spawn(move || {
            let r = soyuz_fs::trash::list().map_err(|e| e.to_string());
            let _ = tx.send(TrashMsg::Listed(r));
            wake();
        });
        if spawned.is_err() {
            self.loading = false;
            self.error = Some(tr!("views-trash-reader-failed").into());
        }
    }

    pub fn poll(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                TrashMsg::Listed(Ok(items)) => {
                    self.items = items;
                    let keys: BTreeSet<String> = self.items.iter().map(key).collect();
                    self.selected.retain(|k| keys.contains(k));
                    self.loading = false;
                    self.loaded = true;
                    self.error = None;
                }
                TrashMsg::Listed(Err(e)) => {
                    self.loading = false;
                    self.loaded = true;
                    self.error = Some(e);
                }
            }
        }
    }

    pub fn take_notice(&mut self) -> Option<Toast> {
        self.notice.take()
    }

    pub fn selected_items(&self) -> Vec<TrashItem> {
        self.items.iter().filter(|i| self.selected.contains(&key(i))).cloned().collect()
    }

    /// A click on row `idx`: `shift` extends from the anchor, `command` toggles.
    pub fn click(&mut self, idx: usize, shift: bool, command: bool) {
        let k = key(&self.items[idx]);
        if shift
            && let Some(a) = self.anchor.clone()
            && let Some(ai) = self.items.iter().position(|i| key(i) == a)
        {
            let (lo, hi) = if ai <= idx { (ai, idx) } else { (idx, ai) };
            if !command {
                self.selected.clear();
            }
            for i in &self.items[lo..=hi] {
                self.selected.insert(key(i));
            }
            return;
        }
        if command {
            if !self.selected.remove(&k) {
                self.selected.insert(k.clone());
            }
        } else {
            self.selected.clear();
            self.selected.insert(k.clone());
        }
        self.anchor = Some(k);
    }
}
