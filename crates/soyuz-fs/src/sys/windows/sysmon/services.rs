//! The service manager (advapi32): list, start, stop. Never elevates: a denied control is
//! reported as `Denied`.

use std::mem::size_of;
use std::ptr::null;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_MORE_DATA, ERROR_SERVICE_ALREADY_RUNNING, ERROR_SERVICE_NEVER_STARTED,
    ERROR_SERVICE_NOT_ACTIVE, GetLastError,
};
use windows_sys::Win32::System::Services::{
    CloseServiceHandle, ControlService, ENUM_SERVICE_STATUS_PROCESSW, EnumServicesStatusExW, OpenSCManagerW,
    OpenServiceW, QUERY_SERVICE_CONFIGW, QueryServiceConfig2W, QueryServiceConfigW, QueryServiceStatus,
    SC_ENUM_PROCESS_INFO, SC_HANDLE, SC_MANAGER_CONNECT, SC_MANAGER_ENUMERATE_SERVICE, SERVICE_AUTO_START,
    SERVICE_BOOT_START, SERVICE_CONFIG_DELAYED_AUTO_START_INFO, SERVICE_CONFIG_DESCRIPTION, SERVICE_CONTINUE_PENDING,
    SERVICE_CONTROL_STOP, SERVICE_DELAYED_AUTO_START_INFO, SERVICE_DEMAND_START, SERVICE_DESCRIPTIONW,
    SERVICE_DISABLED, SERVICE_PAUSE_PENDING, SERVICE_PAUSED, SERVICE_QUERY_CONFIG, SERVICE_QUERY_STATUS,
    SERVICE_RUNNING, SERVICE_START, SERVICE_START_PENDING, SERVICE_STATE_ALL, SERVICE_STATUS, SERVICE_STOP,
    SERVICE_STOP_PENDING, SERVICE_STOPPED, SERVICE_SYSTEM_START, SERVICE_WIN32, StartServiceW,
};
use windows_sys::core::PWSTR;

use super::super::wide_str;
use crate::sysmon::{ControlError, ServiceInfo, ServiceOp, ServiceState};

const MAX_ENUM_BYTES: usize = 32 << 20;
/// How long a restart waits for the service to stop.
const STOP_TIMEOUT: Duration = Duration::from_secs(20);

/// A service manager or service handle.
struct Sc(SC_HANDLE);

impl Sc {
    fn manager(access: u32) -> Result<Sc, ControlError> {
        // SAFETY: local machine, default database; the result is checked.
        let h = unsafe { OpenSCManagerW(null(), null(), access) };
        if h.is_null() { Err(last_control_error()) } else { Ok(Sc(h)) }
    }

    fn service(&self, name: &[u16], access: u32) -> Result<Sc, ControlError> {
        // SAFETY: open manager; NUL-terminated name; the result is checked.
        let h = unsafe { OpenServiceW(self.0, name.as_ptr(), access) };
        if h.is_null() { Err(last_control_error()) } else { Ok(Sc(h)) }
    }
}

impl Drop for Sc {
    fn drop(&mut self) {
        // SAFETY: handle opened above, closed once.
        unsafe {
            CloseServiceHandle(self.0);
        }
    }
}

fn last_control_error() -> ControlError {
    // SAFETY: no arguments.
    match unsafe { GetLastError() } {
        ERROR_ACCESS_DENIED => ControlError::Denied,
        code => ControlError::Other(std::io::Error::from_raw_os_error(code as i32).to_string()),
    }
}

/// The NUL-terminated text at `p` (null reads as empty), bounded to a service name's limits.
///
/// # Safety
/// `p` is null or points to a NUL-terminated UTF-16 string.
unsafe fn pwstr_text(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0usize;
    // SAFETY: per this function's contract the text up to its NUL is readable.
    unsafe {
        while n < 1 << 16 && *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
    }
}

fn state_of(current: u32, exit_code: u32) -> ServiceState {
    match current {
        SERVICE_STOPPED if exit_code != 0 && exit_code != ERROR_SERVICE_NEVER_STARTED => ServiceState::Failed,
        SERVICE_STOPPED => ServiceState::Stopped,
        SERVICE_START_PENDING | SERVICE_CONTINUE_PENDING => ServiceState::Starting,
        SERVICE_STOP_PENDING | SERVICE_PAUSE_PENDING => ServiceState::Stopping,
        SERVICE_RUNNING => ServiceState::Running,
        SERVICE_PAUSED => ServiceState::Paused,
        _ => ServiceState::Unknown,
    }
}

