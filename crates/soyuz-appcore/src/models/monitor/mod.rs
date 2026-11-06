//! The system monitor (`monitor:` tabs): the state and commands behind both frontends' pages.
//!
//! The process-wide sampler is `soyuz_sysmon::Hub`; [`pump`] subscribes while a monitor pane
//! is on screen, so nothing is sampled otherwise (unless `[monitor] background_history` or an
//! alert asks for it). Each pane has its own [`MonitorView`] (page, filter, selection, …),
//! so two panes can show two pages. What a user arranges (columns, sort, view, graph range)
//! is stored in `[monitor]`.
//!
//! Frontends read [`MonitorState`] while drawing and push [`cmd::MonitorCmd`]s through
//! `Cmd::Monitor`. Every blocking read goes through a `soyuz_sysmon::fetch::Loader`.

pub mod cmd;
pub mod labels;
pub mod lists;
pub mod snapshots;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use soyuz_core::Location;
use soyuz_core::config::monitor::{Monitor, MonitorPage};
use soyuz_sysmon::fetch::{self, Loader};
use soyuz_sysmon::{
    ControlError, DetailsWant, Filter, FilterError, Frame, Hub, HubConfig, Metric, ProcDetails, ProcKey, Pt, RowId,
    Subscription, SystemInfo, TableCache, TableOpts, TableRow, Tops, Want,
};

use crate::core::AppCore;
use crate::models::toasts::Toast;
use crate::ops;
use crate::state::pane::PaneId;

pub use cmd::{MonitorCmd, SignalScope, apply};
pub use lists::{ListState, Sortable};
pub use snapshots::{CompareTarget, SnapshotsState};

/// How often the details panel's lists are read again.
const DETAILS_REFRESH: Duration = Duration::from_secs(2);
/// How often the Connections page reads the socket tables.
const CONNECTIONS_REFRESH: Duration = Duration::from_secs(4);
/// How often the Services page reads the service list.
const SERVICES_REFRESH: Duration = Duration::from_secs(10);

/// The tabs of the process details panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DetailsTab {
    #[default]
    General,
    Threads,
    Modules,
    Files,
    Environment,
    Sockets,
}

impl DetailsTab {
    pub const ALL: [DetailsTab; 6] = [
        DetailsTab::General,
        DetailsTab::Threads,
        DetailsTab::Modules,
        DetailsTab::Files,
        DetailsTab::Environment,
        DetailsTab::Sockets,
    ];

    /// The tabs this OS can fill (Windows has no reader for open files or environments).
    pub fn available() -> &'static [DetailsTab] {
        if cfg!(windows) {
            &[DetailsTab::General, DetailsTab::Threads, DetailsTab::Modules, DetailsTab::Sockets]
        } else {
            &Self::ALL
        }
    }

    fn want(self) -> DetailsWant {
        DetailsWant {
            threads: self == DetailsTab::Threads,
            modules: self == DetailsTab::Modules,
            open_files: self == DetailsTab::Files,
            environment: self == DetailsTab::Environment,
        }
    }
}

/// The process details panel.
#[derive(Default)]
pub struct DetailsState {
    pub tab: DetailsTab,
    /// What `data` belongs to.
    pub key: Option<(ProcKey, DetailsTab)>,
    pub data: Option<Result<ProcDetails, ControlError>>,
    loader: Option<Loader<Result<ProcDetails, ControlError>>>,
    read_at: Option<Instant>,
}

impl DetailsState {
    pub fn loading(&self) -> bool {
        self.loader.is_some() && self.data.is_none()
    }
}

/// Editing which logical CPUs a process may run on.
#[derive(Clone, Debug)]
pub struct AffinityEdit {
    pub key: ProcKey,
    pub name: String,
    /// One flag per logical CPU.
    pub cpus: Vec<bool>,
}

