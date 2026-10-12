//! Keys while the command palette is open (step T2.7): the palette reads its own
//! `soyuz_appcore::palette::KeyInput` first, then a `LineEdit` edits `palette.input`.
//! Stub: the core's dispatcher as for the list (`crate::unsupported::palette` closes the
//! palette after the input that opened it).

use crossterm::event::Event;

use super::{TuiFocus, core_input, key_context};
use crate::app::App;

/// What the palette keeps between frames.
#[derive(Debug, Default)]
pub struct State {}

pub fn route(app: &mut App, events: &[Event]) {
    let (kctx, text) = key_context(TuiFocus::List);
    core_input(app, events, kctx, text);
}
