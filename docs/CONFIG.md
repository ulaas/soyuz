# Configuration reference

<!-- Generated from crates/soyuz-core/src/config (schema, doc comments, Config::default()) and the
     comments in docs/config.example.toml by crates/soyuz-app/tests/docs_sync.rs. Do not edit by hand:
     run `SOYUZ_BLESS_DOCS=1 cargo test -p soyuz-app --test docs_sync`. -->

Everything Soyuz does is configured in one TOML file, `config.toml`. Every key is optional:
a missing key takes the default shown here, unknown keys are reported as warnings, and the file
is hot-reloaded (edits apply within a second; a parse error keeps the previous settings and
shows the line and column). The Settings window (<kbd>Ctrl</kbd>+<kbd>,</kbd>) edits the same
file and keeps your comments. An annotated sample is [config.example.toml](config.example.toml).

| OS | Location |
|---|---|
| Windows | `%APPDATA%\soyuz\config\config.toml` |
| Linux | `~/.config/soyuz/config.toml` |
| macOS | `~/Library/Application Support/soyuz/config.toml` |
| Portable | `portable-data\config.toml` next to the executable when an empty file named `portable` sits beside it |

Secrets never go in this file: API keys, remote passwords and MCP tokens live in the OS
credential store, and config values only reference them (`credential:<name>`).

Types: *string*, *integer*, *float*, *boolean*, arrays (`["a", "b"]`) and tables. A type
written as a list of quoted values is an enumeration: use one of them.

## Sections

