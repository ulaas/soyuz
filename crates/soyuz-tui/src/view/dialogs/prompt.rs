//! The view of job prompts (`DialogModel::Prompt`): the job, what happened (a conflict
//! compares the source and the destination: folder, size, date, "Newer" / "Larger"),
//! the form (new name, apply to all, password) and the buttons; the "More…" options of a
//! conflict as a list. Names and messages come from disk: `crate::dialog::safe`.

use std::cmp::Ordering;
use std::path::Path;
use std::time::SystemTime;

use soyuz_appcore::models::dialogs::{DialogModel, PromptModel};
use soyuz_appcore::tr;
use soyuz_core::{EntryInfo, SizeStyle, format_size};
use soyuz_jobs::kinds::extract::ARCHIVE_LIMIT_OP;
use soyuz_jobs::{ConflictInfo, ConflictKind, JobPrompt};

use super::DialogVm;
use crate::dialog::safe;
use crate::input::dialogs::ModelUi;
use crate::input::dialogs::prompt::{MORE, options};
use crate::widgets::ListRow;

/// Box width of a conflict (two entries to compare); other prompts use the default.
pub const CONFLICT_WIDTH: u16 = 76;

/// What the dialog shows.
pub fn vm(m: &DialogModel, ui: &ModelUi) -> DialogVm {
    let DialogModel::Prompt(p) = m else { return crate::dialog::fallback(m) };
    let renaming = p.rename.is_some();
    let more = ui.page == MORE && !renaming;
    let mut d = DialogVm::from_form(m.title(), &ui.form);
    d.body = body(p);
    d.alert = m.alert();
    if matches!(p.prompt, JobPrompt::Conflict(_)) {
        d.width = CONFLICT_WIDTH;
    }
    if more {
        d.list = options(p).into_iter().map(|(label, _)| ListRow::new(label)).collect();
        d.list_state = ui.list.clone();
    }
    d.hints = if more {
        tr!("tui-dialogs-hint-more")
    } else if renaming {
        tr!("tui-dialogs-hint-rename")
    } else {
        tr!("tui-dialogs-hint-prompt")
    }
    .to_string();
    d
}

/// A path as shown.
fn path(p: &Path) -> String {
    safe(&soyuz_fs::strip_verbatim(p).display().to_string())
}

