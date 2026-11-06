//! System monitor readers (see `crate::sysmon`): ntdll tables, performance counters, the
//! kernel graphics interface, iphlpapi, powrprof and the service manager. No COM, no WMI.
//! A value that cannot be read stays `None`; odd data never panics.

mod control;
mod cpu_mem;
mod disks;
mod gpu;
mod info;
mod net;
mod pdh;
mod procs;
mod sensors;
mod services;
mod startup;

use std::mem::size_of;
use std::time::{SystemTime, UNIX_EPOCH};

use windows_sys::Wdk::System::SystemInformation::NtQuerySystemInformation;
use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows_sys::Win32::System::SystemInformation::GetTickCount64;

pub use control::{affinity, process_details, set_affinity, set_priority, signal};
pub use info::system_info;
pub use net::connections;
pub use services::{service_control, services};
pub use startup::startup_items;

use crate::sysmon::{RawSample, SensorKind, SensorRaw, Want};

const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;
const STATUS_BUFFER_OVERFLOW: i32 = 0x8000_0005_u32 as i32;
const STATUS_BUFFER_TOO_SMALL: i32 = 0xC000_0023_u32 as i32;

/// The largest buffer any table query may grow to.
const MAX_TABLE_BYTES: usize = 256 << 20;

/// `NtQuerySystemInformation` into `buf` (kept 8-aligned by its element type, reused between
/// samples), growing it in a bounded loop. Returns the bytes written.
fn query_system(class: i32, buf: &mut Vec<u64>) -> Option<usize> {
    if buf.is_empty() {
        buf.resize(4096 / 8, 0);
    }
    for _ in 0..8 {
        let bytes = buf.len() * 8;
        let mut ret = 0u32;
        // SAFETY: `buf` is writable for `bytes` bytes, `ret` is a valid out-pointer.
        let st = unsafe { NtQuerySystemInformation(class, buf.as_mut_ptr().cast(), bytes as u32, &mut ret) };
        if st >= 0 {
            return Some((ret as usize).min(bytes));
        }
        if st != STATUS_INFO_LENGTH_MISMATCH && st != STATUS_BUFFER_TOO_SMALL && st != STATUS_BUFFER_OVERFLOW {
            return None;
        }
        // The table can grow between calls: leave room.
        let next = (ret as usize + (64 << 10)).max(bytes * 2);
        if next > MAX_TABLE_BYTES {
            return None;
        }
        buf.resize(next / 8, 0);
    }
    None
}

/// One `NtQuerySystemInformation` call with `buf` as is, for classes that want an exact size.
fn query_exact(class: i32, buf: &mut [u64]) -> Option<usize> {
    let bytes = buf.len() * 8;
    let mut ret = 0u32;
    // SAFETY: `buf` is writable for `bytes` bytes, `ret` is a valid out-pointer.
    let st = unsafe { NtQuerySystemInformation(class, buf.as_mut_ptr().cast(), bytes as u32, &mut ret) };
    (st >= 0).then(|| (ret as usize).min(bytes))
}

/// The first `len` bytes of an 8-aligned buffer.
fn as_bytes(buf: &[u64], len: usize) -> &[u8] {
    let len = len.min(buf.len() * 8);
    // SAFETY: the slice covers initialized memory owned by `buf`; u8 has no alignment needs
    // and every bit pattern is valid.
    unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), len) }
}

/// Read a plain integer-only `#[repr(C)]` struct at `off`, bounds checked and with no
/// alignment assumption.
fn read_pod<T: Copy>(buf: &[u8], off: usize) -> Option<T> {
    let end = off.checked_add(size_of::<T>())?;
    if end > buf.len() {
        return None;
    }
    // SAFETY: `off..end` lies inside `buf`; callers only use structs made of integers, for
    // which every bit pattern is valid; the read makes no alignment assumption.
    Some(unsafe { std::ptr::read_unaligned(buf.as_ptr().add(off).cast::<T>()) })
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    read_pod::<u32>(b, at)
}

/// UTF-16 text stored at the absolute address `ptr` when that range lies inside `buf`, the
/// way the kernel returns strings in the caller's buffer. Anything else reads as empty.
fn text_in_buffer(buf: &[u8], ptr: usize, byte_len: usize) -> String {
    let base = buf.as_ptr() as usize;
    let Some(off) = ptr.checked_sub(base) else { return String::new() };
    let Some(end) = off.checked_add(byte_len) else { return String::new() };
    let Some(bytes) = buf.get(off..end) else { return String::new() };
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    String::from_utf16_lossy(&units)
}

