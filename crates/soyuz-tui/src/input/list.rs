//! Keys for the file list: the core's bindings, sequences, Tab / Left / Right and
//! type-ahead (`soyuz_appcore::keys::input::handle`).

use crossterm::event::Event;

use super::{TuiFocus, core_input, key_context};
use crate::app::App;

pub fn route(app: &mut App, events: &[Event]) {
    let (kctx, text) = key_context(TuiFocus::List);
    core_input(app, events, kctx, text);
}
