//! Acting on processes, each call first checking that the PID still is the process the
//! caller saw, and on-demand process details.

use std::mem::size_of;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_BAD_LENGTH, ERROR_INVALID_PARAMETER, FILETIME, GetLastError, HANDLE, HWND, LPARAM,
    STILL_ACTIVE,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32,
};
use windows_sys::Win32::System::Threading::{
    ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, GetExitCodeProcess, GetProcessAffinityMask,
    GetProcessTimes, GetThreadDescription, HIGH_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
    OpenProcess, OpenThread, PROCESS_ACCESS_RIGHTS, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SET_INFORMATION, PROCESS_SUSPEND_RESUME, PROCESS_TERMINATE, QueryFullProcessImageNameW,
    REALTIME_PRIORITY_CLASS, SetPriorityClass, SetProcessAffinityMask, THREAD_QUERY_LIMITED_INFORMATION,
    TerminateProcess,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
};
use windows_sys::core::{BOOL, PWSTR};

use super::super::{Handle, ft_u64};
use super::{procs, ticks_to_unix_ms};
use crate::sysmon::{ControlError, ModuleInfo, ProcDetails, ProcKey, ProcPriority, ProcSignal};

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtSuspendProcess(process: HANDLE) -> i32;
    fn NtResumeProcess(process: HANDLE) -> i32;
}

/// Most modules listed for one process.
const MAX_MODULES: usize = 8192;

fn os_error(code: u32) -> ControlError {
    match code {
        ERROR_ACCESS_DENIED => ControlError::Denied,
        // OpenProcess's answer for a PID that does not exist.
        ERROR_INVALID_PARAMETER => ControlError::Gone,
        c => ControlError::Other(std::io::Error::from_raw_os_error(c as i32).to_string()),
    }
}

fn last_error() -> ControlError {
    // SAFETY: no arguments.
    os_error(unsafe { GetLastError() })
}

/// Open `key`'s process, refusing a PID that now belongs to another process or has exited.
fn open_verified(key: ProcKey, access: PROCESS_ACCESS_RIGHTS) -> Result<Handle, ControlError> {
    // SAFETY: plain call; the result is checked by `Handle::new`.
    let raw = unsafe { OpenProcess(access | PROCESS_QUERY_LIMITED_INFORMATION, 0, key.pid) };
    if raw.is_null() {
        return Err(last_error());
    }
    let h = Handle::new(raw).map_err(|_| ControlError::Gone)?;
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: open handle; valid out-pointers.
    if unsafe { GetProcessTimes(h.raw(), &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        return Err(last_error());
    }
    if key.start_ms != 0 && ticks_to_unix_ms(ft_u64(&created) as i64) != key.start_ms {
        return Err(ControlError::Gone);
    }
    let mut code = 0u32;
    // SAFETY: open handle; valid out-pointer.
    if unsafe { GetExitCodeProcess(h.raw(), &mut code) } != 0 && code != STILL_ACTIVE as u32 {
        return Err(ControlError::Gone);
    }
    Ok(h)
}

/// What the close-message walk found.
struct CloseWalk {
    pid: u32,
    posted: u32,
    denied: u32,
}

unsafe extern "system" fn close_visible(hwnd: HWND, data: LPARAM) -> BOOL {
    // SAFETY: `data` is the pointer to the `CloseWalk` that `post_close` keeps alive for the
    // whole synchronous EnumWindows call.
    let walk = unsafe { &mut *(data as *mut CloseWalk) };
    let mut pid = 0u32;
    // SAFETY: `hwnd` comes from the enumeration; a window that just closed only makes these
    // calls fail, which reads as "not this process's visible window".
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == walk.pid && IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER).is_null() {
            if PostMessageW(hwnd, WM_CLOSE, 0, 0) != 0 {
                walk.posted += 1;
            } else {
                walk.denied += 1;
            }
        }
    }
    1
}

/// Ask a process to exit by closing its top-level windows.
fn post_close(pid: u32) -> Result<(), ControlError> {
    let mut walk = CloseWalk { pid, posted: 0, denied: 0 };
    // SAFETY: the callback only uses the pointer during this synchronous call.
    unsafe {
        EnumWindows(Some(close_visible), (&raw mut walk) as LPARAM);
    }
    match (walk.posted, walk.denied) {
        (0, 0) => Err(ControlError::Unsupported),
        (0, _) => Err(ControlError::Denied),
        _ => Ok(()),
    }
}

