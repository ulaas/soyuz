//! 7z writer (non-solid: one block per file, so browsing / single-entry reads stay fast).

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::Path;

use sevenz_rust2::encoder_options::{AesEncoderOptions, Lzma2Options};
use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod, NtTime, Password};
use soyuz_core::{OpControl, ProgressSink};

use super::{Item, ItemKind};
use crate::ArchiveError;
use crate::reader::{S_IFLNK, UNIX_EXTENSION, sz_err};
use crate::util::{CHUNK, ProgressReader, fs_err};

const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;

pub(crate) fn write(
    part: &Path,
    items: &[Item],
    level: u8,
    password: Option<&str>,
    encrypt_headers: bool,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> Result<(), ArchiveError> {
    let f = File::create(part).map_err(|e| fs_err("create", part, &e))?;
    let map = |e: sevenz_rust2::Error| if ctl.is_cancelled() { ArchiveError::Cancelled } else { sz_err(e, false) };
    let mut w = ArchiveWriter::new(BufWriter::with_capacity(CHUNK, f)).map_err(map)?;
    let codec: EncoderConfiguration =
        if level == 0 { EncoderMethod::COPY.into() } else { Lzma2Options::from_level(u32::from(level)).into() };
    match password {
        Some(pw) => {
            w.set_content_methods(vec![AesEncoderOptions::new(Password::new(pw)).into(), codec]);
            w.set_encrypt_header(encrypt_headers);
        }
        None => {
            w.set_content_methods(vec![codec]);
            w.set_encrypt_header(false);
        }
    }
    for item in items {
        ctl.checkpoint()?;
        let mut e = ArchiveEntry::new();
        e.name = item.name.clone();
        if let Some(t) = item.modified().and_then(|t| NtTime::try_from(t).ok()) {
            e.last_modified_date = t;
            e.has_last_modified_date = true;
        }
        if let Some(t) = item.md.created().ok().and_then(|t| NtTime::try_from(t).ok()) {
            e.creation_date = t;
            e.has_creation_date = true;
        }
        let mut attrs = attributes(item);
        match &item.kind {
            ItemKind::Dir => {
                e.is_directory = true;
                attrs |= FILE_ATTRIBUTE_DIRECTORY;
                e.windows_attributes = attrs;
                e.has_windows_attributes = true;
                w.push_archive_entry::<&[u8]>(e, None).map_err(map)?;
            }
            ItemKind::Symlink { target } => {
                // p7zip convention: a file whose unix mode is S_IFLNK and whose data is the target.
                let mode = item.mode().unwrap_or(0o777) & 0o7777;
                e.windows_attributes = FILE_ATTRIBUTE_ARCHIVE | UNIX_EXTENSION | ((S_IFLNK | mode) << 16);
                e.has_windows_attributes = true;
                e.has_stream = true;
                w.push_archive_entry(e, Some(target.as_bytes())).map_err(map)?;
            }
            ItemKind::File { len } => {
                e.windows_attributes = attrs | FILE_ATTRIBUTE_ARCHIVE;
                e.has_windows_attributes = true;
                let src = File::open(&item.src).map_err(|err| fs_err("open", &item.src, &err))?;
                if *len == 0 {
                    // 7z "empty stream" file.
                    w.push_archive_entry::<File>(e, None).map_err(map)?;
                } else {
                    let r = ProgressReader::new(io::Read::take(src, *len), *len, ctl, sink);
                    w.push_archive_entry(e, Some(r)).map_err(map)?;
                }
            }
        }
    }
    let mut out = w.finish().map_err(|e| super::zip::copy_err(e, ctl, part))?;
    io::Write::flush(&mut out).map_err(|e| fs_err("write", part, &e))?;
    Ok(())
}

/// Windows attributes, plus the unix mode in the high word (p7zip extension) on unix.
fn attributes(item: &Item) -> u32 {
    let mut a = 0;
    if item.md.permissions().readonly() {
        a |= FILE_ATTRIBUTE_READONLY;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Keep hidden/system/readonly; drop reparse/sparse/compressed flags.
        a |= item.md.file_attributes() & 0x7;
    }
    if let Some(mode) = item.mode() {
        let ty = match item.kind {
            ItemKind::Dir => 0o040000,
            ItemKind::Symlink { .. } => S_IFLNK,
            ItemKind::File { .. } => 0o100000,
        };
        a |= UNIX_EXTENSION | ((ty | (mode & 0o7777)) << 16);
    }
    a
}
