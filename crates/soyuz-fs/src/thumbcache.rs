//! Read-only access to Explorer's thumbnail cache (`%LOCALAPPDATA%\Microsoft\Windows\
//! Explorer\thumbcache_<size>.db`, roadmap B3), as a fast thumbnail source that needs no
//! shell API (`IThumbnailCache` is never used).
//!
//! **Cache key.** Every entry is keyed by the 64-bit `System.ThumbnailCacheId`. Explorer
//! derives it in shell32 (`CalculateHashKey`); the algorithm was reverse engineered by
//! Y. Khatri ("Windows 7 Thumbcache hash algorithm", swiftforensics.com, 2012) and is
//! re-implemented in Thumbcache Viewer (`map_entries.cpp`, E. Kutcher, with the Windows
//! 8.1+ precision-loss step). Starting from the seed `0x95E729BA2C37FD21`, each input
//! byte `b` is mixed as `h ^= h * 0x820 + b + (h >> 2)` (wrapping u64), over: the volume
//! GUID (16 bytes, memory layout), the NTFS file index (u64 LE, with the sequence
//! number), the extension as UTF-16LE with its dot and case kept (empty for folders),
//! the UTC last-write time as `(fat_date << 16) | fat_time` (u32 LE) and, when nonzero,
//! the low 32 bits of (that DOS time converted back to a FILETIME − the original) (u32
//! LE). Verified on Windows 11 build 26300 against `System.ThumbnailCacheId` and
//! the entries in `thumbcache_*.db` for files on two NTFS volumes (see
//! `docs/INTEGRATION_NOTES.md`).
//!
//! **File format** (libyal "Windows Explorer Thumbnail Cache database format", format
//! versions 30–32 = Windows 8 / 8.1 / 10–11): a 24-byte header (`CMMM`, version, cache
//! type, …, first entry offset at 16, first free offset at 20), then entries of a 56-byte
//! header (`CMMM`, entry size, hash, identifier size, padding size, data size, width,
//! height, unknown, data checksum, header checksum) followed by the UTF-16 identifier,
//! padding and the picture (BMP, JPEG or PNG). Older versions (Vista / 7) are ignored.
//!
//! Explorer keeps writing these files while we read them, so every hit is re-validated
//! (signature, hash, sizes) before its data is returned, and an index is rebuilt when the
//! file's length or modification time changes. Decoding the picture is the caller's job.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Seed of the cache-id hash (a constant in shell32.dll).
pub const SEED: u64 = 0x95E7_29BA_2C37_FD21;

/// Fold `bytes` into the cache-id hash.
pub fn mix(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= h.wrapping_mul(0x820).wrapping_add(u64::from(b)).wrapping_add(h >> 2);
    }
    h
}

/// `System.ThumbnailCacheId` from its inputs (Windows 8.1 and later).
pub fn cache_id_from(volume_guid: &[u8; 16], file_index: u64, ext: &str, dos_time: u32, precision_loss: u32) -> u64 {
    let mut h = mix(SEED, volume_guid);
    h = mix(h, &file_index.to_le_bytes());
    let ext16: Vec<u8> = ext.encode_utf16().flat_map(u16::to_le_bytes).collect();
    h = mix(h, &ext16);
    h = mix(h, &dos_time.to_le_bytes());
    if precision_loss != 0 {
        h = mix(h, &precision_loss.to_le_bytes());
    }
    h
}

/// The extension as Explorer hashes it: with the dot, case kept, `""` for none.
pub fn hashed_extension(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some(e) if !e.is_empty() => format!(".{e}"),
        _ => String::new(),
    }
}

/// `System.ThumbnailCacheId` of `path` (Windows, local volumes with a GUID; `None`
/// elsewhere, e.g. network shares or times outside the FAT range).
pub fn cache_id(path: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        let k = crate::sys::windows::thumbcache::key_inputs(path).ok()?;
        let ext = if k.is_dir { String::new() } else { hashed_extension(path) };
        Some(cache_id_from(&k.volume_guid, k.file_index, &ext, k.dos_time, k.precision_loss))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

/// One cached picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OsThumb {
    /// Encoded picture (BMP, JPEG or PNG).
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// The size class of the database it came from (`thumbcache_<size>.db`).
    pub db_size: u32,
}

/// Square size classes Explorer stores, smallest first.
pub const DB_SIZES: [u32; 9] = [16, 32, 48, 96, 256, 768, 1280, 1920, 2560];

