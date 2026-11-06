//! What does not change while running: registry, logical processor table, CPU identification.

use std::mem::size_of;
use std::time::{SystemTime, UNIX_EPOCH};

use windows_sys::Win32::System::SystemInformation::{
    ComputerNameDnsHostname, GetComputerNameExW, GetLogicalProcessorInformationEx, GetNativeSystemInfo, GetTickCount64,
    GlobalMemoryStatusEx, MEMORYSTATUSEX, RelationAll, SYSTEM_INFO,
};

use super::super::registry::{self, Hive};
use super::{read_pod, u32_at};
use crate::sysmon::SystemInfo;

const CURRENT_VERSION: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
const CPU_KEY: &str = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";

// SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX: Relationship (u32), Size (u32), then the union.
const RELATION_CORE: u32 = 0;
const RELATION_CACHE: u32 = 2;
const RELATION_PACKAGE: u32 = 3;
/// PROCESSOR_RELATIONSHIP: `GroupCount` (u16) at 22 of the union, `GROUP_AFFINITY[]` at 24.
const PROC_GROUP_COUNT: usize = 8 + 22;
const PROC_GROUPS: usize = 8 + 24;
/// GROUP_AFFINITY: an 8-byte mask, group and reserved words (16 bytes on 64-bit).
const GROUP_AFFINITY_SIZE: usize = size_of::<usize>() + 8;
/// CACHE_RELATIONSHIP: `Level` (u8) at 0, `CacheSize` (u32) at 4 of the union.
const CACHE_LEVEL: usize = 8;
const CACHE_SIZE: usize = 8 + 4;

fn hostname() -> String {
    let mut buf = [0u16; 256];
    let mut len = buf.len() as u32;
    // SAFETY: output buffer of `len` characters.
    let ok = unsafe { GetComputerNameExW(ComputerNameDnsHostname, buf.as_mut_ptr(), &mut len) };
    if ok == 0 { String::new() } else { String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())]) }
}

fn arch() -> &'static str {
    // SAFETY: plain struct of integers and pointers; zero is valid as output space.
    let mut si: SYSTEM_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: `si` is writable.
    unsafe { GetNativeSystemInfo(&mut si) };
    // SAFETY: the union's struct form is what the call fills in.
    match unsafe { si.Anonymous.Anonymous.wProcessorArchitecture } {
        9 => "x64",
        12 => "arm64",
        0 => "x86",
        5 => "arm",
        _ => "",
    }
}

