//! The process table: `SystemProcessInformation` for counters (no per-process handle needed),
//! with a per-process cache of what does need a handle (path, command line, user, …).

use std::collections::{HashMap, HashSet};
use std::mem::{offset_of, size_of};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows_sys::Wdk::System::SystemInformation::SystemProcessInformation;
use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation};
use windows_sys::Win32::Foundation::{FILETIME, HANDLE};
use windows_sys::Win32::Security::{
    GetLengthSid, GetTokenInformation, LookupAccountSidW, SID_NAME_USE, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER,
    TokenElevation, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, QueryDosDeviceW, VerQueryValueW,
};
use windows_sys::Win32::System::SystemInformation::IMAGE_FILE_MACHINE;
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, IsWow64Process2, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

use super::super::{Handle, ft_u64, wide_str};
use super::gpu::ProcGpu;
use super::{
    STATUS_INFO_LENGTH_MISMATCH, as_bytes, query_system, read_pod, text_in_buffer, ticks_to_ms, ticks_to_unix_ms,
};
use crate::sysmon::{ProcKey, ProcPriority, ProcRaw, ProcState, ThreadInfo, Want};

/// `SystemProcessIdInformation`: a process's image path by PID without opening it.
const SYSTEM_PROCESS_ID_INFORMATION: i32 = 88;
/// New descriptions read from executables per sample, so a first sample stays quick.
const DESCRIPTION_BUDGET: u32 = 16;
/// Cached descriptions, after which the cache restarts.
const MAX_DESCRIPTIONS: usize = 4096;
/// A program may rewrite its command line while running (worker titles, for one).
const CMDLINE_REFRESH: Duration = Duration::from_secs(30);

/// `SYSTEM_PROCESS_INFORMATION`. Pointers and handles as `usize`: never dereferenced.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct ProcEntry {
    pub next: u32,
    pub threads: u32,
    pub private_ws: i64,
    pub hard_faults: u32,
    pub threads_high: u32,
    pub cycles: u64,
    pub create: i64,
    pub user: i64,
    pub kernel: i64,
    pub name_len: u16,
    pub name_max: u16,
    pub name_ptr: usize,
    pub base_priority: i32,
    pub pid: usize,
    pub parent: usize,
    pub handles: u32,
    pub session: u32,
    pub key: usize,
    pub peak_virtual: usize,
    pub virtual_size: usize,
    pub page_faults: u32,
    pub peak_ws: usize,
    pub ws: usize,
    pub quota_peak_paged: usize,
    pub quota_paged: usize,
    pub quota_peak_nonpaged: usize,
    pub quota_nonpaged: usize,
    pub pagefile: usize,
    pub peak_pagefile: usize,
    pub private: usize,
    pub read_ops: i64,
    pub write_ops: i64,
    pub other_ops: i64,
    pub read_bytes: i64,
    pub write_bytes: i64,
    pub other_bytes: i64,
}

/// `SYSTEM_THREAD_INFORMATION`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct ThreadEntry {
    pub kernel: i64,
    pub user: i64,
    pub create: i64,
    pub wait_time: u32,
    pub start_address: usize,
    pub process: usize,
    pub thread: usize,
    pub priority: i32,
    pub base_priority: i32,
    pub context_switches: u32,
    pub state: u32,
    pub wait_reason: u32,
}

const THREAD_READY: u32 = 1;
const THREAD_RUNNING: u32 = 2;
const THREAD_STANDBY: u32 = 3;
const THREAD_TERMINATED: u32 = 4;
const THREAD_WAITING: u32 = 5;
const THREAD_DEFERRED_READY: u32 = 7;
const WAIT_SUSPENDED: u32 = 5;

/// Visit each process entry, with the entry's thread records. Stops at anything that does
/// not fit the buffer; a wrong thread count gives an empty thread list.
pub(super) fn walk<'a>(buf: &'a [u8], mut f: impl FnMut(&ProcEntry, &'a [u8])) {
    let mut off = 0usize;
    loop {
        let Some(e) = read_pod::<ProcEntry>(buf, off) else { return };
        let start = off + size_of::<ProcEntry>();
        let threads = (e.threads as usize)
            .checked_mul(size_of::<ThreadEntry>())
            .and_then(|n| n.checked_add(start))
            .and_then(|end| buf.get(start..end))
            .unwrap_or(&[]);
        f(&e, threads);
        // A link shorter than the header would make the walk loop or overlap.
        if (e.next as usize) < size_of::<ProcEntry>() {
            return;
        }
        off += e.next as usize;
    }
}