const ENTRY_HEADER: usize = 56;
const MAX_ENTRY_DATA: u32 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    /// Offset of the entry header.
    start: u64,
    /// Offset and length of the picture.
    offset: u64,
    size: u32,
    width: u32,
    height: u32,
}

/// Parsed entry header (format versions 30–32).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct EntryHeader {
    size: u32,
    hash: u64,
    id_size: u32,
    pad_size: u32,
    data_size: u32,
    width: u32,
    height: u32,
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

fn parse_entry_header(b: &[u8]) -> Option<EntryHeader> {
    if b.get(..4)? != b"CMMM" {
        return None;
    }
    let h = EntryHeader {
        size: u32_at(b, 4)?,
        hash: u64_at(b, 8)?,
        id_size: u32_at(b, 16)?,
        pad_size: u32_at(b, 20)?,
        data_size: u32_at(b, 24)?,
        width: u32_at(b, 28)?,
        height: u32_at(b, 32)?,
    };
    let need = ENTRY_HEADER as u64 + u64::from(h.id_size) + u64::from(h.pad_size) + u64::from(h.data_size);
    (u64::from(h.size) >= need && h.size as usize >= ENTRY_HEADER).then_some(h)
}

/// `(first entry offset, end of used space)` from a file header, for the supported
/// format versions (Windows 8 and later).
fn parse_file_header(b: &[u8]) -> Option<(u64, u64)> {
    if b.get(..4)? != b"CMMM" || !(30..=40).contains(&u32_at(b, 4)?) {
        return None;
    }
    Some((u64::from(u32_at(b, 16)?), u64::from(u32_at(b, 20)?)))
}

/// Index of the entries in one database image: hash → entry (entries with no picture,
/// which Explorer writes as "no thumbnail" markers, are skipped).
fn index_bytes(file: &[u8]) -> HashMap<u64, Entry> {
    let mut out = HashMap::new();
    let Some((first, used)) = parse_file_header(file) else { return out };
    let end = if used > first && used <= file.len() as u64 { used } else { file.len() as u64 };
    let mut off = first;
    while off + ENTRY_HEADER as u64 <= end {
        let Some(h) = parse_entry_header(&file[off as usize..]) else { break };
        if h.data_size > 0 && h.data_size <= MAX_ENTRY_DATA && h.hash != 0 {
            let data_off = off + ENTRY_HEADER as u64 + u64::from(h.id_size) + u64::from(h.pad_size);
            out.insert(
                h.hash,
                Entry { start: off, offset: data_off, size: h.data_size, width: h.width, height: h.height },
            );
        }
        off += u64::from(h.size);
    }
    out
}

/// Look up `id` in a database image held in memory (tests, fuzzing).
pub fn lookup_in_bytes(file: &[u8], id: u64) -> Option<(u32, u32, Vec<u8>)> {
    let e = index_bytes(file).get(&id).copied()?;
    let data = file.get(e.offset as usize..e.offset as usize + e.size as usize)?;
    Some((e.width, e.height, data.to_vec()))
}

/// The folder holding the caches (`%LOCALAPPDATA%\Microsoft\Windows\Explorer`).
pub fn cache_dir() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("Microsoft\\Windows\\Explorer"))
}

struct DbIndex {
    len: u64,
    mtime: Option<std::time::SystemTime>,
    entries: HashMap<u64, Entry>,
}

/// Indexes of the size classes, by database path.
static INDEXES: parking_lot::Mutex<Option<HashMap<PathBuf, DbIndex>>> = parking_lot::Mutex::new(None);

/// Upper bound on a database we index (bigger files are skipped).
const MAX_DB_BYTES: u64 = 512 * 1024 * 1024;

fn entry_in(db: &Path, id: u64, rebuild: bool) -> Option<Entry> {
    let meta = std::fs::metadata(db).ok()?;
    if meta.len() < 24 || meta.len() > MAX_DB_BYTES {
        return None;
    }
    let mtime = meta.modified().ok();
    let mut guard = INDEXES.lock();
    let map = guard.get_or_insert_with(HashMap::new);
    let fresh = map.get(db).is_some_and(|i| i.len == meta.len() && i.mtime == mtime && !rebuild);
    if !fresh {
        // `std::fs::read` opens with share read / write / delete, so Explorer keeps working.
        let bytes = std::fs::read(db).ok()?;
        map.insert(db.to_path_buf(), DbIndex { len: meta.len(), mtime, entries: index_bytes(&bytes) });
    }
    map.get(db)?.entries.get(&id).copied()
}

