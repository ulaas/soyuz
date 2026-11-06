//! `/proc/<pid>/*`.

use std::collections::HashMap;

use super::{field, kb_value};
use crate::sysmon::{ModuleInfo, ProcState};

/// `PF_KTHREAD` from the kernel's `sched.h`.
pub(crate) const PF_KTHREAD: u64 = 0x0020_0000;

/// The fields of `/proc/<pid>/stat` the monitor uses. Times are clock ticks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PidStat<'a> {
    pub pid: u32,
    pub comm: &'a str,
    pub state: char,
    pub ppid: u32,
    pub tty_nr: u32,
    pub flags: u64,
    pub minflt: u64,
    pub majflt: u64,
    pub utime: u64,
    pub stime: u64,
    pub nice: i32,
    pub num_threads: u32,
    pub starttime: u64,
    pub vsize: u64,
    pub rss_pages: u64,
}

/// `comm` may hold spaces and parentheses, so it runs from the first `(` to the *last* `)`.
pub(crate) fn parse_stat(s: &str) -> Option<PidStat<'_>> {
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    if close < open {
        return None;
    }
    let pid = s[..open].trim().parse().ok()?;
    let comm = &s[open + 1..close];
    let mut it = s[close + 1..].split_ascii_whitespace();
    // Field numbers from proc(5); `it` starts at field 3.
    let state = it.next()?.chars().next()?;
    let ppid = it.next()?.parse().ok()?; // 4
    let _pgrp = it.next()?; // 5
    let _session = it.next()?; // 6
    let tty_nr = it.next()?.parse::<i64>().ok()? as u32; // 7
    let _tpgid = it.next()?; // 8
    let flags = it.next()?.parse().ok()?; // 9
    let minflt = it.next()?.parse().ok()?; // 10
    let _cminflt = it.next()?; // 11
    let majflt = it.next()?.parse().ok()?; // 12
    let _cmajflt = it.next()?; // 13
    let utime = it.next()?.parse().ok()?; // 14
    let stime = it.next()?.parse().ok()?; // 15
    let _cutime = it.next()?; // 16
    let _cstime = it.next()?; // 17
    let _priority = it.next()?; // 18
    let nice = it.next()?.parse().ok()?; // 19
    let num_threads = it.next()?.parse().ok()?; // 20
    let _itrealvalue = it.next()?; // 21
    let starttime = it.next()?.parse().ok()?; // 22
    let vsize = it.next()?.parse().ok()?; // 23
    let rss_pages = it.next()?.parse::<i64>().ok()?.max(0) as u64; // 24
    Some(PidStat {
        pid,
        comm,
        state,
        ppid,
        tty_nr,
        flags,
        minflt,
        majflt,
        utime,
        stime,
        nice,
        num_threads,
        starttime,
        vsize,
        rss_pages,
    })
}

/// A state letter of `stat`.
pub(crate) fn state_from_char(c: char) -> ProcState {
    match c {
        'R' => ProcState::Running,
        'S' => ProcState::Sleeping,
        'D' => ProcState::DiskWait,
        'T' | 't' => ProcState::Stopped,
        'Z' | 'X' | 'x' => ProcState::Zombie,
        'I' => ProcState::Idle,
        _ => ProcState::Unknown,
    }
}

/// `tty_nr` to a terminal name (`pts/3`, `tty1`, `ttyS0`); `None` without a terminal.
pub(crate) fn tty_name(tty_nr: u32) -> Option<String> {
    if tty_nr == 0 {
        return None;
    }
    let major = (tty_nr >> 8) & 0xfff;
    let minor = (tty_nr & 0xff) | ((tty_nr >> 12) & 0xf_ff00);
    match major {
        136..=143 => Some(format!("pts/{}", (major - 136) * 256 + minor)),
        4 if minor < 64 => Some(format!("tty{minor}")),
        4 => Some(format!("ttyS{}", minor - 64)),
        _ => None,
    }
}

