//! zip writer.

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::Path;

use soyuz_core::{OpControl, ProgressSink};
use zip::write::{FullFileOptions, ZipWriter};
use zip::{AesMode, CompressionMethod, DateTime};

use super::{Item, ItemKind};
use crate::reader::zip_err;
use crate::util::{ProgressReader, fs_err, system_to_local, time_to_unix_secs};
use crate::{ArchiveError, ZipMethod};

/// zstd levels for UI levels 1..=9.
#[cfg(feature = "zstd")]
const ZSTD_LEVELS: [i64; 10] = [0, 1, 2, 3, 5, 7, 9, 12, 15, 19];

pub(crate) fn write(
    part: &Path,
    items: &[Item],
    level: u8,
    password: Option<&str>,
    method: ZipMethod,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> Result<(), ArchiveError> {
    let (method, zlevel) = match (method, level) {
        (_, 0) | (ZipMethod::Store, _) => (CompressionMethod::Stored, None),
        (ZipMethod::Deflate, l) => (CompressionMethod::Deflated, Some(i64::from(l))),
        #[cfg(feature = "zstd")]
        (ZipMethod::Zstd, l) => (CompressionMethod::Zstd, Some(ZSTD_LEVELS[usize::from(l)])),
        #[cfg(not(feature = "zstd"))]
        (ZipMethod::Zstd, _) => return Err(ArchiveError::Unsupported(crate::ArchiveFormat::Zip)),
    };
    let f = File::create(part).map_err(|e| fs_err("create", part, &e))?;
    let mut zw = ZipWriter::new(BufWriter::with_capacity(crate::util::CHUNK, f)).set_auto_large_file();
    for item in items {
        ctl.checkpoint()?;
        let mut o: FullFileOptions<'_> =
            FullFileOptions::default().compression_method(method).compression_level(zlevel);
        if let Some(t) = item.modified() {
            let (y, mo, d, h, mi, s) = system_to_local(t);
            if let Ok(dt) =
                DateTime::from_date_and_time(y.clamp(1980, 2107) as u16, mo as u8, d as u8, h as u8, mi as u8, s as u8)
            {
                o = o.last_modified_time(dt);
            }
            // Extended timestamp (UTC, 1 s precision): flags=mtime, u32 secs.
            let secs = time_to_unix_secs(t);
            if let Ok(secs) = u32::try_from(secs) {
                let mut data = vec![1u8];
                data.extend_from_slice(&secs.to_le_bytes());
                if let Err(e) = o.add_extra_data(0x5455, &data, false) {
                    log::debug!("zip extended timestamp not added: {e}");
                }
            }
        }
        if let Some(m) = item.mode() {
            o = o.unix_permissions(m & 0o7777);
        }
        if let Some(pw) = password {
            o = o.with_aes_encryption(AesMode::Aes256, pw);
        }
        match &item.kind {
            ItemKind::Dir => {
                zw.add_directory(format!("{}/", item.name), o.compression_method(CompressionMethod::Stored))
                    .map_err(zip_err)?;
            }
            ItemKind::Symlink { target } => {
                zw.add_symlink(&item.name, target, o.compression_method(CompressionMethod::Stored)).map_err(zip_err)?;
            }
            ItemKind::File { len } => {
                let o = o.large_file(*len >= u64::from(u32::MAX));
                zw.start_file(&item.name, o).map_err(zip_err)?;
                let src = File::open(&item.src).map_err(|e| fs_err("open", &item.src, &e))?;
                let mut r = ProgressReader::new(src, *len, ctl, sink);
                io::copy(&mut r, &mut zw).map_err(|e| copy_err(e, ctl, &item.src))?;
            }
        }
    }
    let mut w = zw.finish().map_err(zip_err)?;
    io::Write::flush(&mut w).map_err(|e| fs_err("write", part, &e))?;
    Ok(())
}

pub(crate) fn copy_err(e: io::Error, ctl: &OpControl, src: &Path) -> ArchiveError {
    if ctl.is_cancelled() { ArchiveError::Cancelled } else { fs_err("archive", src, &e) }
}
