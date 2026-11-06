//! Compares two frames (usually two snapshots): machine totals and every process, matched by
//! identity or by program.

use std::collections::HashMap;
use std::sync::Arc;

use soyuz_fs::sysmon::{GpuRaw, ProcKey};

use crate::frame::{Frame, ProcRow};

/// How processes of the two frames are paired.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MatchBy {
    /// The same process: its `ProcKey`.
    #[default]
    Process,
    /// The same program (executable path, or name when unknown): instances are summed.
    Program,
}

/// How a process differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Change {
    Added,
    Removed,
    Changed,
    Same,
}

/// What counts as jitter rather than a change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tolerance {
    /// CPU and GPU: the difference in percentage points.
    pub cpu_points: f32,
    /// Sizes and counts: the difference as a share of the first value (0.02 = 2 %).
    pub rel: f32,
    /// Sizes: differences up to this many bytes are always the same.
    pub bytes_floor: u64,
}

impl Default for Tolerance {
    fn default() -> Self {
        Tolerance { cpu_points: 1.0, rel: 0.02, bytes_floor: 1 << 20 }
    }
}

impl Tolerance {
    /// Any difference counts.
    pub fn exact() -> Self {
        Tolerance { cpu_points: 0.0, rel: 0.0, bytes_floor: 0 }
    }
}

/// The compared values of one process (or one program's sum).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DiffValues {
    /// Where 100 is one logical CPU.
    pub cpu_percent: f32,
    pub rss: u64,
    pub private: Option<u64>,
    pub threads: u64,
    pub handles: Option<u64>,
    pub read_bytes: Option<u64>,
    pub write_bytes: Option<u64>,
    pub gpu_percent: Option<f32>,
    pub gpu_mem: Option<u64>,
}

fn add<T: std::ops::Add<Output = T> + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a + b),
        (a, None) => a,
        (None, b) => b,
    }
}

impl DiffValues {
    fn add(&mut self, row: &ProcRow) {
        let r = &row.raw;
        self.cpu_percent += row.cpu_percent;
        self.rss = self.rss.saturating_add(r.rss);
        self.private = add(self.private, r.private);
        self.threads += u64::from(r.threads);
        self.handles = add(self.handles, r.handles.map(u64::from));
        self.read_bytes = add(self.read_bytes, r.read_bytes);
        self.write_bytes = add(self.write_bytes, r.write_bytes);
        self.gpu_percent = add(self.gpu_percent, r.gpu_percent);
        self.gpu_mem = add(self.gpu_mem, r.gpu_mem);
    }

    fn differs(&self, o: &DiffValues, tol: &Tolerance) -> bool {
        let size = |a: u64, b: u64| a.abs_diff(b) as f64 > (f64::from(tol.rel) * a as f64).max(tol.bytes_floor as f64);
        let opt_size = |a: Option<u64>, b: Option<u64>| matches!((a, b), (Some(a), Some(b)) if size(a, b));
        // A count changing by one is ordinary churn when there is any tolerance at all.
        let count = |a: u64, b: u64| {
            let slack = (f64::from(tol.rel) * a as f64).max(if tol.rel > 0.0 { 1.0 } else { 0.0 });
            a.abs_diff(b) as f64 > slack
        };
        let opt_count = |a: Option<u64>, b: Option<u64>| matches!((a, b), (Some(a), Some(b)) if count(a, b));
        let points = |a: f32, b: f32| (a - b).abs() > tol.cpu_points;
        points(self.cpu_percent, o.cpu_percent)
            || matches!((self.gpu_percent, o.gpu_percent), (Some(a), Some(b)) if points(a, b))
            || size(self.rss, o.rss)
            || opt_size(self.private, o.private)
            || opt_size(self.read_bytes, o.read_bytes)
            || opt_size(self.write_bytes, o.write_bytes)
            || opt_size(self.gpu_mem, o.gpu_mem)
            || count(self.threads, o.threads)
            || opt_count(self.handles, o.handles)
    }
}

