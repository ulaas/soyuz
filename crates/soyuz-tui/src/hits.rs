//! Click targets of the last frame: draw functions record what they put where, the mouse
//! handler (T2.9) looks positions up. Cleared at the start of every draw and on resize, so
//! a click never lands on something from an older layout. Row targets carry the pane's
//! `vis_version`: a click after the listing changed under the pointer is ignored.

use ratatui::layout::{Position, Rect};
use soyuz_appcore::state::pane::PaneId;

/// What is under a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitTarget {
    /// Item `index` (in the pane's visible order) of pane `pane`, as of `vis_version`.
    PaneRow { pane: PaneId, vis_version: u64, index: usize },
    /// Group header `group` of pane `pane`.
    PaneHeader { pane: PaneId, vis_version: u64, group: usize },
    /// Empty space of a pane's list.
    PaneBackground { pane: PaneId },
    /// The details header of column `column` (click to sort).
    PaneColumn { pane: PaneId, column: soyuz_core::Column },
    /// Row `index` of a Miller side column (`parent`, else the child column); resolve it
    /// with `view::pane::miller::entry`.
    MillerRow { pane: PaneId, parent: bool, index: usize },
    /// The splitter between panes (`vertical`: between columns).
    Splitter { vertical: bool },
    /// Tab `index` of the tab strip.
    Tab(usize),
    /// The close mark of tab `index`.
    TabClose(usize),
    /// The "new tab" button.
    NewTab,
    /// Breadcrumb segment `index`.
    Crumb(usize),
    /// The address field (click to edit).
    Address,
    /// Sidebar row `index`.
    SidebarRow(usize),
    /// Filter field.
    Filter,
    /// Palette result `index`.
    PaletteRow(usize),
    /// The palette's input line.
    PaletteInput,
    /// Toast `index` (newest last).
    Toast(usize),
    /// The status line.
    Status,
    /// Button `index` of the open dialog.
    DialogButton(usize),
    /// Field `index` of the open dialog's form.
    DialogField(usize),
    /// Row `index` of a list in the open dialog (open with, recovery, crash reports, …).
    DialogRow(usize),
    /// Item `index` of the open menu at submenu `depth` (0: the menu itself).
    MenuItem { depth: u8, index: usize },
    /// Tab `index` of the bottom panel's tab strip (jobs, history, …).
    BottomTab(usize),
    /// Row `index` of the jobs panel's list.
    JobRow(usize),
    /// A button on row `row` of the jobs panel.
    JobButton { row: usize, button: JobButton },
    /// The preview pane's content (wheel / click to focus it).
    PreviewScroll,
    /// Row `index` of the preview's archive listing.
    ArchiveRow(usize),
    /// Tab `index` of the editor's document tabs.
    EditorTab(usize),
    /// The close mark of editor tab `index`.
    EditorClose(usize),
    /// The editor's text area of document `doc`: the rect's top-left cell shows line `top`,
    /// display column `left` (the click's line and column follow from its position).
    EditorCell { doc: u64, top: usize, left: usize },
    /// The Quick Look overlay (a press outside it closes it).
    QuickLook,
    /// The inline rename field.
    RenameField,
    // ---- Phase T5 + T6 ----
    /// A full-screen overlay (the compare screen, the settings editor) under everything it
    /// draws: pushed first, so a click never falls through to the panes below.
    Overlay,
    /// Row `row` of a dialog's multi-line text area (`DialogVm::text`; the column follows
    /// from the click's position in the row).
    DialogText(u16),
    /// The search bar's query field (`input::search`).
    SearchField,
    /// Option pill `index` of the search bar.
    SearchPill(u8),
    /// A button of the search bar.
    SearchButton(SearchButton),
    /// Button `index` of the search status row (cancel, relevance, smart folder, refine).
    SearchStatusButton(u8),
    /// The tab of terminal session `id` in the terminal pane's strip (`input::terminal`).
    TermTab(u64),
    /// The close mark of terminal session `id`.
    TermTabClose(u64),
    /// "New session" in the strip.
    TermNew,
    /// "Open in an external terminal" in the strip.
    TermExternal,
    /// The terminal grid; the rect's top-left cell is grid cell (`row`, `col`).
    TermCell { row: u16, col: u16 },
    /// Answer `index` of the OSC 52 clipboard bar (allow once, session, deny).
    TermClip(u8),
    /// Tab `index` of the AI panel (chat, recipes, audit, …; `input::ai`).
    AiTab(u8),
    /// The AI composer.
    AiComposer,
    /// A button of the AI panel's header / composer.
    AiButton(AiButton),
    /// Plan card `id` of task `task`.
    AiCard { id: u64, task: u64 },
    /// A button on plan card `id` of task `task`.
    AiCardButton { id: u64, task: u64, button: CardButton },
    /// Row `index` of the AI panel's list pages (recipes, audit) or a card's item list.
    AiRow(usize),
    /// Row `index` of the composer's mention / slash popup.
    AiMention(usize),
    /// "Undo everything" of MCP client row `index`.
    AiMcpUndo(usize),
    /// The chat transcript (wheel scrolls it).
    AiChat,
    /// Field `index` of the compare screen's setup (left, right, method, preset; `input::compare`).
    CompareField(u8),
    /// Result row `index` of the compare screen.
    CompareRow(usize),
    /// Button `index` of the compare screen.
    CompareButton(u8),
    /// Category chip `index` of the compare screen's filter.
    CompareChip(u8),
    /// Saved sync profile `index` on the compare screen.
    CompareProfile(usize),
    /// Section `index` of the settings editor's navigation (`input::settings`).
    SettingsNav(usize),
    /// Row `index` of the settings editor's list.
    SettingsRow(usize),
    /// The settings editor's search field.
    SettingsSearch,
    /// The settings editor's close mark.
    SettingsClose,
    /// Action row `index` of the key bindings page.
    SettingsKeyRow(usize),
    /// Chord chip `chip` of key bindings row `row`.
    SettingsKeyChip { row: usize, chip: usize },
    /// "Add a chord" of key bindings row `index`.
    SettingsKeyAdd(usize),
    // ---- Phase T7 B (devices; `input::sidebar::on_mouse`) ----
    /// The eject mark at the end of sidebar row `index` (a mounted volume with an eject
    /// plan). Device rows themselves are `SidebarRow`s.
    SidebarEject(usize),
    // ---- Phase T7 C (the full-screen tools; `input::screens::on_mouse`) ----
    /// Button `index` of the shown tool screen (toolbar, footer; the screen numbers them).
    ScreenButton(u8),
    /// Chip `index` of the shown tool screen (dupes: minimum size, keep rules).
    ScreenChip(u8),
    /// Field `index` of the shown tool screen (dupes: the add-folder field).
    ScreenField(u8),
    /// Tab `index` of the shown tool screen (treemap side list: contents, largest).
    ScreenTab(u8),
    /// The shown tool screen's close mark.
    ScreenClose,
    /// A scrollable area `index` of the shown tool screen (the wheel scrolls it).
    ScreenArea(u8),
    /// Treemap tile of tree node `id` (frames before children: the deepest wins).
    TreemapTile(u32),
    /// Treemap breadcrumb of tree node `id`.
    TreemapCrumb(u32),
    /// Row `index` of the treemap's side list.
    TreemapSide(usize),
    /// Row `index` of the duplicate finder's list (group headers and files).
    DupesRow(usize),
    /// Root chip `index` of the duplicate finder (its × removes it).
    DupesRoot(usize),
    /// Group header `index` of the duplicate finder (folds).
    DupesHeader(usize),
    /// Button `index` of a tool screen's inline confirm bar (move to trash, cancel).
    ConfirmBar(u8),
    // ---- Phase T7 places (the Trash and Computer pages; `input::list::places::on_mouse`) ----
    /// Row `index` of pane `pane`'s Trash / Computer page (a trashed item, a volume).
    PlacesRow { pane: PaneId, index: usize },
    /// Header button `button` of pane `pane`'s Trash / Computer page.
    PlacesButton { pane: PaneId, button: u8 },
    /// The rest of pane `pane`'s Trash / Computer page.
    PlacesArea { pane: PaneId },
}

