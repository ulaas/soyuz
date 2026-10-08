//! Duplicate finder: size → 4 KiB head blake3 → full blake3.

use std::path::PathBuf;
use std::sync::Arc;

use soyuz_core::{EntryInfo, OpControl};

#[derive(Clone, Debug)]
pub enum DupeEvent {
    Progress {
        hashed: u64,
        total: u64,
        bytes: u64,
    },
    /// One group of identical files.
    Group(Vec<EntryInfo>),
    Done {
        groups: u64,
        wasted_bytes: u64,
    },
}

/// Find duplicate files (≥ `min_size` bytes) under `roots` in the background.
pub fn find_duplicates(
    roots: Vec<PathBuf>,
    min_size: u64,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> (crossbeam_channel::Receiver<DupeEvent>, Arc<OpControl>) {
    let _ = (roots, min_size, wake);
    todo!("M6: soyuz_search::find_duplicates")
}
