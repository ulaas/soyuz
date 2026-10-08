//! Microsoft Cabinet reader (read-only): stored, MSZIP and LZX folders. Each folder is
//! one compressed stream, so extraction decodes every folder once, front to back (see
//! `seq.rs`). Quantum folders and files that continue in another cabinet of a set are
//! listed but report `Unsupported` / an error when read.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use soyuz_core::{OpControl, ProgressSink};

use super::Listing;
use super::iso::{corrupt, le16, le32};
use super::seq::{self, NO_STREAM, StreamSource};
use crate::util::{EntryExtra, fs_err, local_to_system, normalize_name};
use crate::{
    ArchiveEntry, ArchiveError, ArchiveFormat, ArchiveReader, DirIndex, ExtractOptions, ExtractSink, ExtractSummary,
    TestReport,
};

const FLAG_PREV: u16 = 0x0001;
const FLAG_NEXT: u16 = 0x0002;
const FLAG_RESERVE: u16 = 0x0004;
const ATTR_UTF: u16 = 0x80;

#[derive(Clone, Copy, Debug)]
enum Method {
    Store,
    MsZip,
    /// Window size in bits (15..=21).
    Lzx(u8),
    Unsupported(u16),
}

#[derive(Clone, Debug)]
struct Folder {
    first_block: u64,
    blocks: u16,
    method: Method,
}

struct CabData {
    path: PathBuf,
    folders: Vec<Folder>,
    data_reserve: u8,
}

impl StreamSource for CabData {
    fn open_stream(&self, stream: usize) -> Result<Box<dyn Read + Send>, ArchiveError> {
        let folder = self.folders.get(stream).ok_or_else(|| corrupt("file data is in another cabinet of the set"))?;
        if let Method::Unsupported(m) = folder.method {
            log::debug!("CAB compression method {m:#x} is not supported");
            return Err(ArchiveError::Unsupported(ArchiveFormat::Cab));
        }
        let f = File::open(&self.path).map_err(|e| fs_err("open_archive", &self.path, &e))?;
        Ok(Box::new(FolderReader::new(BufReader::with_capacity(64 * 1024, f), folder, self.data_reserve)?))
    }
}

enum Decoder {
    Store,
    MsZip { dict: Vec<u8> },
    Lzx(Box<lzxd::Lzxd>),
}

/// Decoded byte stream of one CAB folder.
struct FolderReader {
    f: BufReader<File>,
    next_block: u64,
    blocks_left: u16,
    reserve: u8,
    dec: Decoder,
    buf: Vec<u8>,
    pos: usize,
}

fn window(bits: u8) -> Option<lzxd::WindowSize> {
    use lzxd::WindowSize as W;
    Some(match bits {
        15 => W::KB32,
        16 => W::KB64,
        17 => W::KB128,
        18 => W::KB256,
        19 => W::KB512,
        20 => W::MB1,
        21 => W::MB2,
        _ => return None,
    })
}

/// CFDATA checksum (MS-CAB 2.6): 32-bit XOR of little-endian words, odd tail bytes
/// folded in reverse order, then combined with the size fields.
fn checksum(data: &[u8], seed: u32) -> u32 {
    let mut csum = seed;
    let (words, rest) = data.as_chunks::<4>();
    for c in words {
        csum ^= u32::from_le_bytes(*c);
    }
    let mut ul = 0u32;
    for (i, &b) in rest.iter().enumerate() {
        ul |= u32::from(b) << (8 * (rest.len() - 1 - i));
    }
    csum ^ ul
}

impl FolderReader {
    fn new(f: BufReader<File>, folder: &Folder, reserve: u8) -> Result<Self, ArchiveError> {
        let dec = match folder.method {
            Method::Store => Decoder::Store,
            Method::MsZip => Decoder::MsZip { dict: Vec::new() },
            Method::Lzx(bits) => {
                let w = window(bits).ok_or_else(|| corrupt(format!("bad LZX window size 2^{bits}")))?;
                Decoder::Lzx(Box::new(lzxd::Lzxd::new(w)))
            }
            Method::Unsupported(_) => return Err(ArchiveError::Unsupported(ArchiveFormat::Cab)),
        };
        Ok(FolderReader {
            f,
            next_block: folder.first_block,
            blocks_left: folder.blocks,
            reserve,
            dec,
            buf: Vec::new(),
            pos: 0,
        })
    }

