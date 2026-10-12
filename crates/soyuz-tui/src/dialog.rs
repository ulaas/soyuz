//! Fallback for the core's prompts (`soyuz_appcore::models::dialogs::DialogModel`).
//!
//! Phase T1 draws every model the same way (title, body text, key hints) so input can
//! never get stuck behind a prompt the terminal UI has no view for yet; T3 builds the
//! real views in `crate::view::dialogs` / `crate::input::dialogs`, whose stubs call
//! [`fallback`], [`on_key`] and [`stale`]. What each model accepts:
//!
//! | Model | Keys |
//! |---|---|
//! | Delete confirmation | Enter / Y confirm, N / Esc cancel |
//! | Quit with running jobs | K / Esc keep open, W / Enter quit when finished, Q cancel jobs and quit |
//! | Job prompt | Enter the default answer (when it has one), Esc decide later |
//! | Everything else | Esc ([`DialogModel::dismiss`]: rejects an MCP request, cancels the rest) |
//!
//! Letters only count without Ctrl / Alt, so a second Ctrl+Q while the quit question is
//! open never cancels the running jobs.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use soyuz_appcore::models::dialogs::{DialogModel, QuitChoice};
use soyuz_appcore::tr;
use soyuz_jobs::JobPrompt;

pub use crate::input::dialogs::Outcome;
use crate::view::dialogs::DialogVm;

/// Text safe to put on the terminal: control characters made visible (names are
/// untrusted and must not smuggle escape sequences).
pub fn safe(s: &str) -> String {
    soyuz_core::naming::reveal_controls(s).chars().map(|c| if c.is_control() { '\u{fffd}' } else { c }).collect()
}

/// The fallback content for `m` (title, body, key hints).
pub fn fallback(m: &DialogModel) -> DialogVm {
    let title = safe(m.title());
    let (body, hints): (Vec<String>, &str) = match m {
        DialogModel::Prompt(p) => {
            let mut body = vec![safe(&p.job_title)];
            body.extend(prompt_detail(&p.prompt));
            let hints = if p.default_answer().is_some() { tr!("tui-hint-prompt") } else { tr!("tui-hint-close") };
            (body, hints)
        }
        DialogModel::DeleteConfirm(d) => {
            let (msg, _, _) = d.texts();
            let mut body = vec![safe(&msg)];
            let names = d.names();
            body.extend(names.iter().take(5).map(|n| format!("  {}", safe(n))));
            if names.len() > 5 {
                body.push(tr!("dialogs-delete-and-more", count = names.len() - 5));
            }
            (body, tr!("tui-hint-delete"))
        }
        DialogModel::Quit(_) => (vec![tr!("dialogs-quit-body").to_string()], tr!("tui-hint-quit")),
        DialogModel::Transfer(t) => {
            (vec![safe(&t.message()), tr!("tui-dialog-unsupported").to_string()], tr!("tui-hint-cancel"))
        }
        DialogModel::McpApproval(_) => (vec![tr!("tui-dialog-unsupported").to_string()], tr!("tui-hint-reject")),
        DialogModel::NewLink(_) | DialogModel::ScriptApproval(_) => {
            (vec![tr!("tui-dialog-unsupported").to_string()], tr!("tui-hint-cancel"))
        }
    };
    DialogVm { title, body, hints: hints.to_string(), alert: m.alert(), ..DialogVm::default() }
}

/// One line about what a job prompt asks.
fn prompt_detail(p: &JobPrompt) -> Option<String> {
    let path = |p: &std::path::Path| safe(&soyuz_fs::strip_verbatim(p).display().to_string());
    Some(match p {
        JobPrompt::Conflict(c) => format!("{} → {}", safe(&c.src.name), path(&c.dst.path)),
        JobPrompt::Error { path: p, error, .. } => format!("{}: {}", path(p), safe(&error.to_string())),
        JobPrompt::NotEnoughSpace { dest, .. } => path(dest),
        JobPrompt::TrashUnsupported { path: p, reason } => format!("{}: {}", path(p), safe(reason)),
        JobPrompt::ReadOnlyItem { path: p } => path(p),
        JobPrompt::Password { archive, .. } => path(archive),
    })
}

/// The plain letter of a key press without Ctrl / Alt (lower case), if any.
fn letter(k: &KeyEvent) -> Option<char> {
    match k.code {
        KeyCode::Char(c) if !k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
            Some(c.to_ascii_lowercase())
        }
        _ => None,
    }
}

