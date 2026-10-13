//! Split and join files (Phase T7, agent A), moved from the GUI's `ui/tools/split.rs`:
//! the pure split / join / checksum functions and the [`SplitModel`] both frontends draw
//! (the GUI's `SplitWindow`, the terminal UI's `TuiDialog::Split`).
//!
//! Splitting writes `name.ext.001`, `.002`, … plus a b3sum-compatible checksum file
//! `name.ext.blake3` (`<hash>  name.ext`); joining finds the consecutive parts,
//! concatenates them into a temporary file, verifies against the checksum file when
//! present and renames the result into place. Both run on a worker thread with byte
//! progress and cancellation; failures and cancels remove partial output, and nothing is
//! ever overwritten. The disk checks a dialog needs before it can start (the file's size,
//! the parts and their checksum file, the parts' total size) run on workers too, never on
//! the UI thread.
//!
//! Splitting is not a jobs-engine operation (no pause, no history, no undo); it never
//! overwrites and cleans up on cancel.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crossbeam_channel::{Receiver, bounded};
use soyuz_core::{OpControl, SizeStyle, format_size};

use super::toasts::Toast;
pub use super::tools::{is_part, split_part};
use crate::ops::fileops::names::show_path;

const BUF: usize = 1024 * 1024;

/// `file.ext` + part number → `file.ext.001`.
pub fn part_path(base: &Path, n: usize) -> PathBuf {
    let mut s = base.as_os_str().to_os_string();
    s.push(format!(".{n:03}"));
    PathBuf::from(s)
}

/// Checksum file next to the original: `file.ext.blake3`.
pub fn checksum_path(base: &Path) -> PathBuf {
    let mut s = base.as_os_str().to_os_string();
    s.push(".blake3");
    PathBuf::from(s)
}

/// All consecutive parts starting at `.001` for the part `any`. Blocking (stats).
pub fn find_parts(any: &Path) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let (base, _) = split_part(any).ok_or_else(|| tr!("tools-split-err-not-part").to_string())?;
    let mut parts = Vec::new();
    for n in 1.. {
        let p = part_path(&base, n);
        if !p.is_file() {
            break;
        }
        parts.push(p);
    }
    if parts.is_empty() {
        return Err(tr!("tools-split-err-not-found", path = part_path(&base, 1).display().to_string()));
    }
    Ok((base, parts))
}

