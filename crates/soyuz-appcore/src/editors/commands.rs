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
//!
//! The PDF viewer's commands ([`Action::is_pdf`], bound in [`KeyContext::Pdf`]) run on
//! the active PDF document, which also takes the editors' find / go-to / document
//! commands ([`pdf_reuses`]). Its copy goes through `EditInput::Copy` (the frontend puts
//! the returned text on the clipboard), like the text editor's.

use soyuz_core::config::{Action, EditorDock, KeyChord, KeyContext, Keymap, Mods};

use super::{DocKind, External};
use crate::core::AppCore;
use crate::host::Host;
use crate::models::toasts::Toast;

/// Which bindings the active document's keys look up ([`binding`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BindDoc {
    /// Text, hex and image documents: [`KeyContext::Editor`].
    #[default]
    Editor,
    /// A PDF document: [`KeyContext::Pdf`] first, then the editor commands it reuses.
    Pdf,
}

impl BindDoc {
    pub fn of(kind: Option<&DocKind>) -> BindDoc {
        match kind {
            Some(DocKind::Pdf(_)) => BindDoc::Pdf,
            _ => BindDoc::Editor,
        }
    }
}

/// The editors' commands a PDF document runs too.
pub fn pdf_reuses(a: &Action) -> bool {
    matches!(
        a,
        Action::EditorFind
            | Action::EditorFindNext
            | Action::EditorFindPrev
            | Action::EditorGoTo
            | Action::EditorMatchCase
            | Action::EditorWholeWord
            | Action::EditorCloseDocument
            | Action::EditorNextDocument
            | Action::EditorPrevDocument
            | Action::EditorMaximize
            | Action::EditorToggleDock
            | Action::EditorNoticeAction
    )
}

/// The command bound to `chord` for the active document (no fallback to the global
/// bindings): [`KeyContext::Editor`] for editor documents; for a PDF document
/// [`KeyContext::Pdf`] first, then the editor commands it reuses. While a text field of
/// the document has the keyboard (`field_focused`: find, page number), viewer chords
/// without Ctrl / Cmd / Alt are skipped so they type.
pub fn binding(km: &Keymap, chord: KeyChord, doc: BindDoc, field_focused: bool) -> Option<Action> {
    let find = |ctx: KeyContext, want: &dyn Fn(&Action) -> bool| {
        km.bindings
            .iter()
            .find(|((c, a), chords)| *c == ctx && want(a) && chords.contains(&chord))
            .map(|((_, a), _)| a.clone())
    };
    match doc {
        BindDoc::Editor => find(KeyContext::Editor, &Action::is_editor),
        BindDoc::Pdf => {
            let typing = field_focused && !chord.mods.intersects(Mods::CTRL | Mods::CMD | Mods::ALT);
            let pdf = if typing { None } else { find(KeyContext::Pdf, &Action::is_pdf) };
            pdf.or_else(|| find(KeyContext::Editor, &|a| a.is_editor() && pdf_reuses(a)))
        }
    }
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
        a if a.is_pdf() => match app.editors.docs.get_mut(idx).map(|d| &mut d.kind) {
            Some(DocKind::Pdf(p)) => {
                p.command(a);
            }
            _ => {
                app.ui.toasts.push(Toast::info(tr!("pdf-no-document")));
            }
        },
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
                (DocKind::Pdf(p), a) => {
                    p.command(a);
                }
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
    if !app.editors.docs.is_empty() {
        // Into the window asking for it.
        crate::models::owners::claim(app, crate::models::owners::Panel::Editors);
    }
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

    fn binding(km: &Keymap, chord: KeyChord) -> Option<Action> {
        super::binding(km, chord, BindDoc::Editor, false)
    }

    #[test]
    fn pdf_documents_look_up_the_viewer_first() {
        let (km, _) = keymap(&Config::default());
        let ch = |m: Mods, k: KeyName| KeyChord::new(m, k);
        let pdf = |c: KeyChord, field: bool| super::binding(&km, c, BindDoc::Pdf, field);
        // R rotates a PDF; on a text document it types.
        assert_eq!(pdf(ch(Mods::empty(), KeyName::R), false), Some(Action::PdfRotateCw));
        assert_eq!(pdf(ch(Mods::SHIFT, KeyName::R), false), Some(Action::PdfRotateCcw));
        assert_eq!(binding(&km, ch(Mods::empty(), KeyName::R)), None);
        // In the find field the letter types; chords with Ctrl still work.
        assert_eq!(pdf(ch(Mods::empty(), KeyName::R), true), None);
        assert_eq!(pdf(ch(Mods::SHIFT, KeyName::R), true), None);
        assert_eq!(pdf(ch(Mods::CTRL, KeyName::Home), true), Some(Action::PdfFirstPage));
        // The reused editor commands.
        assert_eq!(pdf(ch(Mods::CTRL, KeyName::F), false), Some(Action::EditorFind));
        assert_eq!(pdf(ch(Mods::empty(), KeyName::F3), true), Some(Action::EditorFindNext));
        assert_eq!(pdf(ch(Mods::CTRL, KeyName::PageDown), false), Some(Action::EditorNextDocument));
        // Not the ones that edit or save.
        assert_eq!(pdf(ch(Mods::CTRL, KeyName::H), false), None, "replace");
        assert_eq!(pdf(ch(Mods::CTRL, KeyName::S), false), None, "save");
        // Ctrl+= stays the global UI zoom.
        assert_eq!(pdf(ch(Mods::CTRL, KeyName::Equals), false), None);
        assert!(pdf_reuses(&Action::EditorGoTo) && !pdf_reuses(&Action::EditorSave));
        assert_eq!(BindDoc::of(None), BindDoc::Editor);
    }
}
