//! File-level kernel32 calls: stat, directory enumeration, copy, move, delete, links,
//! times, attributes, sparse ranges, block cloning and alternate data streams.

use std::any::Any;
use std::ffi::{OsString, c_void};
use std::fs::File;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::io::AsRawHandle;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_HANDLE_EOF, ERROR_INVALID_PARAMETER, ERROR_MORE_DATA,
    ERROR_NO_MORE_FILES, ERROR_SHARING_VIOLATION, FILETIME, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, COPY_FILE_FAIL_IF_EXISTS, CopyFileExW, CreateDirectoryW, CreateFileW, CreateHardLinkW,
    CreateSymbolicLinkW, DELETE, DeleteFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_ATTRIBUTE_TAG_INFO, FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE,
    FILE_DISPOSITION_FLAG_POSIX_SEMANTICS, FILE_DISPOSITION_INFO_EX, FILE_END_OF_FILE_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FILE_STANDARD_INFO, FILE_WRITE_ATTRIBUTES, FIND_FIRST_EX_LARGE_FETCH, FileAttributeTagInfo,
    FileDispositionInfoEx, FileEndOfFileInfo, FileIdInfo, FileStandardInfo, FindClose, FindExInfoBasic,
    FindExSearchNameMatch, FindFirstFileExW, FindFirstStreamW, FindNextFileW, FindNextStreamW, FindStreamInfoStandard,
    GetFileAttributesW, GetFileInformationByHandle, GetFileInformationByHandleEx, INVALID_FILE_ATTRIBUTES, MoveFileExW,
    OPEN_EXISTING, PROGRESS_CANCEL, PROGRESS_CONTINUE, RemoveDirectoryW, SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE,
    SYMBOLIC_LINK_FLAG_DIRECTORY, SetFileAttributesW, SetFileInformationByHandle, SetFileTime, WIN32_FIND_DATAW,
    WIN32_FIND_STREAM_DATA,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    DUPLICATE_EXTENTS_DATA, FILE_ALLOCATED_RANGE_BUFFER, FSCTL_DUPLICATE_EXTENTS_TO_FILE, FSCTL_GET_REPARSE_POINT,
    FSCTL_QUERY_ALLOCATED_RANGES, FSCTL_SET_COMPRESSION, FSCTL_SET_REPARSE_POINT, FSCTL_SET_SPARSE,
};

use super::{Handle, ft_u64, last_err, u64_ft, wide_path, wide_str};
use crate::sys::reparse::MAXIMUM_REPARSE_DATA_BUFFER_SIZE;

pub(crate) const SHARE_ALL: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

/// Raw metadata of one object, from a handle or a directory entry.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RawStat {
    pub attrs: u32,
    pub created: u64,
    pub accessed: u64,
    pub written: u64,
    pub size: u64,
    pub alloc: Option<u64>,
    pub nlink: Option<u32>,
    pub volume: Option<u64>,
    pub file_index: Option<u64>,
    /// Reparse tag when `FILE_ATTRIBUTE_REPARSE_POINT` is set, else 0.
    pub reparse_tag: u32,
}

impl RawStat {
    pub(crate) fn is_dir(&self) -> bool {
        self.attrs & FILE_ATTRIBUTE_DIRECTORY != 0
    }