/// A button of the search bar ([`HitTarget::SearchButton`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchButton {
    /// Run (or re-run) the search.
    Run,
    /// Save the search.
    Save,
    /// Close the bar.
    Close,
}

/// A button of the AI panel's header / composer ([`HitTarget::AiButton`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiButton {
    /// Send the composer's text.
    Send,
    /// Stop the running task.
    Stop,
    /// Start a new chat.
    NewChat,
    /// Close the panel.
    Close,
    /// Pick the profile / route.
    Profile,
    /// The assistant's settings.
    Settings,
    /// Approve every pending card.
    ApproveAll,
}

/// A button on an AI plan card ([`HitTarget::AiCardButton`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardButton {
    Approve,
    Reject,
    /// Approve every pending card of the same tool.
    ApproveKind,
    /// Edit the plan's names / arguments.
    Edit,
    /// Show or hide every item.
    Expand,
    Undo,
    /// Save the task as a recipe.
    SaveRecipe,
    /// A cost / confirmation question: go on.
    Go,
    /// A cost / confirmation question: stop.
    Cancel,
}

/// A button on a jobs panel row ([`HitTarget::JobButton`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobButton {
    Pause,
    Resume,
    Cancel,
    /// Answer the job's waiting prompt.
    Resolve,
    Undo,
    /// Remove a finished job from the list.
    Dismiss,
    /// Open the job's details.
    Details,
    /// Run the failed items again.
    Retry,
    /// Redo an undone history entry.
    Redo,
    /// Close the panel (the `row` is not used).
    Close,
    /// The jobs tab's toolbar (the `row` is not used).
    PauseAll,
    ResumeAll,
    CancelAll,
    ClearFinished,
    /// Open the job's "More" menu (priority, reorder, start now).
    More,
}

