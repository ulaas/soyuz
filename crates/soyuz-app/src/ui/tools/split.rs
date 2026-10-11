//! File split / join. Splitting writes `name.ext.001`, `.002`, … plus a
//! b3sum-compatible checksum file `name.ext.blake3` (`<hash>  name.ext`); joining finds
//! the consecutive parts, concatenates them into a temporary file, verifies against the
//! checksum file when present and renames the result into place. Both run on a worker
//! thread with byte progress and cancellation; failures and cancels remove partial output.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crossbeam_channel::{Receiver, bounded};
use egui::{Align, FontId, Layout, RichText, Ui, vec2};
use soyuz_core::{OpControl, SizeStyle, format_size};

use super::{button, progress_bar, tool_window};
use crate::app::SoyuzApp;
use crate::theme::{icons::ph, palette};
use crate::ui::toasts::Toast;

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

pub use crate::models::tools::{is_part, split_part};

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
// Window.

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Split,
    Join,
}

enum Outcome {
    Split(SplitOutcome),
    Join(JoinOutcome),
}

/// The blocking split / join call run on the worker.
type Work = Box<dyn FnOnce(&OpControl, &AtomicU64) -> Result<Outcome, String> + Send>;

struct Task {
    rx: Receiver<Result<Outcome, String>>,
    ctl: Arc<OpControl>,
    progress: Arc<AtomicU64>,
    total: u64,
    started: Instant,
}

/// Part size presets (label, bytes); 0 bytes is "Custom" (translated at draw time).
const PRESETS: [(&str, u64); 6] = [
    ("10 MB", 10_000_000),
    ("100 MB", 100_000_000),
    ("700 MB (CD)", 700 * 1024 * 1024),
    ("1 GB", 1_000_000_000),
    ("4 GB − 1 (FAT32)", 4 * 1024 * 1024 * 1024 - 1),
    ("", 0),
];

pub struct SplitWindow {
    pub open: bool,
    mode: Mode,
    path: PathBuf,
    out_dir: String,
    preset: usize,
    custom: String,
    size: Option<u64>,
    parts: Option<Result<(PathBuf, Vec<PathBuf>), String>>,
    /// A `.blake3` checksum file sits next to the parts (checked once, not per frame).
    has_checksum: bool,
    task: Option<Task>,
    result: Option<Result<String, String>>,
}

impl SplitWindow {
    pub fn split(path: PathBuf) -> Self {
        let size = std::fs::metadata(&path).ok().map(|m| m.len());
        let out = path.parent().map(super::tags_ui::show_path).unwrap_or_default();
        SplitWindow {
            open: true,
            mode: Mode::Split,
            path,
            out_dir: out,
            preset: 1,
            custom: "250 MB".into(),
            size,
            parts: None,
            has_checksum: false,
            task: None,
            result: None,
        }
    }

    pub fn join(path: PathBuf) -> Self {
        let parts = Some(find_parts(&path));
        let has_checksum = matches!(&parts, Some(Ok((base, _))) if checksum_path(base).is_file());
        let out = path.parent().map(super::tags_ui::show_path).unwrap_or_default();
        SplitWindow {
            open: true,
            mode: Mode::Join,
            path,
            out_dir: out,
            preset: 1,
            custom: String::new(),
            size: None,
            parts,
            has_checksum,
            task: None,
            result: None,
        }
    }

