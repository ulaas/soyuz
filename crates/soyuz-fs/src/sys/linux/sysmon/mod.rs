//! System monitor readers for Linux (see `crate::sysmon`): `/proc` and `/sys` reads parsed by
//! the pure `crate::sysmon::parse`, NVML through `dlopen`, systemd over D-Bus. No helper
//! program is ever spawned.

mod control;
mod hw;
mod lists;
mod nvml;
mod procs;

use std::collections::HashMap;
use std::ffi::CStr;
use std::fs::File;
use std::io::Read;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(crate) use control::{affinity, set_affinity, set_priority, signal};
pub(crate) use lists::{connections, process_details, service_control, services, startup_items};

use crate::sysmon::parse::{self, net as pnet, system as psys};
use crate::sysmon::{CoreRaw, DiskRaw, NetRaw, RawSample, SystemInfo, Want};

/// Per-process GPU use reads a file per DRM client and asks NVML: not on every sample.
const GPU_INTERVAL: Duration = Duration::from_millis(2500);

/// Clock ticks per second of `/proc` times.
pub(super) fn clk_tck() -> u64 {
    static V: OnceLock<u64> = OnceLock::new();
    // SAFETY: `sysconf` takes an integer and touches no caller memory.
    *V.get_or_init(|| u64::try_from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) }).ok().filter(|v| *v > 0).unwrap_or(100))
}

pub(super) fn page_size() -> u64 {
    static V: OnceLock<u64> = OnceLock::new();
    *V.get_or_init(|| {
        // SAFETY: `sysconf` takes an integer and touches no caller memory.
        u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok().filter(|v| *v > 0).unwrap_or(4096)
    })
}

/// Boot time, unix milliseconds. Read once: the kernel recomputes `btime` from the wall clock
/// and it can shift by a second, which must not change any process's [`ProcKey`] identity.
///
/// [`ProcKey`]: crate::sysmon::ProcKey
pub(super) fn boot_ms() -> i64 {
    static V: OnceLock<i64> = OnceLock::new();
    *V.get_or_init(|| {
        read_text("/proc/stat")
            .and_then(|s| psys::parse_stat(&s, clk_tck()).btime)
            .map_or(0, |b| b.saturating_mul(1000))
    })
}

/// A process's start time from `stat`'s `starttime` ticks, unix milliseconds.
pub(super) fn start_ms_of(starttime: u64) -> i64 {
    boot_ms().saturating_add((starttime.saturating_mul(1000) / clk_tck()).min(i64::MAX as u64) as i64)
}

/// `buf` holds the file on `true`.
pub(super) fn read_into(path: &str, buf: &mut Vec<u8>) -> bool {
    buf.clear();
    File::open(path).and_then(|mut f| f.read_to_end(buf)).is_ok()
}

