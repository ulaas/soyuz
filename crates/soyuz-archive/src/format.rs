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

    pub fn can_write(&self) -> bool {
        match self {
            #[cfg(feature = "rar")]
            ArchiveFormat::Rar => false,
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

/// Magic bytes via `infer` + extension (tar.* by extension + gzip/bz2/xz/zstd magic).
/// M0: extension only (longest match wins, case-insensitive).
pub fn detect(path: &Path) -> Option<ArchiveFormat> {
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
}
