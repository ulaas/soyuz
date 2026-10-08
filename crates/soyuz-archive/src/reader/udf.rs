//! UDF (ECMA-167 / OSTA UDF 1.02 – 2.60) reader for disc images: anchor → volume
//! descriptor sequence → partitions (physical, sparable, metadata) → file set → file
//! entries / extended file entries and file identifier descriptors. Short / long /
//! extended allocation descriptors, continuation extents, in-ICB (embedded) data and
//! unrecorded (sparse) extents are handled. Virtual (VAT, CD-R) partitions are not.

use std::collections::HashSet;
use std::time::SystemTime;

use super::iso::{Ext, Img, MAX_DEPTH, MAX_DIR_BYTES, MAX_NODES, Node, SECTOR, corrupt, le16, le32, le64};
use crate::ArchiveError;

/// Most allocation descriptors followed for one file (incl. continuation extents).
const MAX_EXTENTS: usize = 1 << 20;

const TAG_AVDP: u16 = 2;
const TAG_PD: u16 = 5;
const TAG_LVD: u16 = 6;
const TAG_TD: u16 = 8;
const TAG_FSD: u16 = 256;
const TAG_FID: u16 = 257;
const TAG_AED: u16 = 258;
const TAG_FE: u16 = 261;
const TAG_EFE: u16 = 266;

/// Descriptor tag: id matches and the header checksum is right.
fn tag_ok(b: &[u8], id: u16) -> bool {
    if b.len() < 16 || le16(b, 0).ok() != Some(id) {
        return false;
    }
    let sum = b[..16].iter().enumerate().filter(|(i, _)| *i != 4).fold(0u8, |a, (_, &x)| a.wrapping_add(x));
    sum == b[4]
}

/// Partition reference → where its logical blocks live.
enum Map {
    /// Blocks start at this absolute sector.
    Physical { start: u64 },
    /// UDF 2.5+ metadata partition: blocks are mapped through the metadata file's extents
    /// `(first logical block, block count, absolute sector)`.
    Metadata { extents: Vec<(u64, u64, u64)> },
}

struct Volume {
    maps: Vec<Map>,
}

impl Volume {
    /// Absolute byte ranges for `len` bytes starting at logical block `lbn` of `part`.
    fn locate(&self, part: u16, lbn: u32, len: u64) -> Result<Vec<Ext>, ArchiveError> {
        let map = self.maps.get(usize::from(part)).ok_or_else(|| corrupt(format!("no UDF partition {part}")))?;
        match map {
            Map::Physical { start } => Ok(vec![Ext::Data { offset: (start + u64::from(lbn)) * SECTOR, len }]),
            Map::Metadata { extents } => {
                let mut out = Vec::new();
                let mut block = u64::from(lbn);
                let mut left = len;
                while left > 0 {
                    let &(first, count, sector) = extents
                        .iter()
                        .find(|(first, count, _)| block >= *first && block < first + count)
                        .ok_or_else(|| corrupt("block outside the UDF metadata partition"))?;
                    let span = ((first + count - block) * SECTOR).min(left);
                    out.push(Ext::Data { offset: (sector + block - first) * SECTOR, len: span });
                    left -= span;
                    block += span.div_ceil(SECTOR);
                    if out.len() > MAX_EXTENTS {
                        return Err(corrupt("too many UDF extents"));
                    }
                }
                Ok(out)
            }
        }
    }

    fn read(&self, img: &mut Img, part: u16, lbn: u32, len: u64) -> Result<Vec<u8>, ArchiveError> {
        let mut out = Vec::new();
        for e in self.locate(part, lbn, len)? {
            if let Ext::Data { offset, len } = e {
                out.extend_from_slice(&img.read_at(offset, len)?);
            }
        }
        Ok(out)
    }
}

