//! Archive readers (zip, tar.*, 7z). Implemented in M7.

mod sevenz;
mod tar;
mod zip;

use std::path::Path;

use crate::{ArchiveError, ArchiveFormat, ArchiveReader};

/// Open `path` as `format`. M0: `Unsupported` for every format.
pub(crate) fn open(
    path: &Path,
    format: ArchiveFormat,
    password: Option<&str>,
) -> Result<Box<dyn ArchiveReader>, ArchiveError> {
    let _ = (path, password);
    Err(ArchiveError::Unsupported(format))
}
