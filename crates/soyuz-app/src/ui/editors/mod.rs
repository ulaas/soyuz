//! The editor panel in egui (`crate::editors` holds the documents): the docked or
//! maximized card with its tabs, the text, hex and image views, and keyboard routing
//! while one of the editor's widgets has focus.

pub mod hex;
pub mod image;
mod panel;
pub mod text;
pub mod widgets;

pub use panel::UnsavedDialog;

use egui::{Event, Id};

use crate::app::SoyuzApp;
use crate::editors::{EditInput, KeyMods};

/// Where the focusable widget ids of the editor drawn last frame are kept (temp data).
fn focus_ids_key() -> Id {
    Id::unique("soyuz.editor.focus_ids")
}

fn set_focus_ids(ctx: &egui::Context, ids: Vec<Id>) {
    ctx.data_mut(|d| d.insert_temp(focus_ids_key(), ids));
}

/// The docked panel (call from the shell before the central panel, in every window): the
/// documents' I/O results are applied in the main window's frame, the panel draws in the
/// window that owns it (`models::owners`).
pub fn show(ui: &mut egui::Ui, app: &mut SoyuzApp) {
    let ctx = ui.ctx().clone();
    if crate::models::workspaces::main_frame(&app.core) {
        crate::editors::pump(&mut app.core, &crate::host::EguiHost::new(&ctx));
    }
    if !crate::models::owners::here(&app.core, crate::models::owners::Panel::Editors) {
        return;
    }
    if !app.core.editors.open || app.core.editors.maximized {
        if !app.core.editors.open {
            set_focus_ids(&ctx, Vec::new());
        }
        return;
    }
    panel::docked(ui, app);
}

/// Draw the editor into the central area (maximized).
pub fn show_central(ui: &mut egui::Ui, rect: egui::Rect, app: &mut SoyuzApp) {
    panel::card(ui, rect, app);
}

/// A widget of the editor has keyboard focus (in the window showing the editor: one that
/// showed it before never keeps its keys).
pub fn owns_keyboard(app: &SoyuzApp, ctx: &egui::Context) -> bool {
    app.core.editors.open
        && crate::models::owners::here(&app.core, crate::models::owners::Panel::Editors)
        && ctx
            .memory(|m| m.focused())
            .is_some_and(|f| ctx.data(|d| d.get_temp::<Vec<Id>>(focus_ids_key())).is_some_and(|ids| ids.contains(&f)))
}

/// Keyboard dispatch while the editor has focus ([`crate::editors::handle_keys`]): the
/// keys it takes are removed before the widgets run.
pub fn handle_keys(app: &mut SoyuzApp, ctx: &egui::Context) {
    let cmd_is_ctrl = app.core.config.keys.cmd_is_ctrl;
    let mut frame = crate::ui::input::frame(ctx, cmd_is_ctrl);
    let out = crate::editors::handle_keys(&mut app.core, &mut frame);
    crate::ui::input::strip(ctx, &frame, cmd_is_ctrl);
    if out.release_focus {
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
    }
    for a in out.actions {
        crate::actions::execute(&mut app.core, a, &crate::host::EguiHost::new(ctx));
    }
}

/// egui modifiers as the editors read them.
pub(crate) fn key_mods(m: egui::Modifiers) -> KeyMods {
    KeyMods { shift: m.shift, command: m.command, alt: m.alt }
}

/// This frame's events for a focused editor view.
pub(crate) fn edit_inputs(ui: &egui::Ui) -> Vec<EditInput> {
    ui.input(|i| {
        i.events
            .iter()
            .filter_map(|ev| {
                Some(match ev {
                    Event::Text(t) => EditInput::Text(t.clone()),
                    Event::Ime(egui::ImeEvent::Commit(t)) => EditInput::ImeCommit(t.clone()),
                    Event::Copy => EditInput::Copy,
                    Event::Cut => EditInput::Cut,
                    Event::Paste(s) => EditInput::Paste(s.clone()),
                    Event::Key { key, pressed: true, modifiers, .. } => {
                        EditInput::Key(crate::ui::keys::key_name(*key), key_mods(*modifiers))
                    }
                    _ => return None,
                })
            })
            .collect()
    })
}
