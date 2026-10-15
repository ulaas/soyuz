//! Pure naming helpers: unique default names for new items (resolved against the pane's
//! listing, no I/O), job titles ("Copying 1,204 items to D:\Backup") and summaries.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use soyuz_core::naming::DEFAULT_UNIQUE_PATTERN;
use soyuz_jobs::{JobState, JobSummary};

use crate::models::media_tools::MediaTool;
use crate::services::jobs::{JobMeta, OpKind};

/// A name based on `base` that does not collide with `existing` (`New folder`,
/// `New folder (2)`, …; the extension stays last: `New file (2).txt`).
pub fn unique_in(existing: &[&str], base: &str, case_insensitive: bool) -> String {
    let norm = |s: &str| if case_insensitive { s.to_lowercase() } else { s.to_string() };
    let set: HashSet<String> = existing.iter().map(|s| norm(s)).collect();
    let dir = Path::new("");
    let exists = |p: &Path| p.file_name().is_some_and(|n| set.contains(&norm(&n.to_string_lossy())));
    let out = soyuz_core::unique_name(dir, base, DEFAULT_UNIQUE_PATTERN, &exists);
    out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| base.to_string())
}

/// Whether names compare case-insensitively on this platform's usual filesystems.
pub fn case_insensitive_fs() -> bool {
    cfg!(any(windows, target_os = "macos"))
}

/// Display form of a path (no `\\?\` prefix).
pub fn show_path(p: &Path) -> String {
    soyuz_fs::strip_verbatim(p).to_string_lossy().into_owned()
}

fn quoted(p: &Path) -> String {
    format!("\u{201c}{}\u{201d}", soyuz_core::entry::display_name(p))
}

/// "“report.pdf”" for one item, "1,204 items" otherwise.
pub fn items_phrase(n: u64, first: Option<&Path>) -> String {
    match (n, first) {
        (1, Some(f)) => quoted(f),
        (n, _) => tr!("common-items", count = n),
    }
}

