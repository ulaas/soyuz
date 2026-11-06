//! The process table: `/proc/<pid>/{stat,status,io,cmdline,exe,cgroup}`, plus per-process GPU
//! use from `fdinfo`. Text is cached per [`ProcKey`] and handed out as `Arc`s, and buffers are
//! reused, so a sample of hundreds of processes is reference counts plus counters. Kernel
//! threads skip everything but `stat`.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{clk_tck, nvml, page_size, read_into, start_ms_of};
use crate::sysmon::parse::nice_to_priority;
use crate::sysmon::parse::pid::{self as ppid, DrmClient, EngineUse, PF_KTHREAD};
use crate::sysmon::{ProcKey, ProcRaw, Want};

/// Programs change their title (`setproctitle`) and cgroup after start.
const TEXT_REFRESH: Duration = Duration::from_secs(30);
/// Finding DRM descriptors needs a readlink per fd: each process every this many GPU passes.
const DRM_RESCAN_PASSES: u64 = 4;

/// What is known about one process between samples.
#[derive(Default)]
struct Cached {
    comm: Arc<str>,
    /// `comm`, or the executable's file name once details are read.
    name: Arc<str>,
    exe: Option<Arc<Path>>,
    cmdline: Option<Arc<str>>,
    cgroup: Option<Arc<str>>,
    arch: Option<Arc<str>>,
    tty_nr: u32,
    session: Option<Arc<str>>,
    refreshed: Option<Instant>,
    filled: bool,
    seen: u64,
    /// DRM file descriptors, rescanned in turns.
    drm_fds: Vec<u32>,
    drm_scanned: bool,
    /// Previous counters per client and engine.
    gpu_prev: Vec<GpuCounter>,
    gpu_at: Option<Instant>,
    /// The last GPU pass's result, carried forward between passes.
    gpu_percent: Option<f32>,
    gpu_mem: Option<u64>,
}

struct GpuCounter {
    pdev: String,
    client: u64,
    engine: String,
    value: EngineUse,
}

/// Summed client load per `(PCI device, engine)` over all readable processes, percent.
pub(super) type EngineTotals = HashMap<(String, String), f32>;

/// One collection of per-process GPU use, which runs less often than the samples.
#[derive(Default)]
pub(super) struct GpuPass {
    pub nvml: HashMap<u32, nvml::ProcUse>,
    pub totals: EngineTotals,
}

pub(super) struct ProcTable {
    cache: HashMap<ProcKey, Cached>,
    users: HashMap<u32, Arc<str>>,
    arches: HashMap<&'static str, Arc<str>>,
    passwd_read: bool,
    tick: u64,
    gpu_passes: u64,
    path: String,
    stat_buf: Vec<u8>,
    buf: Vec<u8>,
    has_drm: bool,
}

impl ProcTable {
    pub(super) fn new() -> Self {
        ProcTable {
            cache: HashMap::new(),
            users: HashMap::new(),
            arches: HashMap::new(),
            passwd_read: false,
            tick: 0,
            gpu_passes: 0,
            path: String::with_capacity(64),
            stat_buf: Vec::with_capacity(1024),
            buf: Vec::with_capacity(4096),
            has_drm: Path::new("/dev/dri").exists(),
        }
    }

    fn leaf(&mut self, pid: u32, leaf: &str) -> &str {
        self.path.clear();
        let _ = write!(self.path, "/proc/{pid}/{leaf}");
        &self.path
    }

    fn read_leaf(&mut self, pid: u32, leaf: &str) -> bool {
        self.path.clear();
        let _ = write!(self.path, "/proc/{pid}/{leaf}");
        read_into(&self.path, &mut self.buf)
    }

