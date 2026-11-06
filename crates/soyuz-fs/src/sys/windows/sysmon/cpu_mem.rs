//! CPU time counters and memory levels.

use std::mem::size_of;
use std::ptr::null;

use windows_sys::Wdk::System::SystemInformation::{
    SystemPerformanceInformation, SystemProcessorPerformanceInformation,
};
use windows_sys::Win32::System::Power::{CallNtPowerInformation, PROCESSOR_POWER_INFORMATION, ProcessorInformation};
use windows_sys::Win32::System::ProcessStatus::{K32GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::{GetActiveProcessorCount, GetActiveProcessorGroupCount};

use super::pdh::Query;
use super::{Wrap32, as_bytes, query_exact, query_system, read_pod, ticks_to_ms, u32_at};
use crate::sysmon::{CoreRaw, CpuTimes, MemoryRaw};

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    /// The form of the system query that takes an input; here, a processor group number.
    fn NtQuerySystemInformationEx(
        class: i32,
        input: *const core::ffi::c_void,
        input_len: u32,
        info: *mut core::ffi::c_void,
        info_len: u32,
        returned: *mut u32,
    ) -> i32;
}

/// `SystemPageFileInformation`.
const SYSTEM_PAGEFILE_INFORMATION: i32 = 18;

/// `SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION`: 100 ns units; `kernel` includes idle, DPC and
/// interrupt time.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProcessorTimes {
    idle: i64,
    kernel: i64,
    user: i64,
    dpc: i64,
    interrupt: i64,
    interrupt_count: u32,
    _pad: u32,
}

/// `SYSTEM_PAGEFILE_INFORMATION`: sizes in pages.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PageFile {
    next: u32,
    total_pages: u32,
    in_use_pages: u32,
    peak_pages: u32,
    name_len: u16,
    name_max: u16,
    name_ptr: usize,
}

// Offsets inside SYSTEM_PERFORMANCE_INFORMATION (stable since Windows 7): four 8-byte counters,
// then 32-bit fields.
#[cfg(test)]
const PERF_AVAILABLE_PAGES: usize = 44;
const PERF_PAGE_READ_COUNT: usize = 80;
const PERF_DIRTY_PAGES_WRITE: usize = 96;
const PERF_MAPPED_PAGES_WRITE: usize = 104;
const PERF_CONTEXT_SWITCHES: usize = 296;
/// The struct is 344 bytes on 64-bit; the fields read here end before that.
const PERF_MIN_SIZE: usize = PERF_CONTEXT_SWITCHES + 4;

pub(super) struct CpuRead {
    pub total: CpuTimes,
    pub cores: Vec<CoreRaw>,
    pub ctx_switches: Option<u64>,
    pub interrupts: Option<u64>,
}

#[derive(Default)]
pub(super) struct CpuReader {
    buf: Vec<u64>,
    perf: Vec<u64>,
    /// Bytes of `perf` the last read filled (`None`: it failed).
    perf_len: Option<usize>,
    interrupts: Vec<Wrap32>,
    ctx: Wrap32,
    page_in: Wrap32,
    page_out_dirty: Wrap32,
    page_out_mapped: Wrap32,
    clock: Option<Query>,
    clock_tried: bool,
    power: Vec<PROCESSOR_POWER_INFORMATION>,
    /// Logical CPUs per processor group.
    groups: Vec<u32>,
    /// `% Processor Performance` per logical CPU, across groups.
    perf_percent: Vec<Option<f64>>,
    threads: Option<u64>,
    handles: Option<u64>,
}

