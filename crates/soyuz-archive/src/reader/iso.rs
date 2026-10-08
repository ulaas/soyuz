//! Disc images (read-only): ISO 9660 with Joliet (UCS-2 names) and Rock Ridge (POSIX
//! names, modes, symlinks, deep-directory relocation), and UDF (see `udf.rs`).
//!
//! Which tree is shown: UDF when the image has one (Windows install media keep their real
//! content only there), else Joliet, else the primary ISO 9660 tree (with Rock Ridge when
//! present). Every structure is bounds-checked; directory loops, absurd depths and entry
//! counts are refused, and file extents must lie inside the image.

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use soyuz_core::{OpControl, ProgressSink};

use super::Listing;
use super::random::{self, EntrySource};
use crate::util::{EntryExtra, fs_err, normalize_name};
use crate::{
    ArchiveEntry, ArchiveError, ArchiveFormat, ArchiveReader, DirIndex, ExtractOptions, ExtractSink, ExtractSummary,
    TestReport,
};

pub(crate) const SECTOR: u64 = 2048;
/// Most entries one image may list.
pub(crate) const MAX_NODES: usize = 2_000_000;
/// Deepest directory nesting followed.
pub(crate) const MAX_DEPTH: usize = 256;
/// Largest directory (bytes of records) read.
pub(crate) const MAX_DIR_BYTES: u64 = 64 << 20;

/// Where a piece of file data lives.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Ext {
    /// `len` bytes at byte `offset` of the image.
    Data { offset: u64, len: u64 },
    /// `len` zero bytes (UDF unrecorded / sparse extents).
    Zero(u64),
    /// Data embedded in the metadata (UDF in-ICB files).
    Inline(Vec<u8>),
}

impl Ext {
    pub fn len(&self) -> u64 {
        match self {
            Ext::Data { len, .. } | Ext::Zero(len) => *len,
            Ext::Inline(v) => v.len() as u64,
        }
    }
}

/// One file system object found by a parser.
#[derive(Clone, Debug, Default)]
pub(crate) struct Node {
    /// Raw `/`-joined path (normalized later).
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub mode: Option<u32>,
    pub link: Option<String>,
    pub exts: Vec<Ext>,
}

/// The image file with bounds-checked reads.
pub(crate) struct Img {
    f: BufReader<File>,
    pub len: u64,
}

impl Img {
    pub fn open(path: &Path) -> Result<Img, ArchiveError> {
        let f = File::open(path).map_err(|e| fs_err("open_archive", path, &e))?;
        let len = f.metadata().map_err(|e| fs_err("open_archive", path, &e))?.len();
        Ok(Img { f: BufReader::with_capacity(16 * 1024, f), len })
    }

    /// `n` bytes at `offset` (must lie inside the image).
    pub fn read_at(&mut self, offset: u64, n: u64) -> Result<Vec<u8>, ArchiveError> {
        let end = offset.checked_add(n).ok_or_else(|| corrupt("offset overflow"))?;
        if end > self.len {
            return Err(corrupt(format!("structure at {offset} (+{n}) lies past the end of the image")));
        }
        self.f.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; usize::try_from(n).map_err(|_| corrupt("structure too large"))?];
        self.f.read_exact(&mut buf)?;
        Ok(buf)
    }

    pub fn sector(&mut self, lba: u64) -> Result<Vec<u8>, ArchiveError> {
        self.read_at(lba.checked_mul(SECTOR).ok_or_else(|| corrupt("sector overflow"))?, SECTOR)
    }
}

pub(crate) fn corrupt(msg: impl Into<String>) -> ArchiveError {
    ArchiveError::Corrupt(msg.into())
}

pub(crate) fn le16(b: &[u8], o: usize) -> Result<u16, ArchiveError> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| corrupt("truncated structure"))
}

pub(crate) fn le32(b: &[u8], o: usize) -> Result<u32, ArchiveError> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| corrupt("truncated structure"))
}

