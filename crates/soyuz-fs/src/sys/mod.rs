//! Raw per-OS FFI shims: the only place in this crate where `unsafe` is allowed.
//! Empty in M0 (the baseline uses `std::fs` only).

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
