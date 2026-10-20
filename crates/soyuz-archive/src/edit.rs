//! Editing inside an archive (add / delete / rename entries) by rewriting it: zip, tar,
//! tar.gz / .bz2 / .xz (/ .zst with the `zstd` feature) and 7z.
//!
//! [`rewrite_archive`] writes a *new* archive (the caller picks the output path, normally
//! [`edit_temp_path`]); one planner ([`plan`]) decides what survives, under which name,
//! and what is added, then each format writes it:
//! - zip ([`rewrite_zip`]): unchanged entries are copied raw (no recompression; encrypted
//!   entries stay encrypted), renamed ones raw under their new name;
//! - tar.*: the entries stream through with their headers (mode, owner, times, links)
//!   under their new names, recompressed with the same codec at `level`;
//! - 7z: entries are decoded and encoded again (7z blocks can't be copied one entry at a
//!   time), non-solid LZMA2 at `level`; encrypted 7z archives are refused.
//!
//! Deleted entries are left out and added files are compressed fresh. The original is
//! never modified; replacing it (and keeping it as an undo backup in the trash) is the
//! job layer's business (`soyuz-jobs` `JobKind::EditArchive`).

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
use crate::util::{ProgressReader, fs_err};
use crate::writer::{Item, collect};
use crate::{ArchiveError, ArchiveFormat, CreateOptions, ZipMethod};

/// One change to an archive (the name predates tar / 7z editing). Inner paths are
/// `/`-separated, relative to the archive root (normalized: `\`, `.` and leading `/` are
/// fine).
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

/// What [`rewrite_archive`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZipEditSummary {
    /// Entries copied unchanged.
    pub kept: u64,
    /// Entries copied under a new name.
    pub renamed: u64,
    /// Entries left out.
    pub removed: u64,
    /// Entries written from disk.
    pub added: u64,
}

/// Archives [`rewrite_archive`] can edit: zip, tar, tar.gz / .bz2 / .xz (tar.zst with
/// the `zstd` feature) and 7z; split `.001` sets and spanned `.z01` … `.zip` sets are
/// read-only.
pub fn can_edit(path: &Path) -> bool {
    let format = crate::detect(path);
    let editable = matches!(
        format,
        Some(
            ArchiveFormat::Zip
                | ArchiveFormat::Tar
                | ArchiveFormat::TarGz
                | ArchiveFormat::TarBz2
                | ArchiveFormat::TarXz
                | ArchiveFormat::SevenZ
        )
    ) || (format == Some(ArchiveFormat::TarZst) && cfg!(feature = "zstd"));
    editable && !crate::reader::volumes::is_multi_volume(path)
}

/// Hidden sibling used as the rewrite target for `archive`.
pub fn edit_temp_path(archive: &Path) -> PathBuf {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    archive.with_file_name(format!(".{name}.soyuz-edit.part"))
}

enum Src {
    /// An entry of the source archive (slot `i` is its entry `i`).
    Old,
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

/// The edited archive's entries: the old ones (slot `i` is old entry `i`; dead ones are
/// left out) followed by the added ones, and the added files themselves.
pub(crate) struct Plan {
    slots: Vec<Slot>,
    items: Vec<Item>,
    /// The old entries' names, to tell kept from renamed.
    old: Vec<String>,
}

impl Plan {
    /// The slot of old entry `i`.
    fn old(&self, i: usize) -> &Slot {
        &self.slots[i]
    }

    /// Kept (same name) or renamed, for the summary.
    fn count_old(&self, i: usize, summary: &mut ZipEditSummary) {
        if self.slots[i].name == self.old[i] {
            summary.kept += 1;
        } else {
            summary.renamed += 1;
        }
    }

    /// The added entries.
    fn added(&self) -> impl Iterator<Item = (&Slot, &Item)> {
        self.slots.iter().filter_map(|s| match s.src {
            Src::New(k) if s.alive => Some((s, &self.items[k])),
            _ => None,
        })
    }