/// The fields of `/proc/<pid>/status` the monitor uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PidStatus {
    pub euid: Option<u32>,
    pub rss: Option<u64>,
    pub rss_anon: Option<u64>,
    pub rss_file: Option<u64>,
    pub rss_shmem: Option<u64>,
    pub vm_hwm: Option<u64>,
}

pub(crate) fn parse_status(s: &str) -> PidStatus {
    let mut out = PidStatus::default();
    for line in s.lines() {
        // Cheap dispatch on the first byte: status has about 60 lines and is read per process.
        match line.as_bytes().first() {
            Some(b'U') => {
                if let Some(v) = field(line, "Uid", ':') {
                    out.euid = v.split_ascii_whitespace().nth(1).and_then(|e| e.parse().ok());
                }
            }
            Some(b'V') => {
                if let Some(v) = field(line, "VmRSS", ':') {
                    out.rss = kb_value(v);
                } else if let Some(v) = field(line, "VmHWM", ':') {
                    out.vm_hwm = kb_value(v);
                }
            }
            Some(b'R') => {
                if let Some(v) = field(line, "RssAnon", ':') {
                    out.rss_anon = kb_value(v);
                } else if let Some(v) = field(line, "RssFile", ':') {
                    out.rss_file = kb_value(v);
                } else if let Some(v) = field(line, "RssShmem", ':') {
                    out.rss_shmem = kb_value(v);
                }
            }
            _ => {}
        }
    }
    out
}

/// `/proc/<pid>/io`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PidIo {
    pub read_bytes: Option<u64>,
    pub write_bytes: Option<u64>,
    pub read_ops: Option<u64>,
    pub write_ops: Option<u64>,
}

pub(crate) fn parse_io(s: &str) -> PidIo {
    let mut out = PidIo::default();
    for line in s.lines() {
        let Some((k, v)) = line.split_once(':') else { continue };
        let v = v.trim().parse().ok();
        match k {
            "read_bytes" => out.read_bytes = v,
            "write_bytes" => out.write_bytes = v,
            "syscr" => out.read_ops = v,
            "syscw" => out.write_ops = v,
            _ => {}
        }
    }
    out
}

/// NUL-separated arguments to one line; `None` when empty (kernel threads, zombies).
pub(crate) fn parse_cmdline(b: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(b);
    let joined = text.trim_end_matches('\0').replace('\0', " ");
    let joined = joined.trim();
    (!joined.is_empty()).then(|| joined.to_owned())
}

/// The command line to show: zombies and kernel threads have none, and read `[comm]` as in `ps`.
pub(crate) fn cmdline_or_comm(b: &[u8], comm: &str) -> String {
    parse_cmdline(b).unwrap_or_else(|| format!("[{comm}]"))
}

/// `/proc/<pid>/cgroup`: the unified hierarchy's path, or else the first controller's.
pub(crate) fn parse_cgroup(s: &str) -> Option<String> {
    let mut first = None;
    for line in s.lines() {
        let mut it = line.splitn(3, ':');
        let (Some(id), Some(_), Some(path)) = (it.next(), it.next(), it.next()) else { continue };
        if id == "0" {
            return Some(path.to_owned());
        }
        first.get_or_insert(path);
    }
    first.map(str::to_owned)
}

/// `/etc/passwd`: uid to user name.
pub(crate) fn parse_passwd(s: &str) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    for line in s.lines() {
        if line.starts_with('#') {
            continue;
        }
        let mut it = line.split(':');
        let (Some(name), Some(_), Some(uid)) = (it.next(), it.next(), it.next()) else { continue };
        if let (false, Ok(uid)) = (name.is_empty(), uid.parse()) {
            // The first entry of a uid wins, as in NSS lookups.
            out.entry(uid).or_insert_with(|| name.to_owned());
        }
    }
    out
}

/// The `readlink` of `/proc/<pid>/exe` without a trailing ` (deleted)`.
pub(crate) fn clean_exe_link(link: &str) -> &str {
    link.strip_suffix(" (deleted)").unwrap_or(link)
}