/// The History page.
pub struct HistoryState {
    /// Seconds shown, ending at `end_ms` (or now).
    pub range_secs: u32,
    /// The right edge when panned back (`None`: live).
    pub end_ms: Option<i64>,
    /// The moment under the pointer or the cursor.
    pub cursor_ms: Option<i64>,
    /// Top processes at `cursor_ms`.
    pub tops: Option<Tops>,
    series: HashMap<Metric, Arc<Vec<Pt>>>,
    loaders: HashMap<Metric, Loader<Vec<Pt>>>,
    built_at: u64,
    built_for: (u32, Option<i64>),
    /// The earliest moment on record (memory and disk).
    pub oldest_ms: Option<i64>,
}

impl Default for HistoryState {
    fn default() -> Self {
        HistoryState {
            range_secs: 3600,
            end_ms: None,
            cursor_ms: None,
            tops: None,
            series: HashMap::new(),
            loaders: HashMap::new(),
            built_at: 0,
            built_for: (0, None),
            oldest_ms: None,
        }
    }
}

impl HistoryState {
    /// The ranges the page offers, in seconds.
    pub const RANGES: [u32; 7] = [300, 900, 3600, 6 * 3600, 24 * 3600, 3 * 86400, 7 * 86400];

    /// The series of `metric` over the shown range (empty until read).
    pub fn series(&self, metric: &Metric) -> Arc<Vec<Pt>> {
        self.series.get(metric).cloned().unwrap_or_default()
    }

    /// `(from, to)` in unix milliseconds.
    pub fn window(&self, now_ms: i64) -> (i64, i64) {
        let to = self.end_ms.unwrap_or(now_ms);
        (to - i64::from(self.range_secs) * 1000, to)
    }
}

/// What one monitor pane shows.
pub struct MonitorView {
    pub page: MonitorPage,
    /// The process filter as typed.
    pub filter_text: String,
    pub filter: Filter,
    /// Why `filter_text` does not parse (the last good `filter` stays in force).
    pub filter_error: Option<FilterError>,
    /// The whole filter is one regular expression.
    pub filter_regex: bool,
    /// Ask the frontend to put the keyboard into the filter box.
    pub focus_filter: bool,
    pub collapsed: HashSet<RowId>,
    pub selected: Option<RowId>,
    /// Marked rows; end / kill / suspend act on these when there are any.
    pub marked: HashSet<ProcKey>,
    pub details: DetailsState,
    pub affinity: Option<AffinityEdit>,
    pub connections: ListState<soyuz_sysmon::Connection>,
    pub services: ListState<soyuz_sysmon::ServiceInfo>,
    pub startup: ListState<soyuz_sysmon::StartupItem>,
    pub history: HistoryState,
    pub snapshots: SnapshotsState,
    table: TableCache,
    me: Option<String>,
}

impl MonitorView {
    fn new(cfg: &Monitor) -> Self {
        let (text, regex) =
            if cfg.remember_filter { (cfg.filter.clone(), cfg.filter_regex) } else { Default::default() };
        let mut v = MonitorView {
            page: cfg.page,
            filter_text: String::new(),
            filter: Filter::default(),
            filter_error: None,
            filter_regex: regex,
            focus_filter: false,
            collapsed: HashSet::new(),
            selected: None,
            marked: HashSet::new(),
            details: DetailsState::default(),
            affinity: None,
            connections: ListState::new(lists::Column::Pid as u8, false),
            services: ListState::new(0, false),
            startup: ListState::new(0, false),
            history: HistoryState::default(),
            snapshots: SnapshotsState::default(),
            table: TableCache::new(),
            me: None,
        };
        v.set_filter(text);
        v
    }

    /// Change the process filter. A text that does not parse keeps the last good filter.
    pub fn set_filter(&mut self, text: String) {
        match Filter::parse(&text, self.filter_regex) {
            Ok(f) => {
                self.filter = f;
                self.filter_error = None;
                self.table.invalidate();
            }
            Err(e) => self.filter_error = Some(e),
        }
        self.filter_text = text;
    }

