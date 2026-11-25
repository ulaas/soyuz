//! One open document for the viewer: worker threads that render regions, page bitmaps
//! and thumbnails, extract text and search, reporting through [`Event`]s.
//!
//! * Opening (file read, parse, outline, links) runs on a worker, never the caller's
//!   thread; [`Event::Opened`] or [`Event::Failed`] follows.
//! * Each worker owns its own parsed document (from the shared file bytes) and a
//!   stack-local render cache, and reloads both every [`RECYCLE_PAGES`] distinct pages
//!   (pages cache decoded content inside the document) and after a panic.
//! * Jobs run by priority ([`prio`], lower first). Region jobs carry the generation that
//!   was current when they were requested; once [`PdfSession::set_generation`] moves on
//!   (zoom, rotation), older region jobs are dropped when they reach the front.
//! * A watchdog abandons a job past its time budget ([`Event::PageFailed`] with
//!   [`PdfError::TimedOut`]) and starts a replacement worker; the abandoned thread
//!   finishes on its own and exits.
//! * Textures are uploaded on the workers through the frontend's [`Uploader`]
//!   (premultiplied pixels), then the frontend is woken.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use parking_lot::{Condvar, Mutex};

use super::hayro_backend::{self, RECYCLE_PAGES};
use super::nav::{self, Link, Outline};
use super::text::{self, Match, PageText, Query};
use super::tiles::{PageKey, TileKey};
use super::{DocInfo, MAX_REGION_PX, PageGeom, PdfError, PdfSrc, RenderOpts, guard};
use crate::host::Wake;
use crate::services::preview_gfx::{Pixels, Tex, Uploader};

/// Job priorities (lower runs first). Add a small offset within a band to order jobs of
/// the same kind (e.g. tile distance from the view centre).
pub mod prio {
    pub const VISIBLE_REGION: u32 = 0;
    pub const VISIBLE_BASE: u32 = 1_000;
    pub const VISIBLE_TEXT: u32 = 2_000;
    pub const PREFETCH: u32 = 3_000;
    pub const THUMB: u32 = 4_000;
    pub const FIND: u32 = 5_000;
}

/// Session settings (plain values: the caller maps `[pdf]` config onto them).
#[derive(Clone, Debug)]
pub struct SessionCfg {
    /// Worker threads; 0 = `min(4, cores − 1)`, at least 1.
    pub threads: usize,
    /// Budget of one job (and of opening).
    pub timeout: Duration,
    /// File size cap in bytes.
    pub max_bytes: u64,
    /// Annotations and system fonts (`invert` is per job: it is part of the keys).
    pub opts: RenderOpts,
    /// Long edge of a base-layer page bitmap ([`Job::Base`]).
    pub base_px: u32,
    /// Long edge of a sidebar thumbnail ([`Job::Thumb`]).
    pub thumb_px: u32,
    /// Pages of extracted text kept (least recently used dropped).
    pub text_pages: usize,
}

impl Default for SessionCfg {
    fn default() -> Self {
        SessionCfg {
            threads: 0,
            timeout: Duration::from_secs(20),
            max_bytes: 1 << 30,
            opts: RenderOpts::default(),
            base_px: 1400,
            thumb_px: 192,
            text_pages: 200,
        }
    }
}

/// Work the viewer asks for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Job {
    /// One sharp tile.
    Region(TileKey),
    /// The page fitted into `base_px` (blurry base layer under the tiles).
    Base(PageKey),
    /// The page fitted into `thumb_px`.
    Thumb(PageKey),
    /// Positioned text of a page.
    Text(u32),
    /// Start a search sweep from page 0 (same as [`PdfSession::find`]).
    Find(Query),
}

impl Job {
    fn page(&self) -> u32 {
        match self {
            Job::Region(k) => k.page,
            Job::Base(k) | Job::Thumb(k) => k.page,
            Job::Text(p) => *p,
            Job::Find(_) => 0,
        }
    }
}

