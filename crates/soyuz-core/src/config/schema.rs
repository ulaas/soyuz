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
    /// Font roles (`[fonts]`, see `config::fonts`).
    pub fonts: super::fonts::Fonts,
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
    pub editor: Editor,
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
    /// Legacy (schema 1): read and moved into `[fonts.ui]` on load, never written.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub font: String,
    /// Legacy (schema 1): moved into `[fonts.terminal]` / `code` / `hex` on load.
    #[serde(skip_serializing_if = "String::is_empty")]
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
    /// System fallback fonts for scripts Inter lacks (CJK, Hangul, emoji).
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

/// When system fallback fonts (CJK / Hangul / emoji, from the OS font folder) load.
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

/// What the sidebar shows: the places sections, the folder tree, or both.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SidebarMode {
    /// Favorites, devices, cloud, tags … (no folder tree).
    Places,
    /// Only the folder tree (drives + home).
    Tree,
    /// The places sections with the folder tree as the `tree` section.
    #[default]
    Both,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sidebar {
    pub sections: Vec<SidebarSection>,
    /// `places` | `tree` | `both`.
    pub mode: SidebarMode,
    /// Expand the tree to the active pane's folder and highlight it.
    pub tree_follow: bool,
    /// Hovering a drag over a collapsed tree folder expands it after this delay
    /// (milliseconds; 0 = never).
    pub tree_drag_expand_ms: u32,
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
    /// Deflate level (0 = store, 1-9) for files added while editing inside a zip.
    pub edit_level: u8,
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