/// FILETIME ticks to unix milliseconds (0 when not set).
fn ticks_to_unix_ms(t: i64) -> i64 {
    if t <= 0 { 0 } else { (t - super::EPOCH_DIFF_TICKS as i64) / 10_000 }
}

/// 100 ns units to milliseconds.
fn ticks_to_ms(t: i64) -> u64 {
    (t.max(0) as u64) / 10_000
}

/// Monotonic milliseconds, counting through sleep.
fn mono_ms() -> u64 {
    let (mut now, mut freq) = (0i64, 0i64);
    // SAFETY: valid out-pointers; both calls cannot fail on supported Windows.
    let ok = unsafe { QueryPerformanceCounter(&mut now) != 0 && QueryPerformanceFrequency(&mut freq) != 0 };
    if ok && freq > 0 {
        ((now as i128 * 1000) / i128::from(freq)) as u64
    } else {
        // SAFETY: no arguments.
        unsafe { GetTickCount64() }
    }
}

/// Extends a 32-bit counter that wraps into a 64-bit one.
#[derive(Default)]
struct Wrap32 {
    last: Option<u32>,
    total: u64,
}

impl Wrap32 {
    fn push(&mut self, v: u32) -> u64 {
        match self.last {
            Some(last) => self.total += u64::from(v.wrapping_sub(last)),
            None => self.total = u64::from(v),
        }
        self.last = Some(v);
        self.total
    }
}

/// Keeps per-process caches and query handles between samples.
pub struct Probe {
    procs: procs::ProcReader,
    cpu: cpu_mem::CpuReader,
    disks: disks::DiskReader,
    net: net::NetReader,
    gpu: gpu::GpuReader,
    sensors: sensors::SensorReader,
}

impl Probe {
    pub fn new() -> Self {
        Probe {
            procs: procs::ProcReader::default(),
            cpu: cpu_mem::CpuReader::default(),
            disks: disks::DiskReader::default(),
            net: net::NetReader::default(),
            gpu: gpu::GpuReader::default(),
            sensors: sensors::SensorReader::default(),
        }
    }

