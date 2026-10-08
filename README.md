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
  <a href="docs/GALLERY.md">Screenshots</a> ·
  <a href="docs/KEYBOARD.md">Keyboard</a> ·
  <a href="docs/CONFIG.md">Config reference</a> ·
  <a href="#install">Install</a> ·
  <a href="#building">Build</a> ·
  <a href="CHANGELOG.md">Changelog</a>
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

<p align="center">
  <img src="docs/screenshots/01-details-preview.png" width="860" alt="Soyuz: details view with the preview pane">
</p>

<p align="center">
  <img src="docs/screenshots/02-grid-thumbnails.png" width="280" alt="Grid view with thumbnails and folder mosaics">
  <img src="docs/screenshots/20-activity-copy.png" width="280" alt="Dual pane with a copy job in the activity panel">
  <img src="docs/screenshots/21-ai-plan.png" width="280" alt="The assistant proposing a plan">
</p>

More in the [screenshot gallery](docs/GALLERY.md).

> **Platforms.** Soyuz is built and verified on **Windows 10/11**. The code base keeps its
> Linux and macOS paths compiling where that is cheap, but those platforms have not been
> run or tested yet (owner decision; see [docs/ROADMAP.md](docs/ROADMAP.md)).

## Features

**Browsing**
- Details, list, grid (thumbnails, folder mosaics), gallery and Miller-column views, remembered per folder
- Tabs with groups, pinning and tear-off into windows; dual and quad (2×2) panes with synchronized browsing; saved workspaces
- Breadcrumb omnibar with path editing and completion, live filter, type-ahead, "Where was I?" history graph, Timeline of recent changes
- Sidebar with favorites, drives, cloud folders, tags, smart folders, network places, trash and a lazy folder tree
- Archives browse like folders: zip, 7z, tar.\*, RAR, ISO / UDF, CAB, deb / rpm (zip can be edited in place)
- Remote locations: SFTP, WebDAV, FTP / FTPS, S3-compatible storage and `\\server` share listing, with cached listings

**File operations**
- Background jobs engine: per-device queueing, priorities, pause / resume / cancel, speed graph and ETA, crash-safe journal with resume after a crash
- Conflict dialog with side-by-side comparison, "apply to all", keep both / replace / skip
- Undo / redo for copy, move, rename, trash, new items, archive edits and metadata changes
- Trash support, secure wipe, batch rename with live preview, bulk attributes, links and junctions, verified copies
- OS clipboard interop (copy in Soyuz, paste in your other file manager and back)

**Tools**
- Integrated terminal that follows the current folder (and can drive navigation)
- Preview pane and Quick Look (Space): code with syntax highlighting, images, Markdown, PDF, Office documents, EPUB, fonts, 3D models, audio waveform and playback, video posters, two-file diff
- Built-in text, hex and image editors
- Search by name and content, a background index (SQLite FTS5 + NTFS change journal) with a query language and live smart folders
- Disk-usage treemap, folder compare and two-way sync with scheduled profiles, previous versions from shadow copies, duplicate finder, split / join, checksums
- Git status column, color tags, folder colors, drop shelf, recent places
- Automation: sandboxed Rhai scripts, watcher-driven rules, and a `soyuz cp|mv|rm …` CLI that queues jobs in the running window

**AI (optional, off by default)**
- An assistant panel for Anthropic, OpenAI, Gemini or local OpenAI-compatible models (Ollama, LM Studio, llama.cpp, vLLM)
- Every change is a dry-run plan you approve; it then runs as a normal, undoable job. Per-folder permissions, redaction, budgets and an audit log
- An MCP server (`soyuz --mcp`) so other assistants can drive Soyuz under the same approval rules, and an MCP client for external tools
- API keys live in the Windows Credential Manager, never in `config.toml`

**Make it yours**
- Command palette (`Ctrl+K`) with every action and its shortcut; go-to-folder (`Ctrl+P`) with frecency
- Key presets (Explorer, Commander, Finder, Vim), chords and multi-key sequences, conflict detection ([cheat sheet](docs/KEYBOARD.md))
- Five color themes with dark and light variants, user theme files, the Windows accent color, Mica / Acrylic backdrops
- Settings window plus a hot-reloaded `config.toml` — saving from the UI keeps your comments ([reference](docs/CONFIG.md))
- English and Turkish UI with locale-aware dates, sizes and sorting; switches live

**Accessibility**
- Screen-reader support through AccessKit (named buttons, tabs, list items, progress, dialogs, live toasts) — see [docs/ACCESSIBILITY.md](docs/ACCESSIBILITY.md)
- High-contrast theme, reduced motion, touch density with large targets
- Interface zoom (`Ctrl+=` / `Ctrl+-` / `Ctrl+0`), visible focus rings, keyboard access everywhere

## Keyboard shortcuts

A few defaults of the *Explorer* preset. The full list for every preset is in
[docs/KEYBOARD.md](docs/KEYBOARD.md) (generated from the key maps), with a printable
[one-page version](docs/keyboard.html). Every binding can be changed in **Settings → Keyboard**.

