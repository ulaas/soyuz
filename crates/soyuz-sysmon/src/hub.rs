//! The sampler: one worker thread for the whole process, since Soyuz runs several windows in one
//! process. It starts on the first interest (a subscriber, background history, an alert limit),
//! and when interest ends it keeps its probe for about a minute, not sampling, so that switching
//! tabs does not lose the counters; then the thread ends and frees everything.
//!
//! Nothing here blocks a caller on a sample: readers take short locks only, the sample runs
//! outside all locks.
//!
//! **Wake callbacks** run on the worker after each new frame. In a callback only post a wake-up
//! (`ctx.request_repaint()`, a send on an event channel). Take no lock the UI thread can hold
//! while it calls the hub, or the sampler deadlocks against it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex, MutexGuard, RwLock};
use soyuz_core::config::monitor::{Monitor, SnapshotCommands};
use soyuz_fs::sysmon::{Connection, Probe, ProcKey, RawSample, SystemInfo, Want};

use crate::alerts::{Alert, AlertLimits, AlertWatcher};
use crate::fetch::{Loader, Wake};
use crate::frame::Frame;
use crate::history::{History, Metric, ProcHistory, ProcSample, Pt};
use crate::rates::Rater;
use crate::snapshot::{self, Snapshot, SnapshotMeta};
use crate::store;

/// The two samples at the start of a run are this far apart, so the first shown frame has rates.
const PRIME_MS: u64 = 250;
/// Sampling never leaves less than this to the rest of the machine.
const MIN_SLEEP_MS: u64 = 100;
const MIN_INTERVAL_MS: u64 = 100;
/// The probe (query handles, per-process caches) outlives the last interest by this much.
const DEFAULT_LINGER: Duration = Duration::from_secs(60);
const RAM_HISTORY_MS: i64 = 48 * 3600 * 1000;
const MAX_QUEUED_ALERTS: usize = 64;
const PRUNE_EVERY: Duration = Duration::from_secs(3600);

/// Where samples come from: the real probe, or a fake in tests. Lives on the worker thread only.
pub trait Source {
    fn sample(&mut self, want: Want) -> RawSample;

    /// Read once per hub.
    fn system_info(&mut self) -> SystemInfo {
        SystemInfo::default()
    }

    /// For automatic snapshots.
    fn connections(&mut self) -> Vec<Connection> {
        Vec::new()
    }
}

struct ProbeSource(Probe);

impl Source for ProbeSource {
    fn sample(&mut self, want: Want) -> RawSample {
        self.0.sample(want)
    }

    fn system_info(&mut self) -> SystemInfo {
        soyuz_fs::sysmon::system_info()
    }

    fn connections(&mut self) -> Vec<Connection> {
        let mut list = soyuz_fs::sysmon::connections();
        crate::sanitize::sanitize_connections(&mut list);
        list
    }
}

/// Makes a source on the worker thread, each time a worker starts.
pub type SourceFactory = Arc<dyn Fn() -> Box<dyn Source> + Send + Sync>;

/// What the hub does beyond serving subscribers. See [`HubConfig::from_config`].
#[derive(Clone, Debug, PartialEq)]
pub struct HubConfig {
    /// Keep sampling with no subscriber.
    pub background_history: bool,
    pub background_interval_secs: u32,
    /// Write coarse history to `data_dir`.
    pub history_persist: bool,
    pub history_days: u32,
    /// Allow per-process GPU readings, when a subscriber wants them.
    pub process_gpu: bool,
    /// Holds `history/` and `snapshots/`. `None`: nothing is written.
    pub data_dir: Option<PathBuf>,
    pub alerts: AlertLimits,
    /// Save a snapshot when an alert fires.
    pub alert_snapshot: bool,
    pub snapshots_kept: u32,
    pub snapshot_commands: SnapshotCommands,
}

impl Default for HubConfig {
    fn default() -> Self {
        HubConfig::from_config(&Monitor::default(), None)
    }
}

impl HubConfig {
    /// The `[monitor]` settings with the app's data folder.
    pub fn from_config(cfg: &Monitor, data_dir: Option<PathBuf>) -> Self {
        HubConfig {
            background_history: cfg.background_history,
            background_interval_secs: cfg.background_interval_secs,
            history_persist: cfg.history_persist,
            history_days: cfg.history_days,
            process_gpu: cfg.process_gpu,
            data_dir,
            alerts: AlertLimits::from_config(cfg),
            alert_snapshot: cfg.alert_snapshot,
            snapshots_kept: cfg.snapshots_kept,
            snapshot_commands: cfg.snapshot_commands,
        }
    }

