//! The core's tool queue (`soyuz_appcore::models::tools::{queue, take_queue}`) in the
//! terminal UI, drained every frame like the GUI's `ui/tools/mod.rs`: the model commands
//! (tags, the shelf, Recent, …) apply in the core; the tool windows open their terminal
//! views:
//!
//! | `ToolCmd` | Opens |
//! |---|---|
//! | `Compare` / `CompareProfile` / `SyncProfiles` | the compare screen (`crate::input::compare`) |
//! | `Treemap` / `Duplicates` | a full-screen tool (`crate::input::screens::{open_treemap, open_dupes}`) |
//! | `Split` / `Join` | `crate::input::dialogs::split::{open_split, open_join}` |
//! | `Versions` | `crate::input::dialogs::versions::open` |
//! | `QuickShare` | `crate::input::dialogs::share::open` |
//!
//! The `SOYUZ_OPEN_TOOL` debug hook (`models::tools::debug_open_once`) queues its tools
//! on the first drain; the drop shelf dialog follows the core's shelf flag
//! (`crate::input::dialogs::shelf::sync`).

use soyuz_appcore::models::tools::{self, ToolCmd};

use crate::app::App;
use crate::input::dialogs;

/// Apply and clear the queued tool commands (`App::frame`, after the background pumps
/// and after the input).
pub fn drain(app: &mut App) {
    tools::debug_open_once();
    dialogs::shelf::sync(app);
    let queued = tools::take_queue();
    if queued.is_empty() {
        return;
    }
    let rest = tools::apply_model_cmds(&mut app.core, &app.host, queued);
    for c in rest {
        match c {
            ToolCmd::Compare(left, right) => crate::input::compare::open(app, left, right),
            ToolCmd::CompareProfile(name) => crate::input::compare::open_profile(app, name),
            ToolCmd::SyncProfiles => crate::input::compare::open_profiles(app),
            ToolCmd::Treemap(root) => crate::input::screens::open_treemap(app, root),
            ToolCmd::Duplicates(roots) => crate::input::screens::open_dupes(app, roots),
            ToolCmd::Split(path) => dialogs::split::open_split(app, path),
            ToolCmd::Join(path) => dialogs::split::open_join(app, path),
            ToolCmd::Versions(path, is_dir, current) => dialogs::versions::open(app, path, is_dir, current),
            ToolCmd::QuickShare(paths) => dialogs::share::open(app, paths),
            // Applied by `models::tools::apply_model_cmds`.
            ToolCmd::ShelfAdd(_)
            | ToolCmd::ToggleTag { .. }
            | ToolCmd::ClearTags(_)
            | ToolCmd::RenameTag { .. }
            | ToolCmd::ForgetRecent(_)
            | ToolCmd::VersionAction(_) => {}
        }
    }
}
