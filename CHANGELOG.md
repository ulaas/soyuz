# Changelog

All notable changes to Soyuz. Grouped by roadmap phase (see docs/ROADMAP.md): M = the
original milestones (docs/PLAN.md), R = rigidity, B = looks, F = features, A = AI,
D = distribution. Generated from `git log` by `scripts/changelog.ps1` (or `git cliff`
with `cliff.toml`); commit hashes refer to the main branch.

## Unreleased (0.1.0, last change 2026-10-08)

### Milestones (M0–M13)

- M0: workspace skeleton, soyuz-core, frozen API surface, app shell (7b5fd10)
- M6: soyuz-search — streaming search, folder sizes, size tree, duplicates, folder compare (0c9e4f8)
- M2: soyuz-jobs — background operation engine, prompts, lanes, history and undo (7b5dac9)
- M8: soyuz-term — embedded alacritty-based terminal widget for egui (c556a39)
- M7: soyuz-archive — zip/tar.*/7z browse, extract, create, passwords, zip-slip safety (688656e)
- M1-M3: soyuz-fs — raw per-OS listing, copy engine, links, launch, volumes, watcher, hand-built trash (d900ba6)
- M1/M4/M5: soyuz-app — browsing UI, views, thumbnails, tabs, dual pane, session restore (bcd6b9f)
- M6-M9 app: terminal panel, preview pane, Quick Look, recursive search UI, archive browsing (1ad87a4)
- M2/M3 app: background file ops, activity panel, conflict/error dialogs, DnD, undo, trash view, portable fix (44338fb)
- M11: command palette, go-to-folder, keymap presets/chords, settings UI, config hot reload, custom commands (f5cf4f2)
- M10: properties dialog, batch rename engine, bulk attributes, link dialogs, OS clipboard file interop (80404a6)
- M12: treemap, folder compare/sync, duplicates, git status, tags, drop shelf, split/join, recent (7502e12)
- M13: accessibility, high-contrast/zoom/reduced motion, polish pass, toml_edit settings save, tags/shelf persistence, README/user guide, icon, packaging, licenses (af30789)

### Rigidity (R)

- R6: security threat model, cargo-deny config and bans, security/release workflows with optional signing (3700fbc)
- R2/R3: fault injection, proptest + chaos tests, fuzz targets, jobs journal with crash recovery, crash reports, safe mode, config migrations (57c43b6)
- R4/R5/B1: UI-thread I/O guard + moves, byte-capped caches, benches, DX12 default startup, ntdll listing, Win11 corner fix, Mica/Acrylic backdrop, Snap Layouts (7c7324a)

### Looks (B)

- B2-B4: theme library (5 themes, user theme files, system accent, follow OS), folder colors, icon badges, disk thumbnail cache, folder mosaics, video thumbs via ffmpeg, skeleton rows, spring tabs, spring-loaded folders (09f8540)

### Features (F)

- F1/B5: lazy folder tree sidebar with DnD and keyboard nav; system CJK/emoji fallback fonts; density preview (bfa06f7)
- F9: RAR (pure-Rust rars), ISO9660/Joliet/RockRidge/UDF, CAB, deb/rpm, split 7z/zip, zip editing as undoable job (64568ad)
- F4: built-in text/hex/image editors in a dockable editor panel (038ae93)
- F3: rich previews — PDF (pdfium + lopdf fallback), Office docs, EPUB, fonts, 3D models, audio waveform + playback, video poster, two-file diff (bf4802d)
- F8: two-way sync with state + conflict UI, scheduled sync profiles with reports, Volume Shadow Copy previous versions + restore, verified-copy badges (9354ec9)
- F7: tab groups, saved workspaces, quad layout, synchronized browsing, in-process multi-window with tab tear-off, per-tab history graph, Timeline location (0ee8656)
- F5: soyuz-index (SQLite FTS index, USN journal fast path, query language), instant indexed search, index status, live smart folders (4980b52)
- F6: soyuz-script — Rhai sandbox (actions, menus, columns, previewers), automation rules engine, soyuz CLI companion over named pipe (a2d52a2)
- fonts: role-based fonts — Segoe UI / Segoe UI Semibold from the system, Consolas/Cascadia for terminal/code/hex, embedded Inter + JetBrains Mono fallbacks, Settings → Fonts (4904a48)
- F2: soyuz-vfs — SFTP/WebDAV/FTP(S)/S3 providers, remote locations, connect dialog, known_hosts trust, resumable cross-provider transfer jobs, SMB share listing (f7b6430)
- F10: Fluent localization (en-US + complete tr-TR), live language switch, Turkish date/number/size formatting and collation, accessibility pass, keyboard-only test, ACCESSIBILITY.md (7da3d7e)

### AI (A)

- A1/A2: soyuz-ai crate — streaming providers (Anthropic, OpenAI chat/responses, OpenAI-compatible local, Gemini), credential store, tool catalog with scopes and dry-run plans, AI settings page (49b18e2)
- A3/A4: soyuz-mcp — MCP server (stdio attach via named pipe, headless, localhost HTTP), client trust model and approvals, audit log, client config installers, MCP client for external servers (88b24d6)
- A5-A8: in-app AI agent harness — panel, streaming chat, plan cards, tree/rename diffs, pane highlights, recipes, per-folder AI permissions, redaction, injection taint, budgets, audit viewer, fake-provider harness tests and eval suite (43c10c6)
- AI unification: shared audit with source filter, MCP approvals as plan cards, MCP job tagging/highlights/undo-all, external MCP tools in assistant, treemap highlights, content/vision-based name suggestions (b9c18f3)

### Distribution (D)

- D1: WiX v5 per-user MSI, MSIX manifest, portable zip + checksums, opt-in default file manager with restore, manual update check with checksum + signature verification, release workflow (488e424)

### Other features

- feat(app): custom themed titlebar with logo, integrated tabs and window controls; settings buttons (a907f3c)

### Fixes and security

- fix(core): no panic parsing paths with multibyte chars near prefix boundary (crash loop on session restore); docs: review 2 (9762293)
- fix: review-1 data-safety findings (undo identity stamps, rename failed deps, dupes aliasing, link loops, trash serialization, no rundll32) (ea9bd0e)
- fix: review-2 findings (git hardening, terminal control chars, command quoting, preview hardening, store backups, close confirmation, open-with, new window, flat view, settings wired) (8014b59)
- security: OSC 52 consent, strip special bits, decompression-bomb limits, reveal hidden bidi chars in names, pin toolchain (4798fba)
- fix(app): draw the terminal OSC 52 clipboard consent prompt; add policy/preview tests (8d00d54)

### Documentation

- docs: research notes and implementation plan (4a55570)
- docs: integration notes for search and jobs (688f908)
- docs: terminal integration notes (883f8bc)
- docs: resolve PLAN.md merge markers (term + archive notes) (ccb87df)
- docs: fs integration notes (c4ac624)
- docs: review 1 findings (fs/jobs data safety) (d54db63)
- docs: the project notes (rules, permissions, status) and ROADMAP.md (053cd69)
- docs: roadmap Phase A — AI providers, MCP server/client, in-app agent harness with visual plans (6e7b7b6)
- docs: defer Linux/macOS builds, CI and tests (owner decision) (c86ec4b)
- docs: allow unsafe in soyuz-ai Windows credential module (bdcdd45)
- docs: forbid synthetic mouse/keyboard input in agent testing (cff7c6f)

### Other

- chore: ignore .claude worktrees (1a824b9)
