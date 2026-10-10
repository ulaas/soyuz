# Changelog

All notable changes to Soyuz. Grouped by the commit prefix: M = the
original milestones, R = rigidity, B = looks, F = features, A = AI,
D = distribution. Generated from `git log` by `scripts/changelog.ps1` (or `git cliff`
with `cliff.toml`); commit hashes refer to the main branch.

## Unreleased (0.1.0, last change 2026-10-11)

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

### Other

- chore: ignore local worktrees (4114740)
- closeout: MCP fuzz targets, PDF thumbnails, icon packs, startup adapter picker + timing marks, keyboard-only GUI test in CI, roadmap status (071138b)
- partials: UIA accessibility audit + fixes, signing pipeline validated (self-signed, removed), startup placeholder window, Windows thumbcache reader, RTL deferred (addcb08)
- ci: package Windows/Linux/macOS binaries (zip / tar.gz) on main and tags; install libasound2-dev on Linux; Linux/macOS tests non-gating until unix verification is lifted (d9cce28)
- chore: MIT license (single LICENSE), THIRD-PARTY-NOTICES.md generated by cargo-about (scripts/notices.ps1) and shipped in zip/MSI/MSIX; SOYUZ_DEMO_MASK hides volume labels/sizes/cloud for screenshots (b4af861)
- ci: pin Actions to commit SHAs, no persisted checkout credentials, gitleaks secret-scan job, Pages deploy workflow, Dependabot, CODEOWNERS, .github/SECURITY.md; scripts/public-check.ps1 pre-publish privacy check (a8708d6)
