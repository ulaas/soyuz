//! `/proc/stat`, `meminfo`, `vmstat`, `uptime`, `loadavg`, `cpuinfo`, `diskstats`, zram and
//! `os-release`.

use std::collections::HashSet;

use super::{field, kb_value};
use crate::sysmon::{CpuTimes, MemoryRaw};

/// `/proc/stat`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Stat {
    pub total: CpuTimes,
    pub cores: Vec<CpuTimes>,
    /// The CPU number of each of `cores` (offline CPUs leave gaps).
    pub core_ids: Vec<u32>,
    pub ctx_switches: Option<u64>,
    pub interrupts: Option<u64>,
    /// Boot time, unix seconds.
    pub btime: Option<i64>,
}

/// `hz`: clock ticks per second (`_SC_CLK_TCK`).
pub(crate) fn parse_stat(s: &str, hz: u64) -> Stat {
    let mut out = Stat::default();
    for line in s.lines() {
        let Some((key, rest)) = line.split_once(' ') else { continue };
        if key == "cpu" {
            if let Some(t) = cpu_line(rest, hz) {
                out.total = t;
            }
        } else if let Some(id) = key.strip_prefix("cpu") {
            if let (Ok(id), Some(t)) = (id.parse::<u32>(), cpu_line(rest, hz)) {
                out.cores.push(t);
                out.core_ids.push(id);
            }
        } else if key == "ctxt" {
            out.ctx_switches = rest.trim().parse().ok();
        } else if key == "intr" {
            out.interrupts = rest.split_ascii_whitespace().next().and_then(|v| v.parse().ok());
        } else if key == "btime" {
            out.btime = rest.trim().parse().ok();
        }
    }
    out
}

fn cpu_line(rest: &str, hz: u64) -> Option<CpuTimes> {
    let hz = hz.max(1);
    let mut it = rest.split_ascii_whitespace().map(|v| v.parse::<u64>().ok().map(|t| t.saturating_mul(1000) / hz));
    // The four oldest columns must be there; later ones appeared with newer kernels.
    Some(CpuTimes {
        user: it.next()??,
        nice: it.next()??,
        system: it.next()??,
        idle: it.next()??,
        iowait: it.next().flatten().unwrap_or(0),
        irq: it.next().flatten().unwrap_or(0),
        softirq: it.next().flatten().unwrap_or(0),
        steal: it.next().flatten().unwrap_or(0),
    })
}

/// `/proc/meminfo`. `page_in`, `page_out` and `compressed` come from other files.
pub(crate) fn parse_meminfo(s: &str) -> MemoryRaw {
    let mut m = MemoryRaw::default();
    let (mut free, mut avail, mut swap_free) = (0u64, None, None);
    for line in s.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let val = kb_value(v);
        match k {
            "MemTotal" => m.total = val.unwrap_or(0),
            "MemFree" => free = val.unwrap_or(0),
            "MemAvailable" => avail = val,
            "Buffers" => m.buffers = val,
            "Cached" => m.cached = val,
            "Shmem" => m.shared = val,
            "Slab" => m.kernel = val,
            "SwapTotal" => m.swap_total = val.unwrap_or(0),
            "SwapFree" => swap_free = val,
            "Committed_AS" => m.commit_used = val,
            "CommitLimit" => m.commit_limit = val,
            "Zswap" => m.compressed = val,
            _ => {}
        }
    }
    // Kernels before 3.14 have no MemAvailable.
    m.available = avail
        .unwrap_or_else(|| free.saturating_add(m.buffers.unwrap_or(0)).saturating_add(m.cached.unwrap_or(0)))
        .min(if m.total > 0 { m.total } else { u64::MAX });
    m.swap_used = m.swap_total.saturating_sub(swap_free.unwrap_or(m.swap_total));
    m
}

/// `/sys/block/zram*/mm_stat`: bytes the data takes once compressed (second column).
pub(crate) fn parse_zram_mm_stat(s: &str) -> Option<u64> {
    s.split_ascii_whitespace().nth(1)?.parse().ok()
}

