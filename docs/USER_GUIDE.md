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
- [Installing, updates and the default file manager](#installing-updates-and-the-default-file-manager)
- [Portable mode and data files](#portable-mode-and-data-files)
- [Troubleshooting](#troubleshooting)

## The window

```
┌ sidebar ─┬ tabs ────────────────────────────────────────────────┐
│ [▤] [⑃]  │ ← → ↑  Home › Projects › soyuz        Filter   ⌘ ▤ ◫ │  toolbar + omnibar
│ Favorites├──────────────────────────────────────────────────────┤
│ Devices  │  file list (details / list / grid / gallery / Miller) │  preview pane →
│ Portal   │                                                      │
│ Tags     │                                                      │
│ Other    ├──────────────────────────────────────────────────────┤
│ Network  │  terminal · activity (toggle with Ctrl+` / Ctrl+⇧J)  │
│ Cloud    │                                                      │
└──────────┴──────────────────────────────────────── status bar ──┘
```

- **Tabs** (`Ctrl+T`, `Ctrl+W`, `Ctrl+Shift+T` reopens) can be pinned, dragged and opened
  from the middle mouse button. A tab can hold **two panes** (`F9`) side by side or stacked;
  `Tab` switches between them and the splitter is draggable (double-click resets it).
- The **omnibar** shows the path as clickable segments; the small chevrons open sibling
  folders. Click the empty area or press `Ctrl+L` to type a path (with completion).
- The **status bar** shows counts, selection size, free space, an icon-size slider, view
  and theme switches, and a ring when jobs are running.

### The sidebar

A small icon bar at the top of the sidebar switches between two views (`Ctrl+Shift+E`
toggles too):

- **Places**: collapsible sections, from top to bottom **Favorites**, **Devices**,
  **Portal**, **Tags**, **Other** (Recent, Timeline, Trash), **Network**, **Cloud** and
  Smart folders. Choose and reorder them in *Settings → Views → Sidebar sections*.
- **Disk tree**: your drives and home folder as one folder tree that fills the whole
  sidebar (with a filter box on top). *Settings → Views → Sidebar content → Both* shows the
  tree as a section below the places instead.

**Portal** is a drop zone for the folders you are working with: drag one or more folders
onto the Portal section (from the file list, the disk tree, the shelf, or from Explorer)
and they are pinned there. Dropping only pins (nothing is copied or moved); files,
duplicates and folders that no longer exist are skipped. Click a pin to open it,
middle-click to open it in a new tab, drag pins to reorder them, and right-click for open
in a new tab / other pane, reveal, copy path, **Rename label…** and **Unpin**. A pin whose
folder is gone is dimmed and marked *missing* until you unpin it or the folder returns.
Pins are kept in `config.toml` (`[[sidebar.portal]]`) and can also be edited in
*Settings → Views → Portal*.

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
`Ctrl+A`, `Ctrl+Shift+I` (invert) and *Select by pattern* (palette) cover the rest. Checkbox
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
to *Ask*. While dragging, rest on a folder, a breadcrumb segment, a sidebar place or
another tab for a moment and it opens (spring-loaded folders; delay and on/off in
*Behaviour → Drag and drop*).

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
  Richer previews (each can be switched off in *Settings → Preview & search*, `[preview]`):
  - **PDF**: pages render with pdfium (`PgUp` / `PgDn` flip pages in Quick Look; a *Text*
    tab shows the extracted text). Soyuz does not bundle or download pdfium: get a prebuilt
    `pdfium.dll` (for example from the *bblanchon/pdfium-binaries* GitHub releases, the
    `pdfium-win-x64` archive's `bin/pdfium.dll`) and put it next to `soyuz.exe`, in the
    config folder, or in the folder named by `[preview] pdfium_dir`. No restart is needed.
    With pdfium present the grid and gallery views also show the first page as the PDF's
    thumbnail (`[appearance] pdf_thumbnails`).
    Without it, PDFs show their page count, document info and text (pure-Rust fallback).
  - **Office / OpenDocument**: docx / odt text with headings, bold, italic and lists;
    xlsx / ods first sheet as a table (`sheet_rows`); pptx / odp slide titles and bullets.
  - **EPUB**: cover, title, authors, description and the table of contents.
  - **Fonts** (ttf, otf, ttc, woff): a specimen, the character set and a glyph grid. WOFF 2
    is not decoded.
  - **3D models** (glTF / GLB, STL, OBJ): a shaded quick render; drag to rotate, scroll to
    zoom in Quick Look, double-click to reset. Models spin slowly unless reduced motion or
    `model_turntable = false`.
  - **Audio**: waveform, tags, cover art, duration and bitrate, with play / pause / seek
    (click the waveform). Playback stops when the preview moves away.
  - **Video**: container metadata, plus a poster frame when `ffmpeg` is installed (on
    `PATH`, or set `ffmpeg_path`). Playback inside Soyuz is out of scope; `Enter` opens
    the file in your player.
  - **Diff**: select exactly two text files and the preview compares them (older on the
    left; swap, unified or side by side).
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
  **Two-way sync** remembers the last sync, so a file deleted on one side is deleted on the
  other and edits on both sides become *conflicts* (keep left, keep right, keep both, skip,
  or apply one choice to all). Save the folders as a **profile** and give it a schedule in
  **Sync profiles** (every N minutes, daily at a time, when files change, or at start);
  scheduled runs are normal jobs, never guess on conflicts, and each run (or *dry run*)
  leaves a report of what changed or would change. Nothing runs while Soyuz is closed.
- **Previous versions** — Properties › Previous versions, or *Restore previous version…*
  in the context menu, lists earlier copies of a file or folder from Windows shadow copies
  (restore points, Windows Backup). Open, compare, copy out or restore one (an undoable copy
  job that asks before replacing). Reading shadow copies needs administrator rights; the
  panel says so and can start an elevated Soyuz.
- **Paste and verify** — like Paste, then every copy is read back and hash-compared; jobs
  that verified their copies show a *Verified* badge in the activity panel and history
  (or turn verification on for every copy in Settings › Jobs).
- **Find duplicates** — by size then hash; select keepers and trash the rest.
- **Split / join files**, **checksums** (BLAKE3, SHA-256, SHA-1, MD5, CRC32), **archives**
  (create, extract here / to folder, test; browse zip / 7z / tar / RAR / ISO / CAB / .deb /
  .rpm like folders, including split `.7z.001` / `.zip.001` sets; inside a zip, paste to
  add files, Rename and Delete edit the archive, and the original goes to the trash so it
  can be undone).
- Tool windows (treemap, compare, duplicates, previous versions, split / join, sync
  profiles) sit over a dimmed main window that ignores clicks and shortcuts until you close
  them (Esc closes the frontmost one). Result rows in **Compare** (left / right column =
  that side's copy), **Find duplicates** and **Previous versions** can be dragged onto a
  pane, folder, tab or sidebar entry behind the tool, or out to the desktop / other apps;
  previous versions copy by default.
- **Tags** — eight color labels (`Add tag` actions); the sidebar lists tagged items.
  Tags follow files you rename or move inside Soyuz. To give a tag your own name,
  right-click it in the sidebar → *Rename tag…* (Enter saves, Esc cancels; *Reset name*
  brings the color name back), or edit Settings → Views & layout → Tag names
  (`[tags] names` in `config.toml`). Items stay tagged by color, so renaming changes
  nothing on disk; `tag:red` keeps working in searches.

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

- *Appearance*: color theme (Orbit, Nord, Solarized, Rosé Pine, High Contrast or your own
  files in `themes/`, each with a dark and a light variant), mode (system follows the OS
  switch live / dark / light), accent color (the theme's, the Windows accent, or any color),
  density, font size, interface scale, reduced motion, high contrast, icons and thumbnails
  (icon pack, extension badges, folder mosaics, ffmpeg video frames, PDF first pages
  through pdfium, the on-disk thumbnail cache). **Import theme…** / **Export current…**
  share theme files.
- *Icon packs*: *Appearance → Icon pack* switches between the built-in glyphs, the built-in
  *Flat* pack and your own packs, with a live sample row. A pack is a folder in `icons/`
  (the **Icons folder** button opens it) with a `pack.toml`; pictures are PNG / JPEG /
  WebP / BMP / ICO or SVG, relative to the pack folder:

  ```toml
  name = "My icons"      # shown in Settings (default: the folder name)
  tint = false           # true: white pictures tinted with the theme's file-type colors
  [kinds]                # folder, image, video, audio, archive, code, pdf, doc, sheet,
  folder = "folder.svg"  # slides, text, exe, font, link, file (= everything else)
  file = "file.png"
  [extensions]
  rs = "rust.png"
  [folders]              # folder names, case-insensitive
  ".git" = "git.svg"
  ```

  Anything a pack leaves out keeps the built-in glyph.
- *Fonts*: one font per role — interface (Inter), headings (its semibold face), terminal,
  code editor and hex view (Cascadia Mono), numbers (the interface font). Soyuz ships ten
  screen fonts that need no installation: Inter, IBM Plex Sans and Noto Sans (each with a
  SemiBold face for headings), and Cascadia Mono, JetBrains Mono, IBM Plex Mono and Source
  Code Pro for monospace text. The family field suggests these first (marked *Embedded*),
  then your installed fonts such as Segoe UI or Consolas. The *Embedded* switch ignores
  installed fonts. Each role also has a size offset, a live preview and a notice when a
  font is not installed. Under *Rendering*: hinting, hinting strength (*Crisp* is the
  sharpest at 100 % scaling), sub-pixel positioning and the text weight for dark and light
  themes.
- *Folder colors*: right-click folders → **Folder color** for a color and an emblem; it
  shows on the folder icon, in the breadcrumb and on tabs.
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

## Scripts, automations and the command line

- **Scripts** (Settings → Scripts): Rhai files in the `scripts` folder next to `config.toml`, reloaded on save. A script registers actions (`register_action`, palette + key binding `run_script:<file>/<id>`), right-click entries with conditions (`register_menu_item`), columns (`register_column`, choose them in the column menu) and previewers (`register_previewer`). Scripts run sandboxed with time and operation limits; every change they ask for is shown for approval and runs as an undoable job. Start from a template on the Scripts page.
- **Automations** (Settings → Automations): rules that watch a folder and act on matching items (name globs / regex, kind, size, age) after they stop changing: move, copy, rename with a pattern, tag, trash or run a script. Each rule shows what it would match right now, has *Run now*, a dry-run switch and a run log. Every step is a job with a robot badge in the activity panel, so it can be undone.
- **Command line**: `soyuz cp|mv|rm|trash|ls|open|reveal|jobs` (or `soyuz-cli`, which a shell waits for). With a Soyuz window open the jobs run there, with progress in the activity panel and conflict prompts in the window; otherwise they run in the terminal. `--queue` returns at once, `--json` prints machine-readable events, `--conflict skip|overwrite|rename|ask`. Exit codes: 0 ok, 1 failed, 2 usage, 3 cancelled, 4 unavailable, 5 not found.

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

## Installing, updates and the default file manager

- **Installer (MSI)** — installs for your account only (no admin) to
  `%LOCALAPPDATA%\Programs\Soyuz`, with a Start menu shortcut and an entry in *Installed apps*
  for uninstalling. `msiexec /i soyuz-<version>-windows-x64.msi ALLUSERS=1` installs for all
  users instead. Optional features: "Open in Soyuz" on the folder right-click menu, and adding
  Soyuz to `PATH` so `soyuz C:\some\folder` works in a terminal. Newer MSIs upgrade in place.
- **Portable zip** — `soyuz-<version>-windows-x64-portable.zip` runs from any folder and keeps
  its settings beside the exe (see below).
- **Updates** — Settings → *About & updates*. Enter the GitHub repository Soyuz is released
  from (`[updates] repo`), then **Check for updates**. Soyuz fetches only the public release
  list; it sends nothing about you. A newer release shows its notes and a **Download & verify**
  button: the file goes to your Downloads folder, its SHA-256 must match the release's
  `SHA256SUMS` file (or GitHub's digest), and installers must carry a valid Authenticode
  signature or be unsigned (a broken signature deletes the download). Nothing runs until you
  click **Run installer…** and confirm. *Check once a week* (off by default) only shows a
  notification.
- **Default file manager** — Settings → *General* (top) → **Make Soyuz the default** opens folders
  and drives from other apps, the desktop and Win+R in Soyuz. Soyuz writes an "Open in Soyuz"
  verb for `Directory` and `Drive` under `HKEY_CURRENT_USER\Software\Classes` (your account
  only) and remembers what was there before under `HKEY_CURRENT_USER\Software\Soyuz`.
  **Restore Explorer** removes exactly those entries and restores the previous default.
  Special folders (This PC, Control Panel, Recycle Bin) always stay with File Explorer.
  Restore Explorer before moving or deleting a portable copy that is the default.

## Linux

Soyuz runs on Linux desktops (Wayland or X11; tested on Fedora with GNOME Keyring and the
xdg-desktop-portal). Everything works without root; nothing system-wide is changed unless
you install system-wide yourself.

- **Install from the tarball** — `soyuz-<version>-linux-<arch>.tar.gz` is laid out like a
  prefix (`bin/soyuz`, `share/applications/soyuz.desktop`, icons, metainfo). For your account:
  `tar -C ~/.local --strip-components=1 -xf soyuz-<version>-linux-<arch>.tar.gz` (make sure
  `~/.local/bin` is on `PATH`); system-wide: the same with `sudo tar -C /usr/local …`.
  Uninstall by deleting those files. The portable mode below works on Linux too. A copy
  installed this way updates itself: Settings → *About & updates* downloads the new tarball,
  checks its SHA-256 checksum and offers **Install & restart** (a system-wide copy under
  `/usr/local` needs the `sudo tar` command above instead).
- **Install the RPM** — `sudo dnf install ./soyuz-<version>-1.*.<arch>.rpm` (Fedora,
  openSUSE); remove it with `sudo dnf remove soyuz`. Package-managed copies are updated by
  the package manager, never by Soyuz itself. Settings live in `~/.config/soyuz/`, the search
  index in `~/.local/share/soyuz/`.
- **Flatpak** — `flatpak run io.github.ulaas.Soyuz` (built from
  `installer/linux/flatpak/`, see `docs/RELEASING.md`). It has access to all your files
  (`--filesystem=host`, including drives under `/run/media` and gvfs mounts), so browsing
  works as with the other packages. Differences inside the sandbox:
  - Settings and the search index live in `~/.var/app/io.github.ulaas.Soyuz/` (`config/soyuz/`,
    `data/soyuz/`), not `~/.config/soyuz/`. Put `libpdfium.so` in that `config/soyuz/` folder
    for PDF pages (the app's own `/app` folder is read-only).
  - Updates come from `flatpak update`; Soyuz only tells you a new version exists.
  - Files open with your default apps through the desktop portal (it may ask which app to
    use the first time). *Open with*, *Open terminal here* and terminal apps run on the host
    with `flatpak-spawn --host`; *Open with* lists only the apps the sandbox can see (other
    Flatpak apps), so for a host app use the portal's chooser via *Open*.
  - *Make Soyuz the default* writes `inode/directory=io.github.ulaas.Soyuz.desktop` to your
    real `~/.config/mimeapps.list`, and *Install service* writes a D-Bus service that starts
    `flatpak run … io.github.ulaas.Soyuz`; both are undone by their buttons as usual. The
    trash, the GTK bookmarks, `user-dirs.dirs`, the shared thumbnail cache and your fonts
    (also `~/.local/share/fonts` and the system's) are the desktop's, not copies.
  - The built-in terminal runs inside the sandbox (the runtime's `bash` and tools, not your
    host's); use *Open terminal here* for a host shell. *Run as administrator* (`pkexec`) is
    not available.
  - The window is called `soyuz` (Wayland app id) while the launcher is
    `io.github.ulaas.Soyuz.desktop`; its `StartupWMClass=soyuz` lets GNOME and KDE match them
    (other docks may show a generic icon).
- **Desktop integration** — the desktop entry puts Soyuz in your app launcher. File types open with the app chosen in your desktop's settings
  (`mimeapps.list` and the installed desktop entries are read directly; no `xdg-open`), file
  and folder icons come from your icon theme, light / dark mode and the accent color follow
  the desktop (portal or KDE settings), and the Computer view lists drives from the mount
  table plus UDisks2 for mounting, unlocking and ejecting removable drives.
- **Default file manager** — Settings → *General* (top) → **Make Soyuz the default** sets
  Soyuz as the handler for folders (`inode/directory`) in `~/.config/mimeapps.list`, for your
  account only. **Restore previous** puts back the handler that was there before (the settings
  row names the current one). Separately, **Install service** under *"Show in folder" when
  Soyuz isn't running* writes a D-Bus service file
  (`~/.local/share/dbus-1/services/org.freedesktop.FileManager1.service`), so "Show in folder"
  in browsers and other apps starts Soyuz; **Remove** restores what was there before. While
  Soyuz is running it can also answer those requests itself (Settings → *General* →
  *Linux desktop* → *Answer "Show in folder"*, off by default so it doesn't take over from
  your desktop's file manager unasked; the row shows whether Soyuz is serving or which
  program already owns the name).
- **Clipboard** — copy / cut put files on the system clipboard in the formats GNOME Files,
  Dolphin and other apps use (`text/uri-list`, `x-special/gnome-copied-files`), and paste
  accepts files copied there. Soyuz talks to the clipboard itself: through the Wayland
  `ext-data-control` protocol (niri, Sway, KWin, Hyprland …) and otherwise through the X11
  clipboard (also via XWayland, e.g. under GNOME). Only if neither works in an unusual
  session, turn on *Clipboard helper programs* (*General → Linux desktop*), which then runs
  `wl-copy` / `xclip`.
- **Saved passwords and API keys** — remote-connection secrets and AI provider keys go to the
  Secret Service (GNOME Keyring, KWallet or another provider of `org.freedesktop.secrets`),
  never into `config.toml`. If no keyring is running or it stays locked, Soyuz says so in
  Settings and you can use an environment variable (`env:<VAR>`) for AI keys instead. The
  first access may show your keyring's unlock prompt.
- **Trash** — Delete moves items to the freedesktop trash (`~/.local/share/Trash`, or a
  `.Trash-<uid>` folder on other drives), so GNOME Files and Dolphin show and restore them too.
- **Hidden files** — dotfiles are hidden, and so are the names listed in a folder's `.hidden`
  file (turn that off with *General → Honour .hidden files*; the folder tree and pickers
  follow it too). Ctrl+H shows them.
- **File names on Windows drives** — on FAT, exFAT and NTFS drives (USB sticks, dual-boot
  partitions) new names, renames, links and rename-on-conflict follow the Windows rules
  (no `<>:"|?*`, no reserved names like `CON`), so files stay readable there.
- **Tags** — color tags are also written to the `user.xdg.tags` extended attribute that
  Dolphin and other KDE apps use, and tags set there show up in Soyuz (*Views → Tag names →
  Share tags with other apps*, on by default).
- **Sidebar** — Favorites include your GTK bookmarks (`~/.config/gtk-3.0/bookmarks`, shared
  with GNOME Files; *Views → Show GTK bookmarks*). Cloud lists GNOME Online Accounts drives
  (gvfs) and rclone mounts; an rclone mount appears only there, not again under Drives.
- **Thumbnails and PDF** — thumbnails other apps already made (`~/.cache/thumbnails`) are
  reused (*Appearance → Shared thumbnail cache*). PDF pages and thumbnails need
  `libpdfium.so`: next to the `soyuz` binary, in the config folder, in the folder set under
  *Preview & search*, or in a system library folder (`/usr/lib64`, `/usr/local/lib`, …).
- **Previous versions** — Properties → *Previous versions* and the context menu list earlier
  versions of a file or folder from Snapper (`<mount>/.snapshots`), ZFS
  (`<mount>/.zfs/snapshot`) and Timeshift snapshots. Snapper's `.snapshots` folder is usually
  readable only by root; Soyuz then tells you how many snapshots it could not read. Let your
  account read them with `sudo snapper -c root set-config ALLOW_USERS=<you> SYNC_ACL=yes`.
- **Inode flags and ACLs** — Properties → *Attributes* shows and edits the inode flags
  (append-only, no dump, no access time, no copy-on-write, compress, immutable — what `chattr`
  sets); immutable and append-only need root. The *Security* tab shows the POSIX ACL entries
  (read-only; change them with `setfacl`) and lets you change owner and group.
- **Many folders and the inotify limit** — open folders and indexed roots are watched with
  inotify. When the per-user limit (`fs.inotify.max_user_watches`) is used up, open folders are
  polled instead (*Behaviour → Polling interval*) and the search index re-checks the roots it
  can't watch (*Preview & search → Re-check unwatched folders*; the index status names them).
  Raise the limit with `echo fs.inotify.max_user_watches=524288 | sudo tee
  /etc/sysctl.d/90-inotify.conf && sudo sysctl --system`.
- **Window system** — *General → Linux desktop → Window system*: Automatic uses Wayland when
  available, else X11. Choosing X11 runs Soyuz through XWayland under a Wayland compositor.
  Takes effect after a restart.
- **Drag and drop** — works both ways under Wayland and X11. Files dragged in from other
  apps are always copied (the source app keeps its files). Dragging out offers the files as
  `text/uri-list` and the GNOME file list; `Ctrl` copies and `Shift` moves. After a move
  under Wayland Soyuz only refreshes the folder (the target app moves the files), and under
  X11 the sources go to the trash only when the target asks for it. Dragging out of Soyuz
  and back in again works once per drag under Wayland.
- **Windows shares (SMB)** — type `smb://server/share/folder` or `\\server\share\folder` in
  the path bar, or use *Connect to server* with SMB. `smb://server` lists the server's
  shares (turn off with *Remote → List shares for \\\\server*). Sign in as `user` or `DOMAIN\user`, or as a
  guest; saved passwords go to the Secret Service keyring. SMB 2 and 3 with signing and
  encryption are supported; Kerberos sign-in is not yet.
- **Not available on Linux** — the Mica / Acrylic backdrop, Snap Layouts, dialogs as separate
  windows, the NTFS change journal and Windows attributes are Windows features; their
  settings are hidden on Linux.

## macOS

**Status: implemented but untested.** The macOS integration below compiles, but it has
never been run on a Mac; expect rough edges and please report them. There are no macOS
packages yet (build from source).

- **Opening files** goes through LaunchServices, the same mechanism Finder uses (no `open`
  process). *Open with* lists every application LaunchServices knows for the file, the
  default first, with the name from the app's `Info.plist`. *Show in Finder* selects the
  item in a Finder window. Finder aliases open their target (folders open in Soyuz).
  *Open terminal here* uses Terminal.app, or the app you set in Settings (a bundle path or
  name such as `iTerm`).
- **Copy and paste between Soyuz and Finder** uses the general pasteboard: copied files go
  out as file URLs (any number of files) with their paths as text, and files copied in
  Finder paste into Soyuz. macOS has no standard "cut" on the pasteboard, so a cut made in
  Soyuz is a move only when pasted in Soyuz (and only until another app copies something);
  in Finder use ⌥⌘V to move.
- **AI provider keys** are stored in your login Keychain (service `soyuz`, account = the
  credential name). macOS may ask once whether Soyuz may use the item.
- **Appearance and language** follow System Settings: dark / light mode, the accent color
  (with "Multicolor" Soyuz keeps its own accent) and the first preferred language.
- **Fonts**: the font pickers list the fonts CoreText reports (system, `/Library/Fonts`,
  `~/Library/Fonts`).
- **Previous versions** shows files from Time Machine backups that are already mounted
  and readable: an attached HFS+ backup disk (`Backups.backupdb`) or APFS backups that
  macOS has mounted under `/Volumes/.timemachine` (for example while you browse them in
  Time Machine). Local snapshots that are not mounted need root and are not listed.
- **Terminal panel**: when the shell has no Soyuz integration, the panel still follows its
  working directory.
- **Not available on macOS**: *Run as administrator* (use `sudo` in a terminal), dragging
  files out of Soyuz to other apps, and the OS thumbnail cache (Soyuz uses its own).

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
| `folder_colors.toml` | folder colors and emblems |
| `themes/*.toml` | your color themes |
| `icons/<pack>/pack.toml` | your icon packs |
| `thumbs/` | thumbnail cache (safe to delete) |
| `sync_profiles.toml` | saved sync profiles and their schedules |
| `sync_state/*.json` | two-way sync state per folder pair (what both sides looked like after the last sync) |
| `sync_reports/<profile>/*.json` | run and dry-run reports |
| `smart_folders.toml` | smart folders (saved live queries) |
| `index/` | the background search index (machine-local; `%LOCALAPPDATA%\soyuz\data\index` when not portable) |
| `update-check.json` | when the weekly update check last ran |

## Troubleshooting

- **Blank or flickering window** — try the OpenGL renderer: build with `--features glow`
  and start with `soyuz --renderer glow`.
- **Copies seem slow on a network share** — lower *File operations → Parallel jobs* or set
  the queue mode to *Per device*.
- **A setting does not stick** — check for a parse error toast; the file may have been
  edited into an invalid state. *Settings → Config file → Reload from disk* re-reads it.
- **Logs** — run from a terminal with `RUST_LOG=debug soyuz` (release builds on Windows
  have no console; use the debug build).