/// `comm` is cut at 15 bytes: use the executable's file name when `comm` is its prefix.
pub(crate) fn full_name<'a>(comm: &'a str, exe_file: Option<&'a str>) -> &'a str {
    match exe_file {
        Some(f) if comm.len() >= 15 && f.starts_with(comm) => f,
        _ => comm,
    }
}

/// The inode of a `socket:[12345]` link.
pub(crate) fn socket_inode(link: &str) -> Option<u64> {
    link.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok()
}

/// `e_machine` of an ELF header's first bytes, on the vocabulary of `ProcRaw::arch`.
pub(crate) fn elf_arch(head: &[u8]) -> Option<&'static str> {
    if head.len() < 20 || &head[..4] != b"\x7fELF" {
        return None;
    }
    let raw = [head[18], head[19]];
    let machine = match head[5] {
        1 => u16::from_le_bytes(raw),
        2 => u16::from_be_bytes(raw),
        _ => return None,
    };
    match machine {
        3 => Some("x86"),
        40 => Some("arm"),
        62 => Some("x64"),
        183 => Some("arm64"),
        243 => Some("riscv"),
        _ => None,
    }
}

/// `/proc/<pid>/maps`: unique mapped files in first-seen order, with the total mapped size.
pub(crate) fn parse_maps(s: &str) -> Vec<ModuleInfo> {
    let mut out: Vec<ModuleInfo> = Vec::new();
    let mut index: HashMap<&str, usize> = HashMap::new();
    for line in s.lines() {
        let Some((range, path)) = maps_line(line) else { continue };
        let Some((start, end)) = range.split_once('-') else { continue };
        let (Ok(start), Ok(end)) = (u64::from_str_radix(start, 16), u64::from_str_radix(end, 16)) else { continue };
        let size = end.saturating_sub(start);
        match index.get(path) {
            Some(&i) => out[i].size += size,
            None => {
                index.insert(path, out.len());
                out.push(ModuleInfo { path: path.to_owned(), size, base: start });
            }
        }
    }
    out
}

/// `(range, path)` of a maps line naming a real file. A path may hold spaces.
fn maps_line(line: &str) -> Option<(&str, &str)> {
    let (range, mut rest) = line.split_once(' ')?;
    // Skip perms, offset, dev, inode.
    for _ in 0..4 {
        rest = rest.trim_start();
        rest = rest.split_once(' ')?.1;
    }
    let path = rest.trim();
    (path.starts_with('/') && !path.starts_with("/dev/")).then_some((range, path))
}

/// `/proc/<pid>/environ`: sorted `NAME=value` pairs.
pub(crate) fn parse_environ(b: &[u8]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = b
        .split(|c| *c == 0)
        .filter_map(|kv| {
            let kv = String::from_utf8_lossy(kv);
            let (k, v) = kv.split_once('=')?;
            (!k.is_empty()).then(|| (k.to_owned(), v.to_owned()))
        })
        .collect();
    out.sort();
    out
}

/// One DRM client from `/proc/<pid>/fdinfo/<fd>` (the kernel's drm-usage-stats format).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DrmClient {
    pub pdev: String,
    pub client_id: u64,
    /// Engine name and busy time.
    pub engines: Vec<(String, EngineUse)>,
    /// Bytes resident across memory regions (`drm-resident-*`, else `drm-memory-*`).
    pub memory: u64,
}

/// An engine's busy counter: nanoseconds, or cycles against total cycles (Intel `xe`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EngineUse {
    Nanos(u64),
    Cycles { busy: u64, total: u64 },
}

