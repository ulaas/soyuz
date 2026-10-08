<p align="center">
  <img src="crates/soyuz-app/assets/icon.png" width="112" alt="Soyuz icon">
</p>

<h1 align="center">Soyuz</h1>

<p align="center">
  <b>A keyboard-first, non-blocking file explorer for Windows, Linux and macOS.</b><br>
  Written in Rust with <a href="https://github.com/emilk/egui">egui</a>. One TOML file to configure everything.
</p>

<p align="center">
  <a href="docs/USER_GUIDE.md">User guide</a> ·
  <a href="docs/config.example.toml">Config reference</a> ·
  <a href="#building">Build</a> ·
  <a href="#architecture">Architecture</a>
</p>

---

Soyuz is a file manager that never freezes. Every copy, move, delete, extract or
checksum is a background **job** with real progress, pause / resume / cancel, conflict
handling, history and undo. Listings stream in, folders with 100 000 entries scroll
smoothly, and a dead network share cannot hang the window.

> **No shell backend.** Soyuz does not use `shell32`, `IFileOperation`, `ShellExecute`
> or any COM shell API. All file-system work goes through the kernel directly
> (`kernel32` / `ntdll` / `advapi32` on Windows, `rustix` / `libc` on Unix), so behaviour
> is predictable, cancellable and identical across machines. CI fails the build if a
> shell API sneaks in.

## Features

**Browsing**
- Details, list, grid (thumbnails), gallery and Miller-column views, remembered per folder
- Tabs (pinning, reorder, reopen closed), dual pane with synchronized navigation
- Breadcrumb omnibar with path editing and completion, live filter, type-ahead
- Sidebar with favorites, drives with free-space bars, cloud folders, tags, trash and a folder tree
- Archives (zip, 7z, tar.\*) browse like folders

**File operations**
- Background jobs engine: per-device queueing, priorities, pause / resume / cancel, speed graph and ETA
- Conflict dialog with side-by-side comparison, "apply to all", keep both / replace / skip
- Undo / redo for copy, move, rename, trash, new items and metadata changes
- Trash support, secure wipe, batch rename with live preview, bulk attributes, links and junctions
- OS clipboard interop (copy in Soyuz, paste in your other file manager and back)

**Tools**
- Integrated terminal that follows the current folder (and can drive navigation)
- Preview pane and Quick Look (Space): text with syntax highlighting, images, hex, markdown
- Search by name and content, saved searches, folder sizes
- Disk-usage treemap, folder compare & sync, duplicate finder, split / join, checksums
- Git status column, color tags, drop shelf (persists across restarts), recent places

**Make it yours**
- Command palette (`Ctrl+K`) with every action and its shortcut; go-to-folder (`Ctrl+P`) with frecency
- Key presets (Explorer, Commander, Finder, Vim), chords and multi-key sequences, conflict detection
- Settings window plus a hot-reloaded `config.toml` — saving from the UI keeps your comments
- Custom commands with placeholders, bookmarks with hotkeys

**Accessibility**
- Screen-reader support through AccessKit (named buttons, tabs, list items, progress, dialogs, live toasts)
- High-contrast theme, reduced motion, touch density with large targets
- Interface zoom (`Ctrl+=` / `Ctrl+-` / `Ctrl+0`), visible focus rings, keyboard access everywhere

## Keyboard shortcuts

Default preset (*Explorer*). Every binding can be changed in **Settings → Keyboard** or
`[keys.bindings]`; the palette shows the active shortcut next to each command.

| Action | Shortcut | Action | Shortcut |
|---|---|---|---|
| Command palette | `Ctrl+K` / `Ctrl+Shift+P` | Go to folder | `Ctrl+P` |
| Back / Forward / Up | `Alt+←` / `Alt+→` / `Alt+↑` | Edit path | `Ctrl+L` |
| New tab / Close tab | `Ctrl+T` / `Ctrl+W` | Reopen closed tab | `Ctrl+Shift+T` |
| Next / previous tab | `Ctrl+Tab` / `Ctrl+Shift+Tab` | Dual pane | `F9` |
| Copy / Cut / Paste | `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Paste as link | `Ctrl+Shift+V` |
| Copy / move to other pane | `F5` / `F6` | Rename | `F2` |
| Batch rename | `Ctrl+Shift+R` | Duplicate | `Ctrl+D` |
| New folder / New file | `Ctrl+Shift+N` / `Ctrl+Alt+N` | Properties | `Alt+Enter` |
| Move to trash / Delete permanently | `Del` / `Shift+Del` | Undo / Redo | `Ctrl+Z` / `Ctrl+Y` |
| Filter / Search | `Ctrl+F` / `Ctrl+Shift+F`, `F3` | Quick Look | `Space` |
| Terminal | ``Ctrl+` `` | Activity panel | `Ctrl+Shift+J` |
| Preview pane | `Alt+P` | Toggle hidden files | `Ctrl+H` |
| Views: details / grid / list / Miller / gallery | `Ctrl+Shift+1…5` | Icon size | `Ctrl+Alt+=` / `Ctrl+Alt+-`, `Ctrl+wheel` |
| Interface zoom | `Ctrl+=` / `Ctrl+-` / `Ctrl+0` | Settings | `Ctrl+,` |
| Keyboard shortcuts | `Ctrl+/` | Quit | `Ctrl+Q` |

