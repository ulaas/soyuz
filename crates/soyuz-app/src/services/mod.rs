//! App-owned background services. Library crates that hand out their own receivers
//! (jobs engine, search, folder sizes, terminal) are polled directly by the integration
//! layer; everything here reports through the [`mailbox`].

pub mod ai_service;
pub mod archive;
pub mod config_watch;
pub mod crash;
pub mod git;
pub mod jobs;
pub mod lister;
pub mod mailbox;
pub mod mcp_bridge;
pub mod os_clipboard;
pub mod places;
pub mod preview;
pub mod preview_cache;
pub mod preview_media;
pub mod preview_text;
pub mod recent;
pub mod search;
pub mod store_file;
pub mod tags;
pub mod thumbs;
pub mod tree;
pub mod watcher;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::Context;
use soyuz_core::{EntryInfo, FsError};

use lister::Lister;
use mailbox::{Mailbox, Wake};
use places::PlacesService;
use thumbs::Thumbs;
use watcher::WatchService;

/// State of a one-shot auxiliary listing (breadcrumb dropdowns, completion, Miller).
#[derive(Clone, Debug)]
pub enum AuxState {
    Loading(Instant),
    Ready { entries: Arc<Vec<EntryInfo>>, at: Instant },
    Failed(FsError),
}

pub struct Services {
    pub mailbox: Mailbox,
    pub lister: Lister,
    pub watcher: WatchService,
    pub thumbs: Thumbs,
    pub places: PlacesService,
    pub aux: HashMap<PathBuf, AuxState>,
    /// MCP server of this window (named pipe / HTTP, approvals; roadmap A3).
    pub mcp: mcp_bridge::McpBridge,
}

/// Aux listings older than this are refreshed on the next request.
const AUX_TTL: Duration = Duration::from_secs(5);

impl Services {
    pub fn new(ctx: &Context, watch_debounce: Duration) -> Self {
        let c = ctx.clone();
        let wake: Wake = Arc::new(move || c.request_repaint());
        let mailbox = Mailbox::new(wake.clone());
        let postman = mailbox.postman();
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        Services {
            lister: Lister::new(postman.clone(), 3),
            watcher: WatchService::new(watch_debounce, wake),
            thumbs: Thumbs::new(ctx.clone(), cores.saturating_sub(1).clamp(1, 4)),
            places: PlacesService::new(postman),
            mailbox,
            aux: HashMap::new(),
            mcp: Default::default(),
        }
    }

    /// Entries of `path` from the aux cache, requesting a (re)load when missing/stale.
    pub fn aux_entries(&mut self, path: &Path) -> Option<Arc<Vec<EntryInfo>>> {
        let now = Instant::now();
        match self.aux.get(path) {
            Some(AuxState::Ready { entries, at }) => {
                let e = entries.clone();
                if now.duration_since(*at) > AUX_TTL {
                    self.aux.insert(path.to_path_buf(), AuxState::Ready { entries: e.clone(), at: now });
                    self.lister.aux(path);
                }
                Some(e)
            }
            Some(AuxState::Loading(_)) | Some(AuxState::Failed(_)) => None,
            None => {
                self.aux.insert(path.to_path_buf(), AuxState::Loading(now));
                self.lister.aux(path);
                None
            }
        }
    }

    pub fn aux_state(&self, path: &Path) -> Option<&AuxState> {
        self.aux.get(path)
    }

    /// Forget failed aux entries so they can be retried.
    pub fn aux_forget_failed(&mut self) {
        self.aux.retain(|_, s| !matches!(s, AuxState::Failed(_)));
        if self.aux.len() > 256 {
            self.aux.clear();
        }
    }
}
