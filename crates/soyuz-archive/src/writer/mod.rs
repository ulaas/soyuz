//! Archive writers (zip, tar.*, 7z). Implemented in M7.

mod sevenz;
mod tar;
mod zip;

use std::path::{Path, PathBuf};

use soyuz_core::{OpControl, ProgressSink};

use crate::{ArchiveError, CreateOptions};

/// M0: `Unsupported` for every format.
pub(crate) fn create(
    dest: &Path,
    inputs: &[PathBuf],
    opts: &CreateOptions,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> Result<(), ArchiveError> {
    let _ = (dest, inputs, ctl, sink);
    Err(ArchiveError::Unsupported(opts.format))
}