    /// Decode the next CFDATA block into `buf`; `false` at the end of the folder.
    fn load(&mut self) -> io::Result<bool> {
        if self.blocks_left == 0 {
            return Ok(false);
        }
        self.blocks_left -= 1;
        self.f.seek(SeekFrom::Start(self.next_block))?;
        let mut head = [0u8; 8];
        self.f.read_exact(&mut head)?;
        let stored_sum = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let packed = usize::from(u16::from_le_bytes([head[4], head[5]]));
        let unpacked = usize::from(u16::from_le_bytes([head[6], head[7]]));
        let mut reserve = vec![0u8; usize::from(self.reserve)];
        self.f.read_exact(&mut reserve)?;
        let mut data = vec![0u8; packed];
        self.f.read_exact(&mut data)?;
        self.next_block += 8 + u64::from(self.reserve) + packed as u64;
        if stored_sum != 0 {
            let mut both = reserve;
            both.extend_from_slice(&data);
            let sum = checksum(&both, 0) ^ (packed as u32 | ((unpacked as u32) << 16));
            if sum != stored_sum {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "CAB data block checksum mismatch"));
            }
        }
        let bad = |m: String| io::Error::new(io::ErrorKind::InvalidData, m);
        self.buf = match &mut self.dec {
            Decoder::Store => {
                if packed != unpacked {
                    return Err(bad("stored CAB block with mismatched sizes".into()));
                }
                data
            }
            Decoder::MsZip { dict } => {
                if data.len() < 2 || &data[..2] != b"CK" {
                    return Err(bad("bad MSZIP block signature".into()));
                }
                // Each block is a raw deflate stream that may refer to the previous 32 KiB:
                // prime the inflater with a stored block holding that history.
                let mut z = flate2::Decompress::new(false);
                if !dict.is_empty() {
                    let len = dict.len() as u16;
                    let mut primer = vec![0u8];
                    primer.extend_from_slice(&len.to_le_bytes());
                    primer.extend_from_slice(&(!len).to_le_bytes());
                    primer.extend_from_slice(dict);
                    let mut sink = Vec::with_capacity(dict.len());
                    z.decompress_vec(&primer, &mut sink, flate2::FlushDecompress::Sync)
                        .map_err(|e| bad(format!("MSZIP: {e}")))?;
                }
                let mut out = Vec::with_capacity(unpacked);
                z.decompress_vec(&data[2..], &mut out, flate2::FlushDecompress::Finish)
                    .map_err(|e| bad(format!("MSZIP: {e}")))?;
                if out.len() != unpacked {
                    return Err(bad(format!("MSZIP block is {} bytes, expected {unpacked}", out.len())));
                }
                let keep = out.len().min(32 * 1024);
                *dict = out[out.len() - keep..].to_vec();
                out
            }
            Decoder::Lzx(l) => l.decompress_next(&data, unpacked).map_err(|e| bad(format!("LZX: {e}")))?.to_vec(),
        };
        self.pos = 0;
        Ok(true)
    }
}