    fn part_size(&self) -> Option<u64> {
        match PRESETS[self.preset].1 {
            0 => crate::services::search::parse_size(&self.custom).filter(|s| *s > 0),
            s => Some(s),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, app: &mut SoyuzApp) {
        self.poll(app);
        let title = if self.mode == Mode::Split { tr!("tools-split-title") } else { tr!("tools-join-title") };
        let icon = if self.mode == Mode::Split { ph::ARROWS_SPLIT } else { ph::ARROWS_MERGE };
        let name = soyuz_core::entry::display_name(&self.path);
        let keep = tool_window(ctx, "split", title, &name, icon, vec2(520.0, 330.0), |ui| self.body(ui));
        if !keep {
            if let Some(t) = &self.task {
                t.ctl.cancel();
            }
            self.open = false;
        }
    }

    fn poll(&mut self, app: &mut SoyuzApp) {
        let Some(t) = &self.task else { return };
        let Ok(res) = t.rx.try_recv() else { return };
        self.task = None;
        self.result = Some(match res {
            Ok(Outcome::Split(o)) => {
                let msg = tr!(
                    "tools-split-done-body",
                    count = o.parts.len(),
                    checksum = soyuz_core::entry::display_name(&o.checksum)
                );
                app.ui.toasts.push(Toast::success(tr!("tools-split-done")).body(msg.clone()));
                Ok(msg)
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
                app.ui.toasts.push(toast.body(msg.clone()));
                if o.verified == Some(false) { Err(msg) } else { Ok(msg) }
            }
            Err(e) => Err(e),
        });
    }

    fn start(&mut self) {
        let ctl = OpControl::new();
        let progress = Arc::new(AtomicU64::new(0));
        let (tx, rx) = bounded(1);
        let wake = super::wake();
        let out_dir = PathBuf::from(self.out_dir.trim());
        let (total, job): (u64, Work) = match self.mode {
            Mode::Split => {
                let Some(size) = self.part_size() else {
                    self.result = Some(Err(tr!("tools-split-err-invalid-size").into()));
                    return;
                };
                let src = self.path.clone();
                (
                    self.size.unwrap_or(0),
                    Box::new(move |ctl, pr| split_file(&src, &out_dir, size, ctl, pr).map(Outcome::Split)),
                )
            }
            Mode::Join => {
                let Some(Ok((base, parts))) = self.parts.clone() else { return };
                let total = parts.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
                let name = base.file_name().map(|n| n.to_os_string()).unwrap_or_default();
                let output = out_dir.join(name);
                let ck = checksum_path(&base);
                (
                    total,
                    Box::new(move |ctl, pr| {
                        let ck = ck.is_file().then_some(ck.as_path());
                        join_parts(&parts, &output, ck, ctl, pr).map(Outcome::Join)
                    }),
                )
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

    fn body(&mut self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        let label = |ui: &mut Ui, s: &str| {
            ui.label(RichText::new(s).font(FontId::proportional(11.5)).color(p.text_tertiary));
        };
        match self.mode {
            Mode::Split => {
                label(ui, tr!("tools-split-label-file"));
                let size = self
                    .size
                    .map(|s| format_size(s, SizeStyle::Binary))
                    .unwrap_or_else(|| tr!("tools-split-unknown-size").into());
                ui.label(
                    RichText::new(format!("{}  ·  {size}", super::tags_ui::show_path(&self.path)))
                        .color(p.text_primary),
                );
                ui.add_space(8.0);
                label(ui, tr!("tools-split-label-part-size"));
                ui.horizontal_wrapped(|ui| {
                    for (i, (name, _)) in PRESETS.iter().enumerate() {
                        let mut on = self.preset == i;
                        let name = if name.is_empty() { tr!("tools-split-custom") } else { name };
                        if super::toggle_chip(ui, name, &mut on, p.accent).clicked() {
                            self.preset = i;
                        }
                    }
                });
                if PRESETS[self.preset].1 == 0 {
                    super::text_field(ui, &mut self.custom, tr!("tools-split-custom-hint"), 160.0);
                }
                if let (Some(total), Some(ps)) = (self.size, self.part_size()) {
                    let n = total.div_ceil(ps).max(1);
                    ui.label(
                        RichText::new(tr!("tools-split-plan", count = n, size = format_size(ps, SizeStyle::Binary)))
                            .font(FontId::proportional(12.0))
                            .color(p.text_secondary),
                    );
                }
            }
            Mode::Join => {
                label(ui, tr!("tools-split-label-parts"));
                match &self.parts {
                    Some(Ok((base, parts))) => {
                        let has_ck = self.has_checksum;

                        ui.label(
                            RichText::new(tr!(
                                "tools-join-plan",
                                count = parts.len(),
                                name = soyuz_core::entry::display_name(base)
                            ))
                            .color(p.text_primary),
                        );
                        ui.label(
                            RichText::new(if has_ck {
                                tr!("tools-join-has-checksum")
                            } else {
                                tr!("tools-join-no-checksum")
                            })
                            .font(FontId::proportional(12.0))
                            .color(if has_ck { p.success } else { p.text_tertiary }),
                        );
                    }
                    Some(Err(e)) => {
                        ui.label(RichText::new(e).color(p.danger));
                    }
                    None => {}
                }
            }
        }
        ui.add_space(8.0);
        label(ui, tr!("tools-split-label-output"));
        super::text_field(ui, &mut self.out_dir, tr!("tools-split-folder-hint"), ui.available_width());
        ui.add_space(10.0);
        if let Some(t) = &self.task {
            let done = t.progress.load(Ordering::Relaxed);
            let frac = if t.total > 0 { Some(done as f32 / t.total as f32) } else { None };
            progress_bar(ui, ui.available_width(), frac);
            let secs = t.started.elapsed().as_secs_f64().max(0.001);
            ui.label(
                RichText::new(tr!(
                    "tools-split-progress",
                    done = format_size(done, SizeStyle::Binary),
                    total = format_size(t.total, SizeStyle::Binary),
                    speed = format_size((done as f64 / secs) as u64, SizeStyle::Binary)
                ))
                .font(FontId::proportional(12.0))
                .color(p.text_secondary),
            );
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        if let Some(r) = &self.result {
            let (text, color) = match r {
                Ok(m) => (m.as_str(), p.success),
                Err(e) => (e.as_str(), p.danger),
            };
            ui.label(RichText::new(text).color(color));
        }
        ui.with_layout(Layout::bottom_up(Align::Max), |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.task.is_some() {
                        if button(ui, Some(ph::STOP), tr!("common-cancel"), 0, true).clicked()
                            && let Some(t) = &self.task
                        {
                            t.ctl.cancel();
                        }
                    } else if matches!(self.result, Some(Ok(_))) {
                        if button(ui, Some(ph::CHECK), tr!("tools-done"), 1, true).clicked() {
                            self.open = false;
                        }
                    } else {
                        let ok = match self.mode {
                            Mode::Split => self.size.is_some(),
                            Mode::Join => matches!(self.parts, Some(Ok(_))),
                        };
                        let label =
                            if self.mode == Mode::Split { tr!("tools-split-start") } else { tr!("tools-join-start") };
                        if button(ui, Some(ph::PLAY), label, 1, ok && !self.out_dir.trim().is_empty()).clicked() {
                            self.start();
                        }
                    }
                });
            });
        });
    }
}

#[cfg(test)]
mod tests {
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
}
