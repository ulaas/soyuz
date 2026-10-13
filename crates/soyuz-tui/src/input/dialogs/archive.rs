//! The archive dialogs (`FrontendCmd::Archive`, `TuiDialog::Archive`), like the GUI's
//! `ui::archive`:
//! - **Password**: a listing needs a password ("wrong" when the last one failed); Unlock
//!   remembers it for the session and re-lists the pane (`models::archive::unlock`);
//! - **Extract to…**: what is extracted, the destination folder and "each archive into its
//!   own folder"; Extract runs `ExtractForm::submit`'s jobs. The destination has folder
//!   completion like "Copy to" and the compare screen (`state::crumbs::completion`,
//!   [`prepare`]): Up / Down pick a subfolder, Tab accepts it, a click on one too;
//! - **Compress…**: the archive's name, the format (`writable_formats`), the compression
//!   level (Store / Fast / Normal / Max: levels 0, 3, 6, 9) and, for formats that encrypt,
//!   a password; changing the format keeps the name's stem (`CreateForm::set_format`) and
//!   rebuilds the form. Compress runs `CreateForm::submit` (an existing file keeps the form
//!   open with a toast);
//! - **Rename** an entry of a browsed zip: the new name, refused with slashes
//!   (`valid_entry_name`), then `rename_cmd`'s job.

use std::path::PathBuf;

use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::archive::{
    ArchivePrompt, CreateForm, ExtractForm, format_label, level_label, rename_cmd, unlock, valid_entry_name,
    writable_formats,
};
use soyuz_appcore::services::Services;
use soyuz_appcore::state::crumbs;
use soyuz_appcore::state::pane::PaneId;
use soyuz_appcore::tr;
use soyuz_core::config::KeyName;

use super::prompt::{is_key, left_click, set_text, wheel};
use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::layout::HitTarget;
use crate::widgets::{Button, Field, FieldKind, Form, FormOutcome, LineEdit, SelectList};

/// The compression levels offered (Store, Fast, Normal, Max).
pub const LEVELS: [u8; 4] = [0, 3, 6, 9];

/// Field indices of the Compress form.
pub mod create {
    pub const NAME: usize = 0;
    pub const FORMAT: usize = 1;
    pub const LEVEL: usize = 2;
    /// Only for formats that encrypt.
    pub const PASSWORD: usize = 3;
}

/// Which archive dialog is open, with its form.
#[derive(Debug)]
pub enum Kind {
    Password { pane: PaneId, archive: PathBuf, wrong: bool },
    Extract(ExtractForm),
    Create(CreateForm),
    Rename { archive: PathBuf, from: String, is_dir: bool },
}

/// What the dialog keeps while it is open.
#[derive(Debug)]
pub struct State {
    pub kind: Kind,
    pub form: Form,
    /// Extract to…: the subfolders completing the typed destination ([`prepare`]).
    pub names: Vec<String>,
    /// The picked completion.
    pub list: SelectList,
}

impl State {
    fn new(kind: Kind, form: Form) -> State {
        State { kind, form, names: Vec::new(), list: SelectList::default() }
    }
}

/// Open the dialog for the core's request (`crate::frontend`).
pub fn open(app: &mut App, prompt: ArchivePrompt) {
    app.push_dialog(TuiDialog::Archive(state(prompt)));
}

/// A new dialog's state.
pub fn state(prompt: ArchivePrompt) -> State {
    let buttons = |ok: &str| vec![Button::new(ok), Button::new(tr!("common-cancel"))];
    match prompt {
        ArchivePrompt::Password { pane, archive, wrong } => {
            let mut form = Form::new(vec![Field::secret(tr!("remote-password"))], buttons(tr!("views-archive-unlock")));
            if wrong {
                form.error = Some(tr!("views-archive-password-wrong").into());
            }
            State::new(Kind::Password { pane, archive, wrong }, form)
        }
        ArchivePrompt::Extract(f) => {
            let form = Form::new(
                vec![
                    Field::text(tr!("views-archive-destination"), f.dest.as_str()),
                    Field::check(tr!("views-archive-subfolder"), f.subfolder),
                ],
                buttons(tr!("views-archive-extract")),
            );
            State::new(Kind::Extract(f), form)
        }
        ArchivePrompt::Create(f) => {
            let form = create_form(&f, 0);
            State::new(Kind::Create(f), form)
        }
        ArchivePrompt::Rename { archive, from, is_dir } => {
            let name = from.rsplit('/').next().unwrap_or_default().to_string();
            let form = Form::new(vec![Field::text(tr!("remote-name"), name)], buttons(tr!("common-rename")));
            State::new(Kind::Rename { archive, from, is_dir }, form)
        }
    }
}