impl CpuReader {
    pub(super) fn read(&mut self, hardware: bool) -> CpuRead {
        let mut total = CpuTimes::default();
        let mut cores = Vec::new();
        let mut interrupts = None;
        if self.groups.is_empty() {
            self.groups = processor_groups();
            let largest = self.groups.iter().copied().max().unwrap_or(1) as usize;
            self.buf.resize((largest * size_of::<ProcessorTimes>()).div_ceil(8), 0);
        }
        let mut count = 0u64;
        let mut any = false;
        for (group, &cpus) in self.groups.iter().enumerate() {
            // The kernel takes a size that is a whole number of records, one per processor.
            let words = (cpus as usize * size_of::<ProcessorTimes>()).div_ceil(8);
            let Some(window) = self.buf.get_mut(..words) else { continue };
            let Some(len) = query_group(group as u16, window) else { continue };
            any = true;
            let bytes = as_bytes(window, len);
            for i in 0..len / size_of::<ProcessorTimes>() {
                let Some(p) = read_pod::<ProcessorTimes>(bytes, i * size_of::<ProcessorTimes>()) else { break };
                let times = processor_times(&p);
                add(&mut total, &times);
                let index = cores.len();
                if self.interrupts.len() <= index {
                    self.interrupts.resize_with(index + 1, Wrap32::default);
                }
                count += self.interrupts[index].push(p.interrupt_count);
                cores.push(CoreRaw { times, mhz: None });
            }
        }
        if any {
            interrupts = Some(count);
        }
        self.read_clocks(&mut cores, hardware);
        let ctx_switches = self.read_perf();
        CpuRead { total, cores, ctx_switches, interrupts }
    }

    /// Context switches; also keeps the page counters for [`Self::memory`].
    fn read_perf(&mut self) -> Option<u64> {
        self.perf_len = query_system(SystemPerformanceInformation, &mut self.perf);
        let len = self.perf_len?;
        if len < PERF_MIN_SIZE {
            return None;
        }
        let b = as_bytes(&self.perf, len);
        Some(self.ctx.push(u32_at(b, PERF_CONTEXT_SWITCHES)?))
    }

    fn read_clocks(&mut self, cores: &mut [CoreRaw], hardware: bool) {
        let n = cores.len();
        if n == 0 {
            return;
        }
        self.power.resize(n, PROCESSOR_POWER_INFORMATION::default());
        let bytes = n * size_of::<PROCESSOR_POWER_INFORMATION>();
        // SAFETY: no input; `power` is writable for `bytes` bytes.
        let st = unsafe {
            CallNtPowerInformation(ProcessorInformation, null(), 0, self.power.as_mut_ptr().cast(), bytes as u32)
        };
        if st != 0 {
            return;
        }
        // The reported current clock is usually pinned at the base clock; the real one is
        // the base clock times `% Processor Performance`.
        self.perf_percent.clear();
        self.perf_percent.resize(n, None);
        // The power call answers for this thread's processor group only; other groups' CPUs
        // have their clock from the counter.
        let mut counter_mhz: Vec<Option<f64>> = vec![None; n];
        if hardware {
            if !self.clock_tried {
                self.clock_tried = true;
                self.clock = Query::with(&[
                    r"\Processor Information(*)\% Processor Performance",
                    r"\Processor Information(*)\Processor Frequency",
                ]);
            }
            if let Some(q) = &mut self.clock
                && q.collect()
            {
                let out = &mut self.perf_percent;
                let groups = &self.groups;
                q.values(0, |name, v| {
                    if let Some(i) = cpu_of_instance(name, groups)
                        && let Some(slot) = out.get_mut(i)
                    {
                        *slot = Some(v);
                    }
                });
                q.values(1, |name, v| {
                    if let Some(i) = cpu_of_instance(name, groups)
                        && let Some(slot) = counter_mhz.get_mut(i)
                    {
                        *slot = Some(v);
                    }
                });
            }
        }
        for (i, core) in cores.iter_mut().enumerate() {
            let p = &self.power[i];
            let mhz = match self.perf_percent[i] {
                Some(pct) if p.MaxMhz > 0 => (f64::from(p.MaxMhz) * pct / 100.0) as u32,
                _ if p.CurrentMhz > 0 => p.CurrentMhz,
                _ => counter_mhz[i].map_or(0, |v| v as u32),
            };
            core.mhz = (mhz > 0).then_some(mhz);
        }
    }

