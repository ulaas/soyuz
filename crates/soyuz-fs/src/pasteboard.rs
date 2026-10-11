//! macOS general pasteboard (`NSPasteboard`) for file copy / paste: file URLs
//! (`public.file-url`, one pasteboard item per file) plus the paths as plain text.
//! macOS has no standard "cut" flavour, so callers keep their own cut state keyed by
//! [`change_count`].

use std::path::PathBuf;

use crate::sys::macos::pasteboard as pb;

/// The pasteboard's change count: changes whenever any application writes to it.
pub fn change_count() -> isize {
    pb::change_count()
}

/// Replace the pasteboard with `paths` (as file URLs) and `text`. Returns the change count
/// after the write, `None` when nothing could be written.
pub fn write_files(paths: &[PathBuf], text: &str) -> Option<isize> {
    pb::write_files(paths, text)
}

/// Local file paths on the pasteboard (`None` when it holds no file URLs).
pub fn read_files() -> Option<Vec<PathBuf>> {
    pb::read_files()
}

/// Empty the pasteboard.
pub fn clear() {
    pb::clear();
}