/// OSTA CS0 d-string / file identifier.
fn dstring(b: &[u8]) -> String {
    match b.first() {
        Some(8 | 254) => b[1..].iter().map(|&c| c as char).collect(),
        Some(16 | 255) => {
            let units: Vec<u16> = b[1..].as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::new(),
    }
}

/// ECMA-167 timestamp (12 bytes).
fn timestamp(b: &[u8]) -> Option<SystemTime> {
    let tz = le16(b, 0).ok()?;
    let year = le16(b, 2).ok()? as i16;
    let (mo, d, h, mi, s) = (*b.get(4)?, *b.get(5)?, *b.get(6)?, *b.get(7)?, *b.get(8)?);
    if year <= 0 || mo == 0 || d == 0 {
        return None;
    }
    let date = chrono::NaiveDate::from_ymd_opt(i32::from(year), u32::from(mo), u32::from(d))?;
    let dt = date.and_hms_opt(u32::from(h), u32::from(mi), u32::from(s).min(59))?;
    // Low 12 bits: signed offset from UTC in minutes (-2047 = unspecified), type in bits 12-15.
    let raw = i32::from(tz & 0x0FFF);
    let offset = if raw & 0x800 != 0 { raw - 0x1000 } else { raw };
    let offset = if tz >> 12 == 1 && offset != -2047 { i64::from(offset) * 60 } else { 0 };
    crate::util::unix_secs_to_time(dt.and_utc().timestamp() - offset)
}

/// A file entry, decoded.
struct Entry {
    file_type: u8,
    size: u64,
    modified: Option<SystemTime>,
    mode: u32,
    exts: Vec<Ext>,
}

fn find_anchor(img: &mut Img) -> Result<Vec<u8>, ArchiveError> {
    let last = (img.len / SECTOR).saturating_sub(1);
    for lba in [256, last.saturating_sub(256), last] {
        if (lba + 1) * SECTOR <= img.len {
            let s = img.sector(lba)?;
            if tag_ok(&s, TAG_AVDP) {
                return Ok(s);
            }
        }
    }
    Err(corrupt("no UDF anchor volume descriptor"))
}

fn read_volume(img: &mut Img) -> Result<(Volume, [u8; 16]), ArchiveError> {
    let avdp = find_anchor(img)?;
    let (vds_len, vds_loc) = (le32(&avdp, 16)?, le32(&avdp, 20)?);
    let mut parts: Vec<(u16, u64)> = Vec::new();
    let mut lvd: Option<Vec<u8>> = None;
    for i in 0..u64::from(vds_len / SECTOR as u32).min(256) {
        let s = img.sector(u64::from(vds_loc) + i)?;
        match le16(&s, 0)? {
            TAG_PD if tag_ok(&s, TAG_PD) => parts.push((le16(&s, 22)?, u64::from(le32(&s, 188)?))),
            TAG_LVD if tag_ok(&s, TAG_LVD) && lvd.is_none() => lvd = Some(s),
            TAG_TD => break,
            _ => {}
        }
    }
    let lvd = lvd.ok_or_else(|| corrupt("no UDF logical volume descriptor"))?;
    if le32(&lvd, 212)? != SECTOR as u32 {
        return Err(corrupt("unsupported UDF logical block size"));
    }
    let mut fsd_ad = [0u8; 16];
    fsd_ad.copy_from_slice(lvd.get(248..264).ok_or_else(|| corrupt("truncated UDF volume descriptor"))?);
    let n_maps = le32(&lvd, 268)?;
    let start_of = |num: u16| parts.iter().find(|(n, _)| *n == num).map(|(_, s)| *s);
    let mut maps = Vec::new();
    let mut pending_meta: Vec<(usize, u64, u32)> = Vec::new();
    let mut pos = 440usize;
    for _ in 0..n_maps.min(64) {
        let ty = *lvd.get(pos).ok_or_else(|| corrupt("truncated UDF partition map"))?;
        let len = usize::from(*lvd.get(pos + 1).ok_or_else(|| corrupt("truncated UDF partition map"))?);
        if len < 6 {
            return Err(corrupt("bad UDF partition map"));
        }
        let m = lvd.get(pos..pos + len).ok_or_else(|| corrupt("truncated UDF partition map"))?;
        match ty {
            1 => {
                let start = start_of(le16(m, 4)?).ok_or_else(|| corrupt("UDF partition map without partition"))?;
                maps.push(Map::Physical { start });
            }
            2 => {
                let ident = m.get(5..28).unwrap_or_default();
                let num = le16(m, 38)?;
                let start = start_of(num).ok_or_else(|| corrupt("UDF partition map without partition"))?;
                if ident.starts_with(b"*UDF Metadata Partition") {
                    pending_meta.push((maps.len(), start, le32(m, 40)?));
                    maps.push(Map::Metadata { extents: Vec::new() });
                } else if ident.starts_with(b"*UDF Sparable Partition") {
                    // Images have no defects: the sparing table is not needed.
                    maps.push(Map::Physical { start });
                } else {
                    return Err(corrupt(format!(
                        "unsupported UDF partition type {}",
                        String::from_utf8_lossy(ident).trim_end_matches('\0')
                    )));
                }
            }
            _ => return Err(corrupt("unknown UDF partition map type")),
        }
        pos += len;
    }
    // Metadata partitions: the metadata file (a file entry in the physical partition)
    // lists where the metadata blocks are.
    for (slot, start, file_loc) in pending_meta {
        let mut vol = Volume { maps: vec![Map::Physical { start }] };
        let fe = read_entry(img, &mut vol, 0, file_loc)?;
        let mut extents = Vec::new();
        let mut first = 0u64;
        for e in fe.exts {
            if let Ext::Data { offset, len } = e {
                let count = len.div_ceil(SECTOR);
                extents.push((first, count, offset / SECTOR));
                first += count;
            }
        }
        maps[slot] = Map::Metadata { extents };
    }
    Ok((Volume { maps }, fsd_ad))
}

/// Allocation descriptors of a file entry → data extents.
fn allocation(
    img: &mut Img,
    vol: &Volume,
    part: u16,
    ad_type: u16,
    mut ads: Vec<u8>,
    size: u64,
) -> Result<Vec<Ext>, ArchiveError> {
    let step = match ad_type {
        0 => 8,
        1 => 16,
        2 => 20,
        _ => return Err(corrupt("bad UDF allocation descriptor type")),
    };
    let mut out = Vec::new();
    let mut total = 0u64;
    let mut hops = 0;
    'outer: loop {
        let mut i = 0;
        while i + step <= ads.len() {
            let raw = le32(&ads, i)?;
            let (kind, len) = (raw >> 30, u64::from(raw & 0x3FFF_FFFF));
            if len == 0 {
                break 'outer;
            }
            let (lbn, p) = match ad_type {
                0 => (le32(&ads, i + 4)?, part),
                1 => (le32(&ads, i + 4)?, le16(&ads, i + 8)?),
                _ => (le32(&ads, i + 12)?, le16(&ads, i + 16)?),
            };
            if kind == 3 {
                // Continuation: the rest of the descriptors live in an allocation extent.
                hops += 1;
                if hops > 4096 {
                    return Err(corrupt("too many UDF allocation extents"));
                }
                let aed = vol.read(img, p, lbn, SECTOR)?;
                if !tag_ok(&aed, TAG_AED) {
                    return Err(corrupt("bad UDF allocation extent"));
                }
                let l = le32(&aed, 20)? as usize;
                ads = aed.get(24..24 + l).ok_or_else(|| corrupt("bad UDF allocation extent"))?.to_vec();
                continue 'outer;
            }
            let len = len.min(size.saturating_sub(total));
            if len > 0 {
                if kind == 0 {
                    out.extend(vol.locate(p, lbn, len)?);
                } else {
                    out.push(Ext::Zero(len));
                }
                total += len;
            }
            if out.len() > MAX_EXTENTS {
                return Err(corrupt("too many UDF extents"));
            }
            i += step;
        }
        break;
    }
    Ok(out)
}

