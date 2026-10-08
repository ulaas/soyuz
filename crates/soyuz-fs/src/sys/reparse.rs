//! `REPARSE_DATA_BUFFER` (not shipped by windows-sys, which only has the GUID variant).
//!
//! Layout (all little-endian):
//! ```text
//! u32 ReparseTag | u16 ReparseDataLength | u16 Reserved | <tag-specific data>
//! symlink:     u16 SubstOff, u16 SubstLen, u16 PrintOff, u16 PrintLen, u32 Flags, WCHAR PathBuffer[]
//! mount point: u16 SubstOff, u16 SubstLen, u16 PrintOff, u16 PrintLen, WCHAR PathBuffer[]
//! appexeclink: u32 Version(3), then NUL-separated WCHAR strings (package, app id, target exe, ...)
//! ```
//! Offsets/lengths inside `PathBuffer` are in bytes. Parsing works on byte slices with the
//! field offsets taken from the `#[repr(C)]` structs below, so no unaligned casts happen.
#![cfg_attr(not(windows), allow(dead_code))]

use std::io;
use std::mem::{offset_of, size_of};

pub(crate) const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;
pub(crate) const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;
pub(crate) const IO_REPARSE_TAG_APPEXECLINK: u32 = 0x8000_001B;
/// `SYMLINK_FLAG_RELATIVE`: the substitute name is relative to the link's directory.
pub(crate) const SYMLINK_FLAG_RELATIVE: u32 = 1;
/// `MAXIMUM_REPARSE_DATA_BUFFER_SIZE`.
pub(crate) const MAXIMUM_REPARSE_DATA_BUFFER_SIZE: usize = 16 * 1024;

/// Common header of every reparse buffer.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ReparseHeader {
    pub reparse_tag: u32,
    pub reparse_data_length: u16,
    pub reserved: u16,
}

/// `SymbolicLinkReparseBuffer` (the `PathBuffer` flexible array follows).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct SymbolicLinkReparseBuffer {
    pub substitute_name_offset: u16,
    pub substitute_name_length: u16,
    pub print_name_offset: u16,
    pub print_name_length: u16,
    pub flags: u32,
}

/// `MountPointReparseBuffer` (the `PathBuffer` flexible array follows).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct MountPointReparseBuffer {
    pub substitute_name_offset: u16,
    pub substitute_name_length: u16,
    pub print_name_offset: u16,
    pub print_name_length: u16,
}

const HDR: usize = size_of::<ReparseHeader>();

/// A decoded reparse point. Names are raw UTF-16 (no NUL).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reparse {
    Symlink {
        substitute: Vec<u16>,
        print: Vec<u16>,
        relative: bool,
    },
    MountPoint {
        substitute: Vec<u16>,
        print: Vec<u16>,
    },
    /// App execution alias (`WindowsApps\python.exe`); `target` is the real executable.
    AppExecLink {
        target: Vec<u16>,
    },
    Other(u32),
}

fn corrupt(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("malformed reparse data: {msg}"))
}

fn u16_at(b: &[u8], off: usize) -> io::Result<u16> {
    b.get(off..off + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| corrupt("truncated"))
}

fn u32_at(b: &[u8], off: usize) -> io::Result<u32> {
    b.get(off..off + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| corrupt("truncated"))
}

fn utf16_slice(b: &[u8], off: usize, len: usize) -> io::Result<Vec<u16>> {
    if !len.is_multiple_of(2) {
        return Err(corrupt("odd name length"));
    }
    let bytes = b.get(off..off + len).ok_or_else(|| corrupt("name out of bounds"))?;
    Ok(bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect())
}