#[derive(Clone, Debug, PartialEq)]
pub struct SplitOutcome {
    pub parts: Vec<PathBuf>,
    pub checksum: PathBuf,
    pub hash: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct JoinOutcome {
    pub output: PathBuf,
    pub bytes: u64,
    /// `Some(true)` verified, `Some(false)` mismatch (the output is kept as `.bad`), `None` no checksum file.
    pub verified: Option<bool>,
}

/// File operation named in an error message.
#[derive(Clone, Copy)]
enum IoOp {
    Open,
    Create,
    Read,
    Write,
}

fn io_err(op: IoOp, p: &Path, e: std::io::Error) -> String {
    let path = soyuz_fs::strip_verbatim(p).display().to_string();
    let error = e.to_string();
    match op {
        IoOp::Open => tr!("tools-split-err-open", path = path, error = error),
        IoOp::Create => tr!("tools-split-err-create", path = path, error = error),
        IoOp::Read => tr!("tools-split-err-read", path = path, error = error),
        IoOp::Write => tr!("tools-split-err-write", path = path, error = error),
    }
}

fn exists_err(p: &Path) -> String {
    tr!("tools-split-err-exists", path = soyuz_fs::strip_verbatim(p).display().to_string())
}

/// Split `src` into parts of `part_size` bytes in `out_dir`. Blocking.
pub fn split_file(
    src: &Path,
    out_dir: &Path,
    part_size: u64,
    ctl: &OpControl,
    progress: &AtomicU64,
) -> Result<SplitOutcome, String> {
    if part_size == 0 {
        return Err(tr!("tools-split-err-zero-size").into());
    }
    let name = src.file_name().ok_or_else(|| tr!("tools-split-err-no-name").to_string())?;
    let base = out_dir.join(name);
    let mut input = File::open(src).map_err(|e| io_err(IoOp::Open, src, e))?;
    let total = input.metadata().map(|m| m.len()).unwrap_or(0);
    let n_parts = total.div_ceil(part_size).max(1) as usize;
    let ck = checksum_path(&base);
    for p in (1..=n_parts).map(|n| part_path(&base, n)).chain(std::iter::once(ck.clone())) {
        if soyuz_fs::exists_nofollow(&p) {
            return Err(exists_err(&p));
        }
    }
    let mut written: Vec<PathBuf> = Vec::new();
    let result = (|| -> Result<String, String> {
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; BUF];
        for n in 1..=n_parts {
            let p = part_path(&base, n);
            let f = File::create_new(&p).map_err(|e| io_err(IoOp::Create, &p, e))?;
            written.push(p.clone());
            let mut w = BufWriter::with_capacity(BUF, f);
            let mut left = part_size;
            while left > 0 {
                if ctl.is_cancelled() {
                    return Err(tr!("tools-cancelled").into());
                }
                let want = left.min(BUF as u64) as usize;
                let got = input.read(&mut buf[..want]).map_err(|e| io_err(IoOp::Read, src, e))?;
                if got == 0 {
                    break;
                }
                w.write_all(&buf[..got]).map_err(|e| io_err(IoOp::Write, &p, e))?;
                hasher.update(&buf[..got]);
                left -= got as u64;
                progress.fetch_add(got as u64, Ordering::Relaxed);
            }
            w.flush().map_err(|e| io_err(IoOp::Write, &p, e))?;
        }
        Ok(hasher.finalize().to_hex().to_string())
    })();
    match result {
        Ok(hash) => {
            // Never replaces a checksum file that appeared meanwhile.
            let line = format!("{hash}  {}\n", name.to_string_lossy());
            File::create_new(&ck)
                .and_then(|mut f| f.write_all(line.as_bytes()).and_then(|_| f.sync_all()))
                .map_err(|e| io_err(IoOp::Create, &ck, e))?;
            Ok(SplitOutcome { parts: written, checksum: ck, hash })
        }
        Err(e) => {
            for p in written {
                let _ = std::fs::remove_file(p);
            }
            Err(e)
        }
    }
}

/// Parse a b3sum line (`<hex>  name`), returning the hash.
pub fn parse_checksum(text: &str) -> Option<String> {
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    let hash = line.split_whitespace().next()?;
    (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hash.to_ascii_lowercase())
}

