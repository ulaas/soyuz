//! Format-independent extraction engine.
//!
//! Readers drive an [`Extractor`] in archive order, handing it each wanted entry's data
//! through [`EntryData`]. The extractor owns every safety and policy decision:
//! zip-slip / absolute-path rejection, never writing through a symlink, invalid names,
//! conflicts (policy + sink), streaming copy with progress and cancellation, temp-file +
//! rename writes, modes / times, and the summary.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use soyuz_core::{
    ConflictResolution, Control, ErrorAction, FsErrorKind, OpControl, Overwrite, unique_name, validate_name_with,
};

use crate::util::{CHUNK, EntryExtra, fs_err, fs_kind, path_escapes, read_err, resolve_link_target_via};
use crate::{ArchiveEntry, ArchiveError, ExtractOptions, ExtractSink, ExtractSummary};

/// Data source for one entry.
pub(crate) trait EntryData {
    /// Open the entry data. Streaming formats can do this only once.
    fn open(&mut self) -> Result<Box<dyn Read + '_>, ArchiveError>;
    /// Whether [`EntryData::open`] may be called again after the data was consumed (Retry).
    fn can_reopen(&self) -> bool;
}

/// An `EntryData` over a reader that can be consumed once.
pub(crate) struct OnceData<'a> {
    pub reader: Option<&'a mut dyn Read>,
}

impl EntryData for OnceData<'_> {
    fn open(&mut self) -> Result<Box<dyn Read + '_>, ArchiveError> {
        match self.reader.take() {
            Some(r) => Ok(Box::new(r)),
            None => Err(ArchiveError::Corrupt("entry data already consumed".into())),
        }
    }
    fn can_reopen(&self) -> bool {
        false
    }
}

enum Fail {
    /// Aborts the whole extraction (cancel, conflict-cancel, password problems).
    Fatal(ArchiveError),
    /// Reported to the sink; `consumed` = the entry data was (partly) read.
    Entry(ArchiveError, bool),
}

impl From<ArchiveError> for Fail {
    fn from(e: ArchiveError) -> Self {
        classify(e, false)
    }
}

fn classify(e: ArchiveError, consumed: bool) -> Fail {
    match e {
        ArchiveError::Cancelled
        | ArchiveError::Conflict(_)
        | ArchiveError::PasswordRequired
        | ArchiveError::WrongPassword => Fail::Fatal(e),
        e => Fail::Entry(e, consumed),
    }
}

enum Outcome {
    Done(u64),
    Skipped,
}

/// Where to write after conflict resolution.
enum Target {
    Write { path: PathBuf, replace: bool },
    Merge(PathBuf),
    Skip,
}

pub(crate) struct Extractor<'a> {
    entries: &'a [ArchiveEntry],
    extras: &'a [EntryExtra],
    /// Per entry index: destination path relative to `dest` (after stripping), if wanted.
    plan: Vec<Option<String>>,
    dest: PathBuf,
    opts: &'a ExtractOptions,
    ctl: &'a OpControl,
    sink: &'a mut dyn ExtractSink,
    summary: ExtractSummary,
    skip_all: bool,
    safe_dirs: HashSet<PathBuf>,
    /// Relative dir → actual directory when a dir was renamed by conflict resolution.
    dir_remap: HashMap<String, PathBuf>,
    /// Deferred directory metadata (applied after all content).
    dir_meta: Vec<(PathBuf, Option<SystemTime>, Option<u32>)>,
    /// Relative path → where the entry was actually written (for tar hard links).
    written: HashMap<String, PathBuf>,
    /// Entry path → index, built on the first hard link.
    by_path: Option<HashMap<&'a str, usize>>,
    /// Lower-cased destination paths of every symlink entry, built on the first symlink.
    link_rels: Option<HashSet<String>>,
    buf: Vec<u8>,
    tmp_counter: u64,
}

