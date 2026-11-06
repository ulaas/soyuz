//! System monitor readers for macOS (see `crate::sysmon`): libproc, Mach host statistics,
//! `sysctl` and IOKit registry properties. No helper program is ever spawned.
//!
//! Written from the documented APIs and compile-checked only: there is no Mac to run this on
//! yet, so treat the values as untested until the smoke test `tests/sysmon_smoke_unix.rs` has
//! run on one. SMC sensors are private API and left out.

// `libc` deprecates its Mach declarations in favour of the `mach2` crate; they are correct, and
// these few calls do not justify another dependency.
#![allow(deprecated)]

mod iokit;
mod lists;
mod net;
mod procs;

use std::ffi::{CStr, c_int, c_void};
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub(crate) use lists::{
    affinity, connections, process_details, service_control, services, set_affinity, set_priority, signal,
    startup_items,
};

use crate::sysmon::{CoreRaw, CpuTimes, MemoryRaw, RawSample, SystemInfo, Want};

fn mono_ms() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    // Process-wide, so a new `Probe` continues the same clock.
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// A fixed-size `sysctlbyname` value.
pub(super) fn sysctl_val<T: Copy + Default>(name: &CStr) -> Option<T> {
    let mut v = T::default();
    let mut len = size_of::<T>();
    // SAFETY: `v` is writable for `len` bytes and `name` is NUL-terminated. Callers use plain
    // integer or C struct types, for which any bytes the kernel writes are valid.
    let rc = unsafe {
        libc::sysctlbyname(name.as_ptr(), (&raw mut v).cast::<c_void>(), &raw mut len, std::ptr::null_mut(), 0)
    };
    (rc == 0 && len == size_of::<T>()).then_some(v)
}

/// A string `sysctlbyname` value.
pub(super) fn sysctl_str(name: &CStr) -> Option<String> {
    let mut len = 0usize;
    // SAFETY: a null buffer asks only for the size; `len` is a valid out-pointer.
    let rc = unsafe { libc::sysctlbyname(name.as_ptr(), std::ptr::null_mut(), &raw mut len, std::ptr::null_mut(), 0) };
    if rc != 0 || len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len];
    // SAFETY: `buf` is writable for `len` bytes.
    let rc =
        unsafe { libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr().cast(), &raw mut len, std::ptr::null_mut(), 0) };
    if rc != 0 {
        return None;
    }
    buf.truncate(len);
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    Some(String::from_utf8_lossy(&buf[..end]).into_owned())
}

/// Boot time, unix seconds (read once).
pub(super) fn boot_unix() -> i64 {
    static V: OnceLock<i64> = OnceLock::new();
    *V.get_or_init(|| {
        let mut mib = [libc::CTL_KERN, libc::KERN_BOOTTIME];
        // SAFETY: an all-zero `timeval` is valid.
        let mut tv: libc::timeval = unsafe { std::mem::zeroed() };
        let mut len = size_of::<libc::timeval>();
        // SAFETY: `mib` holds the 2 names passed, and `tv` is writable for `len` bytes.
        let rc =
            unsafe { libc::sysctl(mib.as_mut_ptr(), 2, (&raw mut tv).cast(), &raw mut len, std::ptr::null_mut(), 0) };
        if rc == 0 { tv.tv_sec } else { 0 }
    })
}

fn host() -> libc::host_t {
    static V: OnceLock<libc::host_t> = OnceLock::new();
    // SAFETY: no arguments. Asked once, as every call would add a reference to the port.
    *V.get_or_init(|| unsafe { libc::mach_host_self() })
}

fn page_size() -> u64 {
    // SAFETY: `sysconf` takes an integer and touches no caller memory.
    u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok().filter(|p| *p > 0).unwrap_or(16384)
}

/// Keeps per-process caches between samples.
pub struct Probe {
    procs: procs::ProcTable,
}

impl Probe {
    pub fn new() -> Self {
        Probe { procs: procs::ProcTable::new() }
    }

    pub fn sample(&mut self, want: Want) -> RawSample {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let mut out = RawSample {
            at_ms: now.as_millis() as i64,
            mono_ms: mono_ms(),
            uptime_secs: u64::try_from(now.as_secs() as i64 - boot_unix()).unwrap_or(0),
            load: load_average(),
            memory: memory(),
            ..RawSample::default()
        };
        (out.cpu_total, out.cores) = cpu();
        out.disks = iokit::disks();
        out.nets = net::interfaces();
        if want.processes {
            out.processes = self.procs.read(want);
        }
        if want.hardware {
            out.gpus = iokit::gpus();
            (out.batteries, out.sensors) = iokit::batteries();
        }
        out
    }
}

fn load_average() -> Option<[f32; 3]> {
    let mut l = [0f64; 3];
    // SAFETY: `l` is writable for the 3 elements asked for.
    let n = unsafe { libc::getloadavg(l.as_mut_ptr(), 3) };
    (n == 3).then(|| [l[0] as f32, l[1] as f32, l[2] as f32])
}

