//! PKWARE split / spanned zips (`name.z01`, `name.z02`, …, `name.zip`, as written by
//! `zip -s`, WinZip and the old floppy spanning): the disks are laid end to end and the
//! central directory is rebuilt with offsets into that joined stream (disk numbers set to
//! 0), then appended with a fresh end record. The `zip` crate reads the result as a
//! single-disk archive, so listing, extraction and testing need no changes.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::volumes::SplitReader;
use crate::ArchiveError;

/// Most disks looked for (`.z01` … `.z999`).
const MAX_DISKS: u32 = 999;

const EOCD_SIG: u32 = 0x0605_4b50;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;
const CDH_SIG: u32 = 0x0201_4b50;

/// The disks of a spanned set, in order, when `path` is its `.zip` (the last disk, which
/// has a `.z01` sibling) or its `.z01`. `None` for ordinary files.
pub(crate) fn spanned_parts(path: &Path) -> Option<Vec<PathBuf>> {
    let name = path.file_name()?.to_str()?;
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    let upper = ext.chars().next().is_some_and(|c| c.is_ascii_uppercase());
    let (z, zip) = if upper { ("Z", "ZIP") } else { ("z", "zip") };
    let is_zip = ext.eq_ignore_ascii_case("zip");
    if !is_zip && !ext.eq_ignore_ascii_case("z01") {
        return None;
    }
    let dir = path.parent().unwrap_or(Path::new(""));
    let disk = |n: u32| dir.join(format!("{stem}.{z}{n:02}"));
    let last = if is_zip { path.to_path_buf() } else { dir.join(format!("{stem}.{zip}")) };
    if !disk(1).is_file() || !last.is_file() {
        return None;
    }
    let mut parts = vec![disk(1)];
    for n in 2..=MAX_DISKS {
        let p = disk(n);
        if !p.is_file() {
            break;
        }
        parts.push(p);
    }
    parts.push(last);
    Some(parts)
}

/// The joined disks followed by the rebuilt central directory and end records.
pub(crate) struct SpannedReader {
    disks: SplitReader,
    tail: Vec<u8>,
    pos: u64,
}

impl SpannedReader {
    pub fn open(parts: &[PathBuf]) -> Result<SpannedReader, ArchiveError> {
        let mut disks = SplitReader::open(parts)?;
        let tail = rebuild_directory(&mut disks, parts.len())?;
        Ok(SpannedReader { disks, tail, pos: 0 })
    }

    fn len(&self) -> u64 {
        self.disks.total() + self.tail.len() as u64
    }
}

impl Read for SpannedReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let joined = self.disks.total();
        let n = if self.pos < joined {
            self.disks.seek(SeekFrom::Start(self.pos))?;
            let want = buf.len().min(usize::try_from(joined - self.pos).unwrap_or(usize::MAX));
            self.disks.read(&mut buf[..want])?
        } else {
            let at = usize::try_from(self.pos - joined).unwrap_or(usize::MAX).min(self.tail.len());
            let n = buf.len().min(self.tail.len() - at);
            buf[..n].copy_from_slice(&self.tail[at..at + n]);
            n
        };
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for SpannedReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.len().checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = target.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before start"))?;
        Ok(self.pos)
    }
}