    fn added_bytes(&self) -> u64 {
        self.added()
            .map(|(_, i)| match i.kind {
                crate::writer::ItemKind::File { len } => len,
                _ => 0,
            })
            .sum()
    }
}

/// Apply `edits` to the old entries `(normalized name, is_dir)` (an empty name is a dead
/// entry, left out).
pub(crate) fn plan(
    old: Vec<(String, bool)>,
    edits: &[ZipEdit],
    level: u8,
    ctl: &OpControl,
) -> Result<Plan, ArchiveError> {
    let mut slots: Vec<Slot> = old
        .iter()
        .map(|(name, is_dir)| Slot { src: Src::Old, name: name.clone(), is_dir: *is_dir, alive: !name.is_empty() })
        .collect();
    let mut items: Vec<Item> = Vec::new();
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
    Ok(Plan { slots, items, old: old.into_iter().map(|(n, _)| n).collect() })
}

/// Apply `edits` to `src` (any archive [`can_edit`] accepts) and write the result to
/// `out` (created or truncated). On error or cancellation `out` is removed. `level` 0-9
/// is the compression level for added files (and, for tar.* and 7z, the whole archive).
pub fn rewrite_archive(
    src: &Path,
    out: &Path,
    edits: &[ZipEdit],
    level: u8,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> Result<ZipEditSummary, ArchiveError> {
    let level = level.min(9);
    let res = match crate::detect(src) {
        Some(ArchiveFormat::Zip) => zip_inner(src, out, edits, level, ctl, sink),
        Some(
            f @ (ArchiveFormat::Tar
            | ArchiveFormat::TarGz
            | ArchiveFormat::TarBz2
            | ArchiveFormat::TarXz
            | ArchiveFormat::TarZst),
        ) => tar_inner(src, out, f, edits, level, ctl, sink),
        Some(ArchiveFormat::SevenZ) if !crate::reader::volumes::is_multi_volume(src) => {
            sevenz_inner(src, out, edits, level, ctl, sink)
        }
        Some(f) => Err(ArchiveError::Unsupported(f)),
        None => Err(ArchiveError::Corrupt("not an archive".into())),
    };
    if res.is_err() {
        let _ = std::fs::remove_file(out);
    }
    res.map_err(|e| if ctl.is_cancelled() { ArchiveError::Cancelled } else { e })
}

/// [`rewrite_archive`] for a zip.
pub fn rewrite_zip(
    src: &Path,
    out: &Path,
    edits: &[ZipEdit],
    level: u8,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> Result<ZipEditSummary, ArchiveError> {
    let res = zip_inner(src, out, edits, level, ctl, sink);
    if res.is_err() {
        let _ = std::fs::remove_file(out);
    }
    res.map_err(|e| if ctl.is_cancelled() { ArchiveError::Cancelled } else { e })
}

fn zip_inner(
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
    let mut old = Vec::with_capacity(zip.len());
    let mut sizes = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let f = zip.by_index_raw(i).map_err(zip_err)?;
        sizes.push(f.compressed_size());
        old.push((normalize_inner(&entry_name(&f)), f.is_dir()));
    }
    let plan = plan(old, edits, level, ctl)?;
    let mut summary = ZipEditSummary::default();
    let total: u64 =
        (0..sizes.len()).filter(|&i| plan.old(i).alive).map(|i| sizes[i]).sum::<u64>() + plan.added_bytes();
    let (method, zlevel) = crate::writer::zip::method_level(ZipMethod::Deflate, level)?;
    let file = File::create(out).map_err(|e| fs_err("create", out, &e))?;
    let mut zw = ZipWriter::new(BufWriter::with_capacity(crate::util::CHUNK, file)).set_auto_large_file();
    let comment = zip.comment().to_vec();
    if !comment.is_empty() {
        zw.set_raw_comment(comment.into_boxed_slice()).map_err(zip_err)?;
    }
    let mut done = 0u64;
    for (i, &size) in sizes.iter().enumerate() {
        ctl.checkpoint()?;
        let s = plan.old(i);
        if !s.alive {
            summary.removed += 1;
            continue;
        }
        let f = zip.by_index_raw(i).map_err(zip_err)?;
        if plan.old[i] == s.name {
            zw.raw_copy_file(f).map_err(zip_err)?;
        } else {
            let name = if s.is_dir { format!("{}/", s.name) } else { s.name.clone() };
            zw.raw_copy_file_rename(f, name).map_err(zip_err)?;
        }
        plan.count_old(i, &mut summary);
        done += size;
        sink.on_progress(ByteProgress { done, total, delta: size });
    }
    for (s, item) in plan.added() {
        ctl.checkpoint()?;
        crate::writer::zip::write_item(&mut zw, item, &s.name, method, zlevel, None, ctl, sink)?;
        summary.added += 1;
    }
    let mut w = zw.finish().map_err(zip_err)?;
    io::Write::flush(&mut w).map_err(|e| fs_err("write", out, &e))?;
    w.into_inner().map_err(|e| fs_err("write", out, e.error()))?.sync_all().map_err(|e| fs_err("write", out, &e))?;
    Ok(summary)
}

/// A tar.*: read twice (the plan, then the copy); entries keep their headers.
fn tar_inner(
    src: &Path,
    out: &Path,
    format: ArchiveFormat,
    edits: &[ZipEdit],
    level: u8,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> Result<ZipEditSummary, ArchiveError> {
    let err = |e: io::Error| crate::writer::zip::copy_err(e, ctl, src);
    let mut old = Vec::new();
    let mut ar = tar::Archive::new(crate::reader::tar_decoder(src, format)?);
    for e in ar.entries().map_err(err)? {
        ctl.checkpoint()?;
        let e = e.map_err(err)?;
        let name = normalize_inner(&e.path().map_err(err)?.to_string_lossy());
        old.push((name, e.header().entry_type().is_dir()));
    }
    let plan = plan(old, edits, level, ctl)?;
    let olds = plan.old.len();
    let mut summary = ZipEditSummary::default();
    crate::writer::tar::encode(out, format, level, ctl, |b| {
        let mut ar = tar::Archive::new(crate::reader::tar_decoder(src, format)?);
        for (i, e) in ar.entries().map_err(err)?.enumerate().take(olds) {
            ctl.checkpoint()?;
            let e = e.map_err(err)?;
            let s = plan.old(i);
            if !s.alive {
                summary.removed += 1;
                continue;
            }
            let mut h = e.header().clone();
            let ty = h.entry_type();
            let res = if ty.is_symlink() || ty.is_hard_link() {
                let target = e.link_name().map_err(err)?.map(|t| t.into_owned()).unwrap_or_default();
                h.set_size(0);
                b.append_link(&mut h, &s.name, target)
            } else if s.is_dir {
                h.set_size(0);
                b.append_data(&mut h, format!("{}/", s.name), io::empty())
            } else {
                // Sparse entries are written out whole.
                if ty.is_gnu_sparse() {
                    h.set_entry_type(tar::EntryType::Regular);
                }
                let len = e.size();
                h.set_size(len);
                b.append_data(&mut h, &s.name, ProgressReader::new(e, len, ctl, sink))
            };
            res.map_err(err)?;
            plan.count_old(i, &mut summary);
        }
        for (s, item) in plan.added() {
            ctl.checkpoint()?;
            crate::writer::tar::append_item(b, item, &s.name, ctl, sink)?;
            summary.added += 1;
        }
        Ok(())
    })?;
    Ok(summary)
}

/// A 7z: every kept entry is decoded and encoded again (blocks can't be copied per entry).
fn sevenz_inner(
    src: &Path,
    out: &Path,
    edits: &[ZipEdit],
    level: u8,
    ctl: &OpControl,
    sink: &mut dyn ProgressSink,
) -> Result<ZipEditSummary, ArchiveError> {
    use sevenz_rust2::{Archive, BlockDecoder, EncoderMethod, Password};
    let mut f = io::BufReader::new(File::open(src).map_err(|e| fs_err("open_archive", src, &e))?);
    let pw = Password::empty();
    let archive = Archive::read(&mut f, &pw).map_err(|e| crate::reader::sz_err(e, false))?;
    let encrypted = archive
        .blocks
        .iter()
        .any(|b| b.coders.iter().any(|c| c.encoder_method_id() == EncoderMethod::ID_AES256_SHA256));
    if encrypted {
        return Err(ArchiveError::Io(FsError::with_message(
            FsErrorKind::Unsupported,
            "edit_archive",
            Some(src),
            "encrypted 7z archives can't be edited",
        )));
    }
    let old: Vec<(String, bool)> = archive
        .files
        .iter()
        .map(|f| (if f.is_anti_item { String::new() } else { normalize_inner(&f.name) }, f.is_directory))
        .collect();
    let plan = plan(old, edits, level, ctl)?;
    let mut summary = ZipEditSummary::default();
    let mut w = crate::writer::sevenz::new_writer(out, level, None, false, ctl)?;
    let renamed = |i: usize, e: &sevenz_rust2::ArchiveEntry| {
        let mut n = e.clone();
        n.name = plan.old(i).name.clone();
        n
    };
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get() as u32).min(8);
    for block in 0..archive.blocks.len() {
        ctl.checkpoint()?;
        let mut fi = archive.stream_map.block_first_file_index[block];
        let mut fatal: Option<ArchiveError> = None;
        let res = BlockDecoder::new(threads, block, &archive, &pw, &mut f).for_each_entries(&mut |entry, r| {
            let i = fi;
            fi += 1;
            if !plan.old(i).alive {
                // Decoded anyway: the rest of the block depends on it.
                summary.removed += 1;
                return io::copy(r, &mut io::sink()).map(|_| true).map_err(sevenz_rust2::Error::from);
            }
            let rd = ProgressReader::new(r, entry.size, ctl, &mut *sink);
            match w.push_archive_entry(renamed(i, entry), Some(rd)) {
                Ok(_) => {
                    plan.count_old(i, &mut summary);
                    Ok(true)
                }
                Err(e) => {
                    fatal = Some(crate::writer::sevenz::map(e, ctl));
                    Ok(false)
                }
            }
        });
        if let Some(e) = fatal {
            return Err(e);
        }
        res.map_err(|e| crate::writer::sevenz::map(e, ctl))?;
    }
    // Folders and empty files (no data stream).
    for (i, e) in archive.files.iter().enumerate() {
        if archive.stream_map.file_block_index[i].is_some() {
            continue;
        }
        if !plan.old(i).alive {
            summary.removed += 1;
            continue;
        }
        w.push_archive_entry::<&[u8]>(renamed(i, e), None).map_err(|e| crate::writer::sevenz::map(e, ctl))?;
        plan.count_old(i, &mut summary);
    }
    for (s, item) in plan.added() {
        ctl.checkpoint()?;
        crate::writer::sevenz::push_item(&mut w, item, &s.name, ctl, sink)?;
        summary.added += 1;
    }
    crate::writer::sevenz::finish(w, out, ctl)?;
    Ok(summary)
}
