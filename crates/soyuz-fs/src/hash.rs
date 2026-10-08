//! File checksums.

use std::path::Path;

use soyuz_core::{FsResult, HashAlgo, OpControl, ProgressSink};

use crate::baseline;

/// Lower-case hex digest of the file contents (CRC-32 as 8 hex digits).
pub fn hash_file(path: &Path, algo: HashAlgo, ctl: &OpControl, sink: &mut dyn ProgressSink) -> FsResult<String> {
    baseline::hash_file(path, algo, ctl, sink)
}
