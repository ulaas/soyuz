# CLAUDE.md — Soyuz

Soyuz is a cross-platform (Windows / Linux / macOS) file explorer written in Rust with egui/eframe 0.36. It is keyboard-first and never blocks the UI. Every file operation is a background job that you can pause, resume, cancel and undo. It does not use the Windows shell backend.

## Rules (set by the project owner — follow exactly)

### Hard constraints
- **No Windows shell backend.** Never use shell32, `Win32_UI_Shell`, `IFileOperation`, `SHFileOperation`, `ShellExecute(Ex)`, Shell COM, `url.dll`/`rundll32 …FileProtocolHandler`, or crates that wrap them (e.g. `trash`, `rfd`, `open` with `shellexecute-on-windows`).
  - Implement every operation at the kernel/filesystem level: raw `windows-sys` (kernel32/ntdll/advapi32/user32) on Windows, and `rustix`/`libc` syscalls on unix.
  - CI greps for violations (`.github/workflows/ci.yml`, job `no-shell32`). The grep is plain text and case-insensitive over all of `crates/`, comments included: never write a banned name (`shell32`, `SHGetFileInfo`, `ShellExecute`, `IShellItem`, `CoInitialize`, …) even to say it is *not* used. Describe it instead ("no shell file-info calls"). Run the job's grep locally before committing.
- **Fully configurable.** New behaviour gets a config key in `soyuz-core` (`config/schema.rs` + defaults + `docs/config.example.toml`). It must be editable in the Settings UI, and it must actually be read by the app; no dead settings.
- **AI features (roadmap Phase A).** AI models and MCP clients never get raw fs access.
  - Every change goes through the jobs engine as a dry-run plan, then approval, then a job, so it is visible, follows conflict policies and can be undone.
  - File contents and names are untrusted data.
  - API keys live in the OS credential store, never in `config.toml`.
  - Cloud content sharing is opt-in per folder scope.
- **Beautiful, stable, easy to use.** Use the design tokens (`soyuz-app/src/theme/tokens.rs`, PLAN §1.2). Don't use stock egui looks. Never block the UI thread on I/O. Every destructive action goes to the trash first or can be undone.

### Git permissions
- Local git, read **and write** (add, commit, branch, merge, worktree), is **pre-authorized** for this project. Commit after each meaningful step; you don't need to ask first.
- **Never** run any command that touches a remote (push, pull, fetch, clone into here, `gh` writes).
- **Never** add `Co-Authored-By: Claude` or any other self-attribution to commit messages or PRs.
- Commit message style: `M<n>: …`, `feat(app): …`, `fix(core): …`, `docs: …`.

### Working style the owner asked for
- Plan big work with agents (Fable when credits allow, otherwise the default model), have the plan reviewed, then implement.
- Parallelize with worktree agents, **one crate or one module set per agent**. The orchestrator (main session) commits and merges each worktree and resolves conflicts.
  - Subagents must be told: "no git write commands, the orchestrator commits". Otherwise they refuse because of the global CLAUDE.md.
  - Tell agents which files they own, and that edits to shared central files (`actions.rs`, `ops.rs`, `app.rs`, `cmd.rs`, `ui/panels/mod.rs`, `shell.rs`) must be minimal and additive.
- Great ideas are welcome. Propose them; "no idea is bad".

