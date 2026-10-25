//! Width limits for the panels docked on the right (editor, assistant, preview, activity,
//! shelf): each one stops growing where the file list would get narrower than
//! `[layout] min_file_list_width`, so several open at once never squeeze the listing.

use std::ops::RangeInclusive;

use egui::Ui;

/// No docked panel gets narrower than this, even in a window too small for the reserve.
const PANEL_FLOOR: f32 = 160.0;

/// The width range for a right-docked panel whose own limits are `min..=max`, shown in
/// `ui` (the space the panels drawn before it left), keeping `reserve` points for the
/// file list.
pub fn right_range(ui: &Ui, min: f32, max: f32, reserve: f32) -> RangeInclusive<f32> {
    let room = ui.available_width() - reserve.max(0.0);
    let hi = max.min(room).max(PANEL_FLOOR.min(max));
    min.min(hi)..=hi
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range_in(width: f32, min: f32, max: f32, reserve: f32) -> RangeInclusive<f32> {
        let ctx = egui::Context::default();
        let mut out = 0.0..=0.0;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 600.0))),
            ..Default::default()
        };
        ctx.run_ui(input, |ui| out = right_range(ui, min, max, reserve)).textures_delta.clear();
        out
    }

    #[test]
    fn panels_leave_room_for_the_file_list() {
        // Plenty of room: the panel's own limits.
        assert_eq!(range_in(1600.0, 320.0, 760.0, 320.0), 320.0..=760.0);
        // 700 pt left, 320 reserved: at most 380.
        assert_eq!(range_in(700.0, 320.0, 760.0, 320.0), 320.0..=380.0);
        // Less room than the panel's minimum: the minimum gives way too.
        assert_eq!(range_in(500.0, 320.0, 760.0, 320.0), 180.0..=180.0);
        // Never below the floor, however small the window.
        assert_eq!(range_in(300.0, 320.0, 760.0, 320.0), 160.0..=160.0);
        // Reserve 0 = the old behaviour.
        assert_eq!(range_in(500.0, 320.0, 760.0, 0.0), 320.0..=500.0);
    }
}