    /// The rows of the process table for `frame`; cached until something changes.
    pub fn rows(&mut self, frame: &Frame, generation: u64, cfg: &Monitor) -> &[TableRow] {
        if self.me.is_none() {
            let me = frame
                .procs
                .iter()
                .find(|p| p.raw.key.pid == std::process::id())
                .and_then(|p| p.raw.user.as_deref().map(str::to_string));
            self.me = me;
        }
        let opts = TableOpts::from_config(cfg, self.me.clone(), frame);
        self.table.rows(frame, generation, &self.filter, &opts, &self.collapsed)
    }

    /// The collapsed set changed.
    pub(super) fn table_changed(&mut self) {
        self.table.invalidate();
    }

    /// The name this process runs under (table and confirm dialogs compare with it).
    pub fn me(&self) -> Option<&str> {
        self.me.as_deref()
    }

    /// The selected process in `frame`.
    pub fn selected_row<'a>(&self, frame: &'a Frame) -> Option<&'a soyuz_sysmon::ProcRow> {
        match &self.selected {
            Some(RowId::Proc(k)) => frame.proc(*k),
            _ => None,
        }
    }
}

/// The monitor state of one window.
#[derive(Default)]
pub struct MonitorState {
    sub: Option<(Subscription, u32, Want)>,
    hub_config: Option<Monitor>,
    /// The newest frame (kept when the sampler is paused or asleep).
    pub frame: Option<Arc<Frame>>,
    pub generation: u64,
    pub info: Arc<SystemInfo>,
    views: HashMap<PaneId, MonitorView>,
    series: HashMap<(Metric, u32), Arc<Vec<Pt>>>,
    series_generation: u64,
    pending: Vec<Loader<cmd::Outcome>>,
    /// Tests: the frames come from [`MonitorState::feed_test_frames`], not the sampler.
    test_history: Option<soyuz_sysmon::History>,
}

impl MonitorState {
    /// The view of `pane`, created on first use.
    pub fn view(&mut self, pane: PaneId, cfg: &Monitor) -> &mut MonitorView {
        self.views.entry(pane).or_insert_with(|| MonitorView::new(cfg))
    }

    /// The view of `pane`, if it has one.
    pub fn peek(&self, pane: PaneId) -> Option<&MonitorView> {
        self.views.get(&pane)
    }

    /// Tests: show `frames` (the last is the current one) instead of the real sampler.
    #[doc(hidden)]
    pub fn feed_test_frames(&mut self, frames: Vec<Frame>, info: SystemInfo) {
        let mut history = soyuz_sysmon::History::new(1);
        for f in &frames {
            history.push(f);
        }
        self.test_history = Some(history);
        self.frame = frames.into_iter().last().map(Arc::new);
        self.info = Arc::new(info);
        self.generation += 1;
        self.series.clear();
    }

    /// The page `pane` shows, if it has a view yet.
    pub fn page_of(&self, pane: PaneId) -> Option<MonitorPage> {
        self.views.get(&pane).map(|v| v.page)
    }

    /// The last `seconds` of `metric`, at most `max_points` wide. Read once per frame of data.
    pub fn series(&mut self, metric: &Metric, seconds: u32, max_points: usize) -> Arc<Vec<Pt>> {
        if self.series_generation != self.generation {
            self.series_generation = self.generation;
            self.series.clear();
        }
        if let Some(s) = self.series.get(&(metric.clone(), seconds)) {
            return s.clone();
        }
        let to = self.frame.as_ref().map_or(0, |f| f.at_ms);
        let from = to - i64::from(seconds) * 1000;
        let s = Arc::new(match &self.test_history {
            Some(h) => h.series(metric, from, to, max_points),
            None => Hub::global().with_history(|h| h.series(metric, from, to, max_points)),
        });
        self.series.insert((metric.clone(), seconds), s.clone());
        s
    }

    /// Whether the sampler is frozen.
    pub fn paused(&self) -> bool {
        Hub::global().is_paused()
    }
}