impl<'a> Extractor<'a> {
    pub fn new(
        entries: &'a [ArchiveEntry],
        extras: &'a [EntryExtra],
        selection: Option<&[usize]>,
        dest: &Path,
        opts: &'a ExtractOptions,
        ctl: &'a OpControl,
        sink: &'a mut dyn ExtractSink,
    ) -> Result<Self, ArchiveError> {
        let wanted = expand_selection(entries, selection);
        let mut strip = opts.strip_components;
        if opts.single_top_folder && single_top_dir(entries, &wanted, strip) {
            strip += 1;
        }
        let plan = entries
            .iter()
            .zip(&wanted)
            .map(|(e, &w)| {
                if !w {
                    return None;
                }
                let segs: Vec<&str> = e.path.split('/').filter(|s| !s.is_empty()).collect();
                if segs.len() <= strip {
                    // Stripped away entirely: only meaningful for files (dirs collapse to dest).
                    return if e.is_dir || segs.is_empty() { None } else { Some(String::new()) };
                }
                Some(segs[strip..].join("/"))
            })
            .collect();
        fs::create_dir_all(dest).map_err(|e| fs_err("extract", dest, &e))?;
        let mut safe_dirs = HashSet::new();
        safe_dirs.insert(dest.to_path_buf());
        Ok(Extractor {
            entries,
            extras,
            plan,
            dest: dest.to_path_buf(),
            opts,
            ctl,
            sink,
            summary: ExtractSummary::default(),
            skip_all: false,
            safe_dirs,
            dir_remap: HashMap::new(),
            dir_meta: Vec::new(),
            written: HashMap::new(),
            by_path: None,
            link_rels: None,
            buf: vec![0u8; CHUNK],
            tmp_counter: 0,
        })
    }

    /// Whether entry `idx` will be extracted.
    pub fn wants(&self, idx: usize) -> bool {
        self.plan.get(idx).is_some_and(Option::is_some)
    }

    /// Whether any wanted entry is encrypted.
    pub fn wants_encrypted(&self) -> Option<usize> {
        (0..self.entries.len()).find(|&i| self.wants(i) && self.entries[i].encrypted && !self.entries[i].is_dir)
    }

    /// Report an archive-level error for `idx` (e.g. a broken stream) to the sink.
    /// Returns `Err` when extraction must stop.
    pub fn report(&mut self, idx: usize, err: ArchiveError) -> Result<(), ArchiveError> {
        match classify(err, true) {
            Fail::Fatal(e) => Err(e),
            Fail::Entry(e, _) => {
                self.summary.entries_failed += 1;
                if self.skip_all {
                    return Ok(());
                }
                match self.sink.on_error(&self.entries[idx], &e) {
                    ErrorAction::Abort => Err(e),
                    ErrorAction::SkipAll => {
                        self.skip_all = true;
                        Ok(())
                    }
                    ErrorAction::Retry | ErrorAction::Skip => Ok(()),
                }
            }
        }
    }

    /// Extract entry `idx`. `Err` aborts the whole extraction.
    pub fn handle(&mut self, idx: usize, data: &mut dyn EntryData) -> Result<(), ArchiveError> {
        let Some(rel) = self.plan.get(idx).cloned().flatten() else { return Ok(()) };
        self.ctl.checkpoint()?;
        loop {
            let mut consumed = false;
            match self.try_entry(idx, &rel, data, &mut consumed) {
                Ok(Outcome::Done(bytes)) => {
                    self.summary.entries_ok += 1;
                    self.summary.bytes += bytes;
                    return Ok(());
                }
                Ok(Outcome::Skipped) => {
                    self.summary.entries_skipped += 1;
                    return Ok(());
                }
                Err(Fail::Fatal(e)) => return Err(e),
                Err(Fail::Entry(e, consumed)) => {
                    if self.skip_all {
                        self.summary.entries_failed += 1;
                        return Ok(());
                    }
                    match self.sink.on_error(&self.entries[idx], &e) {
                        ErrorAction::Retry if !consumed || data.can_reopen() => continue,
                        ErrorAction::Retry | ErrorAction::Skip => {}
                        ErrorAction::SkipAll => self.skip_all = true,
                        ErrorAction::Abort => return Err(e),
                    }
                    self.summary.entries_failed += 1;
                    return Ok(());
                }
            }
        }
    }

