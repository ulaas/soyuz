//! Linux-only raw calls rustix does not wrap. Everything else (statx, FICLONE,
//! copy_file_range, renameat2, FS_IOC_GETFLAGS/SETFLAGS, fallocate, fadvise, setpriority)
//! is a safe rustix call made from the public modules.

use std::io;

pub(crate) mod process;
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

/// `poll(2)` on `fds` (`(fd, want_write)`): readability (or writability when
/// `want_write`), hang-up or error. `timeout_ms` < 0 waits forever. Returns one flag per
/// descriptor (at most 4 are polled).
pub fn poll_fds(fds: &[(std::os::fd::BorrowedFd<'_>, bool)], timeout_ms: i32) -> io::Result<[bool; 4]> {
    use std::os::fd::AsRawFd;
    let mut p = [libc::pollfd { fd: -1, events: 0, revents: 0 }; 4];
    for (slot, (fd, write)) in p.iter_mut().zip(fds) {
        let events = if *write { libc::POLLOUT } else { libc::POLLIN };
        *slot = libc::pollfd { fd: fd.as_raw_fd(), events, revents: 0 };
    }
    let n = fds.len().min(4);
    loop {
        // SAFETY: `p` holds `n` initialised `pollfd`s whose descriptors are borrowed (open)
        // for the duration of the call.
        let rc = unsafe { libc::poll(p.as_mut_ptr(), n as libc::nfds_t, timeout_ms) };
        if rc >= 0 {
            return Ok([p[0].revents != 0, p[1].revents != 0, p[2].revents != 0, p[3].revents != 0]);
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// Put `fd` in non-blocking mode (`O_NONBLOCK`).
pub fn set_nonblocking(fd: std::os::fd::BorrowedFd<'_>) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let fd = fd.as_raw_fd();
    // SAFETY: `fcntl(F_GETFL / F_SETFL)` on a borrowed, open descriptor; no memory is passed.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: as above.
    let rc = unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) };
    if rc < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}