    pub(crate) fn is_reparse(&self) -> bool {
        self.attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
}

/// `CreateFileW(OPEN_EXISTING)` with share-all.
pub(crate) fn open(path: &Path, access: u32, flags: u32) -> io::Result<Handle> {
    let w = wide_path(path);
    // SAFETY: `w` is a NUL-terminated UTF-16 string that outlives the call; all other
    // arguments are plain values or null (no security attributes / template).
    let h = unsafe { CreateFileW(w.as_ptr(), access, SHARE_ALL, null(), OPEN_EXISTING, flags, null_mut()) };
    Handle::new(h)
}

/// Open for metadata queries (works for directories; `follow = false` opens the link).
pub(crate) fn open_meta(path: &Path, follow: bool) -> io::Result<Handle> {
    let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
    if !follow {
        flags |= FILE_FLAG_OPEN_REPARSE_POINT;
    }
    open(path, FILE_READ_ATTRIBUTES, flags)
}

fn raw(h: &impl AsRawHandle) -> HANDLE {
    h.as_raw_handle() as HANDLE
}

/// Metadata of an open handle.
pub(crate) fn stat_handle(h: HANDLE) -> io::Result<RawStat> {
    // SAFETY: plain-old-data struct; all-zero is a valid initial value.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    // SAFETY: `h` is a valid open handle and `info` is a correctly sized out-buffer.
    if unsafe { GetFileInformationByHandle(h, &mut info) } == 0 {
        return Err(last_err());
    }
    let mut st = RawStat {
        attrs: info.dwFileAttributes,
        created: ft_u64(&info.ftCreationTime),
        accessed: ft_u64(&info.ftLastAccessTime),
        written: ft_u64(&info.ftLastWriteTime),
        size: (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow),
        alloc: None,
        nlink: Some(info.nNumberOfLinks),
        volume: Some(u64::from(info.dwVolumeSerialNumber)),
        file_index: Some((u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)),
        reparse_tag: 0,
    };
    if st.is_reparse() {
        // SAFETY: plain-old-data struct.
        let mut tag: FILE_ATTRIBUTE_TAG_INFO = unsafe { zeroed() };
        // SAFETY: valid handle, out-buffer of exactly the size we pass.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                h,
                FileAttributeTagInfo,
                (&raw mut tag).cast(),
                size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        };
        if ok != 0 {
            st.reparse_tag = tag.ReparseTag;
        }
    }
    // SAFETY: plain-old-data struct.
    let mut std_info: FILE_STANDARD_INFO = unsafe { zeroed() };
    // SAFETY: valid handle, out-buffer of exactly the size we pass.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            h,
            FileStandardInfo,
            (&raw mut std_info).cast(),
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    };
    if ok != 0 {
        st.alloc = Some(std_info.AllocationSize.max(0) as u64);
    }
    Ok(st)
}

/// Metadata of `path`; falls back to the directory entry when the object cannot be opened
/// (e.g. `pagefile.sys`, files locked without `FILE_SHARE_*`).
pub(crate) fn stat_path(path: &Path, follow: bool) -> io::Result<RawStat> {
    match open_meta(path, follow) {
        Ok(h) => stat_handle(h.raw()),
        Err(e) if matches!(e.raw_os_error(), Some(c) if c == ERROR_SHARING_VIOLATION as i32 || c == ERROR_ACCESS_DENIED as i32) => {
            find_single(path).map_err(|_| e)
        }
        Err(e) => Err(e),
    }
}

fn find_data_to_stat(d: &WIN32_FIND_DATAW) -> RawStat {
    let attrs = d.dwFileAttributes;
    RawStat {
        attrs,
        created: ft_u64(&d.ftCreationTime),
        accessed: ft_u64(&d.ftLastAccessTime),
        written: ft_u64(&d.ftLastWriteTime),
        size: (u64::from(d.nFileSizeHigh) << 32) | u64::from(d.nFileSizeLow),
        reparse_tag: if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 { d.dwReserved0 } else { 0 },
        ..RawStat::default()
    }
}

/// Directory-entry metadata of a single path (no wildcard).
pub(crate) fn find_single(path: &Path) -> io::Result<RawStat> {
    let w = wide_path(path);
    // SAFETY: plain-old-data struct.
    let mut data: WIN32_FIND_DATAW = unsafe { zeroed() };
    // SAFETY: NUL-terminated name, out-buffer of the type FindExInfoBasic expects.
    let h = unsafe {
        FindFirstFileExW(w.as_ptr(), FindExInfoBasic, (&raw mut data).cast(), FindExSearchNameMatch, null(), 0)
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(last_err());
    }
    // SAFETY: `h` is a valid search handle returned above.
    unsafe {
        FindClose(h);
    }
    Ok(find_data_to_stat(&data))
}

