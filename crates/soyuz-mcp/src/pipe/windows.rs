//! Raw Windows named pipe (kernel32 + advapi32 only; no shell, no COM).
//!
//! * The pipe's DACL grants access only to the current user's SID and SYSTEM, and remote
//!   clients are rejected (`PIPE_REJECT_REMOTE_CLIENTS`).
//! * Handles are overlapped so one thread can block in `ReadFile` while another writes:
//!   synchronous handles serialize all I/O on the file object, which would deadlock a
//!   full-duplex relay.
//! * Clients check that the pipe is owned by the same user (or by the Administrators
//!   group, for an elevated window) before trusting it, so another account cannot squat
//!   the name and impersonate Soyuz.

use std::io;
use std::ptr::{null, null_mut};
use std::sync::Arc;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER, ERROR_IO_PENDING, ERROR_NO_DATA,
    ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
    SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile,
    WriteFile,
};
use windows_sys::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, WaitNamedPipeW,
};
use windows_sys::Win32::System::Threading::{CreateEventW, GetCurrentProcess, OpenProcessToken};

const BUF: u32 = 64 * 1024;
/// Well-known SID of BUILTIN\Administrators.
const ADMINS_SID: &str = "S-1-5-32-544";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_error() -> io::Error {
    io::Error::last_os_error()
}

/// An owned kernel handle.
struct Handle(HANDLE);

// SAFETY: a kernel handle is just an index into the process handle table; overlapped I/O
// on it from several threads is allowed (each operation has its own OVERLAPPED + event).
unsafe impl Send for Handle {}
// SAFETY: see `Send`; `&Handle` only exposes the raw value for thread-safe Win32 calls.
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: we own the handle and close it exactly once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// A manual-reset event for one overlapped operation.
fn event() -> io::Result<Handle> {
    // SAFETY: plain call with null security attributes and name.
    let h = unsafe { CreateEventW(null(), 1, 0, null()) };
    if h.is_null() { Err(last_error()) } else { Ok(Handle(h)) }
}

/// Run one overlapped operation to completion. `start` issues it with the OVERLAPPED.
fn overlapped(h: HANDLE, start: impl FnOnce(*mut OVERLAPPED) -> i32) -> io::Result<u32> {
    let ev = event()?;
    // SAFETY: OVERLAPPED is plain data; all-zero is its documented initial state.
    let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
    ov.hEvent = ev.0;
    if start(&mut ov) == 0 {
        // SAFETY: trivially safe.
        let err = unsafe { GetLastError() };
        if err != ERROR_IO_PENDING {
            return Err(io::Error::from_raw_os_error(err as i32));
        }
    }
    let mut n = 0u32;
    // SAFETY: `ov` (and the caller's buffer) outlive the wait because bWait = TRUE.
    if unsafe { GetOverlappedResult(h, &ov, &mut n, 1) } == 0 {
        return Err(last_error());
    }
    Ok(n)
}

fn is_disconnect(e: &io::Error) -> bool {
    matches!(e.raw_os_error().map(|c| c as u32), Some(ERROR_BROKEN_PIPE | ERROR_NO_DATA | ERROR_PIPE_NOT_CONNECTED))
}

/// One end of a connected pipe. Clones share the handle (read on one, write on another).
#[derive(Clone)]
pub struct PipeStream(Arc<Handle>);

impl io::Read for PipeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let h = self.0.0;
        let len = buf.len().min(u32::MAX as usize) as u32;
        let ptr = buf.as_mut_ptr();
        // SAFETY: `ptr`/`len` describe `buf`, which stays borrowed until the op completes.
        match overlapped(h, |ov| unsafe { ReadFile(h, ptr, len, null_mut(), ov) }) {
            Ok(n) => Ok(n as usize),
            Err(e) if is_disconnect(&e) => Ok(0),
            Err(e) => Err(e),
        }
    }
}