/// Apply a key press to the top-most dialog `m`.
pub fn on_key(m: &mut DialogModel, k: &KeyEvent) -> Outcome {
    if k.kind == KeyEventKind::Release {
        return Outcome::Keep;
    }
    let esc = k.code == KeyCode::Esc;
    let enter = k.code == KeyCode::Enter;
    let ch = letter(k);
    match m {
        DialogModel::DeleteConfirm(d) => {
            if enter || ch == Some('y') {
                Outcome::Close(d.confirm())
            } else if esc || ch == Some('n') {
                d.cancel();
                Outcome::Close(None)
            } else {
                Outcome::Keep
            }
        }
        DialogModel::Quit(q) => {
            let choice = match ch {
                Some('k') => Some(QuitChoice::KeepOpen),
                Some('w') => Some(QuitChoice::QuitWhenFinished),
                Some('q') => Some(QuitChoice::CancelAndQuit),
                _ if esc => Some(QuitChoice::KeepOpen),
                // Enter picks the safe default, like the GUI.
                _ if enter => Some(QuitChoice::QuitWhenFinished),
                _ => None,
            };
            match choice {
                Some(c) => Outcome::Close(q.choose(c)),
                None => Outcome::Keep,
            }
        }
        DialogModel::Prompt(p) => {
            if enter && let Some(a) = p.default_answer() {
                Outcome::Close(Some(p.answer(a)))
            } else if esc {
                // Deferred: the job's activity entry offers it again.
                Outcome::Close(None)
            } else {
                Outcome::Keep
            }
        }
        DialogModel::NewLink(_)
        | DialogModel::Transfer(_)
        | DialogModel::McpApproval(_)
        | DialogModel::ScriptApproval(_) => {
            if esc {
                Outcome::Close(None)
            } else {
                Outcome::Keep
            }
        }
    }
}

/// Models that went away by themselves (a job prompt answered elsewhere, an MCP request
/// withdrawn or timed out) should close without a key.
pub fn stale(m: &mut DialogModel) -> bool {
    match m {
        DialogModel::Prompt(p) => !p.poll(),
        DialogModel::McpApproval(a) => !a.is_pending(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use soyuz_appcore::cmd::Cmd;
    use soyuz_appcore::models::dialogs::{DeleteConfirmModel, QuitModel};
    use soyuz_jobs::DeleteMode;

    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn delete_confirm_keys() {
        let mut m: DialogModel = DeleteConfirmModel::new(vec!["/tmp/a.txt".into()], DeleteMode::Trash).into();
        assert!(matches!(on_key(&mut m, &key(KeyCode::Char('x'))), Outcome::Keep));
        assert!(matches!(on_key(&mut m, &key(KeyCode::Esc)), Outcome::Close(None)));
        let mut m: DialogModel = DeleteConfirmModel::new(vec!["/tmp/a.txt".into()], DeleteMode::Trash).into();
        let v = fallback(&m);
        assert!(v.title.contains("a.txt"), "{v:?}");
        assert!(v.body.iter().any(|l| l.contains("a.txt")), "{v:?}");
        assert_eq!(v.hints, tr!("tui-hint-delete"));
        assert!(matches!(on_key(&mut m, &key(KeyCode::Char('Y'))), Outcome::Close(Some(_))));
    }

    #[test]
    fn quit_keys_ignore_ctrl() {
        let open = Arc::new(AtomicBool::new(true));
        let mut m: DialogModel = QuitModel::new(2, open.clone()).into();
        let ctrl_q = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert!(matches!(on_key(&mut m, &ctrl_q), Outcome::Keep));
        assert!(matches!(on_key(&mut m, &key(KeyCode::Char('k'))), Outcome::Close(None)));
        assert!(matches!(
            on_key(&mut m, &key(KeyCode::Char('w'))),
            Outcome::Close(Some(Cmd::Quit { when_idle: true }))
        ));
        assert!(matches!(
            on_key(&mut m, &key(KeyCode::Char('q'))),
            Outcome::Close(Some(Cmd::Quit { when_idle: false }))
        ));
        assert!(matches!(on_key(&mut m, &key(KeyCode::Enter)), Outcome::Close(Some(Cmd::Quit { when_idle: true }))));
        assert!(matches!(on_key(&mut m, &key(KeyCode::Esc)), Outcome::Close(None)));
        assert_eq!(fallback(&m).hints, tr!("tui-hint-quit"));
    }

    #[test]
    fn safe_hides_escape_sequences() {
        let s = safe("evil\x1b]52;c;x\x07.txt");
        assert!(!s.chars().any(|c| c.is_control()), "{s:?}");
    }
}
