//! Split and join files (`TuiDialog::Split`, Phase T7, agent A): part size presets or a
//! custom size, the output folder, progress, cancel; it draws `models::split::SplitModel`
//! (the GUI's `ui/tools/split.rs` window), whose workers do every disk check.
//!
//! Keys: Up / Down / PageUp / PageDown / Home / End pick the part size (split, while the
//! focus is not in a text field); Tab moves between the custom size, the output folder
//! and the buttons; Enter starts (then, after a success, closes: "Done"); Esc closes, or
//! stops a running split / join (it cleans up after itself). Mouse: a click on a preset
//! picks it; fields and buttons as usual. While it runs the fields are read-only and a
//! ticker wakes the frame loop every 100 ms for the progress line. Closing the dialog
//! any way while it runs (Esc twice, a quit, a stale close) cancels: [`State`]'s `Drop`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use soyuz_appcore::core::AppCore;
use soyuz_appcore::host::Wake;
use soyuz_appcore::keys::input::InputEvent;
use soyuz_appcore::models::split::{CUSTOM, Mode, PRESETS, SplitModel};
use soyuz_appcore::tr;

use super::confirm::{ListClick, list_mouse};
use super::prompt::{LIST_PAGE, click, list_key};
use super::{DialogInput, DialogMouse, Outcome, TuiDialog};
use crate::app::App;
use crate::widgets::form::FieldKind;
use crate::widgets::{Button, Field, Form, FormOutcome, SelectList};

/// Buttons, in order.
pub const START: usize = 0;
pub const CANCEL: usize = 1;

/// How often a running split / join redraws (its progress line).
pub const TICK: Duration = Duration::from_millis(100);

/// Wakes the frame loop every [`TICK`] until dropped.
#[derive(Debug)]
pub struct Ticker {
    stop: Arc<AtomicBool>,
}