| Action | Shortcut | Action | Shortcut |
|---|---|---|---|
| Command palette | `Ctrl+K` / `Ctrl+Shift+P` | Go to folder | `Ctrl+P` |
| Back / Forward / Up | `Alt+←` / `Alt+→` / `Alt+↑` | Edit path | `Ctrl+L` |
| New tab / Close tab | `Ctrl+T` / `Ctrl+W` | Dual pane / Quad | `F9` / `Shift+F9` |
| Copy / Cut / Paste | `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy / move to other pane | `F5` / `F6` |
| Rename / Batch rename | `F2` / `Ctrl+Shift+R` | Move to trash / Delete | `Del` / `Shift+Del` |
| Undo / Redo | `Ctrl+Z` / `Ctrl+Y` | Quick Look | `Space` |
| Terminal | ``Ctrl+` `` | Activity panel | `Ctrl+Shift+J` |
| AI assistant | `Ctrl+I` | Settings | `Ctrl+,` |

## Install

Download from the project's GitHub Releases (every file has a `.sha256`, and
`SHA256SUMS-windows.txt` lists them all):

| Package | How |
|---|---|
| `soyuz-<version>-windows-x64.msi` | Per-user install to `%LOCALAPPDATA%\Programs\Soyuz`, no admin rights. Optional features: "Open in Soyuz" folder context-menu entry and a PATH entry. `msiexec /i soyuz.msi ALLUSERS=1` installs per machine. |
| `soyuz-<version>-windows-x64.zip` | Unzip anywhere and run `soyuz.exe`. Settings go to `%APPDATA%\soyuz`. |
| `soyuz-<version>-windows-x64-portable.zip` | Same, plus an empty `portable` file next to the exe: settings stay in `portable-data\` beside it (USB sticks). |

PDF page rendering needs `pdfium.dll` next to `soyuz.exe` or in the config folder (Soyuz
never downloads it; without it PDFs preview as text). Video posters and thumbnails use an
`ffmpeg` found on `PATH`. Soyuz can register itself as the default file manager from
Settings → Advanced (with an exact restore), and checks for updates only when asked.

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
preserved. [docs/CONFIG.md](docs/CONFIG.md) lists every key with its type, default and
meaning (generated from the schema); [`docs/config.example.toml`](docs/config.example.toml) is an
annotated sample, and the [user guide](docs/USER_GUIDE.md) gives a tour.

## Building

Requirements: a stable Rust toolchain (1.95 or newer; `rust-toolchain.toml` selects stable).

```sh
cargo run --release -p soyuz-app          # build and start
cargo build --release -p soyuz-app        # binary: target/release/soyuz(.exe)
cargo test --workspace                    # all tests
```

**Windows** — MSVC toolchain. The Windows SDK resource compiler (`rc.exe`, installed with
the Visual Studio build tools) embeds the app icon into `soyuz.exe`; without it the build
still succeeds. `scripts/package.ps1` builds a release and packages it (each artifact gets
a `.sha256`, and `dist/SHA256SUMS-windows.txt` lists them all):

```powershell
./scripts/package.ps1                     # dist/soyuz-<version>-windows-x64.zip
./scripts/package.ps1 -Portable           # ...-portable.zip with the `portable` marker
./scripts/package.ps1 -Msi                # + dist/soyuz-<version>-windows-x64.msi (WiX v5)
./scripts/package.ps1 -DryRun -Msi        # print the plan, build nothing
./installer/msix/build-msix.ps1           # unsigned MSIX via makeappx (Windows SDK)
```

The MSI (`installer/windows/soyuz.wxs`) needs the WiX .NET tool pinned to v5
(`dotnet tool install --global wix --version 5.0.2`; v6+ requires accepting the OSMF EULA).
It installs per user to `%LOCALAPPDATA%\Programs\Soyuz` without admin rights
(`msiexec /i soyuz.msi ALLUSERS=1` installs per machine), adds a Start menu shortcut and an
uninstall entry, and has two optional features: an "Open in Soyuz" folder context-menu
entry and a PATH entry (`ADDLOCAL=Main,ContextMenu,AddToPath`).

**Linux** (untested; deferred) — install the windowing and font headers first:

```sh
sudo apt-get install libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev \
  libxrandr-dev libxi-dev libgl1-mesa-dev libfontconfig1-dev