/// Total and per-CPU times. Mach counts in ticks of 1/100 s, with no I/O wait or interrupts.
fn cpu() -> (CpuTimes, Vec<CoreRaw>) {
    let mut n: libc::natural_t = 0;
    let mut info: libc::processor_info_array_t = std::ptr::null_mut();
    let mut count: libc::mach_msg_type_number_t = 0;
    // SAFETY: all out-pointers are valid. On success `info` is memory the kernel allocated in
    // this task, released below.
    let rc = unsafe {
        libc::host_processor_info(host(), libc::PROCESSOR_CPU_LOAD_INFO, &raw mut n, &raw mut info, &raw mut count)
    };
    if rc != libc::KERN_SUCCESS || info.is_null() {
        return (CpuTimes::default(), Vec::new());
    }
    // SAFETY: the kernel returned `count` integers at `info`, valid until deallocated.
    let ticks: &[libc::integer_t] = unsafe { std::slice::from_raw_parts(info, count as usize) };
    let states = libc::CPU_STATE_MAX as usize;
    let ms = |v: libc::integer_t| u64::from(v as u32) * 10;
    // Intel Macs report a clock; Apple silicon has no such sysctl.
    let mhz = sysctl_val::<u64>(c"hw.cpufrequency").map(|hz| (hz / 1_000_000) as u32);
    let mut total = CpuTimes::default();
    let cores: Vec<CoreRaw> = ticks
        .chunks_exact(states)
        .map(|c| {
            let times = CpuTimes {
                user: ms(c[libc::CPU_STATE_USER as usize]),
                nice: ms(c[libc::CPU_STATE_NICE as usize]),
                system: ms(c[libc::CPU_STATE_SYSTEM as usize]),
                idle: ms(c[libc::CPU_STATE_IDLE as usize]),
                ..CpuTimes::default()
            };
            total.user += times.user;
            total.nice += times.nice;
            total.system += times.system;
            total.idle += times.idle;
            CoreRaw { times, mhz }
        })
        .collect();
    // SAFETY: `info` and its size are exactly what `host_processor_info` returned; `ticks` is
    // not used after this.
    unsafe {
        libc::vm_deallocate(
            libc::mach_task_self(),
            info as libc::vm_address_t,
            count as usize * size_of::<libc::integer_t>(),
        );
    }
    (total, cores)
}

fn memory() -> MemoryRaw {
    let page = page_size();
    let mut m = MemoryRaw { total: sysctl_val::<u64>(c"hw.memsize").unwrap_or(0), ..MemoryRaw::default() };
    // SAFETY: an all-zero statistics struct is valid.
    let mut s: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    // SAFETY: `s` is writable for the `count` integers passed.
    let rc = unsafe { libc::host_statistics64(host(), libc::HOST_VM_INFO64, (&raw mut s).cast(), &raw mut count) };
    if rc == libc::KERN_SUCCESS {
        let bytes = |pages: u32| u64::from(pages) * page;
        let (internal, purgeable, wired, compressor, external) =
            (s.internal_page_count, s.purgeable_count, s.wire_count, s.compressor_page_count, s.external_page_count);
        // What Activity Monitor calls memory used: app memory, wired and compressed.
        let used = bytes(internal.saturating_sub(purgeable)) + bytes(wired) + bytes(compressor);
        m.available = m.total.saturating_sub(used);
        m.cached = Some(bytes(external));
        m.kernel = Some(bytes(wired));
        m.compressed = Some(bytes(compressor));
        let (pageins, pageouts) = (s.pageins, s.pageouts);
        m.page_in = Some(pageins.saturating_mul(page));
        m.page_out = Some(pageouts.saturating_mul(page));
    }
    if let Some(sw) = swap_usage() {
        m.swap_total = sw.xsu_total;
        m.swap_used = sw.xsu_used;
    }
    m
}

fn swap_usage() -> Option<libc::xsw_usage> {
    // SAFETY: an all-zero `xsw_usage` is valid.
    let mut v: libc::xsw_usage = unsafe { std::mem::zeroed() };
    let mut len = size_of::<libc::xsw_usage>();
    // SAFETY: `v` is writable for `len` bytes.
    let rc = unsafe {
        libc::sysctlbyname(c"vm.swapusage".as_ptr(), (&raw mut v).cast(), &raw mut len, std::ptr::null_mut(), 0)
    };
    (rc == 0).then_some(v)
}

pub fn system_info() -> SystemInfo {
    let int = |name: &CStr| sysctl_val::<c_int>(name).and_then(|v| u32::try_from(v).ok());
    let kb = |name: &CStr| sysctl_val::<i64>(name).and_then(|v| u32::try_from(v / 1024).ok()).filter(|v| *v > 0);
    let cpu_model = sysctl_str(c"machdep.cpu.brand_string").unwrap_or_default();
    let os = match sysctl_str(c"kern.osproductversion") {
        Some(v) => format!("macOS {v}"),
        None => "macOS".to_owned(),
    };
    let logical = int(c"hw.logicalcpu").unwrap_or(1).max(1);
    SystemInfo {
        hostname: sysctl_str(c"kern.hostname").unwrap_or_default(),
        os,
        kernel: sysctl_str(c"kern.osrelease").unwrap_or_default(),
        arch: std::env::consts::ARCH.to_owned(),
        cpu_vendor: sysctl_str(c"machdep.cpu.vendor")
            .unwrap_or_else(|| if cpu_model.starts_with("Apple") { "Apple".to_owned() } else { String::new() }),
        cpu_model,
        physical_cores: int(c"hw.physicalcpu").unwrap_or(logical).max(1),
        logical_cores: logical,
        sockets: int(c"hw.packages").unwrap_or(1).max(1),
        cpu_max_mhz: sysctl_val::<u64>(c"hw.cpufrequency_max").map(|hz| (hz / 1_000_000) as u32),
        cache_l1_kb: kb(c"hw.l1dcachesize"),
        cache_l2_kb: kb(c"hw.l2cachesize"),
        cache_l3_kb: kb(c"hw.l3cachesize"),
        memory_total: sysctl_val::<u64>(c"hw.memsize").unwrap_or(0),
        memory_kind: None,
        boot_unix: boot_unix(),
        virtualization: (sysctl_val::<c_int>(c"kern.hv_vmm_present") == Some(1)).then(|| "Virtual machine".to_owned()),
    }
}