fn threads_of(bytes: &[u8]) -> impl Iterator<Item = ThreadEntry> + '_ {
    bytes.as_chunks::<{ size_of::<ThreadEntry>() }>().0.iter().filter_map(|c| read_pod::<ThreadEntry>(c, 0))
}

fn state_of(threads: &[u8], pid: usize) -> ProcState {
    if pid == 0 {
        return ProcState::Idle;
    }
    let (mut any, mut all_suspended, mut running) = (false, true, false);
    for t in threads_of(threads) {
        any = true;
        if t.state == THREAD_WAITING && t.wait_reason == WAIT_SUSPENDED {
            continue;
        }
        all_suspended = false;
        if matches!(t.state, THREAD_RUNNING | THREAD_READY | THREAD_STANDBY | THREAD_DEFERRED_READY) {
            running = true;
        }
    }
    match (any, all_suspended, running) {
        // No threads left: exited, kept alive by open handles.
        (false, ..) => ProcState::Zombie,
        (true, true, _) => ProcState::Suspended,
        (true, false, true) => ProcState::Running,
        (true, false, false) => ProcState::Sleeping,
    }
}

fn thread_state(t: &ThreadEntry) -> ProcState {
    match t.state {
        THREAD_RUNNING | THREAD_READY | THREAD_STANDBY | THREAD_DEFERRED_READY => ProcState::Running,
        THREAD_WAITING if t.wait_reason == WAIT_SUSPENDED => ProcState::Suspended,
        THREAD_WAITING => ProcState::Sleeping,
        THREAD_TERMINATED => ProcState::Stopped,
        _ => ProcState::Unknown,
    }
}

/// The priority class behind a process's base priority number.
fn priority_of(base: i32) -> Option<ProcPriority> {
    Some(match base {
        ..=0 => return None,
        1..=4 => ProcPriority::Idle,
        5..=7 => ProcPriority::BelowNormal,
        8..=9 => ProcPriority::Normal,
        10..=12 => ProcPriority::AboveNormal,
        13..=15 => ProcPriority::High,
        _ => ProcPriority::Realtime,
    })
}

/// OS-internal pseudo processes (they have no `.exe` name and no user-mode image).
fn is_kernel(pid: u32, name: &str) -> bool {
    pid == 0 || pid == 4 || matches!(name, "Registry" | "Memory Compression" | "Secure System")
}

fn entry_name(buf: &[u8], e: &ProcEntry) -> String {
    if e.pid == 0 {
        return "System Idle Process".to_string();
    }
    text_in_buffer(buf, e.name_ptr, usize::from(e.name_len))
}

/// What rarely changes while a process runs, shared with the rows it appears in.
struct Static {
    name: Arc<str>,
    session: Arc<str>,
    kernel: bool,
    /// The handle-based fields below were read.
    loaded: bool,
    exe: Option<Arc<Path>>,
    cmdline: Option<Arc<str>>,
    /// When the command line was last read. `None`: the process cannot be opened, so there is
    /// nothing to refresh.
    cmdline_at: Option<Instant>,
    user: Option<Arc<str>>,
    elevated: Option<bool>,
    arch: Option<Arc<str>>,
    description: Option<Arc<str>>,
    description_done: bool,
}

/// One sample's process rows and totals.
#[derive(Default)]
pub(super) struct Table {
    pub rows: Vec<ProcRaw>,
    /// Without the idle process's one thread per CPU, which are not work.
    pub threads: u64,
    pub handles: u64,
    /// The compression store's resident size.
    pub compressed: Option<u64>,
}

#[derive(Default)]
pub(super) struct ProcReader {
    buf: Vec<u64>,
    cache: HashMap<ProcKey, Static>,
    /// Account names per SID bytes.
    users: HashMap<Vec<u8>, Option<Arc<str>>>,
    /// Product descriptions per executable.
    descriptions: HashMap<Arc<Path>, Option<Arc<str>>>,
    sessions: HashMap<u32, Arc<str>>,
    arches: HashMap<&'static str, Arc<str>>,
    seen: HashSet<ProcKey>,
    /// Scratch for image paths.
    wide: Vec<u16>,
}