/// `(pgpgin, pgpgout)` of `/proc/vmstat` in bytes (the counters are KiB).
pub(crate) fn parse_vmstat(s: &str) -> (Option<u64>, Option<u64>) {
    let (mut pin, mut pout) = (None, None);
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("pgpgin ") {
            pin = v.trim().parse::<u64>().ok().and_then(|k| k.checked_mul(1024));
        } else if let Some(v) = line.strip_prefix("pgpgout ") {
            pout = v.trim().parse::<u64>().ok().and_then(|k| k.checked_mul(1024));
        }
        if pin.is_some() && pout.is_some() {
            break;
        }
    }
    (pin, pout)
}

/// Seconds since boot, from `/proc/uptime`.
pub(crate) fn parse_uptime(s: &str) -> Option<u64> {
    let v: f64 = s.split_ascii_whitespace().next()?.parse().ok()?;
    (v.is_finite() && v >= 0.0).then_some(v as u64)
}

/// `(load, total scheduling entities)` of `/proc/loadavg`.
pub(crate) fn parse_loadavg(s: &str) -> (Option<[f32; 3]>, Option<u64>) {
    let mut it = s.split_ascii_whitespace();
    let mut one = || it.next().and_then(|v| v.parse::<f32>().ok());
    let load = match (one(), one(), one()) {
        (Some(a), Some(b), Some(c)) => Some([a, b, c]),
        _ => None,
    };
    let threads = it.next().and_then(|v| v.split_once('/')).and_then(|(_, t)| t.parse().ok());
    (load, threads)
}

/// `/proc/cpuinfo`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CpuInfo {
    pub model: String,
    pub vendor: String,
    /// `cpu MHz` of each logical CPU, in order (empty where the architecture has no such line).
    pub mhz: Vec<u32>,
    pub logical: u32,
    pub physical: u32,
    pub sockets: u32,
}

pub(crate) fn parse_cpuinfo(s: &str) -> CpuInfo {
    let mut out = CpuInfo::default();
    let mut cores: HashSet<(&str, &str)> = HashSet::new();
    let mut sockets: HashSet<&str> = HashSet::new();
    let mut physical_id = "0";
    for line in s.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let (k, v) = (k.trim_end(), v.trim());
        match k {
            "processor" if v.parse::<u32>().is_ok() => out.logical += 1,
            "model name" | "cpu" | "Model" | "uarch" if out.model.is_empty() => out.model = v.to_owned(),
            "vendor_id" | "CPU implementer" | "mvendorid" if out.vendor.is_empty() => out.vendor = v.to_owned(),
            "physical id" => {
                physical_id = v;
                sockets.insert(v);
            }
            "core id" => {
                cores.insert((physical_id, v));
            }
            "cpu MHz" => {
                if let Ok(f) = v.parse::<f32>() {
                    out.mhz.push(f.max(0.0) as u32);
                }
            }
            _ => {}
        }
    }
    out.physical = if cores.is_empty() { out.logical } else { cores.len() as u32 };
    out.sockets = sockets.len().max(1) as u32;
    out
}

/// One line of `/proc/diskstats`. Sectors are always 512 bytes there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DiskStat<'a> {
    pub name: &'a str,
    pub read_ops: u64,
    pub read_sectors: u64,
    pub write_ops: u64,
    pub write_sectors: u64,
    pub in_flight: u64,
    pub io_ms: u64,
}

pub(crate) fn parse_diskstats_line(line: &str) -> Option<DiskStat<'_>> {
    let mut it = line.split_ascii_whitespace();
    let _major = it.next()?;
    let _minor = it.next()?;
    let name = it.next()?;
    let mut n = || it.next()?.parse::<u64>().ok();
    let read_ops = n()?;
    let _merged = n()?;
    let read_sectors = n()?;
    let _ms = n()?;
    let write_ops = n()?;
    let _merged = n()?;
    let write_sectors = n()?;
    let _ms = n()?;
    let in_flight = n()?;
    let io_ms = n()?;
    Some(DiskStat { name, read_ops, read_sectors, write_ops, write_sectors, in_flight, io_ms })
}

