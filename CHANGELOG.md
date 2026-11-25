# Changelog

All notable changes to Soyuz. Grouped by the commit prefix: M = the
original milestones, R = rigidity, B = looks, F = features, A = AI,
D = distribution. Generated from `git log` by `scripts/changelog.ps1` (or `git cliff`
with `cliff.toml`); commit hashes refer to the main branch.

## 0.8.7 (2026-11-25)

### Other features

- feat(core): PDF viewer config ([pdf], [preview] pdf_engine, [open] pdf_in_viewer), Pdf key context and actions (8abb37a)
- feat(appcore): hayro PDF engine (services::pdf): rendering, regions, text with positions, outline/links, system-font fallback, guarded sessions; pdfium moved behind it (9d0bf71)
- feat(app): Settings rows for the PDF engine, the PDF viewer ([pdf]) and opening PDFs in the viewer (9b5165d)
- feat(appcore): PDF viewer model (editors::pdf): continuous layout, zoom, rotation, find, selection, links, F4 and [open] pdf_in_viewer, Pdf key routing (a198cf2)
- feat(appcore): previews, Quick Look, thumbnails and document text render through hayro; pdfium only when chosen ([preview] pdf_engine) (309efa6)
- feat(tui): PDF tab: page text with find, go to page, paging and copy; text-only sessions render no pictures (c61c037)
- feat(app): PDF viewer tab: toolbar, find bar, thumbnails/outline sidebar, sharp zoomed canvas, selection, links, status bar; Open in viewer from the preview (12f9bb0)
- feat(app): [pdf] max_file_mib: the viewer's own file size limit (512 MiB), separate from the preview's document limit (ee1431a)

### Fixes and security

- fix(deps): url 2.5.9 (RUSTSEC-2026-0592, Url::origin stack overflow); deny: ignore chrono unmaintained notice with a reason (bfdddb6)

### Documentation

- docs: plan for the hayro PDF engine and embedded PDF viewer (0165e81)
- docs: third-party notices for the PDF engine's embedded fonts, CMaps and ICC profiles (be13948)
- docs: PDF viewer and the built-in PDF engine in the user guide, README and known gaps (2e3172f)
- docs: keep the PDF viewer plan local (untracked, like the other planning docs) (abbed05)
- docs: changelog regenerated (0.3.0 to 0.8.6 and the unreleased PDF viewer) (d0d7395)

### Other

- release: 0.8.7 (a built-in PDF viewer; PDF previews, thumbnails and search no longer need pdfium) (d42f537)

## 0.8.6 (2026-11-21)

### Other features

- feat(app): system monitor shows every GPU, side by side, with per-GPU process columns (5196a1f)

### Other

- release: 0.8.6 (the system monitor shows every GPU, side by side, with per-GPU process columns) (da87039)

## 0.8.5 (2026-11-17)

### Fixes and security

- fix(fs): a folder with watched subfolders can be renamed, trashed and removed (b1b8f3e)
- fix(fs): tests use a sandbox trash instead of the user's trash (09f5db2)
- fix(fs): items outside the test trash sandbox still reach the platform backend (cb3fb23)
- fix(app): opening a window while the main window is minimized no longer crashes (210aee9)

### Other

- release: 0.8.5 (opening a new window while the main window is minimized no longer crashes) (d5efec8)

## 0.8.4 (2026-11-16)

### Other features

- feat(packaging): Soyuz.app for Apple silicon in a drag-to-install disk image (scripts/package-macos.sh) (715e26b)
- feat(appcore): Update and restart replaces a Developer ID signed Soyuz.app from the release disk image (d7c2136)

### Fixes and security

- fix(fs): the macOS trash shows the items Soyuz moved there when ~/.Trash cannot be listed (4318a5c)
- fix(mcp): the unix socket waker unblocks a waiting accept on macOS (87c1b6a)
- fix(app): ignore the -psn_ process serial number older macOS versions pass to apps started from Finder (914875e)
- fix(appcore): a macOS app bundle is offered the disk image as its update, never the tarball of bare executables (af558ba)
- fix(ci): the package tests follow the macOS job to package-macos.sh, Info.plist stays LF, gitleaks skips the secret names in RELEASING.md (dc192c4)
- fix(appcore): the app bundle is exchanged atomically (RENAME_SWAP) and the relaunch keeps HOME (93dd270)
- fix(appcore): the restart helper waits at most 10 minutes for the old process (7045a2a)

### Documentation

- docs: macOS app bundle and disk image packaging (a7c406d)
- docs: the macOS disk image is signed with a Developer ID and notarized (3403a70)
- docs: in-place macOS app update and its fallbacks (d2242b5)

### Other

- ci: the unsigned macOS release job also builds the disk image (fc17664)
- ci: the macOS release job signs Soyuz.app with the Developer ID, notarizes and staples the disk image when the secrets exist (28fd0ca)
- release: 0.8.4 (Soyuz for the Mac: a signed and notarized disk image, and Update and restart replaces Soyuz.app in place) (5482d6d)

## 0.8.3 (2026-11-15)

### Fixes and security

- fix(app): the part of a window that a resize uncovers shows the app's background instead of white on Windows (38774f2)

### Other