    /// Apply deferred directory metadata and return the summary.
    pub fn finish(mut self) -> ExtractSummary {
        for (path, mtime, mode) in self.dir_meta.drain(..).rev() {
            if self.opts.preserve_mode
                && let Some(m) = mode
            {
                set_mode(&path, m);
            }
            if self.opts.preserve_times
                && let Some(t) = mtime
            {
                let ft = filetime::FileTime::from_system_time(t);
                let _ = filetime::set_file_times(&path, ft, ft);
            }
        }
        self.summary
    }

    fn try_entry(
        &mut self,
        idx: usize,
        rel: &str,
        data: &mut dyn EntryData,
        consumed: &mut bool,
    ) -> Result<Outcome, Fail> {
        let entries = self.entries;
        let e = &entries[idx];
        let extra = self.extras.get(idx);
        let raw = extra.and_then(|x| x.raw.as_deref()).unwrap_or(&e.path);
        if path_escapes(raw) {
            return Err(Fail::Entry(ArchiveError::ZipSlip(raw.to_string()), false));
        }
        let rel = if rel.is_empty() {
            // A file whose path was fully stripped: keep its name.
            e.path.rsplit('/').next().unwrap_or_default().to_string()
        } else {
            rel.to_string()
        };
        for seg in rel.split('/') {
            if let Err(err) = validate_name_with(seg, cfg!(windows)) {
                return Err(Fail::Entry(
                    fs_kind(FsErrorKind::InvalidName, "extract", &self.dest.join(&rel), err.to_string()),
                    false,
                ));
            }
        }
        let dst = self.resolve(&rel);
        if self.sink.on_entry(e, &dst) == Control::Stop {
            return Err(Fail::Fatal(ArchiveError::Cancelled));
        }
        let parent = dst.parent().unwrap_or(&self.dest).to_path_buf();
        self.ensure_dir(&parent)?;

        if e.is_dir {
            return self.extract_dir(e, &rel, &dst);
        }
        if e.is_symlink {
            return self.extract_symlink(e, &rel, &dst, data, consumed);
        }
        if extra.is_some_and(|x| x.hardlink) {
            return self.extract_hardlink(e, &rel, &dst);
        }
        self.extract_file(e, &rel, &dst, data, consumed)
    }

    /// Destination for a relative path, honouring renamed parent directories.
    fn resolve(&self, rel: &str) -> PathBuf {
        if !self.dir_remap.is_empty() {
            let mut cut = rel.len();
            while let Some(pos) = rel[..cut].rfind('/') {
                cut = pos;
                if let Some(base) = self.dir_remap.get(&rel[..cut]) {
                    return push_rel(base.clone(), &rel[cut + 1..]);
                }
            }
        }
        push_rel(self.dest.clone(), rel)
    }

    /// Create `dir` (and its ancestors below `dest`), refusing to traverse symlinks.
    fn ensure_dir(&mut self, dir: &Path) -> Result<(), Fail> {
        if self.safe_dirs.contains(dir) {
            return Ok(());
        }
        let Ok(rel) = dir.strip_prefix(&self.dest) else {
            return Err(Fail::Entry(ArchiveError::ZipSlip(dir.display().to_string()), false));
        };
        let mut cur = self.dest.clone();
        for comp in rel.components() {
            cur.push(comp);
            if self.safe_dirs.contains(&cur) {
                continue;
            }
            match fs::symlink_metadata(&cur) {
                Ok(md) if md.file_type().is_symlink() => {
                    return Err(Fail::Entry(
                        ArchiveError::ZipSlip(format!("{} (path goes through a symlink)", cur.display())),
                        false,
                    ));
                }
                Ok(md) if md.is_dir() => {}
                Ok(_) => {
                    return Err(Fail::Entry(
                        fs_kind(FsErrorKind::NotDirectory, "extract", &cur, "a file is in the way"),
                        false,
                    ));
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir(&cur).map_err(|err| Fail::Entry(fs_err("create_dir", &cur, &err), false))?;
                }
                Err(err) => return Err(Fail::Entry(fs_err("extract", &cur, &err), false)),
            }
            self.safe_dirs.insert(cur.clone());
        }
        Ok(())
    }