impl Read for FolderReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pos >= self.buf.len() {
            if !self.load()? {
                return Ok(0);
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

fn read_cstr(f: &mut impl Read, max: usize) -> Result<Vec<u8>, ArchiveError> {
    let mut out = Vec::new();
    let mut b = [0u8; 1];
    loop {
        f.read_exact(&mut b)?;
        if b[0] == 0 {
            return Ok(out);
        }
        if out.len() >= max {
            return Err(corrupt("CAB string too long"));
        }
        out.push(b[0]);
    }
}

/// DOS date / time (local time) → `SystemTime`.
fn dos_time(date: u16, time: u16) -> Option<std::time::SystemTime> {
    let (y, mo, d) = (1980 + i32::from(date >> 9), u32::from((date >> 5) & 0xF), u32::from(date & 0x1F));
    let (h, mi, s) = (u32::from(time >> 11), u32::from((time >> 5) & 0x3F), u32::from(time & 0x1F) * 2);
    local_to_system(y, mo, d, h, mi, s)
}

pub(crate) struct CabReader {
    data: CabData,
    listing: Listing,
}

impl CabReader {
    pub fn open(path: &Path) -> Result<Self, ArchiveError> {
        let file = File::open(path).map_err(|e| fs_err("open_archive", path, &e))?;
        let size = file.metadata().map_err(|e| fs_err("open_archive", path, &e))?.len();
        let mut f = BufReader::new(file);
        let mut h = [0u8; 36];
        f.read_exact(&mut h).map_err(|_| corrupt("not a cabinet"))?;
        if &h[..4] != b"MSCF" {
            return Err(corrupt("not a cabinet"));
        }
        let coff_files = u64::from(le32(&h, 16)?);
        let n_folders = le16(&h, 26)?;
        let n_files = le16(&h, 28)?;
        let flags = le16(&h, 30)?;
        let (mut folder_reserve, mut data_reserve) = (0u8, 0u8);
        if flags & FLAG_RESERVE != 0 {
            let mut r = [0u8; 4];
            f.read_exact(&mut r)?;
            let header_reserve = u16::from_le_bytes([r[0], r[1]]);
            folder_reserve = r[2];
            data_reserve = r[3];
            io::copy(&mut (&mut f).take(u64::from(header_reserve)), &mut io::sink())?;
        }
        if flags & FLAG_PREV != 0 {
            read_cstr(&mut f, 255)?;
            read_cstr(&mut f, 255)?;
        }
        if flags & FLAG_NEXT != 0 {
            read_cstr(&mut f, 255)?;
            read_cstr(&mut f, 255)?;
        }
        let mut folders = Vec::with_capacity(usize::from(n_folders));
        for _ in 0..n_folders {
            let mut b = [0u8; 8];
            f.read_exact(&mut b)?;
            let first_block = u64::from(le32(&b, 0)?);
            if first_block >= size {
                return Err(corrupt("CAB folder starts past the end of the file"));
            }
            let blocks = le16(&b, 4)?;
            let ty = le16(&b, 6)?;
            let method = match ty & 0x000F {
                0 => Method::Store,
                1 => Method::MsZip,
                3 => Method::Lzx(((ty >> 8) & 0x1F) as u8),
                _ => Method::Unsupported(ty),
            };
            io::copy(&mut (&mut f).take(u64::from(folder_reserve)), &mut io::sink())?;
            folders.push(Folder { first_block, blocks, method });
        }
        f.seek(SeekFrom::Start(coff_files))?;
        let mut entries = Vec::with_capacity(usize::from(n_files));
        let mut extras = Vec::with_capacity(usize::from(n_files));
        for _ in 0..n_files {
            let mut b = [0u8; 16];
            f.read_exact(&mut b).map_err(|_| corrupt("truncated CAB file table"))?;
            let size = u64::from(le32(&b, 0)?);
            let offset = u64::from(le32(&b, 4)?);
            let folder = le16(&b, 8)?;
            let (date, time, attrs) = (le16(&b, 10)?, le16(&b, 12)?, le16(&b, 14)?);
            let raw = read_cstr(&mut f, 1024)?;
            let name = if attrs & ATTR_UTF != 0 {
                String::from_utf8_lossy(&raw).into_owned()
            } else {
                raw.iter().map(|&c| c as char).collect()
            };
            let (norm, raw_name) = normalize_name(&name);
            if norm.is_empty() {
                continue;
            }
            // 0xFFFD..=0xFFFF: continued from / into another cabinet of the set.
            let stream =
                if folder >= 0xFFFD || usize::from(folder) >= folders.len() { NO_STREAM } else { usize::from(folder) };
            entries.push(ArchiveEntry {
                index: entries.len(),
                path: norm,
                is_dir: false,
                size,
                compressed: size,
                modified: dos_time(date, time),
                crc32: None,
                encrypted: false,
                mode: None,
                is_symlink: false,
                link_target: None,
            });
            extras.push(EntryExtra { raw: raw_name, ordinal: stream, data_pos: offset, ..Default::default() });
        }
        Ok(CabReader {
            data: CabData { path: path.to_path_buf(), folders, data_reserve },
            listing: Listing { entries, extras, index: None },
        })
    }
}

impl ArchiveReader for CabReader {
    fn format(&self) -> ArchiveFormat {
        ArchiveFormat::Cab
    }

    fn path(&self) -> &Path {
        &self.data.path
    }

    fn entries(&mut self) -> Result<&[ArchiveEntry], ArchiveError> {
        Ok(&self.listing.entries)
    }

    fn dir_index(&mut self) -> Result<Option<&DirIndex>, ArchiveError> {
        Ok(Some(self.listing.index()))
    }

    fn open_entry(&mut self, index: usize) -> Result<Box<dyn Read + Send + '_>, ArchiveError> {
        seq::open_entry(&self.data, &self.listing, index)
    }

    fn extract(
        &mut self,
        selection: Option<&[usize]>,
        dest: &Path,
        opts: &ExtractOptions,
        ctl: &OpControl,
        sink: &mut dyn ExtractSink,
    ) -> Result<ExtractSummary, ArchiveError> {
        let size = std::fs::metadata(&self.data.path).map_or(0, |m| m.len());
        seq::extract(&self.data, &self.listing, size, selection, dest, opts, ctl, sink)
    }

    fn test(&mut self, ctl: &OpControl, sink: &mut dyn ProgressSink) -> Result<TestReport, ArchiveError> {
        seq::test(&self.data, &self.listing, ctl, sink)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cfdata_checksum_matches_reference() {
        // Reference value from the MS-CAB algorithm (same vector as the `cab` crate).
        assert_eq!(checksum(b"\x0e\0\x0e\0Hello, world!\n", 0), 0x7f2e_1a4c);
        assert_eq!(checksum(b"", 0), 0);
    }
}
