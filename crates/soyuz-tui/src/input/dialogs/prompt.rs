//! Job prompts (`DialogModel::Prompt`): file conflicts (Replace / Keep both / Skip /
//! More… / Rename… with apply-to-all), errors (Retry / Skip / Skip all / Abort), the
//! archive size guard, not enough space, trash unsupported, read-only items and archive
//! passwords, like the GUI's `ui/dialogs/conflict.rs` and `error.rs`.
//!
//! The answers are the core's `PromptModel` methods; this file keeps the terminal UI's
//! part: the [`Form`] (built from the model by [`form`]) and the page of a conflict:
//! `0` the buttons, `1` the "More…" options as a list (the GUI's popup menu). Keys:
//! Tab / arrows move, Enter presses the focused or the default button, letters press
//! buttons, Esc decides later (the job's activity entry offers the prompt again), or
//! goes back from "More…" and from typing a new name.
//!
//! The second part of the file has the helpers every core prompt kind shares
//! ([`buttons`], [`click`], [`list_key`], …).

use crossterm::event::{MouseButton, MouseEventKind};
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::dialogs::{DialogModel, PromptAnswer, PromptModel};
use soyuz_appcore::tr;
use soyuz_core::config::KeyName;
use soyuz_core::{ConflictResolution, ErrorAction};
use soyuz_jobs::kinds::extract::ARCHIVE_LIMIT_OP;
use soyuz_jobs::{ConflictKind, JobPrompt};

use super::{DialogInput, DialogMouse, ModelUi, Outcome};
use crate::layout::HitTarget;
use crate::widgets::{Button, Field, FieldKind, Form, FormOutcome, SelectList};

/// The page with the conflict's "More…" options.
pub const MORE: usize = 1;

/// What a button or a "More…" option does.
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    Resolve(ConflictResolution),
    /// Show the "More…" options.
    More,
    /// Back from "More…" or from typing a new name.
    Back,
    StartRename,
    CommitRename,
    CancelJob,
    Error(ErrorAction),
    Space(bool),
    Trash(bool),
    ReadOnly(bool),
    Unlock,
    NoPassword,
}

/// The fields, buttons (left to right, the main one last) and focus of a prompt.
struct Layout {
    fields: Vec<Field>,
    buttons: Vec<(String, Choice)>,
    default: Option<usize>,
    /// The first focus stop (fields, then buttons).
    focus: usize,
}

fn conflict_kind(p: &PromptModel) -> Option<ConflictKind> {
    match &p.prompt {
        JobPrompt::Conflict(c) => Some(c.kind),
        _ => None,
    }
}

/// The "More…" options of a conflict (the GUI's popup menu).
pub fn options(p: &PromptModel) -> Vec<(String, Choice)> {
    let s = |k: &str| k.to_string();
    match conflict_kind(p) {
        Some(ConflictKind::DirExists) => vec![
            (s(tr!("dialogs-conflict-replace-folder")), Choice::Resolve(ConflictResolution::Overwrite)),
            (s(tr!("dialogs-conflict-rename")), Choice::StartRename),
        ],
        Some(_) => vec![
            (s(tr!("dialogs-conflict-replace-newer")), Choice::Resolve(ConflictResolution::OverwriteIfNewer)),
            (s(tr!("dialogs-conflict-replace-larger")), Choice::Resolve(ConflictResolution::OverwriteIfLarger)),
            (s(tr!("dialogs-conflict-replace-different")), Choice::Resolve(ConflictResolution::OverwriteIfDifferent)),
            (s(tr!("dialogs-conflict-rename")), Choice::StartRename),
        ],
        None => Vec::new(),
    }
}