- [`Top level`](#top-level)
- [`[general]`](#general)
- [`[appearance]`](#appearance)
- [`[fonts]`](#fonts)
- [`[fonts.ui]`](#fontsui)
- [`[fonts.ui_semibold]`](#fontsui_semibold)
- [`[fonts.terminal]`](#fontsterminal)
- [`[fonts.code]`](#fontscode)
- [`[fonts.hex]`](#fontshex)
- [`[fonts.numbers]`](#fontsnumbers)
- [`[theme]`](#theme)
- [`[layout]`](#layout)
- [`[workspace]`](#workspace)
- [`[toolbar]`](#toolbar)
- [`[context_menu]`](#context_menu)
- [`[sidebar]`](#sidebar)
- [`[columns]`](#columns)
- [`[behaviour]`](#behaviour)
- [`[jobs]`](#jobs)
- [`[terminal]`](#terminal)
- [`[archive]`](#archive)
- [`[search]`](#search)
- [`[[search.saved]]`](#searchsaved)
- [`[search.index]`](#searchindex)
- [`[preview]`](#preview)
- [`[editor]`](#editor)
- [`[git]`](#git)
- [`[sync]`](#sync)
- [`[updates]`](#updates)
- [`[performance]`](#performance)
- [`[remote]`](#remote)
- [`[keys]`](#keys)
- [`[ai]`](#ai)
- [`[[ai.profiles]]`](#aiprofiles)
- [`[ai.tasks]`](#aitasks)
- [`[ai.tasks.fast]`](#aitasksfast)
- [`[ai.tasks.strong]`](#aitasksstrong)
- [`[ai.scope]`](#aiscope)
- [`[[ai.prices]]`](#aiprices)
- [`[ai.mcp]`](#aimcp)
- [`[[ai.mcp_clients]]`](#aimcp_clients)
- [`[[ai.mcp_servers]]`](#aimcp_servers)
- [`[ai.agent]`](#aiagent)
- [`[[ai.folders]]`](#aifolders)
- [`[[bookmarks]]`](#bookmarks)
- [`[[custom_commands]]`](#custom_commands)
- [`[scripting]`](#scripting)
- [`[automation]`](#automation)
- [`[[automation.rules]]`](#automationrules)
- [`[cli]`](#cli)

## `Top level`

| Key | Type | Default | Description |
|---|---|---|---|
| `schema` | integer | `1` | Schema version (`version` is accepted as an alias); see `config::migrate`. |

## `[general]`

| Key | Type | Default | Description |
|---|---|---|---|
| `start_location` | string | `"home"` | `"home"` \| `"last"` \| `"<path>"`. |
| `open_folders_in` | `"same_tab"` \| `"new_tab"` | `"same_tab"` | Open folders in the `same_tab` or a `new_tab`. |
| `single_click_open` | boolean | `false` | Open files and folders with a single click instead of a double click. |
| `confirm_delete` | boolean | `true` | Confirm moving to trash. |
| `confirm_permanent_delete` | boolean | `true` | Confirm before deleting permanently (Shift+Del). |
| `show_hidden` | boolean | `false` | Show hidden files and folders (also toggled with Ctrl+H). |
| `show_system` | boolean | `false` | Windows system files. |
| `show_extensions` | boolean | `true` | Show file name extensions. |
| `natural_sort` | boolean | `true` | Natural sort: `file2` before `file10`. |
| `case_insensitive_sort` | boolean | `true` | Sort names without regard to letter case. |
| `folders_first` | boolean | `true` | List folders before files. |
| `size_style` | `"binary"` \| `"decimal"` \| `"bytes"` | `"binary"` | `binary`: 1024-based: KiB, MiB, ...; `decimal`: 1000-based: kB, MB, ...; `bytes`: Exact byte count with thousands separators. |
| `date_format` | string | `"relative"` | relative \| short \| long \| iso \| "%Y-%m-%d %H:%M" |
| `default_view` | `"details"` \| `"grid"` \| `"list"` \| `"miller"` \| `"gallery"` | `"details"` | View for folders without a remembered view. |
| `double_click_speed_ms` | integer | `400` | Longest gap between the two clicks of a double click (ms). |
| `language` | string | `"auto"` | UI language: `"auto"` (the OS display language), `"en-US"` or `"tr-TR"`. Switches live. |
| `strict_windows_names` | boolean | `false` | Apply Windows file-name rules on every OS (see `validate_name_with`). |

## `[appearance]`

| Key | Type | Default | Description |
|---|---|---|---|
| `theme` | string | `"system"` | `"system"` \| `"dark"` \| `"light"` \| `"<name>"` (user theme in `themes/`). |
| `density` | `"compact"` \| `"cozy"` \| `"comfortable"` \| `"touch"` | `"cozy"` | Row and control height: `compact`, `cozy`, `comfortable` or `touch`. (`touch`: Large touch targets (rows, buttons and menu items grow to finger size)) |
| `ui_scale` | float | `1.0` | Interface zoom factor (Ctrl+= / Ctrl+- / Ctrl+0 change it). |
| `font` | string | — | Legacy (schema 1): read and moved into `[fonts.ui]` on load, never written. |
| `mono_font` | string | — | Legacy (schema 1): moved into `[fonts.terminal]` / `code` / `hex` on load. |
| `font_size` | float | `13.0` | Base UI text size in points. |
| `reduced_motion` | boolean | `false` | Turn off animations (transitions and springs jump to their end state). |
| `high_contrast` | boolean | `false` | High-contrast palette (stronger text, borders and focus rings). |
| `icon_style` | `"colored"` \| `"mono"` | `"colored"` | `colored` file-type icons or `mono` (single color; folder colors stay). |
| `show_thumbnails` | boolean | `true` | Picture and video thumbnails in the grid and gallery views. |
| `thumbnail_size` | integer | `96` | Default thumbnail edge in the grid view (points). |
| `sidebar_width` | float | `220.0` | Sidebar width in points (dragging its edge updates it). |
| `custom_titlebar` | boolean | `true` | Soyuz draws its own title bar with the tabs in it; off = the system title bar. |
| `backdrop` | `"none"` \| `"mica"` \| `"acrylic"` | `"none"` | Windows 11 window backdrop behind translucent surfaces (no-op elsewhere; read at start). (`none`: Opaque theme colors; `mica`: Mica: the desktop wallpaper, heavily blurred and tinted; `acrylic`: Acrylic: a blur of whatever is behind the window) |
| `snap_layouts` | boolean | `true` | Windows 11 Snap Layouts flyout when hovering the maximize button (custom title bar). |
| `native_dialogs` | boolean | `true` | Dialogs, the settings window and menus that do not fit the window open as their own OS windows (movable outside the main window and onto other monitors); off = drawn inside the window (menus are then clipped to it). |
| `fallback_fonts` | `"auto"` \| `"startup"` \| `"off"` | `"auto"` | System fallback fonts for scripts Inter lacks (CJK, Hangul, emoji, Arabic / Hebrew). (`auto`: When a file name needs them (loaded on a background thread); `startup`: All of them in the background right after startup (also covers terminal output); `off`: Never (missing glyphs render as boxes)) |
| `color_theme` | string | `"orbit"` | Color theme family: a built-in id (`orbit`, `nord`, `solarized`, `rose-pine`, `high-contrast`) or the file stem of a user theme in `themes/*.toml`. Each theme has a dark and a light variant; `theme` picks which one shows. |
| `accent` | string | `"theme"` | `"theme"` (the theme's own accent) \| `"system"` (the OS accent color, followed live) \| `"#RRGGBB"`. |
| `extension_badges` | boolean | `true` | Extension label badges on file icons in the grid / gallery views. |
| `folder_thumbnails` | boolean | `true` | 2×2 mosaic of the first images inside folders (grid / gallery views). |
| `video_thumbnails` | boolean | `true` | First-frame video thumbnails through an `ffmpeg` binary on `PATH` (when found). |
| `pdf_thumbnails` | boolean | `true` | First-page PDF thumbnails through pdfium (when `pdfium.dll` is found, see `[preview] pdfium_dir`); without it PDFs keep their icon. |
| `os_thumbnail_cache` | boolean | `true` | Reuse the thumbnails Explorer already made (Windows `thumbcache_*.db`, read-only) before decoding a file ourselves. |
| `startup_placeholder` | boolean | `true` | Windows: show a native placeholder window (theme background + logo) right away while the GPU renderer starts, replaced by the real window on its first frame. Applies at the next start. |
| `icon_pack` | string | `""` | File icon pack: `""` = the built-in Phosphor glyphs, `"flat"` = the built-in generated pack, else the folder name of a user pack in `icons/<name>/pack.toml`. |
| `thumb_disk_cache_mib` | integer | `256` | On-disk thumbnail cache in `thumbs/` (MiB; 0 = off). Thumbnails survive restarts. |

## `[fonts]`

| Key | Type | Default | Description |
|---|---|---|---|
| `hinting` | boolean | `true` | TrueType hinting: snap stems to the pixel grid (crisper text at small sizes). |
| `subpixel_positioning` | boolean | `true` | Sub-pixel glyph positioning: more even letter spacing, slightly softer stems. |
| `hinting_strength` | `"crisp"` \| `"normal"` \| `"light"` | `"crisp"` | How hard hinting snaps the outlines to the pixel grid (with `hinting = true`). (`crisp`: Snap stems horizontally and vertically: the sharpest text at 100 % scaling; `normal`: egui's default: snap vertically, keep the unhinted letter widths; `light`: Only snap heights (FreeType's "light" mode): truest shapes, softer stems) |
| `text_gamma_dark` | float | `0.45` | Text weight on dark themes: the gamma that maps glyph coverage to opacity (lower = heavier, 1.0 = linear; 0 = egui's built-in `2c - c²` curve, close to 0.5). |
| `text_gamma_light` | float | `0.8` | Text weight on light themes (same scale; 1.0 = linear coverage, egui's default). |

## `[fonts.ui]`

One role's font.

| Key | Type | Default | Description |
|---|---|---|---|
| `family` | string | `"Inter"` | Embedded or installed family name, or a font file path. Empty: `ui_semibold` uses the semibold face of the `ui` family, `numbers` uses the `ui` font, other roles their default. |
| `embedded` | boolean | `false` | Only embedded fonts: the one `family` names when it is embedded, else the role's embedded default (Inter / Inter SemiBold / Cascadia Mono). Never reads system fonts. |
| `size_offset` | float | `0.0` | Points added to the role's text size (−4 … +8). |

## `[fonts.ui_semibold]`

One role's font.

| Key | Type | Default | Description |
|---|---|---|---|
| `family` | string | `""` | Embedded or installed family name, or a font file path. Empty: `ui_semibold` uses the semibold face of the `ui` family, `numbers` uses the `ui` font, other roles their default. |
| `embedded` | boolean | `false` | Only embedded fonts: the one `family` names when it is embedded, else the role's embedded default (Inter / Inter SemiBold / Cascadia Mono). Never reads system fonts. |
| `size_offset` | float | `0.0` | Points added to the role's text size (−4 … +8). |

## `[fonts.terminal]`

One role's font.

| Key | Type | Default | Description |
|---|---|---|---|
| `family` | string | `"Cascadia Mono"` | Embedded or installed family name, or a font file path. Empty: `ui_semibold` uses the semibold face of the `ui` family, `numbers` uses the `ui` font, other roles their default. |
| `embedded` | boolean | `false` | Only embedded fonts: the one `family` names when it is embedded, else the role's embedded default (Inter / Inter SemiBold / Cascadia Mono). Never reads system fonts. |
| `size_offset` | float | `0.0` | Points added to the role's text size (−4 … +8). |

## `[fonts.code]`

One role's font.

| Key | Type | Default | Description |
|---|---|---|---|
| `family` | string | `"Cascadia Mono"` | Embedded or installed family name, or a font file path. Empty: `ui_semibold` uses the semibold face of the `ui` family, `numbers` uses the `ui` font, other roles their default. |
| `embedded` | boolean | `false` | Only embedded fonts: the one `family` names when it is embedded, else the role's embedded default (Inter / Inter SemiBold / Cascadia Mono). Never reads system fonts. |
| `size_offset` | float | `0.0` | Points added to the role's text size (−4 … +8). |

## `[fonts.hex]`

One role's font.

| Key | Type | Default | Description |
|---|---|---|---|
| `family` | string | `"Cascadia Mono"` | Embedded or installed family name, or a font file path. Empty: `ui_semibold` uses the semibold face of the `ui` family, `numbers` uses the `ui` font, other roles their default. |
| `embedded` | boolean | `false` | Only embedded fonts: the one `family` names when it is embedded, else the role's embedded default (Inter / Inter SemiBold / Cascadia Mono). Never reads system fonts. |
| `size_offset` | float | `0.0` | Points added to the role's text size (−4 … +8). |

## `[fonts.numbers]`

One role's font.

| Key | Type | Default | Description |
|---|---|---|---|
| `family` | string | `""` | Embedded or installed family name, or a font file path. Empty: `ui_semibold` uses the semibold face of the `ui` family, `numbers` uses the `ui` font, other roles their default. |
| `embedded` | boolean | `false` | Only embedded fonts: the one `family` names when it is embedded, else the role's embedded default (Inter / Inter SemiBold / Cascadia Mono). Never reads system fonts. |
| `size_offset` | float | `0.0` | Points added to the role's text size (−4 … +8). |

## `[theme]`

`[theme.colors]`: token name (as in PLAN §1.2, e.g. `"accent"`, `"bg.window"`) → hex.

| Key | Type | Default | Description |
|---|---|---|---|
| `colors` | table: name → string | `{}` | optional overrides, hex strings, same token names as PLAN §1.2 |

## `[layout]`

| Key | Type | Default | Description |
|---|---|---|---|
| `sidebar` | boolean | `true` | Show the sidebar. |
| `preview_pane` | boolean | `false` | Show the preview pane. |
| `dual_pane` | boolean | `false` | Start new tabs with two panes. |
| `dual_pane_orientation` | `"vertical"` \| `"horizontal"` | `"vertical"` | Side by side (`vertical` split) or stacked (`horizontal`). |
| `status_bar` | boolean | `true` | Show the status bar. |
| `tab_strip` | `"always"` \| `"multiple"` | `"always"` | Show the tab strip `always` or only with `multiple` tabs. |
| `terminal_height` | float | `260.0` | Height of the terminal / bottom panel in points (120-720). |
| `activity_panel` | `"right"` \| `"bottom"` \| `"floating"` | `"right"` | Where the activity (jobs) panel docks. |

## `[workspace]`

`[workspace]`: windows, tab groups, saved workspaces, synchronized browsing, the per-tab history graph and the timeline (roadmap F7).

| Key | Type | Default | Description |
|---|---|---|---|
| `new_window` | `"in_process"` \| `"process"` | `"in_process"` | What "New window" (Ctrl+N) opens. (`in_process`: Another window of this Soyuz (shares jobs, services and settings); `process`: A separate Soyuz process (`soyuz --new-window`)) |
| `tear_off` | boolean | `true` | Drag a tab out of the tab strip to open it in its own window. |
| `tear_off_distance` | float | `48.0` | How far (points) a tab has to be dragged away from the strip to tear off. |
| `close_window_tabs` | `"ask"` \| `"merge"` \| `"close"` | `"ask"` | Closing a window that still has tabs. (`ask`: Ask whether to move the tabs to another window or close them; `merge`: Move them to another window; `close`: Close them) |
| `restore_windows` | boolean | `true` | Reopen the other windows (with their positions) when the session is restored. |
| `sync_relative` | boolean | `true` | Synchronized browsing follows any relative move (sibling folders, several levels), not only "into a child" / "up one level". |
| `history_graph_limit` | integer | `200` | Places remembered per tab in the "where was I" history graph. |
| `timeline_days` | integer | `7` | The timeline covers this many days (today, yesterday, this week, …). |
| `timeline_scan_folders` | integer | `40` | Recently visited folders scanned (one level) for changed files. |
| `timeline_max_items` | integer | `1000` | Most files the timeline lists. |
| `timeline_use_index` | boolean | `true` | Ask the search index (when one is available) for changed files too. |

## `[toolbar]`

Editable toolbar: action names (§2.10) plus `"|"` (separator), `"spacer"`, `"breadcrumb"`, `"view_menu"`, `"sort_menu"`, `"activity"`.

| Key | Type | Default | Description |
|---|---|---|---|
| `items` | array of strings | `["back", "forward", "up", "\|", "breadcrumb", "spacer", "filter", "view_menu", "s…` | Toolbar items, left to right. |

## `[context_menu]`

| Key | Type | Default | Description |
|---|---|---|---|
| `hide` | array of strings | `[]` | Action names to hide. |
| `order` | array of strings | `[]` | Optional explicit order; unknown names ignored. |

## `[sidebar]`

| Key | Type | Default | Description |
|---|---|---|---|
| `sections` | array of `"favorites"` \| `"devices"` \| `"cloud"` \| `"tags"` \| `"network"` \| `"trash"` \| `"tree"` \| `"smart"` | `["favorites", "smart", "devices", "cloud", "tags", "network", "trash", "tree"]` | Sidebar sections in display order; leave one out to hide it. |
| `mode` | `"places"` \| `"tree"` \| `"both"` | `"both"` | `places` \| `tree` \| `both`. (`places`: Favorites, devices, cloud, tags … (no folder tree); `tree`: Only the folder tree (drives + home); `both`: The places sections with the folder tree as the `tree` section) |
| `tree_follow` | boolean | `true` | Expand the tree to the active pane's folder and highlight it. |
| `tree_drag_expand_ms` | integer | `700` | Hovering a drag over a collapsed tree folder expands it after this delay (milliseconds; 0 = never). |

## `[columns]`

| Key | Type | Default | Description |
|---|---|---|---|
| `details` | array of `"name"` \| `"size"` \| `"kind"` \| `"modified"` \| `"created"` \| `"accessed"` \| `"attributes"` \| `"mode"` \| `"owner"` \| `"extension"` \| `"path"` \| `"tags"` \| `"git_status"` \| `"dimensions"` \| `"duration"` \| `"link_target"` \| `"checksum"` \| `"item_count"` \| `"script1"` \| `"script2"` \| `"script3"` \| `"script4"` | `["name", "size", "kind", "modified"]` | Columns of the details view, in order. |
| `widths` | table: name → float | 4 entries | Column name (`snake_case`) → width in points. |

## `[behaviour]`

| Key | Type | Default | Description |
|---|---|---|---|
| `rename_selects` | `"stem"` \| `"all"` | `"stem"` | Inline rename preselects the name `stem` (without extension) or `all`. |
| `paste_conflict` | `"ask"` \| `"overwrite"` \| `"skip"` \| `"keep_both"` | `"ask"` | Default answer when a pasted name already exists (`ask` shows the conflict dialog). |
| `drag_default` | `"auto"` \| `"copy"` \| `"move"` \| `"ask"` | `"auto"` | Drag and drop default operation. (`auto`: Same device = move, otherwise copy) |
| `remember_folder_views` | boolean | `true` | Remember view, sort and columns per folder (`views.toml`). |
| `restore_session` | boolean | `true` | Reopen the tabs, panes and windows of the last session at startup. |
| `auto_refresh` | boolean | `true` | Watch open folders and refresh them when files change. |
| `watch_debounce_ms` | integer | `250` | Collect file-system change events for this long before refreshing (ms). |
| `type_ahead` | `"jump"` \| `"filter"` | `"jump"` | Typing in the file list `jump`s to the first match or `filter`s the list. |
| `middle_click` | `"new_tab"` \| `"other_pane"` \| `"nothing"` | `"new_tab"` | Middle-clicking a folder opens it in a `new_tab`, the `other_pane`, or does `nothing`. |
| `spring_loaded_folders` | boolean | `true` | Hovering a folder during a drag opens it (Explorer / Finder "spring-loaded folders"). |
| `spring_load_delay_ms` | integer | `700` | Hover time before a spring-loaded folder opens (ms). |

## `[jobs]`

| Key | Type | Default | Description |
|---|---|---|---|
| `max_parallel` | integer | `4` | Jobs that may run at the same time. |
| `per_device_serial` | boolean | `true` | Queue jobs that touch the same drive one after another (faster on spinning disks). |
| `reset_apply_all_between_jobs` | boolean | `true` | "Apply to all" in a conflict dialog lasts for one job only; off = it carries over to the following jobs. |
| `verify_after_copy` | boolean | `false` | Hash-compare every copied file with its source (BLAKE3) after the copy. |
| `copy_buffer_mib` | integer | `8` | Copy buffer size (MiB). |
| `unbuffered_threshold_mib` | integer | `256` | Files larger than this are copied with unbuffered I/O, bypassing the OS cache (MiB). |
| `preserve` | array of `"times"` \| `"attrs"` \| `"xattr"` \| `"acl"` \| `"owner"` \| `"sparse"` | `["times", "attrs", "xattr"]` | Metadata copied along with file data. |
| `reflink` | `"auto"` \| `"never"` \| `"always"` | `"auto"` | Copy-on-write clones (ReFS block cloning, btrfs / APFS reflinks): `auto`, `never` or `always`. |
| `history_limit` | integer | `500` | Finished jobs kept in the history (for undo / redo). |
| `trash_unsupported` | `"ask"` \| `"permanent"` \| `"skip"` | `"ask"` | When a volume has no trash: `ask`, delete `permanent`ly or `skip`. |
| `auto_open_activity` | boolean | `true` | Open the activity panel when a job runs for more than a second. |

## `[terminal]`

| Key | Type | Default | Description |
|---|---|---|---|
| `shell` | string | `""` | Empty = auto-detect. |
| `args` | array of strings | `[]` | Arguments for the shell. |
| `follow_navigation` | boolean | `true` | The terminal changes directory when the focused pane navigates. |
| `cd_on_tab_switch` | boolean | `true` | With one shared terminal, switching tabs also changes its directory. |
| `shell_integration` | boolean | `true` | Inject prompt hooks so the shell reports its working directory (OSC 7). |
| `escape_returns_focus` | boolean | `true` | Esc on an empty prompt line returns keyboard focus to the file list. |
| `scrollback` | integer | `10000` | Lines of scrollback per terminal. |
| `font_size` | float | `13.0` | Terminal font size in points. |
| `theme` | string | `"auto"` | `"auto"` (derive from app theme) \| `"<name>"`. |
| `per_tab` | boolean | `false` | One terminal set per tab (`true`) or one shared by all tabs (`false`). |
| `follow_shell` | boolean | `false` | Reverse sync: when the shell reports a new cwd, navigate the focused pane there. |
| `osc52` | `"ask"` \| `"allow"` \| `"deny"` | `"ask"` | What to do when a program asks to set the clipboard (OSC 52). Clipboard reads are always denied. (`ask`: Show a prompt (Allow once / Always for this session / Deny)) |

## `[archive]`

`[archive]`: extraction safety.

| Key | Type | Default | Description |
|---|---|---|---|
| `preserve_special_bits` | boolean | `false` | Keep setuid / setgid / sticky bits from archive entries (unix). Off: modes are masked to `0o777`. |
| `bomb_ratio` | integer | `100` | Decompression-bomb guard: total extracted bytes may reach `bomb_ratio` × the archive's size (0 = no ratio limit) ... |
| `bomb_floor_mib` | integer | `1024` | ... but never less than this many MiB. |
| `max_entries` | integer | `1000000` | Most entries one extraction may write (0 = unlimited). |
| `max_entry_mib` | integer | `32768` | Largest single entry in MiB (0 = unlimited). |
| `edit_level` | integer | `6` | Deflate level (0 = store, 1-9) for files added while editing inside a zip. |

## `[search]`

| Key | Type | Default | Description |
|---|---|---|---|
| `threads` | integer | `0` | 0 = auto. |
| `respect_gitignore` | boolean | `false` | Skip files ignored by `.gitignore` / `.ignore` when searching. |
| `content_max_mib` | integer | `8` | Content search skips files larger than this (MiB). |

## `[[search.saved]]`

A `[[search.saved]]` entry. `query` is kept as a raw TOML table and deserialized into `soyuz_search::SearchQuery` by the search crate / app (core cannot depend on search).

A list of tables; empty by default. Defaults below are for a new entry's omitted keys.

| Key | Type | Default | Description |
|---|---|---|---|
| `name` | string | — | Name shown in the sidebar and palette. |
| `query` | table | — | The search query (the fields of the search panel). |

## `[search.index]`

`[search.index]`: the optional background file index (SQLite + FTS5) that makes searches over the indexed roots instant and powers smart folders.

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | boolean | `false` | Build and use the background index. |
| `roots` | array of strings | `["~"]` | Folders to index; `~` is the home folder. |
| `exclude` | array of strings | `["node_modules", ".git", ".hg", ".svn", "target", "__pycache__", ".cache", "$REC…` | Names (`node_modules`, `*.tmp`) or path endings (`AppData/Local/Temp`) to skip. |
| `content` | boolean | `false` | Also index the words of small text files (`content:` queries). |
| `content_max_kib` | integer | `1024` | Largest file whose contents are indexed (KiB). |
| `content_extensions` | array of strings | `["txt", "md", "markdown", "rst", "org", "log", "csv", "json", "toml", "yaml", "y…` | Extensions whose contents are indexed. |
| `usn_journal` | boolean | `true` | On NTFS, catch up with changes made while Soyuz was closed by reading the change journal (needs administrator rights; otherwise the roots are re-crawled). |
| `throttle_ms` | integer | `2` | Pause after every 1000 crawled entries (ms); higher is gentler on the disk. |
| `smart_refresh_secs` | integer | `2` | Smart folders refresh at most this often while the index changes (seconds). |
| `use_for_search` | boolean | `true` | Searches use the index when it covers the folder (off = always walk the disk). |

## `[preview]`

| Key | Type | Default | Description |
|---|---|---|---|
| `max_text_kib` | integer | `512` | Text previews read at most this much of a file (KiB). |
| `max_image_mp` | integer | `40` | Largest image the preview decodes (megapixels). |
| `syntax_theme` | string | `"auto"` | Syntax highlighting theme for code previews (`auto` follows the app theme). |
| `pdf` | boolean | `true` | Needs cargo feature `pdf`. |
| `thumb_cache_mib` | integer | `128` | Memory budget of decoded thumbnails (MiB). |
| `preview_cache_mib` | integer | `96` | Memory budget of decoded previews (images, text, media) (MiB). |
| `pdfium_dir` | string | `""` | Folder holding `pdfium.dll` / `libpdfium.so` / `libpdfium.dylib`; empty = next to the executable, then the config dir. Never downloaded. |
| `office` | boolean | `true` | docx / xlsx / pptx / odt / ods / odp structure and text. |
| `epub` | boolean | `true` | EPUB title, author, cover and table of contents. |
| `fonts` | boolean | `true` | Font specimens (ttf / otf / ttc / woff). |
| `models` | boolean | `true` | glTF / GLB / STL / OBJ quick render. |
| `model_turntable` | boolean | `true` | Slowly spin 3D models until they are dragged (off with reduced motion too). |
| `audio_waveform` | boolean | `true` | Decode audio for a waveform image. |
| `audio_playback` | boolean | `true` | Play / pause / seek audio in the preview. |
| `video_ffmpeg` | boolean | `true` | Poster frame for videos through an `ffmpeg` executable (when one is found). |
| `ffmpeg_path` | string | `"ffmpeg"` | The `ffmpeg` program: a name looked up on PATH or a full path. |
| `diff` | boolean | `true` | Side-by-side / unified diff when exactly two text files are selected. |
| `max_document_mib` | integer | `64` | Office / EPUB / PDF files (and each part inside them) larger than this are not parsed (MiB). |
| `sheet_rows` | integer | `200` | Spreadsheet preview: rows read from the first sheet. |
| `max_model_mib` | integer | `128` | 3D model files larger than this are not loaded (MiB). |
| `max_audio_mib` | integer | `512` | Audio files larger than this get no waveform (MiB). |
| `max_diff_kib` | integer | `1024` | Each side of a diff is read up to this size (KiB). |

## `[editor]`

`[editor]`: the built-in text / hex / image editors ("Edit in Soyuz", F4).

| Key | Type | Default | Description |
|---|---|---|---|
| `dock` | `"right"` \| `"bottom"` | `"right"` | Where the editor panel docks. |
| `font_size` | float | `13.0` | Text editor font size (points). |
| `tab_width` | integer | `4` | Columns per tab stop. |
| `wrap` | boolean | `false` | Soft-wrap long lines. |
| `show_whitespace` | boolean | `false` | Draw spaces and tabs as faint dots / arrows. |
| `max_text_mib` | integer | `20` | Text files larger than this open read-only in the hex editor. |
| `backup_to_trash` | boolean | `true` | Before overwriting a file, put a copy of the previous version (`<name>.bak`) in the trash. |
| `jpeg_quality` | integer | `92` | JPEG quality when an edited JPEG is saved (always a lossy re-encode). |
| `max_image_mp` | integer | `200` | Largest image the image editor decodes (megapixels). |

## `[git]`

| Key | Type | Default | Description |
|---|---|---|---|
| `status_column` | boolean | `true` | Show the git status column and badges inside repositories. |
| `refresh_secs` | integer | `5` | Re-read git status at most this often (seconds). |

## `[sync]`

`[sync]`: scheduled sync profiles, two-way sync state and previous versions (F8).

| Key | Type | Default | Description |
|---|---|---|---|
| `scheduler` | boolean | `true` | Run sync profiles on their schedules while the app is open (no OS service). |
| `reports_kept` | integer | `20` | Run reports (dry runs and real runs) kept per profile. |
| `change_debounce_secs` | integer | `10` | "On change" schedules wait this long after the last change before running. |
| `scheduled_dry_run` | boolean | `false` | Scheduled runs only report what they would change (nothing is copied or deleted). |
| `previous_versions` | boolean | `true` | Previous versions: list Windows Volume Shadow Copies. |
| `shadow_probe_max` | integer | `64` | Previous versions: probe `HarddiskVolumeShadowCopy1..=N` when the kernel device directory can't be listed. |

## `[updates]`

`[updates]`: user-initiated update check against GitHub Releases (roadmap D1). Nothing is ever downloaded or installed without a click; there is no telemetry. Defaults: no repo (check disabled), no weekly check, stable releases only.

| Key | Type | Default | Description |
|---|---|---|---|
| `repo` | string | `""` | GitHub repository to check, `owner/name` or `https://github.com/owner/name`. Empty disables the update check. |
| `weekly_check` | boolean | `false` | Also check once a week at startup (off by default; only shows a notification). |
| `include_prereleases` | boolean | `false` | Offer pre-releases (release candidates, betas) too. |

## `[performance]`

`[performance]`: startup and rendering (R5).

| Key | Type | Default | Description |
|---|---|---|---|
| `cpu_first_frame` | boolean | `true` | Windows: draw the real interface on the CPU into the startup placeholder while the GPU renderer starts (about 0.05 s instead of 0.37 s to the first real frame; mouse input works, the keyboard from the GPU window on), then switch to the GPU window. Needs `[appearance] startup_placeholder` and the custom title bar. Applies at the next start. |
| `gpu_backend` | `"auto"` \| `"dx12"` \| `"vulkan"` \| `"gl"` | `"auto"` | Graphics API the GPU renderer uses (`auto` = Direct3D 12 on Windows, the platform default elsewhere). The `WGPU_BACKEND` environment variable and the Windows 11 backdrop (Direct3D 12 only) take precedence. Applies at the next start. (`dx12`: Direct3D 12 (Windows); `gl`: OpenGL / ANGLE through wgpu (compatibility)) |

## `[remote]`

`[remote]`: remote locations (F2). Saved connections live in `remote_connections.json` next to the config; passwords in the OS credential store.

| Key | Type | Default | Description |
|---|---|---|---|
| `cache_ttl_secs` | integer | `30` | Remote folder listings are reused for this long (Refresh always re-lists). |
| `poll_secs` | integer | `0` | Re-list visible remote folders this often to pick up changes (0 = never). |
| `connect_timeout_secs` | integer | `20` | Give up connecting to a server after this long (seconds). |
| `io_timeout_secs` | integer | `60` | Give up on a server that stops answering mid-operation after this long. |
| `preview_cache_mb` | integer | `512` | Remote files are downloaded for preview / open into a per-user temp cache of at most this size (oldest files are evicted). |
| `preview_max_mb` | integer | `64` | Larger remote files are not downloaded just for a preview. |
| `share_enumeration` | boolean | `true` | Typing `\\server` lists its shares (Windows `NetShareEnum`). |

## `[keys]`

| Key | Type | Default | Description |
|---|---|---|---|
| `preset` | `"explorer"` \| `"commander"` \| `"finder"` \| `"vim"` | `"explorer"` | Key binding preset (see docs/KEYBOARD.md). (`finder`: macOS Finder-style (Enter renames, Cmd+Up/Down, Cmd+1..4 views); `vim`: Vim / ranger-style letter keys and two-key sequences (`g g`, `y y`, `d d`)) |
| `cmd_is_ctrl` | boolean | `true` | Treat `Cmd` in bindings as `Ctrl` (on Windows and Linux). |
| `bindings` | table: name → array of strings | `{}` | Action string → chord strings; overrides the preset. A value with spaces is a key sequence (`"g g"`, `"Ctrl+K Ctrl+S"`). |
| `marks` | boolean | `false` | Ranger-style marks in the file list: `m<key>` sets a mark on the current folder, `'<key>` jumps to it (always on with the `vim` preset). |

## `[ai]`

AI providers (roadmap Phase A); keys live in the OS credential store

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | boolean | `true` | Turn on the AI assistant and AI actions (nothing is sent anywhere while off). |
| `default_profile` | string | `""` | Profile name used when a task names none. |
| `privacy` | `"none"` \| `"metadata_only"` \| `"contents"` | `"metadata_only"` | none \| metadata_only \| contents (file contents never leave the machine by default) (`none`: No file information at all; `metadata_only`: Names, sizes, dates and types only; file contents never leave the machine; `contents`: File contents may be sent (still subject to scopes and size caps)) |
| `request_timeout_secs` | integer | `60` | Seconds to wait for the response headers. |
| `idle_timeout_secs` | integer | `120` | Seconds without any streamed byte before the request is abandoned. |
| `max_retries` | integer | `3` | Retries for 429 / overloaded / 5xx before giving up (honours `retry-after`). |
| `budget_warn_percent` | integer | `80` | Warn when a profile's monthly spend reaches this percentage of its cap. |

## `[[ai.profiles]]`

A list of tables; empty by default. Defaults below are for a new entry's omitted keys.

| Key | Type | Default | Description |
|---|---|---|---|
| `name` | string | — | Profile name (referenced by `default_profile` and `[ai.tasks]`). |
| `kind` | `"anthropic"` \| `"openai"` \| `"openai_responses"` \| `"ollama"` \| `"lm_studio"` \| `"llama_cpp"` \| `"vllm"` \| `"localai"` \| `"gemini"` \| `"custom"` | — | Provider API. |
| `base_url` | string | — | Empty = the kind's default. |
| `model` | string | — | Model id sent to the provider. |
| `key` | string | — | `credential:<name>` \| `env:<VAR>` \| empty (no key). |
| `tool_mode` | `"auto"` \| `"native"` \| `"json"` | — | How tools are offered: `auto` probes the model, else native tool calling or the JSON fallback. (`auto`: Native tool calling when the capability probe says it works, else the JSON protocol; `json`: Constrained JSON tool protocol in plain text (for models without tool calling)) |
| `monthly_budget_usd` | float | — | Monthly spending cap in USD for this profile; 0 = no cap. |
| `headers` | table: name → string | — | Extra HTTP headers (custom profiles, proxies). Values are not secret. |
| `max_tokens` | integer | — | Most output tokens per model reply. |
| `vision` | boolean | — | The model accepts images (vision): "Suggest names" sends downscaled pictures from folders whose AI access is "contents". |

## `[ai.tasks]`

Only sub-tables (below).

## `[ai.tasks.fast]`

A `(profile, model)` pair for one kind of task; empty fields fall back to the default profile / that profile's model.

| Key | Type | Default | Description |
|---|---|---|---|
| `profile` | string | `""` | Profile name (empty = `default_profile`). |
| `model` | string | `""` | Model id override (empty = the profile's model). |

## `[ai.tasks.strong]`

A `(profile, model)` pair for one kind of task; empty fields fall back to the default profile / that profile's model.

| Key | Type | Default | Description |
|---|---|---|---|
| `profile` | string | `""` | Profile name (empty = `default_profile`). |
| `model` | string | `""` | Model id override (empty = the profile's model). |

## `[ai.scope]`

Which paths AI tools may touch.

| Key | Type | Default | Description |
|---|---|---|---|
| `allowed_roots` | array of strings | `[]` | Empty = the home folder. |
| `blocked` | array of strings | `[]` | Extra blocked glob patterns (matched against the full path, `/` separators). |
| `allow_sensitive` | boolean | `false` | Allow the built-in sensitive paths (`.ssh`, `.env`, keychains, browser profiles, wallets). Off by default. |
| `max_read_kib` | integer | `256` | Size cap for `read_text`, KiB. |

## `[[ai.prices]]`

One price-table row; `model` is a glob (`claude-opus-*`). Prices in USD per million tokens.

A list of tables; 7 entries by default. The default column shows the first entry.

| Key | Type | Default | Description |
|---|---|---|---|
| `model` | string | `"claude-fable-5*"` | Model id glob (`claude-opus-*`). |
| `input_per_mtok` | float | `10.0` | USD per million input tokens. |
| `output_per_mtok` | float | `50.0` | USD per million output tokens. |
| `cache_read_per_mtok` | float | `1.0` | USD per million cached input tokens read. |
| `cache_write_per_mtok` | float | `12.5` | USD per million input tokens written to the prompt cache. |

## `[ai.mcp]`

`[ai.mcp]`: the Soyuz MCP server.

| Key | Type | Default | Description |
|---|---|---|---|
| `server_enabled` | boolean | `true` | The running app listens on a local named pipe so `soyuz --mcp` can attach and show actions in the live window. Off = `soyuz --mcp` always runs headless. |
| `http_enabled` | boolean | `false` | Streamable HTTP transport on 127.0.0.1 (each client needs its own bearer token). |
| `http_port` | integer | `7863` | Port of the HTTP transport on 127.0.0.1. |
| `allowed_origins` | array of strings | `[]` | Extra browser origins allowed to call the HTTP endpoint (`http://localhost:<port>` and `http://127.0.0.1:<port>` are always allowed; requests without `Origin` too). |
| `register_unknown` | boolean | `true` | Register unknown stdio clients as "pending" (read-only until approved). Off = unknown clients are refused. |
| `approval_timeout_secs` | integer | `300` | Seconds an approval card waits for an answer before the call is refused. |
| `approval_ui` | `"panel"` \| `"dialog"` | `"panel"` | Where MCP approval cards appear. (`panel`: In the assistant panel's approvals area (the panel opens when a request arrives); `dialog`: A modal card over the window (same plan card, one request at a time)) |

## `[[ai.mcp_clients]]`

`[[ai.mcp_clients]]`: an MCP client allowed to use Soyuz.

A list of tables; empty by default. Defaults below are for a new entry's omitted keys.

| Key | Type | Default | Description |
|---|---|---|---|
| `name` | string | — | Identity: the `--mcp-client <name>` argument (stdio) or the name reported by the client; HTTP clients are identified by their token. |
| `pending` | boolean | — | Registered automatically on first connect; read-only until approved in Settings. |
| `token` | string | — | Bearer token for the HTTP transport (`credential:<name>`); empty = stdio only. |
| `allowed_roots` | array of strings | — | Folders this client may use (empty = `[ai.scope] allowed_roots`). Always narrowed to the global AI scope. |
| `max_risk` | `"read"` \| `"write"` \| `"destructive"` | — | `read`: Read-only tools (listing, search, metadata, UI navigation); `write`: File changes through the jobs engine (copy, move, rename, create …); `destructive`: Trash, permanent delete, sync with deletions, custom commands. |
| `require_approval` | boolean | — | Write / destructive tools show an approval card in Soyuz first. Off = they run without asking (only within `max_risk`). |

## `[[ai.mcp_servers]]`

`[[ai.mcp_servers]]`: an external MCP server mounted as extra assistant tools.

A list of tables; empty by default. Defaults below are for a new entry's omitted keys.

| Key | Type | Default | Description |
|---|---|---|---|
| `name` | string | — | Name shown in the assistant's tool list. |
| `enabled` | boolean | — | Connect to this server. |
| `command` | string | — | stdio transport: program to start (no shell is involved) and its arguments. |
| `args` | array of strings | — | Arguments for `command`. |
| `env` | table: name → string | — | Extra environment variables for `command` (values are not secret). |
| `url` | string | — | Streamable HTTP transport: endpoint URL (used instead of `command` when set). |
| `token` | string | — | Bearer token for `url` (`credential:<name>` \| `env:<VAR>` \| empty). |
| `risk` | `"read"` \| `"write"` \| `"destructive"` | — | Risk class of this server's tools (they can do anything the server can). (`read`: Read-only tools (listing, search, metadata, UI navigation); `write`: File changes through the jobs engine (copy, move, rename, create …); `destructive`: Trash, permanent delete, sync with deletions, custom commands) |
| `trust_annotations` | boolean | — | Treat tools the server marks `readOnlyHint` as read-only (only for servers you trust). |
| `require_approval` | boolean | — | Tools above read risk need approval in Soyuz before they run. |
| `timeout_secs` | integer | — | Per-request timeout in seconds. |

## `[ai.agent]`

in-app assistant harness (Ctrl+I)

| Key | Type | Default | Description |
|---|---|---|---|
| `max_steps` | integer | `24` | Model turns per task before the harness stops. |
| `token_budget` | integer | `400000` | Tokens (input + output, all turns) per task; 0 = unlimited. |
| `time_limit_secs` | integer | `600` | Wall-clock limit per task, seconds. |
| `max_auto_reads` | integer | `60` | Read tools the agent may run on its own per task. |
| `compact_after_kib` | integer | `96` | Summarise old tool results once the conversation is larger than this (KiB of text). |
| `keep_recent_results` | integer | `6` | Most recent tool results kept verbatim when compacting. |
| `approval` | `"always"` \| `"auto_write"` | `"always"` | always \| auto_write (conflict-free write plans run unasked, never after reading file contents; destructive always asks) (`always`: Every plan waits for the user (default); `auto_write`: Conflict-free plans of the *write* class run without asking, except when the plan was proposed right after the assistant read untrusted file contents. Destructive plans always ask) |
| `max_risk` | `"read"` \| `"write"` \| `"destructive"` | `"destructive"` | read \| write \| destructive: highest tool class offered to the model (`read`: Read and UI tools only; `write`: Plus copy / move / rename / create (always as plans); `destructive`: Plus trash / delete / sync (always need approval)) |
| `confirm_cost_usd` | float | `0.5` | Ask before starting a task whose estimated cost exceeds this (USD; 0 = never ask). |
| `listing_entries` | integer | `80` | Entries of the current folder summarised into the task context. |
| `redact_cloud` | boolean | `true` | Mask e-mail addresses, API keys and tokens in file data sent to cloud providers. |
| `name_excerpt_chars` | integer | `1500` | "Suggest names": characters of each text / document file sent to the model (only where the folder's AI access is "contents"; 0 = names and metadata only). |
| `vision_max_px` | integer | `768` | "Suggest names": longest side of the downscaled image sent to vision models (px; 0 = never send images). |
| `name_max_files` | integer | `40` | "Suggest names": most files looked at per request. |

## `[[ai.folders]]`

One `[[ai.folders]]` rule.

A list of tables; empty by default. Defaults below are for a new entry's omitted keys.

| Key | Type | Default | Description |
|---|---|---|---|
| `path` | string | — | Folder the rule applies to (and everything below it). |
| `access` | `"none"` \| `"metadata"` \| `"contents"` | — | What AI tools may see there. (`none`: The assistant cannot see the folder at all; `metadata`: Names, sizes, dates and types only; `contents`: File contents may be read (and sent to the model)) |

## `[[bookmarks]]`

A list of tables; empty by default. Defaults below are for a new entry's omitted keys.

| Key | Type | Default | Description |
|---|---|---|---|
| `name` | string | — | Label in menus and the palette. |
| `path` | string | — | Folder the bookmark opens. |
| `hotkey` | string | — | Chord string, empty = none. |
| `icon` | string | — | Icon name (e.g. `"code"`), empty = folder. |

## `[[custom_commands]]`

A list of tables; empty by default. Defaults below are for a new entry's omitted keys.

| Key | Type | Default | Description |
|---|---|---|---|
| `name` | string | — | Label in the context menu and palette. |
| `command` | string | — | Program to run (no shell is involved unless you start one). |
| `args` | array of strings | — | Placeholders: `%p` focused path, `%n` selected names, `%s` selected paths, `%d` current dir, `%o` other pane's dir, `%%` a literal `%` (legacy: `%P` paths, `%D` dir, `%N` name, `%E` ext, `%F` first selected). An argument that is exactly `%s` / `%n` / `%P` expands to one argument per selected item. |
| `context` | array of `"file"` \| `"dir"` \| `"background"` | — | Where the command appears: on a `file`, a `dir`, or the folder `background`. Empty = everywhere. |
| `hotkey` | string | — | Chord string that runs it (empty = none). |
| `terminal` | boolean | — | Run in the integrated terminal panel instead of as a detached process. |

## `[scripting]`

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | boolean | `true` | Load `scripts/*.rhai` (off = no script actions, columns or previewers). |
| `max_operations` | integer | `5000000` | Rhai operations one script call may run before it is stopped. |
| `timeout_ms` | integer | `3000` | Wall-clock limit of an action / rule script call, in milliseconds. |
| `cell_timeout_ms` | integer | `250` | Wall-clock limit of one column value or previewer call, in milliseconds. |
| `allow_read` | boolean | `true` | Scripts may read files (`read_text`, `list`, `stat`, `search`). Off = they only see the paths they are given. |
| `max_read_kib` | integer | `1024` | Largest `read_text` result, in KiB (longer files are cut). |
| `max_list_entries` | integer | `10000` | Most entries `list` / `search` return. |
| `approve_writes` | boolean | `true` | Show the planned jobs of a script action for approval before they run. Off = they are submitted directly (still visible in the activity panel and undoable). |
| `disabled` | array of strings | `[]` | Script file names (`tidy.rhai`) that are not loaded. |

## `[automation]`

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | boolean | `true` | Run the rules while Soyuz is open. |
| `dry_run` | boolean | `false` | Global dry run: rules only log what they would do. |
| `log_entries` | integer | `200` | Run-log entries kept per rule (in memory). |

## `[[automation.rules]]`

A list of tables; empty by default. Defaults below are for a new entry's omitted keys.

| Key | Type | Default | Description |
|---|---|---|---|
| `name` | string | — | Rule name (shown in the run log and job titles). |
| `enabled` | boolean | — | Run this rule. |
| `watch` | string | — | Folder to watch. |
| `recursive` | boolean | — | Include subfolders. |
| `glob` | string | — | Name globs separated by `;` (`"*.pdf;*.epub"`, empty = any name). |
| `regex` | string | — | Optional regular expression on the name (case-insensitive). |
| `kind` | `"files"` \| `"folders"` \| `"any"` | — | Match files, folders or both. |
| `min_size_kib` | integer | — | Lower size bound in KiB (0 = no bound). |
| `max_size_kib` | integer | — | Upper size bound in KiB (0 = no bound). |
| `min_age_minutes` | integer | — | Lower age bound by modification time, in minutes (0 = no bound). |
| `max_age_minutes` | integer | — | Upper age bound in minutes (0 = no bound). |
| `delay_secs` | integer | — | Wait this long after the last change before acting (lets downloads finish). |
| `apply_to_existing` | boolean | — | Also act on matching items already in the folder when the rule starts. |
| `dry_run` | boolean | — | Only log what would happen. |
| `conflict` | `"keep_both"` \| `"skip"` \| `"overwrite"` \| `"ask"` | — | Name conflicts at the target. (`keep_both`: Keep both (the new item gets a numbered name); `overwrite`: Replace the existing item (it goes to the undo history like any overwrite); `ask`: Ask in the activity panel) |
| `actions` | array of strings | — | Steps run on each match, in order. |

## `[cli]`

`[cli]`: the `soyuz cp|mv|rm|…` companion.

| Key | Type | Default | Description |
|---|---|---|---|
| `server_enabled` | boolean | `true` | The window accepts jobs from `soyuz cp\|mv\|rm\|trash\|open\|reveal\|jobs` over a local named pipe (current user only). Off = the CLI always runs its own engine. |

