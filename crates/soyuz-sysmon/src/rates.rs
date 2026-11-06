//! Turns raw counters into rates. Everything is measured against the monotonic clock, CPU
//! percentages against the CPU counters themselves (never wall time). A counter that goes
//! backwards gives 0, a reused PID gives 0 (previous counters are matched by `ProcKey`), and
//! the first sample and a sample after a clock jump give 0 for every rate.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use soyuz_fs::sysmon::{CpuTimes, ProcKey, ProcState, RawSample};

use crate::frame::{CoreFrame, Counts, CpuFrame, DiskFrame, Frame, MemFrame, NetFrame, ProcRow};
use crate::sanitize;

/// Wall and monotonic clock disagreeing by more than this means a suspend or a clock step.
const CLOCK_SKEW_MS: i64 = 2_000;
/// A stall shorter than this never counts as a gap: a sample that simply takes long at a short
/// interval must not zero every rate of every frame.
const MIN_GAP_MS: u64 = 5_000;

/// Stateful converter from raw samples to frames; one per sampler.
#[derive(Default)]
pub struct Rater {
    prev: Option<Prev>,
    procs: HashMap<ProcKey, ProcPrev>,
    interval_ms: u64,
    generation: u64,
}

struct Prev {
    at_ms: i64,
    mono_ms: u64,
    cpu: CpuTimes,
    cores: Vec<CpuTimes>,
    ctx: Option<u64>,
    interrupts: Option<u64>,
    page_in: Option<u64>,
    page_out: Option<u64>,
    disks: HashMap<String, DiskPrev>,
    nets: HashMap<String, NetPrev>,
    had_procs: bool,
}

#[derive(Clone, Copy)]
struct DiskPrev {
    read: u64,
    write: u64,
    read_ops: u64,
    write_ops: u64,
    busy_ms: Option<u64>,
}

#[derive(Clone, Copy)]
struct NetPrev {
    rx: u64,
    tx: u64,
    rx_packets: u64,
    tx_packets: u64,
}

/// A text from the probe and its clean copy, so that a text is checked once per process (and
/// again when the probe refreshes it), not once per sample.
struct Clean<T: ?Sized> {
    src: Arc<T>,
    clean: Arc<T>,
}

impl<T: ?Sized> Clean<T> {
    fn get(cache: &mut Option<Self>, src: &Arc<T>, make: impl Fn(&Arc<T>) -> Arc<T>) -> Arc<T> {
        match cache {
            Some(c) if Arc::ptr_eq(&c.src, src) => c.clean.clone(),
            _ => {
                let clean = make(src);
                *cache = Some(Clean { src: src.clone(), clean: clean.clone() });
                clean
            }
        }
    }

    fn get_opt(cache: &mut Option<Self>, src: &Option<Arc<T>>, make: impl Fn(&Arc<T>) -> Arc<T>) -> Option<Arc<T>> {
        match src {
            Some(s) => Some(Self::get(cache, s, make)),
            None => {
                *cache = None;
                None
            }
        }
    }
}

struct ProcPrev {
    cpu_ms: u64,
    read: Option<u64>,
    write: Option<u64>,
    generation: u64,
    name: Option<Clean<str>>,
    exe: Option<Clean<Path>>,
    cmdline: Option<Clean<str>>,
    user: Option<Clean<str>>,
    session: Option<Clean<str>>,
    arch: Option<Clean<str>>,
    cgroup: Option<Clean<str>>,
    description: Option<Clean<str>>,
}

impl Rater {
    pub fn new() -> Self {
        Self::default()
    }

    /// The interval the sampler aims for, used to recognise a stalled sampler (a frame more
    /// than `max(3 x interval, 5 s)` after the previous one is a gap). 0: unknown, only the
    /// clock comparison counts.
    pub fn set_interval(&mut self, interval_ms: u64) {
        self.interval_ms = interval_ms;
    }

    /// Forgets every previous counter: the next frame is `first` again.
    pub fn reset(&mut self) {
        self.prev = None;
        self.procs.clear();
    }

