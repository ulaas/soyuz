//! Editing inside a zip (add / delete / rename entries) by rewriting it.
//!
//! [`rewrite_zip`] writes a *new* zip next to nothing in particular (the caller picks the
//! output path, normally [`edit_temp_path`]): unchanged entries are copied raw (no
//! recompression; encrypted entries stay encrypted), renamed ones are copied raw under
//! their new name, deleted ones are left out and added files are compressed fresh. The
//! original archive is never modified; replacing it (and keeping it as an undo backup in
//! the trash) is the job layer's business (`soyuz-jobs` `JobKind::EditArchive`).

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use soyuz_core::location::normalize_inner;
use soyuz_core::{ByteProgress, FsError, FsErrorKind, OpControl, ProgressSink};
use zip::ZipArchive;
use zip::write::ZipWriter;

use crate::reader::{entry_name, zip_err};
use crate::util::fs_err;
use crate::writer::{Item, collect};
use crate::{ArchiveError, ArchiveFormat, CreateOptions, ZipMethod};

/// One change to a zip archive. Inner paths are `/`-separated, relative to the archive
/// root (normalized: `\`, `.` and leading `/` are fine).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZipEdit {
    /// Add files / folders (recursively) from disk into `dest_dir` (`""` = the root).
    /// An existing file entry with the same path is replaced.
    Add { sources: Vec<PathBuf>, dest_dir: String },
    /// Remove an entry; a folder path removes everything below it.
    Delete { path: String },
    /// Rename / move an entry; a folder moves everything below it. Fails when the new
    /// path is taken.
    Rename { from: String, to: String },
}

/// What [`rewrite_zip`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZipEditSummary {
    /// Entries copied unchanged (raw).
    pub kept: u64,
    /// Entries copied raw under a new name.
    pub renamed: u64,
    /// Entries left out.
    pub removed: u64,
    /// Entries written from disk.
    pub added: u64,
}

/// Formats [`rewrite_zip`] can edit (zip only; split `.zip.001` sets and spanned
/// `.z01` … `.zip` sets are read-only).
pub fn can_edit(path: &Path) -> bool {
    crate::detect(path) == Some(ArchiveFormat::Zip) && !crate::reader::volumes::is_multi_volume(path)
}

/// Hidden sibling used as the rewrite target for `archive`.
pub fn edit_temp_path(archive: &Path) -> PathBuf {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    archive.with_file_name(format!(".{name}.soyuz-edit.part"))
}

enum Src {
    /// Entry ordinal in the source zip.
    Old(usize),
    /// Index into the added items.
    New(usize),
}

struct Slot {
    src: Src,
    /// Normalized path (no trailing `/`).
    name: String,
    is_dir: bool,
    alive: bool,
}

fn not_found(what: &str) -> ArchiveError {
    ArchiveError::Io(FsError::with_message(
        FsErrorKind::NotFound,
        "edit_archive",
        None,
        format!("{what:?} is not in the archive"),
    ))
}

fn invalid(msg: String) -> ArchiveError {
    ArchiveError::Io(FsError::with_message(FsErrorKind::InvalidName, "edit_archive", None, msg))
}

/// A valid inner path: non-empty segments, no `.` / `..`.
fn clean(path: &str) -> Result<String, ArchiveError> {
    let raw = path.trim_matches(['/', '\\']);
    if raw.split(['/', '\\']).any(|s| s == "..") {
        return Err(invalid(format!("{path:?} leaves the archive")));
    }
    let n = normalize_inner(raw);
    if n.is_empty() {
        return Err(invalid(format!("{path:?} is not an entry name")));
    }
    Ok(n)
}

fn under(name: &str, dir: &str) -> bool {
    name.len() > dir.len() && name.starts_with(dir) && name.as_bytes()[dir.len()] == b'/'
}

