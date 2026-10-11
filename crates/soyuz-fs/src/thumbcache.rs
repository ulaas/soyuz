//! Read-only access to Explorer's thumbnail cache (`%LOCALAPPDATA%\Microsoft\Windows\
//! Explorer\thumbcache_<size>.db`), as a fast thumbnail source that needs no
//! shell API (`IThumbnailCache` is never used).
//!
//! **Cache key.** Every entry is keyed by the 64-bit `System.ThumbnailCacheId`. Explorer
//! derives it in the Windows shell (`CalculateHashKey`); the algorithm was reverse engineered by
//! Y. Khatri ("Windows 7 Thumbcache hash algorithm", swiftforensics.com, 2012) and is
//! re-implemented in Thumbcache Viewer (`map_entries.cpp`, E. Kutcher, with the Windows
//! 8.1+ precision-loss step). Starting from the seed `0x95E729BA2C37FD21`, each input
//! byte `b` is mixed as `h ^= h * 0x820 + b + (h >> 2)` (wrapping u64), over: the volume
//! GUID (16 bytes, memory layout), the NTFS file index (u64 LE, with the sequence
//! number), the extension as UTF-16LE with its dot and case kept (empty for folders),
//! the UTC last-write time as `(fat_date << 16) | fat_time` (u32 LE) and, when nonzero,
//! the low 32 bits of (that DOS time converted back to a FILETIME − the original) (u32
//! LE). Verified on Windows 11 build 26300 against `System.ThumbnailCacheId` and
//! the entries in `thumbcache_*.db` for files on two NTFS volumes.
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
//!
//! **Linux / BSD** use the freedesktop thumbnail cache instead ("Thumbnail Managing
//! Standard" 0.9): `$XDG_CACHE_HOME/thumbnails/{normal,large,x-large,xx-large}/
//! <md5 of the file URI>.png`, written by the desktop's thumbnailers (GNOME, KDE, …). An
//! entry is only used when its `Thumb::URI` and `Thumb::MTime` text chunks match the
//! file, so stale thumbnails of replaced files are never shown. Thumbnails Soyuz decodes
//! itself are written back ([`write_freedesktop`]) so other apps can reuse them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Seed of the cache-id hash (a constant in the Windows shell).
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