    pub(super) fn memory(&mut self) -> MemoryRaw {
        let mut m = MemoryRaw::default();
        let mut status = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
        // SAFETY: `status` is writable and its length field is set.
        if unsafe { GlobalMemoryStatusEx(&mut status) } != 0 {
            m.total = status.ullTotalPhys;
            m.available = status.ullAvailPhys;
        }
        let mut page = 4096u64;
        let mut info =
            PERFORMANCE_INFORMATION { cb: size_of::<PERFORMANCE_INFORMATION>() as u32, ..Default::default() };
        // SAFETY: `info` is writable for the size we pass.
        if unsafe { K32GetPerformanceInfo(&mut info, info.cb) } != 0 {
            page = info.PageSize as u64;
            m.commit_used = Some(info.CommitTotal as u64 * page);
            m.commit_limit = Some(info.CommitLimit as u64 * page);
            m.cached = Some(info.SystemCache as u64 * page);
            m.kernel = Some(info.KernelTotal as u64 * page);
            self.threads = Some(u64::from(info.ThreadCount));
            self.handles = Some(u64::from(info.HandleCount));
        }
        let mut files = Vec::new();
        if let Some(len) = query_system(SYSTEM_PAGEFILE_INFORMATION, &mut files) {
            let (total, used) = page_file_pages(as_bytes(&files, len));
            m.swap_total = total * page;
            m.swap_used = used * page;
        }
        if let Some(len) = self.perf_len.filter(|l| *l >= PERF_MIN_SIZE) {
            let b = as_bytes(&self.perf, len);
            if let Some(v) = u32_at(b, PERF_PAGE_READ_COUNT) {
                m.page_in = Some(self.page_in.push(v) * page);
            }
            if let (Some(d), Some(w)) = (u32_at(b, PERF_DIRTY_PAGES_WRITE), u32_at(b, PERF_MAPPED_PAGES_WRITE)) {
                m.page_out = Some((self.page_out_dirty.push(d) + self.page_out_mapped.push(w)) * page);
            }
        }
        m
    }

    pub(super) fn total_threads(&self) -> Option<u64> {
        self.threads
    }

    pub(super) fn total_handles(&self) -> Option<u64> {
        self.handles
    }

    /// Available pages according to the system performance table, for a layout check.
    #[cfg(test)]
    fn available_pages(&self) -> Option<u32> {
        u32_at(as_bytes(&self.perf, self.perf.len() * 8), PERF_AVAILABLE_PAGES)
    }
}

/// Logical CPUs per active processor group.
fn processor_groups() -> Vec<u32> {
    // SAFETY: no arguments.
    let groups = unsafe { GetActiveProcessorGroupCount() };
    // SAFETY: a group number below the count just read.
    let list: Vec<u32> = (0..groups).map(|g| unsafe { GetActiveProcessorCount(g) }).collect();
    if list.iter().any(|&n| n > 0) { list } else { vec![1] }
}

/// The processor times of one group. Windows without the group form still answers for the
/// group this process runs in, which is all it has below 64 logical CPUs.
fn query_group(group: u16, buf: &mut [u64]) -> Option<usize> {
    let bytes = buf.len() * 8;
    let mut ret = 0u32;
    // SAFETY: the input is a group number of the size we pass; `buf` is writable for `bytes`
    // bytes; `ret` is a valid out-pointer.
    let st = unsafe {
        NtQuerySystemInformationEx(
            SystemProcessorPerformanceInformation,
            (&raw const group).cast(),
            size_of::<u16>() as u32,
            buf.as_mut_ptr().cast(),
            bytes as u32,
            &mut ret,
        )
    };
    if st >= 0 {
        return Some((ret as usize).min(bytes));
    }
    if group == 0 { query_exact(SystemProcessorPerformanceInformation, buf) } else { None }
}

fn processor_times(p: &ProcessorTimes) -> CpuTimes {
    CpuTimes {
        user: ticks_to_ms(p.user),
        system: ticks_to_ms(p.kernel.saturating_sub(p.idle).saturating_sub(p.dpc).saturating_sub(p.interrupt)),
        idle: ticks_to_ms(p.idle),
        irq: ticks_to_ms(p.interrupt),
        softirq: ticks_to_ms(p.dpc),
        ..CpuTimes::default()
    }
}

fn add(total: &mut CpuTimes, t: &CpuTimes) {
    total.user += t.user;
    total.system += t.system;
    total.idle += t.idle;
    total.irq += t.irq;
    total.softirq += t.softirq;
}

/// `(total, in use)` pages over the page file chain. A broken link ends the walk.
fn page_file_pages(b: &[u8]) -> (u64, u64) {
    let (mut total, mut used) = (0u64, 0u64);
    let mut off = 0usize;
    while let Some(f) = read_pod::<PageFile>(b, off) {
        total += u64::from(f.total_pages);
        used += u64::from(f.in_use_pages);
        if (f.next as usize) < size_of::<PageFile>() {
            break;
        }
        off += f.next as usize;
    }
    (total, used)
}

