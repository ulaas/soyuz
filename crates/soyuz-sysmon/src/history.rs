//! History in memory, in three tiers: fine (every frame, 15 minutes, with per-core values),
//! medium (10 s averages and peaks, 6 hours) and coarse (60 s averages and peaks, at most 48 hours
//! in memory; older days stay on disk, see `store::load_series`). Plus [`ProcHistory`], a short
//! per-process trail.
//!
//! A chart's line breaks where a series holds a NaN value: after a stall, a suspend, a clock
//! jump, or a pause longer than three times the tier's step.
//!
//! `DiskRead` and friends are disk totals. Per-process I/O on Windows is different: it includes
//! pipes and sockets (see `ProcRow::read_rate`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use soyuz_fs::sysmon::ProcKey;

use crate::FORMAT_VERSION;
use crate::frame::{Frame, ProcRow};

const SECOND: i64 = 1000;
const FINE_SPAN_MS: i64 = 15 * 60 * SECOND;
const MEDIUM_STEP_MS: i64 = 10 * SECOND;
const MEDIUM_SPAN_MS: i64 = 6 * 3600 * SECOND;
const COARSE_STEP_MS: i64 = 60 * SECOND;
/// Coarse points kept in memory; older days are loaded from disk for the viewed range.
const COARSE_RAM_SPAN_MS: i64 = 48 * 3600 * SECOND;
const DAY_MS: i64 = 24 * 3600 * SECOND;
/// A clock stepping back by more than this rewrites the recent past instead of dropping every
/// frame until the clock has caught up.
const CLOCK_BACK_TOLERANCE_MS: i64 = 60 * SECOND;
const TOP_N: usize = 5;

/// Average and peak over a tier's step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Agg {
    #[serde(rename = "a", default)]
    pub avg: f32,
    #[serde(rename = "m", default)]
    pub max: f32,
}

/// One point of a series. Fine-tier points have `avg == max`. NaN in both: the line breaks here.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pt {
    pub t: i64,
    pub avg: f32,
    pub max: f32,
}

impl Pt {
    /// The drawn point of `pts` nearest to `t`, when there is one close enough to speak for that
    /// moment on a chart spanning `span_ms`: a cursor over a stretch with no recording has no value.
    pub fn near(pts: &[Pt], t: i64, span_ms: i64) -> Option<&Pt> {
        let reach = (span_ms / 40).max(15_000);
        let i = pts.partition_point(|p| p.t < t);
        [i.checked_sub(1), (i < pts.len()).then_some(i)]
            .into_iter()
            .flatten()
            .map(|k| &pts[k])
            .filter(|p| !p.is_break() && p.avg.is_finite() && (p.t - t).abs() <= reach)
            .min_by_key(|p| (p.t - t).abs())
    }

    pub fn is_break(&self) -> bool {
        self.avg.is_nan()
    }
}

/// A process in a top list.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TopProc {
    pub name: Arc<str>,
    pub pid: u32,
    /// Where 100 is one logical CPU, averaged over the step.
    pub cpu: f32,
    /// Peak resident bytes over the step.
    pub mem: u64,
}

/// The top processes of a medium or coarse point.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tops {
    pub by_cpu: Vec<TopProc>,
    pub by_mem: Vec<TopProc>,
}

/// One disk in a [`Point`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, bound(serialize = "V: Serialize", deserialize = "V: Deserialize<'de> + Default"))]
pub struct DiskPoint<V> {
    pub id: Arc<str>,
    pub busy: Option<V>,
    pub read: V,
    pub write: V,
}

/// One network interface in a [`Point`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, bound(serialize = "V: Serialize", deserialize = "V: Deserialize<'de> + Default"))]
pub struct NetPoint<V> {
    pub id: Arc<str>,
    pub rx: V,
    pub tx: V,
}

/// One GPU in a [`Point`]; `mem` in bytes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, bound(serialize = "V: Serialize", deserialize = "V: Deserialize<'de> + Default"))]
pub struct GpuPoint<V> {
    pub load: Option<V>,
    pub mem: Option<V>,
}

/// The recorded values at one time: `f32` in the fine tier ([`FinePoint`]), [`Agg`] in the
/// others ([`AggPoint`], which is also what a history file holds per line).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, bound(serialize = "V: Serialize", deserialize = "V: Deserialize<'de> + Default"))]
pub struct Point<V> {
    /// Format version on disk.
    pub v: u32,
    pub t: i64,
    /// The line breaks before this point.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub brk: bool,
    pub cpu: V,
    pub cpu_user: V,
    pub cpu_system: V,
    pub mem_percent: V,
    /// Bytes.
    pub mem_used: V,
    pub swap_percent: V,
    /// Bytes per second, over the disks and interfaces that count for totals.
    pub disk_read: V,
    pub disk_write: V,
    pub net_rx: V,
    pub net_tx: V,
    pub max_temp: Option<V>,
    pub process_count: V,
    /// Per logical CPU; the fine tier only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cores: Vec<V>,
    pub disks: Vec<DiskPoint<V>>,
    pub nets: Vec<NetPoint<V>>,
    pub gpus: Vec<GpuPoint<V>>,
    /// Medium and coarse points only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tops: Option<Tops>,
}

/// A fine-tier point.
pub type FinePoint = Point<f32>;
/// A medium or coarse point.
pub type AggPoint = Point<Agg>;

/// What a chart can draw.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Metric {
    CpuTotal,
    CpuUser,
    CpuSystem,
    /// One logical CPU; the fine tier only.
    Core(u16),
    MemPercent,
    /// Bytes.
    MemUsed,
    SwapPercent,
    /// Bytes per second over the disks that count for totals.
    DiskRead,
    DiskWrite,
    DiskReadOf(String),
    DiskWriteOf(String),
    DiskBusyOf(String),
    /// Bytes per second over the interfaces that count for totals.
    NetRx,
    NetTx,
    NetRxOf(String),
    NetTxOf(String),
    GpuLoad(u16),
    /// Bytes.
    GpuMem(u16),
    MaxTemp,
    ProcessCount,
}

impl Metric {
    fn fine_only(&self) -> bool {
        matches!(self, Metric::Core(_))
    }
}

