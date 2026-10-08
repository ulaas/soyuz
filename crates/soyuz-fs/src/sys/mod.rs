//! Raw per-OS FFI shims: the only place in this crate where `unsafe` is allowed.
//!
//! * `windows/*` — kernel32 / advapi32 via `windows-sys` (never a shell or COM API).
//! * `unix.rs` / `linux.rs` / `macos.rs` — the few libc calls rustix does not wrap.
//! * `reparse.rs` — the hand-written `REPARSE_DATA_BUFFER` layout (pure, compiled on every
//!   OS so its parser/builder are unit-tested everywhere).

pub(crate) mod reparse;

#[cfg(target_os = "linux")]
pub(crate) mod linux;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(unix)]
pub(crate) mod unix;
#[cfg(windows)]
pub(crate) mod windows;
