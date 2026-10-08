//! Shared helpers for the integration tests.
#![allow(dead_code)]

use std::path::Path;

use soyuz_core::FsErrorKind;
use soyuz_fs::{LinkKind, symlink};

pub fn write(p: &Path, bytes: &[u8]) {
    std::fs::write(p, bytes).unwrap();
}

/// Create a symlink, or return `false` when the OS refuses for lack of privilege
/// (Windows without Developer Mode / elevation) so the test can skip.
pub fn try_symlink(target: &Path, link: &Path, kind: LinkKind) -> bool {
    match symlink(target, link, kind) {
        Ok(()) => true,
        Err(e) if e.kind == FsErrorKind::PermissionDenied => {
            eprintln!("skipping symlink part: {e}");
            false
        }
        Err(e) => panic!("symlink failed: {e}"),
    }
}

/// Deterministic pseudo-random bytes.
pub fn pattern(len: usize) -> Vec<u8> {
    let mut x: u32 = 0x1234_5678;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}
