//! The remote location dialogs (`FrontendCmd::Remote`, `TuiDialog::Remote`), like the
//! GUI's `ui::remote`:
//! - **Connect to server…** over a `models::remote::ConnectForm`: protocol, TLS (WebDAV /
//!   FTP), server or bucket, port or region and endpoint (S3), sign-in method, user,
//!   password / secret key / key passphrase, key file, folder, "save in the sidebar" with
//!   its name and "remember the secret". The fields depend on the protocol and the
//!   sign-in method, so the form is rebuilt from the `ConnectForm` (values kept, focus on
//!   the same field) whenever one of those changes. Connect validates
//!   (`ConnectForm::build`; the error stays in the form) and runs
//!   `ConnectForm::connect`: the secret goes to this session and, when saved, to the OS
//!   credential store (never to `config.toml`);
//!   Servers announced on the local network (`models::remote_discovery`, `[remote]
//!   discover`) are a choice above the form once found; picking one fills it;
//! - **Trust this server?** for an SSH host key (`HostKeyAsk`): host, algorithm,
//!   fingerprint and, for a changed key, the key trusted before. Trust answers `true`;
//!   Cancel, Esc and any other way the dialog goes answer `false` (the state's `Drop`);
//! - **New folder / Rename** on a server (`name_submit`);
//! - the delete confirmation is the shared confirm dialog (`delete_cmd`).

use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::remote::{
    ConnectForm, HostKeyAsk, NameTarget, PROTOCOLS, RemotePrompt, Scheme, delete_cmd, name_submit,
};
use soyuz_appcore::models::remote_discovery::Discovery;
use soyuz_appcore::services::remote::AuthKind;
use soyuz_appcore::tr;

use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::widgets::{Button, Field, FieldKind, Form, FormOutcome, LineEdit};

/// A field of the connect form (the form's fields follow [`layout`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldId {
    /// Servers found on the network (first choice: none, typed in).
    Lan,
    Proto,
    Tls,
    Host,
    Port,
    Region,
    Endpoint,
    Auth,
    User,
    Secret,
    KeyFile,
    Path,
    Save,
    Name,
    Remember,
}

/// The connect dialog's state.
#[derive(Debug)]
pub struct Connect {
    pub form: Box<ConnectForm>,
    pub ui: Form,
    /// What each field of `ui` is.
    pub layout: Vec<FieldId>,
    /// Servers on the local network (`[remote] discover`; not when editing).
    pub lan: Option<Discovery>,
}

/// The host-key question; dropping it unanswered answers "don't trust".
#[derive(Debug)]
pub struct HostKey {
    pub ask: HostKeyAsk,
    pub form: Form,
}

impl Drop for HostKey {
    fn drop(&mut self) {
        self.ask.answer(false);
    }
}

/// What the dialog keeps while it is open.
#[derive(Debug)]
pub enum State {
    Connect(Connect),
    HostKey(HostKey),
    Name { target: NameTarget, form: Form },
}

/// Open the dialog for the core's request (`crate::frontend`).
pub fn open(app: &mut App, prompt: RemotePrompt) {
    match prompt {
        RemotePrompt::Connect(form) => {
            let lan = Discovery::start(app.core.config.remote.discover && form.edit.is_none(), &app.core.wake);
            let mut c = connect_state(*form);
            c.lan = lan;
            app.push_dialog(TuiDialog::Remote(State::Connect(c)));
        }
        RemotePrompt::HostKey(ask) => app.push_dialog(TuiDialog::Remote(State::HostKey(host_key_state(ask)))),
        RemotePrompt::ConfirmDelete { message, paths } => super::confirm::open(
            app,
            tr!("remote-delete-title").to_string(),
            message,
            tr!("common-delete").to_string(),
            delete_cmd(paths),
        ),
        RemotePrompt::Name { target, name } => {
            let ok =
                if matches!(target, NameTarget::NewFolder(_)) { tr!("remote-create") } else { tr!("common-rename") };
            let form = Form::new(
                vec![Field::text(tr!("remote-name"), name)],
                vec![Button::new(ok), Button::new(tr!("common-cancel"))],
            );
            app.push_dialog(TuiDialog::Remote(State::Name { target, form }));
        }
    }
}

