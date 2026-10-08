# Soyuz user guide

A practical tour of Soyuz: how the window is laid out, how to move around quickly, how
file operations behave, and where every setting lives. Shortcuts below are the defaults
of the *Explorer* preset; the command palette always shows the active ones.

- [The window](#the-window)
- [Moving around](#moving-around)
- [Selecting and acting on files](#selecting-and-acting-on-files)
- [Background jobs, conflicts and undo](#background-jobs-conflicts-and-undo)
- [Panels: preview, terminal, activity, shelf](#panels-preview-terminal-activity-shelf)
- [Tools](#tools)
- [Search](#search)
- [Customizing](#customizing)
- [Accessibility](#accessibility)
- [Portable mode and data files](#portable-mode-and-data-files)
- [Troubleshooting](#troubleshooting)

## The window

```
┌ sidebar ─┬ tabs ────────────────────────────────────────────────┐
│ Favorites│ ← → ↑  Home › Projects › soyuz        Filter   ⌘ ▤ ◫ │  toolbar + omnibar
│ Devices  ├──────────────────────────────────────────────────────┤
│ Cloud    │  file list (details / list / grid / gallery / Miller) │  preview pane →
│ Tags     │                                                      │
│ Trash    ├──────────────────────────────────────────────────────┤
│          │  terminal · activity (toggle with Ctrl+` / Ctrl+⇧J)  │
└──────────┴──────────────────────────────────────── status bar ──┘
```

- **Tabs** (`Ctrl+T`, `Ctrl+W`, `Ctrl+Shift+T` reopens) can be pinned, dragged and opened
  from the middle mouse button. A tab can hold **two panes** (`F9`) side by side or stacked;
  `Tab` switches between them and the splitter is draggable (double-click resets it).
- The **omnibar** shows the path as clickable segments; the small chevrons open sibling
  folders. Click the empty area or press `Ctrl+L` to type a path (with completion).
- The **status bar** shows counts, selection size, free space, an icon-size slider, view
  and theme switches, and a ring when jobs are running.

## Moving around

| | |
|---|---|
| `Enter` / double-click | open a folder or a file (default app) |
| `Backspace`, `Alt+↑` | parent folder |
| `Alt+←` / `Alt+→`, mouse back / forward | history (right-click the arrows for the list) |
| `Ctrl+P` | **Go to folder**: fuzzy jump to any folder you visited, ranked by frecency |
| `Ctrl+K` | **Command palette**: every action; prefixes `:` path, `@` bookmark, `/` search, `>` shell, `#` tab |
| typing letters | type-ahead jumps to the first matching name (or filters, see *Behaviour*) |
| `Ctrl+F` | live filter of the current folder |
| `Alt+1…9` | bookmarks |

Views: `Ctrl+Shift+1…5` for details, grid, list, Miller columns and gallery. Each folder
remembers its view, sort and columns; *View → Apply to all folders* makes it the default.
`Ctrl+Alt+=` / `Ctrl+Alt+-` (or `Ctrl`+wheel) change icon size in grid and gallery.

## Selecting and acting on files

Click, `Shift`+click and `Ctrl`+click select; drag on empty space for a rubber band;
`Ctrl+A`, `Ctrl+I` (invert) and *Select by pattern* (palette) cover the rest. Checkbox
mode (status bar or palette) adds tick boxes for mouse and touch users.

| | |
|---|---|
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | copy / cut / paste — works with other apps' file clipboard too |
| `F5` / `F6` | copy / move to the other pane |
| `F2` | rename in place (`Ctrl+Shift+R` batch rename with live preview, regex and counters) |
| `Del` / `Shift+Del` | trash / delete permanently (confirmations are configurable) |
| `Ctrl+Shift+N` / `Ctrl+Alt+N` | new folder / new file |
| `Alt+Enter` | properties: attributes, times, permissions, checksums, details |
| `Space` | Quick Look; arrows move to the next item while it is open |

Drag and drop works between panes, tabs, the sidebar, the shelf and other applications.
The default drop effect follows the platform convention (move on the same drive, copy
across drives); hold `Ctrl` to copy, `Shift` to move, or set *Behaviour → Drag and drop*
to *Ask*.

## Background jobs, conflicts and undo

Every operation runs as a job; the window stays responsive. Small jobs finish quietly
with a toast (with **Undo**); longer ones open the **activity panel** (`Ctrl+Shift+J`)
automatically. Each job card shows progress, speed graph and ETA, and can be paused,
resumed, cancelled, reprioritized or started immediately.

When a destination already exists, the **conflict dialog** compares both files (size,
date, which is newer / larger) and offers *Replace*, *Keep both*, *Skip* and more, with
*Do this for all remaining conflicts*. Errors offer *Retry*, *Skip* and *Skip all*.

`Ctrl+Z` / `Ctrl+Y` undo and redo copies, moves, renames, trashing, new items and
attribute changes. *History* in the activity panel lists past operations with undo, redo
and *Retry failed*.

## Panels: preview, terminal, activity, shelf

- **Preview** (`Alt+P`): syntax-highlighted text, markdown, images, hex for binaries, plus
  file info. *Toggle hex preview* (palette) switches to the hex view.
- **Terminal** (``Ctrl+` ``): your shell in the current folder. It follows navigation (and,
  optionally, navigates the pane when you `cd`). `Esc` on an empty line returns to the
  file list; `Ctrl`+click opens links and paths.
- **Shelf** (palette: *Toggle shelf*): drop files from anywhere, keep browsing, then drag
  them out or copy / move them all at once. Its contents survive restarts.
- **Editor** (`F4`, right-click → *Edit in Soyuz*, palette *Toggle editor panel*): files
  open as tabs in a panel docked right or bottom (or maximized over the panes).
  - *Text*: syntax highlighting, line numbers, find / replace (`Ctrl+F` / `Ctrl+H`; case,
    whole word, regex with `$1`), go to line (`Ctrl+G`), undo / redo, `Ctrl+S`. The
    encoding (UTF-8, UTF-8 BOM, UTF-16) and line endings are kept; click *LF/CRLF* in the
    status line to switch. Files over `[editor] max_text_mib` open read-only in hex.
  - *Hex*: any size, overwrite in the hex or text column (`Tab` switches), data
    inspector, go to offset, find hex bytes or text.
  - *Image*: zoom (wheel, `+`/`-`/`0`/`1`), rotate (`R`), flip (`H`/`V`), crop (`C`, drag,
    `Enter`). PNG/BMP/TIFF/WebP/GIF save losslessly; JPEG is re-encoded.
  - Saving puts the previous version in the trash as `<name>.bak` (`[editor]
    backup_to_trash`). Files changed on disk reload by themselves, or ask when you have
    unsaved changes. Closing a modified tab or the window asks first.

## Tools

All from the palette or the context menu:

- **Disk usage treemap** — where the space went; click to zoom in, sidebar lists the largest items.
- **Compare folders / Synchronize** — by date & size, size or content; review the plan before syncing.
- **Find duplicates** — by size then hash; select keepers and trash the rest.
- **Split / join files**, **checksums** (BLAKE3, SHA-256, SHA-1, MD5, CRC32), **archives**
  (create, extract here / to folder, test; browse zip / 7z / tar / RAR / ISO / CAB / .deb /
  .rpm like folders, including split `.7z.001` / `.zip.001` sets; inside a zip, paste to
  add files, Rename and Delete edit the archive, and the original goes to the trash so it
  can be undone).
- **Tags** — eight color labels (`Add tag` actions); the sidebar lists tagged items.
  Tags follow files you rename or move inside Soyuz.

## Search

`Ctrl+Shift+F` (or `F3`) opens the search bar for the current folder and its subfolders:
name patterns (substring, glob, regex, fuzzy), content search, size / date / kind filters,
`.gitignore` awareness. Results stream into a normal file view, so every action works on
them. Save a search to keep it in the sidebar.

In the default **Contains** mode the text is a small query language:

| Example | Meaning |
|---|---|
| `annual report`, `"exact phrase"` | the name contains every word / the phrase |
| `*.pdf`, `ext:pdf,docx` | name pattern, extensions |
| `kind:image` | `image video audio document archive code file folder` |
| `size:>5MB`, `size:1k..2M`, `size:empty` | file size (binary units) |
| `modified:<7d`, `modified:>1y`, `created:2026-01`, `modified:today` | age (`min h d w mo y`), date, keyword |
| `tag:red`, `path:docs`, `in:"~/Projects"` | color tag, text in the path, below a folder |
| `content:todo`, `regex:^IMG_\d+` | words inside text files, name regex |
| `-draft`, `a OR b`, `(a b) OR c` | exclude, either, group |

**Background index** (Settings → Preview & search → Index, off by default): Soyuz keeps
names, sizes, dates and tags of the indexed folders (default: your home folder, minus
caches and `node_modules`-style folders) in a local SQLite database, so searches there
answer in milliseconds; the scope chip of the search bar can then search **all indexed
folders** at once. The status bar shows what the indexer is doing (click it for the
settings, right-click to rebuild). File contents can be indexed too (`content:`). On NTFS,
running Soyuz as administrator lets it read the change journal, so changes made while it
was closed are applied in seconds; otherwise it re-checks the folders in the background
after each start.

**Smart folders** are saved queries in the sidebar that stay up to date: create one with
**Smart folder** in the search status strip or **New smart folder…** in the sidebar;
right-click one to edit, rename or delete it (deleting only removes the query).

## Customizing

**Settings** (`Ctrl+,`) covers every section of `config.toml` and applies changes live.
Search the settings with `Ctrl+F` inside the window.

- *Appearance*: theme (system / dark / light), accent color, density, font size, interface
  scale, reduced motion, high contrast, icons and thumbnails.
- *Keyboard*: presets (Explorer, Commander, Finder, Vim), click any shortcut to record a new
  one, conflict and shadowing warnings, multi-key sequences such as `g g`.
- *Custom commands*: run programs with placeholders (`%p` focused path, `%s` selected
  paths, `%d` current folder, `%o` other pane …), in the palette, the context menu or a hotkey.
- *Config file*: open `config.toml` in your editor, reload, **Export…** / **Import…** through
  a built-in file picker, or reset everything.

`config.toml` is hot-reloaded: save it in any editor and Soyuz applies it within a second.
A typo is reported with line and column, and the last good settings stay active. When the
settings window saves, it merges into your file and keeps comments and ordering. Most
settings apply instantly; the job queue settings (parallel jobs, queue mode, "apply to all"
reset, history size) apply after a restart, which the settings page notes.

The full reference with defaults is [`config.example.toml`](config.example.toml).

## Accessibility

- **Screen readers**: Soyuz exposes an AccessKit tree (UI Automation on Windows, AT-SPI on
  Linux, NSAccessibility on macOS). Buttons carry their tooltip as name, tabs and toggles
  report their state, file rows are list items with position and selection, job progress
  is a progress indicator, dialogs are dialogs, and toasts are announced politely.
- **Keyboard**: everything has a shortcut or a palette command; inside dialogs and settings
  `Tab` / `Shift+Tab` move through controls with a visible focus ring, `Enter` activates,
  `Esc` closes.
- **High contrast** (*Appearance* or palette *Toggle high contrast*): black / white surfaces,
  AAA text contrast, stronger borders and focus rings.
- **Reduced motion**: turns off panel, dialog, toast and hover animations.
- **Touch density**: 40 px rows and larger buttons, menus and scrollbars.
- **Interface zoom**: `Ctrl+=`, `Ctrl+-`, `Ctrl+0` scale the whole interface (50–300 %),
  stored as `appearance.ui_scale`.

## Portable mode and data files

Put an empty file named `portable` next to the executable and Soyuz keeps everything in
`portable-data/` beside it instead of the user profile:

| File | Contents |
|---|---|
| `config.toml` | your settings |
| `session.toml` | tabs, panes, selection, shelf, small UI state |
| `views.toml` | per-folder view memory |
| `history.jsonl` | job history (for undo) |
| `palette.toml` | frecency, recent commands, marks |
| `tags.toml`, `recent.toml` | tags and recent places |
| `smart_folders.toml` | smart folders (saved live queries) |
| `index/` | the background search index (machine-local; `%LOCALAPPDATA%\soyuz\data\index` when not portable) |

## Troubleshooting

- **Blank or flickering window** — try the OpenGL renderer: build with `--features glow`
  and start with `soyuz --renderer glow`.
- **Copies seem slow on a network share** — lower *File operations → Parallel jobs* or set
  the queue mode to *Per device*.
- **A setting does not stick** — check for a parse error toast; the file may have been
  edited into an invalid state. *Settings → Config file → Reload from disk* re-reads it.
- **Logs** — run from a terminal with `RUST_LOG=debug soyuz` (release builds on Windows
  have no console; use the debug build).