## Platform scope (owner decision, 2026-10-11)
**Windows and Linux are both first-class.** The owner now also develops on a Linux desktop (Fedora, niri Wayland compositor, noctalia shell), and Linux tests are a CI gate. Linux desktop integration is native: UDisks2, Secret Service, FileManager1, the XDG portal over `zbus`; Wayland and X11 clipboard and drag and drop over `wayland-client` / `x11rb`; and the XDG mime / desktop-entry / mimeapps specs implemented in `soyuz-fs` — no `xdg-open` / `gio` spawns. **macOS is still deferred**: keep it compiling, no macOS-specific work.
- Windows regression gate from Linux (no MSVC headers here, so crates with C deps can't be checked): `CC_x86_64_pc_windows_msvc=clang-cl CARGO_FEATURE_PURE=1 cargo clippy -p soyuz-core -p soyuz-fs -p soyuz-archive -p soyuz-search -p soyuz-term --lib --target x86_64-pc-windows-msvc -- -D warnings`.
- On Linux, scripted app runs can't be placed off-screen (Wayland): keep them short and end with `quit`. Tests that need the real desktop (D-Bus, keyring, clipboard, X11 / Wayland) are gated by `SOYUZ_DBUS_TESTS`, `SOYUZ_CLIPBOARD_TESTS`, `SOYUZ_X11_TESTS`, `SOYUZ_WAYLAND_TESTS` and `SOYUZ_TRASH_TESTS`. Never mount, unlock or unmount the owner's real devices in tests.
- `unsafe` for Linux FFI (ioprio, Wayland foreign display for drag and drop) lives in `soyuz-fs/src/sys/linux/`.

## Quality gates (all must pass before committing)
```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```
- Linux cross-check from Windows for crates without C deps of their own (e.g. `soyuz-fs`): `CARGO_FEATURE_DLOPEN=1 CARGO_FEATURE_PURE=1 cargo clippy -p soyuz-fs --lib --target x86_64-unknown-linux-gnu -- -D warnings` (`DLOPEN` keeps `wayland-sys` from asking pkg-config); crates pulling `ring` need a Linux C compiler.
- (Deferred) Linux/macOS cross-checks: `cargo clippy -p soyuz-app --no-default-features --target x86_64-unknown-linux-gnu` / `aarch64-apple-darwin` with `CARGO_FEATURE_PURE=1`.
- Manual app testing: put an empty file named `portable` next to the built exe, so config goes to `portable-data/` instead of `%APPDATA%\soyuz`. Delete both afterwards.
  - **Never send real mouse/keyboard input** (SetCursorPos, mouse_event, SendInput, keybd_event) — the owner uses this machine (often via a fullscreen Remote Desktop window) and synthetic input lands in their session. Drive the app with `SOYUZ_SCRIPT` only. Never run installer UIs interactively (use `msiexec /qn` with per-user test installs).
  - Screenshot only **your own** process window (PowerShell System.Drawing, by `MainWindowHandle`). Other Soyuz instances may be open.
  - Debug hooks: `SOYUZ_SCRIPT` (scripted actions/screenshots, see `devscript.rs`; `until:<file>` holds a state for external drivers), `SOYUZ_WINDOW_POS=x,y` (open the window there, unactivated; `-4000,-4000` = off-screen captures), `SOYUZ_FRAME_LOG=1`, `SOYUZ_OPEN_TOOL` (`share=<path>|…` opens Quick share), `SOYUZ_SHARE_BIND=127.0.0.1` (Quick share listens on loopback only: no firewall prompt in scripted runs). Accessibility audit: `scripts/uia-audit.ps1` (release build). Frame-budget tour: `cargo test -p soyuz-app --release --test frame_budget -- --ignored`.
- Never touch the user's real folders in tests. Use `tempfile`. Real Recycle Bin tests only run with `SOYUZ_TRASH_TESTS=1`.
- `unsafe` is only allowed in `soyuz-fs/src/sys/*` (incl. window-chrome FFI in `sys/windows/chrome.rs`), the Windows clipboard module, `soyuz-ai/src/credentials/windows.rs` (raw advapi32 credential store), `soyuz-mcp/src/pipe/windows.rs` (MCP named pipe, kernel32/advapi32), `soyuz-term/src/tty_tee.rs` (PTY wrapper forwarding alacritty's `EventedReadWrite::register`, for OSC 7 cwd tracking; owner decision 2026-10-11), and `soyuz-ai/src/credentials/macos.rs` (Security.framework Keychain). Other macOS FFI goes in `soyuz-fs/src/sys/macos/`.

## Layout
| Crate | Role |
|---|---|
| `soyuz-core` | Shared types, config schema/defaults/keymap/actions, `Location`, naming/sort/filter. No GUI deps. |
| `soyuz-fs` | Platform layer: listing, metadata, copy engine with OS fast paths, move/delete/links/times/attrs, volumes, watcher, launch (registry association chain), trash by hand per OS, checksums. |
| `soyuz-jobs` | Background operation engine: lanes, priorities, pause/cancel, conflict/error prompts, history with identity-stamped undo. |
| `soyuz-archive` | zip / tar.* / 7z: browse (dir index), extract, create, passwords, zip-slip safety. |
| `soyuz-search` | Streaming search, folder sizes, size tree, duplicates, folder compare. |
| `soyuz-index` | Background file index (SQLite + FTS5), query language, NTFS USN journal parsing, smart-folder queries. |
| `soyuz-term` | Embedded terminal (alacritty_terminal 0.26 plus our own egui renderer). |
| `soyuz-vfs` | Remote locations: `Provider` trait, SFTP / WebDAV / FTP(S) / S3 providers, resumable cross-provider transfers. |
| `soyuz-ai` | AI provider layer (Anthropic, OpenAI, OpenAI-compatible local, Gemini, custom), credential store, tool catalog with risk classes and dry-run plans, audit log. |
| `soyuz-script` | Sandboxed Rhai scripting (actions, menu entries, columns, previewers) for automation. |
| `soyuz-mcp` | MCP server (stdio / named-pipe attach / Streamable HTTP, client trust model, audit) and MCP client for external servers. |
| `soyuz-app` | eframe app (binary `soyuz`): UI, services, dialogs, palette, settings. |

Read first: `docs/PLAN.md` (architecture, APIs, design tokens), `docs/INTEGRATION_NOTES.md` (how to drive each crate, plus per-milestone notes), `docs/REVIEW-1.md`/`REVIEW-2.md` (audits and their status), `docs/ROADMAP.md` (status per item and future work), `docs/USER_GUIDE.md`. Generated references: `docs/CONFIG.md`, `docs/KEYBOARD.md` (+ `keyboard.html`), regenerated with `SOYUZ_BLESS_DOCS=1 cargo test -p soyuz-app --test docs_sync` (the test fails when they are stale). Screenshots: `scripts/screenshots.ps1` (demo tree from `scripts/demo-fixture.ps1`); changelog: `scripts/changelog.ps1`.

## Current status (2026-10-09)
- **Done:** milestones M0–M13 (PLAN §5), both review audits, and the roadmap phases R, B, F, A and D on Windows; see the status table at the top of `docs/ROADMAP.md`. R5: startup opens exactly one window, the real eframe / wgpu window, with its first frame ~355 ms after `main` (the startup placeholder window and the CPU-drawn first frames were removed by owner decision 2026-10-09: they looked like the app launching twice). B5: Inter Variable with optical sizes is an option, not the default (static Inter renders crisper). Deferred (owner decision 2026-10-09): R1 (Linux / macOS), right-to-left layout / bidi, the human NVDA / Narrator run (the automated UIA audit `scripts/uia-audit.ps1` is clean), and a trusted code-signing certificate (pipeline validated with a temporary one).
- **Fonts:** embedded OFL screen fonts are the defaults (Inter + Inter SemiBold for the interface; Inter Variable with optical-size instances selectable, Cascadia Mono for terminal / code / hex; also IBM Plex Sans, Noto Sans, JetBrains Mono, IBM Plex Mono, Source Code Pro), with crisp hinting and text gamma 0.45 dark / 0.80 light. System fonts (Segoe UI, Consolas) stay selectable. Sheets: `docs/font-comparison/` (`scripts/font-comparison.ps1`).
- **Verified:** mainly on Windows. Unsigned Windows, Linux x64 and macOS arm64 builds come out of the GitHub release workflow, and the owner has tested each OS at least once; Linux and macOS have had far less testing than Windows.
- **Known gaps:**
  - RSA SSH keys are disabled (the `rsa` crate fails `cargo deny` with advisory RUSTSEC-2023-0071); ed25519 / ECDSA keys work.
  - Push watching for remotes is missing (polling only). Connect to server finds servers announced over mDNS (one-shot queries on every IPv4 interface, `soyuz_vfs::discover`); tested against a fake responder and a real printer, not yet against a real SFTP / WebDAV / SMB server. WebDAV signs in with Basic or Digest (RFC 7616, MD5 / SHA-256).
  - Windows `thumbcache_*.db` is read (read-only, cache ids derived without the shell); Vista / 7 formats, ReFS and network volumes fall back to Soyuz's own disk cache.
  - Previous versions (Volume Shadow Copies) are untested on real shadow copies. The NTFS journal catch-up works without admin rights on Windows 11 (`FSCTL_READ_UNPRIVILEGED_USN_JOURNAL`, tested); the MFT import still needs admin.
  - No screen-reader certification run yet (NVDA / Narrator). The keyboard-only GUI test has a Windows CI job (`gui-keyboard`, WARP adapter) that has not run on a hosted runner yet; it skips with a warning when no window can start. The UIA audit (`scripts/uia-audit.ps1`, `docs/ACCESSIBILITY_AUDIT.md`) covers six states, not the whole app. No right-to-left layout (deferred).
  - The MSIX is unsigned; Authenticode signing of the exe / MSI needs a certificate (pipeline validated with a temporary self-signed one, `docs/RELEASING.md`). Linux and macOS ship as unsigned tarballs only (no .deb / .rpm / AppImage / .dmg, no notarization yet).
  - RAR volume sets are unsupported (single RAR archives read via pure-Rust `rars`). PKWARE spanned zips (`.z01` … `.zip`) are read (read-only), like byte-split `.001` sets.
  - PDF page rendering and PDF thumbnails need `pdfium.dll` next to the exe or in the config dir (text-only preview and the PDF icon otherwise).
  - Hand-built `$Recycle.Bin` entries not yet checked against Explorer's Recycle Bin window.
  - Two single checks on a click still run on the UI thread: Create archive's "name taken" check and Open with's typed program path (debug builds log every `soyuz_fs` call made on the UI thread). Extract to folder picks its free name on a worker.
- **License:** MIT OR Apache-2.0.
