//! The process table through libproc. Without root, another user's processes answer only the
//! short BSD info and `sysctl(KERN_PROC_PID)`; their memory and CPU stay empty.

use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::sysmon::parse::misc::{ProcArgs, parse_procargs2};
use crate::sysmon::parse::nice_to_priority;
use crate::sysmon::{ProcKey, ProcRaw, ProcState, Want};

/// How often a process's command line is read again.
const CMDLINE_REFRESH: Duration = Duration::from_secs(30);
/// `P_TRANSLATED` of `sys/proc.h`: running under Rosetta.
const P_TRANSLATED: i32 = 0x0002_0000;
/// `sizeof(struct kinfo_proc)` on 64-bit macOS, which the `libc` crate does not declare. Only
/// `kp_proc.p_starttime` (offset 0) and `kp_proc.p_flag` (offset 32) are read from it.
const KINFO_PROC_SIZE: usize = 648;

/// Mach absolute time (what `proc_taskinfo` and `rusage_info` count in) to milliseconds.
fn mach_to_ms(t: u64) -> u64 {
    static TB: OnceLock<(u64, u64)> = OnceLock::new();
    let (numer, denom) = *TB.get_or_init(|| {
        let mut info = libc::mach_timebase_info { numer: 0, denom: 0 };
        // SAFETY: `info` is a valid out-pointer.
        let rc = unsafe { libc::mach_timebase_info(&raw mut info) };
        if rc == 0 && info.denom > 0 { (u64::from(info.numer), u64::from(info.denom)) } else { (1, 1) }
    });
    (u128::from(t) * u128::from(numer) / u128::from(denom) / 1_000_000) as u64
}

/// All PIDs.
pub(super) fn all_pids() -> Vec<c_int> {
    // SAFETY: a null buffer asks for the count.
    let n = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    let Ok(n) = usize::try_from(n) else { return Vec::new() };
    // Room for processes started between the two calls.
    let mut pids = vec![0 as c_int; n + 64];
    let bytes = (pids.len() * size_of::<c_int>()) as c_int;
    // SAFETY: `pids` is writable for `bytes` bytes.
    let got = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast::<c_void>(), bytes) };
    pids.truncate(usize::try_from(got).unwrap_or(0));
    pids
}

/// A libproc struct for `flavor`, when the whole struct came back.
fn pid_info<T: Copy>(pid: c_int, flavor: c_int, arg: u64) -> Option<T> {
    let mut v = std::mem::MaybeUninit::<T>::zeroed();
    let size = size_of::<T>() as c_int;
    // SAFETY: `v` is writable for `size` bytes.
    let got = unsafe { libc::proc_pidinfo(pid, flavor, arg, v.as_mut_ptr().cast(), size) };
    // SAFETY: the kernel filled the whole struct, and every `T` used here is plain C data for
    // which all bit patterns are valid (it also started zeroed).
    (got == size).then(|| unsafe { v.assume_init() })
}

/// The process start `timeval` as one number: `ProcKey::start_raw` on macOS (microseconds).
fn raw_of(sec: u64, usec: u64) -> u64 {
    sec.saturating_mul(1_000_000).saturating_add(usec)
}

/// `(start_raw, p_flag)` from `sysctl(KERN_PROC_PID)`, open for any user's process.
pub(super) fn kinfo(pid: c_int) -> Option<(u64, i32)> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, pid];
    let mut buf = [0u8; KINFO_PROC_SIZE];
    let mut len = buf.len();
    // SAFETY: `mib` holds the 4 names passed, and `buf` is writable for `len` bytes.
    let rc =
        unsafe { libc::sysctl(mib.as_mut_ptr(), 4, buf.as_mut_ptr().cast(), &raw mut len, std::ptr::null_mut(), 0) };
    if rc != 0 || len != KINFO_PROC_SIZE {
        return None;
    }
    let sec = u64::from_ne_bytes(buf[0..8].try_into().ok()?);
    let usec = i32::from_ne_bytes(buf[8..12].try_into().ok()?);
    let flags = i32::from_ne_bytes(buf[32..36].try_into().ok()?);
    Some((raw_of(sec, u64::try_from(usec).unwrap_or(0)), flags))
}