    fn wants_background(&self) -> bool {
        self.background_history || self.alerts.any()
    }
}

struct Sub {
    id: u64,
    interval_ms: u64,
    want: Want,
    wake: Wake,
}

struct Control {
    subs: Vec<Sub>,
    next_id: u64,
    config: HubConfig,
    paused: bool,
    worker_running: bool,
    /// Set when configuration changed, for the worker to pick up.
    config_rev: u64,
}

struct Inner {
    control: Mutex<Control>,
    changed: Condvar,
    latest: RwLock<Option<Arc<Frame>>>,
    generation: AtomicU64,
    history: RwLock<History>,
    procs: RwLock<ProcHistory>,
    alerts: Mutex<Vec<Alert>>,
    info: RwLock<Option<Arc<SystemInfo>>>,
    source: SourceFactory,
    linger: Mutex<Duration>,
    prime_ms: AtomicU64,
}

/// The process-wide sampler. [`Hub::global`] reads the real system; tests make their own.
#[derive(Clone)]
pub struct Hub {
    inner: Arc<Inner>,
}

/// Interest in frames; dropping it ends that interest.
pub struct Subscription {
    inner: Arc<Inner>,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut c = self.inner.control.lock();
        c.subs.retain(|s| s.id != self.id);
        drop(c);
        self.inner.changed.notify_all();
    }
}

