//! The `config.toml` schema. Every section is `#[serde(default)]`, so any
//! subset of keys is a valid file. Defaults live in `defaults.rs`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::columns::{Column, ViewMode};
use crate::size::SizeStyle;
use crate::time::DateFormat;

use super::keys::KeyPreset;

/// Current schema version written by [`save_pretty`](super::save_pretty).
pub const SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Schema version (`version` is accepted as an alias); see `config::migrate`.
    #[serde(alias = "version")]
    pub schema: u32,
    pub general: General,
    pub appearance: Appearance,
    /// Font roles (`[fonts]`, see `config::fonts`).
    pub fonts: super::fonts::Fonts,
    pub theme: ThemeSection,
    pub layout: Layout,
    /// Windows, tab groups, saved workspaces, sync browsing, history graph, timeline.
    pub workspace: WorkspaceConfig,
    pub toolbar: Toolbar,
    pub context_menu: ContextMenu,
    pub sidebar: Sidebar,
    /// Custom display names of the eight color tags.
    pub tags: TagsSection,
    pub columns: Columns,
    pub behaviour: Behaviour,
    pub jobs: Jobs,
    pub terminal: Terminal,
    pub archive: Archive,
    pub search: Search,
    pub preview: Preview,
    pub editor: Editor,
    pub git: Git,
    /// Scheduled sync profiles, two-way sync and previous versions.
    pub sync: SyncSection,
    /// Manual update check against GitHub Releases.
    pub updates: Updates,
    /// Startup and rendering performance.
    pub performance: Performance,
    /// Remote locations (SFTP, WebDAV, FTP/FTPS, S3) and network shares.
    pub remote: RemoteSection,
    pub keys: Keys,
    pub ai: super::ai::Ai,
    pub bookmarks: Vec<Bookmark>,
    pub custom_commands: Vec<CustomCommand>,
    /// Rhai scripts (`scripts/*.rhai`).
    pub scripting: super::automation::Scripting,
    /// Rule-based automations.
    pub automation: super::automation::Automation,
    /// The `soyuz cp|mv|rm|…` companion.
    pub cli: super::automation::Cli,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OpenFoldersIn {
    #[default]
    SameTab,
    NewTab,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// `"home"` | `"last"` | `"<path>"`.
    pub start_location: String,
    /// Open folders in the `same_tab` or a `new_tab`.
    pub open_folders_in: OpenFoldersIn,
    /// Open files and folders with a single click instead of a double click.
    pub single_click_open: bool,
    /// Confirm moving to trash.
    pub confirm_delete: bool,
    /// Confirm before deleting permanently (Shift+Del).
    pub confirm_permanent_delete: bool,
    /// Show hidden files and folders (also toggled with Ctrl+H).
    pub show_hidden: bool,
    /// Windows system files.
    pub show_system: bool,
    /// Show file name extensions.
    pub show_extensions: bool,
    /// Natural sort: `file2` before `file10`.
    pub natural_sort: bool,
    /// Sort names without regard to letter case.
    pub case_insensitive_sort: bool,
    /// List folders before files.
    pub folders_first: bool,
    pub size_style: SizeStyle,
    pub date_format: DateFormat,
    /// View for folders without a remembered view.
    pub default_view: ViewMode,
    /// Longest gap between the two clicks of a double click (ms).
    pub double_click_speed_ms: u32,
    /// UI language: `"auto"` (the OS display language), `"en-US"` or `"tr-TR"`. Switches live.
    pub language: String,
    /// Apply Windows file-name rules on every OS (see `validate_name_with`).
    pub strict_windows_names: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Compact,
    #[default]
    Cozy,
    Comfortable,
    /// Large touch targets (rows, buttons and menu items grow to finger size).
    Touch,
}

impl Density {
    /// List row height in points.
    pub fn row_height(self) -> f32 {
        match self {
            Density::Compact => 24.0,
            Density::Cozy => 28.0,
            Density::Comfortable => 32.0,
            Density::Touch => 40.0,
        }
    }