/// Card / toast title: `Copying 1,204 items to D:\Backup`. `total_items` (the planner's
/// count) replaces the top-level count once known.
pub fn job_title(meta: &JobMeta, active: bool, total_items: Option<u64>, label: &str) -> String {
    // Fluent selector: in progress vs. finished.
    let state = if active { "active" } else { "done" };
    match meta.op {
        OpKind::Undo | OpKind::Redo | OpKind::Retry | OpKind::Other => {
            // Engine labels already describe the original operation.
            let l = lower_first(label.trim_start_matches("Undo: ").trim_start_matches("Redo: "));
            match meta.op {
                OpKind::Undo => tr!("ops-job-undo", state = state, label = l),
                OpKind::Redo => tr!("ops-job-redo", state = state, label = l),
                OpKind::Retry => tr!("ops-job-retry", state = state, label = l),
                _ => tr!("ops-job-other", state = state, label = l),
            }
        }
        OpKind::EmptyTrash => tr!("ops-job-empty-trash", state = state),
        OpKind::Media(tool) => {
            // "Converting “clip.mov” to “clip.mp4”": the output name is the destination.
            let what = items_phrase(meta.items as u64, meta.first.as_deref());
            let dest = meta.dest.as_deref().map(quoted).unwrap_or_default();
            match tool {
                MediaTool::Convert => tr!("ops-job-media-convert", state = state, what = what, dest = dest),
                MediaTool::ExtractAudio => tr!("ops-job-media-extract", state = state, what = what, dest = dest),
                MediaTool::Trim => tr!("ops-job-media-trim", state = state, what = what, dest = dest),
                MediaTool::SaveFrame => tr!("ops-job-media-frame", state = state, what = what, dest = dest),
            }
        }
        OpKind::NewFolder | OpKind::NewFile | OpKind::Link => {
            let name = meta.first.as_deref().map(quoted).unwrap_or_default();
            // The messages select on `has_name` (the name is unknown for some jobs).
            let has = if name.is_empty() { "no" } else { "yes" };
            match meta.op {
                OpKind::NewFolder => tr!("ops-job-new-folder", state = state, has_name = has, name = name),
                OpKind::NewFile => tr!("ops-job-new-file", state = state, has_name = has, name = name),
                _ => tr!("ops-job-link", state = state, has_name = has, name = name),
            }
        }
        _ => {
            let n = match total_items {
                Some(t) if t > meta.items as u64 && meta.items != 1 => t,
                _ => meta.items as u64,
            };
            let what = items_phrase(n, meta.first.as_deref());
            match (&meta.dest, meta.op) {
                (_, OpKind::Trash) => tr!("ops-job-trash", state = state, what = what),
                (Some(d), OpKind::Copy) => tr!("ops-job-copy-to", state = state, what = what, dest = show_path(d)),
                (Some(d), OpKind::Move) => tr!("ops-job-move-to", state = state, what = what, dest = show_path(d)),
                (Some(d), OpKind::Rename) => tr!("ops-job-rename-to", state = state, what = what, dest = quoted(d)),
                (_, OpKind::Copy) => tr!("ops-job-copy", state = state, what = what),
                (_, OpKind::Move) => tr!("ops-job-move", state = state, what = what),
                (_, OpKind::Rename) => tr!("ops-job-rename", state = state, what = what),
                (_, OpKind::Duplicate) => tr!("ops-job-duplicate", state = state, what = what),
                (_, OpKind::Delete) => tr!("ops-job-delete", state = state, what = what),
                (_, OpKind::Wipe) => tr!("ops-job-wipe", state = state, what = what),
                (_, OpKind::SetMeta) => tr!("ops-job-set-meta", state = state, what = what),
                (_, OpKind::Restore) => tr!("ops-job-restore", state = state, what = what),
                _ => tr!("ops-job-other", state = state, label = what),
            }
        }
    }
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// One-line outcome: "Done in 12 s · 1,204 items · 2 skipped · 1 failed".
pub fn outcome(state: &JobState) -> String {
    match state {
        JobState::Done(s) => summary_line(s),
        JobState::Failed(e) => tr!("ops-outcome-failed", error = e.to_string()),
        JobState::Cancelled => tr!("ops-outcome-cancelled").into(),
        _ => String::new(),
    }
}

pub fn summary_line(s: &JobSummary) -> String {
    let mut parts = vec![tr!("ops-summary-done-in", time = soyuz_core::format_eta(s.elapsed.as_secs_f64().max(0.0)))];
    if s.items_ok > 0 {
        parts.push(tr!("common-items", count = s.items_ok));
    }
    if s.items_skipped > 0 {
        parts.push(tr!("ops-summary-skipped", count = s.items_skipped));
    }
    if s.items_failed > 0 {
        parts.push(tr!("ops-summary-failed", count = s.items_failed));
    }
    parts.join(" \u{b7} ")
}

/// `a.txt` → `a - link.txt` (the extension stays last so the type is still visible).
pub fn link_base(target: &Path) -> String {
    let name = soyuz_core::entry::display_name(target);
    let (stem, ext) = match name.rfind('.').filter(|&i| i > 0 && i + 1 < name.len()) {
        Some(i) => (&name[..i], &name[i..]),
        None => (name.as_str(), ""),
    };
    tr!("ops-link-default-name", stem = stem, ext = ext)
}

/// Default (unique) name for a link to `target` among `existing` names.
pub fn link_name(target: &Path, existing: &[&str]) -> String {
    unique_in(existing, &link_base(target), case_insensitive_fs())
}

/// Parent directories of `paths` (deduplicated, for refreshes).
pub fn parents(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in paths {
        if let Some(d) = p.parent()
            && !out.iter().any(|o| o == d)
        {
            out.push(d.to_path_buf());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn unique_default_names() {
        assert_eq!(unique_in(&[], "New folder", true), "New folder");
        assert_eq!(unique_in(&["New folder"], "New folder", true), "New folder (2)");
        assert_eq!(unique_in(&["new FOLDER", "New folder (2)"], "New folder", true), "New folder (3)");
        assert_eq!(unique_in(&["new folder"], "New folder", false), "New folder", "case-sensitive fs");
        assert_eq!(unique_in(&["New file.txt"], "New file.txt", true), "New file (2).txt");
        assert_eq!(link_name(Path::new("/x/a.txt"), &["a - link.txt"]), "a - link (2).txt");
        assert_eq!(link_base(Path::new("/x/Docs")), "Docs - link");
        assert_eq!(link_base(Path::new("/x/.env")), ".env - link");
    }

    #[test]
    fn titles() {
        let mut m = JobMeta::new(OpKind::Copy, &[PathBuf::from("/a/x"), PathBuf::from("/a/y")], Some("/b".into()));
        assert_eq!(job_title(&m, true, None, ""), "Copying 2 items to /b");
        assert_eq!(job_title(&m, true, Some(1204), ""), "Copying 1,204 items to /b");
        assert_eq!(job_title(&m, false, Some(1204), ""), "Copied 1,204 items to /b");
        m.items = 1;
        m.first = Some("/a/report.pdf".into());
        assert_eq!(job_title(&m, true, Some(9), ""), "Copying \u{201c}report.pdf\u{201d} to /b");
        let t = JobMeta::new(OpKind::Trash, &[PathBuf::from("/a/x")], None);
        assert_eq!(job_title(&t, false, None, ""), "Moved \u{201c}x\u{201d} to the trash");
        let u = JobMeta::new(OpKind::Undo, &[], None);
        assert_eq!(
            job_title(&u, true, None, "Undo: Copy 3 items to \u{201c}b\u{201d}"),
            "Undoing: copy 3 items to \u{201c}b\u{201d}"
        );
        let e = JobMeta::new(OpKind::EmptyTrash, &[], None);
        assert_eq!(job_title(&e, true, None, ""), "Emptying trash");
        let m = JobMeta::new(
            OpKind::Media(MediaTool::Convert),
            &[PathBuf::from("/v/clip.mov")],
            Some("/v/clip.mp4".into()),
        );
        assert_eq!(job_title(&m, true, None, ""), "Converting \u{201c}clip.mov\u{201d} to \u{201c}clip.mp4\u{201d}");
        assert_eq!(job_title(&m, false, None, ""), "Converted \u{201c}clip.mov\u{201d} to \u{201c}clip.mp4\u{201d}");
        let f = JobMeta::new(
            OpKind::Media(MediaTool::SaveFrame),
            &[PathBuf::from("/v/clip.mov")],
            Some("/v/clip.png".into()),
        );
        assert_eq!(
            job_title(&f, true, None, ""),
            "Saving a frame of \u{201c}clip.mov\u{201d} to \u{201c}clip.png\u{201d}"
        );
    }

    #[test]
    fn turkish_titles() {
        use soyuz_core::Locale;
        let mut a = crate::i18n::FluentArgs::new();
        a.set("state", "active");
        a.set("what", "1.204 öğe");
        a.set("dest", "D:\\Yedek");
        assert_eq!(crate::i18n::t_args_in("ops-job-copy-to", &a, Locale::TrTr), "Kopyalanıyor: 1.204 öğe → D:\\Yedek");
        let mut a = crate::i18n::FluentArgs::new();
        a.set("state", "done");
        a.set("has_name", "no");
        a.set("name", "");
        assert_eq!(crate::i18n::t_args_in("ops-job-new-folder", &a, Locale::TrTr), "Klasör oluşturuldu");
    }

    #[test]
    fn outcomes() {
        let s = JobSummary {
            items_ok: 1204,
            items_skipped: 2,
            items_failed: 1,
            elapsed: Duration::from_secs(12),
            ..Default::default()
        };
        assert_eq!(summary_line(&s), "Done in 12s \u{b7} 1,204 items \u{b7} 2 skipped \u{b7} 1 failed");
        assert_eq!(outcome(&JobState::Cancelled), "Cancelled");
        assert_eq!(items_phrase(1, None), "1 item");
        assert_eq!(parents(&[PathBuf::from("/a/x"), PathBuf::from("/a/y"), PathBuf::from("/b/z")]).len(), 2);
    }
}