/// The connect dialog for `form`.
pub fn connect_state(form: ConnectForm) -> Connect {
    let mut c = Connect { form: Box::new(form), ui: Form::default(), layout: Vec::new(), lan: None };
    rebuild(&mut c, None);
    // The server field first (the GUI focuses it).
    if let Some(i) = c.layout.iter().position(|f| *f == FieldId::Host) {
        c.ui.focus = i;
    }
    c
}

/// The host-key dialog for `ask`: the focus starts on Cancel when the key changed.
pub fn host_key_state(ask: HostKeyAsk) -> HostKey {
    let changed = ask.previous().is_some();
    let trust = if changed { tr!("remote-hostkey-trust-new") } else { tr!("remote-hostkey-trust") };
    let mut form = Form::new(Vec::new(), vec![Button::new(trust), Button::new(tr!("common-cancel"))]);
    if changed {
        form.focus_button(1);
        form.default_button = Some(1);
    }
    HostKey { ask, form }
}

/// The fields the form shows for its protocol and sign-in method (the GUI's layout).
pub fn layout(f: &ConnectForm) -> Vec<FieldId> {
    use FieldId as F;
    let mut v = vec![F::Proto];
    if f.has_tls() {
        v.push(F::Tls);
    }
    v.push(F::Host);
    if f.proto == Scheme::S3 {
        v.extend([F::Region, F::Endpoint]);
    } else {
        v.push(F::Port);
    }
    v.push(F::Auth);
    if f.auth != AuthKind::Anonymous {
        v.push(F::User);
        if f.auth != AuthKind::Agent {
            v.push(F::Secret);
        }
        if f.auth == AuthKind::KeyFile {
            v.push(F::KeyFile);
        }
    }
    v.push(F::Path);
    v.push(F::Save);
    if f.save {
        v.push(F::Name);
        if !matches!(f.auth, AuthKind::Anonymous | AuthKind::Agent) {
            v.push(F::Remember);
        }
    }
    v
}

fn text_field(label: &str, text: &str, secret: bool) -> Field {
    let mut f = Field::text(label, text);
    if secret {
        f.kind = FieldKind::Text { edit: LineEdit::at_end(text), text: text.to_string(), secret: true };
    }
    f
}

/// One field of `f` as a form field.
fn field(f: &ConnectForm, id: FieldId) -> Field {
    let s3 = f.proto == Scheme::S3;
    match id {
        FieldId::Lan => unreachable!("built by lan_field"),
        FieldId::Proto => {
            let picked = PROTOCOLS.iter().position(|(p, _)| *p == f.proto).unwrap_or(0);
            Field::radio(tr!("remote-protocol"), PROTOCOLS.iter().map(|(_, l)| l.to_string()).collect(), picked)
        }
        FieldId::Tls => Field::check(f.tls_label(), f.tls),
        FieldId::Host => text_field(if s3 { tr!("remote-bucket") } else { tr!("remote-server") }, &f.host, false),
        FieldId::Port => {
            let label = format!("{} ({})", tr!("remote-port"), f.scheme().default_port());
            text_field(&label, &f.port, false)
        }
        FieldId::Region => text_field(tr!("remote-region"), &f.region, false),
        FieldId::Endpoint => text_field(tr!("remote-endpoint"), &f.endpoint, false),
        FieldId::Auth => {
            let options = f.auth_options();
            let picked = options.iter().position(|(a, _)| *a == f.auth).unwrap_or(0);
            Field::radio(tr!("remote-sign-in-with"), options.iter().map(|(_, l)| l.to_string()).collect(), picked)
        }
        FieldId::User => text_field(if s3 { tr!("remote-access-key") } else { tr!("remote-user") }, &f.user, false),
        FieldId::Secret => {
            let label = match (f.auth, s3) {
                (_, true) => tr!("remote-secret-key"),
                (AuthKind::KeyFile, _) => tr!("remote-key-passphrase"),
                _ => tr!("remote-password"),
            };
            let keep = f.edit.as_ref().is_some_and(|c| c.has_secret) && f.secret.is_empty();
            let label = if keep { format!("{label} ({})", tr!("remote-secret-unchanged")) } else { label.to_string() };
            text_field(&label, &f.secret, true)
        }
        FieldId::KeyFile => text_field(tr!("remote-key-file-label"), &f.key_file, false),
        FieldId::Path => {
            let label = match f.proto {
                Scheme::S3 => tr!("remote-prefix"),
                Scheme::Smb => tr!("remote-share-folder"),
                _ => tr!("remote-folder"),
            };
            text_field(label, &f.path, false)
        }
        FieldId::Save => Field::check(tr!("remote-save"), f.save),
        FieldId::Name => text_field(tr!("remote-name-hint"), &f.name, false),
        FieldId::Remember => {
            let label = if cfg!(target_os = "linux") { tr!("remote-remember-linux") } else { tr!("remote-remember") };
            Field::check(label, f.remember)
        }
    }
}

