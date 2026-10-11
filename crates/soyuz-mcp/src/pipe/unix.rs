//! Unix domain socket transport (the unix twin of the Windows named pipe). Safe code only.
//!
//! * Sockets live in a per-user directory: `$XDG_RUNTIME_DIR/soyuz/`, or `/tmp/soyuz-<uid>/`
//!   when `XDG_RUNTIME_DIR` is unset. The directory is created `0700` and both sides refuse
//!   it unless it is a real directory owned by the effective user with no group / other
//!   access, so another account can neither squat the name nor listen in.
//! * One server per name: the server holds an exclusive `flock` on `<name>.lock` for its
//!   lifetime (released by the kernel if the process dies) and refuses to replace a socket
//!   that still accepts connections. A stale socket file left by a crash is removed.
//! * Linux checks the peer (`SO_PEERCRED`) on both ends: the server drops connections from
//!   other uids and the client refuses a server running as another uid.

use std::fs::{DirBuilder, File, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The per-user socket directory (not created here).
pub fn socket_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
        Some(d) if d.is_absolute() => d.join("soyuz"),
        _ => PathBuf::from(format!("/tmp/soyuz-{}", euid())),
    }
}

/// `<socket dir>/<prefix>.sock`.
pub fn socket_path(prefix: &str) -> String {
    socket_dir().join(format!("{prefix}.sock")).to_string_lossy().into_owned()
}

fn euid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// Is `dir` a private directory of ours (owned by the effective user, no group / other
/// bits, not a symlink)?
fn check_dir(dir: &Path) -> io::Result<()> {
    let md = std::fs::symlink_metadata(dir)?;
    if !md.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not a directory; refusing to use it for the Soyuz socket", dir.display()),
        ));
    }
    if md.uid() != euid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is owned by another account (uid {}); refusing to use it", dir.display(), md.uid()),
        ));
    }
    if md.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is accessible to other users (mode {:o}); expected 0700", dir.display(), md.mode() & 0o777),
        ));
    }
    Ok(())
}

/// Create (if needed) and verify the directory that holds `path`.
fn ensure_dir(path: &Path) -> io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    match DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    check_dir(dir)
}

fn lock_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".lock");
    PathBuf::from(s)
}

fn in_use(path: &Path) -> io::Error {
    io::Error::new(io::ErrorKind::AddrInUse, format!("another process already serves {}", path.display()))
}

/// The peer of a connected socket runs as us (Linux only; elsewhere the private directory
/// is the guard).
fn check_peer(stream: &UnixStream) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let cred = rustix::net::sockopt::socket_peercred(stream)?;
        let uid = cred.uid.as_raw();
        if uid != euid() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("the peer runs as another account (uid {uid})"),
            ));
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = stream;
    Ok(())
}

/// One end of a connected socket. Clones share the socket (read on one, write on another).
#[derive(Clone, Debug)]
pub struct PipeStream(Arc<UnixStream>);

impl io::Read for PipeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&*self.0).read(buf)
    }
}

impl io::Write for PipeStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self.0).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        (&*self.0).flush()
    }
}

/// A bound, listening socket plus the lock that makes it the only server of its name.
/// Dropping it removes the socket file, then releases the lock.
pub struct PipeListener {
    listener: Arc<UnixListener>,
    path: PathBuf,
    _lock: File,
}

impl PipeListener {
    /// Bind `name` (a socket path); fails with `AddrInUse` when another process serves it.
    pub fn bind(name: &str) -> io::Result<PipeListener> {
        let path = PathBuf::from(name);
        ensure_dir(&path)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(o_nofollow())
            .open(lock_path(&path))?;
        match rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => {}
            Err(rustix::io::Errno::WOULDBLOCK) => return Err(in_use(&path)),
            Err(e) => return Err(e.into()),
        }
        // A socket file without the lock is either stale (crash) or served by a process
        // that predates the lock; only a dead one is replaced.
        if std::fs::symlink_metadata(&path).is_ok() {
            if UnixStream::connect(&path).is_ok() {
                return Err(in_use(&path));
            }
            std::fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        // The directory is already private; tighten the socket itself too.
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        Ok(PipeListener { listener: Arc::new(listener), path, _lock: lock })
    }

    /// Wait for the next client (connections from other uids are dropped and skipped).
    pub fn accept(&self) -> io::Result<PipeStream> {
        loop {
            let (stream, _) = self.listener.accept()?;
            match check_peer(&stream) {
                Ok(()) => return Ok(PipeStream(Arc::new(stream))),
                Err(e) => log::warn!("{}: rejected a connection: {e}", self.path.display()),
            }
        }
    }