fn layout(p: &PromptModel, page: usize) -> Layout {
    let s = |k: &str| k.to_string();
    let cancel_op = || (s(tr!("dialogs-prompt-cancel-op")), Choice::CancelJob);
    let mut fields = Vec::new();
    let (buttons, default): (Vec<(String, Choice)>, Option<usize>) = match &p.prompt {
        JobPrompt::Conflict(c) => {
            let blocked = c.kind == ConflictKind::MoveIntoSelf;
            if let Some(name) = &p.rename {
                fields.push(Field::text(tr!("dialogs-conflict-new-name"), name.clone()));
            }
            if !blocked {
                fields.push(Field::check(tr!("dialogs-conflict-apply-all"), p.apply_all));
            }
            if p.rename.is_some() {
                let b = vec![
                    cancel_op(),
                    (s(tr!("dialogs-conflict-back")), Choice::Back),
                    (s(tr!("common-rename")), Choice::CommitRename),
                ];
                (b, Some(2))
            } else if page == MORE {
                (vec![(s(tr!("dialogs-conflict-back")), Choice::Back)], None)
            } else {
                match c.kind {
                    ConflictKind::MoveIntoSelf => {
                        (vec![cancel_op(), (s(tr!("common-skip")), Choice::Resolve(ConflictResolution::Skip))], None)
                    }
                    ConflictKind::SameFile => (
                        vec![
                            cancel_op(),
                            (s(tr!("common-skip")), Choice::Resolve(ConflictResolution::Skip)),
                            (s(tr!("dialogs-conflict-keep-both")), Choice::Resolve(ConflictResolution::KeepBoth)),
                        ],
                        None,
                    ),
                    kind => {
                        let primary = if kind == ConflictKind::DirExists {
                            (s(tr!("dialogs-conflict-merge")), Choice::Resolve(ConflictResolution::MergeDir))
                        } else {
                            (s(tr!("dialogs-conflict-replace")), Choice::Resolve(ConflictResolution::Overwrite))
                        };
                        let b = vec![
                            cancel_op(),
                            (s(tr!("dialogs-conflict-more")), Choice::More),
                            (s(tr!("common-skip")), Choice::Resolve(ConflictResolution::Skip)),
                            (s(tr!("dialogs-conflict-keep-both")), Choice::Resolve(ConflictResolution::KeepBoth)),
                            primary,
                        ];
                        // Enter replaces a plain file (the core's default answer).
                        (b, p.default_answer().is_some().then_some(4))
                    }
                }
            }
        }
        JobPrompt::Error { error, .. } if error.op == ARCHIVE_LIMIT_OP => (
            vec![
                (s(tr!("dialogs-prompt-continue")), Choice::Error(ErrorAction::Skip)),
                (s(tr!("dialogs-prompt-abort")), Choice::Error(ErrorAction::Abort)),
            ],
            Some(1),
        ),
        JobPrompt::Error { retryable, .. } => {
            let mut b = vec![
                (s(tr!("dialogs-prompt-abort")), Choice::Error(ErrorAction::Abort)),
                (s(tr!("dialogs-prompt-skip-all")), Choice::Error(ErrorAction::SkipAll)),
                (s(tr!("common-skip")), Choice::Error(ErrorAction::Skip)),
            ];
            if *retryable {
                b.push((s(tr!("common-retry")), Choice::Error(ErrorAction::Retry)));
            }
            let d = b.len() - 1;
            (b, Some(d))
        }
        JobPrompt::NotEnoughSpace { .. } => (
            vec![
                (s(tr!("dialogs-prompt-cancel-op")), Choice::Space(false)),
                (s(tr!("dialogs-prompt-continue-anyway")), Choice::Space(true)),
            ],
            None,
        ),
        JobPrompt::TrashUnsupported { .. } => {
            fields.push(Field::check(tr!("dialogs-prompt-apply-all-items"), p.apply_all));
            (
                vec![
                    cancel_op(),
                    (s(tr!("common-skip")), Choice::Trash(false)),
                    (s(tr!("dialogs-prompt-delete-permanently")), Choice::Trash(true)),
                ],
                None,
            )
        }
        JobPrompt::ReadOnlyItem { .. } => {
            fields.push(Field::check(tr!("dialogs-prompt-apply-all-read-only"), p.apply_all));
            (
                vec![
                    cancel_op(),
                    (s(tr!("common-skip")), Choice::ReadOnly(false)),
                    (s(tr!("dialogs-prompt-change-anyway")), Choice::ReadOnly(true)),
                ],
                None,
            )
        }
        JobPrompt::Password { .. } => {
            let mut f = Field::secret(tr!("dialogs-prompt-password"));
            if let FieldKind::Text { text, edit, .. } = &mut f.kind {
                text.clone_from(&p.password);
                *edit = crate::widgets::LineEdit::at_end(text);
            }
            fields.push(f);
            (
                vec![(s(tr!("common-cancel")), Choice::NoPassword), (s(tr!("dialogs-prompt-unlock")), Choice::Unlock)],
                Some(1),
            )
        }
    };
    // The first stop: a text field (new name, password), else the default button, else
    // the first field (apply to all: Enter then answers nothing, like the GUI), else the
    // first button (the safe "Cancel operation" of "not enough space").
    let text_field = fields.iter().position(|f| matches!(f.kind, FieldKind::Text { .. }));
    let focus = match (text_field, default) {
        (Some(t), _) => t,
        (None, Some(d)) => fields.len() + d,
        (None, None) if !fields.is_empty() => 0,
        (None, None) if conflict_kind(p).is_some() => fields.len() + buttons.len() - 1,
        (None, None) => fields.len(),
    };
    Layout { fields, buttons, default, focus }
}