fn read_entry(img: &mut Img, vol: &mut Volume, part: u16, lbn: u32) -> Result<Entry, ArchiveError> {
    let b = vol.read(img, part, lbn, SECTOR)?;
    let (base, mtime_at) = if tag_ok(&b, TAG_FE) {
        (176usize, 84usize)
    } else if tag_ok(&b, TAG_EFE) {
        (216, 92)
    } else {
        return Err(corrupt("bad UDF file entry"));
    };
    let file_type = b[27];
    let ad_type = le16(&b, 34)? & 7;
    let size = le64(&b, 56)?;
    let perms = le32(&b, 44)?;
    let mode = ((perms >> 10) & 7) << 6 | ((perms >> 5) & 7) << 3 | (perms & 7);
    let (l_ea, l_ad) = (le32(&b, base - 8)? as usize, le32(&b, base - 4)? as usize);
    let start = base.checked_add(l_ea).ok_or_else(|| corrupt("bad UDF file entry"))?;
    let ads = b.get(start..start + l_ad).ok_or_else(|| corrupt("bad UDF file entry"))?.to_vec();
    let exts = if ad_type == 3 {
        let n = (size as usize).min(ads.len());
        vec![Ext::Inline(ads[..n].to_vec())]
    } else {
        allocation(img, vol, part, ad_type, ads, size)?
    };
    Ok(Entry { file_type, size, modified: timestamp(&b[mtime_at..mtime_at + 12]), mode, exts })
}

/// Read a whole (small) file: directory contents, symlink targets.
fn read_data(img: &mut Img, exts: &[Ext], cap: u64) -> Result<Vec<u8>, ArchiveError> {
    let mut out = Vec::new();
    for e in exts {
        let left = cap.saturating_sub(out.len() as u64);
        if left == 0 {
            break;
        }
        match e {
            Ext::Data { offset, len } => out.extend_from_slice(&img.read_at(*offset, (*len).min(left))?),
            Ext::Zero(len) => out.resize(out.len() + (*len).min(left) as usize, 0),
            Ext::Inline(v) => out.extend_from_slice(&v[..v.len().min(left as usize)]),
        }
    }
    Ok(out)
}