    /// One sample in, one frame out.
    pub fn push(&mut self, raw: RawSample) -> Frame {
        let RawSample {
            at_ms,
            mono_ms,
            uptime_secs,
            load,
            cpu_total,
            cores,
            cpu_temp_c,
            ctx_switches,
            interrupts,
            memory,
            disks,
            nets,
            mut gpus,
            mut sensors,
            mut batteries,
            processes,
            total_threads,
            total_handles,
        } = raw;

        let prev = self.prev.take();
        let first = prev.is_none();
        let elapsed_ms = prev.as_ref().map_or(0, |p| mono_ms.saturating_sub(p.mono_ms));
        let gap = prev.as_ref().is_some_and(|p| {
            let wall = at_ms.saturating_sub(p.at_ms);
            let mono = mono_ms.saturating_sub(p.mono_ms) as i64;
            (wall - mono).abs() > CLOCK_SKEW_MS
                || (self.interval_ms > 0 && elapsed_ms > (self.interval_ms * 3).max(MIN_GAP_MS))
        });
        // The basis of all rates. `None`: every rate is 0.
        let base = if gap || elapsed_ms == 0 { None } else { prev.as_ref() };
        let secs = elapsed_ms as f64 / 1000.0;

        // CPU
        let total = cpu_load(base.map(|p| p.cpu), cpu_total);
        let cores_frame: Vec<CoreFrame> = cores
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let load = cpu_load(base.and_then(|p| p.cores.get(i).copied()), c.times);
                CoreFrame { percent: load.percent, user: load.user, system: load.system, mhz: c.mhz }
            })
            .collect();
        let clocks: Vec<u32> = cores.iter().filter_map(|c| c.mhz).collect();
        let mhz_avg =
            (!clocks.is_empty()).then(|| (clocks.iter().map(|&m| m as u64).sum::<u64>() / clocks.len() as u64) as u32);
        let cpu = CpuFrame {
            percent: total.percent,
            user: total.user,
            system: total.system,
            iowait: total.iowait,
            steal: total.steal,
            irq: total.irq,
            cores: cores_frame,
            mhz_avg,
            temp_c: cpu_temp_c.filter(|t| t.is_finite()),
            ctx_per_sec: counter_rate(ctx_switches, base.and_then(|p| p.ctx), base.is_some(), secs),
            irq_per_sec: counter_rate(interrupts, base.and_then(|p| p.interrupts), base.is_some(), secs),
        };

        // Memory
        let used = memory.total.saturating_sub(memory.available);
        let mem_frame = MemFrame {
            raw: memory,
            used,
            percent: percent_of(used, memory.total),
            swap_percent: percent_of(memory.swap_used, memory.swap_total),
            page_in_rate: counter_rate(memory.page_in, base.and_then(|p| p.page_in), base.is_some(), secs),
            page_out_rate: counter_rate(memory.page_out, base.and_then(|p| p.page_out), base.is_some(), secs),
        };

        // Disks
        let mut disk_prev = HashMap::with_capacity(disks.len());
        let disks_frame: Vec<DiskFrame> = disks
            .into_iter()
            .map(|d| {
                let now = DiskPrev {
                    read: d.read_bytes,
                    write: d.write_bytes,
                    read_ops: d.read_ops,
                    write_ops: d.write_ops,
                    busy_ms: d.busy_ms,
                };
                let before = base.and_then(|p| p.disks.get(&d.id));
                let rate =
                    |cur: u64, old: fn(&DiskPrev) -> u64| before.map_or(0.0, |b| delta(cur, old(b)) as f64 / secs);
                let busy_percent = match d.busy_ms {
                    Some(ms) => {
                        let spent = before.and_then(|b| b.busy_ms).map_or(0, |old| delta(ms, old));
                        Some(if before.is_some() && elapsed_ms > 0 {
                            (spent as f32 * 100.0 / elapsed_ms as f32).clamp(0.0, 100.0)
                        } else {
                            0.0
                        })
                    }
                    None => d.busy_percent.filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 100.0)),
                };
                let frame = DiskFrame {
                    read_rate: rate(d.read_bytes, |b| b.read),
                    write_rate: rate(d.write_bytes, |b| b.write),
                    read_iops: rate(d.read_ops, |b| b.read_ops),
                    write_iops: rate(d.write_ops, |b| b.write_ops),
                    busy_percent,
                    queue: d.queue.filter(|v| v.is_finite()),
                    rotational: d.rotational,
                    size: d.size,
                    read_bytes: d.read_bytes,
                    write_bytes: d.write_bytes,
                    model: sanitize::sanitize_text(&d.model).into_owned(),
                    id: d.id,
                };
                disk_prev.insert(frame.id.clone(), now);
                frame
            })
            .collect();

        // Network
        let mut net_prev = HashMap::with_capacity(nets.len());
        let nets_frame: Vec<NetFrame> = nets
            .into_iter()
            .map(|n| {
                let now =
                    NetPrev { rx: n.rx_bytes, tx: n.tx_bytes, rx_packets: n.rx_packets, tx_packets: n.tx_packets };
                let before = base.and_then(|p| p.nets.get(&n.id));
                let rate =
                    |cur: u64, old: fn(&NetPrev) -> u64| before.map_or(0.0, |b| delta(cur, old(b)) as f64 / secs);
                let frame = NetFrame {
                    rx_rate: rate(n.rx_bytes, |b| b.rx),
                    tx_rate: rate(n.tx_bytes, |b| b.tx),
                    rx_packets_rate: rate(n.rx_packets, |b| b.rx_packets),
                    tx_packets_rate: rate(n.tx_packets, |b| b.tx_packets),
                    rx_bytes: n.rx_bytes,
                    tx_bytes: n.tx_bytes,
                    rx_errors: n.rx_errors,
                    tx_errors: n.tx_errors,
                    up: n.up,
                    virtual_if: n.virtual_if,
                    link_bps: n.link_bps,
                    name: sanitize::sanitize_text(&n.name).into_owned(),
                    description: sanitize::sanitize_text(&n.description).into_owned(),
                    addresses: n.addresses,
                    mac: n.mac,
                    id: n.id,
                };
                net_prev.insert(frame.id.clone(), now);
                frame
            })
            .collect();

        // Values that end up in JSON must be finite: a NaN would not load back.
        for g in &mut gpus {
            g.percent = g.percent.filter(|v| v.is_finite());
            g.temperature_c = g.temperature_c.filter(|v| v.is_finite());
            g.power_w = g.power_w.filter(|v| v.is_finite());
            g.engines.retain(|e| e.percent.is_finite());
            clean_string(&mut g.name);
            clean_string(&mut g.vendor);
        }
        sensors.retain(|s| s.value.is_finite());
        for s in &mut sensors {
            s.max = s.max.filter(|v| v.is_finite());
            s.critical = s.critical.filter(|v| v.is_finite());
            clean_string(&mut s.group);
            clean_string(&mut s.label);
        }
        for b in &mut batteries {
            if !b.percent.is_finite() {
                b.percent = 0.0;
            }
            b.power_w = b.power_w.filter(|v| v.is_finite());
            b.health_percent = b.health_percent.filter(|v| v.is_finite());
            clean_string(&mut b.name);
        }
        let load = load.filter(|l| l.iter().all(|v| v.is_finite()));

        // Processes
        let ncpu = cores.len() as u32;
        let cpu_ticks_advanced = base.is_some_and(|p| sum(cpu_total) > sum(p.cpu));
        let machine = cpu_ticks_advanced.then_some(total.percent);
        let had_procs = prev.as_ref().is_some_and(|p| p.had_procs);
        let (rows, added, removed, counts) = self.processes(
            processes,
            ProcEnv { base: base.is_some(), secs, elapsed_ms, ncpu, machine, mem_total: memory.total, had_procs },
            total_threads,
            total_handles,
        );

        self.prev = Some(Prev {
            at_ms,
            mono_ms,
            cpu: cpu_total,
            cores: cores.iter().map(|c| c.times).collect(),
            ctx: ctx_switches,
            interrupts,
            page_in: memory.page_in,
            page_out: memory.page_out,
            disks: disk_prev,
            nets: net_prev,
            had_procs: !rows.is_empty(),
        });

        Frame {
            at_ms,
            uptime_secs,
            load,
            first,
            gap,
            elapsed_ms,
            cpu,
            memory: mem_frame,
            disks: disks_frame,
            nets: nets_frame,
            gpus,
            sensors,
            batteries,
            procs: Arc::from(rows),
            counts,
            added,
            removed,
        }
    }

    fn processes(
        &mut self,
        processes: Vec<soyuz_fs::sysmon::ProcRaw>,
        env: ProcEnv,
        total_threads: Option<u64>,
        total_handles: Option<u64>,
    ) -> (Vec<ProcRow>, Vec<ProcKey>, Vec<ProcKey>, Counts) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let mut added = Vec::new();
        let mut counts = Counts::default();
        let mut threads = 0u64;
        let mut handles = None::<u64>;
        let mut sum_percent = 0f64;
        let cpu_cap = if env.ncpu > 0 { 100.0 * env.ncpu as f32 } else { f32::MAX };

        let mut rows: Vec<ProcRow> = Vec::with_capacity(processes.len());
        for mut raw in processes {
            let key = raw.key;
            let cpu_ms = raw.cpu_user_ms.saturating_add(raw.cpu_system_ms);
            let mut known = true;
            let entry = self.procs.entry(key).or_insert_with(|| {
                known = false;
                ProcPrev {
                    cpu_ms,
                    read: raw.read_bytes,
                    write: raw.write_bytes,
                    generation,
                    name: None,
                    exe: None,
                    cmdline: None,
                    user: None,
                    session: None,
                    arch: None,
                    cgroup: None,
                    description: None,
                }
            });
            if !known && env.had_procs {
                added.push(key);
            }

            raw.name = Clean::get(&mut entry.name, &raw.name, sanitize::sanitize_arc);
            raw.exe = Clean::get_opt(&mut entry.exe, &raw.exe, sanitize::sanitize_path);
            raw.cmdline = Clean::get_opt(&mut entry.cmdline, &raw.cmdline, sanitize::sanitize_arc);
            raw.user = Clean::get_opt(&mut entry.user, &raw.user, sanitize::sanitize_arc);
            raw.session = Clean::get_opt(&mut entry.session, &raw.session, sanitize::sanitize_arc);
            raw.arch = Clean::get_opt(&mut entry.arch, &raw.arch, sanitize::sanitize_arc);
            raw.cgroup = Clean::get_opt(&mut entry.cgroup, &raw.cgroup, sanitize::sanitize_arc);
            raw.description = Clean::get_opt(&mut entry.description, &raw.description, sanitize::sanitize_arc);
            raw.gpu_percent = raw.gpu_percent.filter(|v| v.is_finite());

            let usable = env.base && known && env.elapsed_ms > 0;
            let cpu_percent = if raw.idle || !usable {
                0.0
            } else {
                (delta(cpu_ms, entry.cpu_ms) as f64 * 100.0 / env.elapsed_ms as f64).min(cpu_cap as f64) as f32
            };
            let byte_rate = |now: Option<u64>, old: Option<u64>| {
                now.map(|n| if usable { old.map_or(0.0, |o| delta(n, o) as f64 / env.secs) } else { 0.0 })
            };
            let read_rate = byte_rate(raw.read_bytes, entry.read);
            let write_rate = byte_rate(raw.write_bytes, entry.write);

            entry.cpu_ms = cpu_ms;
            entry.read = raw.read_bytes;
            entry.write = raw.write_bytes;
            entry.generation = generation;

            if raw.idle {
                raw.cpu_user_ms = 0;
                raw.cpu_system_ms = 0;
            } else {
                counts.processes += 1;
                threads += u64::from(raw.threads);
                if let Some(h) = raw.handles {
                    *handles.get_or_insert(0) += u64::from(h);
                }
                if raw.state == ProcState::Running {
                    counts.running += 1;
                }
                sum_percent += f64::from(cpu_percent);
            }
            let mem_percent = percent_of(raw.rss, env.mem_total);
            rows.push(ProcRow { raw, cpu_percent, read_rate, write_rate, mem_percent });
        }

        // Sampling skew must not make the processes add up to more than the machine used.
        if let Some(machine) = env.machine
            && env.ncpu > 0
        {
            let budget = f64::from(machine) * f64::from(env.ncpu);
            if sum_percent > budget && sum_percent > 0.0 {
                let factor = (budget / sum_percent) as f32;
                for r in &mut rows {
                    r.cpu_percent *= factor;
                }
            }
        }

        let mut removed = Vec::new();
        if env.had_procs && rows.is_empty() {
            // A sample without a process table says nothing about who exited.
            self.procs.clear();
        } else {
            self.procs.retain(|key, p| {
                let alive = p.generation == generation;
                if !alive {
                    removed.push(*key);
                }
                alive
            });
        }

        counts.threads = if rows.is_empty() { total_threads.unwrap_or(0) } else { threads };
        counts.handles = handles.or(total_handles);
        (rows, added, removed, counts)
    }
}