/// The form of `p` on `page`.
pub fn form(p: &PromptModel, page: usize) -> Form {
    let l = layout(p, page);
    let labels: Vec<&str> = l.buttons.iter().map(|(s, _)| s.as_str()).collect();
    let mut f = Form::new(l.fields, buttons(&labels));
    f.default_button = l.default;
    f.focus = l.focus;
    refresh(p, &mut f, &l.buttons);
    f
}

/// Enable "Rename" / "Unlock" and show why a typed name is invalid.
fn refresh(p: &PromptModel, f: &mut Form, choices: &[(String, Choice)]) {
    for (b, (_, c)) in f.buttons.iter_mut().zip(choices) {
        b.enabled = match c {
            Choice::CommitRename => p.rename_differs(),
            Choice::Unlock => !p.password.is_empty(),
            _ => true,
        };
    }
    f.error = p.rename.as_ref().and_then(|t| {
        soyuz_core::validate_name_with(t.trim(), p.windows_rules)
            .err()
            .map(|e| tr!("dialogs-props-invalid-name", error = e.to_string()))
    });
}

/// The terminal UI's part of a prompt that just reached the top (focus, cursors).
pub fn ui(m: &DialogModel) -> ModelUi {
    match m {
        DialogModel::Prompt(p) => ModelUi { form: form(p, 0), ..ModelUi::default() },
        _ => ModelUi::default(),
    }
}

/// Copy the form's fields into the model (new name, apply to all, password).
fn sync(p: &mut PromptModel, f: &Form) {
    for field in &f.fields {
        match &field.kind {
            FieldKind::Text { text, secret: true, .. } => p.password.clone_from(text),
            FieldKind::Text { text, .. } => p.rename = Some(text.clone()),
            FieldKind::Check(on) => p.apply_all = *on,
            FieldKind::Radio { .. } => {}
        }
    }
}

fn rebuild(p: &PromptModel, ui: &mut ModelUi) {
    ui.form = form(p, ui.page);
    ui.list = SelectList::default();
}

