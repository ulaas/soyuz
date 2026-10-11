//! egui input → the toolkit-free [`InputFrame`] and back: the frame lists this frame's
//! events in order, the core marks what it took, and [`strip`] removes exactly those
//! events before the widgets run.

use egui::{Event, PointerButton};

use crate::keys::input::{InputEvent, InputFrame};
use crate::ui::keys::event_chord;

fn convert(ev: &Event, current: egui::Modifiers, cmd_is_ctrl: bool) -> InputEvent {
    match ev {
        Event::Text(t) => InputEvent::Text(t.clone()),
        ev => event_chord(ev, current, cmd_is_ctrl).map(InputEvent::Chord).unwrap_or(InputEvent::Other),
    }
}

/// This frame's events of `ctx`, one [`InputEvent`] per egui event.
pub fn frame(ctx: &egui::Context, cmd_is_ctrl: bool) -> InputFrame {
    ctx.input(|i| {
        let current = i.modifiers;
        let events = i.events.iter().map(|ev| convert(ev, current, cmd_is_ctrl)).collect();
        let mut f = InputFrame::new(events);
        f.command_held = current.ctrl || current.alt || current.mac_cmd;
        f.back = i.pointer.button_pressed(PointerButton::Extra1);
        f.forward = i.pointer.button_pressed(PointerButton::Extra2);
        f
    })
}

/// Remove the events the core consumed. Events are matched in order, so an event added
/// or removed by someone else in between does not shift the rest.
pub fn strip(ctx: &egui::Context, frame: &InputFrame, cmd_is_ctrl: bool) {
    if !frame.consumed.contains(&true) {
        return;
    }
    ctx.input_mut(|i| {
        let current = i.modifiers;
        let mut j = 0;
        i.events.retain(|ev| {
            if j >= frame.events.len() {
                return true;
            }
            let got = convert(ev, current, cmd_is_ctrl);
            // Skip frame events that are gone (removed in between).
            let mut k = j;
            while k < frame.events.len() && frame.events[k] != got {
                k += 1;
            }
            if k == frame.events.len() {
                return true; // added in between: not ours
            }
            j = k + 1;
            !frame.consumed[k]
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Key, Modifiers, RawInput};

    fn key(k: Key, m: Modifiers) -> Event {
        Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: m }
    }

    /// Runs one egui pass over `events`, lets `take` mark consumed events, strips them, and
    /// returns what the widgets would still see.
    fn pass(events: Vec<Event>, take: impl Fn(&mut InputFrame)) -> Vec<Event> {
        let ctx = egui::Context::default();
        let raw = RawInput { events, ..Default::default() };
        let mut left = Vec::new();
        let mut out = ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx().clone();
            let mut f = frame(&ctx, true);
            take(&mut f);
            strip(&ctx, &f, true);
            left = ctx.input(|i| i.events.clone());
        });
        out.textures_delta.clear();
        left
    }

    #[test]
    fn strips_exactly_the_consumed_events() {
        let j = key(Key::J, Modifiers::NONE);
        let left = pass(vec![j.clone(), Event::Text("j".into()), key(Key::Q, Modifiers::NONE)], |f| {
            assert_eq!(f.events.len(), 3);
            f.consumed[0] = true;
            f.consumed[1] = true;
        });
        assert_eq!(left, vec![key(Key::Q, Modifiers::NONE)]);
    }

    #[test]
    fn clipboard_events_become_chords() {
        let _ = pass(vec![Event::Copy, Event::PointerGone], |f| {
            assert_eq!(f.events[0], InputEvent::Chord("Ctrl+C".parse().unwrap()));
            assert_eq!(f.events[1], InputEvent::Other);
        });
    }
}
