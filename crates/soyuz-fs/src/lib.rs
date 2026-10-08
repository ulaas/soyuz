//! `soyuz-fs`: the platform filesystem layer.
//!
//! All filesystem work goes through kernel32/ntdll/advapi32 on Windows and
//! rustix/libc on unix — never through a shell or COM shell API (CI enforces this).
//!
//! Per-OS implementations (PLAN §4):
//! * listing: `FindFirstFileExW(FIND_FIRST_EX_LARGE_FETCH)` / `getdents` + `statx` /
//!   `readdir` + `fstatat`;
//! * copy: block clone / `CopyFileExW` (progress callback, pause & cancel inside it) /
//!   `FICLONE` → `copy_file_range` / `fclonefileat`, sparse-aware, metadata preserved;
//! * move / delete / links / times / attributes / ACL view via raw calls, junctions with a
//!   hand-written `REPARSE_DATA_BUFFER`;
//! * volumes with free space, a `notify` watcher, default-app launching through the
//!   registry association chain (no shell API), `.lnk` parsing, and hand-written trash
//!   (freedesktop spec, `~/.Trash` + index, `$Recycle.Bin` `$I`/`$R` pairs).
//!
//! `unsafe` is confined to `sys/*`.
#![deny(unsafe_code)]

pub mod attrs;
pub mod chrome;
pub mod copy;
pub mod create;
pub mod delete;
mod err;
pub mod fault;
#[cfg(any(test, feature = "fuzzing"))]
pub mod fuzzing;
pub mod guard;
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
    CopyMethod, CopyOptions, CopyOutcome, FileIdentity, Preserve, Reflink, SkipReason, copy_dir_metadata, copy_file,
    copy_symlink, device_of, file_identity, same_file,
};
pub use create::{create_dir, create_dir_all, create_file, write_atomic};
pub use delete::{DeleteOptions, DeleteStats, delete_empty_dir, delete_file, delete_link, remove_tree, secure_wipe};
pub use guard::{on_ui_thread, register_ui_thread};
pub use hash::hash_file;
pub use launch::AppInfo;
pub use links::{LinkKind, hardlink, junction, read_link, resolve_link, symlink};
pub use list::{
    ListOptions, ListStats, WalkItem, WalkOptions, WalkStats, exists_nofollow, read_dir, read_dir_streaming, stat, walk,
};
pub use meta::{MetaSnapshot, meta_snapshot};
pub use mv::{
    MoveFast, MoveOptions, RenameMode, is_case_variant, on_disk_name, rename, rename_case_only, try_rename_or_crossdev,
};
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