/// ECMA-167 4/14.16 path components of a symlink.
fn symlink_target(d: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i + 4 <= d.len() {
        let (ty, l) = (d[i], usize::from(d[i + 1]));
        let Some(id) = d.get(i + 4..i + 4 + l) else { break };
        match ty {
            1 | 2 => {
                out.clear();
                out.push('/');
            }
            3..=5 => {
                if !out.is_empty() && !out.ends_with('/') {
                    out.push('/');
                }
                out.push_str(match ty {
                    3 => "..",
                    4 => ".",
                    _ => "",
                });
                if ty == 5 {
                    out.push_str(&dstring(id));
                }
            }
            _ => {}
        }
        i += 4 + l;
    }
    out
}

pub(crate) fn parse(img: &mut Img) -> Result<Vec<Node>, ArchiveError> {
    let (mut vol, fsd_ad) = read_volume(img)?;
    let fsd = vol.read(img, le16(&fsd_ad, 8)?, le32(&fsd_ad, 4)?, SECTOR)?;
    if !tag_ok(&fsd, TAG_FSD) {
        return Err(corrupt("bad UDF file set descriptor"));
    }
    let root = (le16(&fsd, 408)?, le32(&fsd, 404)?);
    let mut nodes = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = vec![(root, String::new(), 0usize)];
    while let Some(((part, lbn), path, depth)) = queue.pop() {
        if !visited.insert((part, lbn)) || depth > MAX_DEPTH {
            continue;
        }
        let dir = read_entry(img, &mut vol, part, lbn)?;
        let data = read_data(img, &dir.exts, dir.size.min(MAX_DIR_BYTES))?;
        let mut pos = 0usize;
        while pos + 38 <= data.len() {
            let fid = &data[pos..];
            if !tag_ok(fid, TAG_FID) {
                break;
            }
            let chars = fid[18];
            let lfi = usize::from(fid[19]);
            let (c_lbn, c_part) = (le32(fid, 24)?, le16(fid, 28)?);
            let liu = usize::from(le16(fid, 36)?);
            let total = (38 + liu + lfi + 3) & !3;
            let name = fid.get(38 + liu..38 + liu + lfi).map(dstring).unwrap_or_default();
            pos += total;
            // Parent link / deleted.
            if chars & 0x08 != 0 || chars & 0x04 != 0 || name.is_empty() || name == "." || name == ".." {
                continue;
            }
            let child_path = if path.is_empty() { name } else { format!("{path}/{name}") };
            let e = match read_entry(img, &mut vol, c_part, c_lbn) {
                Ok(e) => e,
                Err(err) => {
                    log::debug!("UDF entry {child_path} unreadable: {err}");
                    continue;
                }
            };
            match e.file_type {
                4 => {
                    queue.push(((c_part, c_lbn), child_path.clone(), depth + 1));
                    nodes.push(Node {
                        path: child_path,
                        is_dir: true,
                        modified: e.modified,
                        mode: Some(e.mode),
                        ..Default::default()
                    });
                }
                12 => {
                    let target = symlink_target(&read_data(img, &e.exts, 64 * 1024)?);
                    nodes.push(Node {
                        path: child_path,
                        modified: e.modified,
                        mode: Some(e.mode),
                        link: Some(target),
                        ..Default::default()
                    });
                }
                _ => nodes.push(Node {
                    path: child_path,
                    size: e.size,
                    modified: e.modified,
                    mode: Some(e.mode),
                    exts: e.exts,
                    ..Default::default()
                }),
            }
            if nodes.len() > MAX_NODES {
                return Err(ArchiveError::LimitExceeded(format!("more than {MAX_NODES} entries")));
            }
        }
    }
    Ok(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_and_links() {
        assert_eq!(dstring(b"\x08abc"), "abc");
        assert_eq!(dstring(&[16, 0, b'x', 0x00, 0xFC]), "xü");
        // "/", "..", "lib"
        let mut d = vec![1, 0, 0, 0, 3, 0, 0, 0, 5, 4, 0, 0, 8];
        d.extend_from_slice(b"lib");
        assert_eq!(symlink_target(&d), "/../lib");
    }

    #[test]
    fn timestamps() {
        // 2024-05-06 07:08:09 local time at UTC+2 (type 1, +120 min).
        let mut b = vec![0u8; 12];
        b[..2].copy_from_slice(&((1u16 << 12) | 120).to_le_bytes());
        b[2..4].copy_from_slice(&2024u16.to_le_bytes());
        b[4..9].copy_from_slice(&[5, 6, 7, 8, 9]);
        let t = timestamp(&b).unwrap();
        assert_eq!(crate::util::time_to_unix_secs(t), 1_714_979_289 - 7200);
    }
}