    /// `pass` is `Some` when per-process GPU use is due; between passes the last values are
    /// carried forward.
    pub(super) fn read(&mut self, want: Want, mut pass: Option<&mut GpuPass>) -> Vec<ProcRaw> {
        self.tick = self.tick.wrapping_add(1);
        if pass.is_some() {
            self.gpu_passes = self.gpu_passes.wrapping_add(1);
        }
        let now = Instant::now();
        let mut out = Vec::with_capacity(self.cache.len().max(256));
        let Ok(dir) = fs::read_dir("/proc") else { return out };
        for entry in dir.flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            self.path.clear();
            let _ = write!(self.path, "/proc/{pid}/stat");
            if !read_into(&self.path, &mut self.stat_buf) {
                continue; // Gone between the listing and now.
            }
            let stat_buf = std::mem::take(&mut self.stat_buf);
            if let Some(p) = self.build(pid, &stat_buf, want, now, pass.as_deref_mut()) {
                out.push(p);
            }
            self.stat_buf = stat_buf;
        }
        let tick = self.tick;
        self.cache.retain(|_, c| c.seen == tick);
        out
    }

    fn build(
        &mut self,
        pid: u32,
        stat_buf: &[u8],
        want: Want,
        now: Instant,
        pass: Option<&mut GpuPass>,
    ) -> Option<ProcRaw> {
        let text = String::from_utf8_lossy(stat_buf);
        let st = ppid::parse_stat(&text)?;
        let hz = clk_tck();
        let key = ProcKey { pid, start_ms: start_ms_of(st.starttime), start_raw: st.starttime };
        let kernel = st.flags & PF_KTHREAD != 0 || st.ppid == 2 || pid == 2;

        let mut c = self.cache.remove(&key).unwrap_or_default();
        // After `exec` the key stays the same and `comm` changes: everything is stale then.
        if &*c.comm != st.comm {
            let comm: Arc<str> = Arc::from(st.comm);
            c = Cached { name: comm.clone(), comm, ..Cached::default() };
        }
        c.seen = self.tick;
        if c.tty_nr != st.tty_nr {
            c.tty_nr = st.tty_nr;
            c.session = ppid::tty_name(st.tty_nr).map(Arc::from);
        }
        let mut p = ProcRaw {
            key,
            parent: (st.ppid != 0).then_some(st.ppid),
            state: ppid::state_from_char(st.state),
            cpu_user_ms: st.utime.saturating_mul(1000) / hz,
            cpu_system_ms: st.stime.saturating_mul(1000) / hz,
            rss: st.rss_pages.saturating_mul(page_size()),
            virt: (st.vsize > 0).then_some(st.vsize),
            threads: st.num_threads,
            page_faults: Some(st.minflt.saturating_add(st.majflt)),
            priority: Some(nice_to_priority(st.nice)),
            nice: Some(st.nice),
            kernel,
            ..ProcRaw::default()
        };
        if want.process_details {
            self.details(&mut p, &mut c, kernel, now);
        }
        p.name = c.name.clone();
        p.session.clone_from(&c.session);
        if want.process_gpu && !kernel {
            if let Some(pass) = pass {
                c.gpu_percent = None;
                c.gpu_mem = None;
                if self.has_drm {
                    self.gpu_from_fdinfo(pid, &mut c, &mut pass.totals);
                }
                if let Some(u) = pass.nvml.get(&pid) {
                    c.gpu_percent = Some(c.gpu_percent.unwrap_or(0.0).max(u.percent));
                    c.gpu_mem = Some(c.gpu_mem.unwrap_or(0).saturating_add(u.memory));
                }
            }
            p.gpu_percent = c.gpu_percent;
            p.gpu_mem = c.gpu_mem;
        }
        self.cache.insert(key, c);
        Some(p)
    }

    fn details(&mut self, p: &mut ProcRaw, c: &mut Cached, kernel: bool, now: Instant) {
        let pid = p.key.pid;
        if kernel {
            p.user = Some(self.user_name(0));
            if !c.filled {
                c.filled = true;
                c.cmdline = Some(Arc::from(format!("[{}]", c.comm)));
            }
            p.cmdline.clone_from(&c.cmdline);
            return;
        }
        if !c.filled {
            let exe = fs::read_link(self.leaf(pid, "exe")).ok().map(|l| {
                let text = l.to_string_lossy();
                PathBuf::from(ppid::clean_exe_link(&text))
            });
            if exe.is_some() {
                c.arch = self.exe_arch(pid);
            }
            let file = exe.as_ref().and_then(|e| e.file_name()).map(|f| f.to_string_lossy().into_owned());
            let full = ppid::full_name(&c.comm, file.as_deref());
            if full != &*c.comm {
                c.name = Arc::from(full);
            }
            c.exe = exe.map(Arc::from);
        }
        if !c.filled || c.refreshed.is_none_or(|t| now.duration_since(t) >= TEXT_REFRESH) {
            if self.read_leaf(pid, "cmdline") {
                let line = ppid::cmdline_or_comm(&self.buf, &c.comm);
                keep_or_replace(&mut c.cmdline, Some(line.as_str()));
            }
            let cgroup =
                if self.read_leaf(pid, "cgroup") { ppid::parse_cgroup(&String::from_utf8_lossy(&self.buf)) } else { None };
            keep_or_replace(&mut c.cgroup, cgroup.as_deref());
            c.refreshed = Some(now);
            c.filled = true;
        }
        p.exe.clone_from(&c.exe);
        p.cmdline.clone_from(&c.cmdline);
        p.cgroup.clone_from(&c.cgroup);
        p.arch.clone_from(&c.arch);

        if self.read_leaf(pid, "status") {
            let s = ppid::parse_status(&String::from_utf8_lossy(&self.buf));
            if let Some(rss) = s.rss {
                p.rss = rss;
            }
            p.private = s.rss_anon;
            p.shared = match (s.rss_file, s.rss_shmem) {
                (None, None) => None,
                (a, b) => Some(a.unwrap_or(0).saturating_add(b.unwrap_or(0))),
            };
            p.peak_rss = s.vm_hwm;
            if let Some(uid) = s.euid {
                p.user = Some(self.user_name(uid));
                p.elevated = Some(uid == 0);
            }
        }
        // Readable for this user's own processes only (root: all); the rest stay `None`.
        if self.read_leaf(pid, "io") {
            let io = ppid::parse_io(&String::from_utf8_lossy(&self.buf));
            p.read_bytes = io.read_bytes;
            p.write_bytes = io.write_bytes;
            p.read_ops = io.read_ops;
            p.write_ops = io.write_ops;
        }
        p.handles = self.fd_count(pid);
    }

    fn exe_arch(&mut self, pid: u32) -> Option<Arc<str>> {
        let mut head = [0u8; 20];
        let mut f = fs::File::open(self.leaf(pid, "exe")).ok()?;
        f.read_exact(&mut head).ok()?;
        let arch = ppid::elf_arch(&head)?;
        Some(self.arches.entry(arch).or_insert_with(|| Arc::from(arch)).clone())
    }

    /// Open descriptors: since Linux 6.2 the size of the `fd` directory is the count, without
    /// listing it. A size of 0 is also what older kernels report, so that case lists.
    fn fd_count(&mut self, pid: u32) -> Option<u32> {
        let path = self.leaf(pid, "fd");
        match fs::metadata(path) {
            Ok(m) if m.len() > 0 => Some(m.len().min(u64::from(u32::MAX)) as u32),
            Ok(_) => Some(fs::read_dir(path).ok()?.count() as u32),
            Err(_) => None,
        }
    }

    fn user_name(&mut self, uid: u32) -> Arc<str> {
        if !self.passwd_read {
            self.passwd_read = true;
            if let Some(s) = super::read_text("/etc/passwd") {
                self.users = ppid::parse_passwd(&s).into_iter().map(|(k, v)| (k, Arc::from(v))).collect();
            }
        }
        self.users.entry(uid).or_insert_with(|| Arc::from(lookup_user(uid).unwrap_or_else(|| uid.to_string()))).clone()
    }

    fn gpu_from_fdinfo(&mut self, pid: u32, c: &mut Cached, totals: &mut EngineTotals) {
        if !c.drm_scanned || (self.gpu_passes.wrapping_add(u64::from(pid))).is_multiple_of(DRM_RESCAN_PASSES) {
            c.drm_scanned = true;
            c.drm_fds.clear();
            if let Ok(dir) = fs::read_dir(self.leaf(pid, "fd")) {
                for e in dir.flatten() {
                    let is_drm = fs::read_link(e.path()).is_ok_and(|l| l.starts_with("/dev/dri/"));
                    if let (true, Some(fd)) = (is_drm, e.file_name().to_str().and_then(|n| n.parse().ok())) {
                        c.drm_fds.push(fd);
                    }
                }
            }
        }
        if c.drm_fds.is_empty() {
            c.gpu_prev.clear();
            return;
        }
        let mut clients: Vec<DrmClient> = Vec::with_capacity(c.drm_fds.len());
        for fd in &c.drm_fds {
            self.path.clear();
            let _ = write!(self.path, "/proc/{pid}/fdinfo/{fd}");
            if !read_into(&self.path, &mut self.buf) {
                continue;
            }
            if let Some(cl) = ppid::parse_drm_fdinfo(&String::from_utf8_lossy(&self.buf)) {
                // Duplicated descriptors show the same client.
                if !clients.iter().any(|o| o.client_id == cl.client_id && o.pdev == cl.pdev) {
                    clients.push(cl);
                }
            }
        }
        if clients.is_empty() {
            return;
        }
        c.gpu_mem = Some(clients.iter().fold(0u64, |a, cl| a.saturating_add(cl.memory)));

        let now = Instant::now();
        let elapsed = c.gpu_at.map(|t| now.duration_since(t).as_nanos().min(u128::from(u64::MAX)) as u64);
        let mut engines: Vec<(&str, &str, f32)> = Vec::new();
        for cl in &clients {
            for (engine, value) in &cl.engines {
                let prev =
                    c.gpu_prev.iter().find(|g| g.client == cl.client_id && g.pdev == cl.pdev && g.engine == *engine);
                let (Some(prev), Some(elapsed)) = (prev, elapsed) else { continue };
                let Some(pct) = ppid::engine_percent(prev.value, *value, elapsed) else { continue };
                match engines.iter_mut().find(|e| e.0 == cl.pdev && e.1 == engine) {
                    Some(e) => e.2 += pct,
                    None => engines.push((&cl.pdev, engine, pct)),
                }
            }
        }
        if elapsed.is_some() {
            // The busiest engine, the same measure as `GpuRaw::percent`.
            c.gpu_percent = Some(engines.iter().fold(0f32, |m, e| m.max(e.2)).min(100.0));
        }
        for (pdev, engine, pct) in &engines {
            *totals.entry(((*pdev).to_owned(), (*engine).to_owned())).or_insert(0.0) += pct;
        }
        c.gpu_prev.clear();
        for cl in &clients {
            for (engine, value) in &cl.engines {
                c.gpu_prev.push(GpuCounter {
                    pdev: cl.pdev.clone(),
                    client: cl.client_id,
                    engine: engine.clone(),
                    value: *value,
                });
            }
        }
        c.gpu_at = Some(now);
    }
}

/// Keeps the shared text when it did not change, so a refresh allocates only for a real change.
fn keep_or_replace(slot: &mut Option<Arc<str>>, new: Option<&str>) {
    if slot.as_deref() != new {
        *slot = new.map(Arc::from);
    }
}

/// Accounts outside `/etc/passwd` (LDAP, systemd-homed, dynamic users).
fn lookup_user(uid: u32) -> Option<String> {
    let mut buf = vec![0u8; 4096];
    // SAFETY: an all-zero `passwd` is a valid value (null pointers, zero ids); it is only read
    // after `getpwuid_r` filled it.
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer is valid for the call, and `buf`'s real length is passed.
    let rc = unsafe { libc::getpwuid_r(uid, &raw mut pw, buf.as_mut_ptr().cast(), buf.len(), &raw mut result) };
    if rc != 0 || result.is_null() || pw.pw_name.is_null() {
        return None;
    }
    // SAFETY: on success `pw_name` is a NUL-terminated string inside `buf`, which is alive here.
    Some(unsafe { std::ffi::CStr::from_ptr(pw.pw_name) }.to_string_lossy().into_owned())
}