/// Join `parts` into `output` (must not exist). Verifies against `checksum` when given.
pub fn join_parts(
    parts: &[PathBuf],
    output: &Path,
    checksum: Option<&Path>,
    ctl: &OpControl,
    progress: &AtomicU64,
) -> Result<JoinOutcome, String> {
    if output.exists() {
        return Err(exists_err(output));
    }
    let expected = checksum.and_then(|c| std::fs::read_to_string(c).ok()).and_then(|t| parse_checksum(&t));
    let mut tmp = output.as_os_str().to_os_string();
    tmp.push(".soyuz-join.part");
    let tmp = PathBuf::from(tmp);
    let result = (|| -> Result<(u64, String), String> {
        let f = File::create_new(&tmp).map_err(|e| io_err(IoOp::Create, &tmp, e))?;
        let mut w = BufWriter::with_capacity(BUF, f);
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; BUF];
        let mut bytes = 0u64;
        for p in parts {
            let mut input = File::open(p).map_err(|e| io_err(IoOp::Open, p, e))?;
            loop {
                if ctl.is_cancelled() {
                    return Err(tr!("tools-cancelled").into());
                }
                let got = input.read(&mut buf).map_err(|e| io_err(IoOp::Read, p, e))?;
                if got == 0 {
                    break;
                }
                w.write_all(&buf[..got]).map_err(|e| io_err(IoOp::Write, &tmp, e))?;
                hasher.update(&buf[..got]);
                bytes += got as u64;
                progress.fetch_add(got as u64, Ordering::Relaxed);
            }
        }
        w.flush().map_err(|e| io_err(IoOp::Write, &tmp, e))?;
        Ok((bytes, hasher.finalize().to_hex().to_string()))
    })();
    match result {
        Ok((bytes, hash)) => {
            let verified = expected.map(|e| e == hash);
            let target = if verified == Some(false) {
                let mut s = output.as_os_str().to_os_string();
                s.push(".bad");
                PathBuf::from(s)
            } else {
                output.to_path_buf()
            };
            // No-replace: something that appeared at the target meanwhile is never overwritten.
            if let Err(e) = soyuz_fs::rename(&tmp, &target, soyuz_fs::RenameMode::NoReplace) {
                let _ = std::fs::remove_file(&tmp);
                return Err(tr!(
                    "tools-split-err-create",
                    path = soyuz_fs::strip_verbatim(&target).display().to_string(),
                    error = e.to_string()
                ));
            }
            Ok(JoinOutcome { output: target, bytes, verified })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------------------
// The model both frontends draw.

/// Split one file, or join the parts of one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Split,
    Join,
}

/// Part size presets (label, bytes); 0 bytes is "Custom" ([`preset_label`]).
pub const PRESETS: [(&str, u64); 6] = [
    ("10 MB", 10_000_000),
    ("100 MB", 100_000_000),
    ("700 MB (CD)", 700 * 1024 * 1024),
    ("1 GB", 1_000_000_000),
    ("4 GB − 1 (FAT32)", 4 * 1024 * 1024 * 1024 - 1),
    ("", 0),
];

/// The preset picked when a split dialog opens (100 MB).
pub const DEFAULT_PRESET: usize = 1;

/// The "Custom" preset.
pub const CUSTOM: usize = PRESETS.len() - 1;

/// The label of preset `i` ("Custom" translated).
pub fn preset_label(i: usize) -> &'static str {
    match PRESETS.get(i) {
        Some(("", _)) | None => tr!("tools-split-custom"),
        Some((name, _)) => name,
    }
}

enum Outcome {
    Split(SplitOutcome),
    Join(JoinOutcome),
}

/// What the probe worker found before the dialog can start.
enum Probe {
    /// Split: the file's size (`None`: it could not be read).
    Size(Option<u64>),
    /// Join: the parts and whether a checksum file sits next to them.
    Parts(Result<(PathBuf, Vec<PathBuf>), String>, bool),
}

/// The blocking split / join call run on the worker.
type Work = Box<dyn FnOnce(&OpControl, &AtomicU64) -> Result<Outcome, String> + Send>;

struct Task {
    rx: Receiver<Result<Outcome, String>>,
    ctl: Arc<OpControl>,
    progress: Arc<AtomicU64>,
    /// Bytes to copy (a join's total arrives from the worker).
    total: Arc<AtomicU64>,
    started: Instant,
}

/// A split or join dialog's state: the file, the part size, the output folder, the worker
/// and its result. The frontends draw it and call [`SplitModel::poll`] every frame.
pub struct SplitModel {
    pub mode: Mode,
    pub path: PathBuf,
    /// The output folder as typed.
    pub out_dir: String,
    /// The part size preset ([`PRESETS`]).
    pub preset: usize,
    /// The custom part size as typed ("250 MB").
    pub custom: String,
    /// Split: the file's size (`None` while probing or unreadable).
    pub size: Option<u64>,
    /// Join: the parts (`None` while probing).
    pub parts: Option<Result<(PathBuf, Vec<PathBuf>), String>>,
    /// A `.blake3` checksum file sits next to the parts (checked once, by the probe).
    pub has_checksum: bool,
    /// The last run's result: the message shown, `Err` for a failure or a mismatch.
    pub result: Option<Result<String, String>>,
    probe: Option<Receiver<Probe>>,
    task: Option<Task>,
}

impl std::fmt::Debug for SplitModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SplitModel")
            .field("mode", &self.mode)
            .field("path", &self.path)
            .field("out_dir", &self.out_dir)
            .field("preset", &self.preset)
            .field("running", &self.task.is_some())
            .finish_non_exhaustive()
    }
}