/// `Windows 11 Pro 26300`. The registry's product name still says 10 on Windows 11.
fn os_name(product: &str, build: u32) -> String {
    let product = if build >= 22_000 { product.replacen("Windows 10", "Windows 11", 1) } else { product.to_string() };
    if build > 0 { format!("{product} {build}") } else { product }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Topology {
    physical: u32,
    logical: u32,
    sockets: u32,
    l1: u32,
    l2: u32,
    l3: u32,
}

/// Parse the logical processor records; a bad record size ends the walk.
fn parse_topology(b: &[u8]) -> Topology {
    let mut t = Topology::default();
    let mut off = 0usize;
    while let (Some(kind), Some(size)) = (u32_at(b, off), u32_at(b, off + 4)) {
        let size = size as usize;
        if size < 8 || off.saturating_add(size) > b.len() {
            break;
        }
        let rec = &b[off..off + size];
        match kind {
            RELATION_CORE => {
                t.physical += 1;
                let groups = usize::from(read_pod::<u16>(rec, PROC_GROUP_COUNT).unwrap_or(0));
                for g in 0..groups {
                    if let Some(mask) = read_pod::<usize>(rec, PROC_GROUPS + g * GROUP_AFFINITY_SIZE) {
                        t.logical += mask.count_ones();
                    }
                }
            }
            RELATION_PACKAGE => t.sockets += 1,
            RELATION_CACHE => {
                let kb = u32_at(rec, CACHE_SIZE).unwrap_or(0) / 1024;
                match rec.get(CACHE_LEVEL) {
                    Some(1) => t.l1 += kb,
                    Some(2) => t.l2 += kb,
                    Some(3) => t.l3 += kb,
                    _ => {}
                }
            }
            _ => {}
        }
        off += size;
    }
    t
}

fn topology() -> Topology {
    let mut buf = vec![0u64; (16 << 10) / 8];
    for _ in 0..4 {
        let mut len = (buf.len() * 8) as u32;
        // SAFETY: `buf` is 8-aligned and writable for `len` bytes.
        let ok = unsafe { GetLogicalProcessorInformationEx(RelationAll, buf.as_mut_ptr().cast(), &mut len) };
        if ok != 0 {
            return parse_topology(super::as_bytes(&buf, len as usize));
        }
        if len as usize <= buf.len() * 8 || len as usize > 16 << 20 {
            break;
        }
        buf.resize((len as usize).div_ceil(8), 0);
    }
    Topology::default()
}

/// The hypervisor this system runs under. A Windows host with Hyper-V or virtualization-based
/// security also sees the hypervisor bit; its root partition may create partitions, a guest
/// may not.
#[cfg(target_arch = "x86_64")]
fn virtualization() -> Option<String> {
    use std::arch::x86_64::__cpuid;
    if __cpuid(1).ecx & (1 << 31) == 0 {
        return None;
    }
    let leaf = __cpuid(0x4000_0000);
    let mut vendor = [0u8; 12];
    vendor[0..4].copy_from_slice(&leaf.ebx.to_le_bytes());
    vendor[4..8].copy_from_slice(&leaf.ecx.to_le_bytes());
    vendor[8..12].copy_from_slice(&leaf.edx.to_le_bytes());
    let root_partition = __cpuid(0x4000_0003).ebx & 1 != 0;
    hypervisor_name(&vendor, root_partition)
}

#[cfg(not(target_arch = "x86_64"))]
fn virtualization() -> Option<String> {
    None
}

#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
fn hypervisor_name(vendor: &[u8; 12], root_partition: bool) -> Option<String> {
    let text = String::from_utf8_lossy(vendor);
    let text = text.trim_matches(char::from(0));
    Some(
        match text {
            "Microsoft Hv" if root_partition => return None,
            "Microsoft Hv" => "Hyper-V",
            "KVMKVMKVM" => "KVM",
            "VMwareVMware" => "VMware",
            "VBoxVBoxVBox" => "VirtualBox",
            "XenVMMXenVMM" => "Xen",
            "TCGTCGTCGTCG" => "QEMU",
            "prl hyperv" => "Parallels",
            other => other,
        }
        .to_string(),
    )
}

/// See `crate::sysmon::SystemInfo`.
pub fn system_info() -> SystemInfo {
    let build = registry::get_string(Hive::LocalMachine, CURRENT_VERSION, Some("CurrentBuildNumber"))
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let product = registry::get_string(Hive::LocalMachine, CURRENT_VERSION, Some("ProductName"))
        .unwrap_or_else(|| "Windows".into());
    let major = registry::get_dword(Hive::LocalMachine, CURRENT_VERSION, "CurrentMajorVersionNumber").unwrap_or(10);
    let minor = registry::get_dword(Hive::LocalMachine, CURRENT_VERSION, "CurrentMinorVersionNumber").unwrap_or(0);
    let ubr = registry::get_dword(Hive::LocalMachine, CURRENT_VERSION, "UBR").unwrap_or(0);

    let mut status = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    // SAFETY: `status` is writable and its length field is set.
    let memory_total = if unsafe { GlobalMemoryStatusEx(&mut status) } != 0 { status.ullTotalPhys } else { 0 };

    // SAFETY: no arguments.
    let uptime = unsafe { GetTickCount64() } / 1000;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let t = topology();
    let opt_kb = |v: u32| (v > 0).then_some(v);
    SystemInfo {
        hostname: hostname(),
        os: os_name(&product, build),
        kernel: format!("{major}.{minor}.{build}.{ubr}"),
        arch: arch().to_string(),
        cpu_model: registry::get_string(Hive::LocalMachine, CPU_KEY, Some("ProcessorNameString"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
        cpu_vendor: registry::get_string(Hive::LocalMachine, CPU_KEY, Some("VendorIdentifier"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
        physical_cores: t.physical,
        logical_cores: t.logical,
        sockets: t.sockets,
        cpu_max_mhz: registry::get_dword(Hive::LocalMachine, CPU_KEY, "~MHz").filter(|&m| m > 0),
        cache_l1_kb: opt_kb(t.l1),
        cache_l2_kb: opt_kb(t.l2),
        cache_l3_kb: opt_kb(t.l3),
        memory_total,
        memory_kind: None,
        boot_unix: now.saturating_sub(uptime) as i64,
        virtualization: virtualization(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_names() {
        assert_eq!(os_name("Windows 10 Pro", 26300), "Windows 11 Pro 26300");
        assert_eq!(os_name("Windows 10 Pro", 19045), "Windows 10 Pro 19045");
        assert_eq!(os_name("Windows", 0), "Windows");
    }

    #[test]
    fn hypervisors() {
        assert_eq!(hypervisor_name(b"Microsoft Hv", true), None);
        assert_eq!(hypervisor_name(b"Microsoft Hv", false).as_deref(), Some("Hyper-V"));
        assert_eq!(hypervisor_name(b"KVMKVMKVM\0\0\0", false).as_deref(), Some("KVM"));
    }

    fn core_record(mask: usize) -> Vec<u8> {
        let mut r = vec![0u8; PROC_GROUPS + GROUP_AFFINITY_SIZE];
        let size = r.len() as u32;
        r[0..4].copy_from_slice(&RELATION_CORE.to_le_bytes());
        r[4..8].copy_from_slice(&size.to_le_bytes());
        r[PROC_GROUP_COUNT] = 1;
        r[PROC_GROUPS..PROC_GROUPS + size_of::<usize>()].copy_from_slice(&mask.to_le_bytes());
        r
    }

    fn cache_record(level: u8, kb: u32) -> Vec<u8> {
        let mut r = vec![0u8; 48];
        r[0..4].copy_from_slice(&RELATION_CACHE.to_le_bytes());
        r[4..8].copy_from_slice(&48u32.to_le_bytes());
        r[CACHE_LEVEL] = level;
        r[CACHE_SIZE..CACHE_SIZE + 4].copy_from_slice(&(kb * 1024).to_le_bytes());
        r
    }

    #[test]
    fn topology_records() {
        let mut b = core_record(0b11);
        b.extend(core_record(0b1100));
        b.extend(cache_record(2, 512));
        b.extend(cache_record(3, 16_384));
        let t = parse_topology(&b);
        assert_eq!((t.physical, t.logical, t.l1, t.l2, t.l3), (2, 4, 0, 512, 16_384));
    }

    #[test]
    fn topology_survives_bad_sizes() {
        let mut b = core_record(1);
        b[4] = 2; // Size below the header.
        assert_eq!(parse_topology(&b), Topology::default());
        let mut b = core_record(1);
        b[4] = 255; // Size past the end.
        assert_eq!(parse_topology(&b), Topology::default());
        assert_eq!(parse_topology(&[]), Topology::default());
    }
}
