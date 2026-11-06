//! Signals, niceness and CPU affinity. Every call first checks that the PID still belongs to
//! the process the caller means.

use std::io;

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use super::read_text;
use crate::sysmon::parse::pid::parse_stat;
use crate::sysmon::parse::priority_to_nice;
use crate::sysmon::{ControlError, ProcKey, ProcPriority, ProcSignal};

/// The `pid_t` of `key`. PID 0 and values that turn negative would address process groups or
/// everything: they never reach a syscall.
fn checked_pid(key: ProcKey) -> Result<libc::pid_t, ControlError> {
    libc::pid_t::try_from(key.pid).ok().filter(|p| *p > 0).ok_or(ControlError::Gone)
}

/// The PID still names the process of `key`: `stat`'s raw `starttime` is the same.
fn same_process(pid: libc::pid_t, key: ProcKey) -> Result<(), ControlError> {
    let text = read_text(&format!("/proc/{pid}/stat")).ok_or(ControlError::Gone)?;
    let st = parse_stat(&text).ok_or(ControlError::Gone)?;
    if st.starttime == key.start_raw { Ok(()) } else { Err(ControlError::Gone) }
}

/// The `pid_t` of `key`, when it still names the same process.
pub(super) fn verify(key: ProcKey) -> Result<libc::pid_t, ControlError> {
    let pid = checked_pid(key)?;
    same_process(pid, key)?;
    Ok(pid)
}

fn pidfd_open(pid: libc::pid_t) -> io::Result<OwnedFd> {
    // SAFETY: plain integer arguments; no memory is passed.
    let rc = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: on success the syscall returned a new descriptor that nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(rc as i32) })
}

fn map_errno(e: io::Error) -> ControlError {
    match e.raw_os_error() {
        Some(libc::ESRCH) => ControlError::Gone,
        Some(libc::EPERM | libc::EACCES) => ControlError::Denied,
        _ => ControlError::Other(e.to_string()),
    }
}

/// A pidfd pins the process, so the identity check made through it cannot be overtaken by PID
/// reuse. Kernels before 5.3 (or a seccomp filter) have no pidfds: check, then plain `kill`.
pub fn signal(key: ProcKey, sig: ProcSignal) -> Result<(), ControlError> {
    let pid = checked_pid(key)?;
    let s = match sig {
        ProcSignal::Terminate => libc::SIGTERM,
        ProcSignal::Kill => libc::SIGKILL,
        ProcSignal::Suspend => libc::SIGSTOP,
        ProcSignal::Resume => libc::SIGCONT,
        ProcSignal::Hangup => libc::SIGHUP,
        ProcSignal::Interrupt => libc::SIGINT,
    };
    match pidfd_open(pid) {
        Ok(fd) => {
            same_process(pid, key)?;
            let no_info = std::ptr::null::<libc::siginfo_t>();
            // SAFETY: `fd` is a live pidfd, the signal is a plain integer, a null siginfo is the
            // documented "as kill would" form, and flags must be 0.
            let rc = unsafe { libc::syscall(libc::SYS_pidfd_send_signal, fd.as_raw_fd(), s, no_info, 0) };
            if rc == 0 {
                return Ok(());
            }
            let e = io::Error::last_os_error();
            if e.raw_os_error() != Some(libc::ENOSYS) {
                return Err(map_errno(e));
            }
        }
        Err(e) if e.raw_os_error() == Some(libc::ESRCH) => return Err(ControlError::Gone),
        Err(_) => {}
    }
    same_process(pid, key)?;
    // SAFETY: `kill` takes plain integers; `checked_pid` guarantees a single positive PID.
    if unsafe { libc::kill(pid, s) } == 0 { Ok(()) } else { Err(map_errno(io::Error::last_os_error())) }
}

/// The thread ids of a process.
fn tasks(pid: libc::pid_t) -> Vec<libc::pid_t> {
    let mut out: Vec<libc::pid_t> = std::fs::read_dir(format!("/proc/{pid}/task"))
        .map(|d| d.flatten().filter_map(|e| e.file_name().to_str()?.parse().ok()).collect())
        .unwrap_or_default();
    if out.is_empty() {
        out.push(pid);
    }
    out
}

/// Niceness is per thread on Linux, so a whole process means every one of its threads.
pub fn set_priority(key: ProcKey, p: ProcPriority) -> Result<(), ControlError> {
    let pid = verify(key)?;
    let nice = priority_to_nice(p);
    let mut result = Ok(());
    for tid in tasks(pid) {
        // SAFETY: plain integers; `tid` is a positive thread id of the verified process.
        let rc = unsafe { libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, nice) };
        if rc != 0 {
            let e = io::Error::last_os_error();
            // A thread that has just exited is fine; anything else is the answer.
            if e.raw_os_error() != Some(libc::ESRCH) {
                result = Err(map_errno(e));
            }
        }
    }
    result
}

pub fn affinity(key: ProcKey) -> Result<Vec<u32>, ControlError> {
    let pid = verify(key)?;
    // SAFETY: an all-zero `cpu_set_t` is a valid empty set.
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    // SAFETY: `set` is valid for writes of the size passed.
    let rc = unsafe { libc::sched_getaffinity(pid, size_of::<libc::cpu_set_t>(), &raw mut set) };
    if rc != 0 {
        return Err(map_errno(io::Error::last_os_error()));
    }
    let bits = (size_of::<libc::cpu_set_t>() * 8) as u32;
    // SAFETY: `CPU_ISSET` only reads `set`, with an index inside the set.
    Ok((0..bits).filter(|i| unsafe { libc::CPU_ISSET(*i as usize, &set) }).collect())
}

/// Affinity is per thread as well: all threads of the process are moved.
pub fn set_affinity(key: ProcKey, cpus: &[u32]) -> Result<(), ControlError> {
    let pid = verify(key)?;
    let bits = (size_of::<libc::cpu_set_t>() * 8) as u32;
    if cpus.is_empty() || cpus.iter().any(|c| *c >= bits) {
        return Err(ControlError::Other("invalid CPU list".to_owned()));
    }
    // SAFETY: an all-zero `cpu_set_t` is a valid empty set.
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    for c in cpus {
        // SAFETY: `c` was checked to be inside the set just above.
        unsafe { libc::CPU_SET(*c as usize, &mut set) };
    }
    let mut result = Ok(());
    for tid in tasks(pid) {
        // SAFETY: `set` is a valid set of the size passed; `tid` is a positive thread id.
        let rc = unsafe { libc::sched_setaffinity(tid, size_of::<libc::cpu_set_t>(), &raw const set) };
        if rc != 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() != Some(libc::ESRCH) {
                result = Err(map_errno(e));
            }
        }
    }
    result
}
