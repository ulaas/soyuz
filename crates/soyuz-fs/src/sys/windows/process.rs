//! This process's memory counters (`K32GetProcessMemoryInfo`, kernel32).

use std::mem::size_of;

use windows_sys::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

/// `(private bytes, working set)` of the calling process.
pub(crate) fn memory() -> Option<(u64, u64)> {
    let mut c = PROCESS_MEMORY_COUNTERS_EX { cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
    // SAFETY: the pseudo handle of the current process needs no closing; `c` is a writable
    // PROCESS_MEMORY_COUNTERS_EX whose size we pass (the EX layout extends the base one).
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut c as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            c.cb,
        )
    };
    (ok != 0).then_some((c.PrivateUsage as u64, c.WorkingSetSize as u64))
}