impl io::Write for PipeStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let h = self.0.0;
        let len = buf.len().min(u32::MAX as usize) as u32;
        let ptr = buf.as_ptr();
        // SAFETY: as in `read`; the buffer outlives the completed operation.
        match overlapped(h, |ov| unsafe { WriteFile(h, ptr, len, null_mut(), ov) }) {
            Ok(n) => Ok(n as usize),
            Err(e) if is_disconnect(&e) => Err(io::Error::new(io::ErrorKind::BrokenPipe, e)),
            Err(e) => Err(e),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl PipeStream {
    /// Process id of the server end (a client asks before granting it the foreground).
    pub fn server_pid(&self) -> Option<u32> {
        let mut pid = 0u32;
        // SAFETY: valid pipe handle and out pointer.
        (unsafe { GetNamedPipeServerProcessId(self.0.0, &mut pid) } != 0 && pid != 0).then_some(pid)
    }
}

/// String SID of the current process user.
fn current_user_sid() -> io::Result<String> {
    let mut token: HANDLE = null_mut();
    // SAFETY: valid pseudo-handle and out pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(last_error());
    }
    let token = Handle(token);
    let mut len = 0u32;
    // SAFETY: size query with a null buffer.
    unsafe { GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut len) };
    // SAFETY: trivially safe.
    if unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER || len == 0 {
        return Err(last_error());
    }
    // u64 storage keeps TOKEN_USER suitably aligned.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: `buf` holds at least `len` bytes.
    if unsafe { GetTokenInformation(token.0, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) } == 0 {
        return Err(last_error());
    }
    // SAFETY: the call filled a TOKEN_USER at the start of `buf`.
    let sid = unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid };
    sid_string(sid)
}

fn sid_string(sid: PSID) -> io::Result<String> {
    let mut p: *mut u16 = null_mut();
    // SAFETY: `sid` is a valid SID; the API allocates `p` with LocalAlloc.
    if unsafe { ConvertSidToStringSidW(sid, &mut p) } == 0 {
        return Err(last_error());
    }
    // SAFETY: `p` is a NUL-terminated wide string from the API.
    let s = unsafe {
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
    };
    // SAFETY: allocated by ConvertSidToStringSidW.
    unsafe { LocalFree(p.cast()) };
    Ok(s)
}

/// Security descriptor: full access for the current user and SYSTEM only.
struct Descriptor(PSECURITY_DESCRIPTOR);

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
        unsafe { LocalFree(self.0) };
    }
}

fn user_only_descriptor() -> io::Result<Descriptor> {
    let sddl = wide(&format!("D:P(A;;GA;;;{})(A;;GA;;;SY)", current_user_sid()?));
    let mut sd: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: valid NUL-terminated SDDL and out pointer.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, null_mut())
    } == 0
    {
        return Err(last_error());
    }
    Ok(Descriptor(sd))
}

/// Create one server instance of `name`.
fn create_instance(name: &str, first: bool) -> io::Result<Handle> {
    let sd = user_only_descriptor()?;
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: 0,
    };
    let w = wide(name);
    let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    // SAFETY: valid name, flags and security attributes (alive for the call).
    let h = unsafe {
        windows_sys::Win32::System::Pipes::CreateNamedPipeW(
            w.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            BUF,
            BUF,
            0,
            &sa,
        )
    };
    if h == INVALID_HANDLE_VALUE { Err(last_error()) } else { Ok(Handle(h)) }
}

/// A listening pipe name; [`accept`](Self::accept) yields connected streams.
pub struct PipeListener {
    name: String,
    next: Option<Handle>,
}

impl PipeListener {
    /// Bind `name`; fails when another process already serves it.
    pub fn bind(name: &str) -> io::Result<PipeListener> {
        let first = create_instance(name, true)?;
        Ok(PipeListener { name: name.to_string(), next: Some(first) })
    }