impl<V: Copy> Point<V> {
    /// This point's value of a metric, when it has one.
    pub fn value(&self, m: &Metric) -> Option<V> {
        match m {
            Metric::CpuTotal => Some(self.cpu),
            Metric::CpuUser => Some(self.cpu_user),
            Metric::CpuSystem => Some(self.cpu_system),
            Metric::Core(i) => self.cores.get(*i as usize).copied(),
            Metric::MemPercent => Some(self.mem_percent),
            Metric::MemUsed => Some(self.mem_used),
            Metric::SwapPercent => Some(self.swap_percent),
            Metric::DiskRead => Some(self.disk_read),
            Metric::DiskWrite => Some(self.disk_write),
            Metric::DiskReadOf(id) => self.disks.iter().find(|d| *d.id == **id).map(|d| d.read),
            Metric::DiskWriteOf(id) => self.disks.iter().find(|d| *d.id == **id).map(|d| d.write),
            Metric::DiskBusyOf(id) => self.disks.iter().find(|d| *d.id == **id).and_then(|d| d.busy),
            Metric::NetRx => Some(self.net_rx),
            Metric::NetTx => Some(self.net_tx),
            Metric::NetRxOf(id) => self.nets.iter().find(|n| *n.id == **id).map(|n| n.rx),
            Metric::NetTxOf(id) => self.nets.iter().find(|n| *n.id == **id).map(|n| n.tx),
            Metric::GpuLoad(i) => self.gpus.get(*i as usize).and_then(|g| g.load),
            Metric::GpuMem(i) => self.gpus.get(*i as usize).and_then(|g| g.mem),
            Metric::MaxTemp => self.max_temp,
            Metric::ProcessCount => Some(self.process_count),
        }
    }
}

trait PairValue: Copy {
    fn pair(self) -> (f32, f32);
}

impl PairValue for f32 {
    fn pair(self) -> (f32, f32) {
        (self, self)
    }
}

impl PairValue for Agg {
    fn pair(self) -> (f32, f32) {
        (self.avg, self.max)
    }
}

/// Builds a series, bucket by bucket, keeping the peak of each bucket and breaking the line where asked.
pub(crate) struct Decimator {
    out: Vec<Pt>,
    from: i64,
    width: i64,
    bucket: Option<i64>,
    first_t: i64,
    sum: f64,
    max: f32,
    n: u32,
    break_pending: bool,
}

impl Decimator {
    pub(crate) fn new(from: i64, to: i64, max_points: usize) -> Self {
        let width = if max_points == 0 { 1 } else { ((to - from).max(1) / max_points as i64).max(1) };
        Decimator {
            out: Vec::with_capacity(max_points.min(4096)),
            from,
            width,
            bucket: None,
            first_t: 0,
            sum: 0.0,
            max: f32::MIN,
            n: 0,
            break_pending: false,
        }
    }

    fn flush(&mut self) {
        if self.n > 0 {
            self.out.push(Pt { t: self.first_t, avg: (self.sum / f64::from(self.n)) as f32, max: self.max });
        }
        self.n = 0;
        self.sum = 0.0;
        self.max = f32::MIN;
        self.bucket = None;
    }

    /// `value` None: this metric has no value here, which breaks the line.
    pub(crate) fn add(&mut self, t: i64, value: Option<(f32, f32)>, brk: bool) {
        self.break_pending |= brk;
        let Some((avg, max)) = value else {
            self.break_pending = true;
            return;
        };
        if self.break_pending {
            self.flush();
            if let Some(last) = self.out.last()
                && !last.is_break()
            {
                self.out.push(Pt { t: (t - 1).max(last.t), avg: f32::NAN, max: f32::NAN });
            }
            self.break_pending = false;
        }
        let bucket = (t - self.from).div_euclid(self.width);
        if self.bucket != Some(bucket) {
            self.flush();
            self.bucket = Some(bucket);
            self.first_t = t;
        }
        self.sum += f64::from(avg);
        self.max = self.max.max(max);
        self.n += 1;
    }

    pub(crate) fn finish(mut self) -> Vec<Pt> {
        self.flush();
        self.out
    }
}

fn collect_series<V: PairValue>(
    tier: &VecDeque<Point<V>>,
    m: &Metric,
    from: i64,
    to: i64,
    max_points: usize,
) -> Vec<Pt> {
    let start = tier.partition_point(|p| p.t < from);
    let mut d = Decimator::new(from, to, max_points);
    for p in tier.iter().skip(start).take_while(|p| p.t <= to) {
        d.add(p.t, p.value(m).map(PairValue::pair), p.brk);
    }
    d.finish()
}

/// `Acc` adds up a medium or coarse point.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Acc {
    sum: f64,
    max: f32,
    n: u32,
}

impl Default for Acc {
    fn default() -> Self {
        Acc { sum: 0.0, max: f32::MIN, n: 0 }
    }
}

impl Acc {
    fn add(&mut self, v: f32) {
        self.sum += f64::from(v);
        self.max = self.max.max(v);
        self.n += 1;
    }

    fn done(&self) -> Agg {
        if self.n == 0 { Agg::default() } else { Agg { avg: (self.sum / f64::from(self.n)) as f32, max: self.max } }
    }
}

fn add_opt(acc: &mut Option<Acc>, v: Option<f32>) {
    if let Some(v) = v {
        acc.get_or_insert_with(Acc::default).add(v);
    }
}

#[derive(Default)]
struct TopAcc {
    frames: u32,
    procs: HashMap<u32, TopEntry>,
}

struct TopEntry {
    name: Arc<str>,
    cpu_sum: f64,
    mem: u64,
}

impl TopAcc {
    /// One frame's two lists; a process in both counts once.
    fn add(&mut self, cpu_tops: &[&ProcRow], mem_tops: &[&ProcRow]) {
        self.frames += 1;
        let only_mem = mem_tops.iter().filter(|m| !cpu_tops.iter().any(|c| c.raw.key == m.raw.key));
        for row in cpu_tops.iter().chain(only_mem) {
            let e = self.procs.entry(row.raw.key.pid).or_insert_with(|| TopEntry {
                name: row.raw.name.clone(),
                cpu_sum: 0.0,
                mem: 0,
            });
            e.cpu_sum += f64::from(row.cpu_percent);
            e.mem = e.mem.max(row.raw.rss);
        }
    }

