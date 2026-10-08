#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use soyuz_archive::{ArchiveEntry, ArchiveError, ArchiveFormat, CreateOptions, ExtractSink};
use soyuz_core::{ConflictResolution, Control, ErrorAction, OpControl};

/// Deterministic pseudo-random bytes (poorly compressible).
pub fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// A small tree under `root/tree`: nested dirs, an empty dir, an empty file, a 1.5 MiB file,
/// a unicode name. Returns `root/tree`.
pub fn make_tree(root: &Path) -> PathBuf {
    let t = root.join("tree");
    std::fs::create_dir_all(t.join("sub/deeper")).unwrap();
    std::fs::create_dir_all(t.join("empty_dir")).unwrap();
    std::fs::write(t.join("hello.txt"), b"hello world\n").unwrap();
    std::fs::write(t.join("empty.bin"), b"").unwrap();
    std::fs::write(t.join("sub/big.bin"), noise(1_500_000, 7)).unwrap();
    std::fs::write(t.join("sub/deeper/zeros.dat"), vec![0u8; 300_000]).unwrap();
    std::fs::write(t.join("sub/deeper/\u{00e7}i\u{011f}_\u{65e5}\u{672c}.txt"), "unicode".as_bytes()).unwrap();
    t
}

/// Relative path → `Some(content)` for files, `None` for dirs.
pub fn snapshot(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for de in std::fs::read_dir(&d).unwrap() {
            let p = de.unwrap().path();
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            let md = std::fs::symlink_metadata(&p).unwrap();
            if md.is_dir() {
                out.insert(rel, None);
                stack.push(p);
            } else {
                out.insert(rel, Some(std::fs::read(&p).unwrap()));
            }
        }
    }
    out
}

pub fn opts(format: ArchiveFormat, base: &Path) -> CreateOptions {
    CreateOptions {
        format,
        level: 6,
        password: None,
        base_dir: base.to_path_buf(),
        store_symlinks: true,
        follow_symlinks: false,
    }
}

pub fn ext(format: ArchiveFormat) -> &'static str {
    format.extensions()[0]
}

#[derive(Default)]
pub struct TestSink {
    pub conflict: Option<ConflictResolution>,
    pub on_error: Option<ErrorAction>,
    pub errors: Vec<(String, ArchiveError)>,
    pub conflicts: Vec<PathBuf>,
    pub entries: Vec<PathBuf>,
    pub bytes: u64,
    /// Cancel `ctl` once this many bytes were reported.
    pub cancel_after: Option<(Arc<OpControl>, u64)>,
}

impl ExtractSink for TestSink {
    fn on_entry(&mut self, _e: &ArchiveEntry, dst: &Path) -> Control {
        self.entries.push(dst.to_path_buf());
        Control::Continue
    }
    fn on_bytes(&mut self, n: u64) {
        self.bytes += n;
        if let Some((ctl, after)) = &self.cancel_after
            && self.bytes >= *after
        {
            ctl.cancel();
        }
    }
    fn on_conflict(&mut self, dst: &Path, _e: &ArchiveEntry) -> ConflictResolution {
        self.conflicts.push(dst.to_path_buf());
        self.conflict.clone().unwrap_or(ConflictResolution::Skip)
    }
    fn on_error(&mut self, e: &ArchiveEntry, err: &ArchiveError) -> ErrorAction {
        self.errors.push((e.path.clone(), err.clone()));
        self.on_error.unwrap_or(ErrorAction::Skip)
    }
}

/// No leftover temp files anywhere under `root`.
pub fn assert_no_temp_files(root: &Path) {
    for (rel, _) in snapshot(root) {
        assert!(!rel.contains(".soyuz-extract-") && !rel.ends_with(".part"), "leftover temp file {rel}");
    }
}