/// Parse a buffer returned by `FSCTL_GET_REPARSE_POINT`.
pub(crate) fn parse(buf: &[u8]) -> io::Result<Reparse> {
    let tag = u32_at(buf, offset_of!(ReparseHeader, reparse_tag))?;
    let data_len = u16_at(buf, offset_of!(ReparseHeader, reparse_data_length))? as usize;
    let data = buf.get(HDR..HDR + data_len).ok_or_else(|| corrupt("data length exceeds buffer"))?;
    match tag {
        IO_REPARSE_TAG_SYMLINK => {
            type S = SymbolicLinkReparseBuffer;
            let path_buf = size_of::<S>();
            let so = u16_at(data, offset_of!(S, substitute_name_offset))? as usize;
            let sl = u16_at(data, offset_of!(S, substitute_name_length))? as usize;
            let po = u16_at(data, offset_of!(S, print_name_offset))? as usize;
            let pl = u16_at(data, offset_of!(S, print_name_length))? as usize;
            let flags = u32_at(data, offset_of!(S, flags))?;
            Ok(Reparse::Symlink {
                substitute: utf16_slice(data, path_buf + so, sl)?,
                print: utf16_slice(data, path_buf + po, pl)?,
                relative: flags & SYMLINK_FLAG_RELATIVE != 0,
            })
        }
        IO_REPARSE_TAG_MOUNT_POINT => {
            type M = MountPointReparseBuffer;
            let path_buf = size_of::<M>();
            let so = u16_at(data, offset_of!(M, substitute_name_offset))? as usize;
            let sl = u16_at(data, offset_of!(M, substitute_name_length))? as usize;
            let po = u16_at(data, offset_of!(M, print_name_offset))? as usize;
            let pl = u16_at(data, offset_of!(M, print_name_length))? as usize;
            Ok(Reparse::MountPoint {
                substitute: utf16_slice(data, path_buf + so, sl)?,
                print: utf16_slice(data, path_buf + po, pl)?,
            })
        }
        IO_REPARSE_TAG_APPEXECLINK => {
            let version = u32_at(data, 0)?;
            if version != 3 {
                return Err(corrupt("unknown app execution alias version"));
            }
            let words = utf16_slice(data, 4, (data.len() - 4) & !1)?;
            // package id, app user model id, target executable, ...
            let target = words.split(|&c| c == 0).nth(2).map(<[u16]>::to_vec).unwrap_or_default();
            if target.is_empty() {
                return Err(corrupt("app execution alias without target"));
            }
            Ok(Reparse::AppExecLink { target })
        }
        other => Ok(Reparse::Other(other)),
    }
}

fn push_u16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn push_u32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_le_bytes());
}

fn name_bytes(len_units: usize) -> io::Result<u16> {
    u16::try_from(len_units * 2).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "link target too long"))
}

/// Build a mount-point (junction) buffer for `FSCTL_SET_REPARSE_POINT`. `substitute` is the
/// NT path (`\??\C:\target`), `print` the display path (`C:\target`); both without NUL.
pub(crate) fn build_mount_point(substitute: &[u16], print: &[u16]) -> io::Result<Vec<u8>> {
    let sl = name_bytes(substitute.len())?;
    let pl = name_bytes(print.len())?;
    let path_bytes = sl as usize + 2 + pl as usize + 2;
    let data_len = size_of::<MountPointReparseBuffer>() + path_bytes;
    if HDR + data_len > MAXIMUM_REPARSE_DATA_BUFFER_SIZE {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "junction target too long"));
    }
    let mut v = Vec::with_capacity(HDR + data_len);
    push_u32(&mut v, IO_REPARSE_TAG_MOUNT_POINT);
    push_u16(&mut v, data_len as u16);
    push_u16(&mut v, 0);
    push_u16(&mut v, 0); // substitute offset
    push_u16(&mut v, sl);
    push_u16(&mut v, sl + 2); // print offset (after substitute + NUL)
    push_u16(&mut v, pl);
    for &c in substitute.iter().chain(&[0]).chain(print).chain(&[0]) {
        push_u16(&mut v, c);
    }
    Ok(v)
}

/// Build a symbolic-link buffer (used by tests to produce fixtures; links themselves are
/// created with `CreateSymbolicLinkW`).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn build_symlink(substitute: &[u16], print: &[u16], relative: bool) -> io::Result<Vec<u8>> {
    let sl = name_bytes(substitute.len())?;
    let pl = name_bytes(print.len())?;
    let data_len = size_of::<SymbolicLinkReparseBuffer>() + sl as usize + pl as usize;
    let mut v = Vec::with_capacity(HDR + data_len);
    push_u32(&mut v, IO_REPARSE_TAG_SYMLINK);
    push_u16(&mut v, data_len as u16);
    push_u16(&mut v, 0);
    // Windows itself stores the print name first; mimic that layout.
    push_u16(&mut v, pl); // substitute offset
    push_u16(&mut v, sl);
    push_u16(&mut v, 0); // print offset
    push_u16(&mut v, pl);
    push_u32(&mut v, if relative { SYMLINK_FLAG_RELATIVE } else { 0 });
    for &c in print.iter().chain(substitute) {
        push_u16(&mut v, c);
    }
    Ok(v)
}