pub(crate) fn le64(b: &[u8], o: usize) -> Result<u64, ArchiveError> {
    b.get(o..o + 8)
        .map(|s| u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
        .ok_or_else(|| corrupt("truncated structure"))
}

/// Volume descriptors found at the start of an image.
#[derive(Default)]
struct Descriptors {
    primary: Option<Vec<u8>>,
    joliet: Option<Vec<u8>>,
    udf: bool,
}

fn scan_descriptors(img: &mut Img) -> Result<Descriptors, ArchiveError> {
    let mut d = Descriptors::default();
    for lba in 16..16 + 64 {
        if (lba + 1) * SECTOR > img.len {
            break;
        }
        let s = img.sector(lba)?;
        match &s[1..6] {
            b"CD001" => match s[0] {
                1 if d.primary.is_none() => d.primary = Some(s),
                2 if d.joliet.is_none() && matches!(&s[88..91], b"%/@" | b"%/C" | b"%/E") => d.joliet = Some(s),
                _ => {}
            },
            b"NSR02" | b"NSR03" => d.udf = true,
            b"BEA01" | b"BOOT2" | b"CDW02" => {}
            // "TEA01" or anything else: the volume recognition area is over.
            _ => break,
        }
    }
    Ok(d)
}

/// Parse the image into nodes (UDF first, then Joliet, then ISO 9660).
fn parse(img: &mut Img) -> Result<Vec<Node>, ArchiveError> {
    let d = scan_descriptors(img)?;
    let mut udf_err = None;
    if d.udf {
        match super::udf::parse(img) {
            Ok(nodes) => return Ok(nodes),
            Err(e) => {
                log::debug!("UDF tree unreadable, falling back to ISO 9660: {e}");
                udf_err = Some(e);
            }
        }
    }
    // Rock Ridge (POSIX names, modes, links) beats Joliet; Joliet beats 8.3-ish names.
    if let Some(vd) = &d.primary
        && has_rock_ridge(img, vd)?
    {
        return parse_9660(img, vd, false);
    }
    if let Some(vd) = &d.joliet {
        return parse_9660(img, vd, true);
    }
    if let Some(vd) = &d.primary {
        return parse_9660(img, vd, false);
    }
    Err(udf_err.unwrap_or_else(|| corrupt("no ISO 9660 or UDF volume descriptor")))
}

/// `(year since 1900, month, day, hour, minute, second, GMT offset in 15 min)`.
fn record_time(b: &[u8]) -> Option<SystemTime> {
    if b.len() < 7 || b[1] == 0 || b[2] == 0 {
        return None;
    }
    let date = chrono::NaiveDate::from_ymd_opt(1900 + i32::from(b[0]), u32::from(b[1]), u32::from(b[2]))?;
    let dt = date.and_hms_opt(u32::from(b[3]), u32::from(b[4]), u32::from(b[5]).min(59))?;
    let offset = i64::from(b[6] as i8) * 15 * 60;
    let utc = dt.and_utc().timestamp() - offset;
    crate::util::unix_secs_to_time(utc)
}

/// Rock Ridge data of one directory record.
#[derive(Default)]
struct Rr {
    name: Option<String>,
    mode: Option<u32>,
    link: Option<String>,
    /// `RE`: a relocated directory (shown at its `CL` place instead).
    relocated: bool,
    /// `CL`: the real location of a relocated directory.
    child_link: Option<u32>,
}

const S_IFMT: u32 = 0o170000;
const S_IFLNK: u32 = 0o120000;

/// Parse SUSP entries in `su` (following `CE` continuations, at most 32).
fn parse_susp(img: &mut Img, su: &[u8], rr: &mut Rr) -> Result<(), ArchiveError> {
    let mut area = su.to_vec();
    let mut hops = 0;
    let mut nm = String::new();
    let mut nm_seen = false;
    let mut sl = String::new();
    let mut sl_seen = false;
    let mut sl_continue = false;
    loop {
        let mut next = None;
        let mut i = 0usize;
        while i + 4 <= area.len() {
            let sig = [area[i], area[i + 1]];
            let len = usize::from(area[i + 2]);
            if len < 4 || i + len > area.len() {
                break;
            }
            let e = &area[i..i + len];
            match &sig {
                b"NM" if len >= 5 => {
                    let flags = e[4];
                    if flags & 0x06 == 0 {
                        nm.push_str(&String::from_utf8_lossy(&e[5..]));
                        nm_seen = true;
                    }
                }
                b"PX" if len >= 12 => rr.mode = Some(le32(e, 4)?),
                b"SL" if len >= 5 => {
                    sl_seen = true;
                    let mut j = 5;
                    while j + 2 <= e.len() {
                        let cflags = e[j];
                        let clen = usize::from(e[j + 1]);
                        let Some(content) = e.get(j + 2..j + 2 + clen) else { break };
                        if !sl_continue && !sl.is_empty() && !sl.ends_with('/') {
                            sl.push('/');
                        }
                        match cflags & 0x0E {
                            0x02 => sl.push('.'),
                            0x04 => sl.push_str(".."),
                            0x08 => {
                                sl.clear();
                                sl.push('/');
                            }
                            _ => sl.push_str(&String::from_utf8_lossy(content)),
                        }
                        sl_continue = cflags & 0x01 != 0;
                        j += 2 + clen;
                    }
                }
                b"CE" if len >= 28 => {
                    let (block, off, clen) = (le32(e, 4)?, le32(e, 12)?, le32(e, 20)?);
                    next = Some((u64::from(block) * SECTOR + u64::from(off), u64::from(clen).min(SECTOR)));
                }
                b"RE" => rr.relocated = true,
                b"CL" if len >= 12 => rr.child_link = Some(le32(e, 4)?),
                b"ST" => break,
                _ => {}
            }
            i += len;
        }
        hops += 1;
        match next {
            Some((off, n)) if hops < 32 => area = img.read_at(off, n)?,
            _ => break,
        }
    }
    if nm_seen && !nm.is_empty() {
        rr.name = Some(nm);
    }
    if sl_seen {
        rr.link = Some(sl);
    }
    Ok(())
}

/// Plain ISO 9660 name: drop the `;1` version and a trailing dot.
fn iso_name(raw: &[u8], joliet: bool) -> String {
    let mut s = if joliet {
        let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        String::from_utf16_lossy(&units)
    } else {
        raw.iter().map(|&b| b as char).collect()
    };
    if let Some(i) = s.rfind(';')
        && s[i + 1..].chars().all(|c| c.is_ascii_digit())
    {
        s.truncate(i);
    }
    if !joliet && s.ends_with('.') {
        s.pop();
    }
    s
}

struct DirJob {
    lba: u32,
    len: u32,
    path: String,
    depth: usize,
}

/// `(logical block size, root extent, root length)` of a volume descriptor.
fn root_of(vd: &[u8]) -> Result<(u64, u32, u32), ArchiveError> {
    let block = u64::from(le16(vd, 128)?);
    let block = if matches!(block, 512 | 1024 | 2048) { block } else { SECTOR };
    let root = vd.get(156..156 + 34).ok_or_else(|| corrupt("bad volume descriptor"))?;
    Ok((block, le32(root, 2)?, le32(root, 10)?))
}

/// Rock Ridge: an "SP" entry in the root directory's "." record.
fn has_rock_ridge(img: &mut Img, vd: &[u8]) -> Result<bool, ArchiveError> {
    let (block, root_lba, root_len) = root_of(vd)?;
    let first = img.read_at(u64::from(root_lba) * block, u64::from(root_len.clamp(34, 2048)))?;
    let len = usize::from(first[0]);
    let nlen = usize::from(*first.get(32).unwrap_or(&0));
    let su_start = 33 + nlen + usize::from(nlen % 2 == 0);
    Ok(len <= first.len() && su_start + 7 <= len && &first[su_start..su_start + 2] == b"SP")
}

fn parse_9660(img: &mut Img, vd: &[u8], joliet: bool) -> Result<Vec<Node>, ArchiveError> {
    let (block, root_lba, root_len) = root_of(vd)?;
    let rock_ridge = !joliet && has_rock_ridge(img, vd)?;
    let mut nodes = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = vec![DirJob { lba: root_lba, len: root_len, path: String::new(), depth: 0 }];
    while let Some(dir) = queue.pop() {
        if !visited.insert(dir.lba) || dir.depth > MAX_DEPTH {
            continue;
        }
        let len = u64::from(dir.len).min(MAX_DIR_BYTES);
        let data = img.read_at(u64::from(dir.lba) * block, len)?;
        let mut pos = 0usize;
        let mut pending: Option<Node> = None;
        while pos < data.len() {
            let rec_len = usize::from(data[pos]);
            if rec_len == 0 {
                // Records never cross a sector: continue at the next one.
                pos = (pos / SECTOR as usize + 1) * SECTOR as usize;
                continue;
            }
            if rec_len < 34 || pos + rec_len > data.len() {
                break;
            }
            let rec = &data[pos..pos + rec_len];
            pos += rec_len;
            let nlen = usize::from(rec[32]);
            let Some(raw_name) = rec.get(33..33 + nlen) else { continue };
            if nlen == 1 && raw_name[0] <= 1 {
                continue; // "." and ".."
            }
            let flags = rec[25];
            let mut is_dir = flags & 0x02 != 0;
            let mut lba = le32(rec, 2)?;
            let mut size = le32(rec, 10)?;
            let mut rr = Rr::default();
            if rock_ridge {
                let su_start = 33 + nlen + usize::from(nlen % 2 == 0);
                if su_start < rec.len() {
                    parse_susp(img, &rec[su_start..], &mut rr)?;
                }
                if rr.relocated {
                    continue;
                }
                if let Some(cl) = rr.child_link {
                    // A relocated directory: its "." record holds the size.
                    let first = img.read_at(u64::from(cl) * block, 34)?;
                    is_dir = true;
                    lba = cl;
                    size = le32(&first, 10)?;
                }
            }
            let name = rr.name.take().unwrap_or_else(|| iso_name(raw_name, joliet));
            if name.is_empty() || name == "." || name == ".." {
                continue;
            }
            let path = if dir.path.is_empty() { name } else { format!("{}/{name}", dir.path) };
            let modified = record_time(&rec[18..25]);
            if is_dir {
                queue.push(DirJob { lba, len: size, path: path.clone(), depth: dir.depth + 1 });
                nodes.push(Node { path, is_dir: true, modified, mode: rr.mode, ..Default::default() });
            } else {
                let ext = Ext::Data { offset: u64::from(lba) * block, len: u64::from(size) };
                let is_link = rr.mode.is_some_and(|m| m & S_IFMT == S_IFLNK) && rr.link.is_some();
                match pending.as_mut() {
                    Some(p) if p.path == path => {
                        p.size += u64::from(size);
                        p.exts.push(ext);
                    }
                    _ => {
                        if let Some(p) = pending.take() {
                            nodes.push(p);
                        }
                        pending = Some(Node {
                            path,
                            size: if is_link { 0 } else { u64::from(size) },
                            modified,
                            mode: rr.mode,
                            link: if is_link { rr.link.take() } else { None },
                            exts: if is_link { Vec::new() } else { vec![ext] },
                            ..Default::default()
                        });
                    }
                }
                // Multi-extent: more records with the same name follow.
                if flags & 0x80 == 0
                    && let Some(p) = pending.take()
                {
                    nodes.push(p);
                }
            }
            if nodes.len() > MAX_NODES {
                return Err(ArchiveError::LimitExceeded(format!("more than {MAX_NODES} entries")));
            }
        }
        if let Some(p) = pending.take() {
            nodes.push(p);
        }
    }
    Ok(nodes)
}

/// Reads a chain of extents (bounded by the entry size).
pub(crate) struct ExtReader {
    f: BufReader<File>,
    exts: Vec<Ext>,
    idx: usize,
    within: u64,
    left: u64,
    positioned: bool,
}

impl ExtReader {
    pub fn new(path: &Path, image_len: u64, exts: Vec<Ext>, size: u64) -> Result<ExtReader, ArchiveError> {
        for e in &exts {
            if let Ext::Data { offset, len } = e
                && offset.checked_add(*len).is_none_or(|end| end > image_len)
            {
                return Err(corrupt("file data lies past the end of the image"));
            }
        }
        let f = File::open(path).map_err(|e| fs_err("open_archive", path, &e))?;
        Ok(ExtReader {
            f: BufReader::with_capacity(64 * 1024, f),
            exts,
            idx: 0,
            within: 0,
            left: size,
            positioned: false,
        })
    }
}

impl Read for ExtReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.left == 0 || buf.is_empty() {
                return Ok(0);
            }
            let Some(ext) = self.exts.get(self.idx) else { return Ok(0) };
            let avail = ext.len() - self.within;
            if avail == 0 {
                self.idx += 1;
                self.within = 0;
                self.positioned = false;
                continue;
            }
            let want = (buf.len() as u64).min(avail).min(self.left) as usize;
            let n = match ext {
                Ext::Data { offset, .. } => {
                    if !self.positioned {
                        self.f.seek(SeekFrom::Start(offset + self.within))?;
                        self.positioned = true;
                    }
                    let n = self.f.read(&mut buf[..want])?;
                    if n == 0 {
                        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "image ended inside a file"));
                    }
                    n
                }
                Ext::Zero(_) => {
                    buf[..want].fill(0);
                    want
                }
                Ext::Inline(v) => {
                    let s = self.within as usize;
                    buf[..want].copy_from_slice(&v[s..s + want]);
                    want
                }
            };
            self.within += n as u64;
            self.left -= n as u64;
            return Ok(n);
        }
    }
}