/// A display name for the messages (they add the quotes themselves).
fn name(p: &Path) -> String {
    safe(&soyuz_core::entry::display_name(p))
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// The text above the form.
pub fn body(p: &PromptModel) -> Vec<String> {
    let mut b = vec![safe(&p.job_title)];
    if p.total() > 1 {
        b.push(tr!("dialogs-prompt-waiting", total = p.total()));
    }
    b.push(String::new());
    match &p.prompt {
        JobPrompt::Conflict(c) => conflict(&mut b, c),
        JobPrompt::Error { path: at, error, .. } if error.op == ARCHIVE_LIMIT_OP => {
            b.push(tr!("dialogs-prompt-archive-limit", name = name(at)));
            if !error.message.is_empty() {
                b.push(safe(&error.message));
            }
            b.push(tr!("dialogs-prompt-archive-limit-hint").to_string());
            b.push(format!("  {}", path(at)));
        }
        JobPrompt::Error { path: at, error, .. } => {
            b.push(tr!("dialogs-prompt-error", name = name(at)));
            b.push(capitalize(error.kind.describe()));
            if !error.message.is_empty() && error.message != error.kind.describe() {
                b.push(safe(&error.message));
            }
            b.push(format!("  {}", path(at)));
        }
        JobPrompt::NotEnoughSpace { needed, free, dest } => {
            b.push(tr!(
                "dialogs-prompt-space",
                name = name(dest),
                needed = format_size(*needed, SizeStyle::Binary),
                free = format_size(*free, SizeStyle::Binary)
            ));
            b.push(tr!("dialogs-prompt-space-hint").to_string());
        }
        JobPrompt::TrashUnsupported { path: at, reason } => {
            b.push(tr!("dialogs-prompt-trash", name = name(at)));
            if !reason.is_empty() {
                b.push(safe(reason));
            }
            b.push(tr!("dialogs-prompt-trash-permanent").to_string());
        }
        JobPrompt::ReadOnlyItem { path: at } => {
            b.push(tr!("dialogs-prompt-read-only", name = name(at)));
            b.push(tr!("dialogs-prompt-read-only-ask").to_string());
            b.push(format!("  {}", path(at)));
        }
        JobPrompt::Password { archive, wrong } => {
            b.push(tr!("dialogs-prompt-encrypted", name = name(archive)));
            if *wrong {
                b.push(tr!("dialogs-prompt-wrong-password").to_string());
            }
        }
    }
    b
}

/// The conflict's message and the two entries side by side (one under the other here).
fn conflict(b: &mut Vec<String>, c: &ConflictInfo) {
    let dst = safe(&c.dst.name);
    b.push(match c.kind {
        ConflictKind::FileExists => tr!("dialogs-conflict-file-exists", name = &dst),
        ConflictKind::DirExists => tr!("dialogs-conflict-dir-exists", name = &dst),
        ConflictKind::FileVsDir => tr!("dialogs-conflict-kind-differs", name = &dst),
        ConflictKind::SameFile => tr!("dialogs-conflict-same").to_string(),
        ConflictKind::MoveIntoSelf => tr!("dialogs-conflict-into-self", name = safe(&c.src.name)),
        ConflictKind::CaseOnly => tr!("dialogs-conflict-case", name = &dst),
    });
    b.push(String::new());
    let newer = cmp_time(c.src.modified, c.dst.modified);
    let larger = (!c.src.is_dir_like() && !c.dst.is_dir_like()).then(|| c.src.size.cmp(&c.dst.size));
    let src = entry(
        tr!("dialogs-conflict-source"),
        &c.src,
        newer == Some(Ordering::Greater),
        larger == Some(Ordering::Greater),
    );
    let dst = entry(
        tr!("dialogs-conflict-destination"),
        &c.dst,
        newer == Some(Ordering::Less),
        larger == Some(Ordering::Less),
    );
    b.extend(src);
    b.extend(dst);
}

/// Two lines about one side: `Source: a.txt` and `  1 KiB · 2026-10-01 12:00 · Newer · C:\dir`.
pub fn entry(caption: &str, e: &EntryInfo, newer: bool, larger: bool) -> [String; 2] {
    let size = if e.is_dir_like() {
        tr!("dialogs-conflict-folder").to_string()
    } else {
        format_size(e.size, SizeStyle::Binary)
    };
    let date = e
        .modified
        .map(|t| soyuz_core::format_time(t, &soyuz_core::DateFormat::Named(soyuz_core::DateFormatName::Short)))
        .unwrap_or_else(|| "\u{2014}".into());
    let mut facts = vec![size];
    if larger {
        facts.push(tr!("dialogs-conflict-larger").to_string());
    }
    facts.push(date);
    if newer {
        facts.push(tr!("dialogs-conflict-newer").to_string());
    }
    if let Some(dir) = e.path.parent() {
        facts.push(path(dir));
    }
    [format!("{caption}: {}", safe(&e.name)), format!("  {}", facts.join(" \u{00b7} "))]
}

/// Modification times compared; within two seconds counts as equal (FAT timestamps,
/// copies), like the GUI.
fn cmp_time(a: Option<SystemTime>, b: Option<SystemTime>) -> Option<Ordering> {
    let (a, b) = (a?, b?);
    let d = match a.duration_since(b) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => -e.duration().as_secs_f64(),
    };
    Some(if d > 2.0 {
        Ordering::Greater
    } else if d < -2.0 {
        Ordering::Less
    } else {
        Ordering::Equal
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::Duration;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use soyuz_appcore::models::dialogs::PromptShared;
    use soyuz_core::EntryKind;
    use soyuz_jobs::JobId;

    use super::*;
    use crate::input::dialogs::prompt;
    use crate::layout::Hits;
    use crate::theme::TuiTheme;

    fn conflict_model(total: usize) -> DialogModel {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut src = EntryInfo::minimal(PathBuf::from("src").join("a\x1b.txt"), EntryKind::File);
        src.size = 4096;
        src.modified = Some(t + Duration::from_secs(60));
        let mut dst = EntryInfo::minimal(PathBuf::from("dst").join("a\x1b.txt"), EntryKind::File);
        dst.size = 10;
        dst.modified = Some(t);
        let c = ConflictInfo { src, dst, kind: ConflictKind::FileExists, suggested_name: "a (2).txt".into() };
        let shared = Rc::new(RefCell::new(PromptShared { alive: true, total, ..Default::default() }));
        PromptModel::new(JobId(1), JobPrompt::Conflict(c), "Copy 1 item".into(), shared).into()
    }

    fn screen(m: &DialogModel, ui: &ModelUi, w: u16, h: u16) -> String {
        draw(&vm(m, ui), w, h)
    }

    fn draw(d: &DialogVm, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut hits = Hits::default();
        t.draw(|f| super::super::draw_dialog(f, f.area(), d, &TuiTheme::plain(), &mut hits)).unwrap();
        let buf = t.backend().buffer();
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>() + "\n").collect()
    }

    #[test]
    fn conflict_compares_both_sides() {
        let m = conflict_model(3);
        let ui = prompt::ui(&m);
        let d = vm(&m, &ui);
        let all = d.body.join("\n");
        assert!(all.contains("Copy 1 item") && all.contains("1 of 3 waiting"), "{all}");
        assert!(!all.chars().any(|c| c.is_control() && c != '\n'), "names are made safe: {all:?}");
        let src = d.body.iter().position(|l| l.starts_with("Source:")).unwrap();
        assert!(
            d.body[src + 1].contains("4 KiB")
                && d.body[src + 1].contains("Larger")
                && d.body[src + 1].contains("Newer")
        );
        let dst = d.body.iter().position(|l| l.starts_with("Destination:")).unwrap();
        assert!(!d.body[dst + 1].contains("Newer"), "{}", d.body[dst + 1]);
        assert!(d.alert);
        let s = screen(&m, &ui, 100, 30);
        for t in ["[ ] Do this for all", "[ Replace ]", "[ Keep both ]", "[ More… ]", "[ Cancel operation ]"] {
            assert!(s.contains(t), "{t}: {s}");
        }
        // The "More…" page lists the options.
        let mut ui = ui;
        ui.page = MORE;
        ui.form = prompt::form(
            match &m {
                DialogModel::Prompt(p) => p,
                _ => unreachable!(),
            },
            MORE,
        );
        let s = screen(&m, &ui, 100, 30);
        assert!(s.contains("Replace if newer") && s.contains("Rename…") && s.contains("[ Back ]"), "{s}");
    }

    #[test]
    fn every_core_prompt_at_tiny_sizes() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;

        use soyuz_appcore::models::dialogs::{DeleteConfirmModel, LinkChoice, LinkModel, QuitModel, TransferModel};

        use crate::input::dialogs::{approval_mcp, approval_script, delete, link, quit, transfer};

        let mut vms = Vec::new();
        let m = conflict_model(2);
        let mut ui = prompt::ui(&m);
        vms.push(vm(&m, &ui));
        ui.page = MORE;
        vms.push(vm(&m, &ui));
        let m: DialogModel =
            DeleteConfirmModel::new(vec![PathBuf::from("a"), PathBuf::from("b")], soyuz_jobs::DeleteMode::Trash).into();
        vms.push(super::super::delete::vm(&m, &delete::ui(&m)));
        let m: DialogModel = QuitModel::new(1, Arc::new(AtomicBool::new(true))).into();
        vms.push(super::super::quit::vm(&m, &quit::ui(&m)));
        let m: DialogModel = TransferModel::new(vec![PathBuf::from("a")], PathBuf::from("d"), true).into();
        let mut ui = transfer::ui(&m);
        ui.names = vec!["one".into(), "two".into()];
        vms.push(super::super::transfer::vm(&m, &ui));
        let m: DialogModel = LinkModel::new(PathBuf::from("d"), None, LinkChoice::Symlink, Vec::new()).into();
        vms.push(super::super::link::model_vm(&m, &link::ui(&m)));
        let (m, _rx) = approval_mcp::tests::model("move", true);
        vms.push(super::super::approval_mcp::vm(&m, &approval_mcp::ui(&m)));
        let m = approval_script::tests::model(20);
        vms.push(super::super::approval_script::vm(&m, &approval_script::ui(&m)));
        for d in &vms {
            assert!(!d.title.is_empty() && !d.buttons.is_empty() && !d.hints.is_empty(), "{d:?}");
            for (w, h) in (0..=20).flat_map(|w| (0..=6).map(move |h| (w, h))).chain([(40, 12), (120, 40)]) {
                draw(d, w, h);
            }
        }
    }
}
