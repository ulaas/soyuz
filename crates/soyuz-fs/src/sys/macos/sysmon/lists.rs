//! On-demand lists and process control: sockets through libproc, launchd plists as services
//! and startup items, `kill` and `setpriority`.

use std::ffi::c_int;
use std::path::{Path, PathBuf};

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::data::CFData;
use core_foundation::dictionary::CFDictionary;
use core_foundation::propertylist::{CFPropertyList, create_with_data, kCFPropertyListImmutable};
use core_foundation::string::CFString;

use super::procs::{all_pids, c_chars, fds, pid_path, proc_args, start_raw};
use crate::sysmon::parse::net::decode_in_sockinfo;
use crate::sysmon::parse::priority_to_nice;
use crate::sysmon::{
    Connection, ControlError, DetailsWant, OpenFile, ProcDetails, ProcKey, ProcPriority, ProcSignal, ProcState,
    ServiceInfo, ServiceOp, ServiceState, StartupItem, ThreadInfo,
};

/// `proc_pidfdinfo` flavors and `socket_info.soi_kind` of `sys/proc_info.h`.
const PROC_PIDFDVNODEPATHINFO: c_int = 2;
const PROC_PIDFDSOCKETINFO: c_int = 3;
const PROC_PIDLISTTHREADS: c_int = 6;
const SOCKINFO_IN: c_int = 1;
const SOCKINFO_TCP: c_int = 2;

/// `struct proc_fileinfo`.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct ProcFileInfo {
    fi_openflags: u32,
    fi_status: u32,
    fi_offset: i64,
    fi_type: i32,
    fi_guardflags: u32,
}

/// `struct socket_fdinfo`. The 232 bytes of `socket_info` before `soi_kind` (stat, addresses, flags,
/// buffers) are not used here, and the union after it is 528 bytes with 8-byte alignment.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct SocketFdInfo {
    pfi: ProcFileInfo,
    head: [u64; 29],
    soi_kind: c_int,
    rfu_1: u32,
    soi_proto: [u64; 66],
}

const _: () = assert!(size_of::<ProcFileInfo>() == 24);
// `PROC_PIDFDSOCKETINFO_SIZE` in the SDK header.
const _: () = assert!(size_of::<SocketFdInfo>() == 792);

/// `struct vnode_fdinfowithpath`.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct VnodeFdInfo {
    pfi: ProcFileInfo,
    pvip: libc::vnode_info_path,
}

// `PROC_PIDFDVNODEPATHINFO_SIZE` in the SDK header.
const _: () = assert!(size_of::<VnodeFdInfo>() == 1200);

/// A libproc per-descriptor struct, when the whole struct came back.
fn fd_info<T: Copy>(pid: c_int, fd: c_int, flavor: c_int) -> Option<T> {
    let mut v = std::mem::MaybeUninit::<T>::zeroed();
    let size = size_of::<T>() as c_int;
    // SAFETY: `v` is writable for `size` bytes.
    let got = unsafe { libc::proc_pidfdinfo(pid, fd, flavor, v.as_mut_ptr().cast(), size) };
    // SAFETY: the kernel filled the whole struct of plain C data, which also started zeroed.
    (got == size).then(|| unsafe { v.assume_init() })
}

pub fn connections() -> Vec<Connection> {
    let mut out = Vec::new();
    for pid in all_pids() {
        let Some(list) = fds(pid) else { continue };
        for f in list {
            if f.proc_fdtype != libc::PROX_FDTYPE_SOCKET as u32 {
                continue;
            }
            let Some(s) = fd_info::<SocketFdInfo>(pid, f.proc_fd, PROC_PIDFDSOCKETINFO) else { continue };
            let kind = s.soi_kind;
            if kind != SOCKINFO_IN && kind != SOCKINFO_TCP {
                continue;
            }
            // SAFETY: `proto` is plain initialized integers, and viewing them as bytes of the same
            // total size is always valid.
            let bytes =
                unsafe { std::slice::from_raw_parts(s.soi_proto.as_ptr().cast::<u8>(), size_of_val(&s.soi_proto)) };
            if let Some(mut c) = decode_in_sockinfo(bytes, kind == SOCKINFO_TCP) {
                c.pid = u32::try_from(pid).ok();
                out.push(c);
            }
        }
    }
    out
}

/// `(folder, label for the source column)` of launchd plists.
fn launchd_dirs() -> Vec<(PathBuf, String)> {
    let home = crate::path::home();
    vec![
        (home.join("Library/LaunchAgents"), "~/Library/LaunchAgents".to_owned()),
        (PathBuf::from("/Library/LaunchAgents"), "/Library/LaunchAgents".to_owned()),
        (PathBuf::from("/Library/LaunchDaemons"), "/Library/LaunchDaemons".to_owned()),
    ]
}

struct Plist {
    label: Option<String>,
    command: Option<String>,
    run_at_load: bool,
    disabled: bool,
}