/// State a sample's walk passes from row to row.
struct Pass<'a> {
    want: Want,
    gpu: Option<&'a ProcGpu>,
    description_budget: u32,
    devices: Option<Vec<(String, String)>>,
    now: Instant,
}

impl ProcReader {
    pub(super) fn read(&mut self, want: Want, gpu: Option<&ProcGpu>) -> Table {
        let mut table = Table::default();
        if self.buf.is_empty() {
            self.buf.resize((1 << 20) / 8, 0);
        }
        let mut buf = std::mem::take(&mut self.buf);
        if let Some(len) = query_system(SystemProcessInformation, &mut buf) {
            let bytes = as_bytes(&buf, len);
            table.rows.reserve(self.cache.len().max(512));
            let mut pass =
                Pass { want, gpu, description_budget: DESCRIPTION_BUDGET, devices: None, now: Instant::now() };
            self.seen.clear();
            walk(bytes, |e, threads| {
                let row = self.row(bytes, e, threads, &mut pass);
                if row.kernel && &*row.name == "Memory Compression" {
                    table.compressed = Some(row.rss);
                }
                if !row.idle {
                    table.threads += u64::from(e.threads);
                }
                table.handles += u64::from(e.handles);
                table.rows.push(row);
            });
            let seen = &self.seen;
            self.cache.retain(|k, _| seen.contains(k));
        }
        self.buf = buf;
        table
    }

    fn row(&mut self, bytes: &[u8], e: &ProcEntry, threads: &[u8], pass: &mut Pass<'_>) -> ProcRaw {
        let pid = e.pid as u32;
        let idle = e.pid == 0;
        let key = ProcKey { pid, start_ms: ticks_to_unix_ms(e.create), start_raw: e.create.max(0) as u64 };
        self.seen.insert(key);
        let ProcReader { cache, users, descriptions, sessions, arches, wide, .. } = self;
        let s = cache.entry(key).or_insert_with(|| {
            let name = entry_name(bytes, e);
            let session = sessions.entry(e.session).or_insert_with(|| Arc::from(e.session.to_string()));
            Static {
                kernel: is_kernel(pid, &name),
                name: Arc::from(name),
                session: session.clone(),
                loaded: false,
                exe: None,
                cmdline: None,
                cmdline_at: None,
                user: None,
                elevated: None,
                arch: None,
                description: None,
                description_done: false,
            }
        });
        if pass.want.process_details && !s.kernel {
            if s.loaded {
                refresh_cmdline(s, key, pass.now);
            } else {
                load_static(s, key, users, arches, wide, &mut pass.devices, pass.now);
            }
            if !s.description_done
                && let Some(exe) = &s.exe
                && let Some(d) = description(exe, descriptions, &mut pass.description_budget)
            {
                s.description = d;
                s.description_done = true;
            }
        }
        let mut row = ProcRaw {
            key,
            parent: (!idle).then_some(e.parent as u32),
            name: s.name.clone(),
            state: state_of(threads, e.pid),
            // The idle process's times are the machine's idle time, not work it did.
            cpu_user_ms: if idle { 0 } else { ticks_to_ms(e.user) },
            cpu_system_ms: if idle { 0 } else { ticks_to_ms(e.kernel) },
            rss: e.ws as u64,
            private: Some(e.private as u64),
            shared: Some((e.ws as u64).saturating_sub(e.private_ws.max(0) as u64)),
            virt: Some(e.virtual_size as u64),
            peak_rss: Some(e.peak_ws as u64),
            threads: e.threads,
            handles: Some(e.handles),
            // The transfer counts cover all of a process's I/O calls: files, but also pipes,
            // sockets and devices. They are not disk traffic alone.
            read_bytes: Some(e.read_bytes.max(0) as u64),
            write_bytes: Some(e.write_bytes.max(0) as u64),
            read_ops: Some(e.read_ops.max(0) as u64),
            write_ops: Some(e.write_ops.max(0) as u64),
            page_faults: Some(u64::from(e.page_faults)),
            priority: priority_of(e.base_priority),
            nice: Some(e.base_priority),
            session: Some(s.session.clone()),
            kernel: s.kernel,
            idle,
            ..ProcRaw::default()
        };
        if let Some(g) = pass.gpu {
            row.gpu_percent = g.percent.as_ref().map(|m| m.get(&pid).copied().unwrap_or(0.0));
            row.gpu_mem = g.mem.as_ref().map(|m| m.get(&pid).copied().unwrap_or(0));
        }
        if pass.want.process_details && !s.kernel {
            row.exe.clone_from(&s.exe);
            row.cmdline.clone_from(&s.cmdline);
            row.user.clone_from(&s.user);
            row.elevated = s.elevated;
            row.arch.clone_from(&s.arch);
            row.description.clone_from(&s.description);
        }
        row
    }
}