impl Hub {
    /// The one sampler of this process, reading the real system.
    pub fn global() -> &'static Hub {
        static GLOBAL: OnceLock<Hub> = OnceLock::new();
        GLOBAL.get_or_init(|| Hub::with_factory(Arc::new(|| Box::new(ProbeSource(Probe::new())) as Box<dyn Source>)))
    }

    /// A separate hub over any source; the factory runs on the worker thread each time a
    /// worker starts.
    pub fn new_with_source(factory: impl Fn() -> Box<dyn Source> + Send + Sync + 'static) -> Hub {
        Hub::with_factory(Arc::new(factory))
    }

    fn with_factory(source: SourceFactory) -> Hub {
        let config = HubConfig::default();
        Hub {
            inner: Arc::new(Inner {
                history: RwLock::new(History::new(config.history_days)),
                control: Mutex::new(Control {
                    subs: Vec::new(),
                    next_id: 1,
                    config,
                    paused: false,
                    worker_running: false,
                    config_rev: 0,
                }),
                changed: Condvar::new(),
                latest: RwLock::new(None),
                generation: AtomicU64::new(0),
                procs: RwLock::new(ProcHistory::new()),
                alerts: Mutex::new(Vec::new()),
                info: RwLock::new(None),
                source,
                linger: Mutex::new(DEFAULT_LINGER),
                prime_ms: AtomicU64::new(PRIME_MS),
            }),
        }
    }

    /// How long an idle worker keeps its probe (tests shorten it).
    pub fn set_linger(&self, linger: Duration) {
        *self.inner.linger.lock() = linger;
        self.inner.changed.notify_all();
    }

    /// Asks for frames at least every `interval_ms`, with at least what `want` says. All
    /// subscribers' wishes combine: the smallest interval, the union of `Want`s. `wake` runs on
    /// the worker after each new frame: see the module documentation for what it may do.
    pub fn subscribe(&self, interval_ms: u32, want: Want, wake: Wake) -> Subscription {
        let mut c = self.inner.control.lock();
        let id = c.next_id;
        c.next_id += 1;
        c.subs.push(Sub { id, interval_ms: u64::from(interval_ms).max(MIN_INTERVAL_MS), want, wake });
        self.ensure_worker(&mut c);
        drop(c);
        self.inner.changed.notify_all();
        Subscription { inner: self.inner.clone(), id }
    }

    /// Applies the settings. Starts the worker when they ask for sampling with no tab open.
    pub fn configure(&self, config: HubConfig) {
        let mut c = self.inner.control.lock();
        if c.config == config {
            return;
        }
        c.config = config;
        c.config_rev += 1;
        self.ensure_worker(&mut c);
        drop(c);
        self.inner.changed.notify_all();
    }

    fn ensure_worker(&self, c: &mut Control) {
        if c.worker_running || !interested(c) {
            return;
        }
        let inner = self.inner.clone();
        let spawned = std::thread::Builder::new().name("soyuz-sysmon".to_string()).spawn(move || worker(inner));
        match spawned {
            Ok(_) => c.worker_running = true,
            Err(e) => log::warn!("could not start the monitor sampler: {e}"),
        }
    }

    /// The newest frame.
    pub fn latest(&self) -> Option<Arc<Frame>> {
        self.inner.latest.read().clone()
    }

    /// Changes with every new frame.
    pub fn generation(&self) -> u64 {
        self.inner.generation.load(Ordering::Acquire)
    }

    /// Reads the recorded history. Keep it short: the worker waits for this lock to add a frame.
    pub fn with_history<R>(&self, f: impl FnOnce(&History) -> R) -> R {
        f(&self.inner.history.read())
    }

    /// The short trail of one process.
    pub fn proc_series(&self, key: ProcKey) -> Vec<ProcSample> {
        self.inner.procs.read().series(key)
    }

    /// Stops or resumes sampling. Subscribers stay; the numbers freeze.
    pub fn pause(&self, paused: bool) {
        let mut c = self.inner.control.lock();
        c.paused = paused;
        self.ensure_worker(&mut c);
        drop(c);
        self.inner.changed.notify_all();
    }

    pub fn is_paused(&self) -> bool {
        self.inner.control.lock().paused
    }

    /// Alerts fired since the last call.
    pub fn take_alerts(&self) -> Vec<Alert> {
        std::mem::take(&mut *self.inner.alerts.lock())
    }

    /// What does not change while running. Read once on the worker: empty until the first start.
    pub fn system_info(&self) -> Arc<SystemInfo> {
        self.inner.info.read().clone().unwrap_or_default()
    }

    /// The configured data folder.
    pub fn data_dir(&self) -> Option<PathBuf> {
        self.inner.control.lock().config.data_dir.clone()
    }

    /// One metric between two times, on a worker: what memory holds, plus what only the files
    /// have (older than 48 hours). Use it when `from_ms` is older than `History::range`.
    pub fn series_loader(
        &self,
        metric: Metric,
        from_ms: i64,
        to_ms: i64,
        max_points: usize,
        wake: Wake,
    ) -> Loader<Vec<Pt>> {
        let dir = self.data_dir();
        let inner = self.inner.clone();
        Loader::spawn("series", wake, move || {
            let memory_from = inner.history.read().range().map(|r| r.0);
            let older_to = memory_from.map_or(to_ms, |m| (m - 1).min(to_ms));
            let mut out = match &dir {
                Some(dir) if from_ms <= older_to => {
                    store::load_series(dir, &metric, from_ms, older_to, max_points).unwrap_or_default()
                }
                _ => Vec::new(),
            };
            if let Some(m) = memory_from
                && m <= to_ms
            {
                let recent = inner.history.read().series(&metric, from_ms.max(m), to_ms, max_points);
                if let (Some(last), Some(next)) = (out.last(), recent.first())
                    && !last.is_break()
                    && next.t - last.t > 3 * 60_000
                {
                    let t = next.t - 1;
                    out.push(Pt { t, avg: f32::NAN, max: f32::NAN });
                }
                out.extend(recent);
            }
            out
        })
    }

    /// Takes a snapshot of the newest frame on a worker and saves it. `Ok(None)`: no frame yet,
    /// or no data folder.
    pub fn snapshot_now(&self, label: String, wake: Wake) -> Loader<std::io::Result<Option<SnapshotMeta>>> {
        let frame = self.latest();
        let info = self.system_info();
        let (dir, commands, kept) = {
            let c = self.inner.control.lock();
            (c.config.data_dir.clone(), c.config.snapshot_commands, c.config.snapshots_kept)
        };
        Loader::spawn("snapshot", wake, move || {
            let (Some(frame), Some(dir)) = (frame, dir) else { return Ok(None) };
            let mut connections = soyuz_fs::sysmon::connections();
            crate::sanitize::sanitize_connections(&mut connections);
            let snap = Snapshot::capture(&frame, &info, connections, &label, false, commands);
            let meta = snapshot::save(&dir, &snap)?;
            snapshot::prune_auto(&dir, kept)?;
            Ok(Some(meta))
        })
    }

    #[cfg(test)]
    fn worker_running(&self) -> bool {
        self.inner.control.lock().worker_running
    }

    #[cfg(test)]
    fn set_prime_ms(&self, ms: u64) {
        self.inner.prime_ms.store(ms, Ordering::Relaxed);
    }
}

fn interested(c: &Control) -> bool {
    !c.subs.is_empty() || c.config.wants_background()
}