/// Read the entry's picture after re-validating its header (Explorer may have
/// rewritten the file since it was indexed).
fn read_entry(db: &Path, id: u64, e: Entry) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(db).ok()?;
    f.seek(SeekFrom::Start(e.start)).ok()?;
    let mut hdr = [0u8; ENTRY_HEADER];
    f.read_exact(&mut hdr).ok()?;
    let h = parse_entry_header(&hdr)?;
    let data_off = e.start + ENTRY_HEADER as u64 + u64::from(h.id_size) + u64::from(h.pad_size);
    if h.hash != id || h.data_size != e.size || data_off != e.offset {
        return None;
    }
    f.seek(SeekFrom::Start(data_off)).ok()?;
    let mut data = vec![0u8; e.size as usize];
    f.read_exact(&mut data).ok()?;
    Some(data)
}

/// The cached picture of `path` closest to `px` (longest edge), when Explorer has one
/// that is at least three quarters of `px`: size classes ≥ `px` are tried smallest first,
/// then smaller ones largest first.
pub fn lookup(path: &Path, px: u32) -> Option<OsThumb> {
    let dir = cache_dir()?;
    let id = cache_id(path)?;
    lookup_id(&dir, id, px)
}

/// [`lookup`] for a known cache id in `dir`.
pub fn lookup_id(dir: &Path, id: u64, px: u32) -> Option<OsThumb> {
    let mut order: Vec<u32> = DB_SIZES.iter().copied().filter(|s| *s >= px).collect();
    order.extend(DB_SIZES.iter().rev().copied().filter(|s| *s < px));
    for size in order {
        let db = dir.join(format!("thumbcache_{size}.db"));
        for attempt in 0..2 {
            let Some(e) = entry_in(&db, id, attempt == 1) else { break };
            if e.width.max(e.height) * 4 < px * 3 {
                break;
            }
            match read_entry(&db, id, e).filter(|d| looks_like_picture(d)) {
                Some(data) => return Some(OsThumb { data, width: e.width, height: e.height, db_size: size }),
                // Stale offset (the file changed under us): rebuild once.
                None => continue,
            }
        }
    }
    None
}

