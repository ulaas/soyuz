//! `extract` job executor (drives `soyuz_archive`; conflicts / errors / passwords go
//! through the job's prompt protocol).

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use soyuz_archive::{ArchiveEntry, ArchiveError, ExtractOptions, ExtractSink};
use soyuz_core::{ConflictResolution, Control, EntryInfo, EntryKind, ErrorAction, FsError, FsErrorKind, FsResult};

use super::copy;
use crate::ctx::JobCtx;
use crate::prompt::{ConflictInfo, JobPrompt, PromptResponse};
use crate::undo::UndoRecord;

/// Map an archive error to the job error vocabulary.
pub(crate) fn fs_error(e: &ArchiveError, path: &Path) -> FsError {
    match e {
        ArchiveError::Io(fe) => fe.clone().at(path),
        ArchiveError::Unsupported(f) => {
            FsError::with_message(FsErrorKind::Unsupported, "archive", Some(path), format!("{f:?} is not supported"))
        }
        ArchiveError::Corrupt(m) => FsError::with_message(FsErrorKind::Corrupt, "archive", Some(path), m.clone()),
        ArchiveError::PasswordRequired => FsError::new(FsErrorKind::PasswordRequired, "archive", Some(path)),
        ArchiveError::WrongPassword => FsError::new(FsErrorKind::WrongPassword, "archive", Some(path)),
        ArchiveError::ZipSlip(m) => FsError::with_message(
            FsErrorKind::InvalidName,
            "archive",
            Some(path),
            format!("entry escapes the destination: {m}"),
        ),
        ArchiveError::Cancelled => FsError::interrupted("archive"),
        ArchiveError::Conflict(p) => FsError::new(FsErrorKind::AlreadyExists, "archive", Some(p)),
    }
}

struct Sink<'a> {
    ctx: &'a JobCtx,
    archive: &'a Path,
    dest: &'a Path,
    tops: HashSet<PathBuf>,
}

impl Sink<'_> {
    /// First path component below `dest` (the unit recorded for undo).
    fn top_of(&self, dst: &Path) -> Option<PathBuf> {
        let rel = dst.strip_prefix(self.dest).ok()?;
        match rel.components().next()? {
            Component::Normal(n) => Some(self.dest.join(n)),
            _ => None,
        }
    }
}

impl ExtractSink for Sink<'_> {
    fn on_entry(&mut self, e: &ArchiveEntry, dst: &Path) -> Control {
        if self.ctx.checkpoint().is_err() {
            return Control::Stop;
        }
        if let Some(top) = self.top_of(dst)
            && !self.tops.contains(&top)
        {
            if !soyuz_fs::exists_nofollow(&top) {
                self.ctx.record(UndoRecord::Extracted { dst: top.clone() });
            }
            self.tops.insert(top);
        }
        self.ctx.begin_item(dst, e.size);
        self.ctx.item_ok(0);
        Control::Continue
    }

    fn on_bytes(&mut self, n: u64) {
        self.ctx.on_bytes(soyuz_core::ByteProgress { done: 0, total: 0, delta: n });
    }

    fn on_conflict(&mut self, dst: &Path, e: &ArchiveEntry) -> ConflictResolution {
        let kind = if e.is_dir { EntryKind::Dir } else { EntryKind::File };
        let mut src = EntryInfo::minimal(soyuz_core::join_vpath(self.archive, &e.path), kind);
        src.size = e.size;
        src.modified = e.modified;
        let info: ConflictInfo = copy::conflict_info(&src, dst);
        self.ctx.resolve_conflict(info).unwrap_or(ConflictResolution::Cancel)
    }

    fn on_error(&mut self, e: &ArchiveEntry, err: &ArchiveError) -> ErrorAction {
        let path = soyuz_core::join_vpath(self.archive, &e.path);
        let fe = fs_error(err, &path);
        match self.ctx.on_error(&path, &fe) {
            Ok(ErrorAction::Retry) => ErrorAction::Retry,
            Ok(_) => {
                self.ctx.item_failed(&path, fe, 0);
                ErrorAction::Skip
            }
            Err(_) => ErrorAction::Abort,
        }
    }
}

pub(crate) fn run_extract(
    ctx: &JobCtx,
    archive: &Path,
    entries: Option<&[usize]>,
    dest_dir: &Path,
    password: Option<&str>,
    strip_components: usize,
) -> FsResult<()> {
    let mut password = password.map(str::to_string);
    let mut wrong = false;
    let mut reader = loop {
        ctx.checkpoint()?;
        let opened = soyuz_archive::open(archive, password.as_deref()).and_then(|mut r| {
            r.entries()?;
            Ok(r)
        });
        match opened {
            Ok(r) => break r,
            Err(ArchiveError::PasswordRequired) | Err(ArchiveError::WrongPassword) => {
                match ctx.prompt(JobPrompt::Password { archive: archive.to_path_buf(), wrong }) {
                    PromptResponse::Password(Some(p)) => {
                        password = Some(p);
                        wrong = true;
                    }
                    _ => return Err(ctx.cancel_now()),
                }
            }
            Err(e) => return Err(fs_error(&e, archive)),
        }
    };
    let (bytes, items) = {
        let all = reader.entries().map_err(|e| fs_error(&e, archive))?;
        let picked: Vec<&ArchiveEntry> = match entries {
            Some(sel) => all.iter().filter(|e| sel.contains(&e.index)).collect(),
            None => all.iter().collect(),
        };
        (picked.iter().map(|e| e.size).sum::<u64>(), picked.len() as u64)
    };
    ctx.set_totals(bytes, items);
    ctx.check_space(bytes, dest_dir)?;
    ctx.set_running();
    if !copy::ensure_dir(ctx, dest_dir)? {
        return Ok(());
    }
    let opts = ExtractOptions {
        strip_components,
        overwrite: soyuz_core::Overwrite::Never,
        preserve_mode: true,
        preserve_times: true,
        single_top_folder: false,
    };
    let mut sink = Sink { ctx, archive, dest: dest_dir, tops: HashSet::new() };
    match reader.extract(entries, dest_dir, &opts, ctx.ctl(), &mut sink) {
        Ok(s) => {
            if s.entries_skipped > 0 {
                ctx.note(format!("{} entries skipped", s.entries_skipped));
            }
            Ok(())
        }
        Err(ArchiveError::Cancelled) => Err(FsError::interrupted("extract")),
        Err(e) => Err(fs_error(&e, archive)),
    }
}