/// The folder holding the caches (`%LOCALAPPDATA%\Microsoft\Windows\Explorer`;
/// `$XDG_CACHE_HOME/thumbnails` on Linux / BSD; `None` on macOS).
pub fn cache_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA").map(|l| PathBuf::from(l).join("Microsoft\\Windows\\Explorer"));
    }
    if cfg!(all(unix, not(target_os = "macos"))) {
        return Some(crate::launch::linux::host_cache_home().join("thumbnails"));
    }
    None
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
///
/// Linux / BSD: the freedesktop cache ([`lookup_freedesktop`]) with the same size rule.
pub fn lookup(path: &Path, px: u32) -> Option<OsThumb> {
    let dir = cache_dir()?;
    if cfg!(all(unix, not(target_os = "macos"))) {
        return lookup_freedesktop(&dir, path, px);
    }
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

/// Freedesktop size classes (folder, longest edge in px), smallest first.
pub const FREEDESKTOP_SIZES: [(&str, u32); 4] = [("normal", 128), ("large", 256), ("x-large", 512), ("xx-large", 1024)];

/// Cache file name of a URI: lowercase hex MD5 of the URI + `.png`.
pub fn freedesktop_thumb_name(uri: &str) -> String {
    use md5::Digest;
    let d = md5::Md5::digest(uri.as_bytes());
    let mut s = String::with_capacity(36);
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s.push_str(".png");
    s
}

/// The `file://` URI the thumbnailers key `path` by (absolute, percent-encoded).
pub fn freedesktop_uri(path: &Path) -> String {
    crate::launch::linux::uri::file_uri(path)
}

/// `(keyword, text)` pairs of a PNG's `tEXt` chunks.
pub type TextChunks = Vec<(String, String)>;

/// `tEXt` chunks and the IHDR size of a PNG (`None` when it is not a PNG).
pub fn png_text_chunks(png: &[u8]) -> Option<(u32, u32, TextChunks)> {
    let mut rest = png.strip_prefix(b"\x89PNG\r\n\x1a\n")?;
    let (mut w, mut h) = (0, 0);
    let mut texts = Vec::new();
    while rest.len() >= 12 {
        let len = u32::from_be_bytes(rest[..4].try_into().ok()?) as usize;
        let kind = &rest[4..8];
        let data = rest.get(8..8usize.checked_add(len)?)?;
        match kind {
            b"IHDR" if len >= 8 => {
                w = u32::from_be_bytes(data[..4].try_into().ok()?);
                h = u32::from_be_bytes(data[4..8].try_into().ok()?);
            }
            b"tEXt" => {
                if let Some(z) = data.iter().position(|&c| c == 0) {
                    // Keywords are Latin-1; the values we compare are ASCII URIs / numbers.
                    let k: String = data[..z].iter().map(|&c| c as char).collect();
                    let v = String::from_utf8_lossy(&data[z + 1..]).into_owned();
                    texts.push((k, v));
                }
            }
            b"IDAT" | b"IEND" => break,
            _ => {}
        }
        rest = rest.get(12usize.checked_add(len)?..)?;
    }
    Some((w, h, texts))
}

/// A freedesktop thumbnail is valid for a file with this URI and mtime (whole seconds;
/// a fractional `Thumb::MTime` written by some thumbnailers is truncated).
pub fn freedesktop_thumb_valid(texts: &[(String, String)], uri: &str, mtime_secs: i64) -> bool {
    let get = |k: &str| texts.iter().find(|(key, _)| key == k).map(|(_, v)| v.trim());
    let mtime_ok =
        get("Thumb::MTime").and_then(|v| v.parse::<f64>().ok()).is_some_and(|m| m.trunc() as i64 == mtime_secs);
    get("Thumb::URI") == Some(uri) && mtime_ok
}

/// The cached thumbnail of `path` in the freedesktop cache under `dir`, closest to `px`
/// (classes ≥ `px` smallest first, then smaller ones largest first; at least three
/// quarters of `px`). Stale entries (other URI / mtime) are skipped.
pub fn lookup_freedesktop(dir: &Path, path: &Path, px: u32) -> Option<OsThumb> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    let abs = if path.is_absolute() { path.to_path_buf() } else { std::fs::canonicalize(path).ok()? };
    let uri = freedesktop_uri(&abs);
    let name = freedesktop_thumb_name(&uri);
    let mut order: Vec<(&str, u32)> = FREEDESKTOP_SIZES.iter().copied().filter(|s| s.1 >= px).collect();
    order.extend(FREEDESKTOP_SIZES.iter().rev().copied().filter(|s| s.1 < px));
    for (folder, class) in order {
        let Ok(data) = std::fs::read(dir.join(folder).join(&name)) else { continue };
        let Some((w, h, texts)) = png_text_chunks(&data) else { continue };
        if !freedesktop_thumb_valid(&texts, &uri, mtime) {
            continue;
        }
        // u64: width / height come from an untrusted PNG header.
        if u64::from(w.max(h)) * 4 < u64::from(px) * 3 {
            break;
        }
        return Some(OsThumb { data, width: w, height: h, db_size: class });
    }
    None
}

/// The freedesktop class a thumbnail requested at `px` is written to: the smallest class
/// ≥ `px` (`xx-large` for anything bigger).
pub fn freedesktop_class(px: u32) -> (&'static str, u32) {
    FREEDESKTOP_SIZES.iter().copied().find(|s| s.1 >= px).unwrap_or(FREEDESKTOP_SIZES[3])
}