/// A handle on `key`'s process, or `None` when it cannot be opened or the PID was reused
/// since the table was read.
fn open_for_reading(key: ProcKey) -> Option<Handle> {
    // SAFETY: plain call; the result is checked by `Handle::new`.
    let h = Handle::new(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, key.pid) }).ok()?;
    (creation_time(h.raw()) == Some(key.start_raw)).then_some(h)
}

fn load_static(
    s: &mut Static,
    key: ProcKey,
    users: &mut HashMap<Vec<u8>, Option<Arc<str>>>,
    arches: &mut HashMap<&'static str, Arc<str>>,
    wide: &mut Vec<u16>,
    devices: &mut Option<Vec<(String, String)>>,
    now: Instant,
) {
    s.loaded = true;
    if let Some(h) = open_for_reading(key) {
        s.exe = image_path(h.raw(), wide).map(|p| Arc::from(Path::new(&p)));
        s.cmdline = command_line_of(h.raw()).map(Arc::from);
        s.cmdline_at = Some(first_cmdline_time(key.pid, now));
        s.arch = arch(h.raw()).map(|a| arches.entry(a).or_insert_with(|| Arc::from(a)).clone());
        if let Some(token) = open_token(h.raw()) {
            s.elevated = elevated(token.raw());
            s.user = token_sid(token.raw()).and_then(|sid| {
                if let Some(known) = users.get(&sid) {
                    return known.clone();
                }
                let name = account_name(&sid).map(Arc::<str>::from);
                users.insert(sid, name.clone());
                name
            });
        }
    }
    if s.exe.is_none() {
        s.exe = protected_image_path(key.pid, wide, devices).map(|p| Arc::from(Path::new(&p)));
    }
}

/// Spreads refreshes over the period by PID, so they do not all fall in one sample.
fn first_cmdline_time(pid: u32, now: Instant) -> Instant {
    now.checked_sub(Duration::from_secs(u64::from(pid / 4) % CMDLINE_REFRESH.as_secs())).unwrap_or(now)
}

fn refresh_cmdline(s: &mut Static, key: ProcKey, now: Instant) {
    let Some(at) = s.cmdline_at else { return };
    if now.saturating_duration_since(at) < CMDLINE_REFRESH {
        return;
    }
    s.cmdline_at = Some(now);
    let Some(h) = open_for_reading(key) else { return };
    // The shared text stays when nothing changed.
    if let Some(new) = command_line_of(h.raw())
        && s.cmdline.as_deref() != Some(new.as_str())
    {
        s.cmdline = Some(Arc::from(new));
    }
}

/// The description of `exe`: `None` when the budget for new files is used up this sample,
/// to be asked again next time.
fn description(
    exe: &Arc<Path>,
    known: &mut HashMap<Arc<Path>, Option<Arc<str>>>,
    budget: &mut u32,
) -> Option<Option<Arc<str>>> {
    if let Some(d) = known.get(exe) {
        return Some(d.clone());
    }
    if *budget == 0 {
        return None;
    }
    *budget -= 1;
    // Reading a file from a network path may stall: not cheap, so not read.
    let d = if exe.as_os_str().as_encoded_bytes().starts_with(br"\\") { None } else { file_description(exe) };
    let d = d.map(Arc::<str>::from);
    if known.len() >= MAX_DESCRIPTIONS {
        known.clear();
    }
    known.insert(exe.clone(), d.clone());
    Some(d)
}

/// A process's creation time as the OS stores it (`ProcKey::start_raw`).
pub(super) fn creation_time(h: HANDLE) -> Option<u64> {
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: open handle; valid out-pointers.
    let ok = unsafe { GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user) };
    (ok != 0).then(|| ft_u64(&created))
}