/// One process or program in a comparison.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcDiff {
    pub name: Arc<str>,
    pub user: Option<Arc<str>>,
    pub a: Option<DiffValues>,
    pub b: Option<DiffValues>,
    pub change: Change,
    /// `ProcKey` of the first instance in each frame.
    pub key_a: Option<ProcKey>,
    pub key_b: Option<ProcKey>,
    pub pid_a: Option<u32>,
    pub pid_b: Option<u32>,
    pub count_a: u32,
    pub count_b: u32,
}

/// A machine-wide value of both frames.
pub type Pair<T> = (T, T);

/// One disk in both frames (`None` side: not present there).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DiskDiff {
    pub id: String,
    pub read: Pair<Option<f64>>,
    pub write: Pair<Option<f64>>,
    pub busy: Pair<Option<f32>>,
}

/// One interface in both frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetDiff {
    pub id: String,
    pub name: String,
    pub rx: Pair<Option<f64>>,
    pub tx: Pair<Option<f64>>,
}

/// One GPU in both frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuDiff {
    pub id: String,
    pub name: String,
    pub load: Pair<Option<f32>>,
    pub mem: Pair<Option<u64>>,
}

/// The machine in both frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TotalsDiff {
    pub at_ms: Pair<i64>,
    pub cpu: Pair<f32>,
    pub mem_used: Pair<u64>,
    pub mem_percent: Pair<f32>,
    pub swap_used: Pair<u64>,
    pub processes: Pair<u32>,
    pub threads: Pair<u64>,
    pub handles: Pair<Option<u64>>,
    /// Bytes per second over the disks and interfaces that count for totals.
    pub disk_read: Pair<f64>,
    pub disk_write: Pair<f64>,
    pub net_rx: Pair<f64>,
    pub net_tx: Pair<f64>,
    pub disks: Vec<DiskDiff>,
    pub nets: Vec<NetDiff>,
    pub gpus: Vec<GpuDiff>,
}

/// The result of [`compare`]. `procs` is sorted by name; see [`sort_diffs`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Comparison {
    pub totals: TotalsDiff,
    pub procs: Vec<ProcDiff>,
    pub added: u32,
    pub removed: u32,
    pub changed: u32,
    pub same: u32,
}

#[derive(Default)]
struct Side {
    name: Option<Arc<str>>,
    user: Option<Arc<str>>,
    values: DiffValues,
    key: Option<ProcKey>,
    count: u32,
}

