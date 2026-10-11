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
    /// "Save workspace as…", with a suggested name.
    SaveWorkspace(String),
    /// "Manage workspaces".
    ManageWorkspaces,
    /// The history graph of `tab`, starting at `pane`.
    TabHistory { tab: crate::state::workspace::TabId, pane: crate::state::pane::PaneId },
    /// Window `id` (with `tabs` tabs) is closing: ask whether to move or close its tabs.
    CloseWindowPrompt { id: crate::state::workspace_windows::WindowId, tabs: usize },
    /// Show the properties of a folder (from the folder tree).
    Properties(std::path::PathBuf),
    /// Ask for the passphrase / recovery key of a locked device container, then mount it
    /// into `pane`.
    UnlockDevice { object: String, name: String, kind: String, pane: crate::state::pane::PaneId },
    /// A mount failed because Windows left the NTFS volume hibernated: explain and offer
    /// a read-only mount.
    OfferReadOnlyMount { dev: Box<soyuz_fs::udisks::BlockDevice>, name: String },
    /// A smart-folder dialog (`models::smart`).
    Smart(super::smart::SmartPrompt),
    /// A remote-location dialog (`models::remote`).
    Remote(super::remote::RemotePrompt),
    /// An archive dialog (`models::archive`).
    Archive(super::archive::ArchivePrompt),
}