- release: 0.8.3 (on Windows, the part of a window that a resize uncovers shows the app's background instead of white) (bd4b802)

## 0.8.2 (2026-11-12)

### Other

- release: 0.8.2 (the release workflow publishes the release instead of leaving a draft; a watcher test no longer depends on timing; apt retries in CI) (306d793)

## 0.8.1 (2026-11-11)

### Other

- release: 0.8.1 (SQLite and rustix updated for RUSTSEC-2026-0459 and RUSTSEC-2026-0486; first release 0.8.0 can install by itself) (582e6a5)

## 0.8.0 (2026-11-11)

### Other features

- feat(app): the update check runs once a day and "Update and restart" installs a release in one click (db7ff99)

### Fixes and security

- fix(app): drag-out no longer offers the paths as text, so Word inserts a dragged picture instead of its path (cf8f536)
- fix(app): copied files no longer carry their paths as text on Windows, so Word pastes a copied picture instead of its path (7b4b2ec)

### Other

- ci: package manifests for winget, Scoop, Homebrew and AUR; social preview image (11829b7)
- ci: package manifest comments no longer name the placeholders (they were rendered too) (7734028)
- release: 0.8.0 (a daily update check, on by default, and "Update and restart" installs a release in one click) (b1134e5)

## 0.7.5 (2026-11-08)

### Other features

- feat(app): the command palette starts installed applications (ae59cbe)
- feat(app): the palette's app launcher lists Store apps too (49de8e6)
- feat(app): installed apps show their own icons in the command palette (8cb22fe)

### Other

- release: 0.7.5 (the command palette starts installed applications, Store apps included, shown with their own icons) (6b77887)

## 0.7.4 (2026-11-08)

### Other features

- feat(tui): the system monitor draws like btop (db5ad32)
- feat(app): the smart folders section shows while the index is off, with a hint (0fb4d6d)

### Fixes and security

- fix(app): the GPU memory chart ends above the shared-memory row instead of under it (992eae5)
- fix(sysmon): live graphs draw at once after a restart or a stopped recording (7ae2610)
- fix(app): chart hover cards are no longer cut off by small charts (2d87051)

### Other

- release: 0.7.4 (the terminal system monitor draws like btop; live graphs draw at once after a restart; chart hover cards are no longer cut off; the smart folders section shows while the index is off) (494c612)

## 0.7.3 (2026-11-07)

### Fixes and security

- fix(app): the system monitor gets readings in every window, not only the main one (ed68455)

### Other

- release: 0.7.3 (the system monitor gets readings in every window, not only the main one) (f3f1602)

## 0.7.2 (2026-11-07)

### Other features

- feat(app): operations of one kind share a toast that counts them and undoes them all (87d7857)

### Other

- release: 0.7.2 (operations of one kind share a notification that counts them and undoes them all; notifications behind the stack no longer queue up) (a904889)

## 0.7.1 (2026-11-07)

### Fixes and security

- fix(app): a drag to another Soyuz window no longer trashes the source; handed-off windows get the keyboard (da74463)
- fix(app): a menu in one window no longer kills the keyboard of newer windows (d2e1b72)

### Other

- test(app): pin per-window menu state with two viewports, not frame timing (3f1a33b)
- release: 0.7.1 (a drag to another window no longer trashes the original; a menu in one window no longer stops the shortcuts of newer windows; launched windows get the keyboard; a dialog waiting in another window comes forward) (14313f4)

## 0.7.0 (2026-11-06)

### Other features

- feat(core): a made-up machine for the monitor's website screenshots (2519fb6)
- feat(tools): font lab, every font and rendering technique on Soyuz's stack with an id (a2b068a)
- feat(app): LCD subpixel text by default, the owner's pick from the font lab (IN13-LCD5-NM-B1-G80-DS) (34c3d17)

### Documentation

- docs(site): system monitor, media player and tools, and what shipped since 0.4 (41a6eb9)

### Other

- release: 0.7.0 (LCD subpixel text by default with normal hinting and a softer text colour in the Orbit themes; Text smoothing setting; saved configs on the old defaults move to the new ones) (bf3acd1)

## 0.6.0 (2026-11-06)

### Other features

- feat(fs): raw system monitor types and per-OS stubs; [monitor] config schema (2009aa3)
- feat(core): monitor: location, OpenMonitor action, review changes to the process records (8602bf9)
- feat(fs): Windows system monitor readers (79b6527)
- feat(sysmon): rates, history, snapshots, compare, filter, process table, alerts and the sampler hub (6f8e030)
- feat(core): system monitor model: per-pane view state, commands, labels, i18n, settings section (32e6032)
- feat(fs): Windows readers: review pass (processor groups, one handle per action, shared strings, GPU cadence) (1ebba0d)
- feat(fs): Linux and macOS system monitor readers (344e190)
- feat(core): monitor model compiles against the readers (c8d2657)
- feat(fs): Linux and macOS readers: review pass (pidfd control, shared strings, GPU cadence, stacked disks) (b5995e4)
- feat(tui): system monitor pages (e751401)
- feat(app): system monitor pages (649ab23)
- feat(app): system monitor button beside the settings gear (09dfd03)
- feat(monitor): history records only when asked, with start, pause and stop (45b206b)

### Fixes and security

- fix(app): monitor status line, no file controls on a monitor pane; compare uses the table's CPU scale (e10064d)
- fix(app): a history cursor with no recording nearby shows no value (window and terminal UI) (4a1d161)
- fix(app): review findings in the monitor (ffc00d3)
- fix(fs): End process also reaches a program whose only windows are owned ones (36c6fc1)
- fix(fs): macOS lists exited processes that wait for their parent (0e49b29)
- fix(sysmon): a process whose figures the OS hides shows dashes, not zeros (0d4eb7e)
- fix(fs): review of the monitor's readers (52c40c8)
- fix(app): quieter scrollbars in every theme (7211e4c)
- fix(app): filter and search fields center their text (8f7aa0d)
- fix(monitor): the process list no longer scrolls by itself (63ce214)

### Documentation

- docs: system monitor guide, config example; smoke test requires real readers (393d1e8)
- docs: system monitor keys and known gaps (b01f6b6)

### Other

- test(sysmon): process GPU test states its setting (78dcc96)
- test(sysmon): the recent history tiers stay when the worker ends (07e4cf7)
- ci: run the system monitor readers on their own on every OS (48916dd)
- ci: the system monitor step also runs the terminal UI's monitor tests (55304fb)
- ci: real service control and pictures of the monitor window on a Mac (344d596)
- test(tui): the screen-reader override test waits for the new pane's listing (bf39b8f)
- ci: drop the macOS monitor pictures step (0fa3d50)
- test(jobs): the remove test waits for the Removed event (21f47b3)
- release: 0.6.0 (system monitor in a tab of the window and the terminal UI: processes as list or tree with custom columns and text, regex and column filters; CPU, memory, disk, network, GPU and sensor graphs; connections, services, startup programs; optional recorded history, snapshots, compare, alerts) (43fd0a8)

## 0.5.12 (2026-11-05)

### Other features

- feat(app): the cursor moves to a neighbour when the item under it is deleted (cf795d3)

### Fixes and security

- fix(vfs): russh 0.65.1 and pageant 0.2.5 for the open security advisories (f9c8028)
- fix(app): the keyboard-only test no longer hangs when Enter opens a file (1ec35b4)

### Other

- build(deps): bump zip from 9.0.4 to 9.0.5 (#17) (3736185)
- build(deps): bump zip from 9.0.4 to 9.0.5 in /fuzz (#16) (a005a0d)
- release: 0.5.12 (security update: zip 9.3.0 for a critical archive parsing advisory, russh 0.65.1, pageant 0.2.5; the cursor moves to a neighbour when the item under it is deleted) (ceea4f2)

## 0.5.11 (2026-11-05)

### Fixes and security

- fix(jobs): undo no longer takes a new file on a reused inode for the moved one; CI fixes (55ce9a1)

### Other

- release: 0.5.11 (the 0.5.10 changes, built: its release stopped on Linux test failures and a download outage; undo no longer takes a new file on a reused inode for the moved one; async-executor and clipboard-win updated, the old ones were yanked) (b867311)

## 0.5.10 (2026-11-05)

### Other features

- feat(app): Open with gets a program browser and remembers what you pick (66acd82)
- feat(app): names keep what matters when cut, one button system, and smaller touches (95ff08f)
- feat(app): text size reaches every surface; tabular figures; theme radii; narrow-window crumbs (8b91eb3)

### Fixes and security

- fix(app): Enter on a focused Cancel no longer confirms a dialog (2d06707)
- fix(core): never overwrite a config.toml that does not load, and keep unknown keys when saving (331b182)
- fix(jobs): stopped jobs keep an undoable entry; a remote replace trashes the old file last (22cd529)
- fix(app): the filter never hides a delete target; toast Undo names its operation (2bb3268)
- fix(fs): copying a link keeps the old file until the link exists; Windows trash and quoting (8e38a48)
- fix(archive): safer link extraction and spanned zip directories (4ac942e)
- fix(vfs): a resumed transfer must match its source, and length is checked before commit (933ab5d)
- fix(mcp): registering a client refuses a config.toml it cannot read (fe90683)
- fix(app): the selection count leaves out what the filter hides; review 3 status (c8f56fd)
- fix(app): a typed path that does not exist keeps its error page; no toolbar folding; git fallback 15 s (3d7743f)
- fix(app): the narrow toolbar never overlaps; a path typed in the palette keeps its error page (7207209)

### Documentation

- docs: review 3 findings (bugs, performance, UX and typography) (c172c10)

### Other

- perf(core): natural sort and extension keys no longer allocate per comparison (415b990)
- perf(appcore): clicks, cursor position, selection stats, group labels and streaming merges cost less (d58cc3b)
- perf(appcore): job snapshots at most every 100 ms; thumbnail eviction keeps running totals (de05276)
- perf(appcore): write config.toml on a worker, keeping the refusal for a file that does not load (70bfedb)
- perf(appcore): git status refreshes when the git directory changes; refresh_secs is a 60 s fallback (f822ced)
- perf(app): watch the themes folder and poll the OS appearance less often (b74d7dd)
- release: 0.5.10 (Open with program browser that remembers the choice; text size reaches every surface; tabular figures; natural sort about 16 times faster; data safety fixes) (be56d9d)

## 0.5.9 (2026-11-03)

### Other features

- feat(app): clear the activity history; Delete moves to the trash without asking (fcfb34a)
- feat(app): text and code previews use Hack at 13 pt (3c64f0b)

### Fixes and security

- fix(app): the UI no longer freezes when a job with thousands of items finishes (a1eae9e)

### Other

- release: 0.5.9 (text and code previews use Hack; clear the activity history; Delete moves to the trash without asking; no freeze after huge jobs) (9b3f0f3)

## 0.5.8 (2026-10-31)

### Fixes and security

- fix(app): Linux CI: the context-menu tests mark only .exe files executable; zstd 0.14.3 / zstd-safe 8.2.0 (RUSTSEC-2026-0391, RUSTSEC-2026-0395) (25e0c4f)

### Other

- release: 0.5.8 (the 0.5.7 changes, built: its release stopped on a Linux-only test failure; zstd updated for RUSTSEC-2026-0391 / RUSTSEC-2026-0395) (f7b3c90)

## 0.5.7 (2026-10-30)

### Other features

- feat(app): tidier context menus: quick actions bar, submenus, compact menu with Shift+right-click for everything, type entries first, inline tag dots (c494c4e)
- feat(app): Shift+F10 opens the context menu in the window (a675d71)

### Fixes and security

- fix(fs): programs that need administrator rights open again (UAC prompt), and "Run as administrator" is in the context menu (b8a5166)

### Other

- release: 0.5.7 (programs that need administrator rights open with the UAC prompt, Run as administrator in the context menu, a shorter configurable right-click menu) (a400684)

## 0.5.6 (2026-10-28)

### Fixes and security

- fix(fs): exe_icon doc comment no longer names a banned shell API (the no-shell32 CI grep matched it) (dd34d34)

### Documentation

- docs: CLAUDE.md notes that the no-shell32 grep also matches banned names in comments (fc934f9)

### Other

- release: 0.5.6 (CI fix: a doc comment no longer trips the no-shell32 check; no functional changes) (2aa957b)

## 0.5.5 (2026-10-26)

### Fixes and security

- fix(app): no repaint loop while two windows have different sidebar widths (e.g. the main window minimized next to another window): the title bar's settled-width check was one key shared by every viewport, so the windows kept invalidating each other at the display's refresh rate (4b99c32)
- fix(app): a folder opened from Explorer while every Soyuz window is minimized opens at once (as a tab of the restored window) instead of waiting 5 s for the launch to expire and starting a second process; eframe runs no UI pass for a minimized window, so logic() now asks for one when launches or FileManager1 calls wait (it cannot create a window there); log timestamps in milliseconds; SOYUZ_FRAME_LOG names what asked for each frame (45b5ba0)
- fix(app): dragging files out of a secondary window (e.g. one opened from Explorer) into a browser, Explorer or any other app works again: only the main window ran the drag-out hand-off, the drag badge and the "ask" drop menu; afterwards the window gets the swallowed button release as a WM_LBUTTONUP to itself (eframe runs no raw-input hook for those windows) (73cb07e)

### Other

- release: 0.5.5 (folders opened while Soyuz is minimized open at once, drag-out works from every window again, no redraw loop with the main window minimized) (2f71b82)

## 0.5.4 (2026-10-26)

### Other features

- feat(app): programs (.exe / .scr) show their own icon in the details, list, column, Miller and grid views and the terminal UI's grid, read from the PE resources by soyuz_fs::exe_icon (no shell API, nothing from the file runs; Windows' own programs fall back to their SystemResources .mun), made on the thumbnail workers and disk-cached; [appearance] program_icons, Settings -> Appearance, off in safe mode; fuzz target exe_icon (75027ad)

### Fixes and security

- fix(tests): the TUI devices test waits for the sidebar focus and the cursor back on the locked device before pressing Enter (the unlock dialog sometimes never opened) (7ce4cd9)

### Other

- release: 0.5.4 (programs show their own icons in every file view, read directly from the program file) (6658f46)

## 0.5.3 (2026-10-26)

### Other features

- feat(app): single instance: starting Soyuz again hands the launch to the running window ([workspace] launch = new_window | new_tab | new_process) (1dde58b)
- feat(app): every window has its own preview, Quick Look, search bar and bottom / activity panel; searches and folder sizes keep running in secondary windows, FileManager1 / soyuz open land in the focused window, the main window's terminal no longer takes keys typed in another window, and the player follows the window with Quick Look open (b99db85)
- feat(app): the terminal works in every window: sessions belong to their tab or window, focus and keyboard ownership are per window, closing a window with running shells asks first ([terminal] close_window_with_sessions = ask | kill); raising a window no longer injects an Alt press, focus mode is per window, audio plays only in the player's window, and devscript gains win:<k> <step> (6f27353)
- feat(app): editors, the assistant, the shelf and tool windows open in the window they were invoked from and move when invoked from another one ([workspace] panels_follow_window = true); closing their window hands them to the main window with unsaved documents kept, and they take keys only in their own window (c365af7)

### Fixes and security

- fix(fs): tmpfs / ramfs report hard links (and tmpfs sparse files), so duplicate-aware copy links into /tmp on Fedora / Arch; Windows Open-with names come from the exe's file name on every OS (Path only splits on \ on Windows, so the Linux test run named GIMP "C:\GIMP 3\bin\gimp-3") (c54b00d)
- fix(app): scripted / off-screen runs (SOYUZ_WINDOW_POS) don't restore or save eframe's window geometry, so a reused portable-data no longer opens them at 0,0 on screen (794c47f)
- fix(app): panels docked on the right stop growing where the file list would get narrower than [layout] min_file_list_width (320), so the editor and the assistant together no longer squeeze the listing to a sliver; the text editor's status line drops its least important parts when narrow instead of overlapping (57672dc)
- fix(fs): opening HTML files and web links in Chrome / Edge works (their --single-argument %1 takes the rest verbatim, so the path is no longer quoted into http://"c/..."); Windows open_url starts the program registered for any scheme, and Ctrl+click on a web link in the terminal opens the browser in both frontends (cda7ba8)
- fix(tests): CI flakes and a Windows-runner failure: the TUI tree test resolves CI's 8.3 temp path (C:\Users\RUNNER~1, a segment no listing contains), the media pause test lets the packet in flight land before measuring, and the hardware / YUV player tests count frames dropped as late on slow macOS runners (2a4dd81)
- fix(deps): Security workflow advisories: smb2 0.35 (allows ctutils 0.4.5, RUSTSEC-2026-0385) and crypto-bigint 0.7.9 (RUSTSEC-2026-0387, 0.7.5 yanked); ci: drop the per-OS media tests with the FFmpeg runtime (the ffmpeg job and media-v7 stay) (bb72814)

### Documentation

- docs: Phase S handover (single instance, per-window features, memory measurements, config keys, open issues) (8b56fef)
- docs: Phase S is merged into main (c030a06)

### Other

- ci(release): wait for a green CI run on the tagged commit before building (ci-gate job; fails on a failed / cancelled CI run or none within 10 minutes; manual runs can skip it with skip_ci_gate) (0ce7d9c)
- build: pin toolchain to Rust 1.99.1 (was 1.98.1); clippy 1.99's single_element_loop in the media-tools preset picker (32ed8a3)
- linux build fix (f01faf6)
- perf(app): [performance] gpu_memory = "low" (default) starts wgpu's allocator with small blocks and caps the Direct3D 12 descriptor heap (64f55ae)
- perf(app): fallback fonts are mapped instead of read into the heap and load only for text that needs them; thumbnails count their real GPU cost, expire when unused ([preview] thumb_idle_trim_secs = 60) and default to a 64 MiB budget (cd3a642)
- perf(app): the translation catalog parses only the active language (en-US again only on a miss) and drops the other bundle on a language switch, about 3.9 MB less heap (b55d386)
- refactor(app): the launch handoff opens tabs through models::windows::with_window (804e5c6)
- perf(app): listing entries take about half the memory (842 -> 434 B each: no growth slack once a listing finishes, a name index of entry positions instead of copied names, exact-size paths); About shows the process's private memory and working set (f372902)
- release: 0.5.2 (every window runs in one process with much less memory, right-docked panels keep the file list readable, HTML files and web links open correctly in Chrome / Edge, Ctrl+click on terminal web links opens the browser) (c9e78ec)
- release: 0.5.3 (security fix: updated cryptography crates for the SMB and SSH clients close timing side channels, RUSTSEC-2026-0385 / RUSTSEC-2026-0387) (b854332)

## 0.5.1 (2026-10-23)

### Fixes and security

- fix(media,ci): FFmpeg load errors keep the loader's reason (libloading 0.9 shows only "dlopen failed" and keeps dlerror in source()); the Linux media-with-runtime job installs libva2 / libva-drm2 / libdrm2, which the VAAPI build's libavutil links; the chip test expects macOS's modifier symbols there (3742fd1)
- fix(fs): Windows Store-app defaults open like Explorer: a COM-only default (Photos) is handed to explorer.exe as its own process ([open] store_apps_via_explorer, on by default; off = first launchable Open-with program); Store apps with an app execution alias (Paint, Notepad, Snipping Tool) start directly and are listed in Open with by name; UserChoiceLatest is read before UserChoice; print / printto are never the default verb (a .jpg double-click ran jpegfile's printto handler and did nothing), SystemFileAssociations menu verbs no longer count as a default; package executables and their aliases dedupe in Open with (db0a204)

### Other

- release: 0.5.1 (Store-app defaults such as Photos open on double-click again on Windows, Store apps with a command alias in Open with, FFmpeg load errors keep the loader's reason) (959d2b3)

## 0.5.0 (2026-10-22)

## ffmpeg-runtime-9.0.2 (2026-10-22)

### Other features

- feat(app): system folders below the favorites (AppData, ProgramData, Temp …; ~/.config, ~/.local/share, ~/.cache, /tmp …) after a System divider whose hide button asks first and says where to show them again; [sidebar] system_folders / system_folder_list, Settings → Views → System folders picks them; terminal UI lists them too (5578947)
- feat(app): index and smart folder actions in the command palette (New smart folder, Save search as smart folder, Index settings, Rebuild index, Turn file indexing on or off); Rebuild shows only while indexing runs, Save search only on a search pane (37fba2d)
- feat(app): script actions in the Keyboard settings list (GUI and terminal UI), every one of them, not only palette ones, with their script hotkeys shown and rebindable like built-in actions; the key cache is rebuilt when scripts reload (d5c8664)
- feat(app): comparing a changed text file with one of its previous versions shows the lines that changed (the preview's diff view in the GUI panel, the first changed lines in the terminal UI dialog); follows [preview] diff / max_diff_kib (fb82b79)
- feat(app): focus mode (Ctrl+Shift+B, palette): hides the sidebar, status bar, preview, bottom / activity / assistant panels and the tab strip around the listing and brings back exactly what was shown; [layout] focus_mode_hides picks the parts (Settings → Views), GUI and terminal UI (4f339a6)
- feat(jobs): queue settings apply without a restart: JobEngine::set_queue / QueueSettings (max_parallel starts waiting jobs or lets running ones finish, per-device queue for new jobs, apply-to-all reset, history limit trims now); the restart toast and notes are gone (6d45954)
- feat(app): password-protected PDFs: the preview shows a lock card with Enter password…, PDF password… in the item menu and palette (terminal UI too) asks with the password prompt; pdfium and the lopdf fallback open the file with it (kept for the session, never on disk), a wrong one says so (e641fdb)
- feat(app): sorting by a script column uses its values: every row's value is asked of the script worker and the pane re-sorts as they arrive (natural order, rows without a value last; soyuz_core::sort_entries_keyed); headless test (2ca670e)
- feat(app): folder notes: a folder's preview shows its README / notes file (Markdown rendered, read on the worker) under a compact summary, with Edit to open it in the built-in editor; terminal UI shows the text; [preview] folder_notes / folder_notes_files in Settings → Preview (01d0047)
- feat(app): clipboard history of file sets: Ctrl+Alt+V (palette's Clipboard history mode, GUI and terminal UI) lists the sets copied or cut this session, newest first, and Enter copies one again for Ctrl+V; [behaviour] clipboard_history (default 10, 0 = off) in Settings → Behaviour; headless test (5f1a9c6)
- feat(vfs): WebDAV Digest authentication (RFC 7616, MD5 / SHA-256, qop=auth): a probe on the first request finds out whether the server wants Digest, requests then carry a signed Authorization, a stale nonce is re-probed once; the test DAV server gets a Digest mode; RFC test vectors and a round trip (23b26c3)
- feat(app): WOFF 2 fonts in the font preview, decoded with wuff (pure Rust: Brotli and the glyf / loca / hmtx transforms); the declared size is checked against max_document first and a malformed file is an error, never a panic; round-trip test from a bundled font (c1df76b)
- feat(index): NTFS journal catch-up without administrator rights: the volume opens through its root folder when the device is refused and reads with FSCTL_READ_UNPRIVILEGED_USN_JOURNAL (Windows 11); the MFT scan stays admin-only; Windows 10 without rights reports Needs administrator as before; restart catch-up test (2c22364)
- feat(archive): PKWARE spanned / split zips (name.z01 … name.zip, from zip -s or WinZip) open from either end: the disks are joined and the per-disk central directory is rebuilt with global offsets (zip64 when needed) behind a fresh end record, so listing, extraction and testing work unchanged; a missing disk is a clear error; sets are read-only; tested on synthetic sets and a real Info-ZIP set (f56a422)
- feat(archive): CAB cabinet sets: opening any cabinet of a set walks back to the first one and forward through the next-cabinet names; folders continued across cabinets decode as one stream (including data blocks split at the boundary), files stored in several cabinets are listed once, and data in a missing cabinet fails to read with its name; tested on synthetic stored sets and a real makecab MSZIP set (c4ae214)
- feat(search): content search looks inside PDFs: soyuz-core gets a document-text hook (doc_text) that the app fills with its PDF reader (pdfium when installed, else lopdf; the whole document, up to [preview] max_document_mib); the search walker matches the text instead of skipping the file as binary, and the index stores PDF words for content: queries; [search] document_text (default on) in Settings → Search; tests for the walker, the index and the app's registration (71899d5)
- feat(remote): Connect to server lists servers on the local network: one-shot mDNS / DNS-SD queries (legacy unicast from an ephemeral port on every IPv4 interface, so the system's responder is untouched) for SFTP, WebDAV, FTP and, off Windows, SMB, with follow-up SRV / TXT / A questions; picking one fills protocol, server, port, folder and user (GUI list, terminal UI choice); [remote] discover in Settings → Remote; tested with a fake responder, against a real printer, and in the dialogs (6a61e58)
- feat(jobs): "Why is this slow?" in a job's details: the engine's sampler charges each interval to moving / stalled / counting / paused / waiting-for-you, keeps the peak and average speed, and admission records why a queued job waits (the job on the same disk or folders, the parallel limit, disk lookup); the details show speeds, files/s, average file size, where the time went and plain hints (stalls, small files, same disk, verify, long counting); [jobs] io_stats in Settings → Jobs; engine and hint tests (97dd9fc)
- feat(tools): Quick share: the selection (or the folder shown) served read-only over HTTP on the local network while its window is open, with the address as a QR code to scan with a phone; a random 128-bit token in every URL, folders browsable inside (symlinks leading out refused), byte ranges for video seeking, files inline under a sandbox CSP or downloaded with ?dl=1; ends when closed, stopped or after [remote] share_minutes (30), [remote] share_port fixes the port; addresses of virtual adapters (WSL, Hyper-V, VMware, Docker) sort last; GUI tool window and terminal UI dialog (half-block code, C copies the address); SOYUZ_SHARE_BIND debug hook keeps scripted runs on loopback; server, model and dialog tests (1c5e36d)
- feat(fs): disk health on This PC's drive cards: SMART and the NVMe health log read without administrator rights on Windows (FILE_ANY_ACCESS IOCTLs on the volume's physical disks: the NVMe log page, temperature, failure prediction with the ATA attribute table) and through UDisks2 on Linux; a verdict (healthy / needs attention / failing) from the critical warning, wear, spare, bad sectors, media errors and the drive's temperature limit; cards show a coloured dot with temperature and wear, the tooltip every figure, the terminal UI a Health column; a failing drive raises one warning per session; [sidebar] disk_health in Settings → Views → Drives; parser tests, checked on 3 NVMe and 4 SATA drives (7c6d52f)
- feat(tools): Compare as treemap, a visual folder diff: the treemap of a folder with every tile coloured by how it differs from the other pane's folder (or the second selected folder): only here, newer here, newer there, different, differences inside, same; a compare runs beside the scan (again on rescan), the legend sums the bytes per mark in the level shown, the line above the map counts the differences and the items only in the other folder; Differences / Kinds switch (D in the terminal UI, fill characters per mark without colours); [sync] treemap_diff_content compares bytes instead of dates and sizes (Settings → Jobs); palette action, two-folder and two-pane menu rows, SOYUZ_OPEN_TOOL treemap=<a>|<b>; model, mark and terminal UI tests (4b42ce2)
- feat(jobs): duplicate-aware copy: before a copy job moves data, files of the copied set that share a size (64 KiB and up) are hashed (BLAKE3), and each later file with content already copied becomes a copy-on-write clone of that first copy where the destination volume can clone (ReFS / Dev Drive, btrfs, XFS, APFS), or a hard link to it when asked; nothing is hashed where the volume can't, a failed clone / link falls back to a copy, clones keep their own times, undo removes them with the copy, the summary counts them; [jobs] copy_duplicates = clone | link | copy (Settings → Jobs → Duplicates in a copy); engine test (83b0560)
- feat(vfs): archives as a provider: the inside of any archive soyuz-archive reads is a read-only location (soyuz_vfs::archive::ArchiveProvider: list / stat from the dir index, entries streamed through a worker pipe), registry::resolve maps a.zip!/x to it, so Copy / Copy to other pane / Paste take files and folders out of archives as remote-transfer jobs (progress, conflicts, undo; Cut / Move say archives are read-only); archives inside archives and on servers are fetched to a private temp copy and browse, preview, test and extract like local ones (Enter on a .tar in a .zip browses it, Up returns to the outer archive; split_vpath takes the last separator); [archive] nested_max_mib (2048, 0 = off) in Settings → Jobs; Extract here / to folder and zip editing stay with archives on disk; the devscript cd step parses locations like the address bar; provider, job and location tests (a4ede46)
- feat(archive): editing tar and 7z archives, not only zip: one planner (soyuz_archive::edit::plan: delete, rename / move with folder merges, add / replace) and a writer per format behind rewrite_archive: zip entries are still copied raw, tar / tar.gz / tar.bz2 / tar.xz (tar.zst with native zstd) entries stream through with their headers (mode, owner, times, links; sparse ones written whole) under their new names and are recompressed with the same codec, 7z entries are decoded and encoded again (non-solid LZMA2); encrypted 7z and RAR / ISO / CAB stay read-only; the EditArchive job, Paste / Rename / Delete in a browsed archive and its context menu take tar and 7z; [archive] edit_level is the level for the whole rewrite (Settings → Jobs → Compression when editing an archive); archive-crate and job tests for tar, tar.gz, tar.xz and 7z (0e41b10)
- feat(ai): the command palette's ? mode asks the assistant in plain words, and plan cards get an editable job preview: "? move the PDFs here into Documents" starts a task told to propose file operations (earlier requests come back as suggestions, the panel takes the keyboard); Edit on a copy / move / compress / extract / trash / tag card opens a form (a check box per item, unchecked ones left out; the destination; "Edit as JSON" for the rest) whose result is planned again as a new card (ai::panel::edit::form / form_args, schema-checked); a card replaced by an edit closes its editor; terminal UI: the form as a dialog (Insert / click toggles, Space without a destination), J opens the JSON editor; [ai] palette in Settings → AI; form, palette and headless tests (form edit moves elsewhere, ? → plan → approve) (1eec9bd)

### Fixes and security

- fix(app): Extract to folder (and Extract to… into a new folder) picks the free folder name on a worker instead of up to 1000 checks on the UI thread (Cmd::ExtractNewFolder, UiMsg::Cmds); headless test (1deb078)
- fix(core): correct AMF v1.5.3 commit pin (a4e0d31)
- fix(ci): accept FFmpeg signing subkeys (5fe3093)
- fix(ci): allowlist public FFmpeg key fingerprint in gitleaks (f3920ed)
- fix(ci): avoid parsing FFmpeg GPG status (336c054)
- fix(ci): FFmpeg signature check fetched no key: keys.openpgp.org serves the release key without its user ID, gpg skips it but exits 0, so the keyserver.ubuntu.com fallback never ran and --verify had an empty keyring; a fetch now counts only when the key is in the keyring, ubuntu first (ca11b9b)
- fix(ci): win64 FFmpeg configure found no libvpl: libvpl 2.17 writes ${CXX_LIB} into vpl.pc but never sets it, so the C link test of the C++ dispatcher missed libstdc++; pass -DCXX_LIB=-lstdc++ (static through -static-libstdc++ -Wl,-Bstatic) and fail early when vpl.pc lacks it (0e2b292)
- fix(ci): FFmpeg runtime on linux64 and macOS: the build record's toolchain line ended in $([[ win64 ]] && echo …), whose status 1 stopped the script silently under set -e right after FFmpeg built on every slice but win64; macOS ran /bin/bash 3.2, where source <(…) reads nothing ("FFMPEG_VERSION is not set"), so the macOS job installs Homebrew bash first on PATH and the script refuses bash older than 4 (20428cc)
- fix(ci): tests on every runner: the index test compares against the canonical root (Windows runners' TEMP is an 8.3 path, roots are stored long), the FFmpeg offer finds WindowsApps in a Windows path on any host (Path::components only splits on the host's separators), the media search-order test joins its expected config path like the code, and system_folders' env_of helper is compiled only for the Windows / Linux tests that use it (dead code on macOS) (af0eb2a)
- fix(deps): rustls-webpki 0.103.16 (RUSTSEC-2026-0382, distrusted intermediates accepted), webpki-root-certs 1.0.10 (RUSTSEC-2026-0383, distrusted emSign intermediates), inout 0.2.5 (0.2.3 yanked); lockfiles only (21d13a3)
- fix(script): pipe_round_trip raced its own --queue copy on Windows: the job could still be reading a.bin when the test rewrote it (os error 32, sharing violation); the headless fallback step copies its own b.bin (e6ccbc6)

### Other

- release: 0.4.4 (system folders below the favorites with a System divider that hides them) (e9c50d0)
- release: 0.5.0 (tar / 7z archive editing, copying out of archives, Quick share, disk health, Compare as treemap, duplicate-aware copies, PDF content search, assistant requests in plain words) (13ed278)
- Initial plan (0495a80)
- Initial plan (ef9b7d6)

## 0.4.3 (2026-10-19)

### Other features

- feat(app): run programs on Enter / double-click (Linux): an executable ELF binary or AppImage starts from its own folder instead of going to Open with… ([open] run_binaries, on by default; scripts and .so libraries still open as files) (aa4f443)
- feat(app): blank space below the last item of every folder listing, so a full folder still has room to right-click for New folder / Paste ([appearance] list_end_space, default 64 px) (0499768)

### Fixes and security

- fix(fs): build on Windows again: is_runnable_binary uses unix permissions, so compile it only on unix (the launch::linux module is also built on Windows for tests) (0fd475a)
- fix(app): Ctrl+C / Ctrl+V in text fields use the system clipboard: eframe is built without its clipboard feature, so egui kept copies in a private in-process string; an egui plugin now writes CopyText through os_clipboard (raw user32; Linux native Wayland / X11 selection, new text path) and fills paste events from it (8a44b6b)
- fix(app): paste pictures as files again: the paste reads the clipboard picture itself when it holds no files (Windows PNG / CF_DIBV5 / CF_DIB through raw user32, DIBs wrapped as BMP; Linux image/* from the native selection, off the UI thread), decoded on the encoder thread; Paste is enabled for a picture on the clipboard (841a35c)

### Other

- updates for AI agents (965b7d2)
- release: 0.4.3 (copy / paste text with the system clipboard in every text field; paste pictures as files again; blank space below full folder listings for the folder menu; run programs on Enter on Linux) (eb5916d)

## 0.4.2 (2026-10-16)

### Other features

- feat(app): egui 0.37 features: colour emoji, kinetic scrolling with rubber-banding, paste pictures as files (a36096e)
- feat(app): default apps for unrecognized files: Enter on a type no app is registered for opens Open with… ([open] choose_when_unknown); files without an extension share the '.' key (default app, last used, 'Always use this app for files without an extension') (854c710)

### Other

- build: egui / eframe / egui_extras / egui_kittest 0.37 (wgpu 30, accesskit 0.25) (6b11f25)
- build: dependency upgrades: zip 9 (8.6.0 was yanked), sha1 / sha2 0.11, lzma-rust2 0.22, sevenz-rust2 0.24, rars 0.11, smb2 0.31, rodio 0.23, libloading 0.9, getrandom 0.4, all semver-compatible updates (d8a2adb)
- release: 0.4.2 (egui 0.37: colour emoji, kinetic scrolling, paste pictures as files; Open with… for unrecognized file types; dependency upgrades) (ff20220)

## 0.4.1 (2026-10-16)

### Other features

- feat(app): play / pause key for Quick Look: P in the Player context (K stays, P is the hinted key), `[media] quicklook_space_plays` (Space plays / pauses media in Quick Look, Esc closes; Settings → Media), play key in the GUI Quick Look footer and the TUI hints; docs regenerated (00ee424)
- feat(app): default apps per file type: `[open]` section (default_apps rules, remember_choice, last_used); Enter / double-click / archive and remote files / `soyuz open` use the rule, else the system's default; Open with… starts on the default or last-used app and has "Always use this app for .ext files" (GUI and TUI); Shift+Enter opens Open with…; Settings → Behaviour → Opening files with; docs regenerated (bbbd22e)

### Other

- build: dev profile keeps line tables only (dependencies no debug info), so a full debug build + tests is ~13 GB instead of hundreds; scripts/trim-target.ps1 removes stale cargo build units (fingerprint, build-script folder and deps files together) (dfd31b4)
- release: 0.4.1 (default apps per file type inside Soyuz, Shift+Enter for Open with…, P plays / pauses in Quick Look, smaller debug builds) (ca7ecfe)

## 0.4.0 (2026-10-15)

### Other features

- feat(media): Phase V V0 skeleton: soyuz-media crate (public API as stubs, real search_dirs / lib_file_names for FFmpeg 9.0 with 8.x / 7.1 accepted), [media] config + schema 3 migration ([preview] video_ffmpeg -> video, ffmpeg_path dropped), KeyContext::Player, update check only picks soyuz-<version>- assets, unsafe allowlist file + CI check, third_party/ffmpeg pins and layout probe (f7eb3be)
- feat(media): Phase V V1c: FFmpeg runtime service (worker-thread load, status, crash sentinel), video posters / thumbnails / metadata through soyuz-media with preview_media as fallback, ffmpeg CLI removed, one extension table, Settings media group, LGPL notice in About, TUI video thumbnails (78aa723)
- feat(media): Phase V V1a: soyuz-media core: run-time loader (absolute paths, Windows search flags, version-set check, GPL flag), 116 bound symbols, repr(C) prefix layouts per major (7.1 / 8.x / 9.0) with pinned offsets and a probe comparison test, load-time self-test, own AVIOContext for every source with catch_unwind and stale checks, open/decode limits; probe (streams, rotation, HDR, chapters, tags, cover), grab_frame, AudioReader, peaks; with-libs tests and three tiny fixtures (e740520)
- feat(media): Phase V V2a: Player engine (demux / video / audio threads, audio-clock master with wall-clock fallback, accurate coalesced seeks, late-frame dropping, loop, volume / mute, SetOutputSize, bounded Drop that abandons stuck FFmpeg contexts) with clock / queue unit tests and with-runtime player tests (fdba642)
- feat(media): Phase V V2b: FFmpeg audio backend for previews (probe + peaks, covers incl. METADATA_BLOCK_PICTURE, Symphonia fallback), AudioReader-fed rodio playback with live volume / mute, RodioOutput ring sink with a device-side clock, PlayerModel for the GUI and TUI players, Settings rows for the player keys (8ffe12f)
- feat(media): Phase V V2c: transcoder (remux, trim by packets or frame-exact, convert, extract audio, save frame; .part + rename, progress by pos or pts, cancel between packets, dry-run plan) and 14 presets incl. hardware H.264 / HEVC, VP8 / VP9 + Opus, ProRes, MPEG-4, FLAC / AAC / Opus / WAV / ALAC; encoder and muxer FFI over our own write AVIO (aa97e1d)
- feat(app): player and media-tool actions for Phase V wave 3: Player* actions in KeyContext::Player (K / J / L, arrows, - / =, M) dispatched to the player model, MediaConvert / MediaExtractAudio / MediaTrim / MediaSaveFrame (dispatch lands with V3c), palette icons, en-US / tr-TR strings, KEYBOARD.md regenerated (546c4da)
- feat(app): Phase V V3a: GUI video player in the preview pane and Quick Look (poster, play overlay, controls bar with scrubber / buffered range / volume / mute / loop, AccessKit names, debounced output size, one reused texture per player), player opened on a worker (pending state), KeyContext::Player when the player has focus, audio card volume / mute, player smoke test and a video step in the frame-budget tour (fa268c7)
- feat(tui): Phase V V3b: terminal UI player in the preview pane and Quick Look (frames through the terminal image path at [media] tui_fps, status line with text progress bar, volume, mute, loop, mouse seek, screen-reader words), KeyContext::Player in the TUI, headless player tests and snapshots (aea41a0)
- feat(app): Settings row for [media] tui_fps (terminal frame rate) (b930f5b)
- feat(media): Phase V V3c: media tools as jobs (JobKind::Media "media": convert, extract audio, trim, save frame through soyuz-media with conflict policies, trash-backed overwrite, identity-stamped undo, pause / cancel, low priority, journal resume), dialog model with off-thread probe and dry-run plan, GUI and TUI dialogs, context-menu entries, [media] tools setting (22963b3)
- feat(media): Phase V V4c: in-app FFmpeg runtime download (own asset matcher on the ffmpeg-runtime tag, SHA256SUMS check, streamed hash, staged replace of <config>/ffmpeg, reload without restart; Settings button and palette action, hidden in Flatpak / MSIX), app updater never picks runtime assets, Linux in-place updater installs lib/soyuz and the FFmpeg licence texts (4fb86db)
- feat(ai): Phase V V4a: media tools for AI, MCP and Rhai (media_info read tool; media_convert / media_extract_audio / media_trim / media_frame as dry-run plans of JobKind::Media jobs with conflict policy and scope checks; [media] ffmpeg / tools gates; crash sentinel on app and MCP workers; Rhai media_info / convert / extract_audio / trim / save_frame) (ac0e38e)
- feat(media): Phase V V4b: hardware decoding (D3D11VA / VAAPI / VideoToolbox through hw_device_ctx, run-time offset check, transfer to NV12 / P010, software fallback mid-stream) and GPU YUV path (wgpu callback, BT.601 / 709 / 2020, range, PQ / HLG tone map; [media] gpu_yuv, off by default); Settings rows for hw_decode and gpu_yuv (b8a230d)
- feat(media): Phase V gap fixes: [media] threads reaches the transcoder (JobOptions::media, journal), resumed media jobs keep their tool label, chapters / tags / cover art / rotation kept where the container allows (warnings otherwise), pure-Rust faststart for MP4 / MOV ([media] faststart, Settings row) (e253cad)
- feat(media): player polish: 150 ms audio pre-roll before the clock runs (engine and rodio ring), packet queues bounded by bytes, AudioReader is Send, hidden test-seams feature with a mid-stream hardware-fallback test; fix(media): request_seek sets the queue interrupts before publishing the target (47f621f)

### Fixes and security

- fix(app): closing the player drops it on a worker thread (the bounded join never runs on the UI thread) (7ccc275)
- fix(core): player sync_target(None) closes the player (a missing pending key no longer counts as the same key); TUI reads the model's loop toggle and opens the engine without its own loop (e91d83b)
- fix(app): media tools dialog text fields named by their captions, player Mute / Loop report their toggle state; docs: user guide notes on what media tool outputs keep, accessibility re-run (de57719)

### Documentation

- docs: Phase V handover (FFmpeg media): decisions, what wave 1 delivered, how the pieces fit, local testing, first CI run checklist, remaining waves and risks (0762f79)
- docs: Phase V handover and soyuz-media crate docs updated for wave 2 (player, FFmpeg audio + player model, transcoder) (936d468)
- docs: Phase V handover after wave 3 (V3a GUI player, V3b TUI player, V3c media tools as jobs, gates, open points) (6377090)
- docs: Phase V user guide Media section, accessibility notes and UIA audit media states, demo video in the fixture, media screenshots (c16f267)
- docs: Phase V complete: handover (wave 4, gap and player passes, final gates, after-Phase-V list), user guide note on playback start-up (32f95cb)

### Other

- ci: cargo-audit is built with cargo install (cached) instead of taiki-e/install-action, which the repository's Actions allow-list refuses (1590db7)
- build(media): Phase V V1b: FFmpeg 9.0 LGPL build script (dav1d, opus, libvpx, nvenc/AMF/libvpl on Windows, VAAPI on Linux, universal macOS; gpl/nonfree and DLL-import gates), CI ffmpeg / media / media-v7 jobs with layout probe, release packaging of the runtime in every artifact plus a separate ffmpeg-runtime-<ver> release, MSI/MSIX/rpm/deb/macOS bundle changes, entitlements, LGPL notices and RELEASING docs (b825784)
- build(media): layout probe also prints AVFrameSideData.buf (soyuz-media's layout test checks it) (49d82c9)
- test(media): layout oracle also checks the AVCodec prefix (name, long_name, type, id, capabilities) when the probe prints it (1686180)
- build(media): layout probe prints the AVCodec prefix rows (37daeff)
- release: 0.4.0 (Phase V: media through the bundled FFmpeg libraries: video player in the GUI and TUI, media tools as jobs, AI / MCP / Rhai media tools, hardware decoding, in-app runtime download) (f3f99c9)

## 0.3.2 (2026-10-14)

### Other

- test(ai): the junction scope test checks the fully resolved spelling (CI's temp folder is itself a short name, C:\Users\RUNNER~1, so the junction target as written was neither the given nor the resolved spelling) (ae75995)
- test: every test that starts a freshly copied binary retries on "Text file busy" (Linux ETXTBSY while another test thread's fork still holds the copy open): the TUI relaunch harness (shared retry_busy, also used by the binary smoke test), the appcore devscript relaunch and the keyboard-only GUI test (b3d8a7a)
- release: 0.3.2 (CI test fixes: junction scope test on short-name temp folders, ETXTBSY retries) (c20c32c)

## 0.3.1 (2026-10-14)

### Fixes and security

- fix(ai): scope accepts paths spelled like the configured root (8.3 short names such as C:\Users\RUNNER~1, junctions); the resolved path must still be inside a resolved root (7105143)

### Documentation

- docs(site): explain the sample search query in plain words (e2265a7)

### Other

- test(app): keyboard-only test waits for the list to show the renamed file before selecting the next one (CI's software renderer lost the delete) (43aaac9)
- test(tui): CI fixes: snapshot shots force a full redraw (ratatui's test backend kept a stale cell after a wide glyph; the palette shot differed on Linux), the binary smoke test retries a start that hits ETXTBSY on Linux, and places.rs no longer imports a Windows-only helper on macOS / Linux (d26b926)
- release: 0.3.1 (rustls 0.23.46: RUSTSEC-2026-0346 to 0349) (a835021)

## 0.3.0 (2026-10-14)

### Other features

- feat(app): full-size app icon (c1284c6)
- feat(tui): T1.0 soyuz-tui crate skeleton and the [tui] config section (410ea0c)
- feat(app): T1.1 Terminal UI settings page (0e2361c)
- feat(tui): T1.3 key adapter and legacy-terminal keymap overlay (224e612)
- feat(fs): console support for the terminal UI (692ea3f)
- feat(tui): T1.2 TuiHost, terminal guard and panic safety (33c4a11)
- feat(tui): T1.4 / T1.5 frame loop, minimal view and soyuz --tui (40cd267)
- feat(tui): T2.9 mouse (a9416ae)
- feat(config): T2.1 [tui] colour mode, icons, borders and sidebar width (16a5bd3)
- feat(tui): T2.7 command palette and go-to-folder (ef8c87c)
- feat(tui): T2.8 status bar, filter, toasts and border styles (e31c3c3)
- feat(tui): T2.4 tab strip, dual and quad panes (73fd8ea)
- feat(tui): T2.6 sidebar and folder tree (1f5a969)
- feat(tui): T2.3 details, list and Miller pane views (7dc3bc6)
- feat(tui): T2.5 breadcrumb and address bar with completion (4967f93)
- feat(tui): T2.2 design tokens in appcore, TUI colour themes and icon sets (e3967cb)
- feat(tui): T2.10 integration (5e6263f)
- feat(tui): soyuz-tui console binary runs the terminal UI in the launching terminal (c92bb4a)
- feat(tui): stand-in keys inside Windows Terminal; a paste in the file list pastes (4425b8e)
- feat(tui): T3.0+T4.0 shared skeleton (dialog stack, menu/form widgets, panel regions, [tui] keys, new actions) (4b45295)
- feat(tui): T4.2 terminal graphics (uploader store, protocol choice, encode cache, overlay repaint, colour-mode block images) (83bd756)
- feat(tui): T3.2/T3.7 confirm, pattern, columns, open-with, recovery and crash-report dialogs (rules in appcore models::requests) (a8a8e10)
- feat(tui): T3.3 jobs and history panel on a core row model (models::jobs_rows) (abba03e)
- feat(tui): T3.8/T3.9 OSC 52 clipboard policy and context menu from a core catalogue (models::context_menu) (af4de31)
- feat(tui): T3.1 core prompt dialogs (conflict, error, delete, quit, copy/move to, new link, MCP and script approval) (7f18982)
- feat(tui): T4.1/T4.3 preview pane (text, code, markdown, hex, archives, diff, info); preview loader/hex/info moved to appcore (cff380d)
- feat(tui): T4.7 text and hex editors (wide-char layout, find/replace/go-to, paste bursts, unsaved-changes dialog) (14b0325)
- feat(tui): T3.4-T3.6 inline rename, batch rename, attributes and properties (models::{batch_rename,attrs_edit,folder_stats}) (25c9bf4)
- feat(tui): T4.6 Quick Look overlay (c25d5ae)
- feat(tui): T4.4 image previews (pictures, SVG, PDF page, video poster, audio/e-book cover, fonts, 3D still; preview_raster in appcore) (a505afb)
- feat(tui): T4.7 rest: image editor, editor chords as keymap entries (KeyContext::Editor), external-change banner, FocusEditor (c803fdd)
- feat(tui): T4.5 grid and gallery views with thumbnails; fix(appcore): Shift+Left/Right extends the selection (8f853dd)
- feat(tui): T3 gaps: links dialog, sidebar/tab/header menus from the catalogue, jobs toolbar and job menu, all batch-rename rules, checksums in Properties, MCP countdown (ceeeac7)
- feat(tui): T3/T4 integration: remove dead fallbacks and strings, grid wheel step, preview keys scroll Quick Look, e2e test, system-clipboard switch for tests; docs: user guide (e3849ff)
- feat(tui): T5/T6 skeleton: [tui] shell_mode, terminal_rows, ai_ratio, editor; suspend API (FrontendCmd::Suspend); focus kinds, regions, hit targets, dialog kinds and module stubs for terminal, search, AI, compare, settings, location dialogs; per-frame tool queue drain; debounced config save pumped by the core; unsupported.rs removed (85e82e1)
- feat(appcore): AI panel logic in the core (ai::panel: PanelAct, apply, card edits incl. JSON args validated against the tool schema, mentions, Esc rule; ai::diff plan trees); GUI AI panel uses it (435509b)
- feat(appcore): folder compare in the core (models::compare: CompareModel state machine, two-way conflicts; compare_profiles: sync profile CRUD, schedules, reports, scheduler; compare_state); GUI compare and sync windows draw it (8948b88)
- feat(tui): suspend to shell / command / $EDITOR: input reader gate, Ctrl+C handled by the existing console handler while suspended, SIGINT/SIGQUIT dropped on unix, editor resolution without a GUI fallback, config reload after editing (2fde833)
- feat(core): config schema metadata at runtime (config::meta, embedded sources; docs_gen uses it); appcore settings_schema / settings_draft / settings_keys models (typed rows, table editing, validation by deserializing Config, credential-store secrets, keybinding rows and recorder rules) (5375d91)
- feat(term): toolkit-free terminal API (grid snapshots with resolved colours, links, selection; key / mouse / paste / focus encoding; modes, focus, mouse reports); shared colour resolver for the egui renderer; core terminal_palette from design tokens (0e7441a)
- feat(tui): AI assistant panel (chat, plan cards with approve / reject / approve kind / edit names / undo / save recipe, cost question, recipes and audit tabs, slash and @mention popups) and MCP approval cards in panel mode (e4c0f04)
- feat(tui): settings editor generated from the config schema (live apply, search, typed editors, list editor, reset row / section, restart tags, Ctrl+E to $EDITOR) and keybindings page (conflict chips, recorder, filters, presets) (a860c9e)
- feat(tui): embedded terminal pane (session strip, OSC 52 prompt bar, grid with colour-mode mapping, keys and passthrough, selection, mouse reporting, links; Alt+T focuses) (26b9f9c)
- feat(tui): folder compare screen (setup with folder completion, scan with cancel, review with category chips and per-row actions, plan, Sync job, two-way conflict dialog, read-only profiles page with load and run) (abacb36)
- feat(tui): table editor in TUI settings (bookmarks, commands, AI profiles, MCP clients and servers, automation rules, ...: add, duplicate, remove, reorder, per-element forms) and Set secret (OS credential store, only the reference in config.toml) (ecc129a)
- feat(tui): search bar (query and glob fields, nine pills, run / save / close, status row with cancel, relevance, smart folder and refine; save-search and size dialogs); core search status_line shared with the GUI (2570037)
- feat(tui): smart folder, archive (password, extract to, compress, rename entry) and remote (connect, host key, name) dialogs; sidebar network state dots, connect and new-smart rows, menus for connections, smart folders and saved searches; smart folder status segment (dc2e7ab)
- feat(tui): sync profiles fully editable and schedulable in the compare screen (new, edit, rename, delete, save setup as profile, run, dry run, cancel; schedule fields; run reports loaded off the UI thread; redraw while busy) (1787a29)
- feat(tui): polish: toasts below the search bar, search bar hints, sidebar middle-click opens a new tab, Extract-to folder completion, MCP client Generate token, terminal paste from the Windows clipboard, click places the cursor in fields; GUI smart-folder and saved-search menus from the core catalogue (721e90a)
- feat(tui): AI composer is multi-line (Alt+Enter, Ctrl+J, Shift+Enter, paste); JSON plan editing in a dialog and in $EDITOR; smart search card, model / access / tools chips and menus, budget meter; Esc stops a task while the panel is open; Ctrl+I focuses before it closes (c0251e6)
- feat(tui): T7 skeleton: routing, dialog stubs (workspaces, tab history, About, shelf, split / join, unlock, versions), screens host, hit targets, core model stubs; read-only mount offer as a core Confirm; safe-mode relaunch works from soyuz-tui.exe (21ef5ef)
- feat(tui): device rows and menus in the sidebar (mount, unlock, lock, eject, hide), unlock dialog with a zeroized passphrase, previous versions dialog (open, compare, copy to, restore; admin rights as a message only); core DeskBackend seam, device menus and versions model shared with the GUI (14da7f6)
- feat(tui): treemap and duplicates as full-screen tool screens (squarified for the cell aspect, ASCII / mono fills, size breakpoints; keep-one rules, trash through an undoable job); core TreemapModel and DupesModel shared with the GUI (0ff4dd4)
- feat(tui): save and manage workspaces (palette action Manage workspaces), tab history tree, About, shelf dialog, split / join dialog; core split model (disk checks off the UI thread), About facts and history rows shared with the GUI (92f0aa5)
- feat(tui): hide Welcome in the TUI palette (actions::gui_only); folder completion for the split / join output; clickable keep chips in duplicate group headers (and the first-groups click offset fix); shared ascii_text for two-cell signs; GUI device menus drawn from the core builders (720bc15)
- feat(tui): Trash page (restore as an undoable job, delete permanently, empty, row menu) and Computer / This PC page (drives with usage bars, open, drive menu); core trash summary / menu and usage levels shared with the GUI (801ad93)
- feat(app): notification time setting ([appearance] toast_time_scale, default 0.5: toasts expire twice as fast) (ac72840)
- feat(tui): T8 skeleton: [tui] screen_reader + ToggleScreenReader, core SOYUZ_SCRIPT runner API, snapshot / shot / a11y / devscript stubs, screen-reader caret, test scaffolding (1b4c776)
- feat(appcore): SOYUZ_SCRIPT runner in the core (parser, core steps, expect / wait_for results, screen~ checks); GUI devscript is a thin FrontStep adapter (25a84e0)
- feat(tui): screen-reader mode: one column of plain lines, no box drawing or icons, focused pane only with an Other pane line, hardware caret on the focused element, status sentence led by the focused item and rewritten whole, toasts in the status row (75bfb35)
- feat(tui): screen-reader snapshots and polish (progress spoken per 10 %, words for sort and eject marks, one Miller / settings column, form labels with colons, no terminal images); fix: wide characters cut by an overlay no longer hide its border (dfbd691)
- feat(tui): SOYUZ_SCRIPT in the terminal UI (keys, shot as .txt / .snap / .svg, size), SOYUZ_TUI_HEADLESS debug mode, --screen-reader run-only flag, keyboard tour en / tr x Defaults / Legacy, binary smoke test (366e384)

### Fixes and security

- fix(tui): Quick Look asks the loader for terminal pictures (rasterized SVG / 3D, EXIF) (a016c0f)
- fix(app): native submenus open after a short rest on their row and close after a short delay, so passing over a submenu row no longer flashes a window; devscript native:hover drives the pointer inside native popups (48ab994)
- fix(tui): load and save colour tags, git status, recent files and folder colours in the terminal UI (their pumps move into the core's pump_services); tag dots after names and git-status name colours in TUI rows (77e7036)
- fix(appcore): the sync-profile scheduler ticks in the core's per-frame pump, so scheduled profiles run in the terminal UI too (7a9694b)
- fix(tui): settings edits made in dialogs over the settings editor (tables, lists, shortcut recorder) apply in the same frame (d3476b1)
- fix(appcore): recompute a pane's visible rows as soon as its entries change (a patch or relist between frames left stale indices; selected_entries panicked) (08ec79c)
- fix(archive): ruzstd 0.9.1 (RUSTSEC-2026-0344, zstd decompression bomb) (5c04519)
- fix(fs): atomic writes on Windows wait briefly for a reader to close the file instead of dropping the save (store files, config; also the flaky headless workspaces test) (79f128e)
- fix(tui): narrow details panes drop columns before names shrink to a stub (name floor a third of the list, 12 to 18 cells) (919d080)
- fix(appcore): a job cancelled before it did anything no longer shows "0 B of 48 MiB" (537d680)

### Documentation

- docs: Linux and macOS are fully supported (verified on real machines) (262bffc)
- docs(site): sharper product copy, a highlights strip, all three platforms supported (c4fbdab)
- docs: terminal UI preview section in the user guide (4f579d7)
- docs: Trash and This PC pages in the TUI section of the user guide (8814544)
- docs: Terminal UI column in KEYBOARD.md, legacy / Windows Terminal chord table, TUI differences in keyboard.html (two A4 pages) (1ddb9bd)
- docs: Terminal UI chapter (no longer a preview) with screen-reader mode, scripted runs and screenshots; accessibility checklist for the terminal UI; TUI screenshots (scripts/tui-screenshots.ps1, headless SVGs) (5b2eab0)
- docs(site): terminal UI feature with its screenshot (e69af42)
- docs(site): terminal UI section (everything the window does, keyboard and mouse on any terminal, screen-reader mode) with three screenshots, Terminal in the nav, page descriptions mention the terminal; TUI shots regenerated after the column fix (a8288bc)

### Other

- refactor(app): T0.1 Host trait; services take a Wake instead of the egui context (4fc97ab)
- refactor(app): T0.2 dispatch takes &dyn Host instead of the egui context (19274ad)
- refactor(app): T0.3 keyboard input as a toolkit-free InputFrame (5161d47)
- refactor(app): T0.4a plain geometry, layout and theme preference in core state (a60434c)
- refactor(term): T0.10 toolkit-free terminal backend, egui widget behind a feature (9d0875d)
- refactor(app): T0.5 toasts model moves to models::toasts (45dd006)
- refactor(app): T0.5 settings model and the frontend outbox (53526dd)
- refactor(app): T0.5 workspaces and windows models (55e795b)
- refactor(app): T0.5 folder tree, sidebar, portal and devices models (7bff49f)
- refactor(app): T0.5 search, smart folders, remote and archive models (d54852d)
- refactor(app): T0.8a palette, drag and drop and inline rename drawing move to ui/ (734138d)
- refactor(app): T0.5 panels, terminal sessions, properties, tools, quick look and trash models (300388e)
- refactor(app): T0.7 previews and thumbnails without egui types (984bbbb)
- refactor(app): T0.4b semantic icon ids and neutral colours in core (3ee5311)
- refactor(app): T0.8b editors split into document model and GUI view; key conversion, menus, touch marks to ui/ (1ac34c6)
- refactor(app): update_config callers pass the host directly (4e379d6)
- refactor(app): T0.6 dialog models (d0f2b3c)
- refactor(app): T0.9 split SoyuzApp into AppCore and GuiState (4185e66)
- refactor: T0.11 extract the frontend-agnostic core into crates/soyuz-appcore (7ff457b)
- chore(fuzz): refresh Cargo.lock (76c96b3)
- refactor(tui): T2.0 skeleton for browsing parity (d018e1f)
- test(tui): T5/T6 end-to-end run (search, archive, terminal, AI, settings, table edit, $EDITOR, compare) and tiny-size cases for every new region and overlay; docs: user guide and README for the terminal UI (35a1149)
- test(tui): tiny-size AI case uses the composer's chips row (a02679c)
- refactor(appcore): one safe_mode_args helper for the safe-mode relaunch (b959760)
- test(tui): T7 end-to-end run and tiny-size / ASCII / mono sweeps for every T7 dialog and screen; docs: user guide and README for the T7 features (a78337e)
- test(tui): snapshot tests (text grid + style mask + legend, SOYUZ_BLESS_SNAPSHOTS), shot formats .txt / .snap / .svg, 42 view snapshots and 6 app snapshots, CI upload of changed snapshots (d0c684a)
- test(tui): keyboard tour in screen-reader mode (fcabe86)
- test(app): keyboard-only GUI test waits for each check (wait_for:) instead of fixed sleeps, and for the new folder before quitting (2d94b3d)
- release: 0.3.0 (4a333fe)

## 0.2.0 (2026-10-11)

### Other features

- feat(packaging): Linux desktop entry, hicolor icons, AppStream metainfo, rpm spec, PREFIX tarball script, demo/screenshot scripts (17ea026)
- feat(ai,mcp): Secret Service credential store and Unix-socket local IPC (MCP attach, CLI, automation lock) on Linux (9772419)
- feat(index,term,core): inotify-limit polling fallback, Linux index excludes, [linux] config section, fs naming helper, OSC 7 parser, shell detection fix (1d5ad93)
- feat(fs): Linux core — mountinfo volume roots and dedupe, gvfs/sysfs removable, POSIX ACL view, inode flags, byte-safe temp names, .hidden, snapper/ZFS/Timeshift snapshots, idle I/O priority, inotify-limit polling, spec-checked trash topdirs (1ea4c6b)
- feat(fs): Linux desktop — shared-mime-info with magic, Desktop Entry launching and D-Bus activation, mimeapps resolution, icon themes, FileManager1 reveal, portal theme, fontconfig dirs scan, XDG user dirs, mimeapps default-file-manager with exact undo, freedesktop thumbnail cache, UDisks2 (917f854)
- feat(core): general.honour_dot_hidden and behaviour.watch_poll_interval_ms keys (17fd973)
- feat(app): Linux data wiring — XDG places, GTK bookmarks, UDisks2 mount/unmount/eject in the sidebar, freedesktop thumbnails, index rescan/excludes, watch-limit polling, .hidden, per-volume naming rules, credential calls off the UI thread; fix(jobs): trash test race (121a23e)
- feat(app): Linux settings and wording — Linux desktop group, default file manager via mimeapps + FileManager1 service opt-in, previous versions (Snapper/ZFS/Timeshift), inode-flag editor, Secret Service label, Windows-only rows hidden, Linux user guide; feat(core): tags.sync_xdg_tags, sidebar.gtk_bookmarks (2aa95bd)
- feat(app): Linux platform services — native Wayland (ext-data-control) and X11 file clipboard, FileManager1 D-Bus service, window backend choice, file:// and multi-path arguments, tarball self-update, user.xdg.tags, gvfs/rclone cloud places, fontconfig-free font catalog with coverage-based fallback, pdfium system paths, /etc/shells (b033207)
- feat(fs,app): Linux drag and drop — Wayland drop target on winit's display (own queue), X11 XDND drag source, Wayland drag source via own pointer serial; dragout::supported() (bfc3bdf)
- feat(app): Linux wiring — xdg tags display + sync key, GTK bookmarks key, pdfium via system loader, tarball Install & restart, FileManager1 status, per-volume naming in all name dialogs, Linux palette descriptions, rclone shown once, user guide fixes (3c548a4)
- feat(app,fs): unmounted and locked devices in the sidebar — NTFS/exFAT/etc. and BitLocker/LUKS from UDisks2, distinct styling, mount / read-only / unlock / lock / hide, Fast Startup guidance; sidebar.unmounted_devices and sidebar.hidden_devices keys (fe74e9f)
- feat(term): OSC 7 cwd tracking through a PTY tee (tty_tee.rs); shell-reported cwd wins over /proc polling (a01f32d)
- feat(fs,app): write Soyuz-made thumbnails to the freedesktop cache (appearance.write_os_thumbnails, Linux) (5067050)
- feat(macos): Keychain credentials, LaunchServices launching and reveal, CFPreferences theme/locale, CoreText font catalog, NSPasteboard file clipboard, Time Machine previous versions (untested on macOS) (86ec83c)
- feat(vfs,app): SMB2/3 provider off Windows (smb2 crate) with share enumeration; smb:// and \\server\share in the path bar (144bdf9)
- feat(a11y): Linux AT-SPI audit script; name the window, status-bar toggles, splitters and pane headers; tabs/toggles expose state (4cf16e3)
- feat: scripts/changelog.sh (bash port of changelog.ps1); fix(app): Computer view lists rclone mounts under Cloud only (f68eee7)
- feat(packaging): Flatpak manifest (io.github.ulaas.Soyuz) with sandbox-aware host paths, flatpak-spawn for terminals/Open With, host fonts, sandbox mount filtering (bf738c5)
- feat(app): show the git commit the binary is built from (83e1d73)

### Fixes and security

- fix(app): icon pack paths resolve the same on every OS (backslashes, drive prefixes) (4d3291c)
- fix(app): OLE drag-format test runs on Windows only (af03cc0)
- fix(app): drag-out setting follows runtime drag-out support (X11 / Wayland) (c5d67bf)
- fix(mcp): socket test tolerates descriptors briefly held by forked children (81db025)
- fix(term): drop the macOS proc_pidinfo cwd fallback (unsafe outside the allowlist; OSC 7 covers it) (bbf9d7b)
- fix(a11y): vendor accesskit_unix 0.21.1 with the IsEnabled fix so Orca sees Soyuz on at-spi2-core 2.62+ (6c675e9)
- fix(app): safe eject (never system drives; lock containers first), sequenced inode-flag apply with honest partial-failure state, device events drained every frame, unlock dialog secret hygiene, macOS previous versions, desktop Exec %U, audit cleanup (73eac02)
- fix(fs): shell-quote field codes in quoted Exec args, trash never hangs on dead network mounts, icon-theme inherit panic, symlinked mimeapps.list kept, exact desktop-file restore, partial inotify watches dropped on fallback, handle-based Wayland drop target, overflow-safe parsers, single-pass Open With placeholders, Windows file: URIs, long-name overwrite (5e0305e)
- fix(app): paste reads the OS clipboard off the UI thread with bounded waits, delta + serial xdg tag writes, restart only when the close proceeds, deferred font reinstall, single-pass verified update extract, single gvfs probe, clipboard owner/send edge cases (789c9a1)
- fix(app): remote URIs on the command line (desktop %U) open as remote locations (34a047b)
- fix(vfs,jobs,term,ai,mcp,index): hostile remote names never escape the tree, Windows OSC 7 drive paths only, /proc cwd tracking survives stray OSC 7, bounded keyring prompt wait, group kill before reaping, unavailable index roots keep their index (8c7d929)
- fix: restore a green main after dependabot bumps (78b75c7)
- fix(app): --version / --help never panic on a closed or missing stdout (fa0cb27)

### Documentation

- docs(site): download section lists the Linux x64 and macOS arm64 builds and notes that builds are unsigned (861c263)
- docs(site): present Soyuz as a Windows, Linux and macOS app across the page; Linux and macOS builds are tested (324b29f)
- docs: README lists Linux and macOS as tested platforms (38f0d6e)
- docs(app): drag-out support comment (683b596)
- docs: Linux drag-and-drop behaviour in the user guide (de90200)
- docs,app: SMB in the Linux user guide; quiet smb2 INFO logs (4cc10da)
- docs: regenerate changelog (414a4d6)
- docs: regenerate changelog for 0.2.0 (9bdb8ab)

### Other

- chore(deps): bump zstd from 0.13.3 to 0.14.0 (4186303)
- build: workspace deps for Linux desktop integration (zbus, x11rb, wayland) (5274623)
- build: Linux scaffolding for platform work (per-crate deps, sys/linux module dir, xdg_dirs/udisks stubs) (b8918bd)
- ci: Linux tests are a gate, banned-crate check covers the Linux graph, Linux package job uses package.sh (0030c2c)
- build(deps): bump the actions group across 1 directory with 6 updates (61d401b)
- build(deps): bump resvg from 0.45.1 to 0.48.1 (640b8ae)
- test(app): GUI tests use a private XDG_CACHE_HOME (829fb6f)
- Reword thumbcache comments to satisfy no-shell-APIs grep check (49907f3)
- Make package dry-run test platform-aware (79b7d0c)
- build(deps): bump base64 from 0.22.1 to 0.23.1 (ae749f0)
- build(deps): bump md-5 from 0.10.6 to 0.11.0 (1b56592)
- build(deps): bump lzma-rust2 from 0.16.5 to 0.21.0 (bea86e4)
- Keep keyboard-only dialogs in app window (e37f5f2)
- release: 0.2.0 (403da2a)

## 0.1.0 (2026-10-11)

### Milestones (M0–M13)

- M0: workspace skeleton, soyuz-core, frozen API surface, app shell (609284f)
- M6: soyuz-search — streaming search, folder sizes, size tree, duplicates, folder compare (e58d497)
- M2: soyuz-jobs — background operation engine, prompts, lanes, history and undo (503c758)
- M8: soyuz-term — embedded alacritty-based terminal widget for egui (e454462)
- M7: soyuz-archive — zip/tar.*/7z browse, extract, create, passwords, zip-slip safety (92a7c27)
- M1-M3: soyuz-fs — raw per-OS listing, copy engine, links, launch, volumes, watcher, hand-built trash (3bbedc4)
- M1/M4/M5: soyuz-app — browsing UI, views, thumbnails, tabs, dual pane, session restore (1fd67da)
- M6-M9 app: terminal panel, preview pane, Quick Look, recursive search UI, archive browsing (ac5a151)
- M2/M3 app: background file ops, activity panel, conflict/error dialogs, DnD, undo, trash view, portable fix (7b9cebf)
- M11: command palette, go-to-folder, keymap presets/chords, settings UI, config hot reload, custom commands (8d9be6e)
- M10: properties dialog, batch rename engine, bulk attributes, link dialogs, OS clipboard file interop (3ccd499)
- M12: treemap, folder compare/sync, duplicates, git status, tags, drop shelf, split/join, recent (fbd4a19)
- M13: accessibility, high-contrast/zoom/reduced motion, polish pass, toml_edit settings save, tags/shelf persistence, README/user guide, icon, packaging, licenses (0f33b06)

### Rigidity (R)

- R6: security threat model, cargo-deny config and bans, security/release workflows with optional signing (ab96850)
- R2/R3: fault injection, proptest + chaos tests, fuzz targets, jobs journal with crash recovery, crash reports, safe mode, config migrations (6423d2b)
- R4/R5/B1: UI-thread I/O guard + moves, byte-capped caches, benches, DX12 default startup, ntdll listing, Win11 corner fix, Mica/Acrylic backdrop, Snap Layouts (9740c92)
- R5/B5: CPU-rendered first frame while GPU initializes (~43 ms to real UI), Inter Variable with optical-size instances (2e0c785)

### Looks (B)

- B2-B4: theme library (5 themes, user theme files, system accent, follow OS), folder colors, icon badges, disk thumbnail cache, folder mosaics, video thumbs via ffmpeg, skeleton rows, spring tabs, spring-loaded folders (c32c342)

### Features (F)

- F1/B5: lazy folder tree sidebar with DnD and keyboard nav; system CJK/emoji fallback fonts; density preview (0c7be58)
- F9: RAR (pure-Rust rars), ISO9660/Joliet/RockRidge/UDF, CAB, deb/rpm, split 7z/zip, zip editing as undoable job (1b306a5)
- F4: built-in text/hex/image editors in a dockable editor panel (f2b807b)
- F3: rich previews — PDF (pdfium + lopdf fallback), Office docs, EPUB, fonts, 3D models, audio waveform + playback, video poster, two-file diff (7af76e2)
- F8: two-way sync with state + conflict UI, scheduled sync profiles with reports, Volume Shadow Copy previous versions + restore, verified-copy badges (9e2ec2c)
- F7: tab groups, saved workspaces, quad layout, synchronized browsing, in-process multi-window with tab tear-off, per-tab history graph, Timeline location (b3192ea)
- F5: soyuz-index (SQLite FTS index, USN journal fast path, query language), instant indexed search, index status, live smart folders (543737c)
- F6: soyuz-script — Rhai sandbox (actions, menus, columns, previewers), automation rules engine, soyuz CLI companion over named pipe (04f82c5)
- fonts: role-based fonts — Segoe UI / Segoe UI Semibold from the system, Consolas/Cascadia for terminal/code/hex, embedded Inter + JetBrains Mono fallbacks, Settings → Fonts (c2371ad)
- F2: soyuz-vfs — SFTP/WebDAV/FTP(S)/S3 providers, remote locations, connect dialog, known_hosts trust, resumable cross-provider transfer jobs, SMB share listing (ddc819e)
- F10: Fluent localization (en-US + complete tr-TR), live language switch, Turkish date/number/size formatting and collation, accessibility pass, keyboard-only test, ACCESSIBILITY.md (45f6842)
- fonts: embedded screen-optimized OFL fonts (Inter + Cascadia Mono defaults; Plex, Noto, JetBrains, Source Code Pro selectable), hinting/gamma settings, comparison sheets, terminal glyph-cache fix (c7971bb)

### AI (A)

- A1/A2: soyuz-ai crate — streaming providers (Anthropic, OpenAI chat/responses, OpenAI-compatible local, Gemini), credential store, tool catalog with scopes and dry-run plans, AI settings page (cdb3af7)
- A3/A4: soyuz-mcp — MCP server (stdio attach via named pipe, headless, localhost HTTP), client trust model and approvals, audit log, client config installers, MCP client for external servers (3685a0c)
- A5-A8: in-app AI agent harness — panel, streaming chat, plan cards, tree/rename diffs, pane highlights, recipes, per-folder AI permissions, redaction, injection taint, budgets, audit viewer, fake-provider harness tests and eval suite (39b3d0b)
- AI unification: shared audit with source filter, MCP approvals as plan cards, MCP job tagging/highlights/undo-all, external MCP tools in assistant, treemap highlights, content/vision-based name suggestions (0ba04cc)

### Distribution (D)

- D1: WiX v5 per-user MSI, MSIX manifest, portable zip + checksums, opt-in default file manager with restore, manual update check with checksum + signature verification, release workflow (5d2c4d1)
- D2: demo fixture + screenshot gallery, generated keyboard/config references with staleness tests, changelog, localized drive labels, roadmap status, README refresh (407e66d)

### Other features

- feat(app): custom themed titlebar with logo, integrated tabs and window controls; settings buttons (8ec555c)
- feat(app): searchable themed dropdown widget for all settings choices (fonts, theme, accent, shell, AI models, formats, presets), GPU backend setting (a467e9b)
- feat(app): enable sub-pixel glyph positioning by default (268274e)
- feat(app): dialogs as owned native windows; context menus/dropdowns overflow into native popup windows; native_dialogs setting (4c5811b)
- feat: drag files out to other apps and the desktop via OLE DoDragDrop (hand-written IDataObject/IDropSource, no shell) (49fd2ec)
- feat(app): move the default file manager opt-in to the top of Settings → General (e61e530)
- feat(app): drag result rows out of Compare, Find duplicates and Previous versions (onto panes, folders, tabs, sidebar, or out of the window); devscript move: step for synthetic drags (53e35d7)
- feat(app): borderless toolbar rail (subtle fill on hover/press/focus), vertically centered filter/path text, borderless engraved tabs with italic separators (29c15fb)
- feat(app): rename color tags ([tags] names; sidebar inline rename, Settings → Views & layout → Tag names) (63c0cc5)
- feat(app): Portal sidebar section (drop folders to pin), new section order, full-height Disk tree with sidebar icon bar; config schema 2 migration (d57ef58)

### Fixes and security

- fix(core): no panic parsing paths with multibyte chars near prefix boundary (crash loop on session restore); docs: review 2 (6d4e387)
- fix: data-safety fixes (undo identity stamps, rename failed deps, dupes aliasing, link loops, trash serialization, no rundll32) (6500d4c)
- fix: hardening fixes (git hardening, terminal control chars, command quoting, preview hardening, store backups, close confirmation, open-with, new window, flat view, settings wired) (3fac50e)
- security: OSC 52 consent, strip special bits, decompression-bomb limits, reveal hidden bidi chars in names, pin toolchain (311b1bc)
- fix(app): draw the terminal OSC 52 clipboard consent prompt; add policy/preview tests (e095fe9)
- fix(app): empty-state classifier (protected system files ≠ hidden), Name column fills/never freezes wide, slick constant-height toasts with hover details (c86ee3e)
- fix: watcher dropped events under load (debouncer file-id cache) — no cache, re-arm silent watches, relist panes after jobs, manual refresh always relists (0fa3443)
- fix(app): remove startup placeholder and CPU first frame; maximize hidden window before first show — single window launch (c5afed1)
- fix(app): Quick Look and tool windows are modal — panes beneath get no clicks, hover, wheel or shortcuts; Esc closes the frontmost tool window (a300a1b)
- fix(app): tool-window backdrop lets drags through to panes, folders, tabs and the sidebar; clicks stay blocked (8ad4c26)
- fix: Linux/macOS builds — Clone-able never-constructed pipe stream stub off Windows, Windows-only test helpers/imports gated (clippy clean for x86_64-linux and aarch64-macos) (894d2ba)

### Documentation

- docs: project website (site/) for GitHub Pages (92416c9)
- docs: README for the public release (db97961)
- docs: drop references to internal planning documents and milestone tags from comments and docs (f336626)
- docs: privacy-safe screenshots for the website and README (demo tree, SOYUZ_DEMO_MASK) (758676d)
- docs: regenerate changelog (f2d3c95)

### Other

- chore: ignore local worktrees (4114740)
- closeout: MCP fuzz targets, PDF thumbnails, icon packs, startup adapter picker + timing marks, keyboard-only GUI test in CI, roadmap status (071138b)
- partials: UIA accessibility audit + fixes, signing pipeline validated (self-signed, removed), startup placeholder window, Windows thumbcache reader, RTL deferred (addcb08)
- ci: package Windows/Linux/macOS binaries (zip / tar.gz) on main and tags; install libasound2-dev on Linux; Linux/macOS tests non-gating until unix verification is lifted (d9cce28)
- chore: MIT license (single LICENSE), THIRD-PARTY-NOTICES.md generated by cargo-about (scripts/notices.ps1) and shipped in zip/MSI/MSIX; SOYUZ_DEMO_MASK hides volume labels/sizes/cloud for screenshots (b4af861)
- ci: pin Actions to commit SHAs, no persisted checkout credentials, gitleaks secret-scan job, Pages deploy workflow, Dependabot, CODEOWNERS, .github/SECURITY.md; scripts/public-check.ps1 pre-publish privacy check (a8708d6)
- everything freezes under right circumstances (5cddb8e)
- ci: release workflow drafts a GitHub Release with all artifacts; update check defaults to the official repo (d0d7546)
- ci: drop a stale doc reference (85578c4)