/// The combined wish of everyone interested.
fn plan(c: &Control) -> Option<(u64, Want)> {
    if !interested(c) {
        return None;
    }
    let mut interval = u64::MAX;
    let mut want = Want { processes: false, process_details: false, process_gpu: false, hardware: false };
    if c.config.wants_background() {
        interval = u64::from(c.config.background_interval_secs.max(1)) * 1000;
        // So that top lists in history still work.
        want.processes = true;
        want.hardware = true;
    }
    for s in &c.subs {
        interval = interval.min(s.interval_ms);
        want.processes |= s.want.processes;
        want.process_details |= s.want.process_details;
        want.process_gpu |= s.want.process_gpu;
        want.hardware |= s.want.hardware;
    }
    want.process_gpu &= c.config.process_gpu;
    Some((interval, want))
}

/// Marks the worker gone even when a reader panics, so the next interest starts a new one.
struct RunningGuard {
    inner: Arc<Inner>,
    armed: bool,
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        if self.armed {
            let clear_history = {
                let mut c = self.inner.control.lock();
                c.worker_running = false;
                history_on_disk(&c)
            };
            release_memory(&self.inner, clear_history);
        }
    }
}

fn history_on_disk(c: &Control) -> bool {
    c.config.history_persist && c.config.data_dir.is_some()
}

/// With no worker, no big table stays in memory. The recent tiers stay, so that graphs do not
/// start over when a monitor tab comes back; the long tier is on disk and returns on the next
/// start (without a disk it stays too). No control lock here: `with_history` closures may call
/// the hub.
fn release_memory(inner: &Inner, on_disk: bool) {
    *inner.latest.write() = None;
    inner.procs.write().clear();
    if on_disk {
        inner.history.write().release_coarse();
    }
}

struct Persist {
    dir: PathBuf,
    writer: store::Writer,
    last_prune: Instant,
    days: u32,
}

impl Persist {
    fn open(dir: &Path, days: u32, now_ms: i64, inner: &Inner) -> Persist {
        // Blocking reads, on the worker. Failures leave a shorter record, nothing more.
        let _ = store::prune(dir, days, now_ms);
        match store::load_since(dir, now_ms - RAM_HISTORY_MS) {
            Ok(points) if !points.is_empty() => inner.history.write().load_coarse(points),
            Ok(_) => {}
            Err(e) => log::debug!("monitor history not loaded: {e}"),
        }
        let oldest = store::oldest(dir).unwrap_or(None);
        inner.history.write().set_disk_oldest(oldest);
        Persist { dir: dir.to_path_buf(), writer: store::Writer::new(dir), last_prune: Instant::now(), days }
    }
}