pub(super) fn read_text(path: &str) -> Option<String> {
    std::fs::read(path).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// A one-line sysfs value, trimmed.
pub(super) fn read_trimmed(path: &str) -> Option<String> {
    read_text(path).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

fn mono_ms() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    // Process-wide, so a new `Probe` continues the same clock.
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Keeps per-process caches and reused buffers between samples.
pub struct Probe {
    buf: Vec<u8>,
    tick: u64,
    /// `None`: not a whole disk (a partition, or no `/sys/block` entry).
    disks: HashMap<String, Option<DiskStatic>>,
    procs: procs::ProcTable,
    hw: hw::Hardware,
    /// The last per-process GPU collection: it runs on its own cadence, not every sample.
    gpu_at: Option<Instant>,
    engine_totals: procs::EngineTotals,
}

#[derive(Clone)]
struct DiskStatic {
    model: String,
    rotational: Option<bool>,
    size: Option<u64>,
}

impl Probe {
    pub fn new() -> Self {
        Probe {
            buf: Vec::with_capacity(16 * 1024),
            tick: 0,
            disks: HashMap::new(),
            procs: procs::ProcTable::new(),
            hw: hw::Hardware::new(),
            gpu_at: None,
            engine_totals: HashMap::new(),
        }
    }

    pub fn sample(&mut self, want: Want) -> RawSample {
        let mut out = RawSample {
            at_ms: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64),
            mono_ms: mono_ms(),
            ..RawSample::default()
        };
        if read_into("/proc/uptime", &mut self.buf) {
            out.uptime_secs = psys::parse_uptime(&String::from_utf8_lossy(&self.buf)).unwrap_or(0);
        }
        if read_into("/proc/loadavg", &mut self.buf) {
            (out.load, out.total_threads) = psys::parse_loadavg(&String::from_utf8_lossy(&self.buf));
        }
        self.read_cpu(want, &mut out);
        self.read_memory(&mut out);
        self.read_disks(&mut out);
        self.read_nets(&mut out);

        if want.processes {
            let due = want.process_gpu && self.gpu_at.is_none_or(|t| t.elapsed() >= GPU_INTERVAL);
            let mut pass = due.then(|| procs::GpuPass { nvml: self.hw.nvml_process_usage(), ..Default::default() });
            out.processes = self.procs.read(want, pass.as_mut());
            if let Some(pass) = pass {
                self.gpu_at = Some(Instant::now());
                self.engine_totals = pass.totals;
            }
        }
        if !want.process_gpu {
            self.engine_totals.clear();
        }
        if want.hardware {
            out.sensors = self.hw.sensors();
            out.cpu_temp_c = parse::hw::pick_cpu_temp(&out.sensors);
            out.gpus = self.hw.gpus(&self.engine_totals);
            out.batteries = hw::batteries();
        }
        self.tick = self.tick.wrapping_add(1);
        out
    }

    fn read_cpu(&mut self, want: Want, out: &mut RawSample) {
        if !read_into("/proc/stat", &mut self.buf) {
            return;
        }
        let stat = psys::parse_stat(&String::from_utf8_lossy(&self.buf), clk_tck());
        out.cpu_total = stat.total;
        out.ctx_switches = stat.ctx_switches;
        out.interrupts = stat.interrupts;
        let mut path = String::new();
        let mut any_freq = false;
        out.cores = stat
            .cores
            .iter()
            .zip(&stat.core_ids)
            .map(|(times, id)| {
                use std::fmt::Write;
                path.clear();
                let _ = write!(path, "/sys/devices/system/cpu/cpu{id}/cpufreq/scaling_cur_freq");
                let mhz = read_into(&path, &mut self.buf)
                    .then(|| String::from_utf8_lossy(&self.buf).trim().parse::<u64>().ok())
                    .flatten()
                    .map(|khz| (khz / 1000) as u32);
                any_freq |= mhz.is_some();
                CoreRaw { times: *times, mhz }
            })
            .collect();
        // No cpufreq (virtual machines): `/proc/cpuinfo` is slower, so hardware reads only.
        if !any_freq && want.hardware && read_into("/proc/cpuinfo", &mut self.buf) {
            let info = psys::parse_cpuinfo(&String::from_utf8_lossy(&self.buf));
            for (core, mhz) in out.cores.iter_mut().zip(info.mhz) {
                core.mhz = Some(mhz);
            }
        }
    }

    fn read_memory(&mut self, out: &mut RawSample) {
        if read_into("/proc/meminfo", &mut self.buf) {
            out.memory = psys::parse_meminfo(&String::from_utf8_lossy(&self.buf));
        }
        if read_into("/proc/vmstat", &mut self.buf) {
            (out.memory.page_in, out.memory.page_out) = psys::parse_vmstat(&String::from_utf8_lossy(&self.buf));
        }
    }

    fn read_disks(&mut self, out: &mut RawSample) {
        // Capacity and media change rarely: forget what is known once a minute or so.
        if self.tick.is_multiple_of(60) {
            self.disks.clear();
        }
        if !read_into("/proc/diskstats", &mut self.buf) {
            return;
        }
        let mut zram = None;
        let text = String::from_utf8_lossy(&self.buf);
        for line in text.lines() {
            let Some(d) = psys::parse_diskstats_line(line) else { continue };
            if d.name.starts_with("zram") {
                let mm = read_text(&format!("/sys/block/{}/mm_stat", d.name));
                if let Some(c) = mm.and_then(|s| psys::parse_zram_mm_stat(&s)) {
                    zram = Some(zram.unwrap_or(0u64).saturating_add(c));
                }
            }
            if !psys::is_listed_disk_name(d.name) {
                continue;
            }
            let info = self.disks.entry(d.name.to_owned()).or_insert_with(|| disk_static(d.name));
            let Some(info) = info else { continue };
            // An empty card reader or optical drive has nothing to show.
            if info.size == Some(0) {
                continue;
            }
            out.disks.push(DiskRaw {
                id: d.name.to_owned(),
                model: info.model.clone(),
                read_bytes: d.read_sectors.saturating_mul(512),
                write_bytes: d.write_sectors.saturating_mul(512),
                read_ops: d.read_ops,
                write_ops: d.write_ops,
                busy_ms: Some(d.io_ms),
                busy_percent: None,
                queue: Some(d.in_flight as f32),
                rotational: info.rotational,
                size: info.size,
            });
        }
        if let Some(z) = zram {
            out.memory.compressed = Some(out.memory.compressed.unwrap_or(0).saturating_add(z));
        }
    }

    fn read_nets(&mut self, out: &mut RawSample) {
        if !read_into("/proc/net/dev", &mut self.buf) {
            return;
        }
        let mut ifaddrs = if_addresses();
        let text = String::from_utf8_lossy(&self.buf);
        for line in text.lines() {
            let Some(d) = pnet::parse_net_dev_line(line) else { continue };
            let sys = format!("/sys/class/net/{}", d.name);
            let oper = read_trimmed(&format!("{sys}/operstate")).unwrap_or_default();
            let info = ifaddrs.remove(d.name).unwrap_or_default();
            // Loopback and many tunnels report `unknown`: the interface flag decides then.
            let up = oper == "up" || (oper == "unknown" && info.up);
            let description = std::fs::read_link(format!("{sys}/device/driver"))
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_default();
            out.nets.push(NetRaw {
                id: d.name.to_owned(),
                name: d.name.to_owned(),
                description,
                rx_bytes: d.rx_bytes,
                tx_bytes: d.tx_bytes,
                rx_packets: d.rx_packets,
                tx_packets: d.tx_packets,
                rx_errors: d.rx_errors,
                tx_errors: d.tx_errors,
                up,
                virtual_if: std::path::Path::new(&format!("/sys/devices/virtual/net/{}", d.name)).exists(),
                link_bps: if up {
                    read_text(&format!("{sys}/speed")).and_then(|s| pnet::parse_link_speed(&s))
                } else {
                    None
                },
                addresses: info.addresses,
                mac: read_text(&format!("{sys}/address")).and_then(|s| pnet::parse_mac(&s)),
            });
        }
    }
}

fn disk_static(name: &str) -> Option<DiskStatic> {
    let base = format!("/sys/block/{name}");
    // Only whole disks are listed under /sys/block; partitions fall out here.
    let size = read_trimmed(&format!("{base}/size"))?.parse::<u64>().ok().map(|s| s.saturating_mul(512));
    let rotational = read_trimmed(&format!("{base}/queue/rotational")).map(|r| r == "1");
    // Stacked devices have no hardware model; a device-mapper name or RAID level says what they are.
    let model = read_trimmed(&format!("{base}/device/model"))
        .or_else(|| read_trimmed(&format!("{base}/dm/name")))
        .or_else(|| read_trimmed(&format!("{base}/md/level")))
        .unwrap_or_default();
    Some(DiskStatic { model, rotational, size })
}

#[derive(Default)]
struct IfInfo {
    up: bool,
    addresses: Vec<String>,
}

/// Interface flags and addresses by name, from `getifaddrs`.
fn if_addresses() -> HashMap<String, IfInfo> {
    let mut out: HashMap<String, IfInfo> = HashMap::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `head` is a valid out-pointer; on success the list is freed below.
    if unsafe { libc::getifaddrs(&raw mut head) } != 0 {
        return out;
    }
    let mut cur = head;
    while !cur.is_null() {
        // SAFETY: `cur` is a node of the list `getifaddrs` returned, alive until `freeifaddrs`.
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        if ifa.ifa_name.is_null() {
            continue;
        }
        // SAFETY: `ifa_name` is a NUL-terminated string owned by the list.
        let name = unsafe { CStr::from_ptr(ifa.ifa_name) }.to_string_lossy().into_owned();
        let e = out.entry(name).or_default();
        e.up |= ifa.ifa_flags & libc::IFF_UP as u32 != 0;
        if ifa.ifa_addr.is_null() {
            continue;
        }
        // SAFETY: a non-null `ifa_addr` points to a sockaddr of at least the family field.
        let family = i32::from(unsafe { (*ifa.ifa_addr).sa_family });
        if family == libc::AF_INET {
            // SAFETY: family AF_INET means the address is a `sockaddr_in`.
            let a = unsafe { (*ifa.ifa_addr.cast::<libc::sockaddr_in>()).sin_addr.s_addr };
            e.addresses.push(Ipv4Addr::from(u32::from_be(a)).to_string());
        } else if family == libc::AF_INET6 {
            // SAFETY: family AF_INET6 means the address is a `sockaddr_in6`.
            let a = unsafe { (*ifa.ifa_addr.cast::<libc::sockaddr_in6>()).sin6_addr.s6_addr };
            e.addresses.push(Ipv6Addr::from(a).to_string());
        }
    }
    // SAFETY: `head` came from a successful `getifaddrs` and is freed exactly once; no reference
    // into the list outlives this point.
    unsafe { libc::freeifaddrs(head) };
    out
}

pub fn system_info() -> SystemInfo {
    let cpuinfo_text = read_text("/proc/cpuinfo").unwrap_or_default();
    let cpu = psys::parse_cpuinfo(&cpuinfo_text);
    let meminfo = read_text("/proc/meminfo").map(|s| psys::parse_meminfo(&s)).unwrap_or_default();
    let logical =
        if cpu.logical > 0 { cpu.logical } else { std::thread::available_parallelism().map_or(1, |n| n.get() as u32) };
    let (mut l1, mut l2, mut l3) = (None, None, None);
    for i in 0..8 {
        let dir = format!("/sys/devices/system/cpu/cpu0/cache/index{i}");
        let Some(level) = read_trimmed(&format!("{dir}/level")) else { continue };
        let kind = read_trimmed(&format!("{dir}/type")).unwrap_or_default();
        let size = read_trimmed(&format!("{dir}/size")).and_then(|s| psys::parse_cache_size(&s));
        match level.as_str() {
            "1" if kind != "Instruction" => l1 = size,
            "2" => l2 = size,
            "3" => l3 = size,
            _ => {}
        }
    }
    let cpu_max_mhz = read_trimmed("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")
        .and_then(|s| s.parse::<u64>().ok())
        .map(|khz| (khz / 1000) as u32);
    SystemInfo {
        hostname: read_trimmed("/proc/sys/kernel/hostname").unwrap_or_default(),
        os: read_text("/etc/os-release")
            .or_else(|| read_text("/usr/lib/os-release"))
            .and_then(|s| psys::parse_os_release(&s))
            .unwrap_or_else(|| "Linux".to_owned()),
        kernel: read_trimmed("/proc/sys/kernel/osrelease").unwrap_or_default(),
        arch: std::env::consts::ARCH.to_owned(),
        cpu_model: cpu.model,
        cpu_vendor: cpu.vendor,
        physical_cores: cpu.physical.max(1),
        logical_cores: logical,
        sockets: cpu.sockets,
        cpu_max_mhz,
        cache_l1_kb: l1,
        cache_l2_kb: l2,
        cache_l3_kb: l3,
        memory_total: meminfo.total,
        memory_kind: None,
        boot_unix: boot_ms() / 1000,
        virtualization: psys::detect_virtualization(
            &read_text("/proc/version").unwrap_or_default(),
            &read_text("/sys/class/dmi/id/product_name").unwrap_or_default(),
            &read_text("/sys/hypervisor/type").unwrap_or_default(),
            std::path::Path::new("/.dockerenv").exists(),
            cpuinfo_text
                .lines()
                .any(|l| l.starts_with("flags") && l.split_ascii_whitespace().any(|f| f == "hypervisor")),
        ),
    }
}
