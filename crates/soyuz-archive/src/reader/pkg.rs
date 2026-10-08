//! Linux packages browsed as folders (read-only):
//! - `.deb`: an `ar` archive; its `data.tar[.gz|.xz|.zst|.bz2|.lzma]` member is listed
//!   (the installed file tree). Control files are not shown.
//! - `.rpm`: lead + signature header + header, then the payload: a cpio ("newc" / "crc")
//!   archive compressed with gzip, xz, zstd, bzip2 or lzma (detected by magic).
//!
//! The payload is one sequential stream: entry offsets are recorded while listing, and
//! extraction / testing decode it once (`seq.rs`). Opening one entry decodes up to it.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use soyuz_core::{OpControl, ProgressSink};

use super::Listing;
use super::iso::corrupt;
use super::seq::{self, StreamSource};
use crate::util::{CHUNK, EntryExtra, fs_err, normalize_name, read_err, unix_secs_to_time};
use crate::{
    ArchiveEntry, ArchiveError, ArchiveFormat, ArchiveReader, DirIndex, ExtractOptions, ExtractSink, ExtractSummary,
    TestReport,
};

/// Compression of the payload.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Codec {
    None,
    Gz,
    Xz,
    Zst,
    Bz2,
    Lzma,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Inner {
    Tar,
    Cpio,
}

#[derive(Clone, Debug)]
struct Payload {
    path: PathBuf,
    offset: u64,
    len: u64,
    codec: Codec,
    inner: Inner,
}

impl Payload {
    /// The decompressed payload stream.
    fn stream(&self) -> Result<Box<dyn Read + Send>, ArchiveError> {
        let mut f = File::open(&self.path).map_err(|e| fs_err("open_archive", &self.path, &e))?;
        f.seek(SeekFrom::Start(self.offset))?;
        let raw = BufReader::with_capacity(CHUNK, f.take(self.len));
        Ok(match self.codec {
            Codec::None => Box::new(raw),
            Codec::Gz => Box::new(flate2::read::MultiGzDecoder::new(raw)),
            Codec::Xz => Box::new(lzma_rust2::XzReader::new(raw, true)),
            #[cfg(feature = "zstd")]
            Codec::Zst => Box::new(zstd::stream::read::Decoder::with_buffer(raw).map_err(ArchiveError::from)?),
            #[cfg(not(feature = "zstd"))]
            Codec::Zst => Box::new(super::zst::ZstdReader::new(raw)),
            Codec::Bz2 => Box::new(bzip2::read::MultiBzDecoder::new(raw)),
            Codec::Lzma => Box::new(
                lzma_rust2::LzmaReader::new_mem_limit(raw, 1 << 20, None)
                    .map_err(|e| ArchiveError::Corrupt(format!("lzma: {e}")))?,
            ),
        })
    }
}

impl StreamSource for Payload {
    fn open_stream(&self, _stream: usize) -> Result<Box<dyn Read + Send>, ArchiveError> {
        self.stream()
    }
}

fn codec_by_magic(head: &[u8]) -> Codec {
    if head.starts_with(&[0x1F, 0x8B]) {
        Codec::Gz
    } else if head.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0]) {
        Codec::Xz
    } else if head.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        Codec::Zst
    } else if head.starts_with(b"BZh") {
        Codec::Bz2
    } else if head.len() >= 3 && head[0] == 0x5D && head[1] == 0 && head[2] == 0 {
        Codec::Lzma
    } else {
        Codec::None
    }
}

/// Locate `data.tar*` in a .deb.
fn deb_payload(path: &Path, f: &mut BufReader<File>, size: u64) -> Result<Payload, ArchiveError> {
    let mut magic = [0u8; 8];
    f.read_exact(&mut magic).map_err(|_| corrupt("not a Debian package"))?;
    if &magic != b"!<arch>\n" {
        return Err(corrupt("not a Debian package"));
    }
    let mut pos = 8u64;
    while pos + 60 <= size {
        f.seek(SeekFrom::Start(pos))?;
        let mut h = [0u8; 60];
        f.read_exact(&mut h)?;
        if &h[58..60] != b"`\n" {
            return Err(corrupt("bad ar member header"));
        }
        let name = String::from_utf8_lossy(&h[..16]).trim_end().trim_end_matches('/').to_string();
        let len: u64 = std::str::from_utf8(&h[48..58])
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .ok_or_else(|| corrupt("bad ar member size"))?;
        let data = pos + 60;
        if data.checked_add(len).is_none_or(|end| end > size) {
            return Err(corrupt("ar member runs past the end of the file"));
        }
        if let Some(ext) = name.strip_prefix("data.tar") {
            let codec = match ext {
                "" => Codec::None,
                ".gz" => Codec::Gz,
                ".xz" => Codec::Xz,
                ".zst" => Codec::Zst,
                ".bz2" => Codec::Bz2,
                ".lzma" => Codec::Lzma,
                other => return Err(corrupt(format!("unsupported data.tar compression {other:?}"))),
            };
            return Ok(Payload { path: path.to_path_buf(), offset: data, len, codec, inner: Inner::Tar });
        }
        pos = data + len + (len & 1);
    }
    Err(corrupt("no data.tar member in the package"))
}