    fn done(&self, intern: &mut Interner) -> Tops {
        let frames = f64::from(self.frames.max(1));
        let mut list: Vec<TopProc> = self
            .procs
            .iter()
            .map(|(&pid, e)| TopProc { name: intern.get(&e.name), pid, cpu: (e.cpu_sum / frames) as f32, mem: e.mem })
            .collect();
        list.sort_by(|a, b| b.mem.cmp(&a.mem).then(a.pid.cmp(&b.pid)));
        let by_mem: Vec<TopProc> = list.iter().take(TOP_N).cloned().collect();
        list.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(a.pid.cmp(&b.pid)));
        list.truncate(TOP_N);
        Tops { by_cpu: list, by_mem }
    }
}

/// The rows with the largest `key`, largest first; the idle pseudo process is left out.
fn top_rows<'a>(procs: &'a [ProcRow], out: &mut Vec<&'a ProcRow>, key: impl Fn(&ProcRow) -> f64) {
    out.clear();
    for row in procs {
        if row.raw.idle {
            continue;
        }
        let v = key(row);
        if out.len() == TOP_N && v <= key(out[TOP_N - 1]) {
            continue;
        }
        let at = out.partition_point(|o| key(o) >= v);
        out.insert(at, row);
        out.truncate(TOP_N);
    }
}

/// Names and ids shared between points, since 48 hours of points repeat the same few strings.
#[derive(Default)]
struct Interner(HashSet<Arc<str>>);

impl Interner {
    fn get(&mut self, s: &str) -> Arc<str> {
        if let Some(a) = self.0.get(s) {
            return a.clone();
        }
        let a: Arc<str> = Arc::from(s);
        self.0.insert(a.clone());
        a
    }

    fn collect_garbage(&mut self) {
        self.0.retain(|a| Arc::strong_count(a) > 1);
    }

    fn rewrite(&mut self, p: &mut AggPoint) {
        for d in &mut p.disks {
            d.id = self.get(&d.id);
        }
        for n in &mut p.nets {
            n.id = self.get(&n.id);
        }
        if let Some(t) = &mut p.tops {
            for e in t.by_cpu.iter_mut().chain(&mut t.by_mem) {
                e.name = self.get(&e.name);
            }
        }
    }
}

struct Bucket {
    slot: i64,
    brk: bool,
    acc: Accum,
    tops: TopAcc,
}

/// A point being added up.
#[derive(Default)]
struct Accum {
    cpu: Acc,
    cpu_user: Acc,
    cpu_system: Acc,
    mem_percent: Acc,
    mem_used: Acc,
    swap_percent: Acc,
    disk_read: Acc,
    disk_write: Acc,
    net_rx: Acc,
    net_tx: Acc,
    max_temp: Option<Acc>,
    process_count: Acc,
    disks: Vec<(Arc<str>, Option<Acc>, Acc, Acc)>,
    nets: Vec<(Arc<str>, Acc, Acc)>,
    gpus: Vec<(Option<Acc>, Option<Acc>)>,
}

impl Accum {
    fn add(&mut self, p: &FinePoint) {
        self.cpu.add(p.cpu);
        self.cpu_user.add(p.cpu_user);
        self.cpu_system.add(p.cpu_system);
        self.mem_percent.add(p.mem_percent);
        self.mem_used.add(p.mem_used);
        self.swap_percent.add(p.swap_percent);
        self.disk_read.add(p.disk_read);
        self.disk_write.add(p.disk_write);
        self.net_rx.add(p.net_rx);
        self.net_tx.add(p.net_tx);
        add_opt(&mut self.max_temp, p.max_temp);
        self.process_count.add(p.process_count);
        for d in &p.disks {
            let at = match self.disks.iter().position(|e| e.0 == d.id) {
                Some(i) => i,
                None => {
                    self.disks.push((d.id.clone(), None, Acc::default(), Acc::default()));
                    self.disks.len() - 1
                }
            };
            let e = &mut self.disks[at];
            add_opt(&mut e.1, d.busy);
            e.2.add(d.read);
            e.3.add(d.write);
        }
        for n in &p.nets {
            let at = match self.nets.iter().position(|e| e.0 == n.id) {
                Some(i) => i,
                None => {
                    self.nets.push((n.id.clone(), Acc::default(), Acc::default()));
                    self.nets.len() - 1
                }
            };
            self.nets[at].1.add(n.rx);
            self.nets[at].2.add(n.tx);
        }
        if self.gpus.len() < p.gpus.len() {
            self.gpus.resize(p.gpus.len(), (None, None));
        }
        for (acc, g) in self.gpus.iter_mut().zip(&p.gpus) {
            add_opt(&mut acc.0, g.load);
            add_opt(&mut acc.1, g.mem);
        }
    }

    fn done(&self, t: i64, brk: bool, tops: Tops) -> AggPoint {
        Point {
            v: FORMAT_VERSION,
            t,
            brk,
            cpu: self.cpu.done(),
            cpu_user: self.cpu_user.done(),
            cpu_system: self.cpu_system.done(),
            mem_percent: self.mem_percent.done(),
            mem_used: self.mem_used.done(),
            swap_percent: self.swap_percent.done(),
            disk_read: self.disk_read.done(),
            disk_write: self.disk_write.done(),
            net_rx: self.net_rx.done(),
            net_tx: self.net_tx.done(),
            max_temp: self.max_temp.map(|a| a.done()),
            process_count: self.process_count.done(),
            cores: Vec::new(),
            disks: self
                .disks
                .iter()
                .map(|(id, busy, read, write)| DiskPoint {
                    id: id.clone(),
                    busy: busy.map(|a| a.done()),
                    read: read.done(),
                    write: write.done(),
                })
                .collect(),
            nets: self
                .nets
                .iter()
                .map(|(id, rx, tx)| NetPoint { id: id.clone(), rx: rx.done(), tx: tx.done() })
                .collect(),
            gpus: self
                .gpus
                .iter()
                .map(|(load, mem)| GpuPoint { load: load.map(|a| a.done()), mem: mem.map(|a| a.done()) })
                .collect(),
            tops: Some(tops),
        }
    }
}

/// The three tiers of recorded points, in memory.
pub struct History {
    days: u32,
    fine: VecDeque<FinePoint>,
    medium: VecDeque<AggPoint>,
    coarse: VecDeque<AggPoint>,
    medium_bucket: Option<Bucket>,
    coarse_bucket: Option<Bucket>,
    last_t: Option<i64>,
    last_dt: i64,
    pending_break: bool,
    disk_oldest: Option<i64>,
    intern: Interner,
    pushes: u32,
}

