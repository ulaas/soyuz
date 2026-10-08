//! Extended attributes (unix) / alternate data streams (Windows) copying.

use std::path::Path;

/// Copy all extended attributes from `src` to `dst` (open files). Attributes the target
/// filesystem or our privileges reject (`trusted.*`, `security.*` without CAP_SYS_ADMIN,
/// ENOTSUP) are skipped; the first unexpected error is returned after trying the rest.
#[cfg(unix)]
pub(crate) fn copy_fd(src: &std::fs::File, dst: &std::fs::File) -> std::io::Result<()> {
    use xattr::FileExt;
    let names = match src.list_xattr() {
        Ok(n) => n,
        Err(e) if ignorable(&e) => return Ok(()),
        Err(e) => return Err(e),
    };
    let mut first_err = None;
    for name in names {
        if skip_name(&name) {
            continue;
        }
        let res = src.get_xattr(&name).and_then(|v| match v {
            Some(v) => dst.set_xattr(&name, &v),
            None => Ok(()),
        });
        if let Err(e) = res
            && !ignorable(&e)
            && first_err.is_none()
        {
            first_err = Some(e);
        }
    }
    first_err.map_or(Ok(()), Err)
}

/// Path-based variant (directories); follows symlinks.
#[cfg(unix)]
pub(crate) fn copy_path(src: &Path, dst: &Path) -> std::io::Result<()> {
    let names = match xattr::list(src) {
        Ok(n) => n,
        Err(e) if ignorable(&e) => return Ok(()),
        Err(e) => return Err(e),
    };
    for name in names {
        if skip_name(&name) {
            continue;
        }
        if let Ok(Some(v)) = xattr::get(src, &name)
            && let Err(e) = xattr::set(dst, &name, &v)
            && !ignorable(&e)
        {
            return Err(e);
        }
    }
    Ok(())
}

#[cfg(unix)]
fn skip_name(name: &std::ffi::OsStr) -> bool {
    let n = name.to_string_lossy();
    n.starts_with("trusted.") || n == "security.selinux" || n.starts_with("system.nfs4")
}

#[cfg(unix)]
fn ignorable(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(c) if c == libc::ENOTSUP || c == libc::EOPNOTSUPP || c == libc::EPERM || c == libc::EACCES)
        || e.kind() == std::io::ErrorKind::Unsupported
}

/// Copy the named alternate data streams of `src` to `dst` (the unnamed stream is the
/// file content itself). Used by the manual copy paths; `CopyFileExW` copies streams itself.
#[cfg(windows)]
pub(crate) fn copy_streams(src: &Path, dst: &Path) -> std::io::Result<()> {
    use std::ffi::OsString;
    let streams = match crate::sys::windows::fs::streams(src) {
        Ok(s) => s,
        // Filesystems without stream support (FAT, some SMB servers).
        Err(e) if crate::sys::windows::fs::is_unsupported_error(&e) => return Ok(()),
        Err(e) => return Err(e),
    };
    for (name, _size) in streams {
        let with = |p: &Path| {
            let mut s = OsString::from(crate::path::strip_verbatim(&crate::path::normalize(p)));
            s.push(&name);
            std::path::PathBuf::from(s)
        };
        let mut input = std::fs::File::open(with(src))?;
        let mut output = std::fs::File::create(with(dst))?;
        std::io::copy(&mut input, &mut output)?;
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn streams_are_copied() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.txt");
        let b = d.path().join("b.txt");
        std::fs::write(&a, b"main").unwrap();
        std::fs::write(d.path().join("a.txt:extra"), b"hidden stream").unwrap();
        std::fs::write(&b, b"main").unwrap();
        super::copy_streams(&a, &b).unwrap();
        assert_eq!(std::fs::read(d.path().join("b.txt:extra")).unwrap(), b"hidden stream");
        let names: Vec<_> = crate::sys::windows::fs::streams(&b).unwrap().into_iter().map(|s| s.0).collect();
        assert_eq!(names, [std::ffi::OsString::from(":extra:$DATA")]);
    }
}
