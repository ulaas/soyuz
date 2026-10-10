//! NTFS change journal (USN journal) access for the background index.
//!
//! [`UsnVolume::open`] opens the volume device (`\\.\C:`), which Windows only allows for
//! administrators on most systems: callers get `FsErrorKind::PermissionDenied` and should
//! fall back to crawling + watching. Journal and MFT reads return raw kernel buffers (an
//! 8-byte cursor followed by `USN_RECORD_V2` records); `soyuz-index::usn` parses them.
//! Every other platform returns `Unsupported`.

use std::path::{Path, PathBuf};

#[cfg(not(windows))]
use soyuz_core::FsError;
use soyuz_core::FsResult;

/// `USN_JOURNAL_DATA` of a volume.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JournalInfo {
    pub journal_id: u64,
    pub first_usn: i64,
    pub next_usn: i64,
    pub lowest_valid_usn: i64,
    pub max_usn: i64,
}

/// An open NTFS volume. `Send`.
#[derive(Debug)]
pub struct UsnVolume {
    root: PathBuf,
    #[cfg(windows)]
    h: crate::sys::windows::usn::VolumeHandle,
}

impl UsnVolume {
    /// Open the volume whose root is `root` (`C:\` or a mount folder).
    #[track_caller]
    pub fn open(root: &Path) -> FsResult<UsnVolume> {
        let _guard = crate::guard::check("usn_open");
        #[cfg(windows)]
        {
            let h = crate::sys::windows::usn::VolumeHandle::open(root).map_err(crate::err::map("usn_open", root))?;
            Ok(UsnVolume { root: root.to_path_buf(), h })
        }
        #[cfg(not(windows))]
        {
            Err(FsError::unsupported("usn_open", Some(root)))
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `FSCTL_QUERY_USN_JOURNAL`. Fails when the volume has no journal (not NTFS/ReFS,
    /// or journaling turned off).
    pub fn query(&self) -> FsResult<JournalInfo> {
        #[cfg(windows)]
        {
            let r = self.h.query().map_err(crate::err::map("usn_query", &self.root))?;
            Ok(JournalInfo {
                journal_id: r.id,
                first_usn: r.first_usn,
                next_usn: r.next_usn,
                lowest_valid_usn: r.lowest_valid_usn,
                max_usn: r.max_usn,
            })
        }
        #[cfg(not(windows))]
        {
            Err(FsError::unsupported("usn_query", Some(&self.root)))
        }
    }

    /// `FSCTL_READ_USN_JOURNAL` from `start_usn` into `buf`; returns the number of bytes
    /// (8 means "no more records").
    pub fn read_journal(&self, journal_id: u64, start_usn: i64, buf: &mut [u8]) -> FsResult<usize> {
        #[cfg(windows)]
        {
            self.h.read(journal_id, start_usn, buf).map_err(crate::err::map("usn_read", &self.root))
        }
        #[cfg(not(windows))]
        {
            let _ = (journal_id, start_usn, buf);
            Err(FsError::unsupported("usn_read", Some(&self.root)))
        }
    }

    /// `FSCTL_ENUM_USN_DATA` (whole-volume MFT enumeration) from `start_frn`; `None` at
    /// the end.
    pub fn enum_mft(&self, start_frn: u64, buf: &mut [u8]) -> FsResult<Option<usize>> {
        #[cfg(windows)]
        {
            self.h.enum_mft(start_frn, buf).map_err(crate::err::map("usn_enum", &self.root))
        }
        #[cfg(not(windows))]
        {
            let _ = (start_frn, buf);
            Err(FsError::unsupported("usn_enum", Some(&self.root)))
        }
    }

    /// Current path of the file with this reference number (`NotFound` once deleted).
    pub fn path_of(&self, frn: u64) -> FsResult<PathBuf> {
        #[cfg(windows)]
        {
            self.h.path_of(frn).map_err(crate::err::map("usn_path", &self.root))
        }
        #[cfg(not(windows))]
        {
            let _ = frn;
            Err(FsError::unsupported("usn_path", Some(&self.root)))
        }
    }
}

/// Give the calling thread the lowest CPU priority (Windows; no-op elsewhere). I/O
/// priority is left alone: background mode starves a crawl behind any other disk
/// activity. Returns whether it took effect.
pub fn low_priority_thread() -> bool {
    #[cfg(windows)]
    {
        crate::sys::windows::usn::lowest_priority()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_reports_a_clean_error_or_works() {
        // Without administrator rights this fails with PermissionDenied; elevated runs
        // can query the system volume's journal.
        let root = crate::volumes::volume_root(&std::env::temp_dir()).unwrap_or_else(|_| PathBuf::from("C:\\"));
        match UsnVolume::open(&root) {
            Ok(v) => {
                if let Ok(j) = v.query() {
                    assert!(j.next_usn >= j.first_usn);
                }
            }
            Err(e) => assert!(matches!(
                e.kind,
                soyuz_core::FsErrorKind::PermissionDenied
                    | soyuz_core::FsErrorKind::Unsupported
                    | soyuz_core::FsErrorKind::NotFound
                    | soyuz_core::FsErrorKind::Io
            )),
        }
        std::thread::spawn(low_priority_thread).join().unwrap();
    }
}
