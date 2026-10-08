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

/// Set the accessible name of a widget (text fields, combo boxes and sliders, whose
/// visible label is drawn separately or is only a placeholder hint).
pub fn name(resp: &Response, label: &str) {
    with(&resp.ctx, resp.id, |n| n.set_label(label));
}

/// Hide a purely decorative widget (an icon glyph next to a text label) from assistive
/// technology, so screen readers don't announce private-use icon characters.
pub fn decorative(resp: &Response) {
    with(&resp.ctx, resp.id, |n| n.set_hidden());
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

/// A synthetic, hover-only child node of `ui` with a role, a name and an optional
/// description / value (treemap tiles, cards, chart items). Only called while
/// AccessKit is on.
pub fn item(ui: &Ui, id: Id, rect: Rect, role: Role, label: &str, description: Option<&str>) -> Id {
    let r = ui.interact(rect, id, egui::Sense::hover());
    with(ui.ctx(), r.id, |n| {
        n.set_role(role);
        n.set_label(label);
        if let Some(d) = description {
            n.set_description(d);
        }
    });
    r.id
}

/// One row of a tree view (folder tree): name, selection, expansion and depth (1-based
/// level, as screen readers announce it).
#[allow(clippy::too_many_arguments)]
pub fn tree_item(
    ui: &Ui,
    id: Id,
    rect: Rect,
    label: &str,
    selected: bool,
    expanded: Option<bool>,
    depth: usize,
    pos: usize,
    total: usize,
) {
    let r = ui.interact(rect, id, egui::Sense::hover());
    with(ui.ctx(), r.id, |n| {
        n.set_role(Role::TreeItem);
        n.set_label(label);
        n.set_selected(selected);
        if let Some(e) = expanded {
            n.set_expanded(e);
        }
        n.set_level(depth + 1);
        n.set_position_in_set(pos + 1);
        n.set_size_of_set(total);
    });
}

/// A dialog window: role `Dialog` (or `AlertDialog` for confirmations), its title as
/// the name, marked modal when it blocks the rest of the window.
pub fn dialog(ctx: &Context, id: Id, title: &str, alert: bool, modal: bool) {
    with(ctx, id, |n| {
        n.set_role(if alert { Role::AlertDialog } else { Role::Dialog });
        n.set_label(title);
        if modal {
            n.set_modal();
        }
    });
}

/// The embedded terminal: role `Terminal` with the visible screen text as its value,
/// so screen readers can read the output (review mode in NVDA / Narrator).
pub fn terminal(ui: &Ui, id: Id, rect: Rect, label: &str, screen_text: &str) {
    if !active(ui.ctx()) {
        return;
    }
    let r = ui.interact(rect, id, egui::Sense::hover());
    with(ui.ctx(), r.id, |n| {
        n.set_role(Role::Terminal);
        n.set_label(label);
        n.set_value(screen_text);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::accesskit::{Node, NodeId};

    /// Run one frame with AccessKit on and return every node of the tree update.
    fn tree(mut f: impl FnMut(&mut Ui)) -> Vec<(NodeId, Node)> {
        let ctx = Context::default();
        ctx.enable_accesskit();
        // Two frames: the first one can only lay out (sizes are unknown).
        ctx.run_ui(egui::RawInput::default(), &mut f).textures_delta.clear();
        let mut out = ctx.run_ui(egui::RawInput::default(), &mut f);
        out.textures_delta.clear();
        out.platform_output.accesskit_update.take().expect("accesskit update").nodes
    }

    fn find<'a>(nodes: &'a [(NodeId, Node)], role: Role, label: &str) -> &'a Node {
        nodes.iter().map(|(_, n)| n).find(|n| n.role() == role && n.label() == Some(label)).unwrap_or_else(|| {
            let all: Vec<_> = nodes.iter().map(|(_, n)| (n.role(), n.label().map(str::to_string))).collect();
            panic!("no {role:?} named {label:?} in {all:?}")
        })
    }

    #[test]
    fn widget_roles_names_and_states() {
        let nodes = tree(|ui| {
            let b = ui.button("x");
            button(&b, "Geri");
            let t = ui.button("h");
            toggle(&t, "Gizli dosyalar", true);
            let tb = ui.selectable_label(true, "Belgeler");
            tab(&tb, "Belgeler", true);
            let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(100.0, 20.0));
            list_item(ui, Id::new("row-1"), rect, "rapor.pdf", true, 0, 3);
            tree_item(ui, Id::new("tree-1"), rect, "Projeler", false, Some(true), 1, 0, 2);
            item(ui, Id::new("tile-1"), rect, Role::ListItem, "video.mp4", Some("1,2 GB"));
            terminal(ui, Id::new("term"), rect, "Terminal 1", r"PS C:\> dir");
            progress(ui, Id::new("prog"), rect, "Kopyalanıyor", Some(0.25));
        });
        assert!(find(&nodes, Role::Button, "Geri").role() == Role::Button);
        assert!(find(&nodes, Role::Button, "Gizli dosyalar").toggled().is_some());
        assert_eq!(find(&nodes, Role::Tab, "Belgeler").is_selected(), Some(true));
        let row = find(&nodes, Role::ListBoxOption, "rapor.pdf");
        assert_eq!((row.is_selected(), row.position_in_set(), row.size_of_set()), (Some(true), Some(1), Some(3)));
        let t = find(&nodes, Role::TreeItem, "Projeler");
        assert_eq!((t.is_expanded(), t.level()), (Some(true), Some(2)));
        assert_eq!(find(&nodes, Role::ListItem, "video.mp4").description(), Some("1,2 GB"));
        assert_eq!(find(&nodes, Role::Terminal, "Terminal 1").value(), Some(r"PS C:\> dir"));
        assert_eq!(find(&nodes, Role::ProgressIndicator, "Kopyalanıyor").numeric_value(), Some(25.0));
    }

    #[test]
    fn dialogs_are_named_and_modal() {
        let nodes = tree(|ui| {
            egui::Window::new("conflict").id(Id::new("dlg")).show(&ui.ctx().clone(), |ui| {
                dialog(ui.ctx(), ui.id(), "Dosya zaten var", true, true);
                ui.label("…");
            });
        });
        let d = find(&nodes, Role::AlertDialog, "Dosya zaten var");
        assert!(d.is_modal());
    }
}
