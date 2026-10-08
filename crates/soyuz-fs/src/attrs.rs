//! File attributes.

use std::path::Path;

use serde::{Deserialize, Serialize};
use soyuz_core::{Attributes, FsResult};

use crate::baseline;

/// Attribute bits to set and to clear.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AttrSet {
    pub set: Attributes,
    pub clear: Attributes,
}

/// Windows attributes / chflags on macOS / immutable on Linux; unsupported bits →
/// `Err(Unsupported)`. Baseline supports only `READONLY`.
pub fn set_attributes(path: &Path, a: &AttrSet) -> FsResult<()> {
    baseline::set_attributes(path, a)
}
