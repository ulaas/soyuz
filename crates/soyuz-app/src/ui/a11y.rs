//! Accessibility helpers (AccessKit through egui). Most Soyuz widgets are custom-painted,
//! so they get no role or name by default; these helpers attach both (and states such as
//! toggled / selected / progress) to a widget's [`Response`] or to a synthetic node.
//!
//! Everything here is a no-op unless a screen reader / UI automation client has turned
//! AccessKit on (egui then builds the tree each frame).

use egui::accesskit::{Node, Role};
use egui::{Context, Id, Rect, Response, Ui, WidgetInfo, WidgetType};

/// Whether AccessKit tree building is active this frame.
pub fn active(ctx: &Context) -> bool {
    // The root node always exists while AccessKit is on, so probing it adds nothing.
    ctx.accesskit_node_builder(egui::accesskit_root_id(), |_| ()).is_some()
}

/// A push button with an accessible name (icon buttons use their tooltip text).
pub fn button(resp: &Response, label: &str) {
    let enabled = resp.enabled();
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
}

/// A toggle button (pressed / not pressed).
pub fn toggle(resp: &Response, label: &str, on: bool) {
    let enabled = resp.enabled();
    resp.widget_info(|| WidgetInfo::selected(WidgetType::Button, enabled, on, label));
}

/// A tab in a tab strip (selected = the active tab).
pub fn tab(resp: &Response, label: &str, selected: bool) {
    let enabled = resp.enabled();
    resp.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, enabled, selected, label));
    with(&resp.ctx, resp.id, |n| {
        n.set_role(Role::Tab);
        n.set_selected(selected);
    });
}

/// A menu item.
pub fn menu_item(resp: &Response, label: &str) {
    let enabled = resp.enabled();
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    with(&resp.ctx, resp.id, |n| n.set_role(Role::MenuItem));
}

/// Edit the AccessKit node of `id` (no-op when AccessKit is off).
pub fn with(ctx: &Context, id: Id, f: impl FnOnce(&mut Node)) {
    ctx.accesskit_node_builder(id, f);
}

/// Give a container (the file list, a panel, a dialog) a role and a name.
pub fn container(ctx: &Context, id: Id, role: Role, label: &str) {
    with(ctx, id, |n| {
        n.set_role(role);
        n.set_label(label);
    });
}

/// One row of a virtualized list: a hover-only child of `ui` so it is parented to the
/// list, carrying its name, selection and position. Only called while AccessKit is on.
pub fn list_item(ui: &Ui, id: Id, rect: Rect, label: &str, selected: bool, pos: usize, total: usize) {
    let r = ui.interact(rect, id, egui::Sense::hover());
    with(ui.ctx(), r.id, |n| {
        n.set_role(Role::ListBoxOption);
        n.set_label(label);
        n.set_selected(selected);
        n.set_position_in_set(pos + 1);
        n.set_size_of_set(total);
    });
}

/// A determinate (`Some(0..=1)`) or indeterminate progress indicator.
pub fn progress(ui: &Ui, id: Id, rect: Rect, label: &str, fraction: Option<f32>) {
    if !active(ui.ctx()) {
        return;
    }
    let r = ui.interact(rect, id, egui::Sense::hover());
    with(ui.ctx(), r.id, |n| {
        n.set_role(Role::ProgressIndicator);
        n.set_label(label);
        if let Some(f) = fraction {
            n.set_min_numeric_value(0.0);
            n.set_max_numeric_value(100.0);
            n.set_numeric_value(f64::from((f * 100.0).clamp(0.0, 100.0)));
            n.set_value(format!("{:.0}%", f * 100.0));
        }
    });
}
