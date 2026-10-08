//! The `config.toml` schema (PLAN §2.9). Every section is `#[serde(default)]`, so any
//! subset of keys is a valid file. Defaults live in `defaults.rs`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::columns::{Column, ViewMode};
use crate::size::SizeStyle;
use crate::time::DateFormat;

use super::keys::KeyPreset;

/// Current schema version written by [`save_pretty`](super::save_pretty).
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Schema version (`version` is accepted as an alias); see `config::migrate`.
    #[serde(alias = "version")]
    pub schema: u32,
    pub general: General,
    pub appearance: Appearance,
    pub theme: ThemeSection,
    pub layout: Layout,
    pub toolbar: Toolbar,
    pub context_menu: ContextMenu,
    pub sidebar: Sidebar,
    pub columns: Columns,
    pub behaviour: Behaviour,
    pub jobs: Jobs,
    pub terminal: Terminal,
    pub archive: Archive,
    pub search: Search,
    pub preview: Preview,
    pub git: Git,
    pub keys: Keys,
    pub ai: super::ai::Ai,
    pub bookmarks: Vec<Bookmark>,
    pub custom_commands: Vec<CustomCommand>,
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
    pub open_folders_in: OpenFoldersIn,
    pub single_click_open: bool,
    /// Confirm moving to trash.
    pub confirm_delete: bool,
    pub confirm_permanent_delete: bool,
    pub show_hidden: bool,
    /// Windows system files.
    pub show_system: bool,
    pub show_extensions: bool,
    pub natural_sort: bool,
    pub case_insensitive_sort: bool,
    pub folders_first: bool,
    pub size_style: SizeStyle,
    pub date_format: DateFormat,
    pub default_view: ViewMode,
    pub double_click_speed_ms: u32,
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
    /// List row height in points (PLAN §1.2).
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
    pub density: Density,
    pub ui_scale: f32,
    pub font: String,
    pub mono_font: String,
    pub font_size: f32,
    pub reduced_motion: bool,
    /// High-contrast palette (stronger text, borders and focus rings).
    pub high_contrast: bool,
    pub icon_style: IconStyle,
    pub show_thumbnails: bool,
    pub thumbnail_size: u32,
    pub sidebar_width: f32,
    pub custom_titlebar: bool,
    /// Windows 11 window backdrop behind translucent surfaces (no-op elsewhere; read at start).
    pub backdrop: Backdrop,
    /// Windows 11 Snap Layouts flyout when hovering the maximize button (custom title bar).
    pub snap_layouts: bool,
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

/// `[theme.colors]`: token name (as in PLAN §1.2, e.g. `"accent"`, `"bg.window"`) → hex.
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
    pub sidebar: bool,
    pub preview_pane: bool,
    pub dual_pane: bool,
    pub dual_pane_orientation: Orientation,
    pub status_bar: bool,
    pub tab_strip: TabStripMode,
    pub terminal_height: f32,
    pub activity_panel: ActivityPlacement,
}

/// Editable toolbar: action names (§2.10) plus `"|"` (separator), `"spacer"`,
/// `"breadcrumb"`, `"view_menu"`, `"sort_menu"`, `"activity"`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Toolbar {
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
    /// OneDrive / CloudStorage folders detected via env + known dirs.
    Cloud,
    Tags,
    Network,
    Trash,
    Tree,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sidebar {
    pub sections: Vec<SidebarSection>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Columns {
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
    pub rename_selects: RenameSelects,
    pub paste_conflict: PasteConflict,
    pub drag_default: DragDefault,
    pub remember_folder_views: bool,
    pub restore_session: bool,
    pub auto_refresh: bool,
    pub watch_debounce_ms: u64,
    pub type_ahead: TypeAhead,
    pub middle_click: MiddleClick,
    /// Hovering a folder during a drag opens it (Explorer / Finder "spring-loaded folders").
    pub spring_loaded_folders: bool,
    /// Hover time before a spring-loaded folder opens (ms).
    pub spring_load_delay_ms: u32,
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
    pub max_parallel: usize,
    pub per_device_serial: bool,
    pub reset_apply_all_between_jobs: bool,
    pub verify_after_copy: bool,
    pub copy_buffer_mib: u64,
    pub unbuffered_threshold_mib: u64,
    pub preserve: Vec<PreserveItem>,
    pub reflink: ReflinkPref,
    pub history_limit: usize,
    pub trash_unsupported: TrashUnsupportedPolicy,
    /// Open the activity panel when a job runs for more than a second.
    pub auto_open_activity: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Terminal {
    /// Empty = auto-detect.
    pub shell: String,
    pub args: Vec<String>,
    pub follow_navigation: bool,
    pub cd_on_tab_switch: bool,
    pub shell_integration: bool,
    pub escape_returns_focus: bool,
    pub scrollback: usize,
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
}

/// A `[[search.saved]]` entry. `query` is kept as a raw TOML table and deserialized into
/// `soyuz_search::SearchQuery` by the search crate / app (core cannot depend on search).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SavedSearchEntry {
    pub name: String,
    pub query: toml::Table,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Search {
    /// 0 = auto.
    pub threads: usize,
    pub respect_gitignore: bool,
    pub content_max_mib: u64,
    pub saved: Vec<SavedSearchEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preview {
    pub max_text_kib: u64,
    pub max_image_mp: u32,
    pub syntax_theme: String,
    /// Needs cargo feature `pdf`.
    pub pdf: bool,
    /// Memory budget of decoded thumbnails (MiB).
    pub thumb_cache_mib: u64,
    /// Memory budget of decoded previews (images, text, media) (MiB).
    pub preview_cache_mib: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Git {
    pub status_column: bool,
    pub refresh_secs: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keys {
    pub preset: KeyPreset,
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
    pub name: String,
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
    pub name: String,
    pub command: String,
    /// Placeholders: `%p` focused path, `%n` selected names, `%s` selected paths, `%d`
    /// current dir, `%o` other pane's dir, `%%` a literal `%` (legacy: `%P` paths, `%D`
    /// dir, `%N` name, `%E` ext, `%F` first selected). An argument that is exactly `%s` /
    /// `%n` / `%P` expands to one argument per selected item.
    pub args: Vec<String>,
    pub context: Vec<CommandContext>,
    pub hotkey: String,
    /// Run in the integrated terminal panel instead of as a detached process.
    pub terminal: bool,
}