impl SplitModel {
    fn new(mode: Mode, path: PathBuf, custom: &str) -> SplitModel {
        let out_dir = path.parent().map(show_path).unwrap_or_default();
        SplitModel {
            mode,
            path,
            out_dir,
            preset: DEFAULT_PRESET,
            custom: custom.to_string(),
            size: None,
            parts: None,
            has_checksum: false,
            result: None,
            probe: None,
            task: None,
        }
    }

    /// Split `path`: its size is read on a worker ([`Self::probing`] until it arrives).
    pub fn split(path: PathBuf) -> SplitModel {
        let mut m = SplitModel::new(Mode::Split, path, "250 MB");
        let p = m.path.clone();
        m.probe = spawn_probe(move || Probe::Size(std::fs::metadata(&p).ok().map(|m| m.len())));
        m
    }

    /// Join the parts of `path` (any part): they are found on a worker.
    pub fn join(path: PathBuf) -> SplitModel {
        let mut m = SplitModel::new(Mode::Join, path, "");
        let p = m.path.clone();
        m.probe = spawn_probe(move || {
            let parts = find_parts(&p);
            let has = matches!(&parts, Ok((base, _)) if checksum_path(base).is_file());
            Probe::Parts(parts, has)
        });
        m
    }

    /// The disk checks before a start have not finished yet.
    pub fn probing(&self) -> bool {
        self.probe.is_some()
    }

    /// A split / join is running.
    pub fn running(&self) -> bool {
        self.task.is_some()
    }

    /// The last run succeeded (the dialog offers "Done").
    pub fn finished_ok(&self) -> bool {
        matches!(self.result, Some(Ok(_)))
    }

    /// The part size picked (`None`: a custom size that does not parse or is zero).
    pub fn part_size(&self) -> Option<u64> {
        match PRESETS.get(self.preset).map_or(0, |p| p.1) {
            0 => crate::services::search::parse_size(&self.custom).filter(|s| *s > 0),
            s => Some(s),
        }
    }

    /// Split: the part count and size the file will be cut into.
    pub fn plan(&self) -> Option<(u64, u64)> {
        let (total, ps) = (self.size?, self.part_size()?);
        Some((total.div_ceil(ps).max(1), ps))
    }

    /// The plan as a line ("43 parts of 95.4 MiB (+ a .blake3 checksum file)").
    pub fn plan_text(&self) -> Option<String> {
        let (n, ps) = self.plan()?;
        Some(tr!("tools-split-plan", count = n, size = format_size(ps, SizeStyle::Binary)))
    }

    /// Start is possible: the probe found the file / the parts, and there is an output
    /// folder (the custom size is checked by [`Self::start`]).
    pub fn can_start(&self) -> bool {
        let ok = match self.mode {
            Mode::Split => self.size.is_some(),
            Mode::Join => matches!(self.parts, Some(Ok(_))),
        };
        ok && !self.running() && !self.out_dir.trim().is_empty()
    }

    /// Start splitting / joining on a worker (a bad custom size becomes the result).
    pub fn start(&mut self) {
        if !self.can_start() {
            return;
        }
        let ctl = OpControl::new();
        let progress = Arc::new(AtomicU64::new(0));
        let total = Arc::new(AtomicU64::new(0));
        let (tx, rx) = bounded(1);
        let wake = super::tools::wake();
        let out_dir = PathBuf::from(self.out_dir.trim());
        let job: Work = match self.mode {
            Mode::Split => {
                let Some(size) = self.part_size() else {
                    self.result = Some(Err(tr!("tools-split-err-invalid-size").into()));
                    return;
                };
                total.store(self.size.unwrap_or(0), Ordering::Relaxed);
                let src = self.path.clone();
                Box::new(move |ctl, pr| split_file(&src, &out_dir, size, ctl, pr).map(Outcome::Split))
            }
            Mode::Join => {
                let Some(Ok((base, parts))) = self.parts.clone() else { return };
                let name = base.file_name().map(|n| n.to_os_string()).unwrap_or_default();
                let output = out_dir.join(name);
                let ck = checksum_path(&base);
                let sum = total.clone();
                Box::new(move |ctl, pr| {
                    // The parts' total size (progress), off the UI thread.
                    let bytes = parts.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
                    sum.store(bytes, Ordering::Relaxed);
                    let ck = ck.is_file().then_some(ck.as_path());
                    join_parts(&parts, &output, ck, ctl, pr).map(Outcome::Join)
                })
            }
        };
        let (c2, p2) = (ctl.clone(), progress.clone());
        let spawned = std::thread::Builder::new().name("soyuz-split".into()).spawn(move || {
            let r = job(&c2, &p2);
            let _ = tx.send(r);
            wake();
        });
        if spawned.is_ok() {
            self.result = None;
            self.task = Some(Task { rx, ctl, progress, total, started: Instant::now() });
        }
    }

