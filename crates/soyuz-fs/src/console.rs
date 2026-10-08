//! Console helpers for command-line modes of GUI binaries.

/// Windows: attach to the parent process's console so a GUI-subsystem binary run from a
/// terminal (`soyuz cp …`) can print there. `false` when there is no console to attach
/// to (or on other platforms, where stdout already works).
pub fn attach_parent() -> bool {
    #[cfg(windows)]
    {
        crate::sys::windows::console::attach_parent()
    }
    #[cfg(not(windows))]
    {
        false
    }
}