fn worker(inner: Arc<Inner>) {
    let mut guard = RunningGuard { inner: inner.clone(), armed: true };
    let mut source = (inner.source)();
    if inner.info.read().is_none() {
        // The read blocks, so not under the lock.
        let info = Arc::new(source.system_info());
        *inner.info.write() = Some(info);
    }
    let mut rater = Rater::new();
    let mut watcher = AlertWatcher::new();
    let mut persist: Option<Persist> = None;
    let mut seen_rev = None;
    let mut config = HubConfig::default();
    let mut needs_prime = true;
    let mut next_at = Instant::now();
    let mut idle_since: Option<Instant> = None;

    loop {
        // Wait until a sample is due, or leave after a long idle.
        let mut config_changed = false;
        let (interval, want) = {
            let mut c = inner.control.lock();
            loop {
                let wish = if c.paused { None } else { plan(&c) };
                if let Some((interval, want)) = wish {
                    idle_since = None;
                    let due = next_at.min(Instant::now() + Duration::from_millis(interval));
                    if Instant::now() >= due {
                        if seen_rev != Some(c.config_rev) {
                            seen_rev = Some(c.config_rev);
                            config = c.config.clone();
                            config_changed = true;
                        }
                        break (interval, want);
                    }
                    let _ = inner.changed.wait_until(&mut c, due);
                } else {
                    // Idle: paused or nobody interested. Counters go stale; the first sample
                    // after this primes again.
                    needs_prime = true;
                    next_at = Instant::now();
                    if c.paused && interested(&c) {
                        idle_since = None;
                        inner.changed.wait(&mut c);
                        continue;
                    }
                    let since = *idle_since.get_or_insert_with(Instant::now);
                    let linger = *inner.linger.lock();
                    let left = linger.saturating_sub(since.elapsed());
                    if left.is_zero() {
                        let clear_history = history_on_disk(&c);
                        MutexGuard::unlocked(&mut c, || release_memory(&inner, clear_history));
                        // Decided under the lock: a subscriber that comes after this sees the
                        // worker gone and starts a new one.
                        if !interested(&c) {
                            c.worker_running = false;
                            guard.armed = false;
                            return;
                        }
                        // Interest came back while memory was being released.
                        persist = None;
                        seen_rev = None;
                        idle_since = None;
                        continue;
                    }
                    let _ = inner.changed.wait_for(&mut c, left);
                }
            }
        };

        if config_changed {
            inner.history.write().set_days(config.history_days);
            let wanted_dir = config.data_dir.as_ref().filter(|_| config.history_persist);
            match (wanted_dir, &mut persist) {
                (Some(dir), Some(p)) if p.dir == *dir => p.days = config.history_days,
                (Some(dir), _) => persist = Some(Persist::open(dir, config.history_days, now_ms(), &inner)),
                (None, _) => persist = None,
            }
        }
        rater.set_interval(interval);

        let started = Instant::now();
        if needs_prime {
            // Two samples a moment apart: the first frame shown already has rates.
            needs_prime = false;
            let _primed = rater.push(source.sample(want));
            let prime = Duration::from_millis(inner.prime_ms.load(Ordering::Relaxed));
            let mut c = inner.control.lock();
            let until = Instant::now() + prime;
            while Instant::now() < until {
                if inner.changed.wait_until(&mut c, until).timed_out() {
                    break;
                }
            }
            drop(c);
        }
        let frame = rater.push(source.sample(want));
        publish(&inner, frame, &config, source.as_mut(), &mut watcher, &mut persist);

        let spent = started.elapsed();
        let sleep = Duration::from_millis(interval).saturating_sub(spent).max(Duration::from_millis(MIN_SLEEP_MS));
        next_at = Instant::now() + sleep;
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn publish(
    inner: &Arc<Inner>,
    frame: Frame,
    config: &HubConfig,
    source: &mut dyn Source,
    watcher: &mut AlertWatcher,
    persist: &mut Option<Persist>,
) {
    let frame = Arc::new(frame);
    let finished = inner.history.write().push(&frame);
    inner.procs.write().push(&frame);

    if let Some(p) = persist {
        if let Some(point) = finished
            && let Err(e) = p.writer.append(&point)
        {
            log::debug!("monitor history not written: {e}");
        }
        if p.last_prune.elapsed() >= PRUNE_EVERY {
            p.last_prune = Instant::now();
            let _ = store::prune(&p.dir, p.days, frame.at_ms);
            let oldest = store::oldest(&p.dir).unwrap_or(None);
            let mut h = inner.history.write();
            h.set_disk_oldest(oldest);
            h.prune(frame.at_ms);
        }
    }

    let mut fired = watcher.check_limits(&frame, &config.alerts);
    if !fired.is_empty() {
        if config.alert_snapshot
            && let Some(dir) = &config.data_dir
        {
            let info = inner.info.read().clone().unwrap_or_default();
            let snap = Snapshot::capture(&frame, &info, source.connections(), "", true, config.snapshot_commands);
            match snapshot::save(dir, &snap) {
                Ok(meta) => {
                    for a in &mut fired {
                        a.snapshot_id = Some(meta.id.clone());
                    }
                    let _ = snapshot::prune_auto(dir, config.snapshots_kept);
                }
                Err(e) => log::debug!("automatic snapshot not saved: {e}"),
            }
        }
        let mut queue = inner.alerts.lock();
        queue.extend(fired);
        if queue.len() > MAX_QUEUED_ALERTS {
            let excess = queue.len() - MAX_QUEUED_ALERTS;
            queue.drain(..excess);
        }
    }

    *inner.latest.write() = Some(frame);
    inner.generation.fetch_add(1, Ordering::AcqRel);

    let wakes: Vec<Wake> = inner.control.lock().subs.iter().map(|s| s.wake.clone()).collect();
    for wake in wakes {
        wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::proc_raw;
    use soyuz_fs::sysmon::{CpuTimes, MemoryRaw};
    use std::sync::atomic::AtomicU32;

    /// Advances counters by itself and records what it was asked for.
    #[derive(Clone, Default)]
    struct Fake {
        calls: Arc<Mutex<Vec<(Instant, Want)>>>,
        made: Arc<AtomicU32>,
    }

    impl Source for Fake {
        fn sample(&mut self, want: Want) -> RawSample {
            let mut calls = self.calls.lock();
            calls.push((Instant::now(), want));
            let n = calls.len() as u64;
            let mono = n * 200;
            RawSample {
                at_ms: now_ms(),
                mono_ms: mono,
                cpu_total: CpuTimes { user: n * 50, idle: n * 50, ..Default::default() },
                memory: MemoryRaw { total: 100, available: 10, ..Default::default() },
                processes: vec![proc_raw(1, 1, "a", n * 10)],
                ..Default::default()
            }
        }

        fn system_info(&mut self) -> SystemInfo {
            SystemInfo { hostname: "fake".into(), ..Default::default() }
        }
    }

    fn hub() -> (Hub, Fake) {
        let fake = Fake::default();
        let f = fake.clone();
        let hub = Hub::new_with_source(move || {
            f.made.fetch_add(1, Ordering::SeqCst);
            Box::new(f.clone()) as Box<dyn Source>
        });
        hub.set_prime_ms(10);
        hub.set_linger(Duration::from_millis(100));
        (hub, fake)
    }

    fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
        let start = Instant::now();
        while !cond() {
            assert!(start.elapsed() < Duration::from_secs(20), "timed out waiting for: {what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    const WANT_MIN: Want = Want { processes: false, process_details: false, process_gpu: false, hardware: false };

    #[test]
    fn no_interest_no_worker() {
        let (hub, fake) = hub();
        std::thread::sleep(Duration::from_millis(30));
        assert!(!hub.worker_running());
        assert_eq!(fake.calls.lock().len(), 0);
        assert!(hub.latest().is_none());
        assert_eq!(hub.generation(), 0);
    }

    #[test]
    fn first_shown_frame_already_has_rates() {
        let (hub, fake) = hub();
        let _s = hub.subscribe(100, Want::default(), Arc::new(|| {}));
        wait_until("a frame", || hub.latest().is_some());
        let f = hub.latest().unwrap();
        assert!(!f.first, "the priming sample is never published");
        assert!(fake.calls.lock().len() >= 2);
        assert!(f.cpu.percent > 0.0);
        assert_eq!(hub.system_info().hostname, "fake");
    }

    #[test]
    fn wakes_after_each_frame_and_generation_moves() {
        let (hub, _) = hub();
        let n = Arc::new(AtomicU32::new(0));
        let m = n.clone();
        let wake: Wake = Arc::new(move || {
            m.fetch_add(1, Ordering::SeqCst);
        });
        let _s = hub.subscribe(100, Want::default(), wake);
        wait_until("three frames", || n.load(Ordering::SeqCst) >= 3);
        assert!(hub.generation() >= 3);
    }

    #[test]
    fn subscribers_combine_their_wishes() {
        let (hub, fake) = hub();
        let _a = hub.subscribe(5000, Want { processes: true, ..WANT_MIN }, Arc::new(|| {}));
        let _b = hub.subscribe(100, Want { hardware: true, ..WANT_MIN }, Arc::new(|| {}));
        let want = Want { processes: true, hardware: true, ..WANT_MIN };
        wait_until("the OR of both wishes", || fake.calls.lock().iter().any(|c| c.1 == want));
        // And the smaller interval: several samples in far less than 5 s.
        let before = fake.calls.lock().len();
        wait_until("more samples", || fake.calls.lock().len() >= before + 3);
    }

    #[test]
    fn process_gpu_stays_off_unless_the_settings_allow_it() {
        let (hub, fake) = hub();
        hub.configure(HubConfig { process_gpu: false, ..Default::default() });
        let _a = hub.subscribe(100, Want::default(), Arc::new(|| {}));
        wait_until("a sample", || !fake.calls.lock().is_empty());
        assert!(!fake.calls.lock()[0].1.process_gpu);
        hub.configure(HubConfig { process_gpu: true, ..Default::default() });
        wait_until("process GPU on", || fake.calls.lock().iter().any(|c| c.1.process_gpu));
    }

    #[test]
    fn slow_frame_guard_leaves_a_minimum_sleep() {
        struct Slow(Arc<Mutex<Vec<Instant>>>);
        impl Source for Slow {
            fn sample(&mut self, _want: Want) -> RawSample {
                std::thread::sleep(Duration::from_millis(30));
                self.0.lock().push(Instant::now());
                RawSample::default()
            }
        }
        let times = Arc::new(Mutex::new(Vec::new()));
        let t = times.clone();
        let hub = Hub::new_with_source(move || Box::new(Slow(t.clone())) as Box<dyn Source>);
        hub.set_prime_ms(1);
        let _s = hub.subscribe(100, Want::default(), Arc::new(|| {}));
        wait_until("five samples", || times.lock().len() >= 5);
        let times = times.lock();
        for pair in times[1..].windows(2) {
            let gap = pair[1] - pair[0];
            assert!(gap >= Duration::from_millis(100), "{gap:?}");
        }
    }

    #[test]
    fn worker_ends_after_the_linger_and_starts_again() {
        let (hub, fake) = hub();
        let s = hub.subscribe(100, Want::default(), Arc::new(|| {}));
        wait_until("a frame", || hub.latest().is_some());
        assert_eq!(fake.made.load(Ordering::SeqCst), 1);
        drop(s);
        wait_until("the worker to end", || !hub.worker_running());
        wait_until("the frame to go", || hub.latest().is_none());
        let calls = fake.calls.lock().len();
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(fake.calls.lock().len(), calls, "no sampling without interest");
        let _s = hub.subscribe(100, Want::default(), Arc::new(|| {}));
        wait_until("a frame again", || hub.latest().is_some());
        assert_eq!(fake.made.load(Ordering::SeqCst), 2, "a new source for a new worker");
    }

    #[test]
    fn the_probe_stays_while_interest_returns_within_the_linger() {
        let (hub, fake) = hub();
        hub.set_linger(Duration::from_secs(60));
        let s = hub.subscribe(100, Want::default(), Arc::new(|| {}));
        wait_until("a frame", || hub.latest().is_some());
        drop(s);
        std::thread::sleep(Duration::from_millis(30));
        assert!(hub.worker_running());
        let g = hub.generation();
        let _s = hub.subscribe(100, Want::default(), Arc::new(|| {}));
        wait_until("a new frame", || hub.generation() > g);
        assert_eq!(fake.made.load(Ordering::SeqCst), 1, "the same probe");
        assert!(!hub.latest().unwrap().first);
    }

    #[test]
    fn alerts_keep_the_sampler_running_with_no_subscriber() {
        let (hub, fake) = hub();
        hub.configure(HubConfig {
            background_history: false,
            background_interval_secs: 1,
            alerts: AlertLimits { memory_percent: 50, sustain_secs: 0, ..Default::default() },
            alert_snapshot: false,
            ..Default::default()
        });
        wait_until("an alert", || !hub.take_alerts().is_empty());
        assert!(hub.worker_running());
        // Background sampling asks only for what top lists need.
        let want = fake.calls.lock()[0].1;
        assert_eq!(want, Want { processes: true, process_details: false, process_gpu: false, hardware: true });
    }

    #[test]
    fn background_history_records_with_no_subscriber_then_stops_when_turned_off() {
        let (hub, _) = hub();
        hub.configure(HubConfig { background_history: true, background_interval_secs: 1, ..Default::default() });
        wait_until("a recorded point", || hub.with_history(|h| h.range().is_some()));
        hub.configure(HubConfig { background_history: false, ..Default::default() });
        wait_until("the worker to end", || !hub.worker_running());
    }

    #[test]
    fn pause_freezes_the_numbers_and_resume_primes_again() {
        let (hub, fake) = hub();
        let _s = hub.subscribe(100, Want::default(), Arc::new(|| {}));
        wait_until("a frame", || hub.latest().is_some());
        hub.pause(true);
        assert!(hub.is_paused());
        std::thread::sleep(Duration::from_millis(250)); // lets a sample in flight finish
        let calls = fake.calls.lock().len();
        let g = hub.generation();
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(fake.calls.lock().len(), calls);
        assert_eq!(hub.generation(), g);
        assert!(hub.worker_running(), "a paused hub keeps its probe");
        assert!(hub.latest().is_some());
        hub.pause(false);
        wait_until("a new frame", || hub.generation() > g);
    }

    #[test]
    fn process_trails_are_recorded() {
        let (hub, _) = hub();
        let _s = hub.subscribe(100, Want::default(), Arc::new(|| {}));
        let key = crate::testutil::key(1, 1);
        wait_until("a trail", || !hub.proc_series(key).is_empty());
    }

    #[test]
    fn history_is_persisted_and_loaded_on_start() {
        let t = tempfile::tempdir().unwrap();
        // Write what a previous run left: one point a minute ago.
        let old_t = now_ms() - 60_000;
        let old = crate::history::AggPoint { v: 1, t: old_t, ..Default::default() };
        store::Writer::new(t.path()).append(&old).unwrap();

        let (hub, _) = hub();
        hub.set_linger(Duration::from_millis(50));
        let cfg = HubConfig {
            data_dir: Some(t.path().to_path_buf()),
            history_persist: true,
            history_days: 7,
            ..Default::default()
        };
        hub.configure(cfg);
        let s = hub.subscribe(100, Want::default(), Arc::new(|| {}));
        wait_until("the old point in memory", || hub.with_history(|h| h.range().is_some_and(|r| r.0 <= old_t)));
        // After the worker ended, the long tier is back on disk only; the recent tiers stay, so
        // that graphs do not start over when a monitor tab returns.
        drop(s);
        wait_until("the worker to end", || !hub.worker_running());
        wait_until("the long tier to leave", || hub.with_history(|h| h.range().is_none_or(|r| r.0 > old_t)));
        assert!(!store::load(t.path(), 7, now_ms()).unwrap().is_empty());
        assert!(hub.with_history(|h| h.range().is_some()), "the recent tiers stay");
    }

    #[test]
    fn an_alert_can_save_an_automatic_snapshot() {
        let t = tempfile::tempdir().unwrap();
        let (hub, _) = hub();
        hub.configure(HubConfig {
            data_dir: Some(t.path().to_path_buf()),
            history_persist: false,
            alerts: AlertLimits { memory_percent: 50, sustain_secs: 0, ..Default::default() },
            alert_snapshot: true,
            snapshots_kept: 5,
            background_interval_secs: 1,
            ..Default::default()
        });
        let mut got = Vec::new();
        wait_until("an alert", || {
            got = hub.take_alerts();
            !got.is_empty()
        });
        let id = got[0].snapshot_id.clone().expect("a snapshot was saved");
        let list = snapshot::list(t.path()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert!(list[0].auto);
    }

    #[test]
    fn snapshot_now_and_series_loader_work_from_a_worker() {
        let t = tempfile::tempdir().unwrap();
        let (hub, _) = hub();
        hub.configure(HubConfig {
            data_dir: Some(t.path().to_path_buf()),
            history_persist: false,
            ..Default::default()
        });
        let _s = hub.subscribe(100, Want::default(), Arc::new(|| {}));
        wait_until("a frame", || hub.latest().is_some());

        let mut l = hub.snapshot_now("by hand".into(), Arc::new(|| {}));
        let mut done = None;
        wait_until("the snapshot", || {
            done = l.poll();
            done.is_some()
        });
        let meta = done.unwrap().unwrap().expect("there is a frame and a folder");
        assert_eq!(meta.label, "by hand");
        assert!(!meta.auto);

        wait_until("two recorded points", || hub.with_history(|h| h.range().is_some_and(|r| r.1 > r.0)));
        let now = now_ms();
        let mut l = hub.series_loader(Metric::MemPercent, now - 600_000, now + 1000, 100, Arc::new(|| {}));
        let mut series = None;
        wait_until("the series", || {
            series = l.poll();
            series.is_some()
        });
        assert!(!series.unwrap().is_empty());
    }

    #[test]
    fn a_panicking_source_lets_a_later_subscriber_start_over() {
        let n = Arc::new(AtomicU32::new(0));
        let m = n.clone();
        struct Boom;
        impl Source for Boom {
            fn sample(&mut self, _want: Want) -> RawSample {
                panic!("expected in this test");
            }
        }
        let hub = Hub::new_with_source(move || {
            m.fetch_add(1, Ordering::SeqCst);
            Box::new(Boom) as Box<dyn Source>
        });
        let a = hub.subscribe(100, Want::default(), Arc::new(|| {}));
        wait_until("the worker to die", || n.load(Ordering::SeqCst) == 1 && !hub.worker_running());
        drop(a);
        let _b = hub.subscribe(100, Want::default(), Arc::new(|| {}));
        wait_until("a second start", || n.load(Ordering::SeqCst) == 2);
    }

    #[test]
    fn wake_may_read_the_hub_without_deadlocking() {
        let (hub, _) = hub();
        let h = hub.clone();
        let seen = Arc::new(AtomicU32::new(0));
        let s2 = seen.clone();
        let wake: Wake = Arc::new(move || {
            if h.latest().is_some() && h.with_history(|_| true) {
                s2.fetch_add(1, Ordering::SeqCst);
            }
        });
        let _s = hub.subscribe(100, Want::default(), wake);
        wait_until("a wake that read the hub", || seen.load(Ordering::SeqCst) > 0);
    }

    #[test]
    fn global_hub_exists_and_stays_idle_without_interest() {
        let hub = Hub::global();
        assert!(std::ptr::eq(hub, Hub::global()));
        assert!(!hub.is_paused());
    }

    #[test]
    fn the_config_comes_from_the_settings() {
        let cfg = Monitor { background_history: true, history_days: 9, process_gpu: true, ..Default::default() };
        let hc = HubConfig::from_config(&cfg, Some(PathBuf::from("x")));
        assert!(hc.background_history && hc.process_gpu);
        assert_eq!(hc.history_days, 9);
        assert_eq!(hc.data_dir.as_deref(), Some(Path::new("x")));
    }
}
