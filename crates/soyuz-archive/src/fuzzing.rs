//! Entry points for the `fuzz/` targets (cargo feature `fuzzing`): format detection and the
//! archive readers on untrusted bytes. Each function must never panic for any input.

use std::io::Read;
use std::path::PathBuf;

/// Tar header recognition on a raw 512-byte block (or less).
pub fn tar_header(data: &[u8]) {
    let _ = crate::format::is_tar_header(data);
}

/// Write `data` to a per-process temp file named `fuzz.<ext>`, detect its format, open it,
/// list the entries and read a little of each (readers stream from files).
pub fn detect_and_list(data: &[u8], ext: &str) {
    let dir = std::env::temp_dir().join(format!("soyuz-fuzz-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path: PathBuf = dir.join(format!("fuzz.{ext}"));
    if std::fs::write(&path, data).is_err() {
        return;
    }
    let _ = crate::format::detect_by_extension(&path);
    if crate::format::detect(&path).is_none() {
        return;
    }
    let Ok(mut r) = crate::open(&path, Some("password")) else { return };
    let n = r.entries().map(|e| e.len()).unwrap_or(0);
    for i in 0..n.min(16) {
        if let Ok(mut rd) = r.open_entry(i) {
            let mut buf = [0u8; 4096];
            let _ = rd.read(&mut buf);
        }
    }
    let _ = r.dir_index();
}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        super::tar_header(&[0u8; 512]);
        super::tar_header(b"short");
        super::detect_and_list(b"PK\x03\x04garbage", "zip");
        super::detect_and_list(b"7z\xBC\xAF\x27\x1C\x00\x04", "7z");
        super::detect_and_list(&[0x1f, 0x8b, 8, 0, 0, 0], "tar.gz");
        super::detect_and_list(b"", "tar");
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join(format!("soyuz-fuzz-{}", std::process::id())));
    }
}
