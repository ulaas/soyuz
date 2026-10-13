//! The About dialog (`TuiDialog::About`, Phase T7, agent A): "Soyuz", the version and the
//! tagline, the facts (`models::about`: commit, build, platform, renderer, config folder,
//! memory) and the license, with "Copy info" (text only, no icon). Enter / Esc close;
//! Copy info (Alt+C, or C) puts the facts on the clipboard (the local one or OSC 52,
//! `[tui] osc52`).

use soyuz_appcore::cmd::Cmd;
use soyuz_appcore::core::AppCore;
use soyuz_appcore::models::about;
use soyuz_appcore::tr;

use super::prompt::{LIST_PAGE, click, list_key};
use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::icons::IconSet;
use crate::theme::{ColorMode, TuiTheme};
use crate::widgets::{Button, Form, FormOutcome, SelectList};

/// Buttons, in order.
pub const COPY: usize = 0;
pub const CLOSE: usize = 1;

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    /// The facts, as `models::about::facts` gave them when the dialog opened.
    pub facts: Vec<(&'static str, String)>,
    /// The scroll of the facts.
    pub list: SelectList,
    /// Buttons: copy info, close.
    pub form: Form,
}

/// Open the dialog for `FrontendCmd::About`.
pub fn open(app: &mut App) {
    let theme = TuiTheme::cached(&app.core, false);
    let s = state(&renderer(theme.mode(), theme.icons()));
    app.push_dialog(TuiDialog::About(s));
}

/// The renderer fact of the terminal UI: its colour depth and icon set (English, like
/// the other facts).
pub fn renderer(mode: ColorMode, icons: IconSet) -> String {
    let mode = match mode {
        ColorMode::Truecolor => "truecolor",
        ColorMode::Ansi256 => "256 colours",
        ColorMode::Ansi16 => "16 colours",
        ColorMode::Mono => "monochrome",
    };
    let icons = match icons {
        IconSet::Nerd => "Nerd Font icons",
        IconSet::Unicode => "Unicode icons",
        IconSet::Ascii => "ASCII icons",
    };
    format!("terminal UI · {mode} · {icons}")
}

/// A new dialog's state for the renderer fact `renderer`.
pub fn state(renderer: &str) -> State {
    let mut form = Form::new(
        Vec::new(),
        vec![Button::new(tr!("dialogs-about-copy-info")).accel('c'), Button::new(tr!("common-close"))],
    )
    .default_button(CLOSE);
    form.focus_button(CLOSE);
    State { facts: about::facts(renderer), list: SelectList::new(), form }
}

fn keys(s: &mut State, i: &DialogInput) -> Outcome {
    // The facts after the version (it is in the header).
    let n = s.facts.len().saturating_sub(1);
    for ev in &i.events {
        if list_key(&mut s.list, ev, n) {
            continue;
        }
        let o = s.form.apply(ev);
        if let Some(out) = outcome(s, o) {
            return out;
        }
    }
    Outcome::Keep
}

fn outcome(s: &State, o: FormOutcome) -> Option<Outcome> {
    match o {
        FormOutcome::Pressed(COPY) => Some(Outcome::Close(Some(Cmd::CopyText(about::copy_text(&s.facts))))),
        FormOutcome::Pressed(_) | FormOutcome::Cancel => Some(Outcome::Close(None)),
        _ => None,
    }
}

/// A key press or paste.
pub fn on_input(_core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    keys(s, i)
}

/// A mouse event.
pub fn on_mouse(_core: &mut AppCore, s: &mut State, m: &DialogMouse) -> Outcome {
    let n = s.facts.len().saturating_sub(1);
    if super::prompt::wheel(&mut s.list, m, n) {
        s.list.show_cursor(LIST_PAGE);
        return Outcome::Keep;
    }
    let o = click(&mut s.form, m);
    outcome(s, o).unwrap_or(Outcome::Keep)
}

/// The dialog went away by itself: never.
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use soyuz_appcore::keys::input::InputEvent;

    use super::*;

    fn key(s: &str) -> DialogInput {
        DialogInput { key: None, events: vec![InputEvent::Chord(s.parse().unwrap())] }
    }

    #[test]
    fn copy_info_copies_the_facts_and_enter_closes() {
        let r = renderer(ColorMode::Mono, IconSet::Ascii);
        assert_eq!(r, "terminal UI · monochrome · ASCII icons");
        let mut s = state(&r);
        match keys(&mut s, &key("Alt+C")) {
            Outcome::Close(Some(Cmd::CopyText(t))) => {
                assert!(t.starts_with("Soyuz\nVersion: "), "{t}");
                assert!(t.contains("Renderer: terminal UI"), "{t}");
            }
            other => panic!("{other:?}"),
        }
        let mut s = state(&r);
        assert!(matches!(keys(&mut s, &key("Enter")), Outcome::Close(None)));
        let mut s = state(&r);
        assert!(matches!(keys(&mut s, &key("Esc")), Outcome::Close(None)));
    }
}