    fn extract_dir(&mut self, e: &ArchiveEntry, rel: &str, dst: &Path) -> Result<Outcome, Fail> {
        let path = match fs::symlink_metadata(dst) {
            Ok(md) if md.is_dir() && !md.file_type().is_symlink() => dst.to_path_buf(),
            Ok(_) => match self.resolve_conflict(dst, e, true)? {
                Target::Skip => return Ok(Outcome::Skipped),
                Target::Merge(p) => p,
                Target::Write { path, replace } => {
                    if replace {
                        remove_existing(&path).map_err(|err| Fail::Entry(err, false))?;
                    }
                    fs::create_dir(&path).map_err(|err| Fail::Entry(fs_err("create_dir", &path, &err), false))?;
                    path
                }
            },
            Err(_) => {
                fs::create_dir(dst).map_err(|err| Fail::Entry(fs_err("create_dir", dst, &err), false))?;
                dst.to_path_buf()
            }
        };
        if path != dst {
            self.dir_remap.insert(rel.to_string(), path.clone());
        }
        self.safe_dirs.insert(path.clone());
        self.dir_meta.push((path, e.modified, e.mode));
        Ok(Outcome::Done(0))
    }

    fn extract_file(
        &mut self,
        e: &ArchiveEntry,
        rel: &str,
        dst: &Path,
        data: &mut dyn EntryData,
        consumed: &mut bool,
    ) -> Result<Outcome, Fail> {
        let (path, replace) = match self.target_for_file(e, dst)? {
            Some(t) => t,
            None => return Ok(Outcome::Skipped),
        };
        let dir = path.parent().unwrap_or(&self.dest).to_path_buf();
        self.tmp_counter += 1;
        let tmp = dir.join(format!(".soyuz-extract-{}-{}.part", std::process::id(), self.tmp_counter));
        let result = self.write_tmp(e, &tmp, data, consumed);
        let bytes = match result {
            Ok(n) => n,
            Err(f) => {
                let _ = fs::remove_file(&tmp);
                return Err(f);
            }
        };
        if replace
            && let Ok(md) = fs::symlink_metadata(&path)
            && (md.is_dir() || md.file_type().is_symlink())
            && let Err(err) = remove_existing(&path)
        {
            let _ = fs::remove_file(&tmp);
            return Err(Fail::Entry(err, true));
        }
        if let Err(err) = rename_into_place(&tmp, &path, replace) {
            let _ = fs::remove_file(&tmp);
            return Err(Fail::Entry(err, true));
        }
        if self.opts.preserve_mode
            && let Some(m) = e.mode
        {
            set_mode(&path, m);
        }
        if self.opts.preserve_times
            && let Some(t) = e.modified
        {
            let ft = filetime::FileTime::from_system_time(t);
            let _ = filetime::set_file_times(&path, ft, ft);
        }
        self.written.insert(rel.to_string(), path);
        Ok(Outcome::Done(bytes))
    }

    fn write_tmp(
        &mut self,
        e: &ArchiveEntry,
        tmp: &Path,
        data: &mut dyn EntryData,
        consumed: &mut bool,
    ) -> Result<u64, Fail> {
        let mut out = File::create(tmp).map_err(|err| Fail::Entry(fs_err("extract", tmp, &err), false))?;
        *consumed = true;
        let mut reader = data.open().map_err(|err| classify(err, true))?;
        let mut total = 0u64;
        loop {
            self.ctl.checkpoint().map_err(|err| Fail::Fatal(err.into()))?;
            let n = match reader.read(&mut self.buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(err) if err.kind() == io::ErrorKind::Interrupted && !self.ctl.is_cancelled() => continue,
                Err(err) => return Err(classify(read_err(err, Some(self.ctl), e.encrypted), true)),
            };
            out.write_all(&self.buf[..n]).map_err(|err| Fail::Entry(fs_err("write", tmp, &err), true))?;
            total += n as u64;
            self.sink.on_bytes(n as u64);
        }
        drop(reader);
        if total != e.size {
            // Some decoders end early on garbage (e.g. a wrong 7z password) without an error.
            let err = if e.encrypted {
                ArchiveError::WrongPassword
            } else {
                ArchiveError::Corrupt(format!("{}: got {total} bytes, expected {}", e.path, e.size))
            };
            return Err(classify(err, true));
        }
        out.flush().map_err(|err| Fail::Entry(fs_err("write", tmp, &err), true))?;
        Ok(total)
    }

