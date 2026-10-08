//! Archive formats and detection.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveFormat {
    Zip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
    TarZst,
    SevenZ,
    /// Read-only.
    #[cfg(feature = "rar")]
    Rar,
}

impl ArchiveFormat {
    /// Every format compiled in.
    pub fn all() -> &'static [ArchiveFormat] {
        &[
            ArchiveFormat::Zip,
            ArchiveFormat::Tar,
            ArchiveFormat::TarGz,
            ArchiveFormat::TarBz2,
            ArchiveFormat::TarXz,
            ArchiveFormat::TarZst,
            ArchiveFormat::SevenZ,
            #[cfg(feature = "rar")]
            ArchiveFormat::Rar,
        ]
    }

    /// File extensions (lower-case, no leading dot), preferred first.
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            ArchiveFormat::Zip => &["zip"],
            ArchiveFormat::Tar => &["tar"],
            ArchiveFormat::TarGz => &["tar.gz", "tgz"],
            ArchiveFormat::TarBz2 => &["tar.bz2", "tbz2", "tbz"],
            ArchiveFormat::TarXz => &["tar.xz", "txz"],
            ArchiveFormat::TarZst => &["tar.zst", "tzst"],
            ArchiveFormat::SevenZ => &["7z"],
            #[cfg(feature = "rar")]
            ArchiveFormat::Rar => &["rar"],
        }
    }

    /// `TarZst` is writable only with the `zstd` feature; RAR never.
    pub fn can_write(&self) -> bool {
        match self {
            #[cfg(feature = "rar")]
            ArchiveFormat::Rar => false,
            ArchiveFormat::TarZst => cfg!(feature = "zstd"),
            _ => true,
        }
    }

    pub fn supports_password(&self) -> bool {
        match self {
            ArchiveFormat::Zip | ArchiveFormat::SevenZ => true,
            #[cfg(feature = "rar")]
            ArchiveFormat::Rar => true,
            _ => false,
        }
    }
}

/// Magic bytes + extension.
///
/// - The file head is sniffed (zip, 7z, rar, ustar / v7 tar, gzip / bzip2 / xz / zstd).
///   A compressed stream counts as `tar.*` when the extension says so, or when its first
///   decompressed block is a valid tar header (so `x.gz` holding a tar is found, a plain
///   `x.gz` is not an archive).
/// - Magic wins over a wrong extension (`a.zip` that is really a 7z).
/// - Zip-magic files with some other, non-archive extension (`.docx`, `.jar`, `.apk`, ...)
///   return `None`: they are documents / packages first.
/// - Unreadable files (or no recognizable magic) fall back to the extension.
pub fn detect(path: &Path) -> Option<ArchiveFormat> {
    let by_ext = detect_by_extension(path);
    let head = read_head(path);
    let Some(head) = head else { return by_ext };
    match sniff(path, &head, by_ext) {
        Sniff::Format(f) => {
            let has_other_ext = by_ext.is_none() && path.extension().is_some();
            if f == ArchiveFormat::Zip && has_other_ext { None } else { Some(f) }
        }
        Sniff::NotArchive => None,
        Sniff::Unknown => by_ext,
    }
}

enum Sniff {
    Format(ArchiveFormat),
    /// Recognized, but not something we can open (plain .gz, RAR without the feature).
    NotArchive,
    Unknown,
}

fn read_head(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    if !f.metadata().ok()?.is_file() {
        return None;
    }
    let mut buf = Vec::with_capacity(512);
    f.take(512).read_to_end(&mut buf).ok()?;
    Some(buf)
}

#[derive(Clone, Copy)]
enum Codec {
    Gz,
    Bz2,
    Xz,
    Zst,
}

fn sniff(path: &Path, head: &[u8], by_ext: Option<ArchiveFormat>) -> Sniff {
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") || head.starts_with(b"PK\x07\x08") {
        return Sniff::Format(ArchiveFormat::Zip);
    }
    if head.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        return Sniff::Format(ArchiveFormat::SevenZ);
    }
    if head.starts_with(b"Rar!\x1a\x07") {
        #[cfg(feature = "rar")]
        return Sniff::Format(ArchiveFormat::Rar);
        #[cfg(not(feature = "rar"))]
        return Sniff::NotArchive;
    }
    let codec = if head.starts_with(&[0x1F, 0x8B]) {
        Some((Codec::Gz, ArchiveFormat::TarGz))
    } else if head.starts_with(b"BZh") {
        Some((Codec::Bz2, ArchiveFormat::TarBz2))
    } else if head.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0x00]) {
        Some((Codec::Xz, ArchiveFormat::TarXz))
    } else if head.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        Some((Codec::Zst, ArchiveFormat::TarZst))
    } else {
        None
    };
    if let Some((codec, fmt)) = codec {
        if by_ext == Some(fmt) {
            return Sniff::Format(fmt);
        }
        return if decompressed_is_tar(path, codec) { Sniff::Format(fmt) } else { Sniff::NotArchive };
    }
    if is_tar_header(head) {
        return Sniff::Format(ArchiveFormat::Tar);
    }
    Sniff::Unknown
}