/// A process that has exited and waits for its parent: `proc_pidinfo` no longer answers for it,
/// `sysctl` still does. `kp_proc.p_stat` is at offset 36, `p_pid` at 40, `p_comm` at 243.
fn zombie(pid: c_int) -> Option<ProcRaw> {
    let upid = u32::try_from(pid).ok()?;
    let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, pid];
    let mut buf = [0u8; KINFO_PROC_SIZE];
    let mut len = buf.len();
    // SAFETY: `mib` holds the 4 names passed, and `buf` is writable for `len` bytes.
    let rc =
        unsafe { libc::sysctl(mib.as_mut_ptr(), 4, buf.as_mut_ptr().cast(), &raw mut len, std::ptr::null_mut(), 0) };
    if rc != 0 || len != KINFO_PROC_SIZE || buf[36] != 5 {
        return None;
    }
    if i32::from_ne_bytes(buf[40..44].try_into().ok()?) != pid {
        return None;
    }
    let comm = &buf[243..260];
    let end = comm.iter().position(|b| *b == 0).unwrap_or(comm.len());
    let name = String::from_utf8_lossy(&comm[..end]).into_owned();
    let raw = kinfo(pid)?.0;
    Some(ProcRaw {
        key: ProcKey { pid: upid, start_ms: (raw / 1000) as i64, start_raw: raw },
        name: format!("[{name}]").into(),
        state: ProcState::Zombie,
        ..ProcRaw::default()
    })
}

/// A process's identity value, the same whichever call answers: `kinfo` reads the `timeval` that
/// `proc_bsdinfo` reports, and is open for every user's process.
pub(super) fn start_raw(pid: c_int) -> Option<u64> {
    kinfo(pid).map(|k| k.0)
}

pub(super) fn c_chars(s: &[c_char]) -> String {
    let bytes: Vec<u8> = s.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

pub(super) fn pid_path(pid: c_int) -> Option<PathBuf> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: `buf` is writable for the size passed.
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    let n = usize::try_from(n).ok().filter(|n| *n > 0)?;
    buf.truncate(n);
    Some(PathBuf::from(String::from_utf8_lossy(&buf).into_owned()))
}

/// Path, arguments and environment of a process (same user or root only).
pub(super) fn proc_args(pid: c_int) -> Option<ProcArgs> {
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    let mut max: c_int = 0;
    let mut len = size_of::<c_int>();
    // SAFETY: `mib` holds the 2 names passed, and `max` is writable for `len` bytes.
    let rc = unsafe { libc::sysctl(mib.as_mut_ptr(), 2, (&raw mut max).cast(), &raw mut len, std::ptr::null_mut(), 0) };
    let cap = if rc == 0 && max > 0 { max as usize } else { 1 << 20 };
    let mut buf = vec![0u8; cap];
    let mut len = buf.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    // SAFETY: `mib` holds the 3 names passed, and `buf` is writable for `len` bytes.
    let rc =
        unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast(), &raw mut len, std::ptr::null_mut(), 0) };
    if rc != 0 {
        return None;
    }
    buf.truncate(len);
    parse_procargs2(&buf)
}

/// Descriptors of a process: `(fd, PROX_FDTYPE_*)`.
pub(super) fn fds(pid: c_int) -> Option<Vec<libc::proc_fdinfo>> {
    // SAFETY: a null buffer asks for the size needed.
    let need = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    let need = usize::try_from(need).ok().filter(|n| *n > 0)?;
    let item = size_of::<libc::proc_fdinfo>();
    let cap = need / item + 8;
    // SAFETY: an all-zero `proc_fdinfo` is valid.
    let mut v: Vec<libc::proc_fdinfo> = vec![unsafe { std::mem::zeroed() }; cap];
    let bytes = (cap * item) as c_int;
    // SAFETY: `v` is writable for `bytes` bytes.
    let got = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, v.as_mut_ptr().cast(), bytes) };
    let got = usize::try_from(got).ok()?;
    v.truncate(got / item);
    Some(v)
}