/// Where history and snapshots live.
pub fn data_dir() -> PathBuf {
    soyuz_core::config::config_dir().join("monitor")
}

/// Show the monitor: the tab that already has it, else a new tab.
pub fn open_tab(app: &mut AppCore) {
    let open = app.ws.tabs.iter().position(|t| t.panes.iter().any(|p| matches!(p.location, Location::Monitor)));
    match open {
        Some(i) => app.ws.active = i,
        None => ops::open_in_new_tab(app, Location::Monitor, true),
    }
}

/// Per frame (`AppCore::pump_services`): keep the hub configured, subscribe while a monitor
/// pane is on screen, take new frames and finished background reads.
pub fn pump(app: &mut AppCore) {
    let hub = Hub::global();
    if app.ui.monitor.hub_config.as_ref() != Some(&app.config.monitor) {
        hub.configure(HubConfig::from_config(&app.config.monitor, Some(data_dir())));
        app.ui.monitor.hub_config = Some(app.config.monitor.clone());
    }
    let tab = app.ws.tab();
    let shown: Vec<PaneId> =
        tab.panes.iter().take(tab.shown()).filter(|p| matches!(p.location, Location::Monitor)).map(|p| p.id).collect();
    let m = &mut app.ui.monitor;
    if shown.is_empty() || m.test_history.is_some() {
        m.sub = None;
    } else {
        let interval = app.config.monitor.interval_ms.clamp(Monitor::INTERVAL_MS_MIN, Monitor::INTERVAL_MS_MAX);
        let want = Want {
            processes: true,
            process_details: true,
            process_gpu: app.config.monitor.process_gpu,
            hardware: true,
        };
        if !m.sub.as_ref().is_some_and(|(_, i, w)| *i == interval && *w == want) {
            m.sub = Some((hub.subscribe(interval, want, app.wake.clone()), interval, want));
        }
        let generation = hub.generation();
        if generation != m.generation {
            if let Some(f) = hub.latest() {
                for v in m.views.values_mut() {
                    v.marked.retain(|k| !f.removed.contains(k));
                }
                m.frame = Some(f);
                m.generation = generation;
            }
            if m.info.hostname.is_empty() {
                m.info = hub.system_info();
            }
        }
    }
    // Views of panes that went away.
    if !m.views.is_empty() {
        let live: HashSet<PaneId> = app.ws.tabs.iter().flat_map(|t| t.panes.iter().map(|p| p.id)).collect();
        m.views.retain(|id, _| live.contains(id));
    }
    let wake = app.wake.clone();
    let frame = m.frame.clone();
    let generation = m.generation;
    for id in &shown {
        let Some(view) = m.views.get_mut(id) else { continue };
        pump_view(view, frame.as_deref(), generation, &wake);
    }
    cmd::pump_outcomes(app);
    alerts(app);
}

fn pump_view(v: &mut MonitorView, frame: Option<&Frame>, generation: u64, wake: &soyuz_sysmon::Wake) {
    match v.page {
        MonitorPage::Processes => pump_details(v, wake),
        MonitorPage::Connections => {
            let w = wake.clone();
            v.connections.pump(CONNECTIONS_REFRESH, move || fetch::connections(w.clone()));
        }
        MonitorPage::Services => {
            let w = wake.clone();
            v.services.pump(SERVICES_REFRESH, move || fetch::services(w.clone()));
        }
        MonitorPage::Startup => {
            let w = wake.clone();
            v.startup.pump(Duration::from_secs(3600), move || fetch::startup_items(w.clone()));
        }
        MonitorPage::History => pump_history(&mut v.history, frame, generation, wake),
        MonitorPage::Snapshots => v.snapshots.pump(frame, wake),
        _ => {}
    }
    // Details' socket tab reads the same table.
    if v.page == MonitorPage::Processes && v.details.tab == DetailsTab::Sockets {
        let w = wake.clone();
        v.connections.pump(CONNECTIONS_REFRESH, move || fetch::connections(w.clone()));
    }
}

