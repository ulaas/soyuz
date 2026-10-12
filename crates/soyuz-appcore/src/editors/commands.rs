//! The editors' bindable commands ([`Action::is_editor`]): bound in
//! [`KeyContext::Editor`] (the GUI defaults in `soyuz_core::config::defaults`, more in the
//! terminal UI's overlay), looked up by [`super::handle_keys`] while a document has the
//! keyboard, and run from the palette on the active document.
//!
//! Most of them change the core's document model; two need the frontend:
//! `EditorSaveAs` and `EditorResizeImage` leave a request in [`super::Editors::request`]
//! for the frontend's own bars (the terminal UI), and `EditorToggleWrap` /
//! `EditorToggleDock` change `config.toml` (they need the host, so [`super::handle_keys`]
//! hands them back as actions).

use soyuz_core::config::{Action, EditorDock, KeyChord, KeyContext, Keymap};

use super::{DocKind, External};
use crate::core::AppCore;
use crate::host::Host;
use crate::models::toasts::Toast;

/// The editor command bound to `chord` in [`KeyContext::Editor`] (no fallback to the
/// global bindings).
pub fn binding(km: &Keymap, chord: KeyChord) -> Option<Action> {
    km.bindings
        .iter()
        .find(|((ctx, a), chords)| *ctx == KeyContext::Editor && a.is_editor() && chords.contains(&chord))
        .map(|((_, a), _)| a.clone())
}

/// Whether `a` changes the config (it needs the host: [`execute`]).
pub fn needs_host(a: &Action) -> bool {
    matches!(a, Action::EditorToggleWrap | Action::EditorToggleDock)
}

/// Run editor command `a` (from the palette or a key).
pub fn execute(app: &mut AppCore, a: Action, host: &dyn Host) {
    match a {
        Action::EditorToggleWrap => {
            crate::models::settings::update_config(app, host, |c| c.editor.wrap = !c.editor.wrap);
        }
        Action::EditorToggleDock => {
            let next = match app.config.editor.dock {
                EditorDock::Right => EditorDock::Bottom,
                EditorDock::Bottom => EditorDock::Right,
            };
            crate::models::settings::update_config(app, host, |c| c.editor.dock = next);
        }
        other => run(app, &other),
    }
}

/// Run an editor command that needs no host (see [`needs_host`]).
pub fn run(app: &mut AppCore, a: &Action) {
    let idx = app.editors.active;
    match a {
        Action::EditorSave => super::save(app, idx),
        Action::EditorCloseDocument => super::close(app, idx, false),
        Action::EditorNextDocument | Action::EditorPrevDocument => {
            let n = app.editors.docs.len();
            if n > 0 {
                let step = if *a == Action::EditorNextDocument { 1 } else { n - 1 };
                app.editors.active = (app.editors.active + step) % n;
                if let Some(d) = app.editors.active_doc_mut() {
                    d.request_focus();
                }
            }
        }
        Action::EditorMaximize => {
            if app.editors.open {
                app.editors.maximized = !app.editors.maximized;
            }
        }
        Action::EditorSaveAs | Action::EditorResizeImage => {
            if app.editors.active_doc().is_some() {
                app.editors.request = Some(a.clone());
            }
        }
        Action::EditorNoticeAction => notice_action(app, idx),
        Action::EditorReplaceAll => {
            let Some(DocKind::Text(t)) = app.editors.docs.get_mut(idx).map(|d| &mut d.kind) else { return };
            if t.find.open && !t.read_only {
                let n = t.replace_all();
                app.ui.toasts.push_replacing(Toast::info(tr!("tui-editor-replaced", count = n)));
            }
        }
        _ => {
            let Some(d) = app.editors.docs.get_mut(idx) else { return };
            match (&mut d.kind, a) {
                (DocKind::Text(t), Action::EditorFind) => t.open_find(false),
                (DocKind::Text(t), Action::EditorReplace) => t.open_find(true),
                (DocKind::Text(t), Action::EditorGoTo) => t.open_goto(),
                (DocKind::Text(t), Action::EditorFindNext) => t.find_step(true),
                (DocKind::Text(t), Action::EditorFindPrev) => t.find_step(false),
                (DocKind::Text(t), Action::EditorMatchCase | Action::EditorWholeWord | Action::EditorRegex) => {
                    if !t.find.open {
                        return;
                    }
                    let q = &mut t.find.query;
                    match a {
                        Action::EditorMatchCase => q.case = !q.case,
                        Action::EditorWholeWord => q.whole_word = !q.whole_word,
                        _ => q.regex = !q.regex,
                    }
                    t.find_incremental();
                }
                (DocKind::Hex(h), Action::EditorFind) => h.open_find(),
                (DocKind::Hex(h), Action::EditorGoTo) => h.open_goto(),
                (DocKind::Hex(h), Action::EditorFindNext) => h.find_step(true),
                (DocKind::Hex(h), Action::EditorFindPrev) => h.find_step(false),
                _ => {}
            }
        }
    }
}