/// The targets of one frame, in drawing order.
#[derive(Clone, Debug, Default)]
pub struct Hits {
    items: Vec<(Rect, HitTarget)>,
    /// The text boxes of dialog text fields (`DialogField(i)` rows), so a click can put
    /// the cursor under the pointer ([`Hits::field_text_at`]).
    field_texts: Vec<(Rect, usize)>,
    /// Where the terminal's cursor goes in screen-reader mode (Phase T8): the focused
    /// element's cell, recorded by its draw function ([`Hits::set_caret`]; the last one
    /// recorded wins, so overlays drawn later win).
    caret: Option<Position>,
}

impl Hits {
    pub fn clear(&mut self) {
        self.items.clear();
        self.field_texts.clear();
        self.caret = None;
    }

    /// Record the focused element's cell (a list's cursor row, a text field's caret, a
    /// menu's highlighted item, a dialog's focused button). Only the element that has the
    /// keyboard records one; `view::draw` puts the terminal's cursor there in
    /// screen-reader mode.
    pub fn set_caret(&mut self, pos: Position) {
        self.caret = Some(pos);
    }

    /// The caret recorded last this frame.
    pub fn caret(&self) -> Option<Position> {
        self.caret
    }

    /// Record the text box of dialog text field `field` (drawn inside its
    /// `DialogField(field)` row).
    pub fn push_field_text(&mut self, rect: Rect, field: usize) {
        if !rect.is_empty() {
            self.field_texts.push((rect, field));
        }
    }

    /// The text box at cell `(x, y)` and its field, when the top-most target there is
    /// that field's row.
    pub fn field_text_at(&self, x: u16, y: u16) -> Option<(Rect, usize)> {
        let p = Position { x, y };
        let (r, i) = self.field_texts.iter().rev().find(|(r, _)| r.contains(p)).copied()?;
        (self.at(x, y) == Some(HitTarget::DialogField(i))).then_some((r, i))
    }

    /// Record `target` at `rect` (empty rects are skipped).
    pub fn push(&mut self, rect: Rect, target: HitTarget) {
        if !rect.is_empty() {
            self.items.push((rect, target));
        }
    }

    /// The top-most target at cell `(x, y)`: the one drawn last.
    pub fn at(&self, x: u16, y: u16) -> Option<HitTarget> {
        let p = Position { x, y };
        self.items.iter().rev().find(|(r, _)| r.contains(p)).map(|(_, t)| *t)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Every recorded target with its rect (tests, debugging).
    pub fn iter(&self) -> impl Iterator<Item = &(Rect, HitTarget)> {
        self.items.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_drawn_wins() {
        let mut h = Hits::default();
        h.push(Rect::new(0, 0, 10, 10), HitTarget::Status);
        h.push(Rect::new(2, 2, 2, 1), HitTarget::Tab(1));
        h.push(Rect::new(5, 5, 0, 3), HitTarget::NewTab);
        assert_eq!(h.len(), 2);
        assert_eq!(h.at(2, 2), Some(HitTarget::Tab(1)));
        assert_eq!(h.at(4, 2), Some(HitTarget::Status));
        assert_eq!(h.at(10, 0), None);
        h.clear();
        assert!(h.is_empty() && h.at(2, 2).is_none());
    }

    #[test]
    fn last_caret_wins_until_cleared() {
        let mut h = Hits::default();
        assert_eq!(h.caret(), None);
        h.set_caret(Position { x: 1, y: 2 });
        h.set_caret(Position { x: 3, y: 4 });
        assert_eq!(h.caret(), Some(Position { x: 3, y: 4 }));
        h.clear();
        assert_eq!(h.caret(), None);
    }
}