/// Size of an RPM header structure starting at `pos` (magic, index entries, data).
fn rpm_header_len(f: &mut BufReader<File>, pos: u64) -> Result<u64, ArchiveError> {
    f.seek(SeekFrom::Start(pos))?;
    let mut h = [0u8; 16];
    f.read_exact(&mut h).map_err(|_| corrupt("truncated RPM header"))?;
    if h[..3] != [0x8E, 0xAD, 0xE8] {
        return Err(corrupt("bad RPM header magic"));
    }
    let n = u64::from(u32::from_be_bytes([h[8], h[9], h[10], h[11]]));
    let data = u64::from(u32::from_be_bytes([h[12], h[13], h[14], h[15]]));
    if n > 1 << 20 || data > 1 << 30 {
        return Err(corrupt("RPM header too large"));
    }
    Ok(16 + n * 16 + data)
}

fn rpm_payload(path: &Path, f: &mut BufReader<File>, size: u64) -> Result<Payload, ArchiveError> {
    let mut lead = [0u8; 96];
    f.read_exact(&mut lead).map_err(|_| corrupt("not an RPM package"))?;
    if lead[..4] != [0xED, 0xAB, 0xEE, 0xDB] {
        return Err(corrupt("not an RPM package"));
    }
    // Signature header (padded to 8 bytes), then the main header.
    let sig = rpm_header_len(f, 96)?;
    let main_at = 96 + sig.div_ceil(8) * 8;
    let main = rpm_header_len(f, main_at)?;
    let offset = main_at + main;
    if offset >= size {
        return Err(corrupt("RPM has no payload"));
    }
    f.seek(SeekFrom::Start(offset))?;
    let mut head = [0u8; 8];
    let n = f.read(&mut head)?;
    let codec = codec_by_magic(&head[..n]);
    Ok(Payload { path: path.to_path_buf(), offset, len: size - offset, codec, inner: Inner::Cpio })
}

