//! Script loading (hot reload), the process-wide current [`ScriptSet`], script columns
//! (values computed off-thread and cached) and script previewers.
//!
//! The loader thread polls the scripts folder once a second (a handful of `stat`s) and
//! reloads when a file changes or `[scripting]` changes. Column values are computed on
//! one worker thread; the UI only looks them up in a cache keyed by path, size and
//! modification time, so painting never runs script code.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::RwLock;
use soyuz_core::config::Scripting;
use soyuz_core::{Column, EntryInfo};
use soyuz_script::{Limits, ScriptSet};

use crate::services::mailbox::Wake;

static CURRENT: RwLock<Option<Arc<ScriptSet>>> = RwLock::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static WAKE: RwLock<Option<Wake>> = RwLock::new(None);

/// The loaded scripts (`None` before the first load).
pub fn current() -> Option<Arc<ScriptSet>> {
    CURRENT.read().clone()
}

/// Bumped on every (re)load; caches of script data compare it.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::SeqCst)
}

pub(crate) fn set_current(s: Arc<ScriptSet>) {
    *CURRENT.write() = Some(s);
    GENERATION.fetch_add(1, Ordering::SeqCst);
    CELLS_VERSION.fetch_add(1, Ordering::SeqCst);
}

pub(crate) fn set_wake(w: Wake) {
    *WAKE.write() = Some(w);
}

fn wake() {
    if let Some(w) = WAKE.read().as_ref() {
        w();
    }
}

enum LoaderCmd {
    Config(Scripting),
    Reload,
}

/// The hot-reload thread.
pub struct Loader {
    tx: Sender<LoaderCmd>,
    pub rx: Receiver<Arc<ScriptSet>>,
}

impl Loader {
    pub fn start(dir: PathBuf, config: Scripting, wake: Wake) -> Loader {
        let (tx, crx) = unbounded::<LoaderCmd>();
        let (stx, rx) = unbounded();
        let _ = std::thread::Builder::new().name("soyuz-scripts".into()).spawn(move || {
            let mut config = config;
            let mut sig = None;
            loop {
                let mut force = false;
                match crx.recv_timeout(Duration::from_secs(1)) {
                    Ok(LoaderCmd::Config(c)) => {
                        force = c != config;
                        config = c;
                    }
                    Ok(LoaderCmd::Reload) => force = true,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
                }
                let now = soyuz_script::script::signature(&dir);
                if !force && sig.as_ref() == Some(&now) {
                    continue;
                }
                sig = Some(now);
                let limits = Limits::from_config(&config);
                let set = if config.enabled {
                    ScriptSet::load_dir(&dir, limits, &config.disabled)
                } else {
                    ScriptSet::empty(limits).with_dir(&dir)
                };
                for s in &set.scripts {
                    if let Some(e) = &s.error {
                        log::warn!("{}: {e}", s.path.display());
                    }
                }
                if stx.send(Arc::new(set)).is_err() {
                    return;
                }
                wake();
            }
        });
        Loader { tx, rx }
    }

    pub fn configure(&self, c: Scripting) {
        let _ = self.tx.send(LoaderCmd::Config(c));
    }

    pub fn reload(&self) {
        let _ = self.tx.send(LoaderCmd::Reload);
    }
}

// --- columns ---

/// Header label of a script column (`None`: no provider in that slot).
pub fn column_label(col: Column) -> Option<String> {
    let slot = col.script_slot()?;
    let set = current()?;
    set.columns().get(slot).map(|(_, c)| c.title.clone())
}

/// Default width of a script column's provider.
pub fn column_width(col: Column) -> Option<f32> {
    let slot = col.script_slot()?;
    current()?.columns().get(slot).map(|(_, c)| c.width)
}

type CellKey = (usize, PathBuf, u64, Option<SystemTime>);

struct CellReq {
    generation: u64,
    slot: usize,
    entry: EntryInfo,
}

struct CellDone {
    generation: u64,
    key: CellKey,
    value: Option<String>,
}