/// The level choice closest to `level`.
fn level_index(level: u8) -> usize {
    LEVELS.iter().enumerate().min_by_key(|(_, l)| l.abs_diff(level)).map_or(2, |(i, _)| i)
}

/// The Compress form for `f`, focused on stop `focus`.
fn create_form(f: &CreateForm, focus: usize) -> Form {
    let formats = writable_formats();
    let picked = formats.iter().position(|x| *x == f.format).unwrap_or(0);
    let mut fields = vec![
        Field::text(tr!("views-archive-name"), f.name.as_str()),
        Field::radio(
            tr!("views-archive-format"),
            formats.iter().map(|x| format_label(*x).to_string()).collect(),
            picked,
        ),
        Field::radio(
            tr!("views-archive-compression"),
            LEVELS.iter().map(|l| format!("{} ({l})", level_label(*l))).collect(),
            level_index(f.level),
        ),
    ];
    if f.format.supports_password() {
        let mut pw = Field::secret(tr!("views-archive-password-optional"));
        pw.kind = FieldKind::Text { edit: LineEdit::at_end(&f.password), text: f.password.clone(), secret: true };
        fields.push(pw);
    }
    let mut form =
        Form::new(fields, vec![Button::new(tr!("views-archive-compress")), Button::new(tr!("common-cancel"))]);
    form.focus = focus.min(form.fields.len() + form.buttons.len() - 1);
    form
}

fn text(form: &Form, i: usize) -> String {
    form.fields.get(i).and_then(|f| f.value()).unwrap_or_default().to_string()
}

/// Copy the Compress form's fields into `f`; a new format rebuilds the form.
fn sync_create(f: &mut CreateForm, form: &mut Form) {
    f.name = text(form, create::NAME);
    if let Some(l) = form.fields.get(create::LEVEL).and_then(|x| x.selected()) {
        f.level = LEVELS[l.min(LEVELS.len() - 1)];
    }
    if form.fields.len() > create::PASSWORD {
        f.password = text(form, create::PASSWORD);
    }
    let formats = writable_formats();
    let picked = form.fields.get(create::FORMAT).and_then(|x| x.selected()).and_then(|i| formats.get(i).copied());
    if let Some(fmt) = picked
        && fmt != f.format
    {
        f.set_format(fmt);
        let focus = form.focus;
        *form = create_form(f, focus);
    }
}

/// Whether the Compress form shows the "names are encrypted too" note (7z with a password).
pub fn names_encrypted(f: &CreateForm) -> bool {
    format_label(f.format) == "7z" && !f.password.is_empty()
}

fn submit(core: &mut AppCore, s: &mut State) -> Outcome {
    let form = &mut s.form;
    match &mut s.kind {
        Kind::Password { pane, archive, .. } => {
            let pw = text(form, 0);
            if pw.is_empty() {
                return Outcome::Keep;
            }
            Outcome::Close(Some(unlock(*pane, archive, pw)))
        }
        Kind::Extract(f) => {
            f.dest = text(form, 0);
            f.subfolder = form.fields.get(1).and_then(|x| x.on()).unwrap_or(false);
            match f.submit() {
                Some(cmds) => {
                    core.cmds.extend(cmds);
                    Outcome::Close(None)
                }
                None => {
                    form.error = Some(tr!("tui-locations-extract-no-dest").into());
                    Outcome::Keep
                }
            }
        }
        Kind::Create(f) => {
            sync_create(f, form);
            match f.submit() {
                Ok(cmd) => Outcome::Close(Some(cmd)),
                Err(Some(toast)) => {
                    core.cmds.push(Cmd::Toast(toast));
                    Outcome::Keep
                }
                Err(None) => {
                    form.error = Some(tr!("tui-locations-archive-name-empty").into());
                    Outcome::Keep
                }
            }
        }
        Kind::Rename { archive, from, .. } => {
            let name = text(form, 0).trim().to_string();
            if !valid_entry_name(&name) {
                form.error = Some(tr!("views-archive-no-slashes").into());
                return Outcome::Keep;
            }
            Outcome::Close(rename_cmd(archive, from, &name))
        }
    }
}