fn read_plist(path: &Path) -> Option<Plist> {
    let bytes = std::fs::read(path).ok()?;
    let (raw, _) = create_with_data(CFData::from_buffer(&bytes), kCFPropertyListImmutable).ok()?;
    // SAFETY: `create_with_data` returned a new property list under the create rule.
    let plist = unsafe { CFPropertyList::wrap_under_create_rule(raw) };
    let dict = plist.downcast_into::<CFDictionary>()?;
    let get = |key: &str| {
        let k = CFString::new(key);
        let p = *dict.find(k.as_concrete_TypeRef().cast::<std::ffi::c_void>())?;
        // SAFETY: values of a live CF dictionary are valid CF objects; the get rule retains it.
        (!p.is_null()).then(|| unsafe { CFType::wrap_under_get_rule(p) })
    };
    let string = |key: &str| get(key)?.downcast::<CFString>().map(|s| s.to_string());
    let boolean = |key: &str| get(key).and_then(|v| v.downcast::<CFBoolean>()).is_some_and(bool::from);
    let arguments = get("ProgramArguments").and_then(|v| v.downcast::<CFArray>()).map(|arr| {
        arr.get_all_values()
            .into_iter()
            .filter(|p| !p.is_null())
            // SAFETY: elements of a live CFArray are valid CF objects; the get rule retains them.
            .map(|p| unsafe { CFType::wrap_under_get_rule(p) })
            .filter_map(|t| t.downcast::<CFString>().map(|s| s.to_string()))
            .collect::<Vec<_>>()
            .join(" ")
    });
    Some(Plist {
        label: string("Label"),
        command: arguments.filter(|a| !a.is_empty()).or_else(|| string("Program")),
        run_at_load: boolean("RunAtLoad"),
        disabled: boolean("Disabled"),
    })
}

fn plist_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<PathBuf> =
        rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "plist")).collect();
    v.sort();
    v
}

/// launchd jobs by plist. Whether one is loaded needs launchd's private interface: `Unknown`.
pub fn services() -> Vec<ServiceInfo> {
    let mut out = Vec::new();
    for (dir, _) in launchd_dirs() {
        for path in plist_files(&dir) {
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let plist = read_plist(&path);
            let disabled = plist.as_ref().is_some_and(|p| p.disabled);
            let run_at_load = plist.as_ref().is_some_and(|p| p.run_at_load);
            let name = plist.and_then(|p| p.label).unwrap_or(stem);
            out.push(ServiceInfo {
                display_name: name.clone(),
                name,
                description: String::new(),
                state: ServiceState::Unknown,
                start_mode: if disabled {
                    "disabled"
                } else if run_at_load {
                    "auto"
                } else {
                    "manual"
                }
                .to_owned(),
                pid: None,
            });
        }
    }
    out
}

pub fn service_control(_name: &str, _op: ServiceOp) -> Result<(), ControlError> {
    Err(ControlError::Unsupported)
}

pub fn startup_items() -> Vec<StartupItem> {
    let mut out = Vec::new();
    for (dir, source) in launchd_dirs() {
        for path in plist_files(&dir) {
            let Some(p) = read_plist(&path) else { continue };
            if !p.run_at_load {
                continue;
            }
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            out.push(StartupItem {
                name: p.label.unwrap_or(stem),
                command: p.command.unwrap_or_default(),
                source: source.clone(),
                enabled: !p.disabled,
            });
        }
    }
    out
}

/// The `pid_t` of `key`, when it still names the same process. PID 0 and values that turn
/// negative would address process groups or everything: they never reach a syscall.
fn verify(key: ProcKey) -> Result<libc::pid_t, ControlError> {
    let pid = libc::pid_t::try_from(key.pid).map_err(|_| ControlError::Gone)?;
    if pid <= 0 {
        return Err(ControlError::Gone);
    }
    if start_raw(pid) != Some(key.start_raw) {
        return Err(ControlError::Gone);
    }
    Ok(pid)
}

fn map_errno(e: std::io::Error) -> ControlError {
    match e.raw_os_error() {
        Some(libc::ESRCH) => ControlError::Gone,
        Some(libc::EPERM | libc::EACCES) => ControlError::Denied,
        _ => ControlError::Other(e.to_string()),
    }
}

pub fn signal(key: ProcKey, sig: ProcSignal) -> Result<(), ControlError> {
    let pid = verify(key)?;
    let s = match sig {
        ProcSignal::Terminate => libc::SIGTERM,
        ProcSignal::Kill => libc::SIGKILL,
        ProcSignal::Suspend => libc::SIGSTOP,
        ProcSignal::Resume => libc::SIGCONT,
        ProcSignal::Hangup => libc::SIGHUP,
        ProcSignal::Interrupt => libc::SIGINT,
    };
    // SAFETY: `kill` takes plain integers; `verify` guarantees `pid` is a single positive PID.
    if unsafe { libc::kill(pid, s) } == 0 { Ok(()) } else { Err(map_errno(std::io::Error::last_os_error())) }
}

pub fn set_priority(key: ProcKey, p: ProcPriority) -> Result<(), ControlError> {
    let pid = verify(key)?;
    // SAFETY: plain integers; `pid` is a single positive PID.
    let rc = unsafe { libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, priority_to_nice(p)) };
    if rc == 0 { Ok(()) } else { Err(map_errno(std::io::Error::last_os_error())) }
}