/// What the workers report. [`PdfSession::poll`] hands them out in arrival order.
#[derive(Debug)]
pub enum Event {
    /// The document is open: [`PdfSession::info`], `geoms`, `outline` and `links` are set.
    Opened(Arc<DocInfo>),
    /// Opening failed (password, invalid, too large, timed out…).
    Failed(PdfError),
    Region {
        key: TileKey,
        /// The generation the job was requested under.
        generation: u64,
        tex: Tex,
    },
    Base {
        key: PageKey,
        tex: Tex,
    },
    Thumb {
        key: PageKey,
        tex: Tex,
    },
    Text(u32, Arc<PageText>),
    /// One more page searched: its matches (possibly none). `searched == total` ends the
    /// sweep.
    FindProgress {
        id: u64,
        page: u32,
        matches: Vec<Match>,
        searched: u32,
        total: u32,
    },
    PageFailed {
        job: Job,
        error: PdfError,
    },
}

/// Internal work items.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Work {
    Open,
    Job(Job),
    /// Step `n` of find sweep `id`.
    FindStep {
        id: u64,
        n: u32,
    },
    #[cfg(test)]
    Test(TestOp),
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum TestOp {
    SleepMs(u64),
    Panic,
}

impl Work {
    fn page(&self) -> u32 {
        match self {
            Work::Job(j) => j.page(),
            _ => u32::MAX,
        }
    }

    /// The job to name in a failure event.
    fn public(&self) -> Job {
        match self {
            Work::Job(j) => j.clone(),
            _ => Job::Text(u32::MAX),
        }
    }
}

/// The priority queue: ordered by (priority, arrival), deduplicated by work.
#[derive(Default)]
struct Queue {
    order: BTreeMap<(u32, u64), (Work, u64)>,
    index: HashMap<Work, (u32, u64)>,
    seq: u64,
}

impl Queue {
    /// Queue `w` (or raise its priority / refresh its generation if already queued).
    fn push(&mut self, w: Work, prio: u32, generation: u64) {
        if let Some(&(p, s)) = self.index.get(&w) {
            if p <= prio {
                if let Some(e) = self.order.get_mut(&(p, s)) {
                    e.1 = e.1.max(generation);
                }
                return;
            }
            self.order.remove(&(p, s));
        }
        self.seq += 1;
        self.index.insert(w.clone(), (prio, self.seq));
        self.order.insert((prio, self.seq), (w, generation));
    }

    /// The next job; region jobs from before `current` generation are dropped.
    fn pop(&mut self, current: u64) -> Option<(Work, u64)> {
        while let Some((_, (w, g))) = self.order.pop_first() {
            self.index.remove(&w);
            if matches!(w, Work::Job(Job::Region(_))) && g < current {
                continue;
            }
            return Some((w, g));
        }
        None
    }

    fn retain(&mut self, f: impl Fn(&Work) -> bool) {
        self.order.retain(|_, (w, _)| f(w));
        let order = &self.order;
        self.index.retain(|_, k| order.contains_key(k));
    }

    fn len(&self) -> usize {
        self.order.len()
    }
}

/// The open document as the workers share it.
struct Doc {
    bytes: Arc<[u8]>,
    password: Option<String>,
    geoms: Arc<[PageGeom]>,
}

/// What [`Event::Opened`] publishes.
#[derive(Clone)]
struct Opened {
    info: Arc<DocInfo>,
    geoms: Arc<[PageGeom]>,
    outline: Arc<[Outline]>,
    links: Arc<[Vec<Link>]>,
}

struct FindState {
    id: u64,
    query: Query,
    start: u32,
}

#[derive(Default)]
struct State {
    queue: Queue,
    inflight: HashSet<Work>,
    closed: bool,
    doc: Option<Arc<Doc>>,
    find: Option<FindState>,
    find_seq: u64,
    texts: HashMap<u32, (Arc<PageText>, u64)>,
    text_tick: u64,
}

/// One worker's watchdog record.
struct Slot {
    busy: Mutex<Option<(Instant, Work)>>,
    abandoned: AtomicBool,
}

struct Shared {
    state: Mutex<State>,
    cv: Condvar,
    generation: AtomicU64,
    slots: Mutex<Vec<Arc<Slot>>>,
    opened: Mutex<Option<Opened>>,
    tx: Sender<Event>,
    wake: Wake,
    up: Uploader,
    cfg: SessionCfg,
    name: String,
}

impl Shared {
    fn send(&self, e: Event) {
        let _ = self.tx.send(e);
        (self.wake)();
    }

    fn closed(&self) -> bool {
        self.state.lock().closed
    }