fn outcome(core: &mut AppCore, s: &mut State, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(0) => Some(submit(core, s)),
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
        FormOutcome::Changed(_) => {
            match &mut s.kind {
                Kind::Create(f) => {
                    s.form.error = None;
                    sync_create(f, &mut s.form);
                }
                Kind::Rename { .. } => {
                    let name = text(&s.form, 0).trim().to_string();
                    let bad = !name.is_empty() && !valid_entry_name(&name);
                    s.form.error = bad.then(|| tr!("views-archive-no-slashes").into());
                }
                Kind::Password { .. } | Kind::Extract(_) => s.form.error = None,
            }
            None
        }
        _ => None,
    }
}

/// Every frame while the dialog is on top (`input::dialogs::prepare`): Extract to…'s
/// completions of the typed destination (this also asks for its listing, off the UI
/// thread).
pub fn prepare(services: &mut Services, s: &mut State) {
    if !matches!(s.kind, Kind::Extract(_)) {
        return;
    }
    let c = crumbs::completion(services, &text(&s.form, 0));
    if c.names != s.names {
        s.names = c.names;
        s.list = SelectList::default();
    }
}

/// Accept completion `i` into the destination field.
fn accept(s: &mut State, i: usize) {
    let Some(name) = s.names.get(i).cloned() else { return };
    let mut dest = text(&s.form, 0);
    crumbs::accept(&mut dest, &name);
    set_text(&mut s.form, 0, &dest);
    s.names.clear();
    s.list = SelectList::default();
}

/// Up / Down / Tab over the destination's completions: whether `ev` was one of them.
fn completion_key(s: &mut State, ev: &InputEvent) -> bool {
    let n = s.names.len();
    if !matches!(s.kind, Kind::Extract(_)) || s.form.focused_field() != Some(0) || n == 0 {
        return false;
    }
    if is_key(ev, KeyName::Tab) {
        accept(s, s.list.cursor);
    } else if is_key(ev, KeyName::ArrowDown) {
        s.list.select((s.list.cursor + 1) % n, n);
    } else if is_key(ev, KeyName::ArrowUp) {
        s.list.select((s.list.cursor + n - 1) % n, n);
    } else {
        return false;
    }
    true
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        if completion_key(s, ev) {
            continue;
        }
        let o = s.form.apply(ev);
        if let Some(out) = outcome(core, s, o) {
            return out;
        }
    }
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    if matches!(s.kind, Kind::Extract(_)) {
        // A click on a completion accepts it; the wheel scrolls them.
        if let (true, Some(HitTarget::DialogRow(r))) = (left_click(m), m.target) {
            accept(s, r);
            s.form.focus = 0;
            return Outcome::Keep;
        }
        if wheel(&mut s.list, m, s.names.len()) {
            return Outcome::Keep;
        }
    }
    match super::confirm::click(&mut s.form, m) {
        Some(o) => outcome(core, s, o).unwrap_or(Outcome::Keep),
        None => Outcome::Keep,
    }
}