/// `png` with `tEXt` chunks inserted right after its IHDR (`None` when it is not a PNG
/// or a keyword is not 1–79 Latin-1 bytes without NUL).
pub fn png_with_text(png: &[u8], texts: &[(&str, &str)]) -> Option<Vec<u8>> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    let rest = png.strip_prefix(SIG)?;
    if rest.get(4..8)? != b"IHDR" {
        return None;
    }
    let ihdr_end = SIG.len() + 12 + u32::from_be_bytes(rest.get(..4)?.try_into().ok()?) as usize;
    if ihdr_end > png.len() {
        return None;
    }
    let mut out = Vec::with_capacity(png.len() + texts.iter().map(|(k, v)| k.len() + v.len() + 13).sum::<usize>());
    out.extend_from_slice(&png[..ihdr_end]);
    for (k, v) in texts {
        if k.is_empty() || k.len() > 79 || k.bytes().any(|b| b == 0) || v.bytes().any(|b| b == 0) {
            return None;
        }
        let data = [k.as_bytes(), b"\0", v.as_bytes()].concat();
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut crc = crc32fast::Hasher::new();
        crc.update(b"tEXt");
        crc.update(&data);
        out.extend_from_slice(b"tEXt");
        out.extend_from_slice(&data);
        out.extend_from_slice(&crc.finalize().to_be_bytes());
    }
    out.extend_from_slice(&png[ihdr_end..]);
    Some(out)
}

/// Files whose thumbnails are never written: anything inside the thumbnail cache `dir`
/// itself, and items in a trash (`trash_home` = `$XDG_DATA_HOME/Trash`, or any
/// `.Trash` / `.Trash-<uid>` folder on a volume).
pub fn freedesktop_skip(dir: &Path, trash_home: &Path, path: &Path) -> bool {
    path.starts_with(dir)
        || path.starts_with(trash_home)
        || path.components().any(|c| {
            let s = c.as_os_str().to_string_lossy();
            s == ".Trash" || s.starts_with(".Trash-")
        })
}