/// The servers found on the network as a choice (the first one: none, typed in).
fn lan_field(form: &ConnectForm, lan: &Discovery) -> Field {
    let mut options = vec![tr!("remote-lan-manual").to_string()];
    options.extend(lan.servers.iter().map(|s| s.found.name.chars().take(28).collect::<String>()));
    let picked = lan.servers.iter().position(|s| s.host == form.host && s.proto() == form.proto).map_or(0, |i| i + 1);
    let label = if lan.searching() {
        format!("{} ({})", tr!("remote-lan-title"), tr!("remote-lan-looking"))
    } else {
        tr!("remote-lan-title").to_string()
    };
    Field::radio(label, options, picked)
}

/// Rebuild the form from the `ConnectForm`, keeping the focus on `keep` (a field) when it
/// is still shown, else on the same stop number.
fn rebuild(c: &mut Connect, keep: Option<FieldId>) {
    let old_focus = c.ui.focus;
    let error = c.ui.error.take();
    c.layout = layout(&c.form);
    if c.lan.as_ref().is_some_and(|l| l.searching() || !l.servers.is_empty()) {
        c.layout.insert(0, FieldId::Lan);
    }
    let fields = c
        .layout
        .iter()
        .map(|id| match (id, &c.lan) {
            (FieldId::Lan, Some(lan)) => lan_field(&c.form, lan),
            _ => field(&c.form, *id),
        })
        .collect();
    c.ui = Form::new(fields, vec![Button::new(tr!("remote-connect-button")), Button::new(tr!("common-cancel"))]);
    c.ui.error = error;
    let stops = c.ui.fields.len() + c.ui.buttons.len();
    c.ui.focus = match keep.and_then(|k| c.layout.iter().position(|f| *f == k)) {
        Some(i) => i,
        None => old_focus.min(stops - 1),
    };
}

/// Copy the form's fields into the `ConnectForm`.
fn sync(c: &mut Connect) {
    let f = &mut c.form;
    for (i, id) in c.layout.iter().enumerate() {
        let Some(fld) = c.ui.fields.get(i) else { continue };
        let text = || fld.value().unwrap_or_default().to_string();
        match id {
            // Applied by `connect_outcome` once the typed fields are in.
            FieldId::Lan => {}
            FieldId::Proto => {
                if let Some(p) = fld.selected().and_then(|s| PROTOCOLS.get(s)).map(|(p, _)| *p)
                    && p != f.proto
                {
                    f.set_proto(p);
                }
            }
            FieldId::Tls => f.tls = fld.on().unwrap_or(f.tls),
            FieldId::Host => f.host = text(),
            FieldId::Port => f.port = text(),
            FieldId::Region => f.region = text(),
            FieldId::Endpoint => f.endpoint = text(),
            FieldId::Auth => {
                let options = f.auth_options();
                if let Some(a) = fld.selected().and_then(|s| options.get(s)).map(|(a, _)| *a) {
                    f.auth = a;
                }
            }
            FieldId::User => f.user = text(),
            FieldId::Secret => f.secret = text(),
            FieldId::KeyFile => f.key_file = text(),
            FieldId::Path => f.path = text(),
            FieldId::Save => f.save = fld.on().unwrap_or(f.save),
            FieldId::Name => f.name = text(),
            FieldId::Remember => f.remember = fld.on().unwrap_or(f.remember),
        }
    }
}

/// Fields whose change shows or hides others (or relabels them): the form is rebuilt.
fn reshapes(id: FieldId) -> bool {
    matches!(id, FieldId::Lan | FieldId::Proto | FieldId::Tls | FieldId::Auth | FieldId::Save)
}