fn decompressed_is_tar(path: &Path, codec: Codec) -> bool {
    use std::io::Read;
    let Ok(f) = std::fs::File::open(path) else { return false };
    let f = std::io::BufReader::new(f);
    let mut r: Box<dyn Read> = match codec {
        Codec::Gz => Box::new(flate2::read::GzDecoder::new(f)),
        Codec::Bz2 => Box::new(bzip2::read::BzDecoder::new(f)),
        Codec::Xz => Box::new(lzma_rust2::XzReader::new(f, true)),
        Codec::Zst => Box::new(crate::reader::zst::ZstdReader::new(f)),
    };
    let mut block = [0u8; 512];
    r.read_exact(&mut block).is_ok() && is_tar_header(&block)
}

/// ustar / GNU magic, or a v7 header with a valid checksum.
pub(crate) fn is_tar_header(h: &[u8]) -> bool {
    if h.len() < 512 {
        return false;
    }
    if &h[257..262] == b"ustar" {
        return true;
    }
    // v7: checksum field (148..156) is octal of the byte sum with the field as spaces.
    let field = &h[148..156];
    let digits: String = field.iter().take_while(|b| **b != 0 && **b != b' ').map(|b| *b as char).collect::<String>();
    let Ok(stored) = u32::from_str_radix(digits.trim(), 8) else { return false };
    let sum: u32 =
        h[..512].iter().enumerate().map(|(i, b)| if (148..156).contains(&i) { 32 } else { u32::from(*b) }).sum();
    h[0] != 0 && stored == sum
}

/// Extension only (longest match wins, case-insensitive).
pub fn detect_by_extension(path: &Path) -> Option<ArchiveFormat> {
    let name = path.file_name()?.to_string_lossy().to_lowercase();
    let mut best: Option<(usize, ArchiveFormat)> = None;
    for f in ArchiveFormat::all() {
        for ext in f.extensions() {
            let dotted = format!(".{ext}");
            if name.len() > dotted.len() && name.ends_with(&dotted) && best.is_none_or(|(l, _)| dotted.len() > l) {
                best = Some((dotted.len(), *f));
            }
        }
    }
    best.map(|(_, f)| f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_by_extension() {
        assert_eq!(detect(Path::new("/a/B.ZIP")), Some(ArchiveFormat::Zip));
        assert_eq!(detect(Path::new("x.tar.gz")), Some(ArchiveFormat::TarGz));
        assert_eq!(detect(Path::new("x.tgz")), Some(ArchiveFormat::TarGz));
        assert_eq!(detect(Path::new("x.tar")), Some(ArchiveFormat::Tar));
        assert_eq!(detect(Path::new("x.tar.zst")), Some(ArchiveFormat::TarZst));
        assert_eq!(detect(Path::new("x.7z")), Some(ArchiveFormat::SevenZ));
        assert_eq!(detect(Path::new("x.txt")), None);
        assert_eq!(detect(Path::new(".zip")), None);
        assert!(ArchiveFormat::Zip.supports_password());
        assert!(!ArchiveFormat::TarXz.supports_password());
        assert!(ArchiveFormat::SevenZ.can_write());
    }

    #[test]
    fn detect_by_magic() {
        let d = tempfile::tempdir().unwrap();
        // A 7z disguised as .zip.
        let p = d.path().join("fake.zip");
        std::fs::write(&p, [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C, 0, 4]).unwrap();
        assert_eq!(detect(&p), Some(ArchiveFormat::SevenZ));
        // A zip with a document extension is not treated as an archive...
        let p = d.path().join("report.docx");
        std::fs::write(&p, b"PK\x03\x04rest").unwrap();
        assert_eq!(detect(&p), None);
        // ...but one without an extension is.
        let p = d.path().join("blob");
        std::fs::write(&p, b"PK\x03\x04rest").unwrap();
        assert_eq!(detect(&p), Some(ArchiveFormat::Zip));
        // gzip holding a tar, with a plain .gz extension.
        let mut tar = vec![0u8; 1024];
        tar[..5].copy_from_slice(b"a.txt");
        tar[257..262].copy_from_slice(b"ustar");
        let p = d.path().join("x.gz");
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut enc, &tar).unwrap();
        std::fs::write(&p, enc.finish().unwrap()).unwrap();
        assert_eq!(detect(&p), Some(ArchiveFormat::TarGz));
        // gzip of non-tar data is not an archive.
        let p = d.path().join("notes.gz");
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut enc, &[b'x'; 600]).unwrap();
        std::fs::write(&p, enc.finish().unwrap()).unwrap();
        assert_eq!(detect(&p), None);
        // Plain tar without extension.
        let p = d.path().join("tarball");
        std::fs::write(&p, &tar).unwrap();
        assert_eq!(detect(&p), Some(ArchiveFormat::Tar));
        // Missing file: extension fallback.
        assert_eq!(detect(&d.path().join("missing.tar.xz")), Some(ArchiveFormat::TarXz));
    }
}
