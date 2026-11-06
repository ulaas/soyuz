//! Raw system readings for the system monitor (`soyuz-sysmon` turns them into rates,
//! history and tables). Everything here is a counter or a level as the OS reports it; no
//! rates, no formatting. Each OS fills in what it can: a value the OS does not offer is
//! `None`, never a made-up zero. All calls may block for milliseconds: worker threads only.
//!
//! The readers themselves are in `crate::sys::<os>::sysmon` (`Probe`, `connections`,
//! `services`, `process_details`, `signal`, `system_info`, …). Other systems get [`fallback`].

use serde::{Deserialize, Serialize};

mod types_proc;
pub use types_proc::*;

/// What one [`Probe::sample`] should read. Cheap parts are always read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Want {
    /// The process table (off for background history that needs system totals only).
    pub processes: bool,
    /// Per-process details that need a handle or extra files per process, cached per
    /// process where possible: path, command line, user, I/O, priority, …
    pub process_details: bool,
    /// Per-process GPU use (the most expensive reading).
    pub process_gpu: bool,
    /// GPU adapters, sensors, batteries (slower reads).
    pub hardware: bool,
}

impl Default for Want {
    fn default() -> Self {
        Want { processes: true, process_details: true, process_gpu: true, hardware: true }
    }
}

/// CPU time counters of one logical CPU or of the whole machine, in milliseconds since boot.
/// Fields the OS does not have stay 0. `idle` excludes `iowait`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuTimes {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
}

/// One logical CPU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CoreRaw {
    pub times: CpuTimes,
    /// Current clock, MHz.
    pub mhz: Option<u32>,
}

/// Memory levels in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryRaw {
    pub total: u64,
    /// Immediately usable without swapping (`MemAvailable`, Windows `ullAvailPhys`).
    pub available: u64,
    /// File cache that can be dropped.
    pub cached: Option<u64>,
    /// Kernel / block buffers (Linux).
    pub buffers: Option<u64>,
    /// Shared / tmpfs (Linux).
    pub shared: Option<u64>,
    /// Non-pageable kernel memory (Linux `Slab`, Windows paged + nonpaged pool, macOS wired).
    pub kernel: Option<u64>,
    pub swap_total: u64,
    pub swap_used: u64,
    /// Committed memory and its limit (Windows commit charge; Linux `Committed_AS` / `CommitLimit`).
    pub commit_used: Option<u64>,
    pub commit_limit: Option<u64>,
    /// Memory compressed by the OS (macOS compressor, Linux zram; Windows when known).
    pub compressed: Option<u64>,
    /// Bytes paged in / out since boot (for page-rate graphs).
    pub page_in: Option<u64>,
    pub page_out: Option<u64>,
}

/// One physical disk (not a volume). Counters since boot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DiskRaw {
    /// Stable key (`sda`, `nvme0n1`, `PhysicalDrive0`, `disk0`).
    pub id: String,
    /// Model, when known; otherwise empty.
    pub model: String,
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub read_ops: u64,
    pub write_ops: u64,
    /// Milliseconds the disk was busy since boot (`None`: use `busy_percent`).
    pub busy_ms: Option<u64>,
    /// Instant busy percentage where the OS reports only a rate (Windows PDH).
    pub busy_percent: Option<f32>,
    /// Requests in flight / average queue length.
    pub queue: Option<f32>,
    /// `Some(true)`: spinning disk. `Some(false)`: SSD.
    pub rotational: Option<bool>,
    /// Capacity in bytes.
    pub size: Option<u64>,
}

/// One network interface. Counters since boot (or since the interface came up).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NetRaw {
    /// Stable key (`eth0`, Windows interface LUID as a string).
    pub id: String,
    /// What a person calls it (`Ethernet 2`, `wlan0`).
    pub name: String,
    pub description: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
    pub up: bool,
    /// Loopback, tunnels and other virtual interfaces.
    pub virtual_if: bool,
    /// Link speed, bits per second.
    pub link_bps: Option<u64>,
    pub addresses: Vec<String>,
    pub mac: Option<String>,
}

/// One GPU engine's load (`3D`, `Copy`, `Video Decode`, `Compute`, …).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GpuEngine {
    pub name: String,
    pub percent: f32,
}

/// One GPU adapter.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GpuRaw {
    /// Stable key (LUID, PCI address, `card0`).
    pub id: String,
    pub name: String,
    pub vendor: String,
    /// Overall load (highest engine when the OS has no total), percent.
    pub percent: Option<f32>,
    pub engines: Vec<GpuEngine>,
    pub mem_used: Option<u64>,
    pub mem_total: Option<u64>,
    /// System memory the GPU borrowed (Windows shared usage).
    pub shared_used: Option<u64>,
    pub temperature_c: Option<f32>,
    pub power_w: Option<f32>,
    pub fan_rpm: Option<u32>,
    pub clock_mhz: Option<u32>,
    pub mem_clock_mhz: Option<u32>,
}

/// What a sensor measures.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorKind {
    #[default]
    Temperature,
    Fan,
    Power,
    Voltage,
    Current,
}

/// One hardware sensor.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SensorRaw {
    /// Stable key.
    pub id: String,
    /// Chip or zone (`coretemp`, `amdgpu`, `ACPI Thermal Zone`).
    pub group: String,
    /// Label within the group (`Package id 0`, `fan1`).
    pub label: String,
    pub kind: SensorKind,
    /// °C, RPM, W, V or A.
    pub value: f32,
    pub max: Option<f32>,
    pub critical: Option<f32>,
}