pub fn affinity(_key: ProcKey) -> Result<Vec<u32>, ControlError> {
    Err(ControlError::Unsupported)
}

pub fn set_affinity(_key: ProcKey, _cpus: &[u32]) -> Result<(), ControlError> {
    Err(ControlError::Unsupported)
}

fn thread_state(run_state: i32) -> ProcState {
    match run_state {
        libc::TH_STATE_RUNNING => ProcState::Running,
        libc::TH_STATE_STOPPED | libc::TH_STATE_HALTED => ProcState::Stopped,
        libc::TH_STATE_WAITING => ProcState::Sleeping,
        libc::TH_STATE_UNINTERRUPTIBLE => ProcState::DiskWait,
        _ => ProcState::Unknown,
    }
}

fn threads(pid: c_int) -> Option<Vec<ThreadInfo>> {
    let mut ids = vec![0u64; 4096];
    let bytes = (ids.len() * size_of::<u64>()) as c_int;
    // SAFETY: `ids` is writable for `bytes` bytes.
    let got = unsafe { libc::proc_pidinfo(pid, PROC_PIDLISTTHREADS, 0, ids.as_mut_ptr().cast(), bytes) };
    let got = usize::try_from(got).ok().filter(|g| *g > 0)?;
    ids.truncate(got / size_of::<u64>());
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let mut info = std::mem::MaybeUninit::<libc::proc_threadinfo>::zeroed();
        let size = size_of::<libc::proc_threadinfo>() as c_int;
        // SAFETY: `info` is writable for `size` bytes.
        let got = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTHREADINFO, id, info.as_mut_ptr().cast(), size) };
        if got != size {
            continue;
        }
        // SAFETY: the kernel filled the whole struct of plain C data, which also started zeroed.
        let t = unsafe { info.assume_init() };
        out.push(ThreadInfo {
            // Thread handles are 64-bit on macOS; their low bits tell threads apart.
            tid: id as u32,
            name: c_chars(&t.pth_name),
            state: thread_state(t.pth_run_state),
            cpu_ms: t.pth_user_time.saturating_add(t.pth_system_time) / 1_000_000,
            priority: Some(t.pth_curpri),
        });
    }
    Some(out)
}

/// The executable, working folder and command line always; the lists only when `want` asks.
/// Listing modules would need a task port, so `modules` stays empty on macOS.
pub fn process_details(key: ProcKey, want: DetailsWant) -> Result<ProcDetails, ControlError> {
    let pid = verify(key)?;
    let mut d = ProcDetails { exe: pid_path(pid), ..ProcDetails::default() };
    match proc_args(pid) {
        Some(a) => {
            d.cmdline = Some(a.args.join(" ")).filter(|c| !c.is_empty());
            if want.environment {
                d.environment = a.env;
            }
            if d.exe.is_none() && !a.exe.is_empty() {
                d.exe = Some(PathBuf::from(a.exe));
            }
        }
        None => d.limited = true,
    }
    if want.threads {
        match threads(pid) {
            Some(t) => d.threads = t,
            None => d.limited = true,
        }
    }
    let mut vn = std::mem::MaybeUninit::<libc::proc_vnodepathinfo>::zeroed();
    let size = size_of::<libc::proc_vnodepathinfo>() as c_int;
    // SAFETY: `vn` is writable for `size` bytes.
    let got = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDVNODEPATHINFO, 0, vn.as_mut_ptr().cast(), size) };
    if got == size {
        // SAFETY: the kernel filled the whole struct of plain C data, which also started zeroed.
        let vn = unsafe { vn.assume_init() };
        let cwd = c_chars(vn.pvi_cdir.vip_path.as_flattened());
        if !cwd.is_empty() {
            d.cwd = Some(PathBuf::from(cwd));
        }
    } else {
        d.limited = true;
    }
    if want.open_files {
        match fds(pid) {
            Some(list) => {
                for f in list {
                    let path = if f.proc_fdtype == libc::PROX_FDTYPE_VNODE as u32 {
                        fd_info::<VnodeFdInfo>(pid, f.proc_fd, PROC_PIDFDVNODEPATHINFO)
                            .map(|v| c_chars(v.pvip.vip_path.as_flattened()))
                    } else {
                        None
                    };
                    let label = path.filter(|p| !p.is_empty()).unwrap_or_else(|| fd_kind(f.proc_fdtype).to_owned());
                    d.open_files.push(OpenFile { fd: u32::try_from(f.proc_fd).ok(), path: label });
                }
            }
            None => d.limited = true,
        }
    }
    Ok(d)
}

fn fd_kind(t: u32) -> &'static str {
    match t as c_int {
        libc::PROX_FDTYPE_SOCKET => "socket",
        libc::PROX_FDTYPE_PIPE => "pipe",
        libc::PROX_FDTYPE_KQUEUE => "kqueue",
        libc::PROX_FDTYPE_FSEVENTS => "fsevents",
        libc::PROX_FDTYPE_PSHM => "shared memory",
        libc::PROX_FDTYPE_PSEM => "semaphore",
        _ => "other",
    }
}