    /// Conflict handling for a file-like target. `None` = skip.
    fn target_for_file(&mut self, e: &ArchiveEntry, dst: &Path) -> Result<Option<(PathBuf, bool)>, Fail> {
        if fs::symlink_metadata(dst).is_err() {
            return Ok(Some((dst.to_path_buf(), false)));
        }
        match self.resolve_conflict(dst, e, false)? {
            Target::Skip => Ok(None),
            Target::Write { path, replace } => Ok(Some((path, replace))),
            Target::Merge(p) => Ok(Some((p, true))),
        }
    }

    fn resolve_conflict(&mut self, dst: &Path, e: &ArchiveEntry, entry_is_dir: bool) -> Result<Target, Fail> {
        let existing = fs::symlink_metadata(dst).ok();
        let existing_is_dir = existing.as_ref().is_some_and(|m| m.is_dir() && !m.file_type().is_symlink());
        let newer = || match (e.modified, existing.as_ref().and_then(|m| m.modified().ok())) {
            (Some(a), Some(b)) => a > b,
            (Some(_), None) => true,
            _ => false,
        };
        let larger = || e.size > existing.as_ref().map_or(0, |m| m.len());
        let different = || e.size != existing.as_ref().map_or(u64::MAX, |m| m.len());
        let cond = |ok: bool| if ok { Target::Write { path: dst.to_path_buf(), replace: true } } else { Target::Skip };
        match self.opts.overwrite {
            Overwrite::Always => return Ok(cond(true)),
            Overwrite::IfNewer => return Ok(cond(newer())),
            Overwrite::IfLarger => return Ok(cond(larger())),
            Overwrite::IfDifferentSize => return Ok(cond(different())),
            Overwrite::Never => {}
        }
        let mut current = dst.to_path_buf();
        for _ in 0..16 {
            let answer = self.sink.on_conflict(&current, e);
            let t = match answer {
                ConflictResolution::Overwrite => Target::Write { path: current.clone(), replace: true },
                ConflictResolution::OverwriteIfNewer => cond(newer()),
                ConflictResolution::OverwriteIfLarger => cond(larger()),
                ConflictResolution::OverwriteIfDifferent => cond(different()),
                ConflictResolution::Skip => Target::Skip,
                ConflictResolution::Cancel => return Err(Fail::Fatal(ArchiveError::Conflict(current))),
                ConflictResolution::MergeDir if entry_is_dir && existing_is_dir => Target::Merge(current.clone()),
                ConflictResolution::MergeDir => Target::Write { path: current.clone(), replace: true },
                ConflictResolution::KeepBoth => {
                    let dir = current.parent().unwrap_or(&self.dest);
                    let name = current.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let p = unique_name(dir, &name, "", &|p: &Path| fs::symlink_metadata(p).is_ok());
                    Target::Write { path: p, replace: false }
                }
                ConflictResolution::Rename(name) => {
                    if validate_name_with(&name, cfg!(windows)).is_err() || name.contains(['/', '\\']) {
                        return Err(Fail::Entry(
                            fs_kind(FsErrorKind::InvalidName, "extract", &current, format!("invalid name {name:?}")),
                            false,
                        ));
                    }
                    let p = current.parent().unwrap_or(&self.dest).join(&name);
                    if fs::symlink_metadata(&p).is_ok() {
                        current = p;
                        continue;
                    }
                    Target::Write { path: p, replace: false }
                }
            };
            return Ok(t);
        }
        Err(Fail::Entry(fs_kind(FsErrorKind::AlreadyExists, "extract", &current, "unresolved conflict"), false))
    }