    /// Block for the next job and mark `slot` busy with it. `None` once closed.
    fn next(&self, slot: &Slot) -> Option<(Work, u64)> {
        let mut st = self.state.lock();
        loop {
            if st.closed {
                return None;
            }
            if let Some((w, g)) = st.queue.pop(self.generation.load(Ordering::Acquire)) {
                st.inflight.insert(w.clone());
                *slot.busy.lock() = Some((Instant::now(), w.clone()));
                return Some((w, g));
            }
            self.cv.wait(&mut st);
        }
    }

    /// Job done: clear `slot`. `false` when the watchdog abandoned it meanwhile (the
    /// caller then drops its result and exits).
    fn finish(&self, slot: &Slot, w: &Work) -> bool {
        let mut busy = slot.busy.lock();
        if slot.abandoned.load(Ordering::Acquire) {
            return false;
        }
        *busy = None;
        drop(busy);
        self.state.lock().inflight.remove(w);
        true
    }

    fn push(&self, w: Work, prio: u32) {
        let mut st = self.state.lock();
        if st.closed || st.inflight.contains(&w) {
            return;
        }
        st.queue.push(w, prio, self.generation.load(Ordering::Acquire));
        drop(st);
        self.cv.notify_one();
    }

    fn cached_text(&self, page: u32) -> Option<Arc<PageText>> {
        let mut st = self.state.lock();
        st.text_tick += 1;
        let tick = st.text_tick;
        st.texts.get_mut(&page).map(|e| {
            e.1 = tick;
            e.0.clone()
        })
    }

    fn store_text(&self, page: u32, t: Arc<PageText>) {
        let mut st = self.state.lock();
        st.text_tick += 1;
        let tick = st.text_tick;
        st.texts.insert(page, (t, tick));
        let cap = self.cfg.text_pages.max(1);
        while st.texts.len() > cap {
            let Some(oldest) = st.texts.iter().min_by_key(|(_, (_, used))| *used).map(|(p, _)| *p) else { break };
            st.texts.remove(&oldest);
        }
    }

    fn new_slot(&self) -> Arc<Slot> {
        let slot = Arc::new(Slot { busy: Mutex::new(None), abandoned: AtomicBool::new(false) });
        self.slots.lock().push(slot.clone());
        slot
    }

