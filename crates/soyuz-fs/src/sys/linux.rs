//! Linux needs no raw FFI beyond rustix (statx, FICLONE, copy_file_range, renameat2,
//! FS_IOC_GETFLAGS/SETFLAGS, fallocate, fadvise are all safe rustix calls made from the
//! public modules); this module is intentionally empty.
