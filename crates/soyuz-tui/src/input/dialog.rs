//! Keys while a core prompt is open go to the dialog fallback only (like the GUI, where
//! the modal owns the keyboard). T3 brings the real dialog views.

use crossterm::event::Event;

use crate::app::App;

pub fn route(app: &mut App, events: &[Event]) {
    for ev in events {
        let Event::Key(k) = ev else { continue };
        let Some(top) = app.core.dialogs.top_mut() else { break };
        if let crate::dialog::Outcome::Close(cmd) = crate::dialog::on_key(top, k) {
            if let Some(c) = cmd {
                app.core.cmds.push(c);
            }
            app.core.dialogs.close_top();
        }
    }
}