/// `1,3` is logical CPU 3 of group 1, numbered across groups; `_Total` entries give `None`.
fn cpu_of_instance(name: &[u16], groups: &[u32]) -> Option<usize> {
    let s = String::from_utf16_lossy(name);
    let (group, cpu) = s.split_once(',')?;
    let group: usize = group.parse().ok()?;
    let cpu: u32 = cpu.parse().ok()?;
    if cpu >= *groups.get(group)? {
        return None;
    }
    let before: u32 = groups[..group].iter().sum();
    Some((before + cpu) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn layouts_match_the_documented_ones() {
        assert_eq!(size_of::<ProcessorTimes>(), 48);
        assert_eq!(size_of::<PageFile>(), 32);
        assert_eq!(size_of::<PROCESSOR_POWER_INFORMATION>(), 24);
    }

    #[test]
    fn kernel_time_excludes_idle_dpc_and_interrupts() {
        let t = processor_times(&ProcessorTimes {
            idle: 50_000,
            kernel: 100_000,
            user: 20_000,
            dpc: 10_000,
            interrupt: 10_000,
            ..ProcessorTimes::default()
        });
        assert_eq!((t.idle, t.system, t.user, t.softirq, t.irq), (5, 3, 2, 1, 1));
        // A kernel value below its parts stays at zero.
        let t = processor_times(&ProcessorTimes { idle: 50_000, ..ProcessorTimes::default() });
        assert_eq!(t.system, 0);
    }

    #[test]
    fn page_file_chain_is_bounded() {
        let one = |next, total, used| {
            let mut v = vec![0u8; 32];
            v[0..4].copy_from_slice(&u32::to_le_bytes(next));
            v[4..8].copy_from_slice(&u32::to_le_bytes(total));
            v[8..12].copy_from_slice(&u32::to_le_bytes(used));
            v
        };
        let chain = [one(32, 100, 10), one(0, 50, 5)].concat();
        assert_eq!(page_file_pages(&chain), (150, 15));
        assert_eq!(page_file_pages(&one(1, 100, 10)), (100, 10));
        assert_eq!(page_file_pages(&[]), (0, 0));
        assert_eq!(page_file_pages(&one(4096, 100, 10)), (100, 10));
    }

    #[test]
    fn processor_instances() {
        let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
        let groups = [64, 32];
        assert_eq!(cpu_of_instance(&w("0,12"), &groups), Some(12));
        assert_eq!(cpu_of_instance(&w("1,3"), &groups), Some(67));
        assert_eq!(cpu_of_instance(&w("0,_Total"), &groups), None);
        assert_eq!(cpu_of_instance(&w("_Total"), &groups), None);
        assert_eq!(cpu_of_instance(&w("2,0"), &groups), None);
        assert_eq!(cpu_of_instance(&w("1,32"), &groups), None);
    }

    /// The group form of the query answers on the real machine, for the same CPUs.
    #[test]
    fn group_query_covers_every_processor() {
        let groups = processor_groups();
        let total: u32 = groups.iter().sum();
        for (g, &cpus) in groups.iter().enumerate() {
            let mut buf = vec![0u64; cpus as usize * size_of::<ProcessorTimes>() / 8];
            let len = query_group(g as u16, &mut buf).expect("group query");
            assert_eq!(len, cpus as usize * size_of::<ProcessorTimes>());
        }
        assert_eq!(CpuReader::default().read(false).cores.len(), total as usize);
    }

    /// The hand-kept offsets in the performance table agree with a documented API.
    #[test]
    fn performance_table_offsets_agree_with_the_memory_status() {
        let mut r = CpuReader::default();
        r.read(false);
        let pages = u64::from(r.available_pages().expect("performance table"));
        let m = r.memory();
        let pages_status = m.available / 4096;
        let diff = pages.abs_diff(pages_status);
        // Memory moves between the calls; a wrong offset would be off by far more than 2 GB.
        assert!(diff < (2 << 30) / 4096, "{pages} vs {pages_status}");
    }
}
