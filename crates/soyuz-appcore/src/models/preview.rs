//! The preview / info pane as a model: whether it is open, its width and mode. What it
//! renders (loaded content, the two-file diff) belongs to the frontend.

/// Preview pane state (lives in `UiState`).
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewModel {
    pub open: bool,
    pub width: f32,
    /// Info pane mode (`ToggleInfoPane`): only the metadata section, no content viewer.
    pub info_only: bool,
    /// `ToggleHexPreview` was pressed: the frontend flips its hex view on the next draw
    /// (pressed twice before that cancels out).
    pub hex_toggle: bool,
}

impl Default for PreviewModel {
    fn default() -> Self {
        PreviewModel { open: false, width: 340.0, info_only: false, hex_toggle: false }
    }
}

impl PreviewModel {
    pub fn new(open: bool) -> Self {
        PreviewModel { open, ..Default::default() }
    }

    /// `TogglePreviewPane`: from the info pane it switches to the full preview.
    pub fn toggle(&mut self) {
        if self.open && self.info_only {
            self.info_only = false;
            return;
        }
        self.set_open(!self.open);
    }

    /// `ToggleInfoPane`: the same pane showing metadata only.
    pub fn toggle_info(&mut self) {
        if self.open && !self.info_only {
            self.info_only = true;
            return;
        }
        self.info_only = true;
        self.set_open(!self.open);
    }

    fn set_open(&mut self, open: bool) {
        self.open = open;
        if !open {
            self.info_only = false;
        }
    }

    /// `ToggleHexPreview`.
    pub fn toggle_hex(&mut self) {
        self.hex_toggle = !self.hex_toggle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_and_preview_toggles() {
        let mut p = PreviewModel::default();
        p.toggle_info();
        assert!(p.open && p.info_only);
        p.toggle();
        assert!(p.open && !p.info_only);
        p.toggle();
        assert!(!p.open && !p.info_only);
        p.toggle_hex();
        p.toggle_hex();
        assert!(!p.hex_toggle);
    }
}