impl History {
    /// `days`: how long history is kept on disk. Memory holds at most 48 hours of it.
    pub fn new(days: u32) -> Self {
        History {
            days,
            fine: VecDeque::new(),
            medium: VecDeque::new(),
            coarse: VecDeque::new(),
            medium_bucket: None,
            coarse_bucket: None,
            last_t: None,
            last_dt: 0,
            pending_break: false,
            disk_oldest: None,
            intern: Interner::default(),
            pushes: 0,
        }
    }

    /// Changes how long history is kept.
    pub fn set_days(&mut self, days: u32) {
        self.days = days;
    }

    fn coarse_span(&self) -> i64 {
        (i64::from(self.days.max(1)) * DAY_MS).min(COARSE_RAM_SPAN_MS)
    }

    /// Records a frame. `first` and `gap` frames hold no valid rates: they are skipped and
    /// break the line instead. A frame not newer than the last one is dropped (clock steps
    /// must not break the time order). Returns a finished coarse point, for the disk.
    pub fn push(&mut self, frame: &Frame) -> Option<AggPoint> {
        self.push_as(frame, true)
    }

    /// Add a frame. `record` off keeps it for the live graphs only (the fine tier): the longer
    /// tiers and the disk get nothing, and half-filled buckets are dropped, so that a pause leaves
    /// a gap and not a blend of the moments on either side of it.
    pub fn push_as(&mut self, frame: &Frame, record: bool) -> Option<AggPoint> {
        if frame.first || frame.gap {
            self.pending_break = true;
            return None;
        }
        let t = frame.at_ms;
        if let Some(last) = self.last_t
            && t <= last
        {
            if last - t <= CLOCK_BACK_TOLERANCE_MS {
                return None;
            }
            self.rewind(t);
        }
        let dt = self.last_t.map_or(0, |l| t - l);
        let brk = std::mem::take(&mut self.pending_break) || (self.last_dt > 0 && dt > 3 * self.last_dt);
        if dt > 0 {
            self.last_dt = dt;
        }
        self.last_t = Some(t);

        let point = self.fine_point(frame, brk);

        let mut coarse_done = None;
        if record {
            let mut cpu_tops: Vec<&ProcRow> = Vec::with_capacity(TOP_N + 1);
            let mut mem_tops: Vec<&ProcRow> = Vec::with_capacity(TOP_N + 1);
            top_rows(&frame.procs, &mut cpu_tops, |r| f64::from(r.cpu_percent));
            top_rows(&frame.procs, &mut mem_tops, |r| r.raw.rss as f64);
            let tops = (cpu_tops.as_slice(), mem_tops.as_slice());
            if let Some(p) = Self::roll(&mut self.medium_bucket, t, MEDIUM_STEP_MS, &point, tops, &mut self.intern) {
                Self::append(&mut self.medium, p, MEDIUM_STEP_MS, self.last_dt, MEDIUM_SPAN_MS);
            }
            if let Some(p) = Self::roll(&mut self.coarse_bucket, t, COARSE_STEP_MS, &point, tops, &mut self.intern) {
                let span = self.coarse_span();
                coarse_done = Self::append(&mut self.coarse, p, COARSE_STEP_MS, self.last_dt, span);
            }
        } else {
            self.medium_bucket = None;
            self.coarse_bucket = None;
        }

        self.fine.push_back(point);
        while self.fine.front().is_some_and(|p| p.t < t - FINE_SPAN_MS) {
            self.fine.pop_front();
        }

        self.pushes = self.pushes.wrapping_add(1);
        if self.pushes.is_multiple_of(4096) {
            self.intern.collect_garbage();
        }
        coarse_done
    }

    fn fine_point(&mut self, frame: &Frame, brk: bool) -> FinePoint {
        let (disk_read, disk_write) = frame.disk_totals();
        let (net_rx, net_tx) = frame.net_totals();
        let intern = &mut self.intern;
        Point {
            v: FORMAT_VERSION,
            t: frame.at_ms,
            brk,
            cpu: frame.cpu.percent,
            cpu_user: frame.cpu.user,
            cpu_system: frame.cpu.system,
            mem_percent: frame.memory.percent,
            mem_used: frame.memory.used as f32,
            swap_percent: frame.memory.swap_percent,
            disk_read: disk_read as f32,
            disk_write: disk_write as f32,
            net_rx: net_rx as f32,
            net_tx: net_tx as f32,
            max_temp: frame.max_temp_c(),
            process_count: frame.counts.processes as f32,
            cores: frame.cpu.cores.iter().map(|c| c.percent).collect(),
            disks: frame
                .disks
                .iter()
                .map(|d| DiskPoint {
                    id: intern.get(&d.id),
                    busy: d.busy_percent,
                    read: d.read_rate as f32,
                    write: d.write_rate as f32,
                })
                .collect(),
            nets: frame
                .nets
                .iter()
                .map(|n| NetPoint { id: intern.get(&n.id), rx: n.rx_rate as f32, tx: n.tx_rate as f32 })
                .collect(),
            gpus: frame.gpus.iter().map(|g| GpuPoint { load: g.percent, mem: g.mem_used.map(|m| m as f32) }).collect(),
            tops: None,
        }
    }

    /// Adds to the tier's open bucket; returns the previous bucket when `t` entered a new one.
    fn roll(
        bucket: &mut Option<Bucket>,
        t: i64,
        step: i64,
        point: &FinePoint,
        tops: (&[&ProcRow], &[&ProcRow]),
        intern: &mut Interner,
    ) -> Option<AggPoint> {
        let slot = t.div_euclid(step);
        let mut done = None;
        if bucket.as_ref().is_some_and(|b| b.slot != slot)
            && let Some(old) = bucket.take()
        {
            done = Some(old.acc.done(old.slot * step, old.brk, old.tops.done(intern)));
        }
        let b =
            bucket.get_or_insert_with(|| Bucket { slot, brk: false, acc: Accum::default(), tops: TopAcc::default() });
        b.brk |= point.brk;
        b.acc.add(point);
        b.tops.add(tops.0, tops.1);
        done
    }

    /// Adds a finished point in time order, marks a pause, and evicts what is too old.
    fn append(tier: &mut VecDeque<AggPoint>, mut p: AggPoint, step: i64, last_dt: i64, span: i64) -> Option<AggPoint> {
        if let Some(last) = tier.back() {
            if p.t <= last.t {
                return None;
            }
            if p.t - last.t > 3 * step.max(last_dt) {
                p.brk = true;
            }
        }
        let t = p.t;
        tier.push_back(p.clone());
        while tier.front().is_some_and(|o| o.t < t - span) {
            tier.pop_front();
        }
        Some(p)
    }