struct Cells {
    generation: u64,
    values: HashMap<CellKey, Option<String>>,
    pending: HashSet<CellKey>,
    tx: Sender<CellReq>,
    rx: Receiver<CellDone>,
}

const MAX_CELLS: usize = 50_000;
const MAX_PENDING: usize = 4_000;

fn cell_worker() -> (Sender<CellReq>, Receiver<CellDone>) {
    let (tx, rx) = unbounded::<CellReq>();
    let (dtx, drx) = unbounded::<CellDone>();
    let _ = std::thread::Builder::new().name("soyuz-script-columns".into()).spawn(move || {
        while let Ok(req) = rx.recv() {
            if req.generation != GENERATION.load(Ordering::SeqCst) {
                continue;
            }
            let Some(set) = current() else { continue };
            let value = match set.column_value(req.slot, &req.entry) {
                Ok(v) => v,
                Err(e) => {
                    log::debug!("script column {}: {e}", req.slot + 1);
                    Some(format!("⚠ {}", e.message))
                }
            };
            let e = req.entry;
            let key = (req.slot, e.path, e.size, e.modified);
            if dtx.send(CellDone { generation: req.generation, key, value }).is_err() {
                return;
            }
            if rx.is_empty() {
                wake();
            }
        }
    });
    (tx, drx)
}

thread_local! {
    static CELLS: RefCell<Option<Cells>> = const { RefCell::new(None) };
}

/// Bumped whenever script column values arrive (panes sorted by one re-sort then).
static CELLS_VERSION: AtomicU64 = AtomicU64::new(0);

/// Take the finished values off the worker's channel.
fn drain(cells: &mut Cells, generation: u64) {
    let mut any = false;
    for d in cells.rx.try_iter() {
        cells.pending.remove(&d.key);
        if d.generation == generation {
            cells.values.insert(d.key, d.value);
            any = true;
        }
    }
    if any {
        CELLS_VERSION.fetch_add(1, Ordering::SeqCst);
    }
}

/// Changes whenever new script column values are known (it collects them first).
pub fn cells_version() -> u64 {
    CELLS.with_borrow_mut(|c| {
        if let Some(cells) = c {
            drain(cells, GENERATION.load(Ordering::SeqCst));
        }
    });
    CELLS_VERSION.load(Ordering::SeqCst)
}

/// Value of a script column for `e` (empty while it is being computed).
pub fn cell_text(col: Column, e: &EntryInfo) -> String {
    let Some(slot) = col.script_slot() else { return String::new() };
    CELLS.with_borrow_mut(|c| {
        let cells = c.get_or_insert_with(|| {
            let (tx, rx) = cell_worker();
            Cells { generation: 0, values: HashMap::new(), pending: HashSet::new(), tx, rx }
        });
        let generation = GENERATION.load(Ordering::SeqCst);
        if cells.generation != generation {
            cells.generation = generation;
            cells.values.clear();
            cells.pending.clear();
        }
        drain(cells, generation);
        if cells.values.len() > MAX_CELLS {
            cells.values.clear();
        }
        let key = (slot, e.path.clone(), e.size, e.modified);
        if let Some(v) = cells.values.get(&key) {
            return v.clone().unwrap_or_default();
        }
        if !cells.pending.contains(&key) && cells.pending.len() < MAX_PENDING {
            cells.pending.insert(key);
            let _ = cells.tx.send(CellReq { generation, slot, entry: e.clone() });
        }
        String::new()
    })
}

// --- previewers ---

/// What a script previewer produced for `path` (runs on a preview worker thread).
pub fn preview(path: &Path) -> Option<Result<soyuz_script::Preview, soyuz_script::ScriptError>> {
    let set = current()?;
    let name = path.file_name()?.to_string_lossy().into_owned();
    let ext = soyuz_script::api::name_ext(&name)?.to_string();
    set.previewer_for(&ext)?;
    match set.preview(path) {
        Ok(Some(p)) => Some(Ok(p)),
        Ok(None) => None,
        Err(e) => Some(Err(e)),
    }
}