/// `None` for an fd that is not a DRM client.
pub(crate) fn parse_drm_fdinfo(s: &str) -> Option<DrmClient> {
    let mut out = DrmClient::default();
    let mut has_driver = false;
    let (mut resident, mut memory) = (0u64, 0u64);
    let mut cycles: Vec<(&str, u64, u64)> = Vec::new();
    for line in s.lines() {
        let Some(rest) = line.strip_prefix("drm-") else { continue };
        let Some((k, v)) = rest.split_once(':') else { continue };
        let v = v.trim();
        let number = || v.split_ascii_whitespace().next().and_then(|n| n.parse::<u64>().ok());
        if k == "driver" {
            has_driver = true;
        } else if k == "pdev" {
            out.pdev = v.to_owned();
        } else if k == "client-id" {
            out.client_id = number().unwrap_or(0);
        } else if let Some(engine) = k.strip_prefix("engine-") {
            if let (false, Some(ns)) = (engine.starts_with("capacity-"), number()) {
                out.engines.push((engine.to_owned(), EngineUse::Nanos(ns)));
            }
        } else if let Some(engine) = k.strip_prefix("total-cycles-") {
            let n = number().unwrap_or(0);
            match cycles.iter_mut().find(|c| c.0 == engine) {
                Some(c) => c.2 = n,
                None => cycles.push((engine, 0, n)),
            }
        } else if let Some(engine) = k.strip_prefix("cycles-") {
            let n = number().unwrap_or(0);
            match cycles.iter_mut().find(|c| c.0 == engine) {
                Some(c) => c.1 = n,
                None => cycles.push((engine, n, 0)),
            }
        } else if k.starts_with("resident-") {
            resident = resident.saturating_add(drm_size(v));
        } else if k.starts_with("memory-") {
            memory = memory.saturating_add(drm_size(v));
        }
    }
    if !has_driver {
        return None;
    }
    for (engine, busy, total) in cycles {
        out.engines.push((engine.to_owned(), EngineUse::Cycles { busy, total }));
    }
    out.memory = if resident > 0 { resident } else { memory };
    Some(out)
}

/// `123 KiB` as drm-usage-stats words sizes (bare numbers are bytes).
fn drm_size(v: &str) -> u64 {
    let mut it = v.split_ascii_whitespace();
    let n: u64 = it.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    let shift = match it.next() {
        Some("KiB") => 10,
        Some("MiB") => 20,
        Some("GiB") => 30,
        _ => 0,
    };
    n.saturating_mul(1 << shift)
}