fn corrupt(why: &str) -> ArchiveError {
    ArchiveError::Corrupt(format!("spanned zip: {why}"))
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

fn read_at(r: &mut SplitReader, at: u64, len: u64) -> Result<Vec<u8>, ArchiveError> {
    let len = usize::try_from(len).map_err(|_| corrupt("directory too large"))?;
    let mut buf = vec![0u8; len];
    r.seek(SeekFrom::Start(at))?;
    r.read_exact(&mut buf).map_err(|_| corrupt("a disk is shorter than the directory says"))?;
    Ok(buf)
}

/// Where the directory is, read from the last disk's end records.
struct Directory {
    /// Disk number of the last disk (disks counted from 0).
    last_disk: u32,
    /// Global offset and size of the central directory.
    offset: u64,
    size: u64,
    entries: u64,
    comment: Vec<u8>,
}

fn find_directory(disks: &mut SplitReader, count: usize) -> Result<Directory, ArchiveError> {
    let last = count - 1;
    let last_start = disks.start(last);
    let last_len = disks.total() - last_start;
    let span = last_len.min(22 + 0xFFFF + 20);
    let tail = read_at(disks, disks.total() - span, span)?;
    let at = (0..=tail.len().saturating_sub(22))
        .rev()
        .find(|&i| {
            u32_at(&tail, i) == Some(EOCD_SIG)
                && u16_at(&tail, i + 20).is_some_and(|c| i + 22 + usize::from(c) == tail.len())
        })
        .ok_or_else(|| corrupt("no end of central directory record on the last disk"))?;
    let field16 = |o| u16_at(&tail, at + o).unwrap_or(0);
    let field32 = |o| u32_at(&tail, at + o).unwrap_or(0);
    let comment = tail[at + 22..].to_vec();
    let mut dir = Directory {
        last_disk: u32::from(field16(4)),
        offset: u64::from(field32(16)),
        size: u64::from(field32(12)),
        entries: u64::from(field16(10)),
        comment,
    };
    let mut cd_disk = u32::from(field16(6));
    if at >= 20 && u32_at(&tail, at - 20) == Some(ZIP64_LOCATOR_SIG) {
        let rec_disk = u32_at(&tail, at - 16).unwrap_or(0) as usize;
        let rec_off = u64_at(&tail, at - 12).unwrap_or(0);
        if rec_disk >= count {
            return Err(corrupt("zip64 end record names a missing disk"));
        }
        let rec = read_at(disks, disks.start(rec_disk) + rec_off, 56)?;
        if u32_at(&rec, 0) != Some(ZIP64_EOCD_SIG) {
            return Err(corrupt("bad zip64 end record"));
        }
        dir.last_disk = u32_at(&rec, 16).unwrap_or(0);
        cd_disk = u32_at(&rec, 20).unwrap_or(0);
        dir.entries = u64_at(&rec, 32).unwrap_or(0);
        dir.size = u64_at(&rec, 40).unwrap_or(0);
        dir.offset = u64_at(&rec, 48).unwrap_or(0);
    }
    if dir.last_disk as usize + 1 != count {
        return Err(corrupt(&format!(
            "the set has {} disks but {count} were found (a .zNN file is missing)",
            dir.last_disk as u64 + 1
        )));
    }
    if cd_disk as usize >= count {
        return Err(corrupt("the directory starts on a missing disk"));
    }
    dir.offset += disks.start(cd_disk as usize);
    Ok(dir)
}

/// The new central directory (global offsets, disk 0) plus zip64 / plain end records.
fn rebuild_directory(disks: &mut SplitReader, count: usize) -> Result<Vec<u8>, ArchiveError> {
    let dir = find_directory(disks, count)?;
    if dir.offset + dir.size > disks.total() {
        return Err(corrupt("the directory runs past the last disk"));
    }
    let cd = read_at(disks, dir.offset, dir.size)?;
    let mut out = Vec::with_capacity(cd.len() + 128);
    let mut at = 0usize;
    let mut entries = 0u64;
    while at + 46 <= cd.len() && u32_at(&cd, at) == Some(CDH_SIG) {
        let name_len = usize::from(u16_at(&cd, at + 28).unwrap_or(0));
        let extra_len = usize::from(u16_at(&cd, at + 30).unwrap_or(0));
        let comment_len = usize::from(u16_at(&cd, at + 32).unwrap_or(0));
        let end = at + 46 + name_len + extra_len + comment_len;
        if end > cd.len() {
            return Err(corrupt("truncated directory entry"));
        }
        let rec = &cd[at..end];
        out.extend_from_slice(&rewrite_entry(rec, name_len, extra_len, disks)?);
        entries += 1;
        at = end;
    }
    if entries != dir.entries {
        return Err(corrupt("the directory holds fewer entries than the end record says"));
    }
    let cd_offset = disks.total();
    let cd_size = out.len() as u64;
    let zip64 = entries >= 0xFFFF || cd_size >= 0xFFFF_FFFF || cd_offset >= 0xFFFF_FFFF;
    if zip64 {
        let rec_offset = cd_offset + cd_size;
        out.extend_from_slice(&ZIP64_EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&44u64.to_le_bytes());
        out.extend_from_slice(&45u16.to_le_bytes()); // version made by
        out.extend_from_slice(&45u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u32.to_le_bytes()); // this disk
        out.extend_from_slice(&0u32.to_le_bytes()); // directory disk
        out.extend_from_slice(&entries.to_le_bytes());
        out.extend_from_slice(&entries.to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&ZIP64_LOCATOR_SIG.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&rec_offset.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
    }
    let n16 = u16::try_from(entries).unwrap_or(0xFFFF);
    out.extend_from_slice(&EOCD_SIG.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&n16.to_le_bytes());
    out.extend_from_slice(&n16.to_le_bytes());
    out.extend_from_slice(&u32::try_from(cd_size).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(&u32::try_from(cd_offset).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(&(dir.comment.len() as u16).to_le_bytes());
    out.extend_from_slice(&dir.comment);
    Ok(out)
}

/// One central directory entry with its disk number set to 0 and its local header
/// offset made global (in the zip64 extra field when it no longer fits 32 bits).
fn rewrite_entry(rec: &[u8], name_len: usize, extra_len: usize, disks: &SplitReader) -> Result<Vec<u8>, ArchiveError> {
    let comp32 = u32_at(rec, 20).unwrap_or(0);
    let uncomp32 = u32_at(rec, 24).unwrap_or(0);
    let disk16 = u16_at(rec, 34).unwrap_or(0);
    let off32 = u32_at(rec, 42).unwrap_or(0);
    let extra = &rec[46 + name_len..46 + name_len + extra_len];

    // Walk the extra fields: keep all but the zip64 one, whose values are read here.
    let (mut uncomp, mut comp, mut offset, mut disk) =
        (u64::from(uncomp32), u64::from(comp32), u64::from(off32), u32::from(disk16));
    let mut kept = Vec::with_capacity(extra.len());
    let mut i = 0usize;
    while i + 4 <= extra.len() {
        let id = u16_at(extra, i).unwrap_or(0);
        let len = usize::from(u16_at(extra, i + 2).unwrap_or(0));
        let body = extra.get(i + 4..i + 4 + len).ok_or_else(|| corrupt("truncated extra field"))?;
        if id == 0x0001 {
            let mut o = 0usize;
            let mut take = |sentinel: bool| -> Option<u64> {
                if !sentinel {
                    return None;
                }
                let v = u64_at(body, o);
                o += 8;
                v
            };
            uncomp = take(uncomp32 == u32::MAX).unwrap_or(uncomp);
            comp = take(comp32 == u32::MAX).unwrap_or(comp);
            offset = take(off32 == u32::MAX).unwrap_or(offset);
            if disk16 == u16::MAX {
                disk = u32_at(body, o).unwrap_or(disk);
            }
        } else {
            kept.extend_from_slice(&extra[i..i + 4 + len]);
        }
        i += 4 + len;
    }
    if disk as usize >= disks.parts() {
        return Err(corrupt("an entry starts on a missing disk"));
    }
    let global = disks.start(disk as usize) + offset;

    let mut z64 = Vec::new();
    let big = |v: u64| v >= u64::from(u32::MAX);
    if uncomp32 == u32::MAX || big(uncomp) {
        z64.extend_from_slice(&uncomp.to_le_bytes());
    }
    if comp32 == u32::MAX || big(comp) {
        z64.extend_from_slice(&comp.to_le_bytes());
    }
    if big(global) {
        z64.extend_from_slice(&global.to_le_bytes());
    }
    let mut new_extra = Vec::with_capacity(kept.len() + z64.len() + 4);
    if !z64.is_empty() {
        new_extra.extend_from_slice(&1u16.to_le_bytes());
        new_extra.extend_from_slice(&(z64.len() as u16).to_le_bytes());
        new_extra.extend_from_slice(&z64);
    }
    new_extra.extend_from_slice(&kept);
    let new_extra_len = u16::try_from(new_extra.len()).map_err(|_| corrupt("extra fields too long"))?;

    let mut out = Vec::with_capacity(rec.len() + 12);
    out.extend_from_slice(&rec[..46 + name_len]);
    out[30..32].copy_from_slice(&new_extra_len.to_le_bytes());
    out[34..36].copy_from_slice(&0u16.to_le_bytes());
    let off_field = if big(global) { u32::MAX } else { global as u32 };
    out[42..46].copy_from_slice(&off_field.to_le_bytes());
    out.extend_from_slice(&new_extra);
    out.extend_from_slice(&rec[46 + name_len + extra_len..]);
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Turn a single-disk zip into a PKWARE split set: the split signature first, the
    /// data cut every `cut` bytes, and the directory (with per-disk offsets) on the last
    /// disk. Returns the disks' bytes in order (`.z01` … `.zip`).
    pub(crate) fn split_zip(zip: &[u8], cut: usize) -> Vec<Vec<u8>> {
        let eocd = (0..zip.len() - 21).rev().find(|&i| u32_at(zip, i) == Some(EOCD_SIG)).unwrap();
        let cd_size = u32_at(zip, eocd + 12).unwrap() as usize;
        let cd_off = u32_at(zip, eocd + 16).unwrap() as usize;
        let entries = u16_at(zip, eocd + 10).unwrap();
        let mut data = 0x0807_4b50u32.to_le_bytes().to_vec();
        data.extend_from_slice(&zip[..cd_off]);
        let mut disks: Vec<Vec<u8>> = data.chunks(cut).map(<[u8]>::to_vec).collect();
        let last = disks.len() - 1;
        let mut cd = zip[cd_off..cd_off + cd_size].to_vec();
        let mut at = 0;
        while at < cd.len() {
            let n = u16_at(&cd, at + 28).unwrap() as usize;
            let e = u16_at(&cd, at + 30).unwrap() as usize;
            let c = u16_at(&cd, at + 32).unwrap() as usize;
            let global = u32_at(&cd, at + 42).unwrap() as usize + 4;
            cd[at + 34..at + 36].copy_from_slice(&((global / cut) as u16).to_le_bytes());
            cd[at + 42..at + 46].copy_from_slice(&((global % cut) as u32).to_le_bytes());
            at += 46 + n + e + c;
        }
        let cd_at = disks[last].len() as u32;
        let d = &mut disks[last];
        d.extend_from_slice(&cd);
        d.extend_from_slice(&EOCD_SIG.to_le_bytes());
        d.extend_from_slice(&(last as u16).to_le_bytes());
        d.extend_from_slice(&(last as u16).to_le_bytes());
        d.extend_from_slice(&entries.to_le_bytes());
        d.extend_from_slice(&entries.to_le_bytes());
        d.extend_from_slice(&(cd_size as u32).to_le_bytes());
        d.extend_from_slice(&cd_at.to_le_bytes());
        d.extend_from_slice(&0u16.to_le_bytes());
        disks
    }

    /// Write `disks` as `<stem>.z01` … `<stem>.zip` in `dir`; returns the `.zip` path.
    pub(crate) fn write_set(dir: &Path, stem: &str, disks: &[Vec<u8>]) -> PathBuf {
        for (i, d) in disks.iter().enumerate() {
            let name = if i + 1 == disks.len() { format!("{stem}.zip") } else { format!("{stem}.z{:02}", i + 1) };
            std::fs::write(dir.join(name), d).unwrap();
        }
        dir.join(format!("{stem}.zip"))
    }

    fn sample_zip(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        for (name, body) in files {
            let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            w.start_file(*name, opts).unwrap();
            io::Write::write_all(&mut w, body).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn spanned_set_reads_as_one_zip() {
        let d = tempfile::tempdir().unwrap();
        let a: Vec<u8> = (0..5000u32).map(|i| (i % 253) as u8).collect();
        let b = b"hello from the second file".to_vec();
        let disks = split_zip(&sample_zip(&[("a.bin", a.clone()), ("dir/b.txt", b.clone())]), 1500);
        assert!(disks.len() >= 4);
        let zip_path = write_set(d.path(), "set", &disks);

        let parts = spanned_parts(&zip_path).unwrap();
        assert_eq!(parts.len(), disks.len());
        assert_eq!(spanned_parts(&d.path().join("set.z01")).unwrap(), parts);
        assert!(spanned_parts(&d.path().join("set.z02")).is_none());

        let mut z = zip::ZipArchive::new(SpannedReader::open(&parts).unwrap()).unwrap();
        assert_eq!(z.len(), 2);
        let mut got = Vec::new();
        z.by_name("a.bin").unwrap().read_to_end(&mut got).unwrap();
        assert_eq!(got, a);
        got.clear();
        z.by_name("dir/b.txt").unwrap().read_to_end(&mut got).unwrap();
        assert_eq!(got, b);
    }

    #[test]
    fn a_missing_disk_is_reported() {
        let d = tempfile::tempdir().unwrap();
        let data: Vec<u8> = (0..4000u32).map(|i| i as u8).collect();
        let disks = split_zip(&sample_zip(&[("x", data)]), 1000);
        let zip_path = write_set(d.path(), "s", &disks);
        std::fs::remove_file(d.path().join("s.z03")).unwrap();
        let parts = spanned_parts(&zip_path).unwrap();
        let err = SpannedReader::open(&parts).err().unwrap();
        assert!(err.to_string().contains("missing"), "{err}");
    }

    #[test]
    fn plain_zip_is_not_a_set() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("plain.zip");
        std::fs::write(&p, sample_zip(&[("x", b"1".to_vec())])).unwrap();
        assert!(spanned_parts(&p).is_none());
    }
}