/// `\??\C:\x` → `C:\x`, `\??\UNC\srv\share` → `\\srv\share`, `\??\Volume{..}\` →
/// `\\?\Volume{..}\`; anything else unchanged.
pub(crate) fn nt_to_dos(name: &str) -> String {
    if let Some(rest) = name.strip_prefix(r"\??\") {
        if let Some(unc) = rest.strip_prefix("UNC\\") {
            return format!(r"\\{unc}");
        }
        let b = rest.as_bytes();
        if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
            return rest.to_string();
        }
        return format!(r"\\?\{rest}");
    }
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn struct_layout_matches_windows_headers() {
        assert_eq!(size_of::<ReparseHeader>(), 8);
        assert_eq!(size_of::<SymbolicLinkReparseBuffer>(), 12);
        assert_eq!(size_of::<MountPointReparseBuffer>(), 8);
    }

    #[test]
    fn mount_point_round_trip() {
        let buf = build_mount_point(&w(r"\??\C:\target dir"), &w(r"C:\target dir")).unwrap();
        // Header: tag + data length; the data length covers everything after the header.
        assert_eq!(&buf[..4], &IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
        assert_eq!(u16::from_le_bytes([buf[4], buf[5]]) as usize, buf.len() - 8);
        match parse(&buf).unwrap() {
            Reparse::MountPoint { substitute, print } => {
                assert_eq!(String::from_utf16(&substitute).unwrap(), r"\??\C:\target dir");
                assert_eq!(String::from_utf16(&print).unwrap(), r"C:\target dir");
            }
            other => panic!("{other:?}"),
        }
    }

    /// Layout as returned by `fsutil reparsepoint query` for `mklink /D rel ..\x`
    /// (relative symlink, print name stored first).
    #[test]
    fn symlink_fixture() {
        let fixture: Vec<u8> = {
            let mut v = vec![0x0C, 0x00, 0x00, 0xA0, 0x1C, 0x00, 0x00, 0x00];
            // subst off 8, len 8, print off 0, len 8, flags 1
            v.extend_from_slice(&[0x08, 0x00, 0x08, 0x00, 0x00, 0x00, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00]);
            for _ in 0..2 {
                for c in "..\\x".encode_utf16() {
                    v.extend_from_slice(&c.to_le_bytes());
                }
            }
            v
        };
        let parsed = parse(&fixture).unwrap();
        assert_eq!(parsed, Reparse::Symlink { substitute: w(r"..\x"), print: w(r"..\x"), relative: true });
        let built = build_symlink(&w(r"..\x"), &w(r"..\x"), true).unwrap();
        assert_eq!(built, fixture);
        let abs = build_symlink(&w(r"\??\C:\Users"), &w(r"C:\Users"), false).unwrap();
        assert_eq!(
            parse(&abs).unwrap(),
            Reparse::Symlink { substitute: w(r"\??\C:\Users"), print: w(r"C:\Users"), relative: false }
        );
    }

    #[test]
    fn app_exec_link_and_errors() {
        let mut v = Vec::new();
        push_u32(&mut v, IO_REPARSE_TAG_APPEXECLINK);
        let mut data = Vec::new();
        push_u32(&mut data, 3);
        for s in ["Pkg_1.0", "Pkg!App", r"C:\Program Files\WindowsApps\Pkg\app.exe", "0"] {
            for c in s.encode_utf16().chain([0]) {
                push_u16(&mut data, c);
            }
        }
        push_u16(&mut v, data.len() as u16);
        push_u16(&mut v, 0);
        v.extend_from_slice(&data);
        match parse(&v).unwrap() {
            Reparse::AppExecLink { target } => {
                assert_eq!(String::from_utf16(&target).unwrap(), r"C:\Program Files\WindowsApps\Pkg\app.exe")
            }
            other => panic!("{other:?}"),
        }
        assert!(parse(&[1, 2, 3]).is_err());
        let mut bad = build_mount_point(&w("a"), &w("b")).unwrap();
        bad[4] = 0xFF; // data length beyond the buffer
        assert!(parse(&bad).is_err());
        assert_eq!(parse(&[0x17, 0, 0, 0x80, 0, 0, 0, 0]).unwrap(), Reparse::Other(0x8000_0017));
    }

    #[test]
    fn nt_names() {
        assert_eq!(nt_to_dos(r"\??\C:\x"), r"C:\x");
        assert_eq!(nt_to_dos(r"\??\UNC\srv\share\d"), r"\\srv\share\d");
        assert_eq!(nt_to_dos(r"\??\Volume{1234}\"), r"\\?\Volume{1234}\");
        assert_eq!(nt_to_dos(r"..\rel"), r"..\rel");
    }
}