/// The dialog went away by itself: a password prompt whose pane is gone.
pub fn stale(core: &AppCore, s: &State) -> bool {
    match &s.kind {
        Kind::Password { pane, .. } => core.ws.find_pane(*pane).is_none(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use soyuz_appcore::keys::input::InputEvent;

    use super::*;

    fn chord(s: &str) -> InputEvent {
        InputEvent::Chord(s.parse().unwrap())
    }

    #[test]
    fn the_format_choice_rebuilds_the_form_and_keeps_the_stem() {
        let f = CreateForm::new(vec![PathBuf::from("/x/a.txt")], PathBuf::from("/x"), "photos".into());
        let mut s = state(ArchivePrompt::Create(f));
        assert_eq!(text(&s.form, create::NAME), "photos.zip");
        // Zip encrypts: the password field shows.
        assert_eq!(s.form.fields.len(), 4);
        assert_eq!(s.form.fields[create::LEVEL].selected(), Some(2), "Normal (6)");
        s.form.focus = create::FORMAT;
        let tar = writable_formats().iter().position(|f| !f.supports_password());
        if let Some(tar) = tar {
            while s.form.fields[create::FORMAT].selected() != Some(tar) {
                let o = s.form.apply(&chord("Right"));
                assert!(matches!(o, FormOutcome::Changed(_)));
                if let Kind::Create(f) = &mut s.kind {
                    sync_create(f, &mut s.form);
                }
            }
            let Kind::Create(f) = &s.kind else { unreachable!() };
            assert!(f.name.starts_with("photos.") && !f.name.ends_with(".zip"), "{}", f.name);
            assert_eq!(s.form.fields.len(), 3, "no password for this format");
            assert_eq!(s.form.focus, create::FORMAT, "the focus stays on the format");
        }
        // The level radio maps to 0 / 3 / 6 / 9.
        s.form.focus = create::LEVEL;
        s.form.apply(&chord("Right"));
        let Kind::Create(f) = &mut s.kind else { unreachable!() };
        sync_create(f, &mut s.form);
        assert_eq!(f.level, 9);
        assert_eq!(level_index(1), 0);
        assert_eq!(level_index(5), 2);
    }

    #[test]
    fn rename_starts_from_the_entry_name() {
        let s = state(ArchivePrompt::Rename {
            archive: PathBuf::from("/x/a.zip"),
            from: "docs/old.txt".into(),
            is_dir: false,
        });
        assert_eq!(text(&s.form, 0), "old.txt");
        let Kind::Rename { archive, from, .. } = &s.kind else { unreachable!() };
        assert!(rename_cmd(archive, from, "old.txt").is_none(), "same name: nothing to do");
        assert!(rename_cmd(archive, from, "new.txt").is_some());
    }

    #[test]
    fn extract_destination_completes_folders() {
        let sep = std::path::MAIN_SEPARATOR;
        let f = ExtractForm::new(vec![PathBuf::from("/x/a.zip")], None, PathBuf::from("/x"));
        let mut s = state(ArchivePrompt::Extract(f));
        // Completions as `prepare` finds them: Down picks the second, Tab takes it.
        set_text(&mut s.form, 0, &format!("{sep}x{sep}d"));
        s.names = vec!["docs".into(), "downloads".into()];
        assert!(completion_key(&mut s, &chord("Down")));
        assert_eq!(s.list.cursor, 1);
        assert!(completion_key(&mut s, &chord("Up")));
        assert!(completion_key(&mut s, &chord("Up")), "wraps");
        assert_eq!(s.list.cursor, 1);
        assert!(completion_key(&mut s, &chord("Tab")));
        assert_eq!(text(&s.form, 0), format!("{sep}x{sep}downloads{sep}"));
        assert!(s.names.is_empty());
        // Without completions Tab is the form's (on to the checkbox).
        assert!(!completion_key(&mut s, &chord("Tab")));
        // Not on the destination field, nor in other dialogs.
        s.names = vec!["x".into()];
        s.form.focus = 1;
        assert!(!completion_key(&mut s, &chord("Tab")));
        let mut r =
            state(ArchivePrompt::Rename { archive: PathBuf::from("/x/a.zip"), from: "a".into(), is_dir: false });
        r.names = vec!["x".into()];
        assert!(!completion_key(&mut r, &chord("Tab")));
    }

    #[test]
    fn a_wrong_password_says_so() {
        let s =
            state(ArchivePrompt::Password { pane: PaneId::next(), archive: PathBuf::from("/x/s.zip"), wrong: true });
        assert_eq!(s.form.error.as_deref(), Some(tr!("views-archive-password-wrong")));
        assert!(matches!(s.form.fields[0].kind, FieldKind::Text { secret: true, .. }));
    }
}