/// Terminate, kill, suspend, resume.
pub fn signal(key: ProcKey, sig: ProcSignal) -> Result<(), ControlError> {
    match sig {
        ProcSignal::Hangup | ProcSignal::Interrupt => Err(ControlError::Unsupported),
        ProcSignal::Terminate => {
            open_verified(key, 0)?;
            post_close(key.pid)
        }
        ProcSignal::Kill => {
            let h = open_verified(key, PROCESS_TERMINATE)?;
            // SAFETY: open handle with terminate access; verified to be `key`'s process.
            if unsafe { TerminateProcess(h.raw(), 1) } == 0 {
                return Err(last_error());
            }
            Ok(())
        }
        ProcSignal::Suspend | ProcSignal::Resume => {
            let h = open_verified(key, PROCESS_SUSPEND_RESUME)?;
            // SAFETY: open handle with suspend/resume access; verified to be `key`'s process.
            let st = unsafe {
                if sig == ProcSignal::Suspend { NtSuspendProcess(h.raw()) } else { NtResumeProcess(h.raw()) }
            };
            if st < 0 {
                return Err(ControlError::Other(format!("status {:#010x}", st as u32)));
            }
            Ok(())
        }
    }
}

fn priority_class(p: ProcPriority) -> u32 {
    match p {
        ProcPriority::Idle => IDLE_PRIORITY_CLASS,
        ProcPriority::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
        ProcPriority::Normal => NORMAL_PRIORITY_CLASS,
        ProcPriority::AboveNormal => ABOVE_NORMAL_PRIORITY_CLASS,
        ProcPriority::High => HIGH_PRIORITY_CLASS,
        ProcPriority::Realtime => REALTIME_PRIORITY_CLASS,
    }
}

/// Change the priority class.
pub fn set_priority(key: ProcKey, p: ProcPriority) -> Result<(), ControlError> {
    let h = open_verified(key, PROCESS_SET_INFORMATION)?;
    // SAFETY: open handle with set-information access.
    if unsafe { SetPriorityClass(h.raw(), priority_class(p)) } == 0 {
        return Err(last_error());
    }
    Ok(())
}

fn cpus_of_mask(mask: usize) -> Vec<u32> {
    (0..usize::BITS).filter(|i| mask >> i & 1 != 0).collect()
}

fn mask_of_cpus(cpus: &[u32]) -> Option<usize> {
    let mut mask = 0usize;
    for &c in cpus {
        if c >= usize::BITS {
            return None;
        }
        mask |= 1 << c;
    }
    (mask != 0).then_some(mask)
}

/// The logical CPUs a process may run on.
pub fn affinity(key: ProcKey) -> Result<Vec<u32>, ControlError> {
    let h = open_verified(key, 0)?;
    let (mut process, mut system) = (0usize, 0usize);
    // SAFETY: open handle; valid out-pointers.
    if unsafe { GetProcessAffinityMask(h.raw(), &mut process, &mut system) } == 0 {
        return Err(last_error());
    }
    Ok(cpus_of_mask(process))
}

/// Restrict a process to `cpus`.
pub fn set_affinity(key: ProcKey, cpus: &[u32]) -> Result<(), ControlError> {
    let Some(mask) = mask_of_cpus(cpus) else {
        return Err(ControlError::Other("no usable CPU in the set".to_string()));
    };
    let h = open_verified(key, PROCESS_SET_INFORMATION)?;
    // SAFETY: open handle with set-information access.
    if unsafe { SetProcessAffinityMask(h.raw(), mask) } == 0 {
        // SAFETY: no arguments.
        return Err(match unsafe { GetLastError() } {
            // The process exists; here this means CPUs outside the system's set.
            ERROR_INVALID_PARAMETER => ControlError::Other("a CPU in the set does not exist".to_string()),
            c => os_error(c),
        });
    }
    Ok(())
}

