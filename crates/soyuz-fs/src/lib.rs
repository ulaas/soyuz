//! `soyuz-fs`: the platform filesystem layer.
//!
//! All filesystem work goes through kernel32/ntdll/advapi32 on Windows and
//! rustix/libc on unix — never through a shell or COM shell API (CI enforces this).
//!
//! **M0 status:** every public function has a portable `std::fs` implementation in
//! [`baseline`] (real behaviour, no progress fast paths). Trash, junctions and ACL views
//! return `Unsupported`. The per-OS raw implementations (in `sys/*`) replace the bodies
//! of the functions in the public modules without changing any signature (PLAN §2.3).
#![deny(unsafe_code)]

pub mod attrs;
pub mod baseline;
pub mod copy;
pub mod create;
pub mod delete;
pub mod hash;
pub mod launch;
pub mod links;
pub mod list;
pub mod meta;
pub mod mv;
pub mod path;
pub mod perms;
pub mod sparse;
pub mod times;
pub mod trash;
pub mod volumes;
pub mod watch;
pub mod xattr;

#[allow(unsafe_code)]
mod sys;

pub use attrs::{AttrSet, set_attributes};
pub use copy::{
    CopyMethod, CopyOptions, CopyOutcome, Preserve, Reflink, SkipReason, copy_dir_metadata, copy_file, copy_symlink,
    device_of, same_file,
};
pub use create::{create_dir, create_dir_all, create_file, write_atomic};
pub use delete::{DeleteOptions, DeleteStats, delete_empty_dir, delete_file, delete_link, remove_tree, secure_wipe};
pub use hash::hash_file;
pub use launch::AppInfo;
pub use links::{LinkKind, hardlink, junction, read_link, resolve_link, symlink};
pub use list::{
    ListOptions, ListStats, WalkItem, WalkOptions, WalkStats, exists_nofollow, read_dir, read_dir_streaming, stat, walk,
};
pub use meta::{MetaSnapshot, meta_snapshot};
pub use mv::{MoveFast, MoveOptions, RenameMode, rename, rename_case_only, try_rename_or_crossdev};
pub use path::{expand_env, home, is_root, normalize, path_overlaps, strip_verbatim, to_wide_long};
pub use perms::{AccessRights, AceLite, SecurityInfo, chown, effective_access, security_info, set_mode};
pub use times::{TimesSet, set_times};
pub use trash::{TrashItem, TrashSupport, trash_support};
pub use volumes::{FsCaps, FsProbe, RealProbe, VolumeInfo, VolumeKind, space, volume_root, volumes};
pub use watch::{WatchEvent, WatchId, WatchKind, Watcher};

// Re-export the shared vocabulary most callers need alongside this crate.
pub use soyuz_core::{
    ByteProgress, Control, DeviceId, EntryInfo, EntryKind, FsError, FsErrorKind, FsResult, HashAlgo, OpControl,
    Overwrite, ProgressSink,
};