    fn spawn_worker(self: &Arc<Self>, first: Option<hayro::hayro_syntax::Pdf>) {
        let slot = self.new_slot();
        let me = self.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("soyuz-pdf {}", self.name))
            .stack_size(guard::STACK)
            .spawn(move || me.work_loop(&slot, first));
        if let Err(e) = spawned {
            log::warn!("PDF worker thread: {e}");
        }
    }

    /// A worker: render jobs with its own document until the session closes.
    fn work_loop(self: &Arc<Self>, slot: &Arc<Slot>, mut first: Option<hayro::hayro_syntax::Pdf>) {
        let Some(doc) = self.state.lock().doc.clone() else { return };
        let plain = hayro_backend::settings(&RenderOpts { invert: false, ..self.cfg.opts });
        loop {
            if self.closed() {
                return;
            }
            let pdf = match first.take() {
                Some(p) => p,
                None => match hayro_backend::load(doc.bytes.clone(), doc.password.as_deref()) {
                    Ok(p) => p,
                    Err(e) => {
                        log::warn!("PDF worker could not reload the document: {e:?}");
                        return;
                    }
                },
            };
            let cache = hayro::RenderCache::new();
            let mut pages = HashSet::new();
            loop {
                let Some((work, generation)) = self.next(slot) else { return };
                let recycle = pages.insert(work.page()) && pages.len() > RECYCLE_PAGES;
                let r = guard::catch(|| self.run(&doc, &pdf, &cache, &plain, &work, generation));
                if !self.finish(slot, &work) {
                    return;
                }
                let panicked = matches!(r, Err(PdfError::Panicked));
                match r.and_then(|r| r) {
                    Ok(Some(e)) => self.send(e),
                    Ok(None) => {}
                    Err(error) => self.send(Event::PageFailed { job: work.public(), error }),
                }
                if panicked || recycle {
                    break;
                }
            }
        }
    }

    /// Run one job on this worker's document.
    fn run<'a>(
        &self,
        doc: &Doc,
        pdf: &'a hayro::hayro_syntax::Pdf,
        cache: &hayro::RenderCache<'a>,
        is: &hayro::hayro_interpret::InterpreterSettings,
        work: &Work,
        generation: u64,
    ) -> Result<Option<Event>, PdfError> {
        let page = |p: u32| pdf.pages().get(p as usize).ok_or_else(|| PdfError::Other(format!("no page {}", p + 1)));
        let upload = |name: String, mut bmp: super::Bitmap, invert: bool| {
            if invert {
                super::invert_lightness(&mut bmp.rgba);
            }
            let size = [bmp.w as usize, bmp.h as usize];
            Tex::upload(&self.up, name, Pixels { size, rgba: &bmp.rgba, premultiplied: true })
        };
        match work {
            Work::Open | Work::Job(Job::Find(_)) => Ok(None),
            Work::Job(Job::Region(key)) => {
                let geom = doc.geoms.get(key.page as usize).ok_or_else(|| PdfError::Other("no page".into()))?;
                let r = key.region(geom).ok_or_else(|| PdfError::Other("tile past the page".into()))?;
                let bmp = hayro_backend::render_region(page(key.page)?, cache, is, &r, MAX_REGION_PX)?;
                let name = format!("pdf p{} t{},{} @{}", key.page, key.tx, key.ty, key.scale_milli);
                Ok(Some(Event::Region { key: *key, generation, tex: upload(name, bmp, key.invert) }))
            }
            Work::Job(Job::Base(key)) | Work::Job(Job::Thumb(key)) => {
                let thumb = matches!(work, Work::Job(Job::Thumb(_)));
                let px = if thumb { self.cfg.thumb_px } else { self.cfg.base_px }.max(16);
                let bmp = hayro_backend::render_fit_with(page(key.page)?, cache, is, px, px, key.rot)?;
                let tex =
                    upload(format!("pdf p{} {}", key.page, if thumb { "thumb" } else { "base" }), bmp, key.invert);
                Ok(Some(if thumb { Event::Thumb { key: *key, tex } } else { Event::Base { key: *key, tex } }))
            }
            Work::Job(Job::Text(p)) => {
                let t = match self.cached_text(*p) {
                    Some(t) => t,
                    None => {
                        let t = Arc::new(hayro_backend::text(page(*p)?, is));
                        self.store_text(*p, t.clone());
                        t
                    }
                };
                Ok(Some(Event::Text(*p, t)))
            }
            Work::FindStep { id, n } => {
                let total = doc.geoms.len() as u32;
                let (query, start) = {
                    let st = self.state.lock();
                    match &st.find {
                        Some(f) if f.id == *id => (f.query.clone(), f.start),
                        _ => return Ok(None),
                    }
                };
                let p = (start + n) % total.max(1);
                let t = match self.cached_text(p) {
                    Some(t) => t,
                    None => {
                        let t = Arc::new(hayro_backend::text(page(p)?, is));
                        self.store_text(p, t.clone());
                        t
                    }
                };
                let matches = text::find(&t, &query);
                if n + 1 < total {
                    self.push(Work::FindStep { id: *id, n: n + 1 }, prio::FIND);
                }
                Ok(Some(Event::FindProgress { id: *id, page: p, matches, searched: n + 1, total }))
            }
            #[cfg(test)]
            Work::Test(TestOp::SleepMs(ms)) => {
                std::thread::sleep(Duration::from_millis(*ms));
                Ok(None)
            }
            #[cfg(test)]
            Work::Test(TestOp::Panic) => panic!("test panic in a PDF worker (expected)"),
        }
    }

    /// Abandon jobs past the budget and replace their workers.
    fn watchdog(self: Arc<Self>) {
        let tick = (self.cfg.timeout / 4).clamp(Duration::from_millis(10), Duration::from_millis(250));
        loop {
            std::thread::sleep(tick);
            if self.closed() {
                return;
            }
            let slots: Vec<Arc<Slot>> = self.slots.lock().clone();
            for slot in slots {
                let mut busy = slot.busy.lock();
                let Some((since, _)) = &*busy else { continue };
                if since.elapsed() < self.cfg.timeout {
                    continue;
                }
                let Some((_, work)) = busy.take() else { continue };
                slot.abandoned.store(true, Ordering::Release);
                drop(busy);
                self.slots.lock().retain(|s| !Arc::ptr_eq(s, &slot));
                self.state.lock().inflight.remove(&work);
                log::warn!("PDF job {work:?} ran past {:?}; abandoned", self.cfg.timeout);
                if work == Work::Open {
                    self.send(Event::Failed(PdfError::TimedOut));
                    return;
                }
                self.send(Event::PageFailed { job: work.public(), error: PdfError::TimedOut });
                self.spawn_worker(None);
            }
        }
    }

    /// The opening worker: read, parse, publish, start the others, then work.
    fn open(self: Arc<Self>, src: PdfSrc, password: Option<String>, slot: Arc<Slot>) {
        let opened = guard::catch(|| -> Result<_, PdfError> {
            let bytes = src.shared(self.cfg.max_bytes)?;
            let pdf = hayro_backend::load(bytes.clone(), password.as_deref())?;
            let mut info = hayro_backend::info(&pdf, &bytes);
            info.encrypted |= password.is_some();
            let geoms: Arc<[PageGeom]> = hayro_backend::geoms(&pdf).into();
            let outline: Arc<[Outline]> = nav::outline(&pdf).into();
            let links: Arc<[Vec<Link>]> = nav::links(&pdf).into();
            Ok((pdf, bytes, Opened { info: Arc::new(info), geoms, outline, links }))
        })
        .and_then(|r| r);
        if !self.finish(&slot, &Work::Open) {
            return;
        }
        self.slots.lock().retain(|s| !Arc::ptr_eq(s, &slot));
        match opened {
            Err(e) => self.send(Event::Failed(e)),
            Ok((pdf, bytes, o)) => {
                let doc = Doc { bytes, password, geoms: o.geoms.clone() };
                self.state.lock().doc = Some(Arc::new(doc));
                let info = o.info.clone();
                *self.opened.lock() = Some(o);
                self.send(Event::Opened(info));
                let n = self.cfg.threads;
                let n = if n == 0 {
                    std::thread::available_parallelism().map_or(2, |c| c.get()).saturating_sub(1).clamp(1, 4)
                } else {
                    n
                };
                for _ in 1..n {
                    self.spawn_worker(None);
                }
                let me = self.clone();
                let slot = self.new_slot();
                me.work_loop(&slot, Some(pdf));
            }
        }
    }
}

