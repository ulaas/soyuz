//! Quick share (`TuiDialog::Share`, `ToolCmd::QuickShare`): the selection served over
//! HTTP on the local network while the dialog is open (`models::quick_share`, shared
//! with the GUI's window). It shows the address, the QR code drawn with half blocks,
//! the other addresses of this computer (a list: Up / Down or a click picks the one shown
//! and encoded), what was shared and what happened so far (redrawn every second).
//!
//! Keys: Copy address (Alt+C, or C) puts it on the clipboard and keeps the dialog; Enter
//! / Esc stop sharing and close (closing any way stops it: the model's `Drop`).

use std::path::PathBuf;
use std::time::Duration;

use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::quick_share::QuickShare;
use soyuz_appcore::tr;

use super::confirm::{ListClick, list_mouse};
use super::prompt::{click, list_key};
use super::split::Ticker;
use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::widgets::{Button, Form, FormOutcome, SelectList};

/// Buttons, in order.
pub const COPY: usize = 0;
pub const STOP: usize = 1;

/// What the dialog keeps while it is open.
#[derive(Debug)]
pub struct State {
    pub model: QuickShare,
    /// The cursor in the addresses (shown when there are several).
    pub list: SelectList,
    /// Buttons: copy address, stop sharing.
    pub form: Form,
    _ticker: Option<Ticker>,
}

/// Open the dialog for `ToolCmd::QuickShare`.
pub fn open(app: &mut App, paths: Vec<PathBuf>) {
    let model = QuickShare::start(paths, &app.core.config.remote);
    let ticker = Ticker::every(app.core.wake.clone(), Duration::from_secs(1));
    app.push_dialog(TuiDialog::Share(state(model, Some(ticker))));
}

/// A new dialog's state for `model`.
pub fn state(model: QuickShare, ticker: Option<Ticker>) -> State {
    let mut form = Form::new(
        Vec::new(),
        vec![Button::new(tr!("tools-share-copy")).accel('c'), Button::new(tr!("tools-share-stop"))],
    )
    .default_button(STOP);
    form.focus_button(STOP);
    let mut list = SelectList::new();
    list.select(model.picked, model.addresses.len());
    let mut s = State { model, list, form, _ticker: ticker };
    sync(&mut s);
    s
}

/// Buttons as the share's state wants them: Copy only while serving, Stop becomes Close.
fn sync(s: &mut State) {
    let running = s.model.running();
    s.form.buttons[COPY].enabled = running && s.model.url().is_some();
    s.form.buttons[STOP].label = if running { tr!("tools-share-stop") } else { tr!("common-close") }.into();
}

fn pick(s: &mut State, i: usize) {
    s.model.pick(i);
    s.list.select(s.model.picked, s.model.addresses.len());
}

/// What a button does; Copy queues its command on `cmds` (the core's).
fn outcome(cmds: &mut Vec<Cmd>, s: &mut State, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(COPY) => {
            if let Some(url) = s.model.url() {
                cmds.push(Cmd::CopyText(url));
            }
            Some(Outcome::Keep)
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel => {
            s.model.stop();
            Some(Outcome::Close(None))
        }
        _ => None,
    }
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    keys(&mut core.cmds, s, i)
}

fn keys(cmds: &mut Vec<Cmd>, s: &mut State, i: &DialogInput) -> Outcome {
    let n = s.model.addresses.len();
    for ev in &i.events {
        if n > 1 && list_key(&mut s.list, ev, n) {
            let c = s.list.cursor;
            pick(s, c);
            continue;
        }
        let o = s.form.apply(ev);
        if let Some(out) = outcome(cmds, s, o) {
            sync(s);
            return out;
        }
    }
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(core: &mut AppCore, s: &mut State, e: &DialogMouse) -> Outcome {
    let n = s.model.addresses.len();
    if n > 1 {
        match list_mouse(&mut s.list, n, e) {
            Some(ListClick::Row { i, .. }) => {
                pick(s, i);
                return Outcome::Keep;
            }
            Some(ListClick::Scrolled) => return Outcome::Keep,
            None => {}
        }
    }
    let o = click(&mut s.form, e);
    outcome(&mut core.cmds, s, o).unwrap_or(Outcome::Keep)
}

/// The dialog went away by itself: never (an ended share stays to say so).
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}

/// Every frame while it is open: the buttons follow the share (it may have expired).
pub fn tick(_core: &mut AppCore, s: &mut State) {
    sync(s);
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use soyuz_appcore::keys::input::InputEvent;
    use soyuz_core::config::RemoteSection;

    use super::*;

    fn key(s: &str) -> DialogInput {
        DialogInput { key: None, events: vec![InputEvent::Chord(s.parse().unwrap())] }
    }

    #[test]
    fn copy_keeps_it_open_arrows_pick_the_address_and_esc_stops() {
        let d = tempfile::tempdir().unwrap();
        let lo = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let other = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));
        let model = QuickShare::start_on(vec![d.path().to_path_buf()], &RemoteSection::default(), lo, vec![lo, other]);
        let mut s = state(model, None);
        let mut cmds = Vec::new();
        assert!(matches!(keys(&mut cmds, &mut s, &key("Alt+C")), Outcome::Keep));
        assert!(matches!(cmds.last(), Some(Cmd::CopyText(u)) if u.starts_with("http://127.0.0.1:")));
        assert!(matches!(keys(&mut cmds, &mut s, &key("Down")), Outcome::Keep));
        assert_eq!(s.model.picked, 1);
        assert!(s.model.url().unwrap().starts_with("http://127.0.0.2:"));
        assert!(matches!(keys(&mut cmds, &mut s, &key("Esc")), Outcome::Close(None)));
        assert!(!s.model.running());
    }
}