/// The final component of `path` as spelled on disk (one `FindFirstFileExW`, no wildcard).
pub(crate) fn find_single_name(path: &Path) -> io::Result<OsString> {
    let w = wide_path(path);
    // SAFETY: plain-old-data struct.
    let mut data: WIN32_FIND_DATAW = unsafe { zeroed() };
    // SAFETY: NUL-terminated name, out-buffer of the type FindExInfoBasic expects.
    let h = unsafe {
        FindFirstFileExW(w.as_ptr(), FindExInfoBasic, (&raw mut data).cast(), FindExSearchNameMatch, null(), 0)
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(last_err());
    }
    // SAFETY: `h` is a valid search handle returned above.
    unsafe {
        FindClose(h);
    }
    Ok(super::from_wide_nul(&data.cFileName))
}

/// One directory entry.
pub(crate) struct FindEntry {
    pub name: OsString,
    pub stat: RawStat,
}

/// `FindFirstFileExW(FindExInfoBasic, FIND_FIRST_EX_LARGE_FETCH)` iterator; skips `.`/`..`.
pub(crate) struct DirIter {
    handle: HANDLE,
    data: Box<WIN32_FIND_DATAW>,
    have_current: bool,
}

// SAFETY: the search handle is only used through `&mut self`; it may move between threads.
unsafe impl Send for DirIter {}

impl Drop for DirIter {
    fn drop(&mut self) {
        if self.handle != INVALID_HANDLE_VALUE {
            // SAFETY: valid search handle owned by this iterator, closed once.
            unsafe {
                FindClose(self.handle);
            }
        }
    }
}

pub(crate) fn read_dir(dir: &Path) -> io::Result<DirIter> {
    let mut w = wide_path(dir);
    w.pop(); // NUL
    if w.last() != Some(&(b'\\' as u16)) {
        w.push(b'\\' as u16);
    }
    w.push(b'*' as u16);
    w.push(0);
    // SAFETY: plain-old-data struct; boxed so its address is stable.
    let mut data: Box<WIN32_FIND_DATAW> = Box::new(unsafe { zeroed() });
    // SAFETY: NUL-terminated pattern and a valid out-buffer for FindExInfoBasic.
    let h = unsafe {
        FindFirstFileExW(
            w.as_ptr(),
            FindExInfoBasic,
            (&raw mut *data).cast(),
            FindExSearchNameMatch,
            null(),
            FIND_FIRST_EX_LARGE_FETCH,
        )
    };
    if h == INVALID_HANDLE_VALUE {
        let e = last_err();
        // An empty root directory has no "." entry: ERROR_FILE_NOT_FOUND means "empty" when
        // the directory itself exists.
        if e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32)
            && get_attributes(dir).is_ok_and(|a| a & FILE_ATTRIBUTE_DIRECTORY != 0)
        {
            return Ok(DirIter { handle: INVALID_HANDLE_VALUE, data, have_current: false });
        }
        return Err(e);
    }
    Ok(DirIter { handle: h, data, have_current: true })
}

impl Iterator for DirIter {
    type Item = io::Result<FindEntry>;

    fn next(&mut self) -> Option<io::Result<FindEntry>> {
        loop {
            if self.handle == INVALID_HANDLE_VALUE {
                return None;
            }
            if !self.have_current {
                // SAFETY: valid search handle and out-buffer.
                if unsafe { FindNextFileW(self.handle, &raw mut *self.data) } == 0 {
                    let e = last_err();
                    // SAFETY: valid search handle, closed once (then marked invalid).
                    unsafe {
                        FindClose(self.handle);
                    }
                    self.handle = INVALID_HANDLE_VALUE;
                    if e.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                        return None;
                    }
                    return Some(Err(e));
                }
            }
            self.have_current = false;
            let name = super::from_wide_nul(&self.data.cFileName);
            if name == "." || name == ".." {
                continue;
            }
            return Some(Ok(FindEntry { name, stat: find_data_to_stat(&self.data) }));
        }
    }
}

/// `(volume serial, 128-bit file id)` of `path`.
pub(crate) fn file_id(path: &Path, follow: bool) -> io::Result<(u64, [u8; 16])> {
    let h = open_meta(path, follow)?;
    // SAFETY: plain-old-data struct.
    let mut id: FILE_ID_INFO = unsafe { zeroed() };
    // SAFETY: valid handle, out-buffer of the size we pass.
    let ok = unsafe {
        GetFileInformationByHandleEx(h.raw(), FileIdInfo, (&raw mut id).cast(), size_of::<FILE_ID_INFO>() as u32)
    };
    if ok != 0 {
        return Ok((id.VolumeSerialNumber, id.FileId.Identifier));
    }
    let st = stat_handle(h.raw())?;
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&st.file_index.unwrap_or(0).to_le_bytes());
    Ok((st.volume.unwrap_or(0), bytes))
}