    /// Wait for the next client.
    ///
    /// The next instance is created before the connected one is handed out (and after a
    /// failed connect), so the name always has an instance while the listener lives: a
    /// client arriving in between gets `ERROR_PIPE_BUSY` (and waits) or connects at once,
    /// never `ERROR_FILE_NOT_FOUND`, which would read as "nobody serves this name" (and
    /// let a second launcher bind it as the first instance).
    pub fn accept(&mut self) -> io::Result<PipeStream> {
        let h = match self.next.take() {
            Some(h) => h,
            None => create_instance(&self.name, false)?,
        };
        let raw = h.0;
        // SAFETY: `raw` is a pipe server handle opened for overlapped I/O.
        let res = overlapped(raw, |ov| unsafe { ConnectNamedPipe(raw, ov) });
        match create_instance(&self.name, false) {
            Ok(next) => self.next = Some(next),
            Err(e) => log::warn!("{}: cannot create the next pipe instance: {e}", self.name),
        }
        match res {
            Ok(_) => {}
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32) => {}
            Err(e) => return Err(e),
        }
        Ok(PipeStream(Arc::new(h)))
    }
}

/// Connect to `name`. `Ok(None)` when nobody serves it.
pub fn connect(name: &str) -> io::Result<Option<PipeStream>> {
    let w = wide(name);
    for _ in 0..20 {
        // SAFETY: valid name; no security attributes or template.
        let h = unsafe {
            CreateFileW(
                w.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                null_mut(),
            )
        };
        if h != INVALID_HANDLE_VALUE {
            let h = Handle(h);
            check_owner(h.0)?;
            return Ok(Some(PipeStream(Arc::new(h))));
        }
        // SAFETY: trivially safe.
        match unsafe { GetLastError() } {
            ERROR_FILE_NOT_FOUND => return Ok(None),
            ERROR_PIPE_BUSY => {
                // SAFETY: valid name.
                unsafe { WaitNamedPipeW(w.as_ptr(), 250) };
            }
            e => return Err(io::Error::from_raw_os_error(e as i32)),
        }
    }
    Err(io::Error::new(io::ErrorKind::TimedOut, "the Soyuz pipe stayed busy"))
}

/// Refuse pipes owned by another account.
fn check_owner(h: HANDLE) -> io::Result<()> {
    let mut owner: PSID = null_mut();
    let mut sd: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: valid handle; out pointers for the owner SID and the descriptor holding it.
    let rc = unsafe {
        GetSecurityInfo(
            h,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut sd,
        )
    };
    if rc != 0 {
        return Err(io::Error::from_raw_os_error(rc as i32));
    }
    let sd = Descriptor(sd);
    let owner = sid_string(owner)?;
    drop(sd);
    let me = current_user_sid()?;
    if owner == me || owner == ADMINS_SID {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("the Soyuz pipe is owned by another account ({owner})"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    fn unique(tag: &str) -> String {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        format!(r"\\.\pipe\soyuz-test-{tag}-{}-{t:x}", std::process::id())
    }

    /// A client arriving while the server is busy with the previous connection (between
    /// two `accept` calls) still finds the name served, and a second `bind` fails.
    #[test]
    fn name_stays_served_between_accepts() {
        let name = unique("accept");
        let mut l = PipeListener::bind(&name).unwrap();
        assert!(PipeListener::bind(&name).is_err(), "a second first instance is refused");
        let n = 6;
        let server = std::thread::spawn(move || {
            for i in 0..n {
                let mut s = l.accept().unwrap();
                s.write_all(
                    format!(
                        "{i}
"
                    )
                    .as_bytes(),
                )
                .unwrap();
                // A slow accept loop: the next clients connect meanwhile.
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
            l
        });
        let clients: Vec<_> = (0..n)
            .map(|_| {
                let name = name.clone();
                std::thread::spawn(move || {
                    let c = connect(&name).unwrap().expect("the name is served");
                    assert_eq!(c.server_pid(), Some(std::process::id()));
                    let mut line = String::new();
                    BufReader::new(c).read_line(&mut line).unwrap();
                    line
                })
            })
            .collect();
        let mut got: Vec<String> = clients.into_iter().map(|t| t.join().unwrap()).collect();
        got.sort();
        assert_eq!(got.len(), n);
        let l = server.join().unwrap();
        // Still served after the last accept returned (the next instance exists).
        assert!(PipeListener::bind(&name).is_err());
        drop(l);
        assert!(connect(&name).unwrap().is_none(), "dropping the listener frees the name");
        assert!(PipeListener::bind(&name).is_ok());
    }
}
