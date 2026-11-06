//! One sample turned into rates and percentages: what every page shows. All records keep
//! `#[serde(default)]`, so files written by other versions still load. `None` stays `None`:
//! a value the OS does not offer is never a fake zero.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use soyuz_fs::sysmon::{BatteryRaw, GpuRaw, MemoryRaw, ProcKey, ProcRaw, ProcState, SensorKind, SensorRaw};

/// Everything known at one instant.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Frame {
    /// Unix time, milliseconds.
    pub at_ms: i64,
    pub uptime_secs: u64,
    pub load: Option<[f32; 3]>,
    /// The first sample of a run: every rate is 0.
    pub first: bool,
    /// The clocks jumped or the sampler stalled (suspend, debugger, clock step): every rate
    /// is 0, history breaks its line and alerts restart their timers.
    pub gap: bool,
    /// Milliseconds since the previous sample, on the monotonic clock.
    pub elapsed_ms: u64,
    pub cpu: CpuFrame,
    pub memory: MemFrame,
    pub disks: Vec<DiskFrame>,
    pub nets: Vec<NetFrame>,
    pub gpus: Vec<GpuRaw>,
    pub sensors: Vec<SensorRaw>,
    pub batteries: Vec<BatteryRaw>,
    pub procs: Arc<[ProcRow]>,
    pub counts: Counts,
    /// Processes that appeared since the previous frame (empty on the first frame).
    pub added: Vec<ProcKey>,
    /// Processes that went away since the previous frame.
    pub removed: Vec<ProcKey>,
}

/// CPU load in percent of the machine (0..=100).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CpuFrame {
    pub percent: f32,
    pub user: f32,
    pub system: f32,
    pub iowait: f32,
    pub steal: f32,
    pub irq: f32,
    pub cores: Vec<CoreFrame>,
    pub mhz_avg: Option<u32>,
    pub temp_c: Option<f32>,
    pub ctx_per_sec: Option<f64>,
    pub irq_per_sec: Option<f64>,
}

/// One logical CPU, in percent of that CPU.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CoreFrame {
    pub percent: f32,
    pub user: f32,
    pub system: f32,
    pub mhz: Option<u32>,
}

/// Memory levels plus what follows from them.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MemFrame {
    pub raw: MemoryRaw,
    /// `total - available`.
    pub used: u64,
    pub percent: f32,
    pub swap_percent: f32,
    /// Bytes per second.
    pub page_in_rate: Option<f64>,
    pub page_out_rate: Option<f64>,
}

/// One physical disk; rates in bytes per second.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DiskFrame {
    pub id: String,
    pub model: String,
    pub read_rate: f64,
    pub write_rate: f64,
    pub read_iops: f64,
    pub write_iops: f64,
    pub busy_percent: Option<f32>,
    pub queue: Option<f32>,
    pub rotational: Option<bool>,
    pub size: Option<u64>,
    pub read_bytes: u64,
    pub write_bytes: u64,
}

impl DiskFrame {
    /// False for Linux stacked devices (`dm-*`, `md*`, `zram*`), whose I/O re-counts the disks
    /// below them: they are listed, but left out of the totals.
    pub fn in_totals(&self) -> bool {
        let id = self.id.as_str();
        let md = id.strip_prefix("md").is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()));
        !(md || id.starts_with("dm-") || id.starts_with("zram"))
    }
}

/// One network interface; rates per second.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NetFrame {
    pub id: String,
    pub name: String,
    pub description: String,
    pub rx_rate: f64,
    pub tx_rate: f64,
    pub rx_packets_rate: f64,
    pub tx_packets_rate: f64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
    pub up: bool,
    pub virtual_if: bool,
    pub link_bps: Option<u64>,
    pub addresses: Vec<String>,
    pub mac: Option<String>,
}

impl NetFrame {
    /// Only interfaces that are up and real count: loopback and bridges would count traffic twice.
    pub fn in_totals(&self) -> bool {
        self.up && !self.virtual_if
    }
}

/// One process with its rates.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcRow {
    pub raw: ProcRaw,
    /// CPU load where 100 is one logical CPU (at most `100 * ncpu`).
    pub cpu_percent: f32,
    /// Bytes per second; `None` when the process's counters cannot be read. On Windows this
    /// is all I/O of the process, including pipes and sockets.
    pub read_rate: Option<f64>,
    pub write_rate: Option<f64>,
    /// Resident memory as a share of the machine's.
    pub mem_percent: f32,
}

/// Machine-wide counts (the idle pseudo process is not counted).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Counts {
    pub processes: u32,
    pub threads: u64,
    pub handles: Option<u64>,
    pub running: u32,
}

impl Frame {
    /// Logical CPUs in this frame (at least 1).
    pub fn ncpu(&self) -> u32 {
        (self.cpu.cores.len() as u32).max(1)
    }

    /// Read and write bytes per second over the disks that count ([`DiskFrame::in_totals`]).
    pub fn disk_totals(&self) -> (f64, f64) {
        self.disks.iter().filter(|d| d.in_totals()).fold((0.0, 0.0), |(r, w), d| (r + d.read_rate, w + d.write_rate))
    }

    /// Received and sent bytes per second over the interfaces that count ([`NetFrame::in_totals`]).
    pub fn net_totals(&self) -> (f64, f64) {
        self.nets.iter().filter(|n| n.in_totals()).fold((0.0, 0.0), |(r, t), n| (r + n.rx_rate, t + n.tx_rate))
    }

    /// The hottest temperature known: temperature sensors, the CPU package, GPUs.
    pub fn max_temp_c(&self) -> Option<f32> {
        let sensors = self.sensors.iter().filter(|s| s.kind == SensorKind::Temperature).map(|s| s.value);
        let gpus = self.gpus.iter().filter_map(|g| g.temperature_c);
        sensors.chain(gpus).chain(self.cpu.temp_c).filter(|v| v.is_finite()).reduce(f32::max)
    }

    /// The row of a process, by its identity (O(n)).
    pub fn proc(&self, key: ProcKey) -> Option<&ProcRow> {
        self.procs.iter().find(|p| p.raw.key == key)
    }
}

/// The stable id of a run state (`disk_wait`); the filter's `state:` term and CSV use it.
pub fn state_id(s: ProcState) -> &'static str {
    match s {
        ProcState::Running => "running",
        ProcState::Sleeping => "sleeping",
        ProcState::DiskWait => "disk_wait",
        ProcState::Stopped => "stopped",
        ProcState::Zombie => "zombie",
        ProcState::Idle => "idle",
        ProcState::Suspended => "suspended",
        ProcState::Unknown => "unknown",
    }
}