/// State of one battery.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BatteryRaw {
    pub id: String,
    pub name: String,
    /// Charge, percent.
    pub percent: f32,
    pub charging: bool,
    pub on_ac: bool,
    pub seconds_left: Option<u64>,
    /// Discharge (negative) or charge (positive) power.
    pub power_w: Option<f32>,
    /// Current full capacity as a share of the design capacity, percent.
    pub health_percent: Option<f32>,
    pub cycles: Option<u32>,
}

/// Everything read at one instant.
#[derive(Clone, Debug, Default)]
pub struct RawSample {
    /// Unix time, milliseconds.
    pub at_ms: i64,
    /// Monotonic milliseconds (rates use this, never the wall clock).
    pub mono_ms: u64,
    pub uptime_secs: u64,
    /// 1, 5 and 15 minute load average (Unix).
    pub load: Option<[f32; 3]>,
    pub cpu_total: CpuTimes,
    pub cores: Vec<CoreRaw>,
    /// Package temperature when the OS gives one directly.
    pub cpu_temp_c: Option<f32>,
    /// Context switches / interrupts since boot.
    pub ctx_switches: Option<u64>,
    pub interrupts: Option<u64>,
    pub memory: MemoryRaw,
    pub disks: Vec<DiskRaw>,
    pub nets: Vec<NetRaw>,
    pub gpus: Vec<GpuRaw>,
    pub sensors: Vec<SensorRaw>,
    pub batteries: Vec<BatteryRaw>,
    pub processes: Vec<ProcRaw>,
    /// Threads and handles system-wide, where counted separately from the process table.
    pub total_threads: Option<u64>,
    pub total_handles: Option<u64>,
}

/// What does not change while running.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemInfo {
    pub hostname: String,
    /// `Windows 11 Pro 26300`, `Fedora Linux 44`, `macOS 15.2`.
    pub os: String,
    /// Kernel version.
    pub kernel: String,
    pub arch: String,
    pub cpu_model: String,
    pub cpu_vendor: String,
    pub physical_cores: u32,
    pub logical_cores: u32,
    pub sockets: u32,
    /// Base / maximum clock.
    pub cpu_max_mhz: Option<u32>,
    pub cache_l1_kb: Option<u32>,
    pub cache_l2_kb: Option<u32>,
    pub cache_l3_kb: Option<u32>,
    pub memory_total: u64,
    /// Memory speed / type when cheaply known (`DDR5 6000`).
    pub memory_kind: Option<String>,
    /// Boot time, unix seconds.
    pub boot_unix: i64,
    /// Virtual machine / container, when detectable (`WSL`, `KVM`, `Hyper-V`).
    pub virtualization: Option<String>,
}

/// An error from a control call, worded for a toast by the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlError {
    /// The process is gone (or the PID now belongs to another process).
    Gone,
    Denied,
    Unsupported,
    Other(String),
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ControlError::Gone => f.write_str("the process is gone"),
            ControlError::Denied => f.write_str("access denied"),
            ControlError::Unsupported => f.write_str("not supported on this system"),
            ControlError::Other(s) => f.write_str(s),
        }
    }
}

/// Keeps per-process caches and query handles between samples.
pub struct Probe(imp::Probe);

impl Probe {
    /// Never fails: what cannot be opened reads as "not available".
    pub fn new() -> Self {
        Probe(imp::Probe::new())
    }

    /// Read now. A worker thread's call; may take tens of milliseconds.
    pub fn sample(&mut self, want: Want) -> RawSample {
        self.0.sample(want)
    }
}

impl Default for Probe {
    fn default() -> Self {
        Self::new()
    }
}

/// See [`SystemInfo`].
pub fn system_info() -> SystemInfo {
    imp::system_info()
}

/// Open sockets with their owning process.
pub fn connections() -> Vec<Connection> {
    imp::connections()
}

/// Installed services / daemons.
pub fn services() -> Vec<ServiceInfo> {
    imp::services()
}

/// Start, stop or restart a service.
pub fn service_control(name: &str, op: ServiceOp) -> Result<(), ControlError> {
    imp::service_control(name, op)
}

/// Programs that start with the session or machine (read-only list).
pub fn startup_items() -> Vec<StartupItem> {
    imp::startup_items()
}

/// On-demand details of one process: the lists `want` asks for (each can take milliseconds
/// to seconds on a big program, so never per frame).
pub fn process_details(key: ProcKey, want: DetailsWant) -> Result<ProcDetails, ControlError> {
    imp::process_details(key, want)
}

/// Terminate, kill, suspend, resume. Refuses with `Gone` when `key.start_raw` no longer matches
/// the PID. The check and the action use one handle / pidfd where the OS has them.
pub fn signal(key: ProcKey, sig: ProcSignal) -> Result<(), ControlError> {
    imp::signal(key, sig)
}

/// Change the priority class / niceness.
pub fn set_priority(key: ProcKey, p: ProcPriority) -> Result<(), ControlError> {
    imp::set_priority(key, p)
}

/// The logical CPUs a process may run on.
pub fn affinity(key: ProcKey) -> Result<Vec<u32>, ControlError> {
    imp::affinity(key)
}

/// Restrict a process to `cpus` (logical CPU numbers).
pub fn set_affinity(key: ProcKey, cpus: &[u32]) -> Result<(), ControlError> {
    imp::set_affinity(key, cpus)
}

#[cfg(target_os = "linux")]
use crate::sys::linux::sysmon as imp;
#[cfg(target_os = "macos")]
use crate::sys::macos::sysmon as imp;
#[cfg(windows)]
use crate::sys::windows::sysmon as imp;
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
use fallback as imp;

/// Readers that report nothing: other Unixes, and the template of the per-OS API.
#[allow(dead_code)]
pub(crate) mod fallback;