    /// The clock went back far enough to matter: forget what now lies in the future.
    fn rewind(&mut self, t: i64) {
        self.fine.retain(|p| p.t < t);
        self.medium.retain(|p| p.t < t);
        self.coarse.retain(|p| p.t < t);
        self.medium_bucket = None;
        self.coarse_bucket = None;
        self.last_t = self.fine.back().map(|p| p.t);
        self.pending_break = true;
    }

    /// Up to about `max_points` values of `metric` between the two times, from the finest
    /// tier that reaches `from_ms`; several values in one bucket become their average with
    /// the highest peak. NaN values break the line. Empty when no tier holds the metric.
    /// Ranges older than memory: `store::load_series` on a `Loader`, or `Hub::series_loader`.
    pub fn series(&self, metric: &Metric, from_ms: i64, to_ms: i64, max_points: usize) -> Vec<Pt> {
        if metric.fine_only() {
            return collect_series(&self.fine, metric, from_ms, to_ms, max_points);
        }
        let reaches = |oldest: Option<i64>| oldest.is_some_and(|o| o <= from_ms);
        let (fine, medium, coarse) =
            (self.fine.front().map(|p| p.t), self.medium.front().map(|p| p.t), self.coarse.front().map(|p| p.t));
        if reaches(fine) {
            return collect_series(&self.fine, metric, from_ms, to_ms, max_points);
        }
        if reaches(medium) {
            return collect_series(&self.medium, metric, from_ms, to_ms, max_points);
        }
        if reaches(coarse) {
            return collect_series(&self.coarse, metric, from_ms, to_ms, max_points);
        }
        // No tier reaches back that far: the finest one, unless a coarser tier really reaches
        // further than its own bucket (whose start is older than the first sample in it).
        // Otherwise a fresh run would draw 10 s buckets for its first minutes.
        let beats = |coarser: Option<i64>, finer: Option<i64>, step: i64| match (coarser, finer) {
            (Some(c), Some(f)) => f - c > step,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if fine.is_some() && !beats(medium, fine, MEDIUM_STEP_MS) && !beats(coarse, fine, COARSE_STEP_MS) {
            collect_series(&self.fine, metric, from_ms, to_ms, max_points)
        } else if medium.is_some() && !beats(coarse, medium, COARSE_STEP_MS) {
            collect_series(&self.medium, metric, from_ms, to_ms, max_points)
        } else {
            collect_series(&self.coarse, metric, from_ms, to_ms, max_points)
        }
    }

    /// The top processes around a time (within 5 minutes), from the finest tier that has it.
    pub fn tops_near(&self, t_ms: i64) -> Option<Tops> {
        const REACH: i64 = 5 * 60 * SECOND;
        let nearest = |tier: &VecDeque<AggPoint>| -> Option<Tops> {
            let at = tier.partition_point(|p| p.t < t_ms);
            let after = tier.get(at);
            let before = at.checked_sub(1).and_then(|i| tier.get(i));
            let best = match (before, after) {
                (Some(b), Some(a)) => {
                    if t_ms - b.t <= a.t - t_ms {
                        b
                    } else {
                        a
                    }
                }
                (Some(b), None) => b,
                (None, Some(a)) => a,
                (None, None) => return None,
            };
            if (best.t - t_ms).abs() <= REACH { best.tops.clone() } else { None }
        };
        let medium_covers = self.medium.front().is_some_and(|p| p.t <= t_ms + MEDIUM_STEP_MS);
        if medium_covers && let Some(t) = nearest(&self.medium) {
            return Some(t);
        }
        nearest(&self.coarse).or_else(|| nearest(&self.medium))
    }

    /// Oldest and newest time held in memory.
    pub fn range(&self) -> Option<(i64, i64)> {
        let oldest =
            [self.fine.front().map(|p| p.t), self.medium.front().map(|p| p.t), self.coarse.front().map(|p| p.t)]
                .into_iter()
                .flatten()
                .min()?;
        let newest = [self.fine.back().map(|p| p.t), self.medium.back().map(|p| p.t), self.coarse.back().map(|p| p.t)]
            .into_iter()
            .flatten()
            .max()?;
        Some((oldest, newest))
    }

    /// Like [`History::range`], also counting what is on disk.
    pub fn full_range(&self) -> Option<(i64, i64)> {
        let (oldest, newest) = self.range()?;
        Some((self.disk_oldest.map_or(oldest, |d| d.min(oldest)), newest))
    }

    /// The oldest moment actually recorded: the long tiers and the disk, not the live graphs' points.
    pub fn recorded_from(&self) -> Option<i64> {
        [self.medium.front().map(|p| p.t), self.coarse.front().map(|p| p.t), self.disk_oldest]
            .into_iter()
            .flatten()
            .min()
    }

    /// Records how far the files on disk reach back (see `store::oldest`).
    pub fn set_disk_oldest(&mut self, t: Option<i64>) {
        self.disk_oldest = t;
    }

    /// Drops points too old for their tier, measured from `now_ms`.
    pub fn prune(&mut self, now_ms: i64) {
        let coarse_span = self.coarse_span();
        while self.fine.front().is_some_and(|p| p.t < now_ms - FINE_SPAN_MS) {
            self.fine.pop_front();
        }
        while self.medium.front().is_some_and(|p| p.t < now_ms - MEDIUM_SPAN_MS) {
            self.medium.pop_front();
        }
        while self.coarse.front().is_some_and(|p| p.t < now_ms - coarse_span) {
            self.coarse.pop_front();
        }
        self.intern.collect_garbage();
    }

    /// Adds coarse points loaded from disk, older than what was recorded since. Keeps time
    /// order, whatever the file held.
    pub fn load_coarse(&mut self, points: Vec<AggPoint>) {
        let live: Vec<AggPoint> = self.coarse.drain(..).collect();
        let first_live = live.first().map(|p| p.t);
        let older = points.into_iter().filter(|p| first_live.is_none_or(|l| p.t < l));
        for mut p in older.chain(live) {
            if self.coarse.back().is_none_or(|last| p.t > last.t) {
                self.intern.rewrite(&mut p);
                self.coarse.push_back(p);
            }
        }
        if let Some(newest) = self.coarse.back().map(|p| p.t) {
            let span = self.coarse_span();
            while self.coarse.front().is_some_and(|p| p.t < newest - span) {
                self.coarse.pop_front();
            }
        }
    }

    /// Empties the memory tiers, keeping the settings.
    pub fn clear(&mut self) {
        *self = History::new(self.days);
    }

    /// Drop the long tier from memory (`load_coarse` brings it back from disk).
    pub fn release_coarse(&mut self) {
        self.coarse.clear();
    }
}

const PROC_CAP: usize = 300;
const PROC_GRACE_MS: i64 = 120 * SECOND;

/// One sample of [`ProcHistory::series`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProcSample {
    pub t_ms: i64,
    /// Where 100 is one logical CPU.
    pub cpu_percent: f32,
    /// Resident bytes, to a kibibyte.
    pub rss: u64,
}

