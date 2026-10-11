# Changelog

All notable changes to Soyuz. Grouped by the commit prefix: M = the
original milestones, R = rigidity, B = looks, F = features, A = AI,
D = distribution. Generated from `git log` by `scripts/changelog.ps1` (or `git cliff`
with `cliff.toml`); commit hashes refer to the main branch.

## Unreleased

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

### Documentation

- docs(site): download section lists the Linux x64 and macOS arm64 builds and notes that builds are unsigned (861c263)
- docs(site): present Soyuz as a Windows, Linux and macOS app across the page; Linux and macOS builds are tested (324b29f)
- docs: README lists Linux and macOS as tested platforms (38f0d6e)
- docs(app): drag-out support comment (683b596)
- docs: Linux drag-and-drop behaviour in the user guide (de90200)
- docs,app: SMB in the Linux user guide; quiet smb2 INFO logs (4cc10da)

### Other

- build: workspace deps for Linux desktop integration (zbus, x11rb, wayland) (5274623)
- build: Linux scaffolding for platform work (per-crate deps, sys/linux module dir, xdg_dirs/udisks stubs) (b8918bd)
- ci: Linux tests are a gate, banned-crate check covers the Linux graph, Linux package job uses package.sh (0030c2c)
- test(app): GUI tests use a private XDG_CACHE_HOME (829fb6f)

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
- fix(app): empty-state classifier (protected system files ≠ hidden), Name column fills wide, slick constant-height toasts with hover details (c86ee3e)
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
