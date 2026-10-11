//! Linux-only raw calls rustix does not wrap. Everything else (statx, FICLONE,
//! copy_file_range, renameat2, FS_IOC_GETFLAGS/SETFLAGS, fallocate, fadvise, setpriority)
//! is a safe rustix call made from the public modules.

use std::io;

pub(crate) mod wayland_dnd;
pub(crate) mod xdnd;

const IOPRIO_WHO_PROCESS: libc::c_int = 1;
const IOPRIO_CLASS_IDLE: libc::c_int = 3;
const IOPRIO_CLASS_SHIFT: libc::c_int = 13;

/// Kernel id of the calling thread.
pub(crate) fn gettid() -> i32 {
    // SAFETY: `gettid` takes no arguments and cannot fail.
    unsafe { libc::gettid() }
}

/// `ioprio_set(IOPRIO_WHO_PROCESS, tid, IOPRIO_CLASS_IDLE)`: the thread `tid` only gets
/// disk time when no other I/O is pending (honoured by the BFQ scheduler; accepted but
/// without effect under `none` / `mq-deadline`).
pub(crate) fn ioprio_idle(tid: i32) -> io::Result<()> {
    let prio = IOPRIO_CLASS_IDLE << IOPRIO_CLASS_SHIFT;
    // SAFETY: plain syscall with integer arguments; no memory is passed.
    let rc = unsafe { libc::syscall(libc::SYS_ioprio_set, IOPRIO_WHO_PROCESS, tid, prio) };
    if rc == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

/// `ioprio_get(IOPRIO_WHO_PROCESS, tid)` → the I/O priority class (tests).
#[cfg(test)]
pub(crate) fn ioprio_class(tid: i32) -> io::Result<i32> {
    // SAFETY: plain syscall with integer arguments; no memory is passed.
    let rc = unsafe { libc::syscall(libc::SYS_ioprio_get, IOPRIO_WHO_PROCESS, tid) };
    if rc < 0 { Err(io::Error::last_os_error()) } else { Ok((rc as i32) >> IOPRIO_CLASS_SHIFT) }
}