fn start_mode_of(start: u32, delayed: bool) -> &'static str {
    match start {
        SERVICE_BOOT_START => "boot",
        SERVICE_SYSTEM_START => "system",
        SERVICE_AUTO_START if delayed => "auto (delayed)",
        SERVICE_AUTO_START => "auto",
        SERVICE_DEMAND_START => "manual",
        SERVICE_DISABLED => "disabled",
        _ => "",
    }
}

/// Installed services.
pub fn services() -> Vec<ServiceInfo> {
    let Ok(manager) = Sc::manager(SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE) else { return Vec::new() };
    let mut buf = vec![0u64; (256 << 10) / 8];
    let mut count = 0u32;
    let mut ok = false;
    for _ in 0..4 {
        let mut needed = 0u32;
        count = 0;
        let mut resume = 0u32;
        // SAFETY: open manager; `buf` is writable for the size we pass, 8-aligned for the
        // records; valid out-pointers; no group filter.
        let done = unsafe {
            EnumServicesStatusExW(
                manager.0,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                buf.as_mut_ptr().cast(),
                (buf.len() * 8) as u32,
                &mut needed,
                &mut count,
                &mut resume,
                null(),
            )
        };
        if done != 0 {
            ok = true;
            break;
        }
        // SAFETY: no arguments.
        if unsafe { GetLastError() } != ERROR_MORE_DATA {
            return Vec::new();
        }
        let want = buf.len() * 8 + needed as usize + (16 << 10);
        if want > MAX_ENUM_BYTES {
            return Vec::new();
        }
        buf.resize(want.div_ceil(8), 0);
    }
    if !ok {
        return Vec::new();
    }
    let max = buf.len() * 8 / size_of::<ENUM_SERVICE_STATUS_PROCESSW>();
    let n = (count as usize).min(max);
    let mut out = Vec::with_capacity(n);
    let records = buf.as_ptr().cast::<ENUM_SERVICE_STATUS_PROCESSW>();
    for i in 0..n {
        // SAFETY: the call wrote `n` records at the start of `buf`; their string pointers
        // point further into `buf`, which lives through this loop.
        let (name, display, status) = unsafe {
            let r = &*records.add(i);
            (pwstr_text(r.lpServiceName), pwstr_text(r.lpDisplayName), r.ServiceStatusProcess)
        };
        let (start_mode, description) = config_of(&manager, &name);
        out.push(ServiceInfo {
            display_name: display,
            description,
            state: state_of(status.dwCurrentState, status.dwWin32ExitCode),
            start_mode: start_mode.to_string(),
            pid: (status.dwProcessId != 0).then_some(status.dwProcessId),
            name,
        });
    }
    out.sort_by_cached_key(|s| s.display_name.to_lowercase());
    out
}

/// Start mode and description; a service we may not open reads as blank.
fn config_of(manager: &Sc, name: &str) -> (&'static str, String) {
    let Ok(service) = manager.service(&wide_str(name), SERVICE_QUERY_CONFIG) else { return ("", String::new()) };
    // Documented maximum for each of these structs: 8 KB.
    let mut buf = vec![0u64; 8192 / 8];
    let bytes = (buf.len() * 8) as u32;
    let mut needed = 0u32;
    // SAFETY: open service; `buf` is 8-aligned and writable for `bytes` bytes.
    let ok = unsafe { QueryServiceConfigW(service.0, buf.as_mut_ptr().cast(), bytes, &mut needed) };
    let start = if ok != 0 {
        // SAFETY: on success `buf` starts with a QUERY_SERVICE_CONFIGW.
        Some(unsafe { (*buf.as_ptr().cast::<QUERY_SERVICE_CONFIGW>()).dwStartType })
    } else {
        None
    };
    let mut delayed = SERVICE_DELAYED_AUTO_START_INFO { fDelayedAutostart: 0 };
    if start == Some(SERVICE_AUTO_START) {
        // SAFETY: open service; `delayed` is writable for the size we pass.
        let ok = unsafe {
            QueryServiceConfig2W(
                service.0,
                SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
                (&raw mut delayed).cast(),
                size_of::<SERVICE_DELAYED_AUTO_START_INFO>() as u32,
                &mut needed,
            )
        };
        if ok == 0 {
            delayed.fDelayedAutostart = 0;
        }
    }
    let mode = start.map_or("", |s| start_mode_of(s, delayed.fDelayedAutostart != 0));
    // SAFETY: open service; `buf` is 8-aligned and writable for `bytes` bytes.
    let ok = unsafe {
        QueryServiceConfig2W(service.0, SERVICE_CONFIG_DESCRIPTION, buf.as_mut_ptr().cast(), bytes, &mut needed)
    };
    let description = if ok != 0 {
        // SAFETY: on success `buf` starts with a SERVICE_DESCRIPTIONW whose pointer is null or
        // points to NUL-terminated text inside `buf`.
        unsafe { pwstr_text((*buf.as_ptr().cast::<SERVICE_DESCRIPTIONW>()).lpDescription) }
    } else {
        String::new()
    };
    (mode, description)
}

