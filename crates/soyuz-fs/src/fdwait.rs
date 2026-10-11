//! Bounded waits on file descriptors (Linux): `poll(2)` with a timeout, and pipe reads /
//! writes that give up at a deadline instead of blocking forever on a peer that never
//! reads or never closes (clipboard transfers between applications).

use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::time::{Duration, Instant};

/// Wait until one of `fds` (`(fd, want_write)`) is ready, at most `timeout` (`None` =
/// forever). Returns one flag per descriptor (at most 4).
pub fn poll(fds: &[(BorrowedFd<'_>, bool)], timeout: Option<Duration>) -> io::Result<[bool; 4]> {
    let ms = timeout.map_or(-1, |t| i32::try_from(t.as_millis().max(u128::from(!t.is_zero()))).unwrap_or(i32::MAX));
    crate::sys::linux::poll_fds(fds, ms)
}

fn left(deadline: Instant) -> io::Result<Duration> {
    let d = deadline.saturating_duration_since(Instant::now());
    if d.is_zero() { Err(io::Error::new(io::ErrorKind::TimedOut, "deadline passed")) } else { Ok(d) }
}

/// Write all of `data` to `f` (switched to non-blocking mode), giving up at `deadline`.
pub fn write_all_until<F: AsFd + Write>(f: &mut F, mut data: &[u8], deadline: Instant) -> io::Result<()> {
    crate::sys::linux::set_nonblocking(f.as_fd())?;
    while !data.is_empty() {
        match f.write(data) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => data = &data[n..],
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                poll(&[(f.as_fd(), true)], Some(left(deadline)?))?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Read `f` (switched to non-blocking mode) to its end, giving up at `deadline` or when
/// more than `max` bytes arrive.
pub fn read_to_end_until<F: AsFd + Read>(f: &mut F, deadline: Instant, max: usize) -> io::Result<Vec<u8>> {
    crate::sys::linux::set_nonblocking(f.as_fd())?;
    let mut out = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        match f.read(&mut buf) {
            Ok(0) => return Ok(out),
            Ok(n) => {
                if out.len() + n > max {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "too large"));
                }
                out.extend_from_slice(&buf[..n]);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                poll(&[(f.as_fd(), false)], Some(left(deadline)?))?;
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_and_write_give_up_at_the_deadline() {
        let (r, mut w) = std::io::pipe().unwrap();
        // Nobody reads: a body larger than the pipe buffer must time out, not block.
        let big = vec![7u8; 4 << 20];
        let t = Instant::now();
        let e = write_all_until(&mut w, &big, Instant::now() + Duration::from_millis(150)).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        assert!(t.elapsed() < Duration::from_secs(2));
        drop(r);
        // The writer stays open and silent: the read times out too.
        let (mut r2, w2) = std::io::pipe().unwrap();
        let e = read_to_end_until(&mut r2, Instant::now() + Duration::from_millis(100), 1 << 20).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::TimedOut);
        drop(w2);
        assert_eq!(read_to_end_until(&mut r2, Instant::now() + Duration::from_millis(100), 1 << 20).unwrap(), b"");
    }

    #[test]
    fn small_transfers_complete() {
        let (mut r, mut w) = std::io::pipe().unwrap();
        write_all_until(&mut w, b"hello", Instant::now() + Duration::from_secs(1)).unwrap();
        drop(w);
        assert_eq!(read_to_end_until(&mut r, Instant::now() + Duration::from_secs(1), 64).unwrap(), b"hello");
    }
}
