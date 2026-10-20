//! Microsoft Cabinet reader (read-only): stored, MSZIP and LZX folders. Each folder is
//! one compressed stream, so extraction decodes every folder once, front to back (see
//! `seq.rs`). Quantum folders are listed but report `Unsupported` when read.
//!
//! Cabinet sets (`disk1.cab`, `disk2.cab`, … as written by `makecab` with a disk size):
//! opening any cabinet of a set finds the first one through the previous-cabinet names,
//! then follows the next-cabinet names, and reads the whole set as one archive. A folder
//! that continues into the next cabinet is decoded across the files (its last data block
//! may be split between two cabinets), and a file stored in several cabinets is listed
//! once. Data in a cabinet that isn't there is an error naming it when read.

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
/// `iFolder` of a file that starts in the previous cabinet (its data is in this
/// cabinet's first folder).
const FROM_PREV: u16 = 0xFFFD;
/// `iFolder` of a file that continues into the next cabinet (this cabinet's last folder).
const TO_NEXT: u16 = 0xFFFE;
/// `iFolder` of a file that starts in the previous cabinet and continues into the next.
const PREV_AND_NEXT: u16 = 0xFFFF;
/// Most cabinets read for one set.
const MAX_CABINETS: usize = 512;

#[derive(Clone, Copy, Debug)]
enum Method {
    Store,
    MsZip,
    /// Window size in bits (15..=21).
    Lzx(u8),
    Unsupported(u16),
}

/// The part of a folder stored in one cabinet.
#[derive(Clone, Debug)]
struct Segment {
    /// Index into `CabData::cabinets`.
    cabinet: usize,
    first_block: u64,
    blocks: u16,
    reserve: u8,
}

#[derive(Clone, Debug)]
struct Folder {
    method: Method,
    segments: Vec<Segment>,
    /// The folder continues in this cabinet, which wasn't found.
    missing_next: Option<String>,
}

struct CabData {
    path: PathBuf,
    cabinets: Vec<PathBuf>,
    folders: Vec<Folder>,
}

impl StreamSource for CabData {
    fn open_stream(&self, stream: usize) -> Result<Box<dyn Read + Send>, ArchiveError> {
        let folder = self
            .folders
            .get(stream)
            .ok_or_else(|| corrupt("file data starts in a cabinet of the set that wasn't found"))?;
        if let Method::Unsupported(m) = folder.method {
            log::debug!("CAB compression method {m:#x} is not supported");
            return Err(ArchiveError::Unsupported(ArchiveFormat::Cab));
        }
        Ok(Box::new(FolderReader::new(&self.cabinets, folder)?))
    }
}

enum Decoder {
    Store,
    MsZip { dict: Vec<u8> },
    Lzx(Box<lzxd::Lzxd>),
}

/// Decoded byte stream of one CAB folder, across the cabinets it is stored in.
struct FolderReader {
    cabinets: Vec<PathBuf>,
    segments: Vec<Segment>,
    missing_next: Option<String>,
    /// Segment being read (`segments.len()` once all are done).
    seg: usize,
    f: Option<BufReader<File>>,
    next_block: u64,
    blocks_left: u16,
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
    fn new(cabinets: &[PathBuf], folder: &Folder) -> Result<Self, ArchiveError> {
        let dec = match folder.method {
            Method::Store => Decoder::Store,
            Method::MsZip => Decoder::MsZip { dict: Vec::new() },
            Method::Lzx(bits) => {
                let w = window(bits).ok_or_else(|| corrupt(format!("bad LZX window size 2^{bits}")))?;
                Decoder::Lzx(Box::new(lzxd::Lzxd::new(w)))
            }
            Method::Unsupported(_) => return Err(ArchiveError::Unsupported(ArchiveFormat::Cab)),
        };
        let mut r = FolderReader {
            cabinets: cabinets.to_vec(),
            segments: folder.segments.clone(),
            missing_next: folder.missing_next.clone(),
            seg: 0,
            f: None,
            next_block: 0,
            blocks_left: 0,
            dec,
            buf: Vec::new(),
            pos: 0,
        };
        r.enter(0)?;
        Ok(r)
    }