/// Block devices worth listing: not loop, ram or floppy devices. `dm-*`, `md*` and `zram*` are
/// listed, and `soyuz-sysmon` leaves them out of the totals by this same name prefix, as they count
/// the I/O of the disks below them a second time. Partitions are told apart by the reader: only
/// whole disks have a `/sys/block/<name>`.
pub(crate) fn is_listed_disk_name(name: &str) -> bool {
    !["loop", "ram", "fd"].iter().any(|p| name.starts_with(p))
}

/// `PRETTY_NAME` (or `NAME`) of `/etc/os-release`.
pub(crate) fn parse_os_release(s: &str) -> Option<String> {
    let unquote = |v: &str| v.trim().trim_matches(|c| c == '"' || c == '\'').to_owned();
    let mut name = None;
    for line in s.lines() {
        if let Some(v) = field(line, "PRETTY_NAME", '=') {
            let v = unquote(v);
            if !v.is_empty() {
                return Some(v);
            }
        } else if let Some(v) = field(line, "NAME", '=') {
            name = Some(unquote(v));
        }
    }
    name.filter(|n| !n.is_empty())
}

/// `32K`, `1M`, `2048` of a sysfs CPU cache `size` file, to KiB.
pub(crate) fn parse_cache_size(s: &str) -> Option<u32> {
    let s = s.trim();
    let (num, mult) = match s.as_bytes().last()? {
        b'K' | b'k' => (&s[..s.len() - 1], 1),
        b'M' | b'm' => (&s[..s.len() - 1], 1024),
        b'G' | b'g' => (&s[..s.len() - 1], 1024 * 1024),
        _ => (s, 1),
    };
    num.parse::<u32>().ok()?.checked_mul(mult)
}