fn thread_name(tid: u32) -> String {
    // SAFETY: plain call; the result is checked by `Handle::new`.
    let Ok(h) = Handle::new(unsafe { OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, tid) }) else {
        return String::new();
    };
    let mut p: PWSTR = std::ptr::null_mut();
    // SAFETY: open handle; valid out-pointer. A success returns a LocalAlloc'd string.
    let hr = unsafe { GetThreadDescription(h.raw(), &mut p) };
    if hr < 0 || p.is_null() {
        return String::new();
    }
    // SAFETY: on success `p` is a NUL-terminated string that we free once below.
    unsafe {
        let mut n = 0usize;
        while n < 4096 && *p.add(n) != 0 {
            n += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
        windows_sys::Win32::Foundation::LocalFree(p.cast());
        s
    }
}

/// The modules loaded in a process. `None`: not allowed to look.
fn modules(pid: u32) -> Option<Vec<ModuleInfo>> {
    let flags = TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32;
    let mut snapshot = None;
    // The snapshot call can fail with a bad-length error while the process loads a module.
    for _ in 0..4 {
        // SAFETY: plain call; the result is checked by `Handle::new`.
        match Handle::new(unsafe { CreateToolhelp32Snapshot(flags, pid) }) {
            Ok(h) => {
                snapshot = Some(h);
                break;
            }
            Err(e) if e.raw_os_error() == Some(ERROR_BAD_LENGTH as i32) => {}
            Err(_) => return None,
        }
    }
    let snapshot = snapshot?;
    let mut out = Vec::new();
    // SAFETY: plain struct of integers, pointers and arrays; zero is a valid start.
    let mut entry: MODULEENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<MODULEENTRY32W>() as u32;
    // SAFETY: open snapshot; `entry` is writable with its size set.
    let mut more = unsafe { Module32FirstW(snapshot.raw(), &mut entry) } != 0;
    while more && out.len() < MAX_MODULES {
        out.push(ModuleInfo {
            path: super::super::from_wide_nul(&entry.szExePath).to_string_lossy().into_owned(),
            size: u64::from(entry.modBaseSize),
            base: entry.modBaseAddr as u64,
        });
        // SAFETY: as above.
        more = unsafe { Module32NextW(snapshot.raw(), &mut entry) } != 0;
    }
    Some(out)
}

fn image_path_of(h: &Handle) -> Option<PathBuf> {
    let mut wide = vec![0u16; 32_768];
    let mut len = wide.len() as u32;
    // SAFETY: open handle; output buffer of `len` characters.
    let ok = unsafe { QueryFullProcessImageNameW(h.raw(), PROCESS_NAME_WIN32, wide.as_mut_ptr(), &mut len) };
    (ok != 0 && len > 0).then(|| PathBuf::from(String::from_utf16_lossy(&wide[..(len as usize).min(wide.len())])))
}

/// On-demand details: threads and modules. The working folder and environment would mean
/// reading the other process's memory, so they stay empty.
pub fn process_details(key: ProcKey) -> Result<ProcDetails, ControlError> {
    let Some(mut threads) = procs::threads_of_process(key) else { return Err(ControlError::Gone) };
    for t in &mut threads {
        t.name = thread_name(t.tid);
    }
    let mut d = ProcDetails { threads, ..ProcDetails::default() };
    match open_verified(key, 0) {
        Ok(h) => {
            d.exe = image_path_of(&h);
            d.cmdline = procs::command_line_of(h.raw());
        }
        // The idle process cannot be opened at all; for others this means it just exited.
        Err(ControlError::Gone) if key.pid != 0 => return Err(ControlError::Gone),
        Err(_) => d.limited = true,
    }
    match modules(key.pid) {
        Some(m) => d.modules = m,
        None => d.limited = true,
    }
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affinity_masks() {
        assert_eq!(cpus_of_mask(0b1010), [1, 3]);
        assert_eq!(cpus_of_mask(0), Vec::<u32>::new());
        assert_eq!(mask_of_cpus(&[1, 3]), Some(0b1010));
        assert_eq!(mask_of_cpus(&[]), None);
        assert_eq!(mask_of_cpus(&[usize::BITS]), None);
    }

    #[test]
    fn os_errors_map() {
        assert_eq!(os_error(ERROR_ACCESS_DENIED), ControlError::Denied);
        assert_eq!(os_error(ERROR_INVALID_PARAMETER), ControlError::Gone);
        assert!(matches!(os_error(2), ControlError::Other(_)));
    }
}