/// BMP, JPEG or PNG magic.
fn looks_like_picture(d: &[u8]) -> bool {
    d.starts_with(b"BM") || d.starts_with(&[0xFF, 0xD8, 0xFF]) || d.starts_with(b"\x89PNG\r\n\x1a\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A complete vector measured on Windows 11 (build 26300): the inputs of a JPEG in a
    /// temp folder and the `System.ThumbnailCacheId` the shell reported for it, which
    /// was also the key of its entry in `thumbcache_256.db`.
    #[test]
    fn matches_a_measured_cache_id() {
        // Volume {b0c76d5d-8227-4f96-b2ea-a4c95ebd409c}, file index 0x800000047130c.
        let guid = [0x5d, 0x6d, 0xc7, 0xb0, 0x27, 0x82, 0x96, 0x4f, 0xb2, 0xea, 0xa4, 0xc9, 0x5e, 0xbd, 0x40, 0x9c];
        let id = cache_id_from(&guid, 0x8_0000_0047_130c, ".jpg", 0x5d48_9ef5, 0x0098_36be);
        assert_eq!(id, 0x3bcb_b8ae_bb2d_9d0c);
        // Case of the extension matters; a zero precision loss is not hashed.
        assert_ne!(cache_id_from(&guid, 0x8_0000_0047_130c, ".JPG", 0x5d48_9ef5, 0x0098_36be), id);
        assert_ne!(cache_id_from(&guid, 0x8_0000_0047_130c, ".jpg", 0x5d48_9ef5, 0), id);
    }

    #[test]
    fn extension_keeps_case_and_dot() {
        assert_eq!(hashed_extension(Path::new("a/B.JPG")), ".JPG");
        assert_eq!(hashed_extension(Path::new("a/noext")), "");
    }

    fn entry(hash: u64, w: u32, h: u32, data: &[u8]) -> Vec<u8> {
        let id: Vec<u8> = format!("{hash:x}").encode_utf16().flat_map(u16::to_le_bytes).collect();
        let size = (ENTRY_HEADER + id.len() + data.len()) as u32;
        let mut e = Vec::new();
        e.extend_from_slice(b"CMMM");
        e.extend_from_slice(&size.to_le_bytes());
        e.extend_from_slice(&hash.to_le_bytes());
        e.extend_from_slice(&(id.len() as u32).to_le_bytes());
        e.extend_from_slice(&0u32.to_le_bytes());
        e.extend_from_slice(&(data.len() as u32).to_le_bytes());
        e.extend_from_slice(&w.to_le_bytes());
        e.extend_from_slice(&h.to_le_bytes());
        e.extend_from_slice(&[0u8; 4 + 8 + 8]);
        e.extend_from_slice(&id);
        e.extend_from_slice(data);
        e
    }

    fn db(entries: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = entries.concat();
        let mut f = Vec::new();
        f.extend_from_slice(b"CMMM");
        f.extend_from_slice(&32u32.to_le_bytes());
        f.extend_from_slice(&4u32.to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&24u32.to_le_bytes());
        f.extend_from_slice(&(24 + body.len() as u32).to_le_bytes());
        f.extend_from_slice(&body);
        f.extend_from_slice(&[0u8; 64]);
        f
    }

    #[test]
    fn indexes_and_reads_entries() {
        let png = b"\x89PNG\r\n\x1a\nrest".to_vec();
        let file = db(&[entry(7, 256, 192, &png), entry(9, 0, 0, &[]), entry(11, 96, 72, b"BMxx")]);
        assert_eq!(lookup_in_bytes(&file, 7), Some((256, 192, png)));
        assert_eq!(lookup_in_bytes(&file, 9), None, "markers without data are skipped");
        assert_eq!(lookup_in_bytes(&file, 11).map(|t| t.2), Some(b"BMxx".to_vec()));
        assert_eq!(lookup_in_bytes(&file, 12), None);
    }

    #[test]
    fn rejects_old_versions_and_garbage() {
        let mut file = db(&[entry(7, 1, 1, b"BM")]);
        file[4] = 21; // Windows 7
        assert_eq!(lookup_in_bytes(&file, 7), None);
        assert_eq!(lookup_in_bytes(b"CMMM", 7), None);
        let mut bad = db(&[entry(7, 1, 1, b"BM")]);
        bad[24 + 4..24 + 8].copy_from_slice(&10u32.to_le_bytes()); // entry smaller than its header
        assert_eq!(lookup_in_bytes(&bad, 7), None);
    }

    #[test]
    fn lookup_prefers_the_smallest_large_enough_class() {
        let dir = tempfile::tempdir().unwrap();
        let png = b"\x89PNG\r\n\x1a\nx".to_vec();
        std::fs::write(dir.path().join("thumbcache_96.db"), db(&[entry(5, 96, 64, &png)])).unwrap();
        std::fs::write(dir.path().join("thumbcache_256.db"), db(&[entry(5, 256, 170, &png)])).unwrap();
        assert_eq!(lookup_id(dir.path(), 5, 128).map(|t| t.db_size), Some(256));
        assert_eq!(lookup_id(dir.path(), 5, 64).map(|t| t.db_size), Some(96));
        // 96 px is too small for a 512 px request; the 256 px one is kept as well
        // because nothing ≥ 512 exists and 256 * 4 < 512 * 3.
        assert_eq!(lookup_id(dir.path(), 5, 512), None);
        assert_eq!(lookup_id(dir.path(), 6, 64), None);
    }

    /// Reads the real caches of this machine (Windows only, prints a summary).
    #[test]
    #[cfg(windows)]
    #[ignore = "reads the current user's Explorer thumbnail caches"]
    fn real_caches_parse() {
        let dir = cache_dir().unwrap();
        for s in DB_SIZES {
            let p = dir.join(format!("thumbcache_{s}.db"));
            if let Ok(b) = std::fs::read(&p) {
                println!("{s}: {} entries", index_bytes(&b).len());
            }
        }
    }

    /// `SOYUZ_THUMBCACHE_PROBE=<file>`: look the file up in the real caches (a file
    /// Explorer has shown as a thumbnail must be found).
    #[test]
    #[cfg(windows)]
    #[ignore = "needs a file Explorer has thumbnailed"]
    fn real_lookup_probe() {
        let p = PathBuf::from(std::env::var("SOYUZ_THUMBCACHE_PROBE").unwrap());
        let id = cache_id(&p).unwrap();
        let t = lookup(&p, 256).expect("no entry");
        println!(
            "{id:x}: {}x{} from thumbcache_{}.db, {} bytes, {:?}",
            t.width,
            t.height,
            t.db_size,
            t.data.len(),
            &t.data[..4]
        );
    }
}