fn user_name(uid: u32, users: &mut HashMap<u32, Arc<str>>) -> Arc<str> {
    users.entry(uid).or_insert_with(|| Arc::from(lookup_user(uid).unwrap_or_else(|| uid.to_string()))).clone()
}

fn lookup_user(uid: u32) -> Option<String> {
    let mut buf = vec![0u8; 4096];
    // SAFETY: an all-zero `passwd` is valid; it is only read after `getpwuid_r` filled it.
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer is valid for the call, and `buf`'s real length is passed.
    let rc = unsafe { libc::getpwuid_r(uid, &raw mut pw, buf.as_mut_ptr().cast(), buf.len(), &raw mut result) };
    if rc != 0 || result.is_null() || pw.pw_name.is_null() {
        return None;
    }
    // SAFETY: on success `pw_name` is a NUL-terminated string inside `buf`, which is alive here.
    Some(unsafe { CStr::from_ptr(pw.pw_name) }.to_string_lossy().into_owned())
}

/// `pbi_status`, refined by the number of running threads as `SRUN` covers idle programs too.
fn state(status: u32, running_threads: Option<i32>) -> ProcState {
    match status {
        2 => {
            if running_threads.is_some_and(|n| n == 0) {
                ProcState::Sleeping
            } else {
                ProcState::Running
            }
        }
        3 => ProcState::Sleeping,
        4 => ProcState::Stopped,
        5 => ProcState::Zombie,
        _ => ProcState::Unknown,
    }
}

/// Text of one process between samples, handed out as `Arc`s.
#[derive(Default)]
struct Cached {
    comm: String,
    name: Arc<str>,
    exe: Option<Arc<Path>>,
    cmdline: Option<Arc<str>>,
    arch: Option<Arc<str>>,
    refreshed: Option<Instant>,
    filled: bool,
    seen: u64,
}

pub(super) struct ProcTable {
    cache: HashMap<ProcKey, Cached>,
    users: HashMap<u32, Arc<str>>,
    tick: u64,
}

impl ProcTable {
    pub(super) fn new() -> Self {
        ProcTable { cache: HashMap::new(), users: HashMap::new(), tick: 0 }
    }

    pub(super) fn read(&mut self, want: Want) -> Vec<ProcRaw> {
        self.tick = self.tick.wrapping_add(1);
        let now = Instant::now();
        let mut out = Vec::new();
        for pid in all_pids() {
            if let Some(p) = self.read_one(pid, want, now) {
                out.push(p);
            }
        }
        let tick = self.tick;
        self.cache.retain(|_, c| c.seen == tick);
        out
    }

