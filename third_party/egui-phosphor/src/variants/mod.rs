// Every variant uses the same codepoints, so they share one table and each
// variant module re-exports it. `update.py` verifies this still holds.
mod codepoints;

#[cfg(feature = "fill")]
pub mod fill;
#[cfg(feature = "regular")]
pub mod regular;

#[cfg(not(any(feature = "regular", feature = "fill")))]
compile_error!(
    "At least one font variant must be selected as a crate feature. When in doubt, use default features."
);

pub mod bytes {
    #[cfg(feature = "regular")]
    pub mod regular {
        pub const FONT: &[u8] = include_bytes!("../../res/Phosphor.ttf");
    }
    #[cfg(feature = "fill")]
    pub mod fill {
        pub const FONT: &[u8] = include_bytes!("../../res/Phosphor-Fill.ttf");
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Variant {
    #[cfg(feature = "regular")]
    Regular,
    #[cfg(feature = "fill")]
    Fill,
}

impl Variant {
    pub fn font_bytes(&self) -> &'static [u8] {
        match self {
            #[cfg(feature = "regular")]
            Variant::Regular => bytes::regular::FONT,
            #[cfg(feature = "fill")]
            Variant::Fill => bytes::fill::FONT,
        }
    }

    pub fn font_data(&self) -> egui::FontData {
        egui::FontData::from_static(self.font_bytes())
    }
}