/// An open document and its workers. Dropping it stops them (a job in progress finishes
/// first).
pub struct PdfSession {
    shared: Arc<Shared>,
    rx: Receiver<Event>,
    opened: Option<Opened>,
}

impl PdfSession {
    /// Start opening `src` (with `password`, if one was typed). Returns at once; watch
    /// [`PdfSession::poll`] for [`Event::Opened`] / [`Event::Failed`].
    pub fn open(src: PdfSrc, password: Option<String>, cfg: SessionCfg, wake: Wake, up: Uploader) -> PdfSession {
        let (tx, rx) = crossbeam_channel::unbounded();
        let name = match &src {
            PdfSrc::File(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            PdfSrc::Mem(_) => "memory".into(),
        };
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            cv: Condvar::new(),
            generation: AtomicU64::new(0),
            slots: Mutex::new(Vec::new()),
            opened: Mutex::new(None),
            tx,
            wake,
            up,
            cfg,
            name,
        });
        let slot = shared.new_slot();
        *slot.busy.lock() = Some((Instant::now(), Work::Open));
        let opener = shared.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("soyuz-pdf {}", shared.name))
            .stack_size(guard::STACK)
            .spawn(move || opener.open(src, password, slot));
        if let Err(e) = spawned {
            shared.send(Event::Failed(PdfError::Other(e.to_string())));
        } else {
            let dog = shared.clone();
            let _ = std::thread::Builder::new().name("soyuz-pdf watchdog".into()).spawn(move || dog.watchdog());
        }
        PdfSession { shared, rx, opened: None }
    }

    /// Open the file at `path` with the password typed for it this session.
    pub fn open_path(path: PathBuf, cfg: SessionCfg, wake: Wake, up: Uploader) -> PdfSession {
        let password = super::preview_pdf::password_for(&path);
        PdfSession::open(PdfSrc::File(path), password, cfg, wake, up)
    }

    /// Events since the last call.
    pub fn poll(&mut self) -> Vec<Event> {
        let events: Vec<Event> = self.rx.try_iter().collect();
        if self.opened.is_none() && events.iter().any(|e| matches!(e, Event::Opened(_))) {
            self.opened = self.shared.opened.lock().clone();
        }
        events
    }

    /// Document facts (after [`Event::Opened`]).
    pub fn info(&self) -> Option<&Arc<DocInfo>> {
        self.opened.as_ref().map(|o| &o.info)
    }

    pub fn page_count(&self) -> u32 {
        self.geoms().len() as u32
    }

    /// Every page's size (empty until opened).
    pub fn geoms(&self) -> &[PageGeom] {
        self.opened.as_ref().map_or(&[], |o| &o.geoms)
    }

    /// The outline (empty until opened or without one).
    pub fn outline(&self) -> &[Outline] {
        self.opened.as_ref().map_or(&[], |o| &o.outline)
    }

    /// Links of `page` (rendered page points).
    pub fn links(&self, page: u32) -> &[Link] {
        self.opened.as_ref().and_then(|o| o.links.get(page as usize)).map_or(&[], Vec::as_slice)
    }

    /// Queue `job` at priority `prio` (see [`prio`]); a job already queued keeps the
    /// better priority, one in progress is not queued again.
    pub fn request(&self, job: Job, prio: u32) {
        match job {
            Job::Find(q) => {
                self.find(q, 0);
            }
            job => self.shared.push(Work::Job(job), prio),
        }
    }

    /// Start searching for `q` from page `from` (wrapping), replacing any running sweep.
    /// Returns the sweep id carried by its [`Event::FindProgress`] events.
    pub fn find(&self, q: Query, from: u32) -> u64 {
        let id = {
            let mut st = self.shared.state.lock();
            st.find_seq += 1;
            let id = st.find_seq;
            st.find = Some(FindState { id, query: q, start: from });
            st.queue.retain(|w| !matches!(w, Work::FindStep { .. }));
            id
        };
        self.shared.push(Work::FindStep { id, n: 0 }, prio::FIND);
        id
    }

    /// Stop the running search sweep.
    pub fn cancel_find(&self) {
        let mut st = self.shared.state.lock();
        st.find = None;
        st.queue.retain(|w| !matches!(w, Work::FindStep { .. }));
    }

    /// Drop queued jobs `f` selects (e.g. pages scrolled far away).
    pub fn cancel_where(&self, f: impl Fn(&Job) -> bool) {
        self.shared.state.lock().queue.retain(|w| !matches!(w, Work::Job(j) if f(j)));
    }

    /// Start a new generation: queued region jobs from older generations are dropped.
    pub fn set_generation(&self, g: u64) {
        self.shared.generation.store(g, Ordering::Release);
    }

    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Acquire)
    }

    /// Text already extracted for `page`.
    pub fn cached_text(&self, page: u32) -> Option<Arc<PageText>> {
        self.shared.cached_text(page)
    }

    /// Jobs queued or running.
    pub fn pending(&self) -> usize {
        let st = self.shared.state.lock();
        st.queue.len() + st.inflight.len()
    }

    #[cfg(test)]
    fn push_test(&self, op: TestOp) {
        self.shared.push(Work::Test(op), 0);
    }
}