    /// Height of interactive controls (buttons, menu rows) in points.
    pub fn control_height(self) -> f32 {
        match self {
            Density::Touch => 36.0,
            _ => 24.0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IconStyle {
    #[default]
    Colored,
    Mono,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// `"system"` | `"dark"` | `"light"` | `"<name>"` (user theme in `themes/`).
    pub theme: String,
    /// Row and control height: `compact`, `cozy`, `comfortable` or `touch`.
    pub density: Density,
    /// Interface zoom factor (Ctrl+= / Ctrl+- / Ctrl+0 change it).
    pub ui_scale: f32,
    /// Legacy (schema 1): read and moved into `[fonts.ui]` on load, never written.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub font: String,
    /// Legacy (schema 1): moved into `[fonts.terminal]` / `code` / `hex` on load.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub mono_font: String,
    /// Base UI text size in points.
    pub font_size: f32,
    /// Turn off animations (transitions and springs jump to their end state).
    pub reduced_motion: bool,
    /// High-contrast palette (stronger text, borders and focus rings).
    pub high_contrast: bool,
    /// `colored` file-type icons or `mono` (single color; folder colors stay).
    pub icon_style: IconStyle,
    /// Picture and video thumbnails in the grid and gallery views.
    pub show_thumbnails: bool,
    /// Default thumbnail edge in the grid view (points).
    pub thumbnail_size: u32,
    /// Sidebar width in points (dragging its edge updates it).
    pub sidebar_width: f32,
    /// Soyuz draws its own title bar with the tabs in it; off = the system title bar.
    pub custom_titlebar: bool,
    /// Windows 11 window backdrop behind translucent surfaces (no-op elsewhere; read at start).
    pub backdrop: Backdrop,
    /// Windows 11 Snap Layouts flyout when hovering the maximize button (custom title bar).
    pub snap_layouts: bool,
    /// Dialogs, the settings window and menus that do not fit the window open as their own
    /// OS windows (movable outside the main window and onto other monitors); off = drawn
    /// inside the window (menus are then clipped to it).
    pub native_dialogs: bool,
    /// System fallback fonts for scripts Inter lacks (CJK, Hangul, emoji, Arabic / Hebrew).
    pub fallback_fonts: FallbackFonts,
    /// Color theme family: a built-in id (`orbit`, `nord`, `solarized`, `rose-pine`,
    /// `high-contrast`) or the file stem of a user theme in `themes/*.toml`. Each theme
    /// has a dark and a light variant; `theme` picks which one shows.
    pub color_theme: String,
    /// `"theme"` (the theme's own accent) | `"system"` (the OS accent color, followed
    /// live) | `"#RRGGBB"`.
    pub accent: String,
    /// Extension label badges on file icons in the grid / gallery views.
    pub extension_badges: bool,
    /// 2×2 mosaic of the first images inside folders (grid / gallery views).
    pub folder_thumbnails: bool,
    /// First-frame video thumbnails through an `ffmpeg` binary on `PATH` (when found).
    pub video_thumbnails: bool,
    /// First-page PDF thumbnails through pdfium (when `pdfium.dll` is found, see
    /// `[preview] pdfium_dir`); without it PDFs keep their icon.
    pub pdf_thumbnails: bool,
    /// Reuse the thumbnails Explorer already made (Windows `thumbcache_*.db`, read-only)
    /// before decoding a file ourselves.
    pub os_thumbnail_cache: bool,
    /// File icon pack: `""` = the built-in Phosphor glyphs, `"flat"` = the built-in
    /// generated pack, else the folder name of a user pack in `icons/<name>/pack.toml`.
    pub icon_pack: String,
    /// On-disk thumbnail cache in `thumbs/` (MiB; 0 = off). Thumbnails survive restarts.
    pub thumb_disk_cache_mib: u64,
}

/// `[appearance] backdrop`: the system material drawn behind the window (Windows 11).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Backdrop {
    /// Opaque theme colors.
    #[default]
    None,
    /// Mica: the desktop wallpaper, heavily blurred and tinted.
    Mica,
    /// Acrylic: a blur of whatever is behind the window.
    Acrylic,
}

/// When system fallback fonts (CJK / Hangul / emoji / Arabic / Hebrew, from the OS font folder) load.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FallbackFonts {
    /// When a file name needs them (loaded on a background thread).
    #[default]
    Auto,
    /// All of them in the background right after startup (also covers terminal output).
    Startup,
    /// Never (missing glyphs render as boxes).
    Off,
}

/// `[theme.colors]`: design token name (e.g. `"accent"`, `"bg.window"`) → hex.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ThemeSection {
    pub colors: BTreeMap<String, String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    #[default]
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TabStripMode {
    #[default]
    Always,
    Multiple,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActivityPlacement {
    #[default]
    Right,
    Bottom,
    Floating,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    /// Show the sidebar.
    pub sidebar: bool,
    /// Show the preview pane.
    pub preview_pane: bool,
    /// Start new tabs with two panes.
    pub dual_pane: bool,
    /// Side by side (`vertical` split) or stacked (`horizontal`).
    pub dual_pane_orientation: Orientation,
    /// Show the status bar.
    pub status_bar: bool,
    /// Show the tab strip `always` or only with `multiple` tabs.
    pub tab_strip: TabStripMode,
    /// Height of the terminal / bottom panel in points (120-720).
    pub terminal_height: f32,
    /// Where the activity (jobs) panel docks.
    pub activity_panel: ActivityPlacement,
}

/// What "New window" (Ctrl+N) opens.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NewWindowMode {
    /// Another window of this Soyuz (shares jobs, services and settings).
    #[default]
    InProcess,
    /// A separate Soyuz process (`soyuz --new-window`).
    Process,
}

/// Closing a window that still has tabs.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CloseWindowTabs {
    /// Ask whether to move the tabs to another window or close them.
    #[default]
    Ask,
    /// Move them to another window.
    Merge,
    /// Close them.
    Close,
}