    /// A handle that can unblock [`accept`](Self::accept) from another thread.
    pub fn waker(&self) -> Waker {
        Waker(self.listener.clone())
    }
}

impl Drop for PipeListener {
    fn drop(&mut self) {
        // Still ours: we hold the lock until the end of this drop.
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Unblocks a thread waiting in [`PipeListener::accept`].
pub struct Waker(Arc<UnixListener>);

impl Waker {
    /// `shutdown` on a listening socket makes a blocked `accept` return an error (Linux);
    /// later accepts fail at once, so the loop sees its stop flag.
    pub fn wake(&self) {
        let _ = rustix::net::shutdown(&*self.0, rustix::net::Shutdown::Both);
    }
}

/// `O_NOFOLLOW` for the lock file: never follow a planted symlink.
fn o_nofollow() -> i32 {
    rustix::fs::OFlags::NOFOLLOW.bits() as i32
}

/// Connect to `name`. `Ok(None)` when nobody serves it (no directory, no socket, or a stale one).
pub fn connect(name: &str) -> io::Result<Option<PipeStream>> {
    let path = Path::new(name);
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    match check_dir(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    }
    let stream = match UnixStream::connect(path) {
        Ok(s) => s,
        Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) => return Ok(None),
        Err(e) => return Err(e),
    };
    check_peer(&stream)?;
    Ok(Some(PipeStream(Arc::new(stream))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    fn private_dir() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        d
    }

    #[test]
    fn names_live_in_the_user_dir() {
        let p = socket_path("soyuz-cli");
        assert!(p.ends_with("/soyuz-cli.sock"), "{p}");
        assert!(Path::new(&p).is_absolute());
    }

    #[test]
    fn single_server_and_stale_socket() {
        let d = private_dir();
        let name = d.path().join("s.sock").to_string_lossy().into_owned();
        assert!(connect(&name).unwrap().is_none(), "nobody serves it yet");
        let l = PipeListener::bind(&name).unwrap();
        assert_eq!(PipeListener::bind(&name).err().map(|e| e.kind()), Some(io::ErrorKind::AddrInUse));
        let t = std::thread::spawn({
            let name = name.clone();
            move || {
                let mut c = connect(&name).unwrap().unwrap();
                c.write_all(b"hi\n").unwrap();
            }
        });
        let s = l.accept().unwrap();
        let mut line = String::new();
        BufReader::new(s).read_line(&mut line).unwrap();
        assert_eq!(line, "hi\n");
        t.join().unwrap();
        drop(l);
        assert!(!Path::new(&name).exists(), "the socket is removed on drop");
        // A stale socket file (crashed server) is replaced.
        drop(UnixListener::bind(&name).unwrap());
        assert!(Path::new(&name).exists());
        // Other tests in this process spawn children: between fork and exec a child briefly
        // holds copies of our descriptors (the lock, the listening socket), so allow that
        // window to pass before calling the socket stale / free.
        assert!(eventually(|| connect(&name).unwrap().is_none()), "stale socket reads as not served");
        assert!(eventually(|| PipeListener::bind(&name).is_ok()), "a stale socket is replaced");
    }

    /// `ok()` holds within 2 s.
    fn eventually(ok: impl Fn() -> bool) -> bool {
        (0..100).any(|_| {
            ok() || {
                std::thread::sleep(std::time::Duration::from_millis(20));
                false
            }
        })
    }

    #[test]
    fn refuses_shared_directories() {
        let d = tempfile::tempdir().unwrap();
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
        let name = d.path().join("s.sock").to_string_lossy().into_owned();
        assert_eq!(PipeListener::bind(&name).err().map(|e| e.kind()), Some(io::ErrorKind::PermissionDenied));
        assert_eq!(connect(&name).err().map(|e| e.kind()), Some(io::ErrorKind::PermissionDenied));
    }

    #[test]
    fn waker_unblocks_accept() {
        let d = private_dir();
        let name = d.path().join("w.sock").to_string_lossy().into_owned();
        let l = PipeListener::bind(&name).unwrap();
        let w = l.waker();
        let t = std::thread::spawn(move || l.accept().is_err());
        std::thread::sleep(std::time::Duration::from_millis(50));
        w.wake();
        assert!(t.join().unwrap(), "accept returned an error after wake");
    }
}
