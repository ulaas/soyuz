//! tar / tar.gz / tar.bz2 / tar.xz / tar.zst writer.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use soyuz_core::{OpControl, ProgressSink};
use tar::{Builder, EntryType, Header, HeaderMode};

use super::zip::copy_err;
use super::{Item, ItemKind};
use crate::util::{CHUNK, ProgressReader, fs_err, time_to_unix_secs};
use crate::{ArchiveError, ArchiveFormat};

/// zstd levels for UI levels 0..=9.
#[cfg(feature = "zstd")]
const ZSTD_LEVELS: [i32; 10] = [1, 1, 2, 3, 5, 7, 9, 12, 15, 19];

pub(crate) fn write(
    part: &Path,
    items: &[Item],
    format: ArchiveFormat,
    level: u8,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> Result<(), ArchiveError> {
    let f = File::create(part).map_err(|e| fs_err("create", part, &e))?;
    let w = BufWriter::with_capacity(CHUNK, f);
    let io_err = |e: io::Error| copy_err(e, ctl, part);
    let l = u32::from(level);
    let mut out = match format {
        ArchiveFormat::Tar => write_entries(w, items, ctl, sink)?,
        ArchiveFormat::TarGz => {
            let enc = flate2::write::GzEncoder::new(w, flate2::Compression::new(l));
            write_entries(enc, items, ctl, sink)?.finish().map_err(io_err)?
        }
        ArchiveFormat::TarBz2 => {
            let enc = bzip2::write::BzEncoder::new(w, bzip2::Compression::new(l.clamp(1, 9)));
            write_entries(enc, items, ctl, sink)?.finish().map_err(io_err)?
        }
        ArchiveFormat::TarXz => {
            let enc = lzma_rust2::XzWriter::new(w, lzma_rust2::XzOptions::with_preset(l)).map_err(io_err)?;
            write_entries(enc, items, ctl, sink)?.finish().map_err(io_err)?
        }
        #[cfg(feature = "zstd")]
        ArchiveFormat::TarZst => {
            let mut enc = zstd::stream::write::Encoder::new(w, ZSTD_LEVELS[usize::from(level)]).map_err(io_err)?;
            enc.include_checksum(true).map_err(io_err)?;
            write_entries(enc, items, ctl, sink)?.finish().map_err(io_err)?
        }
        other => return Err(ArchiveError::Unsupported(other)),
    };
    out.flush().map_err(io_err)?;
    Ok(())
}

fn write_entries<W: Write>(
    w: W,
    items: &[Item],
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> Result<W, ArchiveError> {
    let mut b = Builder::new(w);
    b.follow_symlinks(false);
    for item in items {
        ctl.checkpoint()?;
        let mut h = Header::new_gnu();
        h.set_metadata_in_mode(&item.md, HeaderMode::Complete);
        if let Some(t) = item.modified() {
            h.set_mtime(time_to_unix_secs(t).max(0) as u64);
        }
        let res = match &item.kind {
            ItemKind::Dir => {
                h.set_entry_type(EntryType::Directory);
                h.set_size(0);
                if item.mode().is_none() {
                    h.set_mode(0o755);
                }
                b.append_data(&mut h, format!("{}/", item.name), io::empty())
            }
            ItemKind::Symlink { target } => {
                h.set_entry_type(EntryType::Symlink);
                h.set_size(0);
                if item.mode().is_none() {
                    h.set_mode(0o777);
                }
                b.append_link(&mut h, &item.name, target)
            }
            ItemKind::File { len } => {
                h.set_entry_type(EntryType::Regular);
                h.set_size(*len);
                if item.mode().is_none() {
                    h.set_mode(if item.md.permissions().readonly() { 0o444 } else { 0o644 });
                }
                let src = File::open(&item.src).map_err(|e| fs_err("open", &item.src, &e))?;
                // Exactly `len` bytes: a file that grows meanwhile must not corrupt the stream.
                let r = ProgressReader::new(io::Read::take(src, *len), *len, ctl, sink);
                b.append_data(&mut h, &item.name, r)
            }
        };
        res.map_err(|e| copy_err(e, ctl, &item.src))?;
    }
    b.into_inner().map_err(|e| copy_err(e, ctl, Path::new("tar")))
}