/// The notice's action: the external-change banner first (reload a changed file, OK on a
/// deleted one), then the document's own notice (edit a lossy text anyway, OK on mixed
/// line endings, allow editing a hex file too large for text).
fn notice_action(app: &mut AppCore, idx: usize) {
    let Some(d) = app.editors.docs.get_mut(idx) else { return };
    match d.external {
        Some(External::Changed) => return super::reload(app, idx),
        Some(External::Deleted) => {
            d.external = None;
            return;
        }
        None => {}
    }
    match &mut d.kind {
        DocKind::Text(t) if t.notice().is_some() => t.dismiss_notice(),
        DocKind::Hex(h) if h.too_large_for_text && h.read_only => h.read_only = false,
        _ => {}
    }
}

/// `FocusEditor`: give the keyboard back to the active document (showing the panel).
pub fn focus(app: &mut AppCore) {
    if let Some(d) = app.editors.active_doc_mut() {
        d.request_focus();
        app.editors.open = true;
    } else {
        app.ui.toasts.push(Toast::info(tr!("editors-empty")));
    }
}

#[cfg(test)]
mod tests {
    use soyuz_core::config::{Config, KeyName, Mods, keymap};

    use super::*;

    #[test]
    fn editor_bindings_only() {
        let (km, _) = keymap(&Config::default());
        let ch = |m: Mods, k: KeyName| KeyChord::new(m, k);
        assert_eq!(binding(&km, ch(Mods::CTRL, KeyName::F)), Some(Action::EditorFind));
        assert_eq!(binding(&km, ch(Mods::CTRL, KeyName::H)), Some(Action::EditorReplace));
        assert_eq!(binding(&km, ch(Mods::empty(), KeyName::F3)), Some(Action::EditorFindNext));
        assert_eq!(binding(&km, ch(Mods::CTRL, KeyName::PageDown)), Some(Action::EditorNextDocument));
        // No fallback to the global map: Ctrl+K is the palette there.
        assert_eq!(binding(&km, ch(Mods::CTRL, KeyName::K)), None);
        // A rebind moves the command.
        let mut c = Config::default();
        c.keys.bindings.insert("editor_replace".into(), vec!["Alt+H".into()]);
        let (km, w) = keymap(&c);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(binding(&km, ch(Mods::ALT, KeyName::H)), Some(Action::EditorReplace));
        assert_eq!(binding(&km, ch(Mods::CTRL, KeyName::H)), None);
        // An editor command without a default binding lands in the editor context.
        let mut c = Config::default();
        c.keys.bindings.insert("editor_toggle_wrap".into(), vec!["Alt+Q".into()]);
        let (km, _) = keymap(&c);
        assert_eq!(binding(&km, ch(Mods::ALT, KeyName::Q)), Some(Action::EditorToggleWrap));
    }
}