struct ProcTrail {
    /// (cpu percent, resident KiB): 8 bytes per sample.
    samples: VecDeque<(f32, u32)>,
    last_seq: u64,
    last_seen_ms: i64,
}

/// A short trail per process: the latest 300 frames, for processes in the last frame, plus two
/// minutes after one exits. Times are shared per frame, so a sample costs 8 bytes.
#[derive(Default)]
pub struct ProcHistory {
    seq: u64,
    times: VecDeque<i64>,
    procs: HashMap<ProcKey, ProcTrail>,
}

impl ProcHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a frame; `first` and `gap` frames have no valid rates and are skipped.
    pub fn push(&mut self, frame: &Frame) {
        if frame.first || frame.gap || frame.procs.is_empty() {
            return;
        }
        self.seq += 1;
        let seq = self.seq;
        self.times.push_back(frame.at_ms);
        if self.times.len() > PROC_CAP {
            self.times.pop_front();
        }
        for row in frame.procs.iter().filter(|r| !r.raw.idle) {
            let trail = self.procs.entry(row.raw.key).or_insert_with(|| ProcTrail {
                samples: VecDeque::with_capacity(8),
                last_seq: seq,
                last_seen_ms: frame.at_ms,
            });
            if trail.last_seq + 1 != seq {
                trail.samples.clear(); // gone from view for a while: indices would no longer line up
            }
            if trail.samples.len() == PROC_CAP {
                trail.samples.pop_front();
            }
            let kib = (row.raw.rss / 1024).min(u64::from(u32::MAX)) as u32;
            trail.samples.push_back((row.cpu_percent, kib));
            trail.last_seq = seq;
            trail.last_seen_ms = frame.at_ms;
        }
        if seq.is_multiple_of(16) {
            let cutoff = frame.at_ms - PROC_GRACE_MS;
            self.procs.retain(|_, t| t.last_seen_ms >= cutoff);
        }
    }

    /// The trail of one process, oldest first; empty when unknown.
    pub fn series(&self, key: ProcKey) -> Vec<ProcSample> {
        let Some(trail) = self.procs.get(&key) else { return Vec::new() };
        let newest_seq = self.seq;
        let oldest_time_seq = newest_seq + 1 - self.times.len() as u64;
        let first_seq = trail.last_seq + 1 - trail.samples.len() as u64;
        trail
            .samples
            .iter()
            .enumerate()
            .filter_map(|(i, &(cpu_percent, kib))| {
                let seq = first_seq + i as u64;
                let t_ms = *self.times.get(seq.checked_sub(oldest_time_seq)? as usize)?;
                Some(ProcSample { t_ms, cpu_percent, rss: u64::from(kib) * 1024 })
            })
            .collect()
    }

    /// Processes held.
    pub fn len(&self) -> usize {
        self.procs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.procs.is_empty()
    }

    pub fn clear(&mut self) {
        self.procs.clear();
        self.times.clear();
    }
}

#[cfg(test)]
mod near_tests {
    use super::Pt;