    /// Start reading segment `i` (open its cabinet).
    fn enter(&mut self, i: usize) -> io::Result<()> {
        self.seg = i;
        let Some(s) = self.segments.get(i) else {
            self.f = None;
            return Ok(());
        };
        let path = &self.cabinets[s.cabinet];
        let f = File::open(path)?;
        self.f = Some(BufReader::with_capacity(64 * 1024, f));
        self.next_block = s.first_block;
        self.blocks_left = s.blocks;
        Ok(())
    }

    /// The next CFDATA block of the folder, moving on to the next cabinet when this one's
    /// part is done: `(data, uncompressed size)`, `None` at the end of the folder.
    fn next_piece(&mut self) -> io::Result<Option<(Vec<u8>, usize)>> {
        while self.blocks_left == 0 {
            if self.seg >= self.segments.len() {
                return Ok(None);
            }
            if self.seg + 1 == self.segments.len() {
                if let Some(name) = &self.missing_next {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("the data continues in cabinet {name}, which wasn't found"),
                    ));
                }
                self.seg += 1;
                return Ok(None);
            }
            self.enter(self.seg + 1)?;
        }
        self.blocks_left -= 1;
        let reserve = usize::from(self.segments[self.seg].reserve);
        let Some(f) = self.f.as_mut() else { return Ok(None) };
        f.seek(SeekFrom::Start(self.next_block))?;
        let mut head = [0u8; 8];
        f.read_exact(&mut head)?;
        let stored_sum = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let packed = usize::from(u16::from_le_bytes([head[4], head[5]]));
        let unpacked = usize::from(u16::from_le_bytes([head[6], head[7]]));
        let mut both = vec![0u8; reserve + packed];
        f.read_exact(&mut both)?;
        self.next_block += 8 + reserve as u64 + packed as u64;
        if stored_sum != 0 {
            let sum = checksum(&both, 0) ^ (packed as u32 | ((unpacked as u32) << 16));
            if sum != stored_sum {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "CAB data block checksum mismatch"));
            }
        }
        both.drain(..reserve);
        Ok(Some((both, unpacked)))
    }

    /// Decode the next CFDATA block into `buf`; `false` at the end of the folder.
    fn load(&mut self) -> io::Result<bool> {
        let bad = |m: String| io::Error::new(io::ErrorKind::InvalidData, m);
        let Some((mut data, mut unpacked)) = self.next_piece()? else { return Ok(false) };
        // A block split between two cabinets: the first piece says 0 bytes uncompressed,
        // the rest of the compressed data (with the real size) opens the next cabinet.
        while unpacked == 0 {
            let Some((more, u)) = self.next_piece()? else {
                return Err(bad("a CAB data block is cut off at the end of the set".into()));
            };
            data.extend_from_slice(&more);
            unpacked = u;
        }
        let packed = data.len();
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

struct RawFolder {
    first_block: u64,
    blocks: u16,
    method: Method,
}

struct RawFile {
    size: u64,
    offset: u64,
    folder: u16,
    date: u16,
    time: u16,
    name: String,
}

/// The tables of one cabinet file.
struct Cabinet {
    path: PathBuf,
    size: u64,
    prev: Option<String>,
    next: Option<String>,
    data_reserve: u8,
    folders: Vec<RawFolder>,
    files: Vec<RawFile>,
}

impl Cabinet {
    fn read(path: &Path) -> Result<Cabinet, ArchiveError> {
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
        let name = |b: Vec<u8>| Some(String::from_utf8_lossy(&b).into_owned()).filter(|s| !s.is_empty());
        let (mut prev, mut next) = (None, None);
        if flags & FLAG_PREV != 0 {
            prev = name(read_cstr(&mut f, 255)?);
            read_cstr(&mut f, 255)?;
        }
        if flags & FLAG_NEXT != 0 {
            next = name(read_cstr(&mut f, 255)?);
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
            folders.push(RawFolder { first_block, blocks, method });
        }
        f.seek(SeekFrom::Start(coff_files))?;
        let mut files = Vec::with_capacity(usize::from(n_files));
        for _ in 0..n_files {
            let mut b = [0u8; 16];
            f.read_exact(&mut b).map_err(|_| corrupt("truncated CAB file table"))?;
            let raw = read_cstr(&mut f, 1024)?;
            let attrs = le16(&b, 14)?;
            let name = if attrs & ATTR_UTF != 0 {
                String::from_utf8_lossy(&raw).into_owned()
            } else {
                raw.iter().map(|&c| c as char).collect()
            };
            files.push(RawFile {
                size: u64::from(le32(&b, 0)?),
                offset: u64::from(le32(&b, 4)?),
                folder: le16(&b, 8)?,
                date: le16(&b, 10)?,
                time: le16(&b, 12)?,
                name,
            });
        }
        Ok(Cabinet { path: path.to_path_buf(), size, prev, next, data_reserve, folders, files })
    }

    /// The last folder continues into the next cabinet.
    fn continues(&self) -> bool {
        self.next.is_some() && self.files.iter().any(|f| f.folder == TO_NEXT || f.folder == PREV_AND_NEXT)
    }

    /// The first folder continues from the previous cabinet.
    fn continued(&self) -> bool {
        self.prev.is_some() && self.files.iter().any(|f| f.folder == FROM_PREV || f.folder == PREV_AND_NEXT)
    }
}

/// `name` in `dir`: as written, else matched without regard to case (sets copied from
/// a FAT disk to a case-sensitive file system).
fn sibling(dir: &Path, name: &str) -> Option<PathBuf> {
    if name.contains(['/', '\\']) || name == ".." {
        return None;
    }
    let p = dir.join(name);
    if p.is_file() {
        return Some(p);
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|e| e.file_name().to_str().is_some_and(|n| n.eq_ignore_ascii_case(name)))
        .map(|e| e.path())
        .filter(|p| p.is_file())
}

/// The cabinets of the set `path` belongs to, in order (just `path` when it stands alone).
fn cabinet_set(path: &Path) -> Result<Vec<Cabinet>, ArchiveError> {
    let opened = Cabinet::read(path)?;
    if opened.prev.is_none() && opened.next.is_none() {
        return Ok(vec![opened]);
    }
    let dir = path.parent().unwrap_or(Path::new(""));
    let same =
        |a: &Path, b: &Path| a == b || a.file_name().zip(b.file_name()).is_some_and(|(x, y)| x.eq_ignore_ascii_case(y));
    let forward = |first: Cabinet| {
        let mut set = vec![first];
        while set.len() < MAX_CABINETS {
            let last = &set[set.len() - 1];
            let Some(p) = last.next.as_deref().and_then(|n| sibling(dir, n)) else { break };
            if set.iter().any(|c| same(&c.path, &p)) {
                break;
            }
            match Cabinet::read(&p) {
                Ok(c) => set.push(c),
                Err(e) => {
                    log::debug!("cabinet set: {} unreadable: {e}", p.display());
                    break;
                }
            }
        }
        set
    };
    // Walk back to the first cabinet (a previous-cabinet name may skip cabinets: makecab
    // names the one the first file started in), then forward through the set.
    let mut first: Option<Cabinet> = None;
    let mut seen = vec![path.to_path_buf()];
    let mut prev = opened.prev.clone();
    while let Some(p) = prev.as_deref().and_then(|n| sibling(dir, n)) {
        if seen.iter().any(|s| same(s, &p)) || seen.len() >= MAX_CABINETS {
            break;
        }
        seen.push(p.clone());
        match Cabinet::read(&p) {
            Ok(c) => {
                prev = c.prev.clone();
                first = Some(c);
            }
            Err(_) => break,
        }
    }
    if let Some(first) = first {
        let set = forward(first);
        if set.iter().any(|c| same(&c.path, path)) {
            return Ok(set);
        }
    }
    Ok(forward(opened))
}

pub(crate) struct CabReader {
    data: CabData,
    listing: Listing,
    /// Bytes of all cabinets in the set (extraction progress).
    size: u64,
}

impl CabReader {
    pub fn open(path: &Path) -> Result<Self, ArchiveError> {
        let cabs = cabinet_set(path)?;
        let mut folders: Vec<Folder> = Vec::new();
        let mut entries = Vec::new();
        let mut extras = Vec::new();
        // Merged index of the previous cabinet's last folder when it continues here.
        let mut open_tail: Option<usize> = None;
        for (ci, cab) in cabs.iter().enumerate() {
            let tail = open_tail.take();
            let orphan = tail.is_none() && cab.continued();
            let mut map = Vec::with_capacity(cab.folders.len());
            for (fi, rf) in cab.folders.iter().enumerate() {
                let seg =
                    Segment { cabinet: ci, first_block: rf.first_block, blocks: rf.blocks, reserve: cab.data_reserve };
                match tail.filter(|_| fi == 0) {
                    Some(m) => {
                        folders[m].segments.push(seg);
                        map.push(m);
                    }
                    None => {
                        map.push(folders.len());
                        folders.push(Folder { method: rf.method, segments: vec![seg], missing_next: None });
                    }
                }
            }
            let merged_prev = ci > 0 && !orphan && cab.continued();
            for f in &cab.files {
                let stream = match f.folder {
                    // Listed already, from the cabinet the file starts in.
                    FROM_PREV | PREV_AND_NEXT if merged_prev => continue,
                    FROM_PREV | PREV_AND_NEXT => NO_STREAM,
                    TO_NEXT => map.last().copied().unwrap_or(NO_STREAM),
                    // The first folder continues a folder whose start isn't here.
                    0 if orphan => NO_STREAM,
                    i => map.get(usize::from(i)).copied().unwrap_or(NO_STREAM),
                };
                let (norm, raw_name) = normalize_name(&f.name);
                if norm.is_empty() {
                    continue;
                }
                entries.push(ArchiveEntry {
                    index: entries.len(),
                    path: norm,
                    is_dir: false,
                    size: f.size,
                    compressed: f.size,
                    modified: dos_time(f.date, f.time),
                    crc32: None,
                    encrypted: false,
                    mode: None,
                    is_symlink: false,
                    link_target: None,
                });
                extras.push(EntryExtra { raw: raw_name, ordinal: stream, data_pos: f.offset, ..Default::default() });
            }
            open_tail = if cab.continues() { map.last().copied() } else { None };
        }
        if let (Some(m), Some(last)) = (open_tail, cabs.last()) {
            folders[m].missing_next = last.next.clone();
        }
        let size = cabs.iter().map(|c| c.size).sum();
        Ok(CabReader {
            data: CabData { path: path.to_path_buf(), cabinets: cabs.into_iter().map(|c| c.path).collect(), folders },
            listing: Listing { entries, extras, index: None },
            size,
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
        seq::extract(&self.data, &self.listing, self.size, selection, dest, opts, ctl, sink)
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

    /// A file entry for [`cabinet`]: name, size, offset in its folder, `iFolder`.
    type FileRow<'a> = (&'a str, u32, u32, u16);

    /// One stored (uncompressed) cabinet: `folders` lists each folder's CFDATA blocks as
    /// `(data, uncompressed size)` (0 = the block continues in the next cabinet).
    fn cabinet(
        prev: Option<&str>,
        next: Option<&str>,
        folders: &[Vec<(Vec<u8>, u16)>],
        files: &[FileRow<'_>],
    ) -> Vec<u8> {
        let mut names = Vec::new();
        let mut flags = 0u16;
        for (n, flag) in [(prev, FLAG_PREV), (next, FLAG_NEXT)] {
            if let Some(n) = n {
                flags |= flag;
                names.extend_from_slice(n.as_bytes());
                names.extend_from_slice(b"\0disk\0");
            }
        }
        let folders_at = 36 + names.len();
        let files_at = folders_at + 8 * folders.len();
        let mut table = Vec::new();
        for (name, size, off, folder) in files {
            table.extend_from_slice(&size.to_le_bytes());
            table.extend_from_slice(&off.to_le_bytes());
            table.extend_from_slice(&folder.to_le_bytes());
            table.extend_from_slice(&[0x21, 0x5A, 0, 0x60, 0x20, 0]); // date, time, attrs
            table.extend_from_slice(name.as_bytes());
            table.push(0);
        }
        let mut data = Vec::new();
        let mut cffolders = Vec::new();
        let mut at = files_at + table.len();
        for blocks in folders {
            cffolders.extend_from_slice(&(at as u32).to_le_bytes());
            cffolders.extend_from_slice(&(blocks.len() as u16).to_le_bytes());
            cffolders.extend_from_slice(&0u16.to_le_bytes());
            for (b, unpacked) in blocks {
                data.extend_from_slice(&0u32.to_le_bytes()); // no checksum
                data.extend_from_slice(&(b.len() as u16).to_le_bytes());
                data.extend_from_slice(&unpacked.to_le_bytes());
                data.extend_from_slice(b);
                at += 8 + b.len();
            }
        }
        let total = files_at + table.len() + data.len();
        let mut out = b"MSCF".to_vec();
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(files_at as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&[3, 1]);
        out.extend_from_slice(&(folders.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&[0x34, 0x12, 0, 0]); // set id, cabinet number
        out.extend_from_slice(&names);
        out.extend_from_slice(&cffolders);
        out.extend_from_slice(&table);
        out.extend_from_slice(&data);
        out
    }

    fn bytes(n: usize, seed: u8) -> Vec<u8> {
        (0..n).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
    }

    /// Three cabinets. `a` (1000 bytes) starts in 1.cab and ends in 2.cab with a data block
    /// split between them; `b` (300) sits in 2.cab's own folder; `c` (900) runs from 2.cab
    /// into 3.cab, and `d` (100) follows it there.
    fn write_set(dir: &Path) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
        let (a, b, c, d) = (bytes(1000, 1), bytes(300, 2), bytes(900, 3), bytes(100, 4));
        let one = cabinet(
            None,
            Some("2.cab"),
            &[vec![(a[..600].to_vec(), 600), (a[600..700].to_vec(), 0)]],
            &[("a.bin", 1000, 0, TO_NEXT)],
        );
        let mut f1 = b.clone();
        f1.extend_from_slice(&c[..500]);
        let two = cabinet(
            Some("1.cab"),
            Some("3.cab"),
            &[vec![(a[700..800].to_vec(), 200), (a[800..].to_vec(), 200)], vec![(f1, 800)]],
            &[("a.bin", 1000, 0, FROM_PREV), ("dir/b.txt", 300, 0, 1), ("c.bin", 900, 300, TO_NEXT)],
        );
        let mut tail = c[500..].to_vec();
        tail.extend_from_slice(&d);
        // A previous-cabinet name may skip cabinets (makecab writes the one the first file
        // started in); here it skips 2.cab.
        let three = cabinet(
            Some("1.cab"),
            None,
            &[vec![(tail, 500)]],
            &[("c.bin", 900, 300, FROM_PREV), ("d.bin", 100, 1200, 0)],
        );
        std::fs::write(dir.join("1.cab"), one).unwrap();
        std::fs::write(dir.join("2.cab"), two).unwrap();
        std::fs::write(dir.join("3.CAB"), three).unwrap();
        (a, b, c, d)
    }

    fn read_all(r: &mut CabReader) -> Vec<(String, Result<Vec<u8>, String>)> {
        let n = r.entries().unwrap().len();
        (0..n)
            .map(|i| {
                let name = r.entries().unwrap()[i].path.clone();
                let mut out = Vec::new();
                let res = r.open_entry(i).and_then(|mut s| Ok(s.read_to_end(&mut out).map(|_| out)?));
                (name, res.map_err(|e| e.to_string()))
            })
            .collect()
    }

    #[test]
    fn a_cabinet_set_reads_as_one_archive_from_any_cabinet() {
        let d = tempfile::tempdir().unwrap();
        let (a, b, c, dd) = write_set(d.path());
        let want = vec![
            ("a.bin".to_string(), Ok(a)),
            ("dir/b.txt".to_string(), Ok(b)),
            ("c.bin".to_string(), Ok(c)),
            ("d.bin".to_string(), Ok(dd)),
        ];
        for name in ["1.cab", "2.cab", "3.CAB"] {
            let mut r = CabReader::open(&d.path().join(name)).unwrap();
            assert_eq!(read_all(&mut r), want, "opened {name}");
            assert_eq!(r.path(), d.path().join(name));
        }
    }

    #[test]
    fn a_missing_cabinet_names_itself() {
        let d = tempfile::tempdir().unwrap();
        let (a, b, ..) = write_set(d.path());
        std::fs::remove_file(d.path().join("3.CAB")).unwrap();
        let mut r = CabReader::open(&d.path().join("1.cab")).unwrap();
        let got = read_all(&mut r);
        assert_eq!(got[0], ("a.bin".to_string(), Ok(a)));
        assert_eq!(got[1], ("dir/b.txt".to_string(), Ok(b)));
        let err = got[2].1.as_ref().unwrap_err();
        assert!(err.contains("3.cab"), "{err}");

        // Opened from the middle with the first cabinet gone: what starts there can't be read.
        std::fs::remove_file(d.path().join("1.cab")).unwrap();
        let mut r = CabReader::open(&d.path().join("2.cab")).unwrap();
        let got = read_all(&mut r);
        assert_eq!(got[0].0, "a.bin");
        assert!(got[0].1.is_err());
        assert!(got[1].1.is_ok());
    }
}