/// An engine's load between two counters, percent, over `elapsed_ns` of wall time.
pub(crate) fn engine_percent(prev: EngineUse, now: EngineUse, elapsed_ns: u64) -> Option<f32> {
    match (prev, now) {
        (EngineUse::Nanos(a), EngineUse::Nanos(b)) => {
            (elapsed_ns > 0).then(|| (b.saturating_sub(a) as f64 / elapsed_ns as f64 * 100.0).min(100.0) as f32)
        }
        (EngineUse::Cycles { busy: a, total: ta }, EngineUse::Cycles { busy: b, total: tb }) => {
            let total = tb.saturating_sub(ta);
            (total > 0).then(|| (b.saturating_sub(a) as f64 / total as f64 * 100.0).min(100.0) as f32)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "1234 (Web Content) S 1 1234 1234 34816 1234 4194304 500 0 7 0 120 30 0 0 20 5 12 0 98765 \
                        2147483648 4096 18446744073709551615 1 1 0 0 0 0 0 0 0 0 0 0 17 3 0 0 0 0 0";

    #[test]
    fn stat_with_plain_name() {
        let s = parse_stat(STAT).unwrap();
        assert_eq!(s.pid, 1234);
        assert_eq!(s.comm, "Web Content");
        assert_eq!(s.state, 'S');
        assert_eq!(s.ppid, 1);
        assert_eq!(s.tty_nr, 34816);
        assert_eq!(s.flags, 4194304);
        assert_eq!((s.minflt, s.majflt), (500, 7));
        assert_eq!((s.utime, s.stime), (120, 30));
        assert_eq!(s.nice, 5);
        assert_eq!(s.num_threads, 12);
        assert_eq!(s.starttime, 98765);
        assert_eq!(s.vsize, 2147483648);
        assert_eq!(s.rss_pages, 4096);
    }

    #[test]
    fn stat_with_hostile_names() {
        for comm in ["a) R 99 (b", "(((", ")))", "x y z", ") (", "", "1 2 3 4"] {
            let line = format!("7 ({comm}) R 1 1 1 0 -1 0 0 0 0 0 1 2 0 0 20 -5 1 0 5 100 2 0");
            let s = parse_stat(&line).unwrap_or_else(|| panic!("failed on {comm:?}"));
            assert_eq!(s.comm, comm);
            assert_eq!(s.state, 'R');
            assert_eq!(s.nice, -5);
            assert_eq!(s.rss_pages, 2);
        }
    }

    #[test]
    fn stat_malformed_is_none() {
        for bad in ["", "7", "7 (x", "7 x)", ") 7 (x", "x (y) R", "7 (x) R 1", "7 (x) R a b c", "7 (x)", "7 (é"] {
            assert!(parse_stat(bad).is_none(), "{bad:?}");
        }
        // Every truncation of a real line and of odd bytes: no panic.
        for i in 0..STAT.len() {
            let _ = parse_stat(&STAT[..i]);
        }
        let _ = parse_stat("\u{0}\u{1}(\u{2}é)\u{3}");
    }

    #[test]
    fn states() {
        assert_eq!(state_from_char('R'), ProcState::Running);
        assert_eq!(state_from_char('D'), ProcState::DiskWait);
        assert_eq!(state_from_char('t'), ProcState::Stopped);
        assert_eq!(state_from_char('Z'), ProcState::Zombie);
        assert_eq!(state_from_char('I'), ProcState::Idle);
        assert_eq!(state_from_char('?'), ProcState::Unknown);
    }

    #[test]
    fn terminals() {
        assert_eq!(tty_name(0), None);
        assert_eq!(tty_name(34816).as_deref(), Some("pts/0"));
        assert_eq!(tty_name(34819).as_deref(), Some("pts/3"));
        assert_eq!(tty_name(1025).as_deref(), Some("tty1"));
        assert_eq!(tty_name(1088).as_deref(), Some("ttyS0"));
        assert_eq!(tty_name(0xffff_ffff), None);
    }

    #[test]
    fn status() {
        let s = "Name:\tx\nUmask:\t0022\nUid:\t1000\t1001\t1000\t1000\nVmHWM:\t 100 kB\nVmRSS:\t 80 kB\nRssAnon:\t 50 kB\n\
                 RssFile:\t 20 kB\nRssShmem:\t 10 kB\n";
        let st = parse_status(s);
        assert_eq!(st.euid, Some(1001));
        assert_eq!(st.vm_hwm, Some(100 * 1024));
        assert_eq!(st.rss, Some(80 * 1024));
        assert_eq!(st.rss_anon, Some(50 * 1024));
        assert_eq!(st.rss_file, Some(20 * 1024));
        assert_eq!(st.rss_shmem, Some(10 * 1024));
        // A kernel thread has no Vm lines.
        assert_eq!(parse_status("Name:\tkthreadd\nUid:\t0\t0\t0\t0\n").rss, None);
        assert_eq!(parse_status("Uid:\nVmRSS:\tjunk\nRss").euid, None);
        assert_eq!(parse_status(""), PidStatus::default());
    }

    #[test]
    fn io() {
        let s =
            "rchar: 10\nwchar: 20\nsyscr: 3\nsyscw: 4\nread_bytes: 4096\nwrite_bytes: 8192\ncancelled_write_bytes: 0\n";
        let i = parse_io(s);
        assert_eq!(i.read_bytes, Some(4096));
        assert_eq!(i.write_bytes, Some(8192));
        assert_eq!(i.read_ops, Some(3));
        assert_eq!(i.write_ops, Some(4));
        assert_eq!(parse_io("read_bytes: x\nwhat"), PidIo::default());
    }

    #[test]
    fn cmdline() {
        assert_eq!(parse_cmdline(b"/bin/sh\0-c\0echo hi\0").as_deref(), Some("/bin/sh -c echo hi"));
        assert_eq!(parse_cmdline(b"").as_deref(), None);
        assert_eq!(parse_cmdline(b"\0\0").as_deref(), None);
        assert_eq!(parse_cmdline(b"a\xffb\0").as_deref(), Some("a\u{fffd}b"));
    }

    #[test]
    fn zombie() {
        // A zombie: state Z, no memory, an empty command line, and a comm with a closing paren.
        let s = parse_stat("42 (a) b) Z 1 42 42 0 -1 4228108 1 0 0 0 3 4 0 0 20 0 1 0 77 0 0 0").unwrap();
        assert_eq!(s.comm, "a) b");
        assert_eq!(state_from_char(s.state), ProcState::Zombie);
        assert_eq!((s.vsize, s.rss_pages), (0, 0));
        assert_eq!(cmdline_or_comm(b"", s.comm), "[a) b]");
        assert_eq!(cmdline_or_comm(b"\0", "x"), "[x]");
        assert_eq!(cmdline_or_comm(b"/bin/x\0-y\0", "x"), "/bin/x -y");
    }

    #[test]
    fn cgroups() {
        assert_eq!(parse_cgroup("0::/user.slice/session-1.scope\n").as_deref(), Some("/user.slice/session-1.scope"));
        assert_eq!(parse_cgroup("12:cpu:/a\n0::/b\n").as_deref(), Some("/b"));
        assert_eq!(parse_cgroup("12:cpu:/a\n11:mem:/c\n").as_deref(), Some("/a"));
        assert_eq!(parse_cgroup("junk\n").as_deref(), None);
    }

    #[test]
    fn passwd() {
        let m = parse_passwd(
            "#c\nroot:x:0:0:root:/root:/bin/sh\nalias:x:0:0::/:\nme:x:1000:1000::/h:/bin/sh\nbad\nx:y:z\n:x:5:\n",
        );
        assert_eq!(m.get(&0).map(String::as_str), Some("root"));
        assert_eq!(m.get(&1000).map(String::as_str), Some("me"));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn names_and_links() {
        assert_eq!(clean_exe_link("/a b (deleted)"), "/a b");
        assert_eq!(clean_exe_link("/a"), "/a");
        assert_eq!(full_name("gnome-shell-cal", Some("gnome-shell-calendar-server")), "gnome-shell-calendar-server");
        assert_eq!(full_name("bash", Some("dash")), "bash");
        assert_eq!(full_name("Web Content", Some("firefox")), "Web Content");
        assert_eq!(full_name("thread-pool-wor", Some("soyuz")), "thread-pool-wor");
        assert_eq!(socket_inode("socket:[42]"), Some(42));
        assert_eq!(socket_inode("pipe:[42]"), None);
        assert_eq!(socket_inode("socket:[x]"), None);
    }

    #[test]
    fn elf() {
        let mut h = [0u8; 20];
        h[..4].copy_from_slice(b"\x7fELF");
        h[5] = 1;
        h[18] = 62;
        assert_eq!(elf_arch(&h), Some("x64"));
        h[18] = 183;
        assert_eq!(elf_arch(&h), Some("arm64"));
        h[5] = 2;
        h[18] = 0;
        h[19] = 3;
        assert_eq!(elf_arch(&h), Some("x86"));
        assert_eq!(elf_arch(b"#!/bin/sh"), None);
        h[0] = 0;
        assert_eq!(elf_arch(&h), None);
    }

    #[test]
    fn maps() {
        let s = "00400000-00401000 r-xp 00000000 08:01 123    /usr/bin/my app\n\
                 00401000-00403000 r--p 00001000 08:01 123    /usr/bin/my app\n\
                 7f0000000000-7f0000001000 r-xp 00000000 08:01 7   /usr/lib/libc.so.6\n\
                 7f0000002000-7f0000003000 rw-p 00000000 00:00 0 \n\
                 7ffc000-7ffd000 rw-p 00000000 00:00 0   [stack]\n\
                 80000-81000 rw-s 00000000 00:05 9   /dev/zero (deleted)\n\
                 zz-yy r-xp 0 0 0 /bad\nshort\n";
        let m = parse_maps(s);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].path, "/usr/bin/my app");
        assert_eq!(m[0].size, 0x3000);
        assert_eq!(m[0].base, 0x400000);
        assert_eq!(m[1].path, "/usr/lib/libc.so.6");
        assert_eq!(parse_maps(""), Vec::new());
    }

    #[test]
    fn environ() {
        let e = parse_environ(b"B=2\0A=1=x\0\0noequals\0=v\0C=\xff\0");
        assert_eq!(e[0], ("A".to_owned(), "1=x".to_owned()));
        assert_eq!(e[1], ("B".to_owned(), "2".to_owned()));
        assert_eq!(e[2], ("C".to_owned(), "\u{fffd}".to_owned()));
        assert_eq!(e.len(), 3);
    }

    #[test]
    fn drm_amdgpu() {
        let s = "pos:\t0\nflags:\t02100002\nmnt_id:\t26\ndrm-driver:\tamdgpu\ndrm-pdev:\t0000:03:00.0\ndrm-client-id:\t42\n\
                 drm-memory-vram:\t1024 KiB\ndrm-memory-gtt:\t1 MiB\ndrm-engine-gfx:\t1000 ns\ndrm-engine-capacity-gfx:\t2\n\
                 drm-engine-compute:\t5 ns\n";
        let c = parse_drm_fdinfo(s).unwrap();
        assert_eq!(c.pdev, "0000:03:00.0");
        assert_eq!(c.client_id, 42);
        assert_eq!(c.memory, 2 << 20);
        assert_eq!(c.engines[0], ("gfx".to_owned(), EngineUse::Nanos(1000)));
        assert_eq!(c.engines[1], ("compute".to_owned(), EngineUse::Nanos(5)));
        assert_eq!(c.engines.len(), 2);
    }

    #[test]
    fn drm_xe_cycles_and_resident() {
        let s = "drm-driver: xe\ndrm-client-id: 7\ndrm-pdev: 0000:00:02.0\ndrm-total-cycles-rcs: 2000\ndrm-cycles-rcs: 500\n\
                 drm-resident-vram0: 2048\ndrm-memory-vram0: 4096\n";
        let c = parse_drm_fdinfo(s).unwrap();
        assert_eq!(c.memory, 2048);
        assert_eq!(c.engines, vec![("rcs".to_owned(), EngineUse::Cycles { busy: 500, total: 2000 })]);
    }

    #[test]
    fn drm_not_a_client() {
        assert_eq!(parse_drm_fdinfo("pos:\t0\nflags:\t0\n"), None);
        assert_eq!(parse_drm_fdinfo(""), None);
        let c = parse_drm_fdinfo("drm-driver: x\ndrm-engine-gfx: x ns\ndrm-memory-a: y\ndrm-\n").unwrap();
        assert!(c.engines.is_empty());
        assert_eq!(c.memory, 0);
    }

    #[test]
    fn engine_load() {
        let p = engine_percent(EngineUse::Nanos(0), EngineUse::Nanos(500), 1000).unwrap();
        assert!((p - 50.0).abs() < 0.01);
        assert_eq!(engine_percent(EngineUse::Nanos(9), EngineUse::Nanos(1), 1000), Some(0.0));
        assert_eq!(engine_percent(EngineUse::Nanos(0), EngineUse::Nanos(5000), 1000), Some(100.0));
        assert_eq!(engine_percent(EngineUse::Nanos(0), EngineUse::Nanos(1), 0), None);
        let a = EngineUse::Cycles { busy: 100, total: 1000 };
        let b = EngineUse::Cycles { busy: 300, total: 2000 };
        assert!((engine_percent(a, b, 1).unwrap() - 20.0).abs() < 0.01);
        assert_eq!(engine_percent(a, a, 1), None);
        assert_eq!(engine_percent(a, EngineUse::Nanos(1), 1), None);
    }
}