    #[test]
    fn near_finds_close_points_only() {
        let pts = [
            Pt { t: 0, avg: 1.0, max: 1.0 },
            Pt { t: 10_000, avg: f32::NAN, max: f32::NAN },
            Pt { t: 20_000, avg: 3.0, max: 3.0 },
        ];
        assert_eq!(Pt::near(&pts, 2_000, 60_000).map(|p| p.t), Some(0));
        assert_eq!(Pt::near(&pts, 19_000, 60_000).map(|p| p.t), Some(20_000));
        // Long after the last point, as when pointing at a time before recording began.
        assert!(Pt::near(&pts, 3_000_000, 3_600_000).is_none());
        assert!(Pt::near(&pts, 200_000, 3_600_000).is_none());
        assert!(Pt::near(&[], 5, 60_000).is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{CoreFrame, DiskFrame, MemFrame, NetFrame};
    use crate::testutil::{frame_of, key, row};

    #[test]
    fn a_fresh_run_reads_the_fine_tier() {
        let mut h = History::new(1);
        // The medium bucket starts before the first sample; that alone is no longer reach.
        for i in 0..8 {
            let mut f = frame(0, 10.0);
            f.at_ms = 100_005_000 + i * 1000;
            h.push(&f);
        }
        let pts = h.series(&Metric::CpuTotal, 100_012_000 - 120_000, 100_012_000, 0);
        assert_eq!(pts.iter().filter(|p| !p.is_break()).count(), 8);
    }

    fn frame(t: i64, cpu: f32) -> Frame {
        let mut f = frame_of(vec![row(1, None, "a", cpu, 1000), row(2, None, "b", cpu / 2.0, 5000)]);
        f.at_ms = t;
        f.cpu.percent = cpu;
        f.cpu.cores = vec![CoreFrame { percent: cpu, ..Default::default() }];
        f.memory = MemFrame { used: 1000, percent: 25.0, ..Default::default() };
        f.disks = vec![DiskFrame { id: "d0".into(), read_rate: 100.0, busy_percent: Some(5.0), ..Default::default() }];
        f.nets = vec![NetFrame { id: "n0".into(), rx_rate: 10.0, up: true, ..Default::default() }];
        f
    }

    fn values(s: &[Pt]) -> Vec<f32> {
        s.iter().map(|p| p.avg).collect()
    }

    const T0: i64 = 1_000_000_000_000 - 1_000_000_000_000 % 60_000;

    #[test]
    fn fine_tier_returns_each_frame_when_it_fits() {
        let mut h = History::new(7);
        for i in 0..5 {
            h.push(&frame(T0 + i * 1000, i as f32));
        }
        let s = h.series(&Metric::CpuTotal, T0, T0 + 4000, 100);
        assert_eq!(values(&s), [0.0, 1.0, 2.0, 3.0, 4.0]);
        assert_eq!(s[1].t, T0 + 1000);
        assert_eq!(s[3].max, 3.0, "a fine point's peak is its value");
        assert_eq!(h.range(), Some((T0, T0 + 4000)));
    }

    #[test]
    fn first_and_gap_frames_are_skipped_and_break_the_line() {
        let mut h = History::new(7);
        let mut f = frame(T0, 50.0);
        f.first = true;
        h.push(&f);
        h.push(&frame(T0 + 1000, 1.0));
        h.push(&frame(T0 + 2000, 2.0));
        let mut g = frame(T0 + 3000, 99.0);
        g.gap = true;
        h.push(&g);
        h.push(&frame(T0 + 4000, 4.0));
        let s = h.series(&Metric::CpuTotal, T0, T0 + 5000, 100);
        assert_eq!(s.len(), 4, "{s:?}");
        assert_eq!(s[0].avg, 1.0);
        assert_eq!(s[1].avg, 2.0);
        assert!(s[2].is_break());
        assert_eq!(s[3].avg, 4.0);
    }

    #[test]
    fn a_pause_longer_than_three_steps_breaks_the_line() {
        let mut h = History::new(7);
        h.push(&frame(T0, 1.0));
        h.push(&frame(T0 + 1000, 2.0));
        h.push(&frame(T0 + 2500, 3.0)); // a little late: no break
        h.push(&frame(T0 + 60_000, 4.0));
        let s = h.series(&Metric::CpuTotal, T0, T0 + 60_000, 100);
        assert_eq!(s.iter().filter(|p| p.is_break()).count(), 1, "{s:?}");
        assert!(s[3].is_break());
        assert_eq!(s[4].avg, 4.0);
    }

    #[test]
    fn decimation_averages_and_keeps_peaks() {
        let mut h = History::new(7);
        for i in 0..100 {
            let v = if i == 42 { 90.0 } else { 10.0 };
            h.push(&frame(T0 + i * 1000, v));
        }
        let s = h.series(&Metric::CpuTotal, T0, T0 + 99_000, 10);
        assert!(s.len() <= 12, "{}", s.len());
        let peak = s.iter().find(|p| p.max == 90.0).expect("the peak survives");
        assert!(peak.avg < 20.0, "but is not the average: {}", peak.avg);
    }

    #[test]
    fn finer_tiers_expire_and_coarser_ones_take_over() {
        let mut h = History::new(7);
        // 20 minutes at 1 Hz.
        for i in 0..1200 {
            h.push(&frame(T0 + i * 1000, (i % 10) as f32));
        }
        let now = T0 + 1199 * 1000;
        assert!(h.fine.front().unwrap().t >= now - FINE_SPAN_MS);
        // The last five minutes come from the fine tier, every frame.
        let recent = h.series(&Metric::CpuTotal, now - 300_000, now, 0);
        assert_eq!(recent.len(), 301);
        // The whole 20 minutes do not fit the fine tier: 10 s points.
        let all = h.series(&Metric::CpuTotal, T0, now, 0);
        assert!((115..=121).contains(&all.len()), "{}", all.len());
        assert!(all.iter().all(|p| p.max == 9.0), "each 10 s holds a full cycle");
        assert!(all.iter().all(|p| (p.avg - 4.5).abs() < 0.01), "{:?}", all[0]);
        // Per-core values exist only in the fine tier.
        assert_eq!(h.series(&Metric::Core(0), T0, now, 0).first().map(|p| p.t), h.fine.front().map(|p| p.t));
        assert!(h.medium.back().unwrap().cores.is_empty());
    }

    #[test]
    fn coarse_tier_is_limited_to_48_hours_in_memory() {
        let mut h = History::new(90);
        let mut t = T0;
        let mut done = 0;
        // Sparse frames, so the test stays fast.
        while t < T0 + 50 * 3600 * 1000 {
            if h.push(&frame(t, 1.0)).is_some() {
                done += 1;
            }
            t += 60_000;
        }
        let oldest = h.coarse.front().unwrap().t;
        let newest = h.coarse.back().unwrap().t;
        assert_eq!(newest - oldest, COARSE_RAM_SPAN_MS);
        assert_eq!(done, 50 * 60 - 1, "each finished minute is handed out for the disk");
        assert!(h.medium.back().unwrap().t - h.medium.front().unwrap().t <= MEDIUM_SPAN_MS);
    }

    #[test]
    fn days_limit_the_coarse_tier_when_shorter() {
        let mut h = History::new(1);
        assert_eq!(h.coarse_span(), DAY_MS);
        h.set_days(90);
        assert_eq!(h.coarse_span(), COARSE_RAM_SPAN_MS);
    }

    #[test]
    fn frames_from_the_past_are_dropped_and_a_big_step_back_rewinds() {
        let mut h = History::new(7);
        h.push(&frame(T0, 1.0));
        h.push(&frame(T0 + 1000, 2.0));
        h.push(&frame(T0 + 1000, 3.0));
        h.push(&frame(T0 + 500, 4.0));
        assert_eq!(values(&h.series(&Metric::CpuTotal, T0, T0 + 2000, 0)), [1.0, 2.0]);
        // An hour back: the line goes on from the new time, in order.
        let back = T0 - 3_600_000;
        h.push(&frame(back, 5.0));
        h.push(&frame(back + 1000, 6.0));
        let s = h.series(&Metric::CpuTotal, back, T0 + 2000, 0);
        assert_eq!(values(&s), [5.0, 6.0]);
    }

    #[test]
    fn missing_metrics_give_an_empty_series() {
        let mut h = History::new(7);
        h.push(&frame(T0, 1.0));
        assert!(h.series(&Metric::GpuLoad(0), T0, T0 + 1000, 0).is_empty());
        assert!(h.series(&Metric::DiskReadOf("nope".into()), T0, T0 + 1000, 0).is_empty());
        assert_eq!(h.series(&Metric::DiskReadOf("d0".into()), T0, T0 + 1000, 0)[0].avg, 100.0);
        assert_eq!(h.series(&Metric::DiskBusyOf("d0".into()), T0, T0 + 1000, 0)[0].avg, 5.0);
        assert_eq!(h.series(&Metric::NetRxOf("n0".into()), T0, T0 + 1000, 0)[0].avg, 10.0);
        assert_eq!(h.series(&Metric::ProcessCount, T0, T0 + 1000, 0)[0].avg, 0.0);
    }

    #[test]
    fn a_disk_that_appears_later_breaks_only_its_own_series() {
        let mut h = History::new(7);
        let mut a = frame(T0, 1.0);
        a.disks.clear();
        h.push(&a);
        h.push(&frame(T0 + 1000, 1.0));
        let mut c = frame(T0 + 2000, 1.0);
        c.disks.clear();
        h.push(&c);
        h.push(&frame(T0 + 3000, 1.0));
        let s = h.series(&Metric::DiskReadOf("d0".into()), T0, T0 + 3000, 0);
        assert_eq!(s.len(), 3);
        assert!(s[1].is_break());
        assert_eq!(h.series(&Metric::CpuTotal, T0, T0 + 3000, 0).len(), 4);
    }

    #[test]
    fn tops_hold_five_and_leave_out_idle() {
        let mut h = History::new(7);
        let mut rows: Vec<ProcRow> =
            (1..=8).map(|i| row(i, None, &format!("p{i}"), i as f32, 100 - u64::from(i))).collect();
        let mut idle = row(99, None, "Idle", 1000.0, 1 << 30);
        idle.raw.idle = true;
        rows.push(idle);
        for i in 0..11 {
            let mut f = frame_of(rows.clone());
            f.at_ms = T0 + i * 1000;
            h.push(&f);
        }
        let tops = h.tops_near(T0 + 3000).expect("a medium point exists");
        assert_eq!(tops.by_cpu.len(), 5);
        assert_eq!(tops.by_cpu[0].pid, 8);
        assert_eq!(&*tops.by_cpu[0].name, "p8");
        assert_eq!(tops.by_cpu[0].cpu, 8.0);
        assert_eq!(tops.by_mem[0].pid, 1);
        assert_eq!(tops.by_mem[0].mem, 99);
        assert!(tops.by_cpu.iter().chain(&tops.by_mem).all(|t| t.pid != 99));
        assert_eq!(h.tops_near(T0 + 3_600_000), None, "too far away");
    }

    #[test]
    fn names_are_shared_between_points() {
        let mut h = History::new(7);
        for i in 0..25 {
            h.push(&frame(T0 + i * 1000, 1.0));
        }
        let a = h.medium[0].tops.as_ref().unwrap();
        let b = h.medium[1].tops.as_ref().unwrap();
        assert!(Arc::ptr_eq(&a.by_cpu[0].name, &b.by_cpu[0].name));
        assert!(Arc::ptr_eq(&h.medium[0].disks[0].id, &h.medium[1].disks[0].id));
    }

    #[test]
    fn loaded_points_go_before_the_live_ones_in_order() {
        let mut h = History::new(7);
        for i in 0..130 {
            h.push(&frame(T0 + 300_000 + i * 1000, 1.0));
        }
        let live = h.coarse.front().unwrap().t;
        let older = AggPoint { t: T0, v: 1, ..Default::default() };
        let dup = AggPoint { t: live, v: 1, ..Default::default() };
        h.load_coarse(vec![older, dup]);
        assert_eq!(h.coarse.front().unwrap().t, T0);
        assert!(h.coarse.iter().zip(h.coarse.iter().skip(1)).all(|(a, b)| a.t < b.t));
        assert_eq!(h.range().unwrap().0, T0);
        h.set_disk_oldest(Some(T0 - 5));
        assert_eq!(h.full_range().unwrap().0, T0 - 5);
    }

    #[test]
    fn prune_drops_old_points() {
        let mut h = History::new(7);
        h.push(&frame(T0, 1.0));
        h.push(&frame(T0 + 1000, 1.0));
        h.prune(T0 + FINE_SPAN_MS + 500);
        assert_eq!(h.fine.len(), 1);
        h.clear();
        assert_eq!(h.range(), None);
    }

    #[test]
    fn proc_history_is_capped_and_shares_times() {
        let mut p = ProcHistory::new();
        for i in 0..400 {
            let mut f = frame(T0 + i * 1000, i as f32);
            f.at_ms = T0 + i * 1000;
            p.push(&f);
        }
        let s = p.series(key(1, 100));
        assert_eq!(s.len(), PROC_CAP);
        assert_eq!(s[0].t_ms, T0 + 100 * 1000);
        assert_eq!(s[0].cpu_percent, 100.0);
        assert_eq!(s.last().unwrap().t_ms, T0 + 399 * 1000);
        assert_eq!(s[0].rss, 0, "1000 bytes rounds down to whole kibibytes");
        assert!(p.series(key(1, 999)).is_empty(), "a reused PID is another process");
    }

    #[test]
    fn proc_history_drops_exited_processes_after_the_grace() {
        let mut p = ProcHistory::new();
        for i in 0..20 {
            p.push(&frame(T0 + i * 1000, 1.0));
        }
        assert_eq!(p.len(), 2);
        for i in 0..200 {
            let mut f = frame_of(vec![row(3, None, "c", 1.0, 1)]);
            f.at_ms = T0 + 20_000 + i * 1000;
            p.push(&f);
            if i == 30 {
                assert_eq!(p.series(key(1, 100)).len(), 20, "still there during the grace");
            }
        }
        assert!(p.series(key(1, 100)).is_empty());
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn a_process_returning_after_a_hole_restarts_its_trail() {
        let mut p = ProcHistory::new();
        p.push(&frame(T0, 1.0));
        let mut other = frame_of(vec![row(3, None, "c", 1.0, 1)]);
        other.at_ms = T0 + 1000;
        p.push(&other);
        p.push(&frame(T0 + 2000, 2.0));
        let s = p.series(key(1, 100));
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].t_ms, T0 + 2000);
    }

    #[test]
    fn agg_points_round_trip_through_json() {
        let mut h = History::new(7);
        let mut done = None;
        for i in 0..70 {
            if let Some(p) = h.push(&frame(T0 + i * 1000, 1.0)) {
                done = Some(p);
            }
        }
        let p = done.expect("a minute went by");
        let line = serde_json::to_string(&p).unwrap();
        assert!(line.contains("\"v\":1"));
        let back: AggPoint = serde_json::from_str(&line).unwrap();
        assert_eq!(back, p);
        // Unknown and missing fields are fine: the format can grow.
        let lenient: AggPoint = serde_json::from_str(r#"{"t":5,"future":1,"cpu":{"a":2.0}}"#).unwrap();
        assert_eq!(lenient.t, 5);
        assert_eq!(lenient.cpu.avg, 2.0);
    }
}