pub(crate) fn get_attributes(path: &Path) -> io::Result<u32> {
    let w = wide_path(path);
    // SAFETY: NUL-terminated path.
    let a = unsafe { GetFileAttributesW(w.as_ptr()) };
    if a == INVALID_FILE_ATTRIBUTES { Err(last_err()) } else { Ok(a) }
}

pub(crate) fn set_attributes(path: &Path, attrs: u32) -> io::Result<()> {
    let w = wide_path(path);
    // SAFETY: NUL-terminated path.
    if unsafe { SetFileAttributesW(w.as_ptr(), attrs) } == 0 { Err(last_err()) } else { Ok(()) }
}

pub(crate) fn delete_file(path: &Path) -> io::Result<()> {
    let w = wide_path(path);
    // SAFETY: NUL-terminated path.
    if unsafe { DeleteFileW(w.as_ptr()) } == 0 { Err(last_err()) } else { Ok(()) }
}

pub(crate) fn remove_dir(path: &Path) -> io::Result<()> {
    let w = wide_path(path);
    // SAFETY: NUL-terminated path.
    if unsafe { RemoveDirectoryW(w.as_ptr()) } == 0 { Err(last_err()) } else { Ok(()) }
}

/// Delete through `FileDispositionInfoEx` with POSIX semantics (the name disappears
/// immediately even while other handles are open). Never follows reparse points.
pub(crate) fn posix_delete(path: &Path, ignore_readonly: bool) -> io::Result<()> {
    let h = open(path, DELETE, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)?;
    let mut flags = FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS;
    if ignore_readonly {
        flags |= FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE;
    }
    let info = FILE_DISPOSITION_INFO_EX { Flags: flags };
    // SAFETY: valid handle opened with DELETE access; input buffer of the size we pass.
    let ok = unsafe {
        SetFileInformationByHandle(
            h.raw(),
            FileDispositionInfoEx,
            (&raw const info).cast(),
            size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
        )
    };
    if ok == 0 { Err(last_err()) } else { Ok(()) }
}

/// True when `e` means "this API/flag is not available here" (old Windows, FAT, SMB).
pub(crate) fn is_unsupported_error(e: &io::Error) -> bool {
    matches!(e.raw_os_error(), Some(1 | 50 | 87 | 124))
}

pub(crate) fn move_file(src: &Path, dst: &Path, flags: u32) -> io::Result<()> {
    let s = wide_path(src);
    let d = wide_path(dst);
    // SAFETY: both NUL-terminated paths outlive the call.
    if unsafe { MoveFileExW(s.as_ptr(), d.as_ptr(), flags) } == 0 { Err(last_err()) } else { Ok(()) }
}

/// `CreateSymbolicLinkW`; `target` is stored verbatim (relative targets stay relative).
pub(crate) fn create_symlink(link: &Path, target: &OsString, dir: bool) -> io::Result<()> {
    let l = wide_path(link);
    let t = wide_str(target);
    let base = if dir { SYMBOLIC_LINK_FLAG_DIRECTORY } else { 0 };
    // SAFETY: NUL-terminated strings outlive the call.
    let ok =
        unsafe { CreateSymbolicLinkW(l.as_ptr(), t.as_ptr(), base | SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE) };
    if ok {
        return Ok(());
    }
    let e = last_err();
    if e.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
        // Pre-1703 Windows does not know ALLOW_UNPRIVILEGED_CREATE.
        // SAFETY: as above.
        if unsafe { CreateSymbolicLinkW(l.as_ptr(), t.as_ptr(), base) } {
            return Ok(());
        }
        return Err(last_err());
    }
    Err(e)
}

pub(crate) fn create_hardlink(dst: &Path, src: &Path) -> io::Result<()> {
    let d = wide_path(dst);
    let s = wide_path(src);
    // SAFETY: NUL-terminated paths; no security attributes.
    if unsafe { CreateHardLinkW(d.as_ptr(), s.as_ptr(), null()) } == 0 { Err(last_err()) } else { Ok(()) }
}