cargo install cargo-deb && cargo deb -p soyuz-app   # optional .deb
```

**macOS** (untested; deferred) — Xcode command-line tools. `cargo install cargo-bundle && cargo bundle -p soyuz-app --release`
produces `Soyuz.app`.

**Cargo features** (`soyuz-app`):

| Feature | Default | Effect |
|---|---|---|
| `zstd` | yes | Native zstd (writing `.tar.zst`, zstd zip entries). Compiles C; use `--no-default-features` when cross-compiling without a C toolchain (zstd archives stay readable). |
| `pdf` | yes | PDF page rendering through pdfium, loaded at runtime from `pdfium.dll` (text-only fallback without it). |
| `glow` | no | OpenGL renderer as a fallback: `cargo run -p soyuz-app --features glow -- --renderer glow`. |

`soyuz-archive` has a `rar` feature (default on) for the pure-Rust RAR reader.

Debug hooks for automation: `SOYUZ_SCRIPT` (scripted actions and screenshots),
`SOYUZ_FRAME_LOG=1` (per-frame timing; an idle window renders no frames),
`SOYUZ_THROTTLE_MBPS` (slow copies to watch progress), `SOYUZ_OPEN_TOOL`, `SOYUZ_AI_FAKE`
(scripted assistant). `scripts/screenshots.ps1` regenerates the gallery from a generated demo
tree (`scripts/demo-fixture.ps1`), and `scripts/changelog.ps1` writes `CHANGELOG.md`.

## Architecture

A Cargo workspace of twelve crates with an acyclic dependency graph. Only `soyuz-term` and
`soyuz-app` depend on egui; everything else is plain Rust and unit-tested in isolation.

| Crate | Role |
|---|---|
| [`soyuz-core`](crates/soyuz-core) | Shared vocabulary: entries, locations, errors, sorting, filtering, locale formatting, cancellation, the config schema, actions and key maps. No GUI, no I/O threads. |
| [`soyuz-fs`](crates/soyuz-fs) | Platform layer over raw OS calls: streaming listings, copy with progress / reflink / sparse, move, delete, trash, links, attributes, ACLs, volumes, watching, launching, shadow copies, USN journal. |
| [`soyuz-jobs`](crates/soyuz-jobs) | The operation engine: queue and scheduler, planning, prompts (conflicts / errors), progress snapshots, crash journal, history, undo / redo. |
| [`soyuz-archive`](crates/soyuz-archive) | Archive readers and writers (zip, 7z, tar + gz / bz2 / xz / zst; RAR, ISO / UDF, CAB, deb / rpm read-only) with an indexed virtual file system. |
| [`soyuz-search`](crates/soyuz-search) | Parallel name / content search, folder sizes, size trees for the treemap, duplicate detection, folder compare. |
| [`soyuz-index`](crates/soyuz-index) | Background index (SQLite + FTS5), query language, smart-folder queries. |
| [`soyuz-vfs`](crates/soyuz-vfs) | Remote providers (SFTP, WebDAV, FTP / FTPS, S3) and resumable cross-provider transfers. |
| [`soyuz-term`](crates/soyuz-term) | Embedded terminal widget (alacritty_terminal + ConPTY / PTY) with selection, links and shell integration. |
| [`soyuz-ai`](crates/soyuz-ai) | AI providers, credential store, tool catalog with risk classes and dry-run plans, audit log. |
| [`soyuz-mcp`](crates/soyuz-mcp) | MCP server (stdio, named-pipe attach, localhost HTTP) with a client trust model, and an MCP client. |
| [`soyuz-script`](crates/soyuz-script) | Sandboxed Rhai scripting for actions, menus, columns, previewers and automation rules. |
| [`soyuz-app`](crates/soyuz-app) | The `soyuz` binary: egui shell, views, dialogs, palette, settings, services (listers, watcher, thumbnails, previews), the assistant and the design system. |

## Documentation

| Document | What it covers |
|---|---|
| [User guide](docs/USER_GUIDE.md) | A tour of the app |
| [Screenshot gallery](docs/GALLERY.md) | Every major screen |
| [Keyboard](docs/KEYBOARD.md) / [printable](docs/keyboard.html) | Default shortcuts of all presets (generated) |
| [Configuration](docs/CONFIG.md) | Every `config.toml` key (generated); [annotated sample](docs/config.example.toml) |
| [Accessibility](docs/ACCESSIBILITY.md) | Screen readers, keyboard-only use, contrast |
| [Security](SECURITY.md) | Threat model and reporting |
| [Roadmap](docs/ROADMAP.md) | Status of every roadmap item and what is next |
| [Changelog](CHANGELOG.md) | Changes by milestone |
| [Plan](docs/PLAN.md) / [integration notes](docs/INTEGRATION_NOTES.md) | Architecture and per-crate notes for contributors |

## Inside the app

Widgets never mutate state directly: they push `Cmd`s, `ops` applies
them, and `actions::execute` is the single dispatcher behind key bindings, menus,
toolbar buttons and the palette. I/O always happens on worker threads that wake the UI
when results arrive, so the UI thread only renders — and renders nothing while idle.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option. Bundled fonts: Inter, IBM Plex Sans / Mono, Noto Sans, Cascadia Mono, JetBrains Mono and
Source Code Pro (SIL Open Font License, see `crates/soyuz-app/assets/fonts/`); icons:
Phosphor (MIT). Segoe UI, Consolas and other system fonts can be chosen but are read from
the Windows installation and never distributed.

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in Soyuz by you, as defined in the Apache-2.0 license, shall be dual licensed
as above, without any additional terms or conditions.