/// `[workspace]`: windows, tab groups, saved workspaces, synchronized browsing, the
/// per-tab history graph and the timeline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceConfig {
    /// What "New window" (Ctrl+N) opens.
    pub new_window: NewWindowMode,
    /// Drag a tab out of the tab strip to open it in its own window.
    pub tear_off: bool,
    /// How far (points) a tab has to be dragged away from the strip to tear off.
    pub tear_off_distance: f32,
    /// Closing a window that still has tabs.
    pub close_window_tabs: CloseWindowTabs,
    /// Reopen the other windows (with their positions) when the session is restored.
    pub restore_windows: bool,
    /// Synchronized browsing follows any relative move (sibling folders, several levels),
    /// not only "into a child" / "up one level".
    pub sync_relative: bool,
    /// Places remembered per tab in the "where was I" history graph.
    pub history_graph_limit: usize,
    /// The timeline covers this many days (today, yesterday, this week, …).
    pub timeline_days: u32,
    /// Recently visited folders scanned (one level) for changed files.
    pub timeline_scan_folders: usize,
    /// Most files the timeline lists.
    pub timeline_max_items: usize,
    /// Ask the search index (when one is available) for changed files too.
    pub timeline_use_index: bool,
}

/// Editable toolbar: action names plus `"|"` (separator), `"spacer"`,
/// `"breadcrumb"`, `"view_menu"`, `"sort_menu"`, `"activity"`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Toolbar {
    /// Toolbar items, left to right.
    pub items: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ContextMenu {
    /// Action names to hide.
    pub hide: Vec<String>,
    /// Optional explicit order; unknown names ignored.
    pub order: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarSection {
    Favorites,
    Devices,
    /// Portal: folders pinned by dropping them on the section (`[[sidebar.portal]]`).
    Portal,
    /// OneDrive / CloudStorage folders detected via env + known dirs.
    Cloud,
    Tags,
    Network,
    /// "Other": Recent, Timeline and Trash.
    Trash,
    /// The disk tree as a section (only drawn in `mode = "both"`).
    Tree,
    /// Smart folders (saved live searches). Shown last when not listed.
    Smart,
}

/// What the sidebar shows: the places sections, the disk tree, or both. The icon bar at
/// the top of the sidebar switches between `places` and `tree`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SidebarMode {
    /// Favorites, devices, portal, tags … (no disk tree).
    #[default]
    Places,
    /// Only the disk tree (drives + home), filling the sidebar.
    Tree,
    /// The places sections with the disk tree as the `tree` section.
    Both,
}