impl Ticker {
    fn start(wake: Wake) -> Ticker {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let _ = std::thread::Builder::new().name("soyuz-split-tick".into()).spawn(move || {
            while !s.load(Ordering::Relaxed) {
                std::thread::sleep(TICK);
                wake();
            }
        });
        Ticker { stop }
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// What the dialog keeps while it is open.
#[derive(Debug, Default)]
pub struct State {
    /// The split / join (`None` only in a default state).
    pub model: Option<SplitModel>,
    /// The cursor in the part size presets (split).
    pub list: SelectList,
    /// Fields: the custom size (split), the output folder; buttons: start / done, cancel.
    pub form: Form,
    ticker: Option<Ticker>,
}

impl Drop for State {
    fn drop(&mut self) {
        // A closed dialog never leaves a worker writing parts.
        if let Some(m) = &self.model {
            m.cancel();
        }
    }
}

/// Open the dialog to split `path` (`ToolCmd::Split`).
pub fn open_split(app: &mut App, path: PathBuf) {
    app.push_dialog(TuiDialog::Split(state(SplitModel::split(path))));
}

/// Open the dialog to join the parts of `path` (`ToolCmd::Join`).
pub fn open_join(app: &mut App, path: PathBuf) {
    app.push_dialog(TuiDialog::Split(state(SplitModel::join(path))));
}

/// A new dialog's state for `model`.
pub fn state(model: SplitModel) -> State {
    let mut fields = Vec::new();
    if model.mode == Mode::Split {
        fields.push(Field::text(tr!("tui-split-custom"), model.custom.as_str()));
    }
    fields.push(Field::text(tr!("tui-split-output"), model.out_dir.as_str()));
    let start = if model.mode == Mode::Split { tr!("tools-split-start") } else { tr!("tools-join-start") };
    let mut form = Form::new(fields, vec![Button::new(start), Button::new(tr!("common-cancel"))]).default_button(START);
    let mut list = SelectList::new();
    list.select(model.preset, PRESETS.len());
    let mut s = State { model: Some(model), list, form: Form::default(), ticker: None };
    sync(&mut form, s.model.as_ref());
    form.focus_button(START);
    s.form = form;
    s
}

/// The custom size field (split only).
fn custom_field(m: &SplitModel) -> Option<usize> {
    (m.mode == Mode::Split).then_some(0)
}

/// The output folder field.
fn output_field(m: &SplitModel) -> usize {
    usize::from(m.mode == Mode::Split)
}

/// Fields and buttons as the model's state wants them: read-only while running, the
/// custom size only for the custom preset, Start only when the model can start, Done
/// after a success.
fn sync(form: &mut Form, m: Option<&SplitModel>) {
    let Some(m) = m else { return };
    let running = m.running();
    if let Some(c) = custom_field(m) {
        form.fields[c].enabled = !running && m.preset == CUSTOM;
    }
    form.fields[output_field(m)].enabled = !running;
    let start = if m.finished_ok() {
        tr!("tools-done")
    } else if m.mode == Mode::Split {
        tr!("tools-split-start")
    } else {
        tr!("tools-join-start")
    };
    form.buttons[START].label = start.into();
    form.buttons[START].enabled = m.finished_ok() || m.can_start();
    form.buttons[CANCEL].label = if m.finished_ok() { tr!("common-close") } else { tr!("common-cancel") }.into();
    // The focus leaves what was disabled: to Cancel while running, else to Start.
    let stop_ok = |f: &Form, i: usize| match i.checked_sub(f.fields.len()) {
        None => f.fields[i].enabled,
        Some(b) => f.buttons.get(b).is_some_and(|b| b.enabled),
    };
    if !stop_ok(form, form.focus) {
        form.focus_button(if running || !form.buttons[START].enabled { CANCEL } else { START });
    }
}

fn text_of(form: &Form, i: usize) -> String {
    match form.fields.get(i).map(|f| &f.kind) {
        Some(FieldKind::Text { text, .. }) => text.clone(),
        _ => String::new(),
    }
}

fn outcome(s: &mut State, o: FormOutcome, wake: &Wake) -> Option<Outcome> {
    let m = s.model.as_mut()?;
    match o {
        FormOutcome::Pressed(START) if m.finished_ok() => return Some(Outcome::Close(None)),
        FormOutcome::Pressed(START) => {
            m.start();
            if m.running() {
                s.ticker = Some(Ticker::start(wake.clone()));
            }
        }
        FormOutcome::Pressed(_) | FormOutcome::Cancel if m.running() => m.cancel(),
        FormOutcome::Pressed(_) | FormOutcome::Cancel => return Some(Outcome::Close(None)),
        FormOutcome::Changed(i) => {
            let text = text_of(&s.form, i);
            if custom_field(m) == Some(i) {
                m.custom = text;
            } else {
                m.out_dir = text;
            }
            // A new size or folder: the last run's message goes.
            m.result = None;
        }
        FormOutcome::Consumed | FormOutcome::Ignored => {}
    }
    None
}

/// Pick preset `i` (split).
fn pick(s: &mut State, i: usize) {
    if let Some(m) = s.model.as_mut().filter(|m| m.mode == Mode::Split && !m.running()) {
        m.preset = i.min(PRESETS.len() - 1);
        m.result = None;
        s.list.select(m.preset, PRESETS.len());
    }
}

fn preset_key(s: &mut State, ev: &InputEvent) -> bool {
    let Some(m) = &s.model else { return false };
    let text_focus = s.form.focused_field().is_some();
    if m.mode != Mode::Split || m.running() || text_focus {
        return false;
    }
    if !list_key(&mut s.list, ev, PRESETS.len()) {
        return false;
    }
    let i = s.list.cursor;
    pick(s, i);
    true
}

/// A key press or paste.
pub fn on_input(core: &mut AppCore, s: &mut State, i: &DialogInput) -> Outcome {
    for ev in &i.events {
        if !preset_key(s, ev) {
            let o = s.form.apply(ev);
            if let Some(out) = outcome(s, o, &core.wake) {
                return out;
            }
        }
        sync(&mut s.form, s.model.as_ref());
    }
    Outcome::Keep
}

/// A mouse event.
pub fn on_mouse(core: &mut AppCore, s: &mut State, e: &DialogMouse) -> Outcome {
    let split = s.model.as_ref().is_some_and(|m| m.mode == Mode::Split);
    if split {
        match list_mouse(&mut s.list, PRESETS.len(), e) {
            Some(ListClick::Row { i, .. }) => {
                pick(s, i);
                sync(&mut s.form, s.model.as_ref());
                return Outcome::Keep;
            }
            Some(ListClick::Scrolled) => return Outcome::Keep,
            None => {}
        }
    }
    let o = click(&mut s.form, e);
    let out = outcome(s, o, &core.wake).unwrap_or(Outcome::Keep);
    sync(&mut s.form, s.model.as_ref());
    out
}

/// The dialog went away by itself: never.
pub fn stale(_core: &AppCore, _s: &State) -> bool {
    false
}

/// Every frame while it is open (`input::dialogs::tick`, from `view::prepare`): the probe's
/// findings and a finished run's result (its toast); the ticker stops with the run.
pub fn tick(core: &mut AppCore, s: &mut State) {
    if let Some(t) = advance(s) {
        core.ui.toasts.push(t);
    }
}

/// [`tick`] without the core: returns the toast of a finished run.
fn advance(s: &mut State) -> Option<soyuz_appcore::models::toasts::Toast> {
    let m = s.model.as_mut()?;
    let probing = m.probing();
    let toast = m.poll();
    if !m.running() {
        s.ticker = None;
    }
    let ready = probing && !m.probing() && m.can_start();
    sync(&mut s.form, s.model.as_ref());
    // The probe's findings arrived: Start (disabled until now) takes the focus.
    if ready {
        s.form.focus_button(START);
    }
    s.list.show_cursor(LIST_PAGE.min(PRESETS.len()));
    toast
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn key(s: &str) -> DialogInput {
        DialogInput { key: None, events: vec![InputEvent::Chord(s.parse().unwrap())] }
    }

    fn keys(s: &mut State, k: &str) -> Outcome {
        let i = key(k);
        let mut out = Outcome::Keep;
        for ev in &i.events {
            if !preset_key(s, ev) {
                let o = s.form.apply(ev);
                if let Some(o) = outcome(s, o, &soyuz_appcore::host::no_wake()) {
                    out = o;
                }
            }
            sync(&mut s.form, s.model.as_ref());
        }
        out
    }

    fn settle(s: &mut State, done: impl Fn(&SplitModel) -> bool) {
        let start = Instant::now();
        while !done(s.model.as_ref().unwrap()) {
            assert!(start.elapsed() < Duration::from_secs(5));
            advance(s);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn arrows_pick_the_preset_and_custom_enables_its_field() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("a.bin");
        std::fs::write(&f, vec![7u8; 3000]).unwrap();
        let mut s = state(SplitModel::split(f));
        settle(&mut s, |m| !m.probing());
        assert_eq!(s.form.focused_button(), Some(START));
        assert!(!s.form.fields[0].enabled, "the custom size is off for a preset");
        keys(&mut s, "End");
        assert_eq!(s.model.as_ref().unwrap().preset, CUSTOM);
        assert!(s.form.fields[0].enabled);
        keys(&mut s, "Up");
        assert_eq!(s.model.as_ref().unwrap().preset, CUSTOM - 1);
        // Esc closes while idle.
        assert!(matches!(keys(&mut s, "Esc"), Outcome::Close(None)));
    }

    #[test]
    fn dropping_the_dialog_stops_the_split() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("big.bin");
        std::fs::write(&f, vec![1u8; 32 * 1024 * 1024]).unwrap();
        let mut s = state(SplitModel::split(f.clone()));
        settle(&mut s, |m| !m.probing());
        let m = s.model.as_mut().unwrap();
        m.preset = CUSTOM;
        m.custom = "1 MiB".into();
        m.start();
        assert!(m.running());
        drop(s);
        // Cancelled: the worker removes what it wrote (unless it finished first).
        let part = |n| soyuz_appcore::models::split::part_path(&f, n);
        std::thread::sleep(Duration::from_millis(300));
        let start = Instant::now();
        while part(1).exists() && !soyuz_appcore::models::split::checksum_path(&f).exists() {
            assert!(start.elapsed() < Duration::from_secs(20), "the worker never stopped");
            std::thread::sleep(Duration::from_millis(10));
        }
        let all = (1..=32).all(|n| part(n).exists());
        assert!(all || !part(1).exists(), "no partial output");
    }

    #[test]
    fn a_join_of_a_non_part_cannot_start() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("plain.txt");
        std::fs::write(&f, b"x").unwrap();
        let mut s = state(SplitModel::join(f));
        settle(&mut s, |m| !m.probing());
        assert_eq!(s.form.fields.len(), 1, "a join has only the output folder");
        assert!(!s.form.buttons[START].enabled);
        assert_ne!(s.form.focused_button(), Some(START));
        assert!(matches!(keys(&mut s, "Enter"), Outcome::Keep), "Enter does not start");
    }
}