impl Side {
    fn add(&mut self, row: &ProcRow) {
        if self.count == 0 {
            self.name = Some(row.raw.name.clone());
            self.user = row.raw.user.clone();
            self.key = Some(row.raw.key);
        }
        self.values.add(row);
        self.count += 1;
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum Match {
    Key(ProcKey),
    Program(String),
}

fn match_of(row: &ProcRow, by: MatchBy) -> Match {
    match by {
        MatchBy::Process => Match::Key(row.raw.key),
        MatchBy::Program => Match::Program(
            row.raw.exe.as_deref().map_or_else(|| row.raw.name.to_string(), |p| p.to_string_lossy().into_owned()),
        ),
    }
}

fn gather(frame: &Frame, by: MatchBy, map: &mut HashMap<Match, (Side, Side)>, second: bool) {
    for row in frame.procs.iter().filter(|r| !r.raw.idle) {
        let sides = map.entry(match_of(row, by)).or_default();
        if second { sides.1.add(row) } else { sides.0.add(row) }
    }
}

/// Compares two frames. Rows that differ by less than `tolerance` count as [`Change::Same`].
pub fn compare(a: &Frame, b: &Frame, by: MatchBy, tolerance: Tolerance) -> Comparison {
    let mut map: HashMap<Match, (Side, Side)> = HashMap::with_capacity(a.procs.len().max(b.procs.len()));
    gather(a, by, &mut map, false);
    gather(b, by, &mut map, true);

    let mut out = Comparison { totals: totals(a, b), ..Default::default() };
    out.procs.reserve(map.len());
    for (_, (sa, sb)) in map {
        let change = match (sa.count > 0, sb.count > 0) {
            (true, false) => Change::Removed,
            (false, true) => Change::Added,
            _ if sa.values.differs(&sb.values, &tolerance) => Change::Changed,
            _ => Change::Same,
        };
        match change {
            Change::Added => out.added += 1,
            Change::Removed => out.removed += 1,
            Change::Changed => out.changed += 1,
            Change::Same => out.same += 1,
        }
        let name = sb.name.clone().or_else(|| sa.name.clone()).unwrap_or_else(|| Arc::from(""));
        let user = sb.user.clone().or_else(|| sa.user.clone());
        out.procs.push(ProcDiff {
            name,
            user,
            a: (sa.count > 0).then_some(sa.values),
            b: (sb.count > 0).then_some(sb.values),
            change,
            key_a: sa.key,
            key_b: sb.key,
            pid_a: sa.key.map(|k| k.pid),
            pid_b: sb.key.map(|k| k.pid),
            count_a: sa.count,
            count_b: sb.count,
        });
    }
    sort_diffs(&mut out.procs, DiffSort::Name, false);
    out
}

fn totals(a: &Frame, b: &Frame) -> TotalsDiff {
    let (ar, aw) = a.disk_totals();
    let (br, bw) = b.disk_totals();
    let (arx, atx) = a.net_totals();
    let (brx, btx) = b.net_totals();

    let mut disks: Vec<DiskDiff> = Vec::with_capacity(a.disks.len().max(b.disks.len()));
    for d in &a.disks {
        let other = b.disks.iter().find(|o| o.id == d.id);
        disks.push(DiskDiff {
            id: d.id.clone(),
            read: (Some(d.read_rate), other.map(|o| o.read_rate)),
            write: (Some(d.write_rate), other.map(|o| o.write_rate)),
            busy: (d.busy_percent, other.and_then(|o| o.busy_percent)),
        });
    }
    for d in b.disks.iter().filter(|d| !a.disks.iter().any(|o| o.id == d.id)) {
        disks.push(DiskDiff {
            id: d.id.clone(),
            read: (None, Some(d.read_rate)),
            write: (None, Some(d.write_rate)),
            busy: (None, d.busy_percent),
        });
    }

    let mut nets: Vec<NetDiff> = Vec::with_capacity(a.nets.len().max(b.nets.len()));
    for n in &a.nets {
        let other = b.nets.iter().find(|o| o.id == n.id);
        nets.push(NetDiff {
            id: n.id.clone(),
            name: n.name.clone(),
            rx: (Some(n.rx_rate), other.map(|o| o.rx_rate)),
            tx: (Some(n.tx_rate), other.map(|o| o.tx_rate)),
        });
    }
    for n in b.nets.iter().filter(|n| !a.nets.iter().any(|o| o.id == n.id)) {
        nets.push(NetDiff {
            id: n.id.clone(),
            name: n.name.clone(),
            rx: (None, Some(n.rx_rate)),
            tx: (None, Some(n.tx_rate)),
        });
    }

    let gpu_diff = |g: &GpuRaw, other: Option<&GpuRaw>, swapped: bool| {
        let (load_o, mem_o) = (other.and_then(|o| o.percent), other.and_then(|o| o.mem_used));
        GpuDiff {
            id: g.id.clone(),
            name: g.name.clone(),
            load: if swapped { (load_o, g.percent) } else { (g.percent, load_o) },
            mem: if swapped { (mem_o, g.mem_used) } else { (g.mem_used, mem_o) },
        }
    };
    let mut gpus: Vec<GpuDiff> =
        a.gpus.iter().map(|g| gpu_diff(g, b.gpus.iter().find(|o| o.id == g.id), false)).collect();
    gpus.extend(b.gpus.iter().filter(|g| !a.gpus.iter().any(|o| o.id == g.id)).map(|g| gpu_diff(g, None, true)));

    TotalsDiff {
        at_ms: (a.at_ms, b.at_ms),
        cpu: (a.cpu.percent, b.cpu.percent),
        mem_used: (a.memory.used, b.memory.used),
        mem_percent: (a.memory.percent, b.memory.percent),
        swap_used: (a.memory.raw.swap_used, b.memory.raw.swap_used),
        processes: (a.counts.processes, b.counts.processes),
        threads: (a.counts.threads, b.counts.threads),
        handles: (a.counts.handles, b.counts.handles),
        disk_read: (ar, br),
        disk_write: (aw, bw),
        net_rx: (arx, brx),
        net_tx: (atx, btx),
        disks,
        nets,
        gpus,
    }
}

/// What [`sort_diffs`] orders by; the `Delta` ones by `b - a`, a missing side counting as 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DiffSort {
    #[default]
    Name,
    User,
    Change,
    CpuDelta,
    RssDelta,
    PrivateDelta,
    ThreadsDelta,
    HandlesDelta,
    ReadDelta,
    WriteDelta,
    GpuDelta,
    GpuMemDelta,
    CountDelta,
}

fn delta(d: &ProcDiff, f: impl Fn(&DiffValues) -> f64) -> f64 {
    d.b.as_ref().map_or(0.0, &f) - d.a.as_ref().map_or(0.0, &f)
}

fn sort_value(d: &ProcDiff, by: DiffSort) -> f64 {
    let opt = |v: Option<u64>| v.map_or(0.0, |v| v as f64);
    match by {
        DiffSort::Name | DiffSort::User => 0.0,
        DiffSort::Change => match d.change {
            Change::Added => 0.0,
            Change::Removed => 1.0,
            Change::Changed => 2.0,
            Change::Same => 3.0,
        },
        DiffSort::CpuDelta => delta(d, |v| f64::from(v.cpu_percent)),
        DiffSort::RssDelta => delta(d, |v| v.rss as f64),
        DiffSort::PrivateDelta => delta(d, |v| opt(v.private)),
        DiffSort::ThreadsDelta => delta(d, |v| v.threads as f64),
        DiffSort::HandlesDelta => delta(d, |v| opt(v.handles)),
        DiffSort::ReadDelta => delta(d, |v| opt(v.read_bytes)),
        DiffSort::WriteDelta => delta(d, |v| opt(v.write_bytes)),
        DiffSort::GpuDelta => delta(d, |v| f64::from(v.gpu_percent.unwrap_or(0.0))),
        DiffSort::GpuMemDelta => delta(d, |v| opt(v.gpu_mem)),
        DiffSort::CountDelta => f64::from(d.count_b) - f64::from(d.count_a),
    }
}

/// Sorts rows; ties by name, then pid.
pub fn sort_diffs(rows: &mut [ProcDiff], by: DiffSort, descending: bool) {
    rows.sort_by(|x, y| {
        let primary = match by {
            DiffSort::Name => x.name.to_lowercase().cmp(&y.name.to_lowercase()),
            DiffSort::User => x.user.cmp(&y.user),
            _ => sort_value(x, by).total_cmp(&sort_value(y, by)),
        };
        let primary = if descending { primary.reverse() } else { primary };
        primary.then_with(|| x.name.cmp(&y.name)).then_with(|| x.pid_b.or(x.pid_a).cmp(&y.pid_b.or(y.pid_a)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{DiskFrame, NetFrame};
    use crate::testutil::{frame_of, key, row};
    use std::path::Path;

    fn program(pid: u32, start: i64, name: &str, cpu: f32, rss: u64) -> ProcRow {
        let mut r = row(pid, None, name, cpu, rss);
        r.raw.key = key(pid, start);
        r.raw.exe = Some(Arc::from(Path::new(&format!("/bin/{name}"))));
        r
    }

    const MB: u64 = 1 << 20;

    #[test]
    fn added_removed_changed_same_by_process() {
        let a = frame_of(vec![
            program(1, 10, "keeps", 5.0, 100 * MB),
            program(2, 10, "grows", 5.0, 100 * MB),
            program(3, 10, "leaves", 5.0, 100 * MB),
        ]);
        let b = frame_of(vec![
            program(1, 10, "keeps", 5.5, 100 * MB),
            program(2, 10, "grows", 5.0, 200 * MB),
            program(4, 10, "arrives", 5.0, 100 * MB),
        ]);
        let c = compare(&a, &b, MatchBy::Process, Tolerance::default());
        assert_eq!((c.added, c.removed, c.changed, c.same), (1, 1, 1, 1));
        let by_name = |n: &str| c.procs.iter().find(|p| &*p.name == n).unwrap();
        assert_eq!(by_name("keeps").change, Change::Same);
        assert_eq!(by_name("grows").change, Change::Changed);
        assert_eq!(by_name("leaves").change, Change::Removed);
        assert!(by_name("leaves").b.is_none());
        assert_eq!(by_name("leaves").pid_a, Some(3));
        assert_eq!(by_name("arrives").change, Change::Added);
        assert!(by_name("arrives").a.is_none());
        assert_eq!(by_name("arrives").key_b, Some(key(4, 10)));
    }

    #[test]
    fn a_reused_pid_is_two_processes_by_process_but_one_by_program() {
        let a = frame_of(vec![program(7, 10, "app", 1.0, MB)]);
        let b = frame_of(vec![program(7, 99, "app", 1.0, MB)]);
        let c = compare(&a, &b, MatchBy::Process, Tolerance::default());
        assert_eq!((c.added, c.removed), (1, 1));
        let c = compare(&a, &b, MatchBy::Program, Tolerance::default());
        assert_eq!((c.added, c.removed, c.same), (0, 0, 1));
    }

    #[test]
    fn program_match_sums_instances() {
        let a = frame_of(vec![program(1, 1, "chrome", 2.0, 10 * MB), program(2, 1, "chrome", 3.0, 20 * MB)]);
        let b = frame_of(vec![
            program(5, 1, "chrome", 1.0, 10 * MB),
            program(6, 1, "chrome", 1.0, 20 * MB),
            program(7, 1, "chrome", 8.0, 90 * MB),
        ]);
        let c = compare(&a, &b, MatchBy::Program, Tolerance::default());
        assert_eq!(c.procs.len(), 1);
        let d = &c.procs[0];
        assert_eq!((d.count_a, d.count_b), (2, 3));
        assert_eq!(d.a.unwrap().rss, 30 * MB);
        assert_eq!(d.b.unwrap().rss, 120 * MB);
        assert_eq!(d.a.unwrap().cpu_percent, 5.0);
        assert_eq!(d.change, Change::Changed);
    }

    #[test]
    fn program_match_falls_back_to_the_name() {
        let a = frame_of(vec![row(1, None, "noexe", 1.0, MB)]);
        let b = frame_of(vec![row(2, None, "noexe", 1.0, MB)]);
        let c = compare(&a, &b, MatchBy::Program, Tolerance::default());
        assert_eq!((c.same, c.added), (1, 0));
    }

    #[test]
    fn tolerance_treats_each_kind_of_value_by_its_own_rule() {
        let base = |cpu, rss, threads| {
            let mut r = program(1, 1, "x", cpu, rss);
            r.raw.threads = threads;
            frame_of(vec![r])
        };
        let t = Tolerance::default();
        let change = |a: &Frame, b: &Frame, t: Tolerance| compare(a, b, MatchBy::Process, t).procs[0].change;
        let a = base(10.0, 500 * MB, 10);
        assert_eq!(change(&a, &base(10.9, 500 * MB, 10), t), Change::Same, "within a CPU point");
        assert_eq!(change(&a, &base(11.5, 500 * MB, 10), t), Change::Changed);
        assert_eq!(change(&a, &base(10.0, 505 * MB, 10), t), Change::Same, "within 2 %");
        assert_eq!(change(&a, &base(10.0, 520 * MB, 10), t), Change::Changed);
        assert_eq!(change(&a, &base(10.0, 500 * MB, 11), t), Change::Same, "one thread is churn");
        assert_eq!(change(&a, &base(10.0, 500 * MB, 12), t), Change::Changed);
        let small = base(10.0, MB / 2, 10);
        assert_eq!(change(&small, &base(10.0, MB, 10), t), Change::Same, "under the byte floor");
        assert_eq!(change(&a, &base(10.0, 500 * MB + 1, 10), Tolerance::exact()), Change::Changed);
        assert_eq!(change(&a, &base(10.0, 500 * MB, 10), Tolerance::exact()), Change::Same);
    }

    #[test]
    fn values_only_one_side_knows_are_no_change() {
        let mut ra = program(1, 1, "x", 1.0, MB);
        ra.raw.private = None;
        let mut rb = ra.clone();
        rb.raw.private = Some(900 * MB);
        let c = compare(&frame_of(vec![ra]), &frame_of(vec![rb]), MatchBy::Process, Tolerance::default());
        assert_eq!(c.procs[0].change, Change::Same);
    }

    #[test]
    fn idle_is_left_out() {
        let mut idle = row(0, None, "Idle", 800.0, 0);
        idle.raw.idle = true;
        let f = frame_of(vec![idle]);
        assert!(compare(&f, &f, MatchBy::Process, Tolerance::default()).procs.is_empty());
    }

    #[test]
    fn totals_pair_up_with_the_same_exclusions_as_elsewhere() {
        let mut a = frame_of(vec![]);
        a.at_ms = 1;
        a.cpu.percent = 10.0;
        a.counts.processes = 5;
        a.memory.used = 100;
        a.disks = vec![
            DiskFrame { id: "sda".into(), read_rate: 5.0, ..Default::default() },
            DiskFrame { id: "dm-0".into(), read_rate: 5.0, ..Default::default() },
            DiskFrame { id: "old".into(), write_rate: 1.0, ..Default::default() },
        ];
        a.nets = vec![
            NetFrame { id: "eth0".into(), rx_rate: 7.0, up: true, ..Default::default() },
            NetFrame { id: "lo".into(), rx_rate: 7.0, up: true, virtual_if: true, ..Default::default() },
        ];
        a.gpus = vec![GpuRaw { id: "g".into(), percent: Some(10.0), ..Default::default() }];
        let mut b = a.clone();
        b.at_ms = 2;
        b.cpu.percent = 20.0;
        b.disks = vec![
            DiskFrame { id: "sda".into(), read_rate: 6.0, ..Default::default() },
            DiskFrame { id: "new".into(), read_rate: 2.0, ..Default::default() },
        ];
        b.gpus = vec![
            GpuRaw { id: "g".into(), percent: Some(30.0), ..Default::default() },
            GpuRaw { id: "h".into(), percent: Some(1.0), ..Default::default() },
        ];
        let t = compare(&a, &b, MatchBy::Process, Tolerance::default()).totals;
        assert_eq!(t.at_ms, (1, 2));
        assert_eq!(t.cpu, (10.0, 20.0));
        assert_eq!(t.processes, (5, 5));
        assert_eq!(t.disk_read, (5.0, 8.0), "dm-0 re-counts sda");
        assert_eq!(t.net_rx, (7.0, 7.0), "loopback left out");
        let id = |i: &str| t.disks.iter().find(|d| d.id == i).unwrap();
        assert_eq!(id("sda").read, (Some(5.0), Some(6.0)));
        assert_eq!(id("old").read, (Some(0.0), None));
        assert_eq!(id("new").read, (None, Some(2.0)));
        assert_eq!(t.gpus[0].load, (Some(10.0), Some(30.0)));
        assert_eq!(t.gpus[1].load, (None, Some(1.0)));
    }

    #[test]
    fn sorting_by_a_delta() {
        let a = frame_of(vec![program(1, 1, "x", 1.0, 100 * MB), program(2, 1, "y", 1.0, 100 * MB)]);
        let b = frame_of(vec![
            program(1, 1, "x", 1.0, 150 * MB),
            program(2, 1, "y", 9.0, 100 * MB),
            program(3, 1, "z", 2.0, 400 * MB),
        ]);
        let mut c = compare(&a, &b, MatchBy::Process, Tolerance::default());
        assert_eq!(c.procs.iter().map(|p| &*p.name).collect::<Vec<_>>(), ["x", "y", "z"]);
        sort_diffs(&mut c.procs, DiffSort::RssDelta, true);
        assert_eq!(c.procs.iter().map(|p| &*p.name).collect::<Vec<_>>(), ["z", "x", "y"]);
        sort_diffs(&mut c.procs, DiffSort::CpuDelta, true);
        assert_eq!(&*c.procs[0].name, "y");
        sort_diffs(&mut c.procs, DiffSort::Change, false);
        assert_eq!(c.procs[0].change, Change::Added);
        sort_diffs(&mut c.procs, DiffSort::Name, true);
        assert_eq!(&*c.procs[0].name, "z");
    }
}