struct ProcEnv {
    base: bool,
    secs: f64,
    elapsed_ms: u64,
    ncpu: u32,
    machine: Option<f32>,
    mem_total: u64,
    had_procs: bool,
}

fn clean_string(s: &mut String) {
    if let std::borrow::Cow::Owned(clean) = sanitize::sanitize_text(s) {
        *s = clean;
    }
}

fn delta(now: u64, old: u64) -> u64 {
    now.saturating_sub(old)
}

fn counter_rate(now: Option<u64>, old: Option<u64>, have_base: bool, secs: f64) -> Option<f64> {
    now.map(|n| if have_base { old.map_or(0.0, |o| delta(n, o) as f64 / secs) } else { 0.0 })
}

fn percent_of(part: u64, whole: u64) -> f32 {
    if whole == 0 { 0.0 } else { ((part as f64 * 100.0 / whole as f64) as f32).clamp(0.0, 100.0) }
}

fn sum(t: CpuTimes) -> u64 {
    t.user + t.nice + t.system + t.idle + t.iowait + t.irq + t.softirq + t.steal
}

#[derive(Default, Clone, Copy)]
struct CpuLoad {
    percent: f32,
    user: f32,
    system: f32,
    iowait: f32,
    steal: f32,
    irq: f32,
}

/// Shares of the CPU counters' own total: `percent = 1 - d(idle) / d(sum of all fields)`, in
/// percent. Dividing by the counters' sum rather than wall time keeps it within 0..=100.
fn cpu_load(old: Option<CpuTimes>, now: CpuTimes) -> CpuLoad {
    let Some(old) = old else { return CpuLoad::default() };
    let d = |a: u64, b: u64| a.saturating_sub(b) as f64;
    let user = d(now.user, old.user) + d(now.nice, old.nice);
    let system = d(now.system, old.system);
    let idle = d(now.idle, old.idle);
    let iowait = d(now.iowait, old.iowait);
    let irq = d(now.irq, old.irq) + d(now.softirq, old.softirq);
    let steal = d(now.steal, old.steal);
    let total = user + system + idle + iowait + irq + steal;
    if total <= 0.0 {
        return CpuLoad::default();
    }
    let share = |v: f64| ((v * 100.0 / total) as f32).clamp(0.0, 100.0);
    CpuLoad {
        percent: share(total - idle),
        user: share(user),
        system: share(system),
        iowait: share(iowait),
        steal: share(steal),
        irq: share(irq),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use soyuz_fs::sysmon::{DiskRaw, MemoryRaw, NetRaw};

    fn at(mono: u64) -> RawSample {
        sample(1_000_000 + mono as i64, mono)
    }

    #[test]
    fn first_frame_has_zero_rates() {
        let mut r = Rater::new();
        let mut s = at(1000);
        s.processes = vec![proc_raw(1, 10, "a", 5_000)];
        s.disks = vec![DiskRaw { id: "d".into(), read_bytes: 1 << 30, ..Default::default() }];
        let f = r.push(s);
        assert!(f.first);
        assert!(!f.gap);
        assert_eq!(f.procs[0].cpu_percent, 0.0);
        assert_eq!(f.disks[0].read_rate, 0.0);
        assert_eq!(f.cpu.percent, 0.0);
        assert!(f.added.is_empty());
    }

    #[test]
    fn cpu_math_uses_counter_sum_not_wall_time() {
        let mut r = Rater::new();
        let mut a = at(0);
        a.cpu_total = times(0, 0, 0);
        r.push(a);
        let mut b = at(1000);
        // 8 CPUs: 4000 ms user, 1000 ms system, 3000 ms idle over one second.
        b.cpu_total = times(4000, 1000, 3000);
        let f = r.push(b);
        assert!(!f.first);
        assert!((f.cpu.percent - 62.5).abs() < 0.01, "{}", f.cpu.percent);
        assert!((f.cpu.user - 50.0).abs() < 0.01);
        assert!((f.cpu.system - 12.5).abs() < 0.01);
    }

    #[test]
    fn counter_reset_gives_zero_and_not_a_spike() {
        let mut r = Rater::new();
        let mut a = at(0);
        a.disks = vec![DiskRaw { id: "d".into(), read_bytes: 5_000_000, ..Default::default() }];
        a.nets = vec![NetRaw { id: "n".into(), rx_bytes: 9_000_000, up: true, ..Default::default() }];
        a.cpu_total = times(100, 100, 100);
        r.push(a);
        let mut b = at(1000);
        b.disks = vec![DiskRaw { id: "d".into(), read_bytes: 10, ..Default::default() }];
        b.nets = vec![NetRaw { id: "n".into(), rx_bytes: 20, up: true, ..Default::default() }];
        b.cpu_total = times(0, 0, 0);
        let f = r.push(b);
        assert_eq!(f.disks[0].read_rate, 0.0);
        assert_eq!(f.nets[0].rx_rate, 0.0);
        assert_eq!(f.cpu.percent, 0.0);
    }

    #[test]
    fn rates_are_per_second() {
        let mut r = Rater::new();
        let mut a = at(0);
        a.disks = vec![DiskRaw { id: "d".into(), read_bytes: 0, write_ops: 0, busy_ms: Some(0), ..Default::default() }];
        a.nets = vec![NetRaw { id: "n".into(), tx_bytes: 0, up: true, ..Default::default() }];
        a.memory = MemoryRaw { total: 100, available: 40, page_in: Some(0), ..Default::default() };
        a.ctx_switches = Some(0);
        r.push(a);
        let mut b = at(2000);
        b.disks =
            vec![DiskRaw { id: "d".into(), read_bytes: 4000, write_ops: 10, busy_ms: Some(500), ..Default::default() }];
        b.nets = vec![NetRaw { id: "n".into(), tx_bytes: 6000, up: true, ..Default::default() }];
        b.memory = MemoryRaw { total: 100, available: 40, page_in: Some(2048), ..Default::default() };
        b.ctx_switches = Some(300);
        let f = r.push(b);
        assert_eq!(f.disks[0].read_rate, 2000.0);
        assert_eq!(f.disks[0].write_iops, 5.0);
        assert_eq!(f.disks[0].busy_percent, Some(25.0));
        assert_eq!(f.nets[0].tx_rate, 3000.0);
        assert_eq!(f.memory.page_in_rate, Some(1024.0));
        assert_eq!(f.memory.page_out_rate, None);
        assert_eq!(f.memory.used, 60);
        assert_eq!(f.memory.percent, 60.0);
        assert_eq!(f.cpu.ctx_per_sec, Some(150.0));
        assert_eq!(f.cpu.irq_per_sec, None);
    }

    #[test]
    fn instant_busy_percent_is_passed_through() {
        let mut r = Rater::new();
        let mut a = at(0);
        a.disks = vec![DiskRaw { id: "d".into(), busy_percent: Some(33.0), ..Default::default() }];
        let f = r.push(a);
        assert_eq!(f.disks[0].busy_percent, Some(33.0));
    }

    #[test]
    fn process_percent_is_clamped_and_summed_to_the_machine() {
        let mut r = Rater::new();
        let mut a = at(0);
        a.cores = cores(2);
        a.cpu_total = times(0, 0, 0);
        a.processes = vec![proc_raw(1, 10, "a", 0), proc_raw(2, 10, "b", 0)];
        r.push(a);
        // 1 s elapsed: a claims 5 CPUs of time, b one, on a machine of two that was 50 % busy.
        let mut b = at(1000);
        b.cores = cores(2);
        b.cpu_total = times(1000, 0, 1000);
        b.processes = vec![proc_raw(1, 10, "a", 5000), proc_raw(2, 10, "b", 1000)];
        let f = r.push(b);
        let total: f32 = f.procs.iter().map(|p| p.cpu_percent).sum();
        assert!((total - 100.0).abs() < 0.1, "budget is 50 % of two CPUs, got {total}");
        assert!(f.procs[0].cpu_percent <= 200.0);
        assert!(f.procs[0].cpu_percent > f.procs[1].cpu_percent);
    }

    #[test]
    fn pid_reuse_is_a_new_process_not_a_spike() {
        let mut r = Rater::new();
        let mut a = at(0);
        a.processes = vec![proc_raw(7, 100, "old", 50)];
        r.push(a);
        let mut b = at(1000);
        b.processes = vec![proc_raw(7, 900, "new", 80_000)];
        let f = r.push(b);
        assert_eq!(f.procs[0].cpu_percent, 0.0);
        assert_eq!(f.added, vec![key(7, 900)]);
        assert_eq!(f.removed, vec![key(7, 100)]);
    }

    #[test]
    fn idle_process_is_zeroed_and_not_counted() {
        let mut r = Rater::new();
        let mut a = at(0);
        a.cores = cores(1);
        let mut idle = proc_raw(0, 0, "Idle", 0);
        idle.idle = true;
        idle.threads = 32;
        a.processes = vec![idle.clone(), proc_raw(5, 1, "x", 0)];
        r.push(a);
        let mut b = at(1000);
        b.cores = cores(1);
        idle.cpu_user_ms = 90_000;
        b.processes = vec![idle, proc_raw(5, 1, "x", 0)];
        let f = r.push(b);
        assert_eq!(f.procs[0].cpu_percent, 0.0);
        assert_eq!(f.procs[0].raw.cpu_user_ms, 0);
        assert_eq!(f.counts.processes, 1);
        assert_eq!(f.counts.threads, 1, "the idle pseudo process's threads do not count");
    }

    #[test]
    fn clock_jump_marks_a_gap_with_zero_rates() {
        let mut r = Rater::new();
        let mut a = sample(1_000_000, 0);
        a.disks = vec![DiskRaw { id: "d".into(), read_bytes: 0, ..Default::default() }];
        r.push(a);
        // The wall clock moved an hour, the monotonic clock one second.
        let mut b = sample(1_000_000 + 3_600_000, 1000);
        b.disks = vec![DiskRaw { id: "d".into(), read_bytes: 9_999, ..Default::default() }];
        let f = r.push(b);
        assert!(f.gap);
        assert!(!f.first);
        assert_eq!(f.disks[0].read_rate, 0.0);
        // The next one is measured normally again.
        let mut c = sample(1_000_000 + 3_600_000 + 1000, 2000);
        c.disks = vec![DiskRaw { id: "d".into(), read_bytes: 10_999, ..Default::default() }];
        let f = r.push(c);
        assert!(!f.gap);
        assert_eq!(f.disks[0].read_rate, 1000.0);
    }

    #[test]
    fn a_stalled_sampler_is_a_gap_but_a_slow_sample_is_not() {
        let mut r = Rater::new();
        r.set_interval(1000);
        r.push(at(0));
        assert!(!r.push(at(3_500)).gap, "a few seconds of slow sampling must keep its rates");
        assert!(r.push(at(30_000)).gap);
    }

    #[test]
    fn dirty_names_are_cleaned_and_clean_ones_stay_shared() {
        let mut r = Rater::new();
        let mut p = proc_raw(1, 1, "evil\u{1B}[2J", 0);
        p.user = Some(Arc::from("bob"));
        let user = p.user.clone().unwrap();
        let mut s = at(0);
        s.processes = vec![p];
        let f = r.push(s);
        assert_eq!(&*f.procs[0].raw.name, "evil");
        assert!(Arc::ptr_eq(f.procs[0].raw.user.as_ref().unwrap(), &user));
    }

    #[test]
    fn a_text_is_checked_once_until_the_probe_refreshes_it() {
        let mut r = Rater::new();
        let dirty: Arc<str> = Arc::from("x\u{7}");
        let mut p = proc_raw(1, 1, "n", 0);
        p.cmdline = Some(dirty.clone());
        let mut s = at(0);
        s.processes = vec![p.clone()];
        let one = r.push(s).procs[0].raw.cmdline.clone().unwrap();
        let mut s = at(1000);
        s.processes = vec![p.clone()];
        let two = r.push(s).procs[0].raw.cmdline.clone().unwrap();
        assert!(Arc::ptr_eq(&one, &two), "the cached clean text is reused");
        p.cmdline = Some(Arc::from("refreshed\u{7}"));
        let mut s = at(2000);
        s.processes = vec![p];
        assert_eq!(&**r.push(s).procs[0].raw.cmdline.as_ref().unwrap(), "refreshed");
    }

    #[test]
    fn added_and_removed_track_the_process_table() {
        let mut r = Rater::new();
        let mut a = at(0);
        a.processes = vec![proc_raw(1, 1, "a", 0), proc_raw(2, 1, "b", 0)];
        r.push(a);
        let mut b = at(1000);
        b.processes = vec![proc_raw(2, 1, "b", 0), proc_raw(3, 1, "c", 0)];
        let f = r.push(b);
        assert_eq!(f.added, vec![key(3, 1)]);
        assert_eq!(f.removed, vec![key(1, 1)]);
    }

    #[test]
    fn a_sample_without_a_process_table_says_nothing_about_exits() {
        let mut r = Rater::new();
        let mut a = at(0);
        a.processes = vec![proc_raw(1, 1, "a", 0)];
        r.push(a);
        let f = r.push(at(1000));
        assert!(f.removed.is_empty());
        let mut c = at(2000);
        c.processes = vec![proc_raw(1, 1, "a", 0)];
        assert!(r.push(c).added.is_empty());
    }

    #[test]
    fn nan_values_never_reach_a_frame() {
        let mut r = Rater::new();
        let mut a = at(0);
        a.cpu_temp_c = Some(f32::NAN);
        a.load = Some([f32::NAN, 0.0, 0.0]);
        a.sensors = vec![crate::SensorRaw { value: f32::NAN, ..Default::default() }];
        a.gpus = vec![crate::GpuRaw { percent: Some(f32::INFINITY), ..Default::default() }];
        let f = r.push(a);
        assert_eq!(f.cpu.temp_c, None);
        assert_eq!(f.load, None);
        assert!(f.sensors.is_empty());
        assert_eq!(f.gpus[0].percent, None);
        let json = serde_json::to_string(&f).unwrap();
        let _: Frame = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn totals_leave_out_virtual_and_stacked_devices() {
        let mut r = Rater::new();
        let disk = |id: &str, n| DiskRaw { id: id.into(), read_bytes: n, ..Default::default() };
        let net =
            |id: &str, n, up, virt| NetRaw { id: id.into(), rx_bytes: n, up, virtual_if: virt, ..Default::default() };
        let mut a = at(0);
        a.disks = vec![disk("nvme0n1", 0), disk("dm-0", 0), disk("md127", 0), disk("zram0", 0), disk("mdisk", 0)];
        a.nets = vec![net("eth0", 0, true, false), net("lo", 0, true, true), net("eth1", 0, false, false)];
        r.push(a);
        let mut b = at(1000);
        b.disks =
            vec![disk("nvme0n1", 100), disk("dm-0", 100), disk("md127", 100), disk("zram0", 100), disk("mdisk", 7)];
        b.nets = vec![net("eth0", 50, true, false), net("lo", 50, true, true), net("eth1", 50, false, false)];
        let f = r.push(b);
        assert_eq!(f.disk_totals().0, 107.0);
        assert_eq!(f.net_totals().0, 50.0);
        assert_eq!(f.disks.len(), 5, "stacked devices are still listed");
    }
}