/// Apply `edits` to `src` and write the result to `out` (created or truncated). On error
/// or cancellation `out` is removed. `level` 0-9 is the deflate level for added files
/// (0 = store).
pub fn rewrite_zip(
    src: &Path,
    out: &Path,
    edits: &[ZipEdit],
    level: u8,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> Result<ZipEditSummary, ArchiveError> {
    let res = rewrite_inner(src, out, edits, level, ctl, sink);
    if res.is_err() {
        let _ = std::fs::remove_file(out);
    }
    res.map_err(|e| if ctl.is_cancelled() { ArchiveError::Cancelled } else { e })
}

fn rewrite_inner(
    src: &Path,
    out: &Path,
    edits: &[ZipEdit],
    level: u8,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> Result<ZipEditSummary, ArchiveError> {
    if crate::reader::volumes::is_multi_volume(src) {
        return Err(ArchiveError::Unsupported(ArchiveFormat::Zip));
    }
    let f = File::open(src).map_err(|e| fs_err("open_archive", src, &e))?;
    let mut zip = ZipArchive::new(io::BufReader::new(f)).map_err(zip_err)?;
    let mut slots = Vec::with_capacity(zip.len());
    let mut sizes = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let f = zip.by_index_raw(i).map_err(zip_err)?;
        let name = normalize_inner(&entry_name(&f));
        sizes.push(f.compressed_size());
        let alive = !name.is_empty();
        slots.push(Slot { src: Src::Old(i), is_dir: f.is_dir(), name, alive });
    }
    let mut items: Vec<Item> = Vec::new();
    let mut summary = ZipEditSummary::default();
    for edit in edits {
        ctl.checkpoint()?;
        match edit {
            ZipEdit::Delete { path } => {
                let p = clean(path)?;
                let mut hit = false;
                for s in slots.iter_mut().filter(|s| s.alive && (s.name == p || under(&s.name, &p))) {
                    s.alive = false;
                    hit = true;
                }
                if !hit {
                    return Err(not_found(path));
                }
            }
            ZipEdit::Rename { from, to } => {
                let (from_n, to_n) = (clean(from)?, clean(to)?);
                if to_n == from_n {
                    continue;
                }
                if under(&to_n, &from_n) {
                    return Err(invalid(format!("cannot move {from:?} into itself")));
                }
                let moving: Vec<usize> = (0..slots.len())
                    .filter(|&i| slots[i].alive && (slots[i].name == from_n || under(&slots[i].name, &from_n)))
                    .collect();
                if moving.is_empty() {
                    return Err(not_found(from));
                }
                // Taken names → whether that entry is a folder.
                let taken: std::collections::HashMap<String, bool> = slots
                    .iter()
                    .enumerate()
                    .filter(|(i, s)| s.alive && !moving.contains(i))
                    .map(|(_, s)| (s.name.clone(), s.is_dir))
                    .collect();
                let renamed: Vec<String> =
                    moving.iter().map(|&i| format!("{to_n}{}", &slots[i].name[from_n.len()..])).collect();
                let mut merged_dirs = HashSet::new();
                for (&i, n) in moving.iter().zip(&renamed) {
                    match taken.get(n) {
                        None => {}
                        // A folder moving onto an existing folder merges into it.
                        Some(true) if slots[i].is_dir => {
                            merged_dirs.insert(i);
                        }
                        Some(_) => return Err(ArchiveError::Conflict(PathBuf::from(n))),
                    }
                }
                for (&i, n) in moving.iter().zip(renamed) {
                    slots[i].name = n;
                    if merged_dirs.contains(&i) {
                        slots[i].alive = false;
                    }
                }
            }
            ZipEdit::Add { sources, dest_dir } => {
                let dest = if dest_dir.trim_matches(['/', '\\']).is_empty() { String::new() } else { clean(dest_dir)? };
                for source in sources {
                    let base = source.parent().map(Path::to_path_buf).unwrap_or_default();
                    let opts = CreateOptions {
                        format: ArchiveFormat::Zip,
                        level,
                        password: None,
                        base_dir: base,
                        store_symlinks: true,
                        follow_symlinks: false,
                    };
                    for item in collect(std::slice::from_ref(source), &opts, ctl)? {
                        let name = if dest.is_empty() { item.name.clone() } else { format!("{dest}/{}", item.name) };
                        let is_dir = matches!(item.kind, crate::writer::ItemKind::Dir);
                        let existing: Vec<usize> =
                            (0..slots.len()).filter(|&i| slots[i].alive && slots[i].name == name).collect();
                        if is_dir && existing.iter().any(|&i| slots[i].is_dir) {
                            continue; // folder already there: merge into it
                        }
                        if existing.iter().any(|&i| slots[i].is_dir) {
                            return Err(ArchiveError::Conflict(PathBuf::from(&name)));
                        }
                        for i in existing {
                            slots[i].alive = false; // replaced
                        }
                        slots.push(Slot { src: Src::New(items.len()), name, is_dir, alive: true });
                        items.push(item);
                    }
                }
            }
        }
    }
    // Write.
    let total: u64 = slots
        .iter()
        .filter(|s| s.alive)
        .map(|s| match s.src {
            Src::Old(i) => sizes[i],
            Src::New(k) => match items[k].kind {
                crate::writer::ItemKind::File { len } => len,
                _ => 0,
            },
        })
        .sum();
    let (method, zlevel) = crate::writer::zip::method_level(ZipMethod::Deflate, level)?;
    let file = File::create(out).map_err(|e| fs_err("create", out, &e))?;
    let mut zw = ZipWriter::new(BufWriter::with_capacity(crate::util::CHUNK, file)).set_auto_large_file();
    let comment = zip.comment().to_vec();
    if !comment.is_empty() {
        zw.set_raw_comment(comment.into_boxed_slice()).map_err(zip_err)?;
    }
    let mut done = 0u64;
    for s in &slots {
        ctl.checkpoint()?;
        match s.src {
            Src::Old(i) => {
                if !s.alive {
                    summary.removed += 1;
                    continue;
                }
                let f = zip.by_index_raw(i).map_err(zip_err)?;
                let original = normalize_inner(&entry_name(&f));
                if original == s.name {
                    zw.raw_copy_file(f).map_err(zip_err)?;
                    summary.kept += 1;
                } else {
                    let name = if s.is_dir { format!("{}/", s.name) } else { s.name.clone() };
                    zw.raw_copy_file_rename(f, name).map_err(zip_err)?;
                    summary.renamed += 1;
                }
                done += sizes[i];
                sink.on_progress(ByteProgress { done, total, delta: sizes[i] });
            }
            Src::New(k) => {
                if !s.alive {
                    continue;
                }
                let item = &items[k];
                crate::writer::zip::write_item(&mut zw, item, &s.name, method, zlevel, None, ctl, sink)?;
                if let crate::writer::ItemKind::File { len } = item.kind {
                    done += len;
                }
                summary.added += 1;
            }
        }
    }
    let mut w = zw.finish().map_err(zip_err)?;
    io::Write::flush(&mut w).map_err(|e| fs_err("write", out, &e))?;
    w.into_inner().map_err(|e| fs_err("write", out, e.error()))?.sync_all().map_err(|e| fs_err("write", out, &e))?;
    Ok(summary)
}