## Configuration

Everything lives in one file, `config.toml`:

| OS | Location |
|---|---|
| Windows | `%APPDATA%\soyuz\config\config.toml` |
| Linux | `~/.config/soyuz/config.toml` |
| macOS | `~/Library/Application Support/soyuz/config.toml` |
| Portable | `portable-data\config.toml` next to the executable when an empty file named `portable` sits beside it |

Edits made in a text editor apply within a second; a parse error is shown with its line
and column and the previous settings stay active. The settings window writes the file
back with [`toml_edit`](https://docs.rs/toml_edit), so your comments and ordering are
preserved. See [`docs/config.example.toml`](docs/config.example.toml) for every key and
the [user guide](docs/USER_GUIDE.md) for a tour.

## Building

Requirements: a stable Rust toolchain (1.95 or newer; `rust-toolchain.toml` selects stable).

```sh
cargo run --release -p soyuz-app          # build and start
cargo build --release -p soyuz-app        # binary: target/release/soyuz(.exe)
cargo test --workspace                    # all tests
```

**Windows** — MSVC toolchain. The Windows SDK resource compiler (`rc.exe`, installed with
the Visual Studio build tools) embeds the app icon into `soyuz.exe`; without it the build
still succeeds. `scripts/package.ps1` builds a release and zips it:

```powershell
./scripts/package.ps1                     # dist/soyuz-<version>-windows-x64.zip
```

**Linux** — install the windowing and font headers first:

```sh
sudo apt-get install libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev \
  libxrandr-dev libxi-dev libgl1-mesa-dev libfontconfig1-dev
cargo install cargo-deb && cargo deb -p soyuz-app   # optional .deb
```

**macOS** — Xcode command-line tools. `cargo install cargo-bundle && cargo bundle -p soyuz-app --release`
produces `Soyuz.app`.

**Cargo features** (`soyuz-app`):

| Feature | Default | Effect |
|---|---|---|
| `zstd` | yes | Native zstd (writing `.tar.zst`, zstd zip entries). Compiles C; use `--no-default-features` when cross-compiling without a C toolchain (zstd archives stay readable). |
| `glow` | no | OpenGL renderer as a fallback: `cargo run -p soyuz-app --features glow -- --renderer glow`. |

Debug hooks for automation: `SOYUZ_SCRIPT` (scripted actions and screenshots),
`SOYUZ_FRAME_LOG=1` (per-frame timing; an idle window renders no frames),
`SOYUZ_THROTTLE_MBPS` (slow copies to watch progress), `SOYUZ_OPEN_TOOL`.

## Architecture

A Cargo workspace of seven crates with an acyclic dependency graph. Only `soyuz-term` and
`soyuz-app` depend on egui; everything else is plain Rust and unit-tested in isolation.

| Crate | Role |
|---|---|
| [`soyuz-core`](crates/soyuz-core) | Shared vocabulary: entries, locations, errors, sorting, filtering, cancellation, the config schema, actions and key maps. No GUI, no I/O threads. |
| [`soyuz-fs`](crates/soyuz-fs) | Platform layer over raw OS calls: streaming listings, copy with progress / reflink / sparse, move, delete, trash, links, attributes, ACLs, volumes, watching, launching. |
| [`soyuz-jobs`](crates/soyuz-jobs) | The operation engine: queue and scheduler, planning, prompts (conflicts / errors), progress snapshots, history journal, undo / redo. |
| [`soyuz-archive`](crates/soyuz-archive) | Archive readers and writers (zip, 7z, tar + gz / bz2 / xz / zst) with an indexed virtual file system. |
| [`soyuz-search`](crates/soyuz-search) | Parallel name / content search, folder sizes, size trees for the treemap, duplicate detection, folder compare. |
| [`soyuz-term`](crates/soyuz-term) | Embedded terminal widget (alacritty_terminal + ConPTY / PTY) with selection, links and shell integration. |
| [`soyuz-app`](crates/soyuz-app) | The `soyuz` binary: egui shell, views, dialogs, palette, settings, services (lister pool, watcher, thumbnails, previews) and the design system. |

```
soyuz-app ──► soyuz-jobs ──► soyuz-fs ──────► soyuz-core
    │              └───────► soyuz-archive ─► soyuz-core
    ├──► soyuz-search ─────► soyuz-fs, soyuz-core
    └──► soyuz-term          (egui + alacritty_terminal only)
                                         arrows = "depends on"
```

Inside the app, widgets never mutate state directly: they push `Cmd`s, `ops` applies
them, and `actions::execute` is the single dispatcher behind key bindings, menus,
toolbar buttons and the palette. I/O always happens on worker threads that wake the UI
when results arrive, so the UI thread only renders — and renders nothing while idle.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Bundled fonts: Inter and JetBrains Mono
(SIL Open Font License, see `crates/soyuz-app/assets/fonts/`); icons: Phosphor (MIT).
Segoe UI and Consolas are used from the Windows installation and are not distributed.

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in Soyuz by you, as defined in the Apache-2.0 license, shall be dual licensed
as above, without any additional terms or conditions.