/// Counts bytes read (the decoded position).
struct Counting<R> {
    inner: R,
    pos: u64,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

const S_IFMT: u32 = 0o170000;
const S_IFDIR: u32 = 0o040000;
const S_IFREG: u32 = 0o100000;
const S_IFLNK: u32 = 0o120000;

fn hex8(b: &[u8]) -> Result<u64, ArchiveError> {
    let s = std::str::from_utf8(b).map_err(|_| corrupt("bad cpio header"))?;
    u64::from_str_radix(s, 16).map_err(|_| corrupt("bad cpio header"))
}

fn skip_exact(r: &mut impl Read, n: u64) -> Result<(), ArchiveError> {
    let got = io::copy(&mut r.take(n), &mut io::sink()).map_err(|e| read_err(e, None, false))?;
    if got != n {
        return Err(corrupt("unexpected end of the cpio stream"));
    }
    Ok(())
}

/// List a cpio "newc" / "crc" stream.
fn list_cpio(r: Box<dyn Read + Send>) -> Result<(Vec<ArchiveEntry>, Vec<EntryExtra>), ArchiveError> {
    let mut r = Counting { inner: r, pos: 0 };
    let mut entries: Vec<ArchiveEntry> = Vec::new();
    let mut extras: Vec<EntryExtra> = Vec::new();
    // (dev, ino) → entry index, for hard links (data sits on the last link).
    let mut inodes: std::collections::HashMap<(u64, u64, u64), usize> = Default::default();
    loop {
        let mut h = [0u8; 110];
        r.read_exact(&mut h).map_err(|e| read_err(e, None, false))?;
        if &h[..6] != b"070701" && &h[..6] != b"070702" {
            return Err(corrupt("unsupported cpio format (only newc / crc are)"));
        }
        let f = |i: usize| hex8(&h[6 + 8 * i..14 + 8 * i]);
        let (ino, mode, mtime, nlink) = (f(0)?, f(1)? as u32, f(5)?, f(4)?);
        let (size, devmaj, devmin, namesize) = (f(6)?, f(7)?, f(8)?, f(11)?);
        if namesize == 0 || namesize > 64 * 1024 {
            return Err(corrupt("bad cpio name size"));
        }
        let mut name = vec![0u8; namesize as usize];
        r.read_exact(&mut name).map_err(|e| read_err(e, None, false))?;
        name.pop(); // NUL
        skip_exact(&mut r, (4 - (110 + namesize) % 4) % 4)?;
        let name = String::from_utf8_lossy(&name).into_owned();
        if name == "TRAILER!!!" {
            break;
        }
        let data_pos = r.pos;
        let kind = mode & S_IFMT;
        let mut link_target = None;
        if kind == S_IFLNK {
            let mut t = vec![0u8; size.min(64 * 1024) as usize];
            r.read_exact(&mut t).map_err(|e| read_err(e, None, false))?;
            skip_exact(&mut r, size - t.len() as u64)?;
            link_target = Some(String::from_utf8_lossy(&t).into_owned());
        } else {
            skip_exact(&mut r, size)?;
        }
        skip_exact(&mut r, (4 - size % 4) % 4)?;
        if !matches!(kind, S_IFDIR | S_IFREG | S_IFLNK) {
            continue; // devices, FIFOs, sockets
        }
        let (path, raw) = normalize_name(&name);
        if path.is_empty() {
            continue;
        }
        let idx = entries.len();
        entries.push(ArchiveEntry {
            index: idx,
            path,
            is_dir: kind == S_IFDIR,
            size: if kind == S_IFREG { size } else { 0 },
            compressed: size,
            modified: unix_secs_to_time(mtime as i64),
            crc32: None,
            encrypted: false,
            mode: Some(mode & 0o7777),
            is_symlink: kind == S_IFLNK,
            link_target,
        });
        extras.push(EntryExtra { raw, data_pos, ordinal: 0, ..Default::default() });
        if kind == S_IFREG && nlink > 1 {
            let key = (devmaj, devmin, ino);
            match inodes.get(&key).copied() {
                // A later link of the same inode: the data is on the last one, so the
                // first entry takes the data and the others become hard links to it.
                Some(first) => {
                    if size > 0 {
                        entries[first].size = size;
                        entries[first].compressed = size;
                        extras[first].data_pos = data_pos;
                    }
                    let target = entries[first].path.clone();
                    entries[idx].size = 0;
                    entries[idx].link_target = Some(target);
                    extras[idx].hardlink = true;
                }
                None => {
                    inodes.insert(key, idx);
                }
            }
        }
    }
    Ok((entries, extras))
}

/// List the tar payload of a .deb.
fn list_tar(r: Box<dyn Read + Send>) -> Result<(Vec<ArchiveEntry>, Vec<EntryExtra>), ArchiveError> {
    let mut entries = Vec::new();
    let mut extras = Vec::new();
    let mut ar = tar::Archive::new(r);
    for (ordinal, e) in ar.entries().map_err(|e| read_err(e, None, false))?.enumerate() {
        let mut e = e.map_err(|e| read_err(e, None, false))?;
        if let Some((mut entry, mut extra)) =
            super::tar::describe_entry(&mut e, ordinal).map_err(|e| read_err(e, None, false))?
        {
            if extra.sparse {
                return Err(corrupt("sparse files in packages are not supported"));
            }
            entry.index = entries.len();
            extra.ordinal = 0;
            entries.push(entry);
            extras.push(extra);
        }
    }
    Ok((entries, extras))
}

pub(crate) struct PkgReader {
    format: ArchiveFormat,
    payload: Payload,
    listing: Listing,
}

impl PkgReader {
    pub fn open(path: &Path, format: ArchiveFormat) -> Result<Self, ArchiveError> {
        let file = File::open(path).map_err(|e| fs_err("open_archive", path, &e))?;
        let size = file.metadata().map_err(|e| fs_err("open_archive", path, &e))?.len();
        let mut f = BufReader::new(file);
        let payload = match format {
            ArchiveFormat::Deb => deb_payload(path, &mut f, size)?,
            _ => rpm_payload(path, &mut f, size)?,
        };
        let stream = payload.stream()?;
        let (entries, extras) = match payload.inner {
            Inner::Tar => list_tar(stream)?,
            Inner::Cpio => list_cpio(stream)?,
        };
        Ok(PkgReader { format, payload, listing: Listing { entries, extras, index: None } })
    }
}

impl ArchiveReader for PkgReader {
    fn format(&self) -> ArchiveFormat {
        self.format
    }

    fn path(&self) -> &Path {
        &self.payload.path
    }

    fn entries(&mut self) -> Result<&[ArchiveEntry], ArchiveError> {
        Ok(&self.listing.entries)
    }

    fn dir_index(&mut self) -> Result<Option<&DirIndex>, ArchiveError> {
        Ok(Some(self.listing.index()))
    }

    fn open_entry(&mut self, index: usize) -> Result<Box<dyn Read + Send + '_>, ArchiveError> {
        seq::open_entry(&self.payload, &self.listing, index)
    }

    fn extract(
        &mut self,
        selection: Option<&[usize]>,
        dest: &Path,
        opts: &ExtractOptions,
        ctl: &OpControl,
        sink: &mut dyn ExtractSink,
    ) -> Result<ExtractSummary, ArchiveError> {
        let size = std::fs::metadata(&self.payload.path).map_or(0, |m| m.len());
        seq::extract(&self.payload, &self.listing, size, selection, dest, opts, ctl, sink)
    }

    fn test(&mut self, ctl: &OpControl, sink: &mut dyn ProgressSink) -> Result<TestReport, ArchiveError> {
        seq::test(&self.payload, &self.listing, ctl, sink)
    }
}