/// Carry out `c`; `Some` closes the dialog.
fn act(p: &mut PromptModel, ui: &mut ModelUi, c: Choice) -> Option<Outcome> {
    let answer = |p: &mut PromptModel, a: PromptAnswer| Some(Outcome::Close(Some(p.answer(a))));
    match c {
        Choice::Resolve(r) => {
            let a = p.resolve(r);
            answer(p, a)
        }
        Choice::More => {
            ui.page = MORE;
            rebuild(p, ui);
            None
        }
        Choice::Back => {
            p.cancel_rename();
            ui.page = 0;
            rebuild(p, ui);
            None
        }
        Choice::StartRename => {
            p.start_rename();
            // The field takes the focus here (`form`), not through the GUI's flag.
            p.focus_rename = false;
            ui.page = 0;
            rebuild(p, ui);
            None
        }
        Choice::CommitRename => match p.commit_rename() {
            Some(a) if p.rename_differs() => answer(p, a),
            _ => None,
        },
        Choice::CancelJob => answer(p, PromptAnswer::CancelJob),
        Choice::Error(e) => {
            let a = p.error(e);
            answer(p, a)
        }
        Choice::Space(go) => {
            let a = p.space(go);
            answer(p, a)
        }
        Choice::Trash(permanent) => {
            let a = p.trash_unsupported(permanent);
            answer(p, a)
        }
        Choice::ReadOnly(force) => {
            let a = p.read_only(force);
            answer(p, a)
        }
        Choice::Unlock => match p.submit_password() {
            Some(a) => answer(p, a),
            None => None,
        },
        Choice::NoPassword => {
            let a = p.cancel_password();
            answer(p, a)
        }
    }
}

/// What a form outcome does to the prompt; `Some` closes it.
fn on_form(p: &mut PromptModel, ui: &mut ModelUi, out: FormOutcome) -> Option<Outcome> {
    match out {
        FormOutcome::Cancel if ui.page == MORE || p.rename.is_some() => act(p, ui, Choice::Back),
        // Decide later: `DialogModel::dismiss` defers the prompt.
        FormOutcome::Cancel => Some(Outcome::Close(None)),
        FormOutcome::Changed(_) => {
            sync(p, &ui.form);
            let l = layout(p, ui.page);
            refresh(p, &mut ui.form, &l.buttons);
            None
        }
        FormOutcome::Pressed(b) => {
            let c = layout(p, ui.page).buttons.get(b)?.1.clone();
            act(p, ui, c)
        }
        FormOutcome::Consumed | FormOutcome::Ignored => None,
    }
}

/// A key press or paste.
pub fn on_input(m: &mut DialogModel, ui: &mut ModelUi, i: &DialogInput) -> Outcome {
    let DialogModel::Prompt(p) = m else { return Outcome::Keep };
    for ev in &i.events {
        if ui.page == MORE && p.rename.is_none() {
            let opts = options(p);
            // Enter picks the option under the cursor, unless "Back" has the focus.
            if is_key(ev, KeyName::Enter) && ui.form.focused_button().is_none() {
                if let Some((_, c)) = opts.get(ui.list.cursor).cloned()
                    && let Some(out) = act(p, ui, c)
                {
                    return out;
                }
                continue;
            }
            if list_key(&mut ui.list, ev, opts.len()) {
                continue;
            }
        }
        let out = ui.form.apply(ev);
        if let Some(o) = on_form(p, ui, out) {
            return o;
        }
    }
    Outcome::Keep
}

/// A mouse event: buttons, fields, the "More…" options (click picks, the wheel scrolls).
pub fn on_mouse(m: &mut DialogModel, ui: &mut ModelUi, e: &DialogMouse) -> Outcome {
    let DialogModel::Prompt(p) = m else { return Outcome::Keep };
    if ui.page == MORE && p.rename.is_none() {
        let opts = options(p);
        if let (true, Some(HitTarget::DialogRow(r))) = (left_click(e), e.target) {
            ui.list.select(r, opts.len());
            if let Some((_, c)) = opts.get(r).cloned() {
                return act(p, ui, c).unwrap_or(Outcome::Keep);
            }
            return Outcome::Keep;
        }
        if wheel(&mut ui.list, e, opts.len()) {
            return Outcome::Keep;
        }
    }
    let out = click(&mut ui.form, e);
    on_form(p, ui, out).unwrap_or(Outcome::Keep)
}