fn image_path(h: HANDLE, wide: &mut Vec<u16>) -> Option<String> {
    wide.resize(32_768, 0);
    let mut len = wide.len() as u32;
    // SAFETY: open handle; output buffer of `len` characters; `len` is a valid in/out pointer.
    let ok = unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, wide.as_mut_ptr(), &mut len) };
    (ok != 0 && len > 0).then(|| String::from_utf16_lossy(&wide[..(len as usize).min(wide.len())]))
}

/// `SYSTEM_PROCESS_ID_INFORMATION`: a PID in, a UNICODE_STRING over our buffer out.
#[repr(C)]
struct ProcessIdInfo {
    pid: usize,
    len: u16,
    max: u16,
    buffer: *mut u16,
}

/// The image path of a process that cannot be opened (protected ones), which comes as an NT
/// device path.
fn protected_image_path(pid: u32, wide: &mut Vec<u16>, devices: &mut Option<Vec<(String, String)>>) -> Option<String> {
    if pid == 0 {
        return None;
    }
    wide.resize(32_768, 0);
    let mut info = ProcessIdInfo {
        pid: pid as usize,
        len: 0,
        max: (wide.len() * 2).min(usize::from(u16::MAX) & !1) as u16,
        buffer: wide.as_mut_ptr(),
    };
    // SAFETY: `info` matches the class's layout and its size is passed; its buffer pointer
    // and `max` describe the writable `wide`.
    let st = unsafe {
        windows_sys::Wdk::System::SystemInformation::NtQuerySystemInformation(
            SYSTEM_PROCESS_ID_INFORMATION,
            (&raw mut info).cast(),
            size_of::<ProcessIdInfo>() as u32,
            null_mut(),
        )
    };
    if st < 0 || info.len == 0 {
        return None;
    }
    let nt = String::from_utf16_lossy(&wide[..usize::from(info.len / 2).min(wide.len())]);
    Some(dos_path(&nt, devices.get_or_insert_with(dos_devices)))
}

/// `(NT device, drive)` pairs: `\Device\HarddiskVolume3` is `C:`.
fn dos_devices() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut target = [0u16; 512];
    for letter in b'A'..=b'Z' {
        let drive = format!("{}:", letter as char);
        let w = wide_str(&drive);
        // SAFETY: NUL-terminated name; output buffer of the character count we pass.
        let n = unsafe { QueryDosDeviceW(w.as_ptr(), target.as_mut_ptr(), target.len() as u32) };
        if n > 0 {
            let end = target.iter().position(|&c| c == 0).unwrap_or(target.len());
            out.push((String::from_utf16_lossy(&target[..end]), drive));
        }
    }
    out
}

/// An NT device path in drive-letter or UNC form, unchanged when no device matches.
fn dos_path(nt: &str, devices: &[(String, String)]) -> String {
    if let Some(rest) = nt.strip_prefix(r"\Device\Mup\") {
        return format!(r"\\{rest}");
    }
    for (device, drive) in devices {
        if let Some(rest) = nt.strip_prefix(device.as_str())
            && rest.starts_with('\\')
        {
            return format!("{drive}{rest}");
        }
    }
    nt.to_string()
}

pub(super) fn command_line_of(h: HANDLE) -> Option<String> {
    let mut buf = vec![0u64; 4096 / 8];
    for _ in 0..4 {
        let bytes = buf.len() * 8;
        let mut ret = 0u32;
        // SAFETY: open handle; `buf` is writable for `bytes` bytes; valid out-pointer.
        let st = unsafe {
            NtQueryInformationProcess(h, ProcessCommandLineInformation, buf.as_mut_ptr().cast(), bytes as u32, &mut ret)
        };
        if st == STATUS_INFO_LENGTH_MISMATCH {
            let want = (ret as usize).max(bytes * 2);
            if want > 1 << 20 {
                return None;
            }
            buf.resize(want.div_ceil(8), 0);
            continue;
        }
        if st < 0 {
            return None;
        }
        // A UNICODE_STRING followed by its text, inside the buffer.
        let b = as_bytes(&buf, (ret as usize).min(bytes));
        let len = usize::from(read_pod::<u16>(b, 0)?);
        let ptr = read_pod::<usize>(b, offset_of!(UnicodeHeader, buffer))?;
        let s = text_in_buffer(b, ptr, len);
        return (!s.is_empty()).then_some(s);
    }
    None
}

