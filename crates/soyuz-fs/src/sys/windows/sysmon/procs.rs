//! The process table: `SystemProcessInformation` for counters (no per-process handle needed),
//! with a per-process cache of what does need a handle (path, command line, user, …).

use std::collections::{HashMap, HashSet};
use std::mem::{offset_of, size_of};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use windows_sys::Wdk::System::SystemInformation::SystemProcessInformation;
use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessCommandLineInformation};
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Security::{
    GetLengthSid, GetTokenInformation, LookupAccountSidW, SID_NAME_USE, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER,
    TokenElevation, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, QueryDosDeviceW, VerQueryValueW,
};
use windows_sys::Win32::System::SystemInformation::IMAGE_FILE_MACHINE;
use windows_sys::Win32::System::Threading::{
    IsWow64Process2, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};

use super::super::{Handle, wide_str};
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

/// What does not change while a process runs.
#[derive(Default)]
struct Static {
    exe: Option<PathBuf>,
    cmdline: Option<String>,
    user: Option<String>,
    elevated: Option<bool>,
    arch: Option<String>,
}

/// One sample's process rows and totals.
#[derive(Default)]
pub(super) struct Table {
    pub rows: Vec<ProcRaw>,
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
    users: HashMap<Vec<u8>, Option<String>>,
    /// Product descriptions per executable.
    descriptions: HashMap<PathBuf, Option<String>>,
    seen: HashSet<ProcKey>,
    /// Scratch for image paths.
    wide: Vec<u16>,
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
            table.rows.reserve(512);
            let mut budget = DESCRIPTION_BUDGET;
            let mut devices = None;
            self.seen.clear();
            walk(bytes, |e, threads| {
                let row = self.row(bytes, e, threads, want, gpu, &mut budget, &mut devices);
                if row.name == "Memory Compression" && row.kernel {
                    table.compressed = Some(row.rss);
                }
                table.threads += u64::from(e.threads);
                table.handles += u64::from(e.handles);
                table.rows.push(row);
            });
            let seen = &self.seen;
            self.cache.retain(|k, _| seen.contains(k));
        }
        self.buf = buf;
        table
    }

    #[allow(clippy::too_many_arguments)]
    fn row(
        &mut self,
        bytes: &[u8],
        e: &ProcEntry,
        threads: &[u8],
        want: Want,
        gpu: Option<&ProcGpu>,
        budget: &mut u32,
        devices: &mut Option<Vec<(String, String)>>,
    ) -> ProcRaw {
        let pid = e.pid as u32;
        let name = entry_name(bytes, e);
        let kernel = is_kernel(pid, &name);
        let key = ProcKey { pid, start_ms: ticks_to_unix_ms(e.create) };
        let mut row = ProcRaw {
            key,
            parent: (pid != 0).then_some(e.parent as u32),
            state: state_of(threads, e.pid),
            cpu_user_ms: ticks_to_ms(e.user),
            cpu_system_ms: ticks_to_ms(e.kernel),
            rss: e.ws as u64,
            private: Some(e.private as u64),
            shared: Some((e.ws as u64).saturating_sub(e.private_ws.max(0) as u64)),
            virt: Some(e.virtual_size as u64),
            peak_rss: Some(e.peak_ws as u64),
            threads: e.threads,
            handles: Some(e.handles),
            read_bytes: Some(e.read_bytes.max(0) as u64),
            write_bytes: Some(e.write_bytes.max(0) as u64),
            read_ops: Some(e.read_ops.max(0) as u64),
            write_ops: Some(e.write_ops.max(0) as u64),
            page_faults: Some(u64::from(e.page_faults)),
            priority: priority_of(e.base_priority),
            nice: Some(e.base_priority),
            session: Some(e.session.to_string()),
            kernel,
            name,
            ..ProcRaw::default()
        };
        if let Some(g) = gpu {
            row.gpu_percent = g.percent.as_ref().map(|m| m.get(&pid).copied().unwrap_or(0.0));
            row.gpu_mem = g.mem.as_ref().map(|m| m.get(&pid).copied().unwrap_or(0));
        }
        self.seen.insert(key);
        if want.process_details && !kernel {
            if !self.cache.contains_key(&key) {
                let s = self.read_static(pid, devices);
                self.cache.insert(key, s);
            }
            if let Some(s) = self.cache.get(&key) {
                row.exe.clone_from(&s.exe);
                row.cmdline.clone_from(&s.cmdline);
                row.user.clone_from(&s.user);
                row.elevated = s.elevated;
                row.arch.clone_from(&s.arch);
            }
            if let Some(exe) = row.exe.as_deref() {
                row.description = self.description(exe, budget);
            }
        }
        row
    }

    fn name_of_sid(&mut self, sid: &[u8]) -> Option<String> {
        if let Some(known) = self.users.get(sid) {
            return known.clone();
        }
        let name = account_name(sid);
        self.users.insert(sid.to_vec(), name.clone());
        name
    }

    fn read_static(&mut self, pid: u32, devices: &mut Option<Vec<(String, String)>>) -> Static {
        let mut s = Static::default();
        // SAFETY: plain call; the result is checked by `Handle::new`.
        let opened = Handle::new(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) });
        if let Ok(h) = &opened {
            s.exe = image_path(h.raw(), &mut self.wide).map(PathBuf::from);
            s.cmdline = command_line_of(h.raw());
            s.arch = arch(h.raw());
            if let Some(token) = open_token(h.raw()) {
                s.elevated = elevated(token.raw());
                s.user = self.user_of(token.raw());
            }
        }
        if s.exe.is_none() {
            s.exe = protected_image_path(pid, &mut self.wide, devices).map(PathBuf::from);
        }
        s
    }

    fn user_of(&mut self, token: HANDLE) -> Option<String> {
        let sid = token_sid(token)?;
        self.name_of_sid(&sid)
    }

    fn description(&mut self, exe: &Path, budget: &mut u32) -> Option<String> {
        if let Some(known) = self.descriptions.get(exe) {
            return known.clone();
        }
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        // Reading a file from a network path may stall: not cheap, so not read.
        let d = if exe.as_os_str().as_encoded_bytes().starts_with(br"\\") { None } else { file_description(exe) };
        if self.descriptions.len() >= MAX_DESCRIPTIONS {
            self.descriptions.clear();
        }
        self.descriptions.insert(exe.to_path_buf(), d.clone());
        d
    }
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