/// The prompt went away by itself (the job no longer waits).
pub fn stale(m: &mut DialogModel) -> bool {
    crate::dialog::stale(m)
}

// ---- shared by the core prompt kinds ----------------------------------------------------

/// Rows a list key pages by (the dialog shows at most this many).
pub const LIST_PAGE: usize = crate::view::dialogs::MAX_LIST_ROWS as usize;
/// Rows the mouse wheel scrolls a dialog list by.
pub const WHEEL_ROWS: i32 = 3;

/// Buttons for `labels` (left to right) with letter accelerators: each takes the first
/// ASCII letter of its label that no button to its right took (the main, right-most
/// button picks first), in any language.
pub fn buttons(labels: &[&str]) -> Vec<Button> {
    let mut out: Vec<Button> = labels.iter().map(|l| Button::new(*l)).collect();
    let mut taken: Vec<char> = Vec::new();
    for b in out.iter_mut().rev() {
        let free =
            b.label.chars().map(|c| c.to_ascii_lowercase()).find(|c| c.is_ascii_lowercase() && !taken.contains(c));
        if let Some(c) = free {
            taken.push(c);
            b.accel = Some(c);
        }
    }
    out
}

/// `ev` is `key` without modifiers.
pub fn is_key(ev: &InputEvent, key: KeyName) -> bool {
    matches!(ev, InputEvent::Chord(c) if c.key == key && c.mods.is_empty())
}

/// Up / Down / PageUp / PageDown / Home / End on a dialog list of `len` rows.
pub fn list_key(list: &mut SelectList, ev: &InputEvent, len: usize) -> bool {
    let nav = [KeyName::ArrowUp, KeyName::ArrowDown, KeyName::PageUp, KeyName::PageDown, KeyName::Home, KeyName::End];
    if len == 0 || !nav.iter().any(|k| is_key(ev, *k)) {
        return false;
    }
    list.apply(ev, len, LIST_PAGE)
}

/// A left-button press.
pub fn left_click(e: &DialogMouse) -> bool {
    e.event.kind == MouseEventKind::Down(MouseButton::Left)
}

/// The wheel over a dialog: scroll its list of `len` rows (the cursor follows, as the
/// view keeps it in sight).
pub fn wheel(list: &mut SelectList, e: &DialogMouse, len: usize) -> bool {
    let rows = match e.event.kind {
        MouseEventKind::ScrollDown => WHEEL_ROWS,
        MouseEventKind::ScrollUp => -WHEEL_ROWS,
        _ => return false,
    };
    if len == 0 {
        return false;
    }
    let c = list.cursor as i64 + i64::from(rows);
    list.select(c.max(0) as usize, len);
    true
}

/// A left click on a button (presses it) or a field (focuses it; a checkbox toggles, a
/// radio group that has the focus picks its next option).
pub fn click(form: &mut Form, e: &DialogMouse) -> FormOutcome {
    if !left_click(e) {
        return FormOutcome::Ignored;
    }
    match e.target {
        Some(HitTarget::DialogButton(b)) => form.click_button(b),
        Some(HitTarget::DialogField(i)) => {
            let focused = form.focused_field() == Some(i);
            if let Some(Field { kind: FieldKind::Radio { options, selected }, enabled: true, .. }) =
                form.fields.get_mut(i)
                && focused
            {
                *selected = (*selected + 1) % options.len().max(1);
                return FormOutcome::Changed(i);
            }
            form.click_field(i)
        }
        _ => FormOutcome::Ignored,
    }
}