/// `CreateDirectoryW`, optionally with a security descriptor.
pub(crate) fn create_dir(path: &Path, sd: Option<*mut c_void>) -> io::Result<()> {
    let w = wide_path(path);
    let sa = sd.map(|sd| windows_sys::Win32::Security::SECURITY_ATTRIBUTES {
        nLength: size_of::<windows_sys::Win32::Security::SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: 0,
    });
    let sa_ptr = sa.as_ref().map_or(null(), |s| s as *const _);
    // SAFETY: NUL-terminated path; `sa_ptr` is null or points at a live SECURITY_ATTRIBUTES
    // whose descriptor the caller keeps alive for the duration of the call.
    if unsafe { CreateDirectoryW(w.as_ptr(), sa_ptr) } == 0 { Err(last_err()) } else { Ok(()) }
}

/// Raw `FSCTL_GET_REPARSE_POINT` buffer of the link itself.
pub(crate) fn get_reparse_data(path: &Path) -> io::Result<Vec<u8>> {
    let h = open(path, FILE_READ_ATTRIBUTES, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)?;
    let mut buf = vec![0u8; MAXIMUM_REPARSE_DATA_BUFFER_SIZE];
    let mut ret = 0u32;
    // SAFETY: valid handle; output buffer of the length we pass; no overlapped I/O.
    let ok = unsafe {
        DeviceIoControl(
            h.raw(),
            FSCTL_GET_REPARSE_POINT,
            null(),
            0,
            buf.as_mut_ptr().cast(),
            buf.len() as u32,
            &mut ret,
            null_mut(),
        )
    };
    if ok == 0 {
        return Err(last_err());
    }
    buf.truncate(ret as usize);
    Ok(buf)
}

/// `FSCTL_SET_REPARSE_POINT` on an existing (empty) directory.
pub(crate) fn set_reparse_data(dir: &Path, data: &[u8]) -> io::Result<()> {
    let h = open(dir, GENERIC_WRITE, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)?;
    let mut ret = 0u32;
    // SAFETY: valid handle; input buffer of the length we pass; no overlapped I/O.
    let ok = unsafe {
        DeviceIoControl(
            h.raw(),
            FSCTL_SET_REPARSE_POINT,
            data.as_ptr().cast(),
            data.len() as u32,
            null_mut(),
            0,
            &mut ret,
            null_mut(),
        )
    };
    if ok == 0 { Err(last_err()) } else { Ok(()) }
}

/// `SetFileTime`; `None` leaves a timestamp unchanged.
pub(crate) fn set_times(
    path: &Path,
    follow: bool,
    created: Option<u64>,
    accessed: Option<u64>,
    written: Option<u64>,
) -> io::Result<()> {
    let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
    if !follow {
        flags |= FILE_FLAG_OPEN_REPARSE_POINT;
    }
    let h = open(path, FILE_WRITE_ATTRIBUTES, flags)?;
    set_times_handle(h.raw(), created, accessed, written)
}

pub(crate) fn set_times_handle(
    h: HANDLE,
    created: Option<u64>,
    accessed: Option<u64>,
    written: Option<u64>,
) -> io::Result<()> {
    let c = created.map(u64_ft);
    let a = accessed.map(u64_ft);
    let w = written.map(u64_ft);
    let p = |x: &Option<FILETIME>| x.as_ref().map_or(null(), |f| f as *const FILETIME);
    // SAFETY: valid handle with FILE_WRITE_ATTRIBUTES; each pointer is null or points at a
    // live FILETIME on this stack frame.
    if unsafe { SetFileTime(h, p(&c), p(&a), p(&w)) } == 0 { Err(last_err()) } else { Ok(()) }
}