struct DiscData {
    path: PathBuf,
    len: u64,
    exts: Vec<Vec<Ext>>,
}

impl EntrySource for DiscData {
    fn open_index(&mut self, idx: usize, entry: &ArchiveEntry) -> Result<Box<dyn Read + Send + '_>, ArchiveError> {
        let exts = self.exts.get(idx).cloned().unwrap_or_default();
        Ok(Box::new(ExtReader::new(&self.path, self.len, exts, entry.size)?))
    }
}

pub(crate) struct IsoReader {
    data: DiscData,
    listing: Listing,
}

impl IsoReader {
    pub fn open(path: &Path) -> Result<Self, ArchiveError> {
        let mut img = Img::open(path)?;
        let nodes = parse(&mut img)?;
        let mut entries = Vec::with_capacity(nodes.len());
        let mut extras = Vec::with_capacity(nodes.len());
        let mut exts = Vec::with_capacity(nodes.len());
        for n in nodes {
            let (norm, raw) = normalize_name(&n.path);
            if norm.is_empty() {
                continue;
            }
            let compressed: u64 = n.exts.iter().map(Ext::len).sum();
            entries.push(ArchiveEntry {
                index: entries.len(),
                path: norm,
                is_dir: n.is_dir,
                size: if n.is_dir { 0 } else { n.size },
                compressed,
                modified: n.modified,
                crc32: None,
                encrypted: false,
                mode: n.mode,
                is_symlink: n.link.is_some(),
                link_target: n.link,
            });
            extras.push(EntryExtra { raw, ordinal: exts.len(), ..Default::default() });
            exts.push(n.exts);
        }
        Ok(IsoReader {
            data: DiscData { path: path.to_path_buf(), len: img.len, exts },
            listing: Listing { entries, extras, index: None },
        })
    }
}