/// A `[[sidebar.portal]]` entry: a folder pinned to the Portal section.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PortalPin {
    /// The pinned folder.
    pub path: PathBuf,
    /// Label shown in the sidebar; empty = the folder's name.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sidebar {
    /// Sidebar sections in display order; leave one out to hide it.
    pub sections: Vec<SidebarSection>,
    /// `places` | `tree` | `both`.
    pub mode: SidebarMode,
    /// Expand the tree to the active pane's folder and highlight it.
    pub tree_follow: bool,
    /// Hovering a drag over a collapsed tree folder expands it after this delay
    /// (milliseconds; 0 = never).
    pub tree_drag_expand_ms: u32,
    /// Folders pinned to the Portal section (drop folders on it to add), in order.
    pub portal: Vec<PortalPin>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Columns {
    /// Columns of the details view, in order.
    pub details: Vec<Column>,
    /// Column name (`snake_case`) → width in points.
    pub widths: BTreeMap<String, f32>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RenameSelects {
    #[default]
    Stem,
    All,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PasteConflict {
    #[default]
    Ask,
    Overwrite,
    Skip,
    KeepBoth,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DragDefault {
    /// Same device = move, otherwise copy.
    #[default]
    Auto,
    Copy,
    Move,
    Ask,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TypeAhead {
    #[default]
    Jump,
    Filter,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MiddleClick {
    #[default]
    NewTab,
    OtherPane,
    Nothing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Behaviour {
    /// Inline rename preselects the name `stem` (without extension) or `all`.
    pub rename_selects: RenameSelects,
    /// Default answer when a pasted name already exists (`ask` shows the conflict dialog).
    pub paste_conflict: PasteConflict,
    /// Drag and drop default operation.
    pub drag_default: DragDefault,
    /// Remember view, sort and columns per folder (`views.toml`).
    pub remember_folder_views: bool,
    /// Reopen the tabs, panes and windows of the last session at startup.
    pub restore_session: bool,
    /// Watch open folders and refresh them when files change.
    pub auto_refresh: bool,
    /// Collect file-system change events for this long before refreshing (ms).
    pub watch_debounce_ms: u64,
    /// Typing in the file list `jump`s to the first match or `filter`s the list.
    pub type_ahead: TypeAhead,
    /// Middle-clicking a folder opens it in a `new_tab`, the `other_pane`, or does `nothing`.
    pub middle_click: MiddleClick,
    /// Hovering a folder during a drag opens it (Explorer / Finder "spring-loaded folders").
    pub spring_loaded_folders: bool,
    /// Hover time before a spring-loaded folder opens (ms).
    pub spring_load_delay_ms: u32,
    /// Dragging items out of the window hands them to other apps (desktop, Explorer,
    /// editors) as files (Windows OLE drag and drop).
    pub drag_out: bool,
}

/// Mirrors `soyuz_fs::Preserve` bits (core cannot depend on fs).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreserveItem {
    Times,
    Attrs,
    Xattr,
    Acl,
    Owner,
    Sparse,
}

/// Mirrors `soyuz_fs::Reflink`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReflinkPref {
    #[default]
    Auto,
    Never,
    Always,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrashUnsupportedPolicy {
    #[default]
    Ask,
    Permanent,
    Skip,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Jobs {
    /// Jobs that may run at the same time.
    pub max_parallel: usize,
    /// Queue jobs that touch the same drive one after another (faster on spinning disks).
    pub per_device_serial: bool,
    /// "Apply to all" in a conflict dialog lasts for one job only; off = it carries over to the following jobs.
    pub reset_apply_all_between_jobs: bool,
    /// Hash-compare every copied file with its source (BLAKE3) after the copy.
    pub verify_after_copy: bool,
    /// Copy buffer size (MiB).
    pub copy_buffer_mib: u64,
    /// Files larger than this are copied with unbuffered I/O, bypassing the OS cache (MiB).
    pub unbuffered_threshold_mib: u64,
    /// Metadata copied along with file data.
    pub preserve: Vec<PreserveItem>,
    /// Copy-on-write clones (ReFS block cloning, btrfs / APFS reflinks): `auto`, `never` or `always`.
    pub reflink: ReflinkPref,
    /// Finished jobs kept in the history (for undo / redo).
    pub history_limit: usize,
    /// When a volume has no trash: `ask`, delete `permanent`ly or `skip`.
    pub trash_unsupported: TrashUnsupportedPolicy,
    /// Open the activity panel when a job runs for more than a second.
    pub auto_open_activity: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Terminal {
    /// Empty = auto-detect.
    pub shell: String,
    /// Arguments for the shell.
    pub args: Vec<String>,
    /// The terminal changes directory when the focused pane navigates.
    pub follow_navigation: bool,
    /// With one shared terminal, switching tabs also changes its directory.
    pub cd_on_tab_switch: bool,
    /// Inject prompt hooks so the shell reports its working directory (OSC 7).
    pub shell_integration: bool,
    /// Esc on an empty prompt line returns keyboard focus to the file list.
    pub escape_returns_focus: bool,
    /// Lines of scrollback per terminal.
    pub scrollback: usize,
    /// Terminal font size in points.
    pub font_size: f32,
    /// `"auto"` (derive from app theme) | `"<name>"`.
    pub theme: String,
    /// One terminal set per tab (`true`) or one shared by all tabs (`false`).
    pub per_tab: bool,
    /// Reverse sync: when the shell reports a new cwd, navigate the focused pane there.
    pub follow_shell: bool,
    /// What to do when a program asks to set the clipboard (OSC 52). Clipboard reads
    /// are always denied.
    pub osc52: Osc52Policy,
}

/// `terminal.osc52`: clipboard writes requested by terminal output.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Osc52Policy {
    /// Show a prompt (Allow once / Always for this session / Deny).
    #[default]
    Ask,
    Allow,
    Deny,
}

/// `[archive]`: extraction safety.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Archive {
    /// Keep setuid / setgid / sticky bits from archive entries (unix). Off: modes are
    /// masked to `0o777`.
    pub preserve_special_bits: bool,
    /// Decompression-bomb guard: total extracted bytes may reach `bomb_ratio` × the
    /// archive's size (0 = no ratio limit) ...
    pub bomb_ratio: u64,
    /// ... but never less than this many MiB.
    pub bomb_floor_mib: u64,
    /// Most entries one extraction may write (0 = unlimited).
    pub max_entries: u64,
    /// Largest single entry in MiB (0 = unlimited).
    pub max_entry_mib: u64,
    /// Deflate level (0 = store, 1-9) for files added while editing inside a zip.
    pub edit_level: u8,
}

/// A `[[search.saved]]` entry. `query` is kept as a raw TOML table and deserialized into
/// `soyuz_search::SearchQuery` by the search crate / app (core cannot depend on search).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SavedSearchEntry {
    /// Name shown in the sidebar and palette.
    pub name: String,
    /// The search query (the fields of the search panel).
    pub query: toml::Table,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Search {
    /// 0 = auto.
    pub threads: usize,
    /// Skip files ignored by `.gitignore` / `.ignore` when searching.
    pub respect_gitignore: bool,
    /// Content search skips files larger than this (MiB).
    pub content_max_mib: u64,
    /// Saved searches.
    pub saved: Vec<SavedSearchEntry>,
    /// Background index: `[search.index]`.
    pub index: SearchIndex,
}

/// `[search.index]`: the optional background file index (SQLite + FTS5) that makes
/// searches over the indexed roots instant and powers smart folders.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchIndex {
    /// Build and use the background index.
    pub enabled: bool,
    /// Folders to index; `~` is the home folder.
    pub roots: Vec<String>,
    /// Names (`node_modules`, `*.tmp`) or path endings (`AppData/Local/Temp`) to skip.
    pub exclude: Vec<String>,
    /// Also index the words of small text files (`content:` queries).
    pub content: bool,
    /// Largest file whose contents are indexed (KiB).
    pub content_max_kib: u64,
    /// Extensions whose contents are indexed.
    pub content_extensions: Vec<String>,
    /// On NTFS, catch up with changes made while Soyuz was closed by reading the change
    /// journal (needs administrator rights; otherwise the roots are re-crawled).
    pub usn_journal: bool,
    /// Pause after every 1000 crawled entries (ms); higher is gentler on the disk.
    pub throttle_ms: u64,
    /// Smart folders refresh at most this often while the index changes (seconds).
    pub smart_refresh_secs: u64,
    /// Searches use the index when it covers the folder (off = always walk the disk).
    pub use_for_search: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preview {
    /// Text previews read at most this much of a file (KiB).
    pub max_text_kib: u64,
    /// Largest image the preview decodes (megapixels).
    pub max_image_mp: u32,
    /// Syntax highlighting theme for code previews (`auto` follows the app theme).
    pub syntax_theme: String,
    /// Needs cargo feature `pdf`.
    pub pdf: bool,
    /// Memory budget of decoded thumbnails (MiB).
    pub thumb_cache_mib: u64,
    /// Memory budget of decoded previews (images, text, media) (MiB).
    pub preview_cache_mib: u64,
    /// Folder holding `pdfium.dll` / `libpdfium.so` / `libpdfium.dylib`; empty = next to
    /// the executable, then the config dir. Never downloaded.
    pub pdfium_dir: String,
    /// docx / xlsx / pptx / odt / ods / odp structure and text.
    pub office: bool,
    /// EPUB title, author, cover and table of contents.
    pub epub: bool,
    /// Font specimens (ttf / otf / ttc / woff).
    pub fonts: bool,
    /// glTF / GLB / STL / OBJ quick render.
    pub models: bool,
    /// Slowly spin 3D models until they are dragged (off with reduced motion too).
    pub model_turntable: bool,
    /// Decode audio for a waveform image.
    pub audio_waveform: bool,
    /// Play / pause / seek audio in the preview.
    pub audio_playback: bool,
    /// Poster frame for videos through an `ffmpeg` executable (when one is found).
    pub video_ffmpeg: bool,
    /// The `ffmpeg` program: a name looked up on PATH or a full path.
    pub ffmpeg_path: String,
    /// Side-by-side / unified diff when exactly two text files are selected.
    pub diff: bool,
    /// Office / EPUB / PDF files (and each part inside them) larger than this are not parsed (MiB).
    pub max_document_mib: u64,
    /// Spreadsheet preview: rows read from the first sheet.
    pub sheet_rows: u32,
    /// 3D model files larger than this are not loaded (MiB).
    pub max_model_mib: u64,
    /// Audio files larger than this get no waveform (MiB).
    pub max_audio_mib: u64,
    /// Each side of a diff is read up to this size (KiB).
    pub max_diff_kib: u64,
}

/// `[editor]`: the built-in text / hex / image editors ("Edit in Soyuz", F4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Editor {
    /// Where the editor panel docks.
    pub dock: EditorDock,
    /// Text editor font size (points).
    pub font_size: f32,
    /// Columns per tab stop.
    pub tab_width: u8,
    /// Soft-wrap long lines.
    pub wrap: bool,
    /// Draw spaces and tabs as faint dots / arrows.
    pub show_whitespace: bool,
    /// Text files larger than this open read-only in the hex editor.
    pub max_text_mib: u64,
    /// Before overwriting a file, put a copy of the previous version (`<name>.bak`) in
    /// the trash.
    pub backup_to_trash: bool,
    /// JPEG quality when an edited JPEG is saved (always a lossy re-encode).
    pub jpeg_quality: u8,
    /// Largest image the image editor decodes (megapixels).
    pub max_image_mp: u32,
}

/// `editor.dock`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EditorDock {
    #[default]
    Right,
    Bottom,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Git {
    /// Show the git status column and badges inside repositories.
    pub status_column: bool,
    /// Re-read git status at most this often (seconds).
    pub refresh_secs: u64,
}

/// `[updates]`: user-initiated update check against GitHub Releases. Nothing
/// is ever downloaded or installed without a click; there is no telemetry. Defaults: the
/// official repository, no weekly check, stable releases only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Updates {
    /// GitHub repository to check, `owner/name` or `https://github.com/owner/name`.
    /// Empty disables the update check.
    pub repo: String,
    /// Also check once a week at startup (off by default; only shows a notification).
    pub weekly_check: bool,
    /// Offer pre-releases (release candidates, betas) too.
    pub include_prereleases: bool,
}

/// The official repository, checked by "Check for updates" unless `[updates] repo` says
/// otherwise.
pub const DEFAULT_UPDATE_REPO: &str = "ulaas/soyuz";

impl Default for Updates {
    fn default() -> Self {
        Updates { repo: DEFAULT_UPDATE_REPO.into(), weekly_check: false, include_prereleases: false }
    }
}

/// Longest custom tag name, in characters (longer names are cut).
pub const TAG_NAME_MAX_CHARS: usize = 40;

/// `[tags]`: display names of the eight color tags. Membership is stored by color, so
/// renaming a tag never touches the tagged items.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TagsSection {
    /// Custom names in color order (red, orange, yellow, green, cyan, blue, purple, gray).
    /// An empty entry (or a missing one) shows the translated color name; names are
    /// trimmed and cut at 40 characters.
    pub names: Vec<String>,
}

impl TagsSection {
    /// The cleaned custom name of tag `i` (0-based), `None` when it uses the default.
    pub fn custom_name(&self, i: usize) -> Option<String> {
        self.names.get(i).map(|s| clean_tag_name(s)).filter(|s| !s.is_empty())
    }
}

/// A tag name as shown and stored: control characters become spaces, surrounding
/// whitespace is trimmed and the result is cut at [`TAG_NAME_MAX_CHARS`] characters.
pub fn clean_tag_name(s: &str) -> String {
    let s: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    s.trim().chars().take(TAG_NAME_MAX_CHARS).collect::<String>().trim_end().to_string()
}

/// `[performance]`: startup and rendering.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Performance {
    /// Graphics API the GPU renderer uses (`auto` = Direct3D 12 on Windows, the platform
    /// default elsewhere). The `WGPU_BACKEND` environment variable and the Windows 11
    /// backdrop (Direct3D 12 only) take precedence. Applies at the next start.
    pub gpu_backend: GpuBackend,
}

impl Default for Performance {
    fn default() -> Self {
        Performance { gpu_backend: GpuBackend::Auto }
    }
}

/// `[performance] gpu_backend`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GpuBackend {
    #[default]
    Auto,
    /// Direct3D 12 (Windows).
    Dx12,
    Vulkan,
    /// OpenGL / ANGLE through wgpu (compatibility).
    Gl,
}