/// A virtual machine or container, from what the reader found on the system.
pub(crate) fn detect_virtualization(
    proc_version: &str,
    dmi_product: &str,
    hypervisor_type: &str,
    dockerenv: bool,
    cpuinfo_hypervisor: bool,
) -> Option<String> {
    let ver = proc_version.to_ascii_lowercase();
    if ver.contains("microsoft") || ver.contains("wsl") {
        return Some("WSL".to_owned());
    }
    if dockerenv {
        return Some("Docker".to_owned());
    }
    let dmi = dmi_product.trim().to_ascii_lowercase();
    let from_dmi = [
        ("kvm", "KVM"),
        ("qemu", "QEMU"),
        ("standard pc", "QEMU"),
        ("vmware", "VMware"),
        ("virtualbox", "VirtualBox"),
        ("virtual machine", "Hyper-V"),
        ("hvm domu", "Xen"),
        ("parallels", "Parallels"),
    ];
    if let Some((_, n)) = from_dmi.iter().find(|(k, _)| dmi.contains(k)) {
        return Some((*n).to_owned());
    }
    match hypervisor_type.trim() {
        "" => {}
        "xen" => return Some("Xen".to_owned()),
        other => return Some(other.to_owned()),
    }
    cpuinfo_hypervisor.then(|| "Virtual machine".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat() {
        let s = "cpu  100 2 30 400 5 6 7 8 9 10\ncpu0 50 1 15 200 2 3 3 4 0 0\ncpu2 50 1 15 200 3 3 4 4\nintr 12345 1 2 3\n\
                 ctxt 999\nbtime 1700000000\nprocesses 5\nprocs_running 1\ncpuX 1 2 3 4\n";
        let st = parse_stat(s, 100);
        assert_eq!(st.total.user, 1000);
        assert_eq!(st.total.nice, 20);
        assert_eq!(st.total.idle, 4000);
        assert_eq!(st.total.steal, 80);
        assert_eq!(st.cores.len(), 2);
        assert_eq!(st.cores[1].iowait, 30);
        assert_eq!(st.core_ids, [0, 2]);
        assert_eq!(st.ctx_switches, Some(999));
        assert_eq!(st.interrupts, Some(12345));
        assert_eq!(st.btime, Some(1700000000));
    }

    #[test]
    fn stat_guest_columns_are_not_added() {
        // 8 fields (old kernels) and 10 fields: guest time is already inside user and nice.
        let eight = parse_stat("cpu  10 20 30 40 50 60 70 80
", 100).total;
        let ten = parse_stat("cpu  10 20 30 40 50 60 70 80 9000 9000
", 100).total;
        assert_eq!(eight, ten);
        assert_eq!(eight.user, 100);
        assert_eq!(eight.nice, 200);
        assert_eq!(eight.steal, 800);
    }

    #[test]
    fn stat_malformed() {
        let st = parse_stat("cpu  a b c d\ncpu0 1\nctxt x\nintr\nbtime y\n", 0);
        assert_eq!(st, Stat::default());
        // Old kernels have only four columns; hz 0 must not divide by zero.
        let st = parse_stat("cpu  1 2 3 4\n", 0);
        assert_eq!(st.total.user, 1000);
        assert_eq!(st.total.steal, 0);
    }

    #[test]
    fn meminfo() {
        let s = "MemTotal:       16000 kB\nMemFree:         1000 kB\nMemAvailable:    8000 kB\nBuffers:          100 kB\n\
                 Cached:          2000 kB\nSwapTotal:       4000 kB\nSwapFree:        3000 kB\nZswap:             7 kB\nShmem:             50 kB\n\
                 Slab:             300 kB\nCommitLimit:   12000 kB\nCommitted_AS:   9000 kB\n";
        let m = parse_meminfo(s);
        assert_eq!(m.total, 16000 * 1024);
        assert_eq!(m.available, 8000 * 1024);
        assert_eq!(m.buffers, Some(100 * 1024));
        assert_eq!(m.cached, Some(2000 * 1024));
        assert_eq!(m.shared, Some(50 * 1024));
        assert_eq!(m.kernel, Some(300 * 1024));
        assert_eq!(m.swap_used, 1000 * 1024);
        assert_eq!(m.commit_used, Some(9000 * 1024));
        assert_eq!(m.commit_limit, Some(12000 * 1024));
        assert_eq!(m.compressed, Some(7 * 1024));
    }

    #[test]
    fn meminfo_partial() {
        let m = parse_meminfo("MemTotal: 100 kB\nMemFree: 10 kB\nBuffers: 1 kB\nCached: 2 kB\n");
        assert_eq!(m.available, 13 * 1024);
        assert_eq!(m.swap_used, 0);
        assert_eq!(m.kernel, None);
        let m = parse_meminfo("junk\nMemTotal: x\nSwapFree: 5 kB\n: :\n");
        assert_eq!(m.total, 0);
        assert_eq!(m.swap_used, 0);
    }

    #[test]
    fn zram() {
        assert_eq!(parse_zram_mm_stat("  4096  1024  2048 0 2048 0 0 0 0\n"), Some(1024));
        assert_eq!(parse_zram_mm_stat("4096\n"), None);
        assert_eq!(parse_zram_mm_stat("x y"), None);
    }

    #[test]
    fn vmstat() {
        assert_eq!(parse_vmstat("nr_free 5\npgpgin 2\npgpgout 3\nmore 9\n"), (Some(2048), Some(3072)));
        assert_eq!(parse_vmstat("pgpgin x\n"), (None, None));
    }

    #[test]
    fn uptime_load() {
        assert_eq!(parse_uptime("1234.56 999.0\n"), Some(1234));
        assert_eq!(parse_uptime("-1 0"), None);
        assert_eq!(parse_uptime("NaN"), None);
        assert_eq!(parse_uptime(""), None);
        let (l, t) = parse_loadavg("0.50 1.00 2.50 3/456 789\n");
        assert_eq!(l, Some([0.5, 1.0, 2.5]));
        assert_eq!(t, Some(456));
        assert_eq!(parse_loadavg("x"), (None, None));
        assert_eq!(parse_loadavg("1 2 3 4"), (Some([1.0, 2.0, 3.0]), None));
    }

    #[test]
    fn cpuinfo_x86() {
        let s = "processor\t: 0\nvendor_id\t: GenuineIntel\nmodel name\t: Intel(R) X\ncpu MHz\t\t: 3400.123\nphysical id\t: 0\ncore id\t\t: 0\n\n\
                 processor\t: 1\nvendor_id\t: GenuineIntel\nmodel name\t: Intel(R) X\ncpu MHz\t\t: 800.0\nphysical id\t: 0\ncore id\t\t: 0\n\n\
                 processor\t: 2\nmodel name\t: Intel(R) X\ncpu MHz\t\t: junk\nphysical id\t: 1\ncore id\t\t: 0\n";
        let c = parse_cpuinfo(s);
        assert_eq!(c.model, "Intel(R) X");
        assert_eq!(c.vendor, "GenuineIntel");
        assert_eq!(c.mhz, vec![3400, 800]);
        assert_eq!((c.logical, c.physical, c.sockets), (3, 2, 2));
    }

    #[test]
    fn cpuinfo_arm_and_junk() {
        let c = parse_cpuinfo("processor : 0\nCPU implementer : 0x41\nprocessor : 1\n");
        assert_eq!(c.vendor, "0x41");
        assert_eq!((c.logical, c.physical, c.sockets), (2, 2, 1));
        assert_eq!(parse_cpuinfo("processor : x\n::\n").logical, 0);
    }

    #[test]
    fn diskstats() {
        let l = "   8       0 sda 100 5 2000 50 200 6 4000 70 3 90 120 0 0 0 0 0 0\n";
        let d = parse_diskstats_line(l).unwrap();
        assert_eq!(d.name, "sda");
        assert_eq!((d.read_ops, d.read_sectors), (100, 2000));
        assert_eq!((d.write_ops, d.write_sectors), (200, 4000));
        assert_eq!((d.in_flight, d.io_ms), (3, 90));
        assert!(parse_diskstats_line("8 0 sda 1 2").is_none());
        assert!(parse_diskstats_line("").is_none());
        assert!(parse_diskstats_line("8 0 sda a b c d e f g h i j").is_none());
    }

    #[test]
    fn disk_names() {
        for good in ["sda", "nvme0n1", "vda", "mmcblk0", "sr0", "nbd3", "dm-0", "md127", "zram0"] {
            assert!(is_listed_disk_name(good), "{good}");
        }
        for bad in ["loop0", "ram1", "fd0"] {
            assert!(!is_listed_disk_name(bad), "{bad}");
        }
    }

    #[test]
    fn os_release() {
        assert_eq!(
            parse_os_release("NAME=\"Fedora\"\nPRETTY_NAME=\"Fedora Linux 44\"\n").as_deref(),
            Some("Fedora Linux 44")
        );
        assert_eq!(parse_os_release("NAME=Arch\n").as_deref(), Some("Arch"));
        assert_eq!(parse_os_release("PRETTY_NAME=\"\"\n"), None);
        assert_eq!(parse_os_release(""), None);
    }

    #[test]
    fn cache_sizes() {
        assert_eq!(parse_cache_size("32K\n"), Some(32));
        assert_eq!(parse_cache_size("1M"), Some(1024));
        assert_eq!(parse_cache_size("512"), Some(512));
        assert_eq!(parse_cache_size(""), None);
        assert_eq!(parse_cache_size("K"), None);
        assert_eq!(parse_cache_size("é"), None);
        assert_eq!(parse_cache_size("4294967295G"), None);
    }

    #[test]
    fn virtualization() {
        let wsl = "Linux version 6.6 (microsoft@microsoft.com)";
        assert_eq!(detect_virtualization(wsl, "", "", false, true).as_deref(), Some("WSL"));
        assert_eq!(detect_virtualization("Linux", "", "", true, false).as_deref(), Some("Docker"));
        assert_eq!(detect_virtualization("Linux", "KVM\n", "", false, true).as_deref(), Some("KVM"));
        assert_eq!(detect_virtualization("Linux", "", "xen\n", false, false).as_deref(), Some("Xen"));
        assert_eq!(detect_virtualization("Linux", "X570", "", false, true).as_deref(), Some("Virtual machine"));
        assert_eq!(detect_virtualization("Linux", "X570", "", false, false), None);
    }
}