    fn extract_symlink(
        &mut self,
        e: &ArchiveEntry,
        rel: &str,
        dst: &Path,
        data: &mut dyn EntryData,
        consumed: &mut bool,
    ) -> Result<Outcome, Fail> {
        let target = match &e.link_target {
            Some(t) => t.clone(),
            None => {
                *consumed = true;
                let mut r = data.open().map_err(|err| classify(err, true))?;
                let mut s = Vec::new();
                r.by_ref()
                    .take(64 * 1024)
                    .read_to_end(&mut s)
                    .map_err(|err| classify(read_err(err, Some(self.ctl), e.encrypted), true))?;
                String::from_utf8_lossy(&s).into_owned()
            }
        };
        // Reject targets that pass through another link: one from this archive (extracted
        // before or after this one) or one already on disk under the destination.
        let (entries, plan) = (self.entries, &self.plan);
        let link_rels = self.link_rels.get_or_insert_with(|| {
            entries
                .iter()
                .zip(plan)
                .filter(|(x, _)| x.is_symlink)
                .filter_map(|(_, p)| p.as_ref().map(|p| p.to_lowercase()))
                .collect()
        });
        let dest = &self.dest;
        let mut is_link = |p: &str| {
            link_rels.contains(&p.to_lowercase())
                || fs::symlink_metadata(push_rel(dest.clone(), p)).is_ok_and(|m| m.file_type().is_symlink())
        };
        let Some(resolved) = resolve_link_target_via(rel, &target, &mut is_link) else {
            return Err(Fail::Entry(ArchiveError::ZipSlip(format!("{} -> {target}", e.path)), *consumed));
        };
        let (path, replace) = match self.target_for_file(e, dst)? {
            Some(t) => t,
            None => return Ok(Outcome::Skipped),
        };
        if replace {
            remove_existing(&path).map_err(|err| Fail::Entry(err, *consumed))?;
        }
        let target_is_dir = self.entries.iter().any(|x| x.is_dir && x.path == resolved)
            || fs::metadata(push_rel(self.dest.clone(), &resolved)).is_ok_and(|m| m.is_dir());
        make_symlink(&target, &path, target_is_dir).map_err(|err| Fail::Entry(fs_err("symlink", &path, &err), true))?;
        if self.opts.preserve_times
            && let Some(t) = e.modified
        {
            let ft = filetime::FileTime::from_system_time(t);
            let _ = filetime::set_symlink_file_times(&path, ft, ft);
        }
        Ok(Outcome::Done(0))
    }

    fn extract_hardlink(&mut self, e: &ArchiveEntry, rel: &str, dst: &Path) -> Result<Outcome, Fail> {
        let target = e.link_target.clone().unwrap_or_default();
        if path_escapes(&target) {
            return Err(Fail::Entry(ArchiveError::ZipSlip(format!("{} => {target}", e.path)), false));
        }
        let norm = soyuz_core::location::normalize_inner(&target);
        // The target was written under its own (stripped) relative path.
        let entries = self.entries;
        let by_path =
            self.by_path.get_or_insert_with(|| entries.iter().enumerate().map(|(i, x)| (x.path.as_str(), i)).collect());
        let src = by_path
            .get(norm.as_str())
            .copied()
            .and_then(|i| self.plan[i].clone())
            .and_then(|r| self.written.get(&r).cloned());
        let Some(src) = src else {
            return Err(Fail::Entry(
                fs_kind(
                    FsErrorKind::NotFound,
                    "extract",
                    dst,
                    format!("hard link target {target:?} was not extracted"),
                ),
                false,
            ));
        };
        let (path, replace) = match self.target_for_file(e, dst)? {
            Some(t) => t,
            None => return Ok(Outcome::Skipped),
        };
        if replace {
            remove_existing(&path).map_err(|err| Fail::Entry(err, false))?;
        }
        let bytes = match fs::hard_link(&src, &path) {
            Ok(()) => 0,
            Err(_) => fs::copy(&src, &path).map_err(|err| Fail::Entry(fs_err("copy", &path, &err), false))?,
        };
        self.written.insert(rel.to_string(), path);
        Ok(Outcome::Done(bytes))
    }
}