struct CopyCtx<'a> {
    f: &'a mut dyn FnMut(u64, u64) -> bool,
    panic: Option<Box<dyn Any + Send>>,
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn copy_progress(
    total: i64,
    transferred: i64,
    _stream_size: i64,
    _stream_transferred: i64,
    _stream_no: u32,
    _reason: u32,
    _src: HANDLE,
    _dst: HANDLE,
    data: *const c_void,
) -> u32 {
    // SAFETY: `data` is the `&mut CopyCtx` passed to `CopyFileExW` by `copy_file_ex`; it
    // outlives the (synchronous) call and is only touched from this callback, which runs on
    // the copying thread.
    let ctx = unsafe { &mut *(data as *mut CopyCtx<'_>) };
    if ctx.panic.is_some() {
        return PROGRESS_CANCEL;
    }
    // An `extern "system"` fn must never unwind into kernel32.
    match catch_unwind(AssertUnwindSafe(|| (ctx.f)(transferred.max(0) as u64, total.max(0) as u64))) {
        Ok(true) => PROGRESS_CONTINUE,
        Ok(false) => PROGRESS_CANCEL,
        Err(p) => {
            ctx.panic = Some(p);
            PROGRESS_CANCEL
        }
    }
}

/// `CopyFileExW(src, dst, progress, flags | COPY_FILE_FAIL_IF_EXISTS)`. `on_progress(done,
/// total)` returns `false` to cancel (the partial destination is deleted by Windows).
/// A panic inside `on_progress` cancels the copy and is resumed after the call returns.
pub(crate) fn copy_file_ex(
    src: &Path,
    dst: &Path,
    flags: u32,
    on_progress: &mut dyn FnMut(u64, u64) -> bool,
) -> io::Result<()> {
    let s = wide_path(src);
    let d = wide_path(dst);
    let mut ctx = CopyCtx { f: on_progress, panic: None };
    let mut cancel = 0;
    // SAFETY: NUL-terminated paths; the callback receives `&mut ctx` (live for the whole
    // synchronous call); `cancel` is a valid BOOL.
    let ok = unsafe {
        CopyFileExW(
            s.as_ptr(),
            d.as_ptr(),
            Some(copy_progress),
            (&raw mut ctx).cast(),
            &mut cancel,
            flags | COPY_FILE_FAIL_IF_EXISTS,
        )
    };
    let err = if ok == 0 { Some(last_err()) } else { None };
    if let Some(p) = ctx.panic.take() {
        resume_unwind(p);
    }
    match err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn ioctl(h: HANDLE, code: u32, input: &[u8], output: &mut [u8]) -> io::Result<u32> {
    let mut ret = 0u32;
    // SAFETY: valid handle; buffers of the lengths we pass; no overlapped I/O.
    let ok = unsafe {
        DeviceIoControl(
            h,
            code,
            if input.is_empty() { null() } else { input.as_ptr().cast() },
            input.len() as u32,
            if output.is_empty() { null_mut() } else { output.as_mut_ptr().cast() },
            output.len() as u32,
            &mut ret,
            null_mut(),
        )
    };
    if ok == 0 { Err(last_err()) } else { Ok(ret) }
}

/// `FSCTL_SET_SPARSE` (`FILE_SET_SPARSE_BUFFER { SetSparse }`).
pub(crate) fn set_sparse(f: &File, on: bool) -> io::Result<()> {
    ioctl(raw(f), FSCTL_SET_SPARSE, &[u8::from(on)], &mut []).map(drop)
}

/// `FSCTL_SET_COMPRESSION` (`COMPRESSION_FORMAT_DEFAULT` / `NONE`) on a path.
pub(crate) fn set_compression(path: &Path, on: bool) -> io::Result<()> {
    let h = open(
        path,
        windows_sys::Win32::Storage::FileSystem::FILE_GENERIC_READ | windows_sys::Win32::Foundation::GENERIC_WRITE,
        FILE_FLAG_BACKUP_SEMANTICS,
    )?;
    let fmt: u16 = if on { 1 } else { 0 };
    ioctl(h.raw(), FSCTL_SET_COMPRESSION, &fmt.to_le_bytes(), &mut []).map(drop)
}

/// Allocated (non-hole) ranges `(offset, length)` of a sparse file.
pub(crate) fn allocated_ranges(f: &File, size: u64) -> io::Result<Vec<(u64, u64)>> {
    const ENTRY: usize = size_of::<FILE_ALLOCATED_RANGE_BUFFER>();
    let mut out = Vec::new();
    let mut start = 0u64;
    let mut buf = vec![0u8; ENTRY * 512];
    while start < size {
        let mut input = Vec::with_capacity(ENTRY);
        input.extend_from_slice(&(start as i64).to_le_bytes());
        input.extend_from_slice(&((size - start) as i64).to_le_bytes());
        let (n, more) = match ioctl(raw(f), FSCTL_QUERY_ALLOCATED_RANGES, &input, &mut buf) {
            Ok(n) => (n as usize, false),
            Err(e) if e.raw_os_error() == Some(ERROR_MORE_DATA as i32) => (buf.len(), true),
            Err(e) => return Err(e),
        };
        let mut last_end = start;
        for chunk in buf[..n - n % ENTRY].as_chunks::<ENTRY>().0 {
            let off = i64::from_le_bytes(chunk[..8].try_into().expect("8 bytes")) as u64;
            let len = i64::from_le_bytes(chunk[8..16].try_into().expect("8 bytes")) as u64;
            out.push((off, len));
            last_end = off + len;
        }
        if !more || last_end <= start {
            break;
        }
        start = last_end;
    }
    Ok(out)
}

/// `SetFileInformationByHandle(FileEndOfFileInfo)`.
pub(crate) fn set_end_of_file(f: &File, size: u64) -> io::Result<()> {
    let info = FILE_END_OF_FILE_INFO { EndOfFile: size as i64 };
    // SAFETY: valid handle opened for writing; input buffer of the size we pass.
    let ok = unsafe {
        SetFileInformationByHandle(
            raw(f),
            FileEndOfFileInfo,
            (&raw const info).cast(),
            size_of::<FILE_END_OF_FILE_INFO>() as u32,
        )
    };
    if ok == 0 { Err(last_err()) } else { Ok(()) }
}

/// `FSCTL_DUPLICATE_EXTENTS_TO_FILE` (ReFS / Dev Drive block cloning).
pub(crate) fn duplicate_extents(dst: &File, src: &File, src_off: u64, dst_off: u64, len: u64) -> io::Result<()> {
    let data = DUPLICATE_EXTENTS_DATA {
        FileHandle: raw(src),
        SourceFileOffset: src_off as i64,
        TargetFileOffset: dst_off as i64,
        ByteCount: len as i64,
    };
    // SAFETY: `data` is a live, correctly laid out input struct; reinterpreting it as bytes
    // for the duration of the call is sound (no padding is read by us).
    let bytes =
        unsafe { std::slice::from_raw_parts((&raw const data).cast::<u8>(), size_of::<DUPLICATE_EXTENTS_DATA>()) };
    ioctl(raw(dst), FSCTL_DUPLICATE_EXTENTS_TO_FILE, bytes, &mut []).map(drop)
}

/// Named alternate data streams of `path` (`(":name:$DATA", size)`), excluding the
/// unnamed default stream.
pub(crate) fn streams(path: &Path) -> io::Result<Vec<(OsString, u64)>> {
    let w = wide_path(path);
    // SAFETY: plain-old-data struct.
    let mut data: WIN32_FIND_STREAM_DATA = unsafe { zeroed() };
    // SAFETY: NUL-terminated path and an out-buffer of the type FindStreamInfoStandard expects.
    let h = unsafe { FindFirstStreamW(w.as_ptr(), FindStreamInfoStandard, (&raw mut data).cast(), 0) };
    if h == INVALID_HANDLE_VALUE {
        let e = last_err();
        // ERROR_HANDLE_EOF: no streams at all (directories without ADS).
        if e.raw_os_error() == Some(ERROR_HANDLE_EOF as i32) {
            return Ok(Vec::new());
        }
        return Err(e);
    }
    let mut out = Vec::new();
    loop {
        let name = super::from_wide_nul(&data.cStreamName);
        if name != "::$DATA" {
            out.push((name, data.StreamSize.max(0) as u64));
        }
        // SAFETY: valid stream search handle and out-buffer.
        if unsafe { FindNextStreamW(h, (&raw mut data).cast()) } == 0 {
            break;
        }
    }
    // SAFETY: valid search handle, closed once.
    unsafe {
        FindClose(h);
    }
    Ok(out)
}

/// Raw handle of a std file (for volume queries on an open file).
pub(crate) fn file_handle(f: &File) -> HANDLE {
    raw(f)
}