    /// Stop the running split / join (it cleans up and reports "cancelled").
    pub fn cancel(&self) {
        if let Some(t) = &self.task {
            t.ctl.cancel();
        }
    }

    /// While running: bytes done, bytes in all and the speed (bytes per second).
    pub fn progress(&self) -> Option<(u64, u64, u64)> {
        let t = self.task.as_ref()?;
        let done = t.progress.load(Ordering::Relaxed);
        let secs = t.started.elapsed().as_secs_f64().max(0.001);
        Some((done, t.total.load(Ordering::Relaxed), (done as f64 / secs) as u64))
    }

    /// The progress as a line ("1.2 GiB of 4.2 GiB · 180 MiB/s").
    pub fn progress_text(&self) -> Option<String> {
        let (done, total, speed) = self.progress()?;
        Some(tr!(
            "tools-split-progress",
            done = format_size(done, SizeStyle::Binary),
            total = format_size(total, SizeStyle::Binary),
            speed = format_size(speed, SizeStyle::Binary)
        ))
    }

    /// Every frame: take the probe's findings and a finished run's result. Returns the
    /// toast a finished run shows (success, or a checksum mismatch).
    pub fn poll(&mut self) -> Option<Toast> {
        if let Some(rx) = &self.probe
            && let Ok(p) = rx.try_recv()
        {
            self.probe = None;
            match p {
                Probe::Size(s) => self.size = s,
                Probe::Parts(parts, has) => {
                    self.parts = Some(parts);
                    self.has_checksum = has;
                }
            }
        }
        let t = self.task.as_ref()?;
        let res = t.rx.try_recv().ok()?;
        self.task = None;
        let (toast, result) = match res {
            Ok(Outcome::Split(o)) => {
                let msg = tr!(
                    "tools-split-done-body",
                    count = o.parts.len(),
                    checksum = soyuz_core::entry::display_name(&o.checksum)
                );
                (Some(Toast::success(tr!("tools-split-done")).body(msg.clone())), Ok(msg))
            }
            Ok(Outcome::Join(o)) => {
                let name = soyuz_core::entry::display_name(&o.output);
                let (toast, msg) = match o.verified {
                    Some(true) => (Toast::success(tr!("tools-join-done")), tr!("tools-join-verified", name = &name)),
                    Some(false) => {
                        (Toast::error(tr!("tools-join-mismatch")), tr!("tools-join-mismatch-body", name = &name))
                    }
                    None => (Toast::success(tr!("tools-join-done")), tr!("tools-join-unverified", name = &name)),
                };
                let result = if o.verified == Some(false) { Err(msg.clone()) } else { Ok(msg.clone()) };
                (Some(toast.body(msg)), result)
            }
            Err(e) => (None, Err(e)),
        };
        self.result = Some(result);
        toast
    }
}