impl Drop for PdfSession {
    fn drop(&mut self) {
        self.shared.state.lock().closed = true;
        self.shared.cv.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures;
    use super::*;
    use crate::services::preview_gfx::no_upload;

    fn session(bytes: Vec<u8>, password: Option<&str>, cfg: SessionCfg) -> PdfSession {
        PdfSession::open(
            PdfSrc::Mem(bytes.into()),
            password.map(str::to_string),
            cfg,
            crate::host::no_wake(),
            no_upload(),
        )
    }

    /// Poll until `done` says so (or panic after 30 s).
    fn until(s: &mut PdfSession, mut done: impl FnMut(&Event) -> bool) -> Vec<Event> {
        let t = Instant::now();
        let mut all = Vec::new();
        loop {
            for e in s.poll() {
                let d = done(&e);
                all.push(e);
                if d {
                    return all;
                }
            }
            assert!(t.elapsed() < Duration::from_secs(30), "timed out; got {all:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn opened(s: &mut PdfSession) {
        let ev = until(s, |e| matches!(e, Event::Opened(_) | Event::Failed(_)));
        assert!(matches!(ev.last(), Some(Event::Opened(_))), "{ev:?}");
    }

    #[test]
    fn queue_orders_dedups_and_drops_stale_regions() {
        let mut q = Queue::default();
        let region =
            |tx| Work::Job(Job::Region(TileKey { page: 0, scale_milli: 1000, rot: 0, tx, ty: 0, invert: false }));
        q.push(Work::Job(Job::Text(3)), prio::VISIBLE_TEXT, 0);
        q.push(region(0), prio::VISIBLE_REGION, 0);
        q.push(region(1), prio::VISIBLE_REGION + 1, 1);
        q.push(Work::Job(Job::Text(3)), prio::PREFETCH, 0);
        assert_eq!(q.len(), 3, "deduplicated");
        q.push(Work::Job(Job::Text(3)), 0, 0);
        // Generation 1 is current: the region from generation 0 is dropped.
        assert_eq!(q.pop(1).map(|w| w.0), Some(Work::Job(Job::Text(3))), "raised to the front");
        assert_eq!(q.pop(1).map(|w| w.0), Some(region(1)));
        assert_eq!(q.pop(1), None);
        assert_eq!(q.index.len(), 0);
    }

    #[test]
    fn opens_renders_and_extracts() {
        let cfg = SessionCfg { threads: 2, base_px: 400, thumb_px: 100, ..Default::default() };
        let mut s = session(fixtures::with_nav(), None, cfg);
        opened(&mut s);
        assert_eq!(s.page_count(), 3);
        assert_eq!(s.info().unwrap().pages, 3);
        assert_eq!(s.outline().len(), 3);
        assert_eq!(s.links(0).len(), 3);
        assert!(s.links(9).is_empty());
        let pk = PageKey { page: 0, rot: 0, invert: false };
        s.request(Job::Base(pk), prio::VISIBLE_BASE);
        s.request(Job::Thumb(PageKey { rot: 1, ..pk }), prio::THUMB);
        let tile = TileKey { page: 1, scale_milli: 2000, rot: 0, tx: 1, ty: 0, invert: true };
        s.set_generation(5);
        s.request(Job::Region(tile), prio::VISIBLE_REGION);
        s.request(Job::Text(1), prio::VISIBLE_TEXT);
        let mut seen = 0;
        let ev = until(&mut s, |e| {
            if !matches!(e, Event::PageFailed { .. }) {
                seen += 1;
            }
            seen == 4 || matches!(e, Event::PageFailed { .. })
        });
        for e in &ev {
            match e {
                Event::Base { key, tex } => {
                    assert_eq!(*key, pk);
                    assert_eq!(tex.size, [309, 400]);
                }
                Event::Thumb { tex, .. } => assert_eq!(tex.size, [100, 77], "rotated thumbnail"),
                Event::Region { key, generation, tex } => {
                    assert_eq!((*key, *generation), (tile, 5));
                    assert_eq!(tex.size, [1224 - 1024, 1024]);
                }
                Event::Text(1, t) => assert_eq!(t.text, "Chapter two"),
                e => panic!("unexpected {e:?}"),
            }
        }
        assert!(s.cached_text(1).is_some());
        // A bad tile is reported, not fatal.
        s.request(Job::Region(TileKey { tx: 9, ..tile }), 0);
        let ev = until(&mut s, |e| matches!(e, Event::PageFailed { .. }));
        assert!(matches!(ev.last(), Some(Event::PageFailed { job: Job::Region(_), .. })));
    }

    #[test]
    fn find_sweeps_every_page_from_the_start_page() {
        let mut s = session(fixtures::many_pages(7), None, SessionCfg { threads: 1, ..Default::default() });
        opened(&mut s);
        let id = s.find(Query { text: "OF THE sweep".into(), ..Default::default() }, 5);
        let mut pages = Vec::new();
        let ev = until(&mut s, |e| matches!(e, Event::FindProgress { searched, total, .. } if searched == total));
        for e in ev {
            if let Event::FindProgress { id: i, page, matches, total, .. } = e {
                assert_eq!((i, total), (id, 7));
                assert_eq!(matches.len(), 1);
                pages.push(page);
            }
        }
        assert_eq!(pages, [5, 6, 0, 1, 2, 3, 4]);
        // A new search replaces the old one.
        let a = s.find(Query { text: "Page 3".into(), ..Default::default() }, 0);
        let b = s.find(Query { text: "Page 4".into(), whole_word: true, ..Default::default() }, 0);
        assert!(b > a);
        let ev = until(
            &mut s,
            |e| matches!(e, Event::FindProgress { id, searched, total, .. } if *id == b && searched == total),
        );
        let hits: u32 = ev
            .iter()
            .filter_map(|e| match e {
                Event::FindProgress { id, matches, .. } if *id == b => Some(matches.len() as u32),
                _ => None,
            })
            .sum();
        assert_eq!(hits, 1);
        s.cancel_find();
    }

    #[test]
    fn passwords_and_bad_files() {
        let mut s = session(fixtures::encrypted("secret"), None, SessionCfg::default());
        let ev = until(&mut s, |e| matches!(e, Event::Opened(_) | Event::Failed(_)));
        assert!(matches!(ev.last(), Some(Event::Failed(PdfError::Password { wrong: false }))), "{ev:?}");
        let mut s = session(fixtures::encrypted("secret"), Some("nope"), SessionCfg::default());
        let ev = until(&mut s, |e| matches!(e, Event::Opened(_) | Event::Failed(_)));
        assert!(matches!(ev.last(), Some(Event::Failed(PdfError::Password { wrong: true }))));
        let mut s = session(fixtures::encrypted("secret"), Some("secret"), SessionCfg::default());
        opened(&mut s);
        assert!(s.info().unwrap().encrypted);
        let mut s = session(b"garbage".to_vec(), None, SessionCfg::default());
        let ev = until(&mut s, |e| matches!(e, Event::Opened(_) | Event::Failed(_)));
        assert!(matches!(ev.last(), Some(Event::Failed(PdfError::Invalid))));
        let mut s = session(fixtures::text_pages(), None, SessionCfg { max_bytes: 100, ..Default::default() });
        let ev = until(&mut s, |e| matches!(e, Event::Opened(_) | Event::Failed(_)));
        assert!(matches!(ev.last(), Some(Event::Failed(PdfError::TooLarge))));
    }

    #[test]
    fn timeouts_and_panics_are_survived() {
        let cfg = SessionCfg { threads: 1, timeout: Duration::from_millis(300), ..Default::default() };
        let mut s = session(fixtures::text_pages(), None, cfg);
        opened(&mut s);
        s.push_test(TestOp::SleepMs(3_000));
        let ev = until(&mut s, |e| matches!(e, Event::PageFailed { .. }));
        assert!(matches!(ev.last(), Some(Event::PageFailed { error: PdfError::TimedOut, .. })), "{ev:?}");
        // The replacement worker serves the next job.
        s.request(Job::Text(0), 0);
        until(&mut s, |e| matches!(e, Event::Text(0, _)));
        s.push_test(TestOp::Panic);
        let ev = until(&mut s, |e| matches!(e, Event::PageFailed { .. }));
        assert!(matches!(ev.last(), Some(Event::PageFailed { error: PdfError::Panicked, .. })));
        s.request(Job::Text(2), 0);
        until(&mut s, |e| matches!(e, Event::Text(2, _)));
    }

    #[test]
    fn stale_regions_are_dropped_after_a_generation_change() {
        let cfg = SessionCfg { threads: 1, ..Default::default() };
        let mut s = session(fixtures::text_pages(), None, cfg);
        opened(&mut s);
        // Keep the single worker busy so the requests below stay queued.
        s.push_test(TestOp::SleepMs(400));
        std::thread::sleep(Duration::from_millis(100));
        let tile = |tx| TileKey { page: 0, scale_milli: 3000, rot: 0, tx, ty: 0, invert: false };
        s.request(Job::Region(tile(0)), 0);
        s.set_generation(1);
        s.request(Job::Region(tile(1)), 0);
        s.request(Job::Text(0), 1);
        let ev = until(&mut s, |e| matches!(e, Event::Text(..)));
        let regions: Vec<_> = ev
            .iter()
            .filter_map(|e| match e {
                Event::Region { key, generation, .. } => Some((key.tx, *generation)),
                _ => None,
            })
            .collect();
        assert_eq!(regions, [(1, 1)], "the generation-0 tile was dropped");
        assert_eq!(s.pending(), 0);
    }
}