/// `UNICODE_STRING`, for its field offset.
#[repr(C)]
struct UnicodeHeader {
    len: u16,
    max: u16,
    buffer: usize,
}

fn arch(h: HANDLE) -> Option<&'static str> {
    let mut process: IMAGE_FILE_MACHINE = 0;
    let mut native: IMAGE_FILE_MACHINE = 0;
    // SAFETY: open handle; valid out-pointers.
    if unsafe { IsWow64Process2(h, &mut process, &mut native) } == 0 {
        return None;
    }
    let machine = if process == 0 { native } else { process };
    match machine {
        0x8664 => Some("x64"),
        0xAA64 => Some("arm64"),
        0x014C => Some("x86"),
        0x01C4 => Some("arm"),
        _ => None,
    }
}

fn open_token(h: HANDLE) -> Option<Handle> {
    let mut token: HANDLE = null_mut();
    // SAFETY: open handle; valid out-pointer.
    let ok = unsafe { OpenProcessToken(h, TOKEN_QUERY, &mut token) };
    if ok == 0 {
        return None;
    }
    Handle::new(token).ok()
}

fn elevated(token: HANDLE) -> Option<bool> {
    let mut e = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut ret = 0u32;
    // SAFETY: open token; `e` has the size we pass; valid out-pointer.
    let ok = unsafe {
        GetTokenInformation(token, TokenElevation, (&raw mut e).cast(), size_of::<TOKEN_ELEVATION>() as u32, &mut ret)
    };
    (ok != 0).then_some(e.TokenIsElevated != 0)
}

/// The token's user SID as bytes.
fn token_sid(token: HANDLE) -> Option<Vec<u8>> {
    let mut buf = [0u64; 64];
    let mut ret = 0u32;
    // SAFETY: open token; `buf` is 8-aligned and writable for the size we pass.
    let ok = unsafe {
        GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), size_of::<[u64; 64]>() as u32, &mut ret)
    };
    if ok == 0 {
        return None;
    }
    // SAFETY: on success `buf` starts with a TOKEN_USER whose SID pointer points into `buf`
    // and stays valid while `buf` lives.
    unsafe {
        let user = &*buf.as_ptr().cast::<TOKEN_USER>();
        let sid = user.User.Sid;
        let n = GetLengthSid(sid) as usize;
        Some(std::slice::from_raw_parts(sid.cast::<u8>(), n.min(68)).to_vec())
    }
}

fn account_name(sid: &[u8]) -> Option<String> {
    let mut sid = sid.to_vec();
    let mut name = [0u16; 256];
    let mut domain = [0u16; 256];
    let (mut nl, mut dl) = (name.len() as u32, domain.len() as u32);
    let mut kind: SID_NAME_USE = 0;
    // SAFETY: local lookup; `sid` holds a whole SID copied from a token; output buffers of
    // the character counts we pass; valid out-pointers.
    let ok = unsafe {
        LookupAccountSidW(
            null(),
            sid.as_mut_ptr().cast(),
            name.as_mut_ptr(),
            &mut nl,
            domain.as_mut_ptr(),
            &mut dl,
            &mut kind,
        )
    };
    (ok != 0).then(|| String::from_utf16_lossy(&name[..(nl as usize).min(name.len())]))
}