fn empty_status() -> SERVICE_STATUS {
    SERVICE_STATUS {
        dwServiceType: 0,
        dwCurrentState: 0,
        dwControlsAccepted: 0,
        dwWin32ExitCode: 0,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: 0,
    }
}

fn start(manager: &Sc, name: &[u16]) -> Result<(), ControlError> {
    let service = manager.service(name, SERVICE_START)?;
    // SAFETY: open service; no arguments.
    if unsafe { StartServiceW(service.0, 0, null()) } != 0 {
        return Ok(());
    }
    // SAFETY: no arguments.
    if unsafe { GetLastError() } == ERROR_SERVICE_ALREADY_RUNNING {
        return Ok(());
    }
    Err(last_control_error())
}

fn stop(manager: &Sc, name: &[u16], access: u32) -> Result<Sc, ControlError> {
    let service = manager.service(name, SERVICE_STOP | access)?;
    let mut status = empty_status();
    // SAFETY: open service; `status` is writable.
    if unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &mut status) } != 0 {
        return Ok(service);
    }
    // SAFETY: no arguments.
    if unsafe { GetLastError() } == ERROR_SERVICE_NOT_ACTIVE {
        return Ok(service);
    }
    Err(last_control_error())
}

/// Start, stop or restart a service.
pub fn service_control(name: &str, op: ServiceOp) -> Result<(), ControlError> {
    let manager = Sc::manager(SC_MANAGER_CONNECT)?;
    let wide = wide_str(name);
    match op {
        ServiceOp::Start => start(&manager, &wide),
        ServiceOp::Stop => stop(&manager, &wide, 0).map(|_| ()),
        ServiceOp::Restart => {
            let service = stop(&manager, &wide, SERVICE_QUERY_STATUS)?;
            let began = Instant::now();
            loop {
                let mut status = empty_status();
                // SAFETY: open service; `status` is writable.
                if unsafe { QueryServiceStatus(service.0, &mut status) } == 0 {
                    return Err(last_control_error());
                }
                if status.dwCurrentState == SERVICE_STOPPED {
                    break;
                }
                if began.elapsed() >= STOP_TIMEOUT {
                    return Err(ControlError::Other("the service did not stop in time".to_string()));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            start(&manager, &wide)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_and_modes() {
        assert_eq!(state_of(SERVICE_RUNNING, 0), ServiceState::Running);
        assert_eq!(state_of(SERVICE_STOPPED, 0), ServiceState::Stopped);
        assert_eq!(state_of(SERVICE_STOPPED, ERROR_SERVICE_NEVER_STARTED), ServiceState::Stopped);
        assert_eq!(state_of(SERVICE_STOPPED, 5), ServiceState::Failed);
        assert_eq!(state_of(99, 0), ServiceState::Unknown);
        assert_eq!(start_mode_of(SERVICE_AUTO_START, true), "auto (delayed)");
        assert_eq!(start_mode_of(SERVICE_DEMAND_START, false), "manual");
        assert_eq!(start_mode_of(SERVICE_DISABLED, false), "disabled");
    }

    #[test]
    fn null_text_is_empty() {
        // SAFETY: null is allowed by the contract.
        assert_eq!(unsafe { pwstr_text(std::ptr::null_mut()) }, "");
    }
}
