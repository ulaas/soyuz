//! Requests from app logic to whichever frontend is running (Phase T0): things only a
//! frontend can do, such as opening its settings window. Core code pushes onto
//! `SoyuzApp::frontend`; the frontend drains the queue every frame.

use super::settings::Section;

#[derive(Clone, Debug, PartialEq)]
pub enum FrontendCmd {
    /// Open the settings, optionally on a page.
    OpenSettings(Option<Section>),
    /// Close the settings (writes a pending `config.toml` save).
    CloseSettings,
    /// A smart-folder dialog (`models::smart`).
    Smart(super::smart::SmartPrompt),
    /// A remote-location dialog (`models::remote`).
    Remote(super::remote::RemotePrompt),
    /// An archive dialog (`models::archive`).
    Archive(super::archive::ArchivePrompt),
}