fn connect_outcome(c: &mut Connect, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(0) => {
            sync(c);
            match c.form.build() {
                Ok(uri) => Some(Outcome::Close(Some(c.form.connect(uri)))),
                Err(e) => {
                    c.ui.error = Some(e);
                    Some(Outcome::Keep)
                }
            }
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
        FormOutcome::Changed(i) => {
            let id = c.layout.get(i).copied();
            c.ui.error = None;
            sync(c);
            if id == Some(FieldId::Lan)
                && let Some(pick) = c.ui.fields.get(i).and_then(|f| f.selected()).and_then(|s| s.checked_sub(1))
                && let Some(server) = c.lan.as_ref().and_then(|l| l.servers.get(pick)).cloned()
            {
                c.form.fill_lan(&server);
            }
            if let Some(id) = id.filter(|id| reshapes(*id)) {
                rebuild(c, Some(id));
            }
            None
        }
        _ => None,
    }
}

/// Every frame while it is open (`input::dialogs::tick`): servers found on the network
/// appear as a choice above the form.
pub fn tick(_core: &mut AppCore, s: &mut State) {
    let State::Connect(c) = s else { return };
    if c.lan.as_mut().is_some_and(|l| l.poll()) {
        sync(c);
        let keep = c.layout.get(c.ui.focus).copied();
        rebuild(c, keep);
    }
}

fn host_key_outcome(h: &mut HostKey, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(0) => {
            h.ask.answer(true);
            Some(Outcome::Close(None))
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => {
            h.ask.answer(false);
            Some(Outcome::Close(None))
        }
        _ => None,
    }
}

fn name_outcome(target: &NameTarget, form: &mut Form, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(0) => {
            let name = form.fields.first().and_then(|f| f.value()).unwrap_or_default().to_string();
            Some(match name_submit(target, &name) {
                Err(e) => {
                    form.error = Some(e);
                    Outcome::Keep
                }
                Ok(cmd) => Outcome::Close(cmd),
            })
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
        FormOutcome::Changed(_) => {
            form.error = None;
            None
        }
        _ => None,
    }
}

fn outcome(s: &mut State, o: FormOutcome) -> Option<Outcome> {
    match s {
        State::Connect(c) => connect_outcome(c, o),
        State::HostKey(h) => host_key_outcome(h, o),
        State::Name { target, form } => name_outcome(target, form, o),
    }
}

fn form_of(s: &mut State) -> &mut Form {
    match s {
        State::Connect(c) => &mut c.ui,
        State::HostKey(h) => &mut h.form,
        State::Name { form, .. } => form,
    }
}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        let o = form_of(s).apply(ev);
        if let Some(out) = outcome(s, o) {
            return out;
        }
    }
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(_core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    match super::confirm::click(form_of(s), m).and_then(|o| outcome(s, o)) {
        Some(out) => out,
        None => Outcome::Keep,
    }
}

