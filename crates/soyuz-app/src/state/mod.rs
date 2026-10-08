//! Application state (single-threaded, owned by the UI thread): workspace → tabs →
//! panes, plus the pure logic modules they use.

pub mod crumbs;
pub mod history;
pub mod listing;
pub mod pane;
pub mod selection;
pub mod session;
pub mod view_memory;
pub mod workspace;
pub mod workspace_store;
pub mod workspace_sync;
pub mod workspace_timeline;
pub mod workspace_visits;
pub mod workspace_windows;
