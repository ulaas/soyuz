# Phase S handover: single instance and less RAM (status 2026-10-25)

Phase S makes a second start of Soyuz hand the launch to the running window, gives every
in-process window the full feature set, and cuts the memory an idle and a busy window
use. Work runs on branch `phase-s` (not merged into `main` yet). All packages (S1, S2,
S3-0/S3a/S3b/S3c, S4/S5, S6a, S6b) are done; see [Open issues](#open-issues) for what is left.

## What is done

| Step | Commit | Contents |
|---|---|---|
| S1 | `64f55ae` | `[performance] gpu_memory = "low"` (default): egui-wgpu starts with `MemoryHints::MemoryUsage` instead of `Performance` (which reserves a 128 MB device + 64 MB host block up front, ~3 MB of it used by an idle window), and the shader-visible CBV/SRV/UAV heap holds 131,072 descriptors instead of 1M (~30 MB). `"performance"` keeps the old behaviour. Settings → Appearance (next start). |
| S2 | `1dde58b`, `804e5c6` | Single instance. A GUI start binds a per-user instance endpoint before eframe (and before the crash journal, FileManager1 threads and the config migration). If one is already bound, it sends the absolute paths, grants the running window the foreground and exits; the running window opens a new in-process window or tabs in the focused window (`[workspace] launch`). Launches within 300 ms merge into one window. The endpoint name hashes the config dir, the Windows session id, elevation and the unix display, so portable / test runs, other sessions and elevated starts never hand over. `--new-window`, `--safe-mode`, `--renderer`, `--dbus-file-manager`, `SOYUZ_SCRIPT` / `SOYUZ_WINDOW_POS` runs and `new_process` start their own process; a failed handoff starts a secondary process that leaves the session alone. Raising uses `SetForegroundWindow` on the viewport (winit's focus injects a synthetic Alt). The Windows pipe listener creates the next pipe instance before returning a connection, so no concurrent launcher can slip into the gap. |
| S3-0 / S3a | `b99db85` | Full-featured secondary windows: every window has its own preview, Quick Look, search bar and bottom / activity panel; searches and folder sizes keep running in secondary windows; FileManager1 and `soyuz open` land in the focused window; the main window's terminal no longer takes keys typed in another window; the player follows the window that has Quick Look open. |
| Fix | `794c47f` | Scripted / off-screen runs (`SOYUZ_WINDOW_POS`) neither restore nor save eframe's window geometry, so a reused `portable-data` no longer opens them at 0,0 on screen. |
| S4 / S5 | `cd3a642` | Fonts and thumbnails. `soyuz_fs::mapped`: read-only file mappings (`sys/mmap.rs`; Windows keeps `FILE_SHARE_READ \| FILE_SHARE_DELETE`, unix maps only files the user cannot write, so an in-place overwrite cannot SIGBUS us) used as egui `FontData` blobs for the fallback fonts; the Linux coverage scan pages in only the `cmap`. The terminal reports the non-Latin characters it draws (`TermResponse::new_chars`), so CJK / Hangul / emoji / RTL fonts load on demand instead of on every terminal show. Thumbnails count their real GPU cost (pixels rounded to 64 KiB plus 64 KiB per DX12 bind group, at most 16,384 textures = a quarter of the low-memory descriptor heap), Failed slots are evicted, and a timer frees textures not drawn for `thumb_idle_trim_secs`. |
| S6a | `b55d386` | Lazy i18n: the translation catalog parses only the active language (en-US again only on a miss) and drops the other bundle on a language switch. |
| S3b | `6f27353` | Terminal per window: sessions belong to their tab (`Owner::Tab`) or, in shared mode, their window (`Owner::Window`); focus, rename field and keyboard ownership are per window (`WindowUi.terminal`), and terminal actions run in the window they come from, in that window's folder. Sessions follow tabs that move or merge into another window and the promoted window when the main one closes. Closing a window with running shells asks first (`[terminal] close_window_with_sessions`). Also: `models::windows::raise` sends `WindowCmd::Raise` (no synthetic Alt), focus mode is saved per window, audio plays only in the player's window, devscript `win:<k> <step>` runs a step inside secondary window `k`. |
| S3c | `c365af7` | Shared panels follow their window (`models/owners.rs`): the editors, the assistant, the shelf and tool windows stay one instance each but have an owner window; invoking one from another window moves it there (`[workspace] panels_follow_window`). Closing the owner window hands its panels to the main window, unsaved documents included (quitting still asks); an editor or tool window takes keys only in its own window. The session remembers which secondary window held the editors. |
| S6b | `f372902` | Listing memory, process memory in About, exe-size experiment; details below. |

### S6b details

- **Listing entries** (`soyuz-appcore/src/state/pane.rs`, `soyuz-fs/src/list.rs`):
  - A finished listing gives back the doubling slack of `Pane::entries`
    (`shrink_to_fit` once the listing is no longer loading; `on_patch` reserves exactly
    what a patch adds, so watcher patches do not double and shrink the buffer).
  - The name index no longer keeps two extra copies of every name (`keys: Vec<Arc<str>>`
    plus a `HashMap<Arc<str>, usize>`): a private `NameIndex` (`hashbrown::HashTable<u32>`,
    the hashbrown version `indexmap` already pulls in) stores only entry indices and reads
    names from `entries`; a repeated name (search results) maps to its last entry, as the
    map did. API: `Pane::keys` is gone, `key_at` returns `Option<&str>`, new `index_of` and
    `key_of`.
  - Listing paths are built in an exact-size buffer (`join_exact`; `Path::join` grew its copy
    of the folder by doubling).
  - Probe: `cargo test -p soyuz-appcore --release --test listing_memory -- --ignored --nocapture`
    (20,000 files streamed into a `Pane`, private bytes before / after).
- **Process memory in About** (`soyuz_fs::process::memory()`): private bytes and working set
  on Windows (`K32GetProcessMemoryInfo`, kernel32, `sys/windows/process.rs`), `RssAnon` and
  `VmRSS` from `/proc/self/status` on Linux (`sys/linux/process.rs`), `None` on macOS. The
  About facts (GUI and terminal UI, and Copy info) gain a "Process" line; the reading is
  reused for 2 s (`models::about::PROCESS_REFRESH`) because the GUI dialog rebuilds its
  facts every frame, and the GUI dialog repaints every 2 s while open.
- **Exe size**: `opt-level = "s"` for the UI crates was measured and **not adopted** (see
  [Measurements](#measurements)); the release profile is unchanged.

## Measurements

Release builds on Windows, private bytes unless noted. In parentheses: the same state with
`gpu_memory = "performance"` (the pre-S1 behaviour).

| State | Private MB |
|---|---|
| Idle, one window | 140 (345) |
| Three windows | 165–183 (372) |
| Busy: 2,001-file folder, text preview, terminal open | 155–165 (358) |
| Each extra in-process window | ~12 |

| Item | Before | After |
|---|---|---|
| Handoff launcher (second start) | — | exits in ~115 ms |
| Showing the terminal (S4) | +42 MB | +9 MB (the terminal itself) |
| i18n catalog (S6a) | — | −3.9 MB heap |
| Listing, per entry (S6b, 20,000 files) | 842 B | 434 B |
| Listing, 20,000 entries (S6b) | 16.1 MiB | 8.3 MiB |
| `size_of::<EntryInfo>()` | 216 B | 216 B (unchanged) |

Exe size and the release frame-budget tour
(`cargo test -p soyuz-app --release --test frame_budget -- --ignored --nocapture`), one run each:

| Build | soyuz.exe | Tour worst | Tour p95 | 720p CPU / GPU worst |
|---|---|---|---|---|
| `opt-level = 3` (kept) | 90,882,048 B (86.7 MiB) | 3.48 ms | 1.49 ms | 0.91 / 1.12 ms |
| `opt-level = "s"` for 27 UI crates | 79,446,016 B (75.8 MiB, −12.6%) | 6.69 ms | 2.43 ms | 1.49 / 2.43 ms |

The `"s"` build covered soyuz-app, soyuz-tui, egui / epaint / eframe / egui-wgpu /
egui-winit / egui_extras / emath / ecolor / egui_glow, winit, accesskit (4 crates), naga,
rhai, ratatui (3 crates), ab_glyph, ttf-parser, skrifa, read-fonts, usvg and resvg. The p95
rose by 63% and the worst frame almost doubled (still far under the 16 ms budget), so it was
left out. Since soyuz-app is the binary, its opt-level probably also applies to the thin-LTO
step for every crate; a try without soyuz-app may keep most of the size win at less cost.

## Config keys added

| Key | Default | Package |
|---|---|---|
| `[performance] gpu_memory` | `"low"` | S1 — `low` (small up-front GPU reservations) \| `performance` (next start) |
| `[workspace] launch` | `"new_window"` | S2 — `new_window` \| `new_tab` (hand over to the running window) \| `new_process` (next start) |
| `[preview] thumb_idle_trim_secs` | `60` | S5 — free thumbnails not drawn for this long; 0 = keep until the budget is full |
| `[preview] thumb_cache_mib` | `64` (was `128`) | S5 — now a graphics memory budget |
| `[terminal] close_window_with_sessions` | `"ask"` | S3b — `ask` \| `kill`: closing a window whose terminals still run shells |
| `[workspace] panels_follow_window` | `true` | S3c — editors, assistant, shelf and tool windows open in the window they were invoked from; `false` keeps them in the main window |

Each key has a Settings row and `docs/CONFIG.md` / `docs/config.example.toml` entries. S6a and
S6b added no keys (the About memory line is read-only information).

## Open issues

- **Secondary windows and OS drops**: dropping files from another program onto a secondary
  window does nothing (`dnd::external_drops`, the portal drop and the shelf drop run in the
  main frame only); `ui.sidebar.portal.rect` is shared, so main-window portal hit-testing can
  use the last-drawn window's rect.
- **Focus mode vs. shared panels**: the status-bar shelf toggle is highlighted in every
  window while the shelf is open; tool-window positions reset when a tool window moves to
  another window (egui keeps area memory per viewport).
- **Typed text in a scripted terminal** is unverified: devscript `keys:` don't reach the
  terminal in any window, so the per-window terminal was checked by unit tests and a
  screenshot of a running session only.
- **Search bar in a secondary window** was not checked in a scripted run (now possible with
  `win:<k>`).
- **Linux**: the activation token a launcher gets (`XDG_ACTIVATION_TOKEN`) is not passed to
  the new window yet, so Wayland compositors may only flash it.
- **`EntryInfo` itself** is still 216 B. Storing only the name (deriving the path from the
  pane folder) touches ~2,400 `.path` uses across the crates; packing the times
  (`Option<SystemTime>` is 16 B each) and moving the rarely set fields (`alloc_size`,
  `nlink`, `inode`, `reparse_tag`, `dev`, `owner`, `group`) behind one box changes public
  field types used in a few hundred places. Estimated 216 → ~150 B per entry; a separate,
  wider change.
- The memory numbers are from Windows only, measured before S3b/S3c (which add per-window
  state only); Linux shows `RssAnon` / `VmRSS` in About but has
  not been measured state by state.
- Shift+arrow range selection builds the visible key list on each press (one small `Arc<str>`
  per visible entry; before S6b it cloned shared ones). Fine at 20,000 entries; watch it on
  very large folders.
- The exe-size and frame-time comparison is one run per build; repeat it before deciding on
  `opt-level = "s"` again.