/// `[sync]`: scheduled sync profiles, two-way sync state and previous versions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncSection {
    /// Run sync profiles on their schedules while the app is open (no OS service).
    pub scheduler: bool,
    /// Run reports (dry runs and real runs) kept per profile.
    pub reports_kept: u32,
    /// "On change" schedules wait this long after the last change before running.
    pub change_debounce_secs: u64,
    /// Scheduled runs only report what they would change (nothing is copied or deleted).
    pub scheduled_dry_run: bool,
    /// Previous versions: list Windows Volume Shadow Copies.
    pub previous_versions: bool,
    /// Previous versions: probe `HarddiskVolumeShadowCopy1..=N` when the kernel device
    /// directory can't be listed.
    pub shadow_probe_max: u32,
}

/// `[remote]`: remote locations. Saved connections live in
/// `remote_connections.json` next to the config; passwords in the OS credential store.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteSection {
    /// Remote folder listings are reused for this long (Refresh always re-lists).
    pub cache_ttl_secs: u64,
    /// Re-list visible remote folders this often to pick up changes (0 = never).
    pub poll_secs: u64,
    /// Give up connecting to a server after this long (seconds).
    pub connect_timeout_secs: u64,
    /// Give up on a server that stops answering mid-operation after this long.
    pub io_timeout_secs: u64,
    /// Remote files are downloaded for preview / open into a per-user temp cache of at
    /// most this size (oldest files are evicted).
    pub preview_cache_mb: u64,
    /// Larger remote files are not downloaded just for a preview.
    pub preview_max_mb: u64,
    /// Typing `\\server` lists its shares (Windows `NetShareEnum`).
    pub share_enumeration: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keys {
    /// Key binding preset (see docs/KEYBOARD.md).
    pub preset: KeyPreset,
    /// Treat `Cmd` in bindings as `Ctrl` (on Windows and Linux).
    pub cmd_is_ctrl: bool,
    /// Action string → chord strings; overrides the preset. A value with spaces is a
    /// key sequence (`"g g"`, `"Ctrl+K Ctrl+S"`).
    pub bindings: BTreeMap<String, Vec<String>>,
    /// Ranger-style marks in the file list: `m<key>` sets a mark on the current folder,
    /// `'<key>` jumps to it (always on with the `vim` preset).
    pub marks: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Bookmark {
    /// Label in menus and the palette.
    pub name: String,
    /// Folder the bookmark opens.
    pub path: PathBuf,
    /// Chord string, empty = none.
    pub hotkey: String,
    /// Icon name (e.g. `"code"`), empty = folder.
    pub icon: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandContext {
    File,
    Dir,
    /// Empty area of a folder.
    Background,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CustomCommand {
    /// Label in the context menu and palette.
    pub name: String,
    /// Program to run (no shell is involved unless you start one).
    pub command: String,
    /// Placeholders: `%p` focused path, `%n` selected names, `%s` selected paths, `%d`
    /// current dir, `%o` other pane's dir, `%%` a literal `%` (legacy: `%P` paths, `%D`
    /// dir, `%N` name, `%E` ext, `%F` first selected). An argument that is exactly `%s` /
    /// `%n` / `%P` expands to one argument per selected item.
    pub args: Vec<String>,
    /// Where the command appears: on a `file`, a `dir`, or the folder `background`. Empty = everywhere.
    pub context: Vec<CommandContext>,
    /// Chord string that runs it (empty = none).
    pub hotkey: String,
    /// Run in the integrated terminal panel instead of as a detached process.
    pub terminal: bool,
}