fn arch(h: HANDLE) -> Option<String> {
    let mut process: IMAGE_FILE_MACHINE = 0;
    let mut native: IMAGE_FILE_MACHINE = 0;
    // SAFETY: open handle; valid out-pointers.
    if unsafe { IsWow64Process2(h, &mut process, &mut native) } == 0 {
        return None;
    }
    let machine = if process == 0 { native } else { process };
    Some(
        match machine {
            0x8664 => "x64",
            0xAA64 => "arm64",
            0x014C => "x86",
            0x01C4 => "arm",
            _ => return None,
        }
        .to_string(),
    )
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
        if e.pid as u32 == key.pid && (key.start_ms == 0 || ticks_to_unix_ms(e.create) == key.start_ms) {
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
    fn nt_paths_become_dos_paths() {
        let d = vec![(r"\Device\HarddiskVolume3".to_string(), "C:".to_string())];
        assert_eq!(dos_path(r"\Device\HarddiskVolume3\Windows\a.exe", &d), r"C:\Windows\a.exe");
        assert_eq!(dos_path(r"\Device\HarddiskVolume30\a.exe", &d), r"\Device\HarddiskVolume30\a.exe");
        assert_eq!(dos_path(r"\Device\Mup\srv\share\a.exe", &d), r"\\srv\share\a.exe");
    }
}