fn spawn_probe(f: impl FnOnce() -> Probe + Send + 'static) -> Option<Receiver<Probe>> {
    let (tx, rx) = bounded(1);
    let wake = super::tools::wake();
    std::thread::Builder::new()
        .name("soyuz-split-probe".into())
        .spawn(move || {
            let _ = tx.send(f());
            wake();
        })
        .ok()?;
    Some(rx)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn data(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 31 % 251) as u8).collect()
    }

    #[test]
    fn split_join_round_trip() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("movie.bin");
        let bytes = data(2_500_123);
        std::fs::write(&src, &bytes).unwrap();
        let out = t.path().join("parts");
        std::fs::create_dir(&out).unwrap();
        let ctl = OpControl::new();
        let pr = AtomicU64::new(0);
        let s = split_file(&src, &out, 1_000_000, &ctl, &pr).unwrap();
        assert_eq!(s.parts.len(), 3);
        assert_eq!(pr.load(Ordering::Relaxed), bytes.len() as u64);
        assert_eq!(std::fs::metadata(&s.parts[0]).unwrap().len(), 1_000_000);
        assert_eq!(std::fs::metadata(&s.parts[2]).unwrap().len(), 500_123);
        assert!(s.parts[0].to_string_lossy().ends_with("movie.bin.001"));
        assert_eq!(parse_checksum(&std::fs::read_to_string(&s.checksum).unwrap()), Some(s.hash.clone()));

        let (base, parts) = find_parts(&s.parts[1]).unwrap();
        assert_eq!(base, out.join("movie.bin"));
        assert_eq!(parts, s.parts);
        let dest_dir = t.path().join("joined");
        std::fs::create_dir(&dest_dir).unwrap();
        let output = dest_dir.join("movie.bin");
        let pr = AtomicU64::new(0);
        let j = join_parts(&parts, &output, Some(&s.checksum), &ctl, &pr).unwrap();
        assert_eq!(j.verified, Some(true));
        assert_eq!(j.output, output);
        assert_eq!(std::fs::read(&output).unwrap(), bytes);
        // Joining again refuses to overwrite.
        assert!(join_parts(&parts, &output, None, &ctl, &pr).is_err());
    }

    #[test]
    fn corrupt_part_fails_verification() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("a.dat");
        std::fs::write(&src, data(300_000)).unwrap();
        let ctl = OpControl::new();
        let pr = AtomicU64::new(0);
        let s = split_file(&src, t.path(), 100_000, &ctl, &pr).unwrap();
        let mut b = std::fs::read(&s.parts[1]).unwrap();
        b[10] ^= 0xFF;
        std::fs::write(&s.parts[1], b).unwrap();
        let out = t.path().join("out.dat");
        let j = join_parts(&s.parts, &out, Some(&s.checksum), &ctl, &pr).unwrap();
        assert_eq!(j.verified, Some(false));
        assert!(!out.exists(), "a mismatching result is not put in place");
        assert!(j.output.to_string_lossy().ends_with(".bad"));
    }

    #[test]
    fn cancel_cleans_up_and_refuses_existing_parts() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("b.dat");
        std::fs::write(&src, data(50_000)).unwrap();
        let ctl = OpControl::new();
        ctl.cancel();
        let pr = AtomicU64::new(0);
        assert!(split_file(&src, t.path(), 10_000, &ctl, &pr).is_err());
        assert!(!part_path(&src, 1).exists(), "partial parts removed");
        std::fs::write(part_path(&src, 2), b"x").unwrap();
        assert!(split_file(&src, t.path(), 10_000, &OpControl::new(), &pr).unwrap_err().contains("exists"));
    }

    #[test]
    fn split_and_join_never_replace_existing_files() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("c.dat");
        std::fs::write(&src, data(30_000)).unwrap();
        let ctl = OpControl::new();
        let pr = AtomicU64::new(0);
        let out = t.path().join("out");
        std::fs::create_dir(&out).unwrap();
        // An existing checksum file is not replaced (and nothing is written).
        std::fs::write(checksum_path(&out.join("c.dat")), b"mine").unwrap();
        assert!(split_file(&src, &out, 10_000, &ctl, &pr).unwrap_err().contains("exists"));
        assert_eq!(std::fs::read(checksum_path(&out.join("c.dat"))).unwrap(), b"mine");
        assert!(!part_path(&out.join("c.dat"), 1).exists());
        // A mismatching join whose `.bad` name is taken leaves that file alone.
        let s = split_file(&src, t.path(), 10_000, &ctl, &pr).unwrap();
        let mut b = std::fs::read(&s.parts[0]).unwrap();
        b[0] ^= 0xFF;
        std::fs::write(&s.parts[0], b).unwrap();
        let joined = t.path().join("joined.dat");
        let bad = t.path().join("joined.dat.bad");
        std::fs::write(&bad, b"earlier result").unwrap();
        assert!(join_parts(&s.parts, &joined, Some(&s.checksum), &ctl, &pr).is_err());
        assert_eq!(std::fs::read(&bad).unwrap(), b"earlier result");
        assert!(!joined.exists());
        let leftovers = std::fs::read_dir(t.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".soyuz-join.part"));
        assert_eq!(leftovers.count(), 0, "temp file removed");
    }

    #[test]
    fn part_names() {
        assert_eq!(split_part(Path::new("/x/a.tar.gz.007")), Some((PathBuf::from("/x/a.tar.gz"), 7)));
        assert!(is_part(Path::new("v.001")));
        assert!(!is_part(Path::new("v.txt")));
        assert!(!is_part(Path::new("v.01")));
        assert_eq!(part_path(Path::new("f.iso"), 12), PathBuf::from("f.iso.012"));
        assert_eq!(parse_checksum("zz  f"), None);
    }

    /// Poll `m` until `done` holds (5 s at most).
    fn settle(m: &mut SplitModel, done: impl Fn(&SplitModel) -> bool) -> Option<Toast> {
        let start = Instant::now();
        let mut toast = None;
        while !done(m) {
            assert!(start.elapsed() < Duration::from_secs(5), "timed out: {m:?}");
            toast = m.poll().or(toast);
            std::thread::sleep(Duration::from_millis(5));
        }
        toast.or_else(|| m.poll())
    }

    #[test]
    fn the_model_probes_off_thread_then_splits_and_joins() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("big.bin");
        std::fs::write(&src, data(3 * 1024 * 1024)).unwrap();
        let mut m = SplitModel::split(src.clone());
        assert_eq!(m.out_dir, show_path(t.path()));
        // A pending probe cannot start.
        if m.probing() {
            assert!(!m.can_start());
            m.start();
            assert!(!m.running());
        }
        settle(&mut m, |m| !m.probing());
        assert_eq!(m.size, Some(3 * 1024 * 1024));
        m.preset = CUSTOM;
        m.custom = "nonsense".into();
        m.start();
        assert!(!m.running());
        assert!(matches!(&m.result, Some(Err(_))), "a bad custom size is the result");
        m.custom = "1 MiB".into();
        assert_eq!(m.plan(), Some((3, 1024 * 1024)));
        assert!(m.plan_text().unwrap().starts_with('3'));
        m.start();
        assert!(m.running());
        let toast = settle(&mut m, |m| !m.running());
        assert!(m.finished_ok(), "{:?}", m.result);
        assert!(toast.is_some());
        let parts: Vec<PathBuf> = (1..=3).map(|n| part_path(&src, n)).collect();
        assert!(parts.iter().all(|p| p.is_file()));
        assert!(checksum_path(&src).is_file());

        // Join into a sub-folder: verified.
        let mut j = SplitModel::join(parts[0].clone());
        settle(&mut j, |m| !m.probing());
        assert!(j.has_checksum);
        assert!(matches!(&j.parts, Some(Ok((_, p))) if p.len() == 3));
        let sub = t.path().join("out");
        std::fs::create_dir(&sub).unwrap();
        j.out_dir = show_path(&sub);
        j.start();
        settle(&mut j, |m| !m.running());
        assert!(j.finished_ok(), "{:?}", j.result);
        assert_eq!(std::fs::read(sub.join("big.bin")).unwrap(), std::fs::read(&src).unwrap());
    }

    #[test]
    fn a_cancelled_model_leaves_nothing() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("c.bin");
        std::fs::write(&src, data(4 * 1024 * 1024)).unwrap();
        let mut m = SplitModel::split(src.clone());
        settle(&mut m, |m| !m.probing());
        m.preset = CUSTOM;
        m.custom = "1 MiB".into();
        m.start();
        m.cancel();
        settle(&mut m, |m| !m.running());
        if !m.finished_ok() {
            assert!(!part_path(&src, 1).exists(), "partial parts removed");
            assert!(!checksum_path(&src).exists());
        }
        // A join of something that is not a part reports why.
        let mut j = SplitModel::join(src);
        settle(&mut j, |m| !m.probing());
        assert!(matches!(&j.parts, Some(Err(_))));
        assert!(!j.can_start());
    }
}