    pub fn sample(&mut self, want: Want) -> RawSample {
        let at_ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        let mono = mono_ms();
        // SAFETY: no arguments.
        let uptime_secs = unsafe { GetTickCount64() } / 1000;

        let cpu = self.cpu.read(want.hardware);
        let mut memory = self.cpu.memory();
        let disks = self.disks.read(want.hardware);
        let nets = self.net.read();
        let (gpus, proc_gpu) = self.gpu.read(want.hardware, want.processes && want.process_gpu);
        let table = if want.processes { self.procs.read(want, proc_gpu.as_ref()) } else { procs::Table::default() };

        memory.compressed = table.compressed;
        let mut sensors = Vec::new();
        let mut batteries = Vec::new();
        if want.hardware {
            sensors = self.sensors.read();
            sensors.extend(self.disks.sensors());
            for g in &gpus {
                if let Some(t) = g.temperature_c {
                    sensors.push(SensorRaw {
                        id: format!("gpu:{}:temp", g.id),
                        group: g.name.clone(),
                        label: "Temperature".to_string(),
                        kind: SensorKind::Temperature,
                        value: t,
                        max: self.gpu.temperature_limits(&g.id).0,
                        critical: self.gpu.temperature_limits(&g.id).1,
                    });
                }
                if let Some(rpm) = g.fan_rpm {
                    sensors.push(SensorRaw {
                        id: format!("gpu:{}:fan", g.id),
                        group: g.name.clone(),
                        label: "Fan".to_string(),
                        kind: SensorKind::Fan,
                        value: rpm as f32,
                        max: None,
                        critical: None,
                    });
                }
            }
            batteries = sensors::batteries();
        }

        RawSample {
            at_ms,
            mono_ms: mono,
            uptime_secs,
            load: None,
            cpu_total: cpu.total,
            cores: cpu.cores,
            cpu_temp_c: None,
            ctx_switches: cpu.ctx_switches,
            interrupts: cpu.interrupts,
            memory,
            disks,
            nets,
            gpus,
            sensors,
            batteries,
            total_threads: if want.processes { Some(table.threads) } else { self.cpu.total_threads() },
            total_handles: if want.processes { Some(table.handles) } else { self.cpu.total_handles() },
            processes: table.rows,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cost of each part on this machine: `--release --ignored --nocapture sysmon_part_timings`.
    #[test]
    #[ignore = "prints timings of the real machine"]
    fn sysmon_part_timings() {
        use std::time::{Duration, Instant};
        fn time(label: &str, rounds: u32, mut f: impl FnMut()) {
            let (mut sum, mut best, mut worst) = (Duration::ZERO, Duration::MAX, Duration::ZERO);
            for _ in 0..rounds {
                std::thread::sleep(Duration::from_millis(200));
                let t = Instant::now();
                f();
                let e = t.elapsed();
                sum += e;
                best = best.min(e);
                worst = worst.max(e);
            }
            println!("{label:28} avg {:>10.3?} best {best:>10.3?} worst {worst:>10.3?}", sum / rounds);
        }
        let n = 15;
        let mut cpu = cpu_mem::CpuReader::default();
        time("cpu, no clocks", n, || drop(cpu.read(false).cores));
        time("cpu with clock counter", n, || drop(cpu.read(true).cores));
        time("memory", n, || {
            let _ = cpu.memory();
        });
        let mut disks = disks::DiskReader::default();
        time("disks", n, || drop(disks.read(true)));
        let mut net = net::NetReader::default();
        time("network", n, || drop(net.read()));
        let mut sensors = sensors::SensorReader::default();
        time("thermal and power", n, || drop(sensors.read()));
        time("batteries", n, || drop(sensors::batteries()));
        let want = Want { hardware: false, process_gpu: false, ..Want::default() };
        let mut procs = procs::ProcReader::default();
        time("processes, cached details", n, || drop(procs.read(want, None).rows));
        let mut buf = vec![0u64; (1 << 20) / 8];
        time("process table call alone", n, || {
            query_system(windows_sys::Wdk::System::SystemInformation::SystemProcessInformation, &mut buf);
        });
        gpu::part_timings(&mut |label, f| time(label, n, f));
        let mut q = pdh::Query::with(&[r"\GPU Engine(*)\Utilization Percentage"]);
        let mut instances = 0;
        time("pdh gpu engine", n, || {
            if let Some(q) = &mut q {
                q.collect();
                instances = 0;
                q.values(0, |_, _| instances += 1);
            }
        });
        println!("gpu engine instances: {instances}");
        let mut q = pdh::Query::with(&[r"\GPU Process Memory(*)\Dedicated Usage"]);
        time("pdh gpu process memory", n, || {
            if let Some(q) = &mut q {
                q.collect();
                q.values(0, |_, _| {});
            }
        });
        let mut q = pdh::Query::with(&[r"\GPU Adapter Memory(*)\Dedicated Usage"]);
        time("pdh gpu adapter memory", n, || {
            if let Some(q) = &mut q {
                q.collect();
                q.values(0, |_, _| {});
            }
        });
    }

    #[test]
    fn probe_is_send() {
        fn check<T: Send>() {}
        check::<Probe>();
    }

    #[test]
    fn wrap32_extends_across_the_wrap() {
        let mut w = Wrap32::default();
        assert_eq!(w.push(u32::MAX - 1), u64::from(u32::MAX - 1));
        assert_eq!(w.push(1), u64::from(u32::MAX) + 2);
    }

    #[test]
    fn read_pod_is_bounds_checked() {
        let b = [1u8, 0, 0, 0, 2];
        assert_eq!(read_pod::<u32>(&b, 0), Some(1));
        assert_eq!(read_pod::<u32>(&b, 2), None);
        assert_eq!(read_pod::<u32>(&b, usize::MAX), None);
    }

    #[test]
    fn text_outside_the_buffer_reads_as_empty() {
        let b: Vec<u8> = "ab".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let base = b.as_ptr() as usize;
        assert_eq!(text_in_buffer(&b, base, 4), "ab");
        assert_eq!(text_in_buffer(&b, base, 6), "");
        assert_eq!(text_in_buffer(&b, base.wrapping_sub(2), 2), "");
        assert_eq!(text_in_buffer(&b, 0, 2), "");
        assert_eq!(text_in_buffer(&b, usize::MAX, 2), "");
    }

    #[test]
    fn filetime_to_unix() {
        assert_eq!(ticks_to_unix_ms(0), 0);
        assert_eq!(ticks_to_unix_ms(super::super::EPOCH_DIFF_TICKS as i64 + 10_000), 1);
        assert_eq!(ticks_to_ms(-5), 0);
        assert_eq!(ticks_to_ms(25_000), 2);
    }
}