/// `FileDescription` from an executable's version resource.
fn file_description(exe: &Path) -> Option<String> {
    let path = wide_str(exe);
    let mut ignored = 0u32;
    // SAFETY: NUL-terminated path; valid out-pointer.
    let size = unsafe { GetFileVersionInfoSizeW(path.as_ptr(), &mut ignored) } as usize;
    if size == 0 || size > 16 << 20 {
        return None;
    }
    let mut block = vec![0u8; size];
    // SAFETY: NUL-terminated path; `block` is writable for `size` bytes.
    if unsafe { GetFileVersionInfoW(path.as_ptr(), 0, size as u32, block.as_mut_ptr().cast()) } == 0 {
        return None;
    }
    let mut lang = None;
    let mut ptr = null_mut();
    let mut len = 0u32;
    let sub = wide_str(r"\VarFileInfo\Translation");
    // SAFETY: version block read above; NUL-terminated sub-block; valid out-pointers.
    if unsafe { VerQueryValueW(block.as_ptr().cast(), sub.as_ptr(), &mut ptr, &mut len) } != 0 && len >= 4 {
        // SAFETY: the call points `ptr` at `len >= 4` bytes inside `block`.
        let t: [u16; 2] = unsafe { std::ptr::read_unaligned(ptr.cast::<[u16; 2]>()) };
        lang = Some(format!("{:04x}{:04x}", t[0], t[1]));
    }
    let mut langs: Vec<String> = lang.into_iter().collect();
    langs.extend(["040904b0".to_string(), "040904e4".to_string()]);
    for l in langs {
        let sub = wide_str(format!(r"\StringFileInfo\{l}\FileDescription"));
        let mut ptr = null_mut();
        let mut chars = 0u32;
        // SAFETY: version block read above; NUL-terminated sub-block; valid out-pointers.
        if unsafe { VerQueryValueW(block.as_ptr().cast(), sub.as_ptr(), &mut ptr, &mut chars) } != 0 && chars > 0 {
            let chars = (chars as usize).min(size / 2);
            // SAFETY: for a string value `ptr` points at `chars` UTF-16 units inside `block`.
            let units = unsafe { std::slice::from_raw_parts(ptr.cast::<u16>(), chars) };
            let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());
            let s = String::from_utf16_lossy(&units[..end]).trim().to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

/// The threads of one process, verified against `key`. `None`: no such process any more.
pub(super) fn threads_of_process(key: ProcKey) -> Option<Vec<ThreadInfo>> {
    let mut buf = vec![0u64; (1 << 20) / 8];
    let len = query_system(SystemProcessInformation, &mut buf)?;
    let bytes = as_bytes(&buf, len);
    let mut out = None;
    walk(bytes, |e, threads| {
        if e.pid as u32 == key.pid && e.create.max(0) as u64 == key.start_raw {
            out = Some(
                threads_of(threads)
                    .map(|t| ThreadInfo {
                        tid: t.thread as u32,
                        name: String::new(),
                        state: thread_state(&t),
                        cpu_ms: ticks_to_ms(t.kernel).saturating_add(ticks_to_ms(t.user)),
                        priority: Some(t.priority),
                    })
                    .collect::<Vec<_>>(),
            );
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn layouts_match_the_documented_ones() {
        assert_eq!(size_of::<ProcEntry>(), 256);
        assert_eq!(size_of::<ThreadEntry>(), 80);
        assert_eq!(offset_of!(ProcEntry, create), 32);
        assert_eq!(offset_of!(ProcEntry, name_ptr), 64);
        assert_eq!(offset_of!(ProcEntry, base_priority), 72);
        assert_eq!(offset_of!(ProcEntry, pid), 80);
        assert_eq!(offset_of!(ProcEntry, handles), 96);
        assert_eq!(offset_of!(ProcEntry, ws), 144);
        assert_eq!(offset_of!(ProcEntry, private), 200);
        assert_eq!(offset_of!(ProcEntry, read_bytes), 232);
        assert_eq!(offset_of!(ThreadEntry, thread), 48);
        assert_eq!(offset_of!(ThreadEntry, state), 68);
        assert_eq!(size_of::<ProcessIdInfo>(), 24);
    }

    fn bytes_of<T: Copy>(v: &T) -> Vec<u8> {
        // SAFETY: test-only view of a plain `#[repr(C)]` struct; padding is zeroed by the
        // `Default` construction or simply ignored by the reader.
        unsafe { std::slice::from_raw_parts((v as *const T).cast::<u8>(), size_of::<T>()) }.to_vec()
    }

    fn entry(pid: usize, next: u32, threads: u32) -> ProcEntry {
        ProcEntry { pid, next, threads, ..ProcEntry::default() }
    }

    #[test]
    fn walk_follows_links_and_survives_bad_data() {
        let mut b = bytes_of(&entry(1, 256, 0));
        b.extend(bytes_of(&entry(2, 0, 0)));
        let mut pids = Vec::new();
        walk(&b, |e, _| pids.push(e.pid));
        assert_eq!(pids, [1, 2]);

        // A link into the entry itself ends the walk instead of looping.
        let b = bytes_of(&entry(3, 8, 0));
        let mut n = 0;
        walk(&b, |_, _| n += 1);
        assert_eq!(n, 1);

        // A link past the end, a huge thread count, a truncated buffer.
        let b = bytes_of(&entry(4, 100_000, u32::MAX));
        let mut got = Vec::new();
        walk(&b, |e, t| got.push((e.pid, t.len())));
        assert_eq!(got, [(4, 0)]);
        let mut n = 0;
        walk(&b[..100], |_, _| n += 1);
        assert_eq!(n, 0);
        walk(&[], |_, _| n += 1);
        assert_eq!(n, 0);
    }

    #[test]
    fn thread_states_give_the_process_state() {
        let t = |state, wait_reason| bytes_of(&ThreadEntry { state, wait_reason, ..ThreadEntry::default() });
        let suspended = t(THREAD_WAITING, WAIT_SUSPENDED);
        let waiting = t(THREAD_WAITING, 6);
        let running = t(THREAD_RUNNING, 0);
        assert_eq!(state_of(&suspended, 9), ProcState::Suspended);
        assert_eq!(state_of(&[suspended.clone(), waiting.clone()].concat(), 9), ProcState::Sleeping);
        assert_eq!(state_of(&[waiting, running].concat(), 9), ProcState::Running);
        assert_eq!(state_of(&[], 9), ProcState::Zombie);
        assert_eq!(state_of(&[], 0), ProcState::Idle);
    }

    #[test]
    fn base_priority_maps_to_classes() {
        assert_eq!(priority_of(4), Some(ProcPriority::Idle));
        assert_eq!(priority_of(6), Some(ProcPriority::BelowNormal));
        assert_eq!(priority_of(8), Some(ProcPriority::Normal));
        assert_eq!(priority_of(10), Some(ProcPriority::AboveNormal));
        assert_eq!(priority_of(13), Some(ProcPriority::High));
        assert_eq!(priority_of(24), Some(ProcPriority::Realtime));
        assert_eq!(priority_of(0), None);
    }

    #[test]
    fn refreshes_are_spread_over_the_period() {
        let now = Instant::now() + Duration::from_secs(3600);
        let age = |pid| now.duration_since(first_cmdline_time(pid, now));
        assert!(age(4) < CMDLINE_REFRESH);
        assert_ne!(age(4), age(8));
        assert!((0..4000).step_by(4).all(|pid| age(pid) < CMDLINE_REFRESH));
    }

    /// The real path: this process's row picks up a changed command line once it is due, and
    /// keeps sharing the same text until then.
    #[test]
    fn command_line_refreshes_when_due() {
        let want = Want { hardware: false, process_gpu: false, ..Want::default() };
        let me = std::process::id();
        let mut r = ProcReader::default();
        let row = |t: Table| t.rows.into_iter().find(|p| p.key.pid == me).expect("this process");
        let first = row(r.read(want, None));
        let real = first.cmdline.clone().expect("own command line");
        let second = row(r.read(want, None));
        assert!(Arc::ptr_eq(&real, second.cmdline.as_ref().unwrap()), "text is shared between samples");
        assert!(Arc::ptr_eq(first.exe.as_ref().unwrap(), second.exe.as_ref().unwrap()));
        assert!(Arc::ptr_eq(&first.name, &second.name));

        let entry = r.cache.get_mut(&first.key).unwrap();
        entry.cmdline = Some(Arc::from("old text"));
        entry.cmdline_at = Some(Instant::now() + Duration::from_secs(3600));
        assert_eq!(row(r.read(want, None)).cmdline.as_deref(), Some("old text"), "not due yet");
        let entry = r.cache.get_mut(&first.key).unwrap();
        entry.cmdline_at = Instant::now().checked_sub(CMDLINE_REFRESH);
        assert_eq!(row(r.read(want, None)).cmdline.as_deref(), Some(&*real), "due");
    }

    #[test]
    fn nt_paths_become_dos_paths() {
        let d = vec![(r"\Device\HarddiskVolume3".to_string(), "C:".to_string())];
        assert_eq!(dos_path(r"\Device\HarddiskVolume3\Windows\a.exe", &d), r"C:\Windows\a.exe");
        assert_eq!(dos_path(r"\Device\HarddiskVolume30\a.exe", &d), r"\Device\HarddiskVolume30\a.exe");
        assert_eq!(dos_path(r"\Device\Mup\srv\share\a.exe", &d), r"\\srv\share\a.exe");
    }
}