/// The dialog went away by itself: never (a host-key question waits for its answer).
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use soyuz_appcore::keys::input::InputEvent;

    use super::*;

    fn chord(s: &str) -> InputEvent {
        InputEvent::Chord(s.parse().unwrap())
    }

    fn at(c: &Connect, id: FieldId) -> usize {
        c.layout.iter().position(|f| *f == id).unwrap()
    }

    #[test]
    fn the_layout_follows_protocol_and_sign_in() {
        let mut f = ConnectForm::new();
        let sftp = layout(&f);
        assert!(sftp.contains(&FieldId::Port) && !sftp.contains(&FieldId::Tls) && sftp.contains(&FieldId::Secret));
        f.auth = AuthKind::Agent;
        assert!(!layout(&f).contains(&FieldId::Secret) && !layout(&f).contains(&FieldId::Remember));
        f.set_proto(Scheme::S3);
        let s3 = layout(&f);
        assert!(s3.contains(&FieldId::Region) && s3.contains(&FieldId::Endpoint) && !s3.contains(&FieldId::Port));
        f.save = false;
        assert!(!layout(&f).contains(&FieldId::Name));
    }

    #[test]
    fn picking_a_protocol_rebuilds_the_form_and_keeps_typed_values() {
        let mut c = connect_state(ConnectForm::new());
        assert_eq!(c.ui.focus, at(&c, FieldId::Host));
        c.ui.apply(&InputEvent::Text("Example.org".into()));
        let o = c.ui.apply(&chord("Shift+Tab"));
        assert_eq!(o, FormOutcome::Consumed);
        // To WebDAV: a TLS box appears; the focus stays on the protocol.
        let webdav = PROTOCOLS.iter().position(|(p, _)| *p == Scheme::WebDav).unwrap();
        while c.ui.fields[at(&c, FieldId::Proto)].selected() != Some(webdav) {
            let o = c.ui.apply(&chord("Right"));
            connect_outcome(&mut c, o);
        }
        assert!(c.layout.contains(&FieldId::Tls));
        assert_eq!(c.ui.focus, at(&c, FieldId::Proto));
        assert_eq!(c.ui.fields[at(&c, FieldId::Host)].value(), Some("Example.org"));
        assert_eq!(c.form.proto, Scheme::WebDav);
        // A host is enough for WebDAV (Connect would open it; not run here: it saves).
        assert_eq!(c.form.build().map(|u| u.display()).ok().as_deref(), Some("webdavs://example.org/"));
    }

    #[test]
    fn a_server_found_on_the_network_fills_the_form() {
        use soyuz_appcore::models::remote_discovery::{Found, LanServer};
        let server = LanServer {
            found: Found {
                scheme: Scheme::WebDav,
                name: "Shared Files".into(),
                host: "nas.local".into(),
                addrs: vec![],
                port: 8080,
                path: Some("/dav".into()),
                user: None,
            },
            host: "nas.local".into(),
        };
        let mut c = connect_state(ConnectForm::new());
        c.lan = Some(Discovery::fixed(vec![server]));
        rebuild(&mut c, Some(FieldId::Host));
        assert_eq!(c.layout[0], FieldId::Lan);
        assert_eq!(c.ui.fields[0].selected(), Some(0), "typed in");
        // Something typed first is kept in the form until a server is picked.
        c.ui.focus = at(&c, FieldId::User);
        c.ui.apply(&InputEvent::Text("ann".into()));
        c.ui.focus = 0;
        let o = c.ui.apply(&chord("Right"));
        connect_outcome(&mut c, o);
        assert_eq!((c.form.proto, c.form.host.as_str(), c.form.port.as_str()), (Scheme::WebDav, "nas.local", "8080"));
        assert_eq!((c.form.path.as_str(), c.form.user.as_str()), ("/dav", "ann"));
        assert!(c.layout.contains(&FieldId::Tls), "rebuilt for WebDAV");
        assert_eq!(c.ui.fields[at(&c, FieldId::Host)].value(), Some("nas.local"));
        assert_eq!(c.ui.fields[0].selected(), Some(1));
        assert_eq!(c.ui.focus, 0, "the focus stays on the choice");
    }

    #[test]
    fn an_empty_host_keeps_the_dialog_open_with_the_error() {
        let mut c = connect_state(ConnectForm::new());
        c.form.save = false;
        assert!(matches!(connect_outcome(&mut c, FormOutcome::Pressed(0)), Some(Outcome::Keep)));
        assert_eq!(c.ui.error.as_deref(), Some(tr!("remote-error-server")));
        assert!(matches!(connect_outcome(&mut c, FormOutcome::Cancel), Some(Outcome::Close(None))));
    }

    #[test]
    fn host_keys_are_answered_once_and_dropping_says_no() {
        let (tx, rx) = crossbeam_channel::bounded(2);
        let mut h = host_key_state(HostKeyAsk::from_parts("h", 22, "ssh-ed25519", "SHA256:x", None, tx));
        assert!(matches!(host_key_outcome(&mut h, FormOutcome::Pressed(0)), Some(Outcome::Close(None))));
        drop(h);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![true]);
        let (tx, rx) = crossbeam_channel::bounded(2);
        let h = host_key_state(HostKeyAsk::from_parts("h", 22, "a", "f", Some("old".into()), tx));
        assert_eq!(h.form.focused_button(), Some(1), "a changed key starts on Cancel");
        drop(h);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![false]);
    }
}
