//! Console attachment for the GUI-subsystem `soyuz.exe` (kernel32 `AttachConsole`).

use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

/// Attach to the console of the parent process (the shell that started us).
pub(crate) fn attach_parent() -> bool {
    // SAFETY: plain kernel32 call without pointers; fails harmlessly when the parent
    // has no console or we already have one.
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
}