fn push_rel(mut base: PathBuf, rel: &str) -> PathBuf {
    for seg in rel.split('/').filter(|s| !s.is_empty()) {
        base.push(seg);
    }
    base
}

/// Mark wanted entries: the selection plus everything below selected directories.
fn expand_selection(entries: &[ArchiveEntry], selection: Option<&[usize]>) -> Vec<bool> {
    let Some(sel) = selection else { return vec![true; entries.len()] };
    let mut wanted = vec![false; entries.len()];
    let mut prefixes: Vec<String> = Vec::new();
    for &i in sel {
        if let Some(e) = entries.get(i) {
            wanted[i] = true;
            if e.is_dir {
                prefixes.push(format!("{}/", e.path));
            }
        }
    }
    if !prefixes.is_empty() {
        for (i, e) in entries.iter().enumerate() {
            if !wanted[i] && prefixes.iter().any(|p| e.path.starts_with(p.as_str())) {
                wanted[i] = true;
            }
        }
    }
    wanted
}

/// All wanted entries live under one top-level directory (after `strip` components).
fn single_top_dir(entries: &[ArchiveEntry], wanted: &[bool], strip: usize) -> bool {
    let mut top: Option<&str> = None;
    let mut any = false;
    for (e, _) in entries.iter().zip(wanted).filter(|(_, w)| **w) {
        let segs: Vec<&str> = e.path.split('/').filter(|s| !s.is_empty()).collect();
        if segs.len() <= strip {
            continue;
        }
        let first = segs[strip];
        // A file directly at the (stripped) top level means there is no wrapper folder.
        if segs.len() == strip + 1 && !e.is_dir {
            return false;
        }
        match top {
            None => top = Some(first),
            Some(t) if t == first => {}
            Some(_) => return false,
        }
        any = true;
    }
    any
}

fn remove_existing(path: &Path) -> Result<(), ArchiveError> {
    let md = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return Ok(()),
    };
    let r = if md.file_type().is_symlink() {
        fs::remove_file(path).or_else(|_| fs::remove_dir(path))
    } else if md.is_dir() {
        // Only empty directories are replaced; never delete user data recursively.
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    };
    r.map_err(|e| fs_err("remove", path, &e))
}

fn rename_into_place(tmp: &Path, path: &Path, replace: bool) -> Result<(), ArchiveError> {
    if !replace && fs::symlink_metadata(path).is_ok() {
        return Err(fs_kind(FsErrorKind::AlreadyExists, "extract", path, "destination appeared during extraction"));
    }
    match fs::rename(tmp, path) {
        Ok(()) => Ok(()),
        Err(e) if replace && cfg!(windows) => {
            // Read-only targets refuse replacement on Windows: clear the flag and retry once.
            if let Ok(md) = fs::metadata(path) {
                let mut p = md.permissions();
                if p.readonly() {
                    #[allow(clippy::permissions_set_readonly_false)]
                    p.set_readonly(false);
                    let _ = fs::set_permissions(path, p);
                    return fs::rename(tmp, path).map_err(|e| fs_err("rename", path, &e));
                }
            }
            Err(fs_err("rename", path, &e))
        }
        Err(e) => Err(fs_err("rename", path, &e)),
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o7777));
}

#[cfg(not(unix))]
fn set_mode(path: &Path, mode: u32) {
    // Windows: honour only "no write bit at all" as read-only, for files.
    if mode & 0o222 == 0
        && let Ok(md) = fs::metadata(path)
        && md.is_file()
    {
        let mut p = md.permissions();
        p.set_readonly(true);
        let _ = fs::set_permissions(path, p);
    }
}

#[cfg(unix)]
fn make_symlink(target: &str, link: &Path, _is_dir: bool) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn make_symlink(target: &str, link: &Path, is_dir: bool) -> io::Result<()> {
    let t = target.replace('/', "\\");
    if is_dir { std::os::windows::fs::symlink_dir(t, link) } else { std::os::windows::fs::symlink_file(t, link) }
}

#[cfg(not(any(unix, windows)))]
fn make_symlink(_target: &str, _link: &Path, _is_dir: bool) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "symlinks are not supported on this platform"))
}
