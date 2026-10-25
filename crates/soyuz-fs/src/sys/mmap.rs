//! Read-only views of whole files (`MapViewOfFile` / `mmap`), wrapped safely by
//! [`crate::mapped`]. The caller keeps the file open as long as it needs the share mode
//! (Windows); the view itself stays valid on its own until it is dropped.

use std::fs::File;
use std::io;
use std::ptr::NonNull;

/// A read-only view of the first `len` bytes of a file (`len` > 0).
pub(crate) struct View {
    ptr: NonNull<u8>,
    len: usize,
}

// SAFETY: the view is read-only memory owned by this value; nothing mutates it through
// the pointer, so sharing or moving it between threads is sound.
unsafe impl Send for View {}
// SAFETY: as above (only `&[u8]` is ever handed out).
unsafe impl Sync for View {}

impl View {
    pub(crate) fn bytes(&self) -> &[u8] {
        // SAFETY: `ptr` maps `len` readable bytes until `drop` unmaps them.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

#[cfg(windows)]
impl View {
    /// Map `len` bytes of `file` (opened for reading).
    pub(crate) fn map(file: &File, len: usize) -> io::Result<View> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Memory::{CreateFileMappingW, FILE_MAP_READ, MapViewOfFile, PAGE_READONLY};
        if len == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty file"));
        }
        let h = file.as_raw_handle();
        // SAFETY: `h` is a valid open file handle; no name, default security.
        let section = unsafe { CreateFileMappingW(h, std::ptr::null(), PAGE_READONLY, 0, 0, std::ptr::null()) };
        if section.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `section` is the mapping just created; offset 0, `len` ≤ the file size.
        let view = unsafe { MapViewOfFile(section, FILE_MAP_READ, 0, 0, len) };
        let err = io::Error::last_os_error();
        // The view keeps the section alive; the handle is not needed any more.
        // SAFETY: `section` is a handle we own.
        unsafe { CloseHandle(section) };
        NonNull::new(view.Value.cast::<u8>()).map(|ptr| View { ptr, len }).ok_or(err)
    }
}

#[cfg(windows)]
impl Drop for View {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Memory::{MEMORY_MAPPED_VIEW_ADDRESS, UnmapViewOfFile};
        // SAFETY: `ptr` is the base address MapViewOfFile returned; unmapped exactly once.
        unsafe { UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: self.ptr.as_ptr().cast() }) };
    }
}

#[cfg(unix)]
impl View {
    /// Map `len` bytes of `file` (opened for reading), private and read-only.
    pub(crate) fn map(file: &File, len: usize) -> io::Result<View> {
        use rustix::mm::{MapFlags, ProtFlags, mmap};
        if len == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty file"));
        }
        // SAFETY: a fresh mapping at an address the kernel picks (no existing memory is
        // replaced); `file` is open for reading.
        let ptr = unsafe { mmap(std::ptr::null_mut(), len, ProtFlags::READ, MapFlags::PRIVATE, file, 0) }?;
        NonNull::new(ptr.cast::<u8>())
            .map(|ptr| View { ptr, len })
            .ok_or_else(|| io::Error::other("mmap returned null"))
    }
}

#[cfg(unix)]
impl Drop for View {
    fn drop(&mut self) {
        // SAFETY: `ptr`/`len` are exactly the mapping made in `map`; unmapped once.
        let _ = unsafe { rustix::mm::munmap(self.ptr.as_ptr().cast(), self.len) };
    }
}