/// Put `text` into text field `i`, the cursor at its end.
pub fn set_text(form: &mut Form, i: usize, text: &str) {
    if let Some(FieldKind::Text { text: t, edit, .. }) = form.fields.get_mut(i).map(|f| &mut f.kind) {
        *t = text.to_string();
        *edit = crate::widgets::LineEdit::at_end(t);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
    use soyuz_appcore::cmd::Cmd;
    use soyuz_appcore::models::dialogs::PromptShared;
    use soyuz_appcore::ops::fileops::FileOpCmd;
    use soyuz_core::{EntryInfo, EntryKind, FsError, FsErrorKind};
    use soyuz_jobs::{ConflictInfo, JobId, PromptResponse};

    use super::*;

    fn chord(s: &str) -> DialogInput {
        DialogInput { key: Some(KeyEvent::new(KeyCode::Null, KeyModifiers::NONE)), events: vec![ev(s)] }
    }

    fn ev(s: &str) -> InputEvent {
        InputEvent::Chord(s.parse().unwrap())
    }

    fn text(s: &str) -> DialogInput {
        DialogInput { key: None, events: vec![InputEvent::Text(s.into())] }
    }

    fn model(prompt: JobPrompt) -> DialogModel {
        let shared = Rc::new(RefCell::new(PromptShared { alive: true, total: 1, ..Default::default() }));
        PromptModel::new(JobId(3), prompt, "Copy".into(), shared).into()
    }

    fn conflict(kind: ConflictKind) -> JobPrompt {
        JobPrompt::Conflict(ConflictInfo {
            src: EntryInfo::minimal(PathBuf::from("src").join("a.txt"), EntryKind::File),
            dst: EntryInfo::minimal(PathBuf::from("dst").join("a.txt"), EntryKind::File),
            kind,
            suggested_name: "a (2).txt".into(),
        })
    }

    /// The response a closing outcome carries, as text (`PromptResponse` has no `PartialEq`).
    fn response(out: Outcome) -> String {
        match out {
            Outcome::Close(Some(Cmd::FileOp(FileOpCmd::Respond { response, .. }))) => format!("{response:?}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    fn r(p: PromptResponse) -> String {
        format!("{p:?}")
    }

    fn resolved(resolution: ConflictResolution, apply_to_all: bool) -> String {
        r(PromptResponse::Conflict { resolution, apply_to_all })
    }

    fn mouse(kind: MouseEventKind, target: Option<HitTarget>) -> DialogMouse {
        DialogMouse { event: MouseEvent { kind, column: 0, row: 0, modifiers: KeyModifiers::NONE }, target }
    }

    fn left(target: HitTarget) -> DialogMouse {
        mouse(MouseEventKind::Down(MouseButton::Left), Some(target))
    }

    #[test]
    fn accelerators_take_free_letters_main_button_first() {
        let b = buttons(&["Cancel operation", "More…", "Skip", "Keep both", "Replace"]);
        let a: Vec<Option<char>> = b.iter().map(|b| b.accel).collect();
        assert_eq!(a, [Some('c'), Some('m'), Some('s'), Some('k'), Some('r')]);
        let b = buttons(&["Sil", "Sil", "Şş"]);
        assert_eq!(b.iter().map(|b| b.accel).collect::<Vec<_>>(), [Some('i'), Some('s'), None]);
    }

    #[test]
    fn conflict_keep_both_for_all() {
        let mut m = model(conflict(ConflictKind::FileExists));
        let mut u = ui(&m);
        assert_eq!(u.form.focused_button(), Some(4), "the focus on Replace");
        // Tab to the checkbox (wrapping), Space ticks it, K keeps both.
        assert!(matches!(on_input(&mut m, &mut u, &chord("Tab")), Outcome::Keep));
        assert_eq!(u.form.focused_field(), Some(0));
        on_input(&mut m, &mut u, &chord("Space"));
        assert!(matches!(&m, DialogModel::Prompt(p) if p.apply_all));
        let got = response(on_input(&mut m, &mut u, &chord("K")));
        assert_eq!(got, resolved(ConflictResolution::KeepBoth, true));
    }

    #[test]
    fn conflict_enter_replaces_and_esc_defers() {
        let mut m = model(conflict(ConflictKind::FileExists));
        let mut u = ui(&m);
        let got = response(on_input(&mut m, &mut u, &chord("Enter")));
        assert_eq!(got, resolved(ConflictResolution::Overwrite, false));
        let mut m = model(conflict(ConflictKind::FileExists));
        let mut u = ui(&m);
        assert!(matches!(on_input(&mut m, &mut u, &chord("Esc")), Outcome::Close(None)));
    }

    #[test]
    fn conflict_more_options_and_rename() {
        let mut m = model(conflict(ConflictKind::FileExists));
        let mut u = ui(&m);
        on_input(&mut m, &mut u, &chord("M"));
        assert_eq!(u.page, MORE);
        // Esc goes back, M again, Down to "Replace if larger", Enter picks it.
        on_input(&mut m, &mut u, &chord("Esc"));
        assert_eq!(u.page, 0);
        on_input(&mut m, &mut u, &chord("M"));
        on_input(&mut m, &mut u, &chord("Down"));
        let got = response(on_input(&mut m, &mut u, &chord("Enter")));
        assert_eq!(got, resolved(ConflictResolution::OverwriteIfLarger, false));

        // Rename…: the last option; the field holds the suggested name.
        let mut m = model(conflict(ConflictKind::FileExists));
        let mut u = ui(&m);
        on_input(&mut m, &mut u, &chord("M"));
        on_input(&mut m, &mut u, &chord("End"));
        assert!(matches!(on_input(&mut m, &mut u, &chord("Enter")), Outcome::Keep));
        assert_eq!(u.form.fields[0].value(), Some("a (2).txt"));
        assert_eq!(u.form.focused_field(), Some(0));
        // An empty name disables Rename and says why.
        for _ in 0.."a (2).txt".len() {
            on_input(&mut m, &mut u, &chord("Backspace"));
        }
        assert!(u.form.error.is_some());
        assert!(matches!(on_input(&mut m, &mut u, &chord("Enter")), Outcome::Keep));
        on_input(&mut m, &mut u, &text("b.txt"));
        assert!(u.form.error.is_none());
        let got = response(on_input(&mut m, &mut u, &chord("Enter")));
        assert_eq!(got, resolved(ConflictResolution::Rename("b.txt".into()), false));
    }

    #[test]
    fn rename_esc_goes_back() {
        let mut m = model(conflict(ConflictKind::DirExists));
        let mut u = ui(&m);
        assert_eq!(u.form.focused_field(), Some(0), "no default answer: the focus on apply to all");
        assert!(matches!(on_input(&mut m, &mut u, &chord("Enter")), Outcome::Keep), "Enter answers nothing");
        // Merge took M: "More…" has O.
        on_input(&mut m, &mut u, &chord("O"));
        assert!(matches!(&m, DialogModel::Prompt(p) if options(p).len() == 2));
        on_input(&mut m, &mut u, &chord("Down"));
        on_input(&mut m, &mut u, &chord("Enter"));
        assert!(matches!(&m, DialogModel::Prompt(p) if p.rename.is_some()));
        on_input(&mut m, &mut u, &chord("Esc"));
        assert!(matches!(&m, DialogModel::Prompt(p) if p.rename.is_none()));
        assert_eq!(u.page, 0);
        let got = response(on_input(&mut m, &mut u, &chord("Alt+M")));
        assert_eq!(got, resolved(ConflictResolution::MergeDir, false));
    }

    #[test]
    fn errors_space_trash_read_only_password() {
        let error = FsError::new(FsErrorKind::PermissionDenied, "read", None);
        let mut m = model(JobPrompt::Error { path: PathBuf::from("x"), error: error.clone(), retryable: true });
        let mut u = ui(&m);
        assert_eq!(
            response(on_input(&mut m, &mut u, &chord("Enter"))),
            r(PromptResponse::Error { action: ErrorAction::Retry, apply_to_all: false })
        );
        let mut m = model(JobPrompt::Error { path: PathBuf::from("x"), error, retryable: false });
        let mut u = ui(&m);
        assert_eq!(u.form.buttons.len(), 3);
        assert_eq!(
            response(on_input(&mut m, &mut u, &chord("A"))),
            r(PromptResponse::Error { action: ErrorAction::Abort, apply_to_all: false })
        );

        let mut m = model(JobPrompt::NotEnoughSpace { needed: 10, free: 1, dest: PathBuf::from("d") });
        let mut u = ui(&m);
        assert_eq!(u.form.focused_button(), Some(0), "the safe button has the focus");
        assert_eq!(response(on_input(&mut m, &mut u, &chord("Enter"))), r(PromptResponse::Space { proceed: false }));

        let mut m = model(JobPrompt::TrashUnsupported { path: PathBuf::from("t"), reason: "network".into() });
        let mut u = ui(&m);
        on_input(&mut m, &mut u, &chord("Space"));
        let b = u.form.buttons.iter().position(|b| b.label == tr!("dialogs-prompt-delete-permanently")).unwrap();
        let got = response(on_mouse(&mut m, &mut u, &left(HitTarget::DialogButton(b))));
        assert_eq!(got, r(PromptResponse::TrashUnsupported { permanent: true, apply_to_all: true }));

        let mut m = model(JobPrompt::ReadOnlyItem { path: PathBuf::from("r") });
        let mut u = ui(&m);
        assert!(matches!(on_mouse(&mut m, &mut u, &left(HitTarget::DialogField(0))), Outcome::Keep));
        assert!(matches!(&m, DialogModel::Prompt(p) if p.apply_all), "a click ticks the box");

        let mut m = model(JobPrompt::Password { archive: PathBuf::from("a.zip"), wrong: false });
        let mut u = ui(&m);
        assert!(!u.form.buttons[1].enabled, "Unlock needs a password");
        assert!(matches!(on_input(&mut m, &mut u, &chord("Enter")), Outcome::Keep));
        on_input(&mut m, &mut u, &text("pw"));
        assert!(u.form.buttons[1].enabled);
        assert_eq!(response(on_input(&mut m, &mut u, &chord("Enter"))), r(PromptResponse::Password(Some("pw".into()))));
        let mut m = model(JobPrompt::Password { archive: PathBuf::from("a.zip"), wrong: true });
        let mut u = ui(&m);
        assert_eq!(response(on_input(&mut m, &mut u, &chord("Alt+C"))), r(PromptResponse::Password(None)));
    }

    #[test]
    fn cancel_operation_cancels_the_job() {
        let mut m = model(conflict(ConflictKind::SameFile));
        let mut u = ui(&m);
        match on_input(&mut m, &mut u, &chord("C")) {
            Outcome::Close(Some(Cmd::FileOp(FileOpCmd::CancelJob(j)))) => assert_eq!(j, JobId(3)),
            other => panic!("{other:?}"),
        }
        // Moving a folder into itself: Skip only, never for all.
        let mut m = model(conflict(ConflictKind::MoveIntoSelf));
        let mut u = ui(&m);
        assert!(u.form.fields.is_empty());
        let got = response(on_input(&mut m, &mut u, &chord("Enter")));
        assert_eq!(got, resolved(ConflictResolution::Skip, false));
    }

    #[test]
    fn more_options_by_mouse() {
        let mut m = model(conflict(ConflictKind::FileExists));
        let mut u = ui(&m);
        assert!(matches!(on_mouse(&mut m, &mut u, &left(HitTarget::DialogButton(1))), Outcome::Keep));
        assert_eq!(u.page, MORE);
        assert!(matches!(on_mouse(&mut m, &mut u, &mouse(MouseEventKind::ScrollDown, None)), Outcome::Keep));
        assert_eq!(u.list.cursor, 3);
        let got = response(on_mouse(&mut m, &mut u, &left(HitTarget::DialogRow(0))));
        assert_eq!(got, resolved(ConflictResolution::OverwriteIfNewer, false));
    }
}