fn pump_details(v: &mut MonitorView, wake: &soyuz_sysmon::Wake) {
    let d = &mut v.details;
    if let Some(l) = d.loader.as_mut()
        && let Some(result) = l.poll()
    {
        d.data = Some(result);
        d.loader = None;
        d.read_at = Some(Instant::now());
    }
    let Some(RowId::Proc(key)) = v.selected else {
        d.key = None;
        d.data = None;
        d.loader = None;
        return;
    };
    if d.tab == DetailsTab::General || d.tab == DetailsTab::Sockets {
        return;
    }
    let target = (key, d.tab);
    let changed = d.key != Some(target);
    if changed {
        d.key = Some(target);
        d.data = None;
        d.loader = None;
        d.read_at = None;
    }
    let due = d.read_at.is_none_or(|t| t.elapsed() >= DETAILS_REFRESH);
    if due && d.loader.is_none() {
        d.loader = Some(fetch::process_details(key, d.tab.want(), wake.clone()));
    }
}

fn pump_history(h: &mut HistoryState, frame: Option<&Frame>, generation: u64, wake: &soyuz_sysmon::Wake) {
    h.loaders.retain(|metric, l| match l.poll() {
        Some(points) => {
            h.series.insert(metric.clone(), Arc::new(points));
            false
        }
        None => true,
    });
    let now = frame.map_or(0, |f| f.at_ms);
    let key = (h.range_secs, h.end_ms);
    // The live edge moves about every ten seconds on long ranges; a pan or a range change at once.
    let live_step = u64::from(h.range_secs / 300).max(1);
    if h.built_for == key && generation < h.built_at + live_step {
        return;
    }
    // Reads of another window would land in this one.
    if h.built_for != key {
        h.loaders.clear();
    }
    h.built_for = key;
    h.built_at = generation;
    let (from, to) = h.window(now);
    let hub = Hub::global();
    h.oldest_ms = hub.with_history(|hist| hist.full_range().map(|r| r.0));
    let in_memory = hub.with_history(|hist| hist.range().is_some_and(|(oldest, _)| oldest <= from));
    for metric in history_metrics(frame) {
        if in_memory {
            let points = hub.with_history(|hist| hist.series(&metric, from, to, 600));
            h.series.insert(metric, Arc::new(points));
        } else if !h.loaders.contains_key(&metric) {
            h.loaders.insert(metric.clone(), hub.series_loader(metric, from, to, 600, wake.clone()));
        }
    }
    if let Some(t) = h.cursor_ms {
        h.tops = hub.with_history(|hist| hist.tops_near(t));
    }
}

/// The charts of the History page.
pub fn history_metrics(frame: Option<&Frame>) -> Vec<Metric> {
    let mut out = vec![
        Metric::CpuTotal,
        Metric::MemPercent,
        Metric::SwapPercent,
        Metric::DiskRead,
        Metric::DiskWrite,
        Metric::NetRx,
        Metric::NetTx,
        Metric::MaxTemp,
        Metric::ProcessCount,
    ];
    if let Some(f) = frame {
        out.extend((0..f.gpus.len().min(8) as u16).map(Metric::GpuLoad));
    }
    out
}

/// Alerts from the hub become notifications.
fn alerts(app: &mut AppCore) {
    for a in Hub::global().take_alerts() {
        let title = labels::alert_title(&a);
        let mut toast = Toast::warning(title).body(labels::alert_body(&a));
        toast = toast.cmd(tr!("monitor-alert-open"), crate::cmd::Cmd::Action(soyuz_core::config::Action::OpenMonitor));
        app.ui.toasts.push(toast);
    }
}

/// `true` when the focused pane shows the monitor (input routing).
pub fn focused(app: &AppCore) -> bool {
    matches!(app.ws.pane().location, Location::Monitor)
}