impl ArchiveReader for IsoReader {
    fn format(&self) -> ArchiveFormat {
        ArchiveFormat::Iso
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
        random::open_entry(&mut self.data, &self.listing, index)
    }

    fn extract(
        &mut self,
        selection: Option<&[usize]>,
        dest: &Path,
        opts: &ExtractOptions,
        ctl: &OpControl,
        sink: &mut dyn ExtractSink,
    ) -> Result<ExtractSummary, ArchiveError> {
        let size = self.data.len;
        random::extract(&mut self.data, &self.listing, size, selection, dest, opts, ctl, sink)
    }

    fn test(&mut self, ctl: &OpControl, sink: &mut dyn ProgressSink) -> Result<TestReport, ArchiveError> {
        random::test(&mut self.data, &self.listing, ctl, sink)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(iso_name(b"README.TXT;1", false), "README.TXT");
        assert_eq!(iso_name(b"NOEXT.;1", false), "NOEXT");
        let j: Vec<u8> = "Grüße.txt;1".encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(iso_name(&j, true), "Grüße.txt");
    }

    #[test]
    fn record_times() {
        // 2024-05-06 07:08:09 at GMT+1 (4 × 15 min).
        let t = record_time(&[124, 5, 6, 7, 8, 9, 4]).unwrap();
        assert_eq!(crate::util::time_to_unix_secs(t), 1_714_979_289 - 3600);
        assert_eq!(record_time(&[0, 0, 0, 0, 0, 0, 0]), None);
    }
}