    fn read_one(&mut self, pid: c_int, want: Want, now: Instant) -> Option<ProcRaw> {
        let upid = u32::try_from(pid).ok()?;
        let bsd = pid_info::<libc::proc_bsdinfo>(pid, libc::PROC_PIDTBSDINFO, 0);
        let task = pid_info::<libc::proc_taskinfo>(pid, libc::PROC_PIDTASKINFO, 0);
        let mut p = ProcRaw { kernel: pid == 0, ..ProcRaw::default() };
        let uid;
        let comm;
        if let Some(b) = bsd {
            let name = c_chars(&b.pbi_name);
            comm = if name.is_empty() { c_chars(&b.pbi_comm) } else { name };
            p.parent = (b.pbi_ppid != 0).then_some(b.pbi_ppid);
            p.state = state(b.pbi_status, task.map(|t| t.pti_numrunning));
            p.nice = Some(b.pbi_nice);
            p.priority = Some(nice_to_priority(b.pbi_nice));
            let raw = raw_of(b.pbi_start_tvsec, b.pbi_start_tvusec);
            p.key = ProcKey { pid: upid, start_ms: (raw / 1000) as i64, start_raw: raw };
            uid = b.pbi_uid;
        } else {
            let Some(s) = pid_info::<libc::proc_bsdshortinfo>(pid, libc::PROC_PIDT_SHORTBSDINFO, 0) else {
                return zombie(pid);
            };
            let raw = start_raw(pid).unwrap_or(0);
            comm = c_chars(&s.pbsi_comm);
            p.parent = (s.pbsi_ppid != 0).then_some(s.pbsi_ppid);
            p.state = state(s.pbsi_status, task.map(|t| t.pti_numrunning));
            p.key = ProcKey { pid: upid, start_ms: (raw / 1000) as i64, start_raw: raw };
            uid = s.pbsi_uid;
            // SAFETY: plain integers. -1 is a valid niceness, so errno tells failure apart.
            let nice = unsafe {
                *libc::__error() = 0;
                libc::getpriority(libc::PRIO_PROCESS, pid as libc::id_t)
            };
            // SAFETY: reading this thread's errno.
            if nice != -1 || unsafe { *libc::__error() } == 0 {
                p.nice = Some(nice);
                p.priority = Some(nice_to_priority(nice));
            }
        }
        if let Some(t) = task {
            p.cpu_user_ms = mach_to_ms(t.pti_total_user);
            p.cpu_system_ms = mach_to_ms(t.pti_total_system);
            p.rss = t.pti_resident_size;
            p.virt = Some(t.pti_virtual_size);
            p.threads = u32::try_from(t.pti_threadnum).unwrap_or(0);
            p.page_faults = u64::try_from(t.pti_faults).ok();
        }
        let mut c = self.cache.remove(&p.key).unwrap_or_default();
        // After `exec` the key stays the same and the name changes: everything is stale then.
        if c.comm != comm {
            c = Cached { name: Arc::from(comm.as_str()), comm, ..Cached::default() };
        }
        c.seen = self.tick;
        if want.process_details {
            self.details(&mut p, &mut c, pid, uid, now);
        }
        p.name = c.name.clone();
        self.cache.insert(p.key, c);
        Some(p)
    }

    fn details(&mut self, p: &mut ProcRaw, c: &mut Cached, pid: c_int, uid: u32, now: Instant) {
        p.user = Some(user_name(uid, &mut self.users));
        p.elevated = Some(uid == 0);
        if !c.filled {
            let exe = pid_path(pid);
            if let Some(file) = exe.as_ref().and_then(|e| e.file_name()) {
                c.name = Arc::from(file.to_string_lossy().as_ref());
            }
            c.exe = exe.map(Arc::from);
            let translated = kinfo(pid).is_some_and(|k| k.1 & P_TRANSLATED != 0);
            let arch = match std::env::consts::ARCH {
                "aarch64" if translated => "x64",
                "aarch64" => "arm64",
                "x86_64" => "x64",
                other => other,
            };
            c.arch = Some(Arc::from(arch));
        }
        // Programs change their title after start.
        if !c.filled || c.refreshed.is_none_or(|t| now.duration_since(t) >= CMDLINE_REFRESH) {
            let line = proc_args(pid).map(|a| a.args.join(" ")).filter(|l| !l.is_empty());
            if c.cmdline.as_deref() != line.as_deref() {
                c.cmdline = line.map(Arc::from);
            }
            c.refreshed = Some(now);
            c.filled = true;
        }
        p.exe.clone_from(&c.exe);
        p.cmdline.clone_from(&c.cmdline);
        p.arch.clone_from(&c.arch);

        // SAFETY: an all-zero `rusage_info_v4` is valid.
        let mut ri: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
        // SAFETY: the flavor names exactly this struct, which is writable.
        let rc = unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V4, (&raw mut ri).cast()) };
        if rc == 0 {
            p.private = Some(ri.ri_phys_footprint);
            p.peak_rss = Some(ri.ri_lifetime_max_phys_footprint);
            p.read_bytes = Some(ri.ri_diskio_bytesread);
            p.write_bytes = Some(ri.ri_diskio_byteswritten);
        }
        p.handles = fds(pid).map(|f| f.len() as u32);
    }
}