/// Write `png` (an RGBA PNG whose longest edge is at most `class_px`, without text chunks)
/// as the freedesktop thumbnail of `path` into the cache under `dir` (`<dir>/<class>/
/// <md5 of the URI>.png`), with the `Thumb::URI`, `Thumb::MTime`, `Thumb::Size`,
/// `Thumb::Mimetype` (when given) and `Software` chunks. Atomic: a 0600 temp file in the
/// same folder, then a rename; missing folders are created 0700.
///
/// `expected_mtime` is the file's mtime before it was decoded: when the file changed
/// since (or is gone, or is in a trash / the cache itself), nothing is written and
/// `Ok(None)` is returned. Otherwise the written path.
pub fn write_freedesktop_in(
    dir: &Path,
    trash_home: &Path,
    path: &Path,
    png: &[u8],
    class_px: u32,
    expected_mtime: Option<std::time::SystemTime>,
    mime: Option<&str>,
) -> std::io::Result<Option<PathBuf>> {
    use std::io::Write;
    let abs = if path.is_absolute() { path.to_path_buf() } else { std::path::absolute(path)? };
    if freedesktop_skip(dir, trash_home, &abs) {
        return Ok(None);
    }
    let meta = std::fs::metadata(&abs)?;
    let mtime = meta.modified()?;
    if !meta.is_file() || expected_mtime.is_some_and(|e| e != mtime) {
        return Ok(None);
    }
    let secs = mtime.duration_since(std::time::UNIX_EPOCH).map_err(std::io::Error::other)?.as_secs();
    let uri = freedesktop_uri(&abs);
    let (secs, size) = (secs.to_string(), meta.len().to_string());
    let mut texts = vec![("Thumb::URI", uri.as_str()), ("Thumb::MTime", secs.as_str()), ("Thumb::Size", size.as_str())];
    if let Some(m) = mime.filter(|m| !m.is_empty()) {
        texts.push(("Thumb::Mimetype", m));
    }
    texts.push(("Software", "Soyuz"));
    let invalid = || std::io::Error::new(std::io::ErrorKind::InvalidData, "not a PNG");
    let data = png_with_text(png, &texts).ok_or_else(invalid)?;
    let (w, h, _) = png_text_chunks(&data).ok_or_else(invalid)?;
    if w == 0 || h == 0 || w.max(h) > class_px {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "thumbnail larger than its class"));
    }
    let folder = FREEDESKTOP_SIZES
        .iter()
        .find(|s| s.1 == class_px)
        .map(|s| s.0)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a freedesktop size class"))?;
    let target_dir = dir.join(folder);
    let mut db = std::fs::DirBuilder::new();
    db.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut db, 0o700);
    db.create(&target_dir)?;
    let name = freedesktop_thumb_name(&uri);
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = target_dir.join(format!(".{name}.soyuz-{}-{seq}.tmp", std::process::id()));
    let mut oo = std::fs::OpenOptions::new();
    oo.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut oo, 0o600);
    let result = (|| {
        let mut f = oo.open(&tmp)?;
        f.write_all(&data)?;
        f.flush()?;
        drop(f);
        let dest = target_dir.join(&name);
        std::fs::rename(&tmp, &dest)?;
        Ok(dest)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map(Some)
}

/// [`write_freedesktop_in`] into the user's cache (`$XDG_CACHE_HOME/thumbnails`), with
/// the file's MIME type. Linux / BSD only (`Ok(None)` elsewhere). Blocking: call it on a
/// worker thread.
pub fn write_freedesktop(
    path: &Path,
    png: &[u8],
    class_px: u32,
    expected_mtime: Option<std::time::SystemTime>,
) -> std::io::Result<Option<PathBuf>> {
    if !cfg!(all(unix, not(target_os = "macos"))) {
        return Ok(None);
    }
    let Some(dir) = cache_dir() else { return Ok(None) };
    let trash_home = crate::launch::linux::data_home().join("Trash");
    let mime = crate::launch::mime_type(path);
    write_freedesktop_in(&dir, &trash_home, path, png, class_px, expected_mtime, mime.as_deref())
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

    fn png(w: u32, h: u32, texts: &[(&str, &str)]) -> Vec<u8> {
        fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            out.extend_from_slice(&[0; 4]); // CRC (not checked)
        }
        let mut p = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        chunk(&mut p, b"IHDR", &ihdr);
        for (k, v) in texts {
            chunk(&mut p, b"tEXt", &[k.as_bytes(), b"\0", v.as_bytes()].concat());
        }
        chunk(&mut p, b"IDAT", &[0; 4]);
        chunk(&mut p, b"IEND", &[]);
        p
    }

    #[test]
    fn freedesktop_names_and_chunks() {
        // Value from the Thumbnail Managing Standard's example.
        assert_eq!(freedesktop_thumb_name("file:///home/jens/photos/me.png"), "c6ee772d9e49320e97ec29a7eb5b1697.png");
        let p = png(256, 192, &[("Thumb::URI", "file:///x.jpg"), ("Thumb::MTime", "1700000000")]);
        let (w, h, t) = png_text_chunks(&p).unwrap();
        assert_eq!((w, h), (256, 192));
        assert!(freedesktop_thumb_valid(&t, "file:///x.jpg", 1_700_000_000));
        assert!(!freedesktop_thumb_valid(&t, "file:///x.jpg", 1_700_000_001), "stale mtime");
        assert!(!freedesktop_thumb_valid(&t, "file:///y.jpg", 1_700_000_000), "other file");
        assert!(png_text_chunks(b"GIF89a").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn freedesktop_lookup_validates_and_picks_sizes() {
        let d = tempfile::tempdir().unwrap();
        let cache = d.path().join("thumbnails");
        let f = d.path().join("photo 1.jpg");
        std::fs::write(&f, b"jpeg").unwrap();
        let mtime =
            std::fs::metadata(&f).unwrap().modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let uri = freedesktop_uri(&f);
        assert!(uri.starts_with("file:///") && uri.ends_with("/photo%201.jpg"), "{uri}");
        let name = freedesktop_thumb_name(&uri);
        let put = |folder: &str, w: u32, h: u32, mt: u64| {
            std::fs::create_dir_all(cache.join(folder)).unwrap();
            let p = png(w, h, &[("Thumb::URI", &uri), ("Thumb::MTime", &mt.to_string())]);
            std::fs::write(cache.join(folder).join(&name), p).unwrap();
        };
        put("normal", 128, 96, mtime);
        put("large", 256, 192, mtime + 5); // stale
        assert_eq!(lookup_freedesktop(&cache, &f, 128).map(|t| (t.db_size, t.width)), Some((128, 128)));
        assert_eq!(lookup_freedesktop(&cache, &f, 160).map(|t| t.db_size), Some(128), "stale large is skipped");
        assert_eq!(lookup_freedesktop(&cache, &f, 256), None, "128 px is too small for 256");
        put("large", 256, 192, mtime);
        assert_eq!(lookup_freedesktop(&cache, &f, 200).map(|t| t.db_size), Some(256));
        assert_eq!(lookup_freedesktop(&cache, &d.path().join("missing.jpg"), 128), None);
        // A hostile IHDR size must not overflow the size check.
        put("x-large", u32::MAX, 1, mtime);
        assert_eq!(lookup_freedesktop(&cache, &f, 400).map(|t| t.db_size), Some(512));
    }

    /// A real (zlib-compressed) RGBA PNG, as the app's encoder makes it.
    fn rgba_png(w: u32, h: u32) -> Vec<u8> {
        // Stored deflate blocks: enough for a valid tiny PNG without a zlib dependency.
        let row = 1 + w as usize * 4;
        let raw: Vec<u8> =
            (0..h).flat_map(|_| std::iter::once(0u8).chain(std::iter::repeat_n(0x80, row - 1))).collect();
        let mut z = vec![0x78, 0x01];
        for (i, block) in raw.chunks(65_535).enumerate() {
            let last = (i + 1) * 65_535 >= raw.len();
            z.push(u8::from(last));
            z.extend_from_slice(&(block.len() as u16).to_le_bytes());
            z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
            z.extend_from_slice(block);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &x in &raw {
            a = (a + u32::from(x)) % 65_521;
            b = (b + a) % 65_521;
        }
        z.extend_from_slice(&((b << 16) | a).to_be_bytes());
        fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut c = crc32fast::Hasher::new();
            c.update(kind);
            c.update(data);
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            out.extend_from_slice(&c.finalize().to_be_bytes());
        }
        let mut p = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        chunk(&mut p, b"IHDR", &ihdr);
        chunk(&mut p, b"IDAT", &z);
        chunk(&mut p, b"IEND", &[]);
        p
    }

    /// Every chunk's CRC is right and the chunk sequence ends with IEND.
    fn chunks_valid(png: &[u8]) -> bool {
        let Some(mut rest) = png.strip_prefix(b"\x89PNG\r\n\x1a\n") else { return false };
        while rest.len() >= 12 {
            let len = u32::from_be_bytes(rest[..4].try_into().unwrap()) as usize;
            let Some(body) = rest.get(4..8 + len) else { return false };
            let crc = u32::from_be_bytes(rest[8 + len..12 + len].try_into().unwrap());
            if crc32fast::hash(body) != crc {
                return false;
            }
            if &body[..4] == b"IEND" {
                return rest.len() == 12;
            }
            rest = &rest[12 + len..];
        }
        false
    }

    #[test]
    fn text_chunks_are_inserted_with_valid_crcs() {
        let p = rgba_png(4, 3);
        assert!(chunks_valid(&p));
        let t = png_with_text(&p, &[("Thumb::URI", "file:///a%20b.png"), ("Software", "Soyuz")]).unwrap();
        assert!(chunks_valid(&t));
        let (w, h, texts) = png_text_chunks(&t).unwrap();
        assert_eq!((w, h), (4, 3));
        assert_eq!(texts[0], ("Thumb::URI".into(), "file:///a%20b.png".into()));
        assert_eq!(texts[1], ("Software".into(), "Soyuz".into()));
        assert!(png_with_text(b"GIF89a", &[]).is_none());
        assert!(png_with_text(&p, &[("", "x")]).is_none());
        assert!(png_with_text(&p, &[("k", "a\0b")]).is_none());
    }

    #[test]
    fn classes_and_skips() {
        assert_eq!(freedesktop_class(64), ("normal", 128));
        assert_eq!(freedesktop_class(128), ("normal", 128));
        assert_eq!(freedesktop_class(129), ("large", 256));
        assert_eq!(freedesktop_class(512), ("x-large", 512));
        assert_eq!(freedesktop_class(4000), ("xx-large", 1024));
        let (cache, trash) = (Path::new("/c/thumbnails"), Path::new("/d/Trash"));
        assert!(freedesktop_skip(cache, trash, Path::new("/c/thumbnails/normal/x.png")));
        assert!(freedesktop_skip(cache, trash, Path::new("/d/Trash/files/a.jpg")));
        assert!(freedesktop_skip(cache, trash, Path::new("/mnt/usb/.Trash-1000/files/a.jpg")));
        assert!(freedesktop_skip(cache, trash, Path::new("/mnt/usb/.Trash/1000/files/a.jpg")));
        assert!(!freedesktop_skip(cache, trash, Path::new("/home/u/Pictures/a.jpg")));
        assert!(!freedesktop_skip(cache, trash, Path::new("/home/u/.Trashy/a.jpg")));
    }

    #[cfg(unix)]
    #[test]
    fn freedesktop_write_round_trips_atomically_with_private_modes() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let cache = d.path().join("cache/thumbnails");
        let trash = d.path().join("data/Trash");
        let f = d.path().join("my photo #1.png");
        std::fs::write(&f, b"source").unwrap();
        let mtime = std::fs::metadata(&f).unwrap().modified().unwrap();
        let png = rgba_png(128, 96);
        let out = write_freedesktop_in(&cache, &trash, &f, &png, 128, Some(mtime), Some("image/png")).unwrap().unwrap();
        assert_eq!(out, cache.join("normal").join(freedesktop_thumb_name(&freedesktop_uri(&f))));
        // Read back through the normal lookup.
        let t = lookup_freedesktop(&cache, &f, 128).unwrap();
        assert_eq!((t.width, t.height, t.db_size), (128, 96, 128));
        assert!(chunks_valid(&t.data));
        let (_, _, texts) = png_text_chunks(&t.data).unwrap();
        let get = |k: &str| texts.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        assert_eq!(get("Thumb::Size").as_deref(), Some("6"));
        assert_eq!(get("Thumb::Mimetype").as_deref(), Some("image/png"));
        assert_eq!(get("Software").as_deref(), Some("Soyuz"));
        let secs = mtime.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs().to_string();
        assert_eq!(get("Thumb::MTime"), Some(secs), "integer seconds");
        // Modes: 0600 file, 0700 folders (umask can only remove bits).
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&out), 0o600);
        assert_eq!(mode(&cache), 0o700);
        assert_eq!(mode(&cache.join("normal")), 0o700);
        // No temp files left behind; a rewrite replaces the entry in place.
        write_freedesktop_in(&cache, &trash, &f, &png, 128, Some(mtime), None).unwrap().unwrap();
        let names: Vec<_> = std::fs::read_dir(cache.join("normal")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1, "{names:?}");
        // Changed during decoding → nothing written.
        let other = mtime - std::time::Duration::from_secs(10);
        assert_eq!(write_freedesktop_in(&cache, &trash, &f, &png, 256, Some(other), None).unwrap(), None);
        assert!(!cache.join("large").exists());
        // Too big for the class, or not a class.
        assert!(write_freedesktop_in(&cache, &trash, &f, &rgba_png(200, 10), 128, None, None).is_err());
        assert!(write_freedesktop_in(&cache, &trash, &f, &png, 100, None, None).is_err());
        // Files in the trash or the cache itself are skipped.
        std::fs::create_dir_all(trash.join("files")).unwrap();
        let trashed = trash.join("files/x.png");
        std::fs::write(&trashed, b"x").unwrap();
        assert_eq!(write_freedesktop_in(&cache, &trash, &trashed, &png, 128, None, None).unwrap(), None);
        assert_eq!(write_freedesktop_in(&cache, &trash, &out, &png, 128, None, None).unwrap(), None);
        // Missing source.
        assert!(write_freedesktop_in(&cache, &trash, &d.path().join("gone.png"), &png, 128, None, None).is_err());
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
