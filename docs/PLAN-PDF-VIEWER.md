# PDF engine (hayro) + embedded PDF viewer — plan v2

Status: approved for implementation (2026-11-25). v1 was drafted by a planning agent and
reviewed against the repo and the hayro 0.9.0 sources; this v2 folds in every review correction.

## Owner decisions (fixed)
- **hayro 0.9** (pure Rust, MIT OR Apache-2.0) is the default PDF engine, always built in.
- **pdfium stays optional**: `[preview] pdf_engine = "pdfium"` uses it when the library loads (else hayro).
  pdfium serves the preview pane, Quick Look, thumbnails and document text only; the viewer is hayro-only in v1.
- Scope: engine swap **and** a full viewer.
- **F4 on a PDF opens the viewer**; "Open as hex" moves to the viewer tab's menu.
- Out-of-process rendering is **deferred** (in-process guards only).
- **System-font fallback** for non-embedded fonts is **in** (`[pdf] system_fonts`).

## 1. hayro facts (verified; probe crate compiled and ran)
- `hayro_syntax::Pdf` and `Page<'static>` are **Send + Sync**. `RenderCache<'a>` is `!Send` (Rc/RefCell) and borrows the Pdf.
- `Pdf::new_with_password(data: impl Into<PdfData>, pw: &str) -> Result<Pdf, LoadPdfError>`;
  `LoadPdfError::{Decryption(DecryptionError), Invalid}`; `DecryptionError::{MissingIDEntry, PasswordProtected, InvalidEncryption, UnsupportedAlgorithm}`.
  `PasswordProtected` = none or wrong → distinguish via `preview_pdf::password_for(path)`. Passwords are passed as UTF-8 bytes (non-ASCII on R2–R4 may fail; note it).
- `PdfData: From<Vec<u8>>`, `From<Arc<T: AsRef<[u8]> + Send + Sync>>`.
- `pdf.pages()`, `pdf.metadata()` (title/author/subject/keywords/creator/producer `Option<Box<[u8]>>`, dates), `pdf.version()`, `pdf.xref()`.
  `Page::{media_box, crop_box, intersected_crop_box, rotation, render_dimensions, initial_transform(invert_y), raw, xref}`.
- Raw objects (outline, links): `XRef::root_id/get::<T>`, `Dict::{get, get_ref, keys, entries}`, `Array::iter::<T>()`. Strings read via raw Dicts are **decrypted** (`object/string.rs:114`).
- `hayro::render(page, &RenderCache, &InterpreterSettings, &RenderSettings, &PixmapSettings) -> Pixmap` (u16 dims).
  **`PixmapSettings::default().bg_color` is TRANSPARENT → always pass WHITE.**
- `hayro::render_into(page, cache, &InterpreterSettings, &RenderSettings, &mut vello_cpu::RenderContext, Affine)`.
  Region/tile: `RenderContext::new(w, h)`, transform `translate(-ox,-oy) * scale(s) * user_rot * page.initial_transform(true).to_kurbo()`
  — verified pixel-exact vs the full render on a `/Rotate 90` page. User rotation needs a translation to stay in positive coords.
  Finish with `ctx.flush(); ctx.render_with(&mut pixmap, &mut Resources::default(), RasterizerSettings { target_init: TargetInit::Clear(WHITE), .. })`.
- `Pixmap::data_as_u8_slice()` = premultiplied RGBA8 (equal to straight on opaque white).
- hayro re-exports `kurbo`, `vello_cpu`, `hayro_interpret`, `hayro_syntax` → only one new dependency (`hayro`).
- Each region render re-interprets the whole page → use **large regions** (≥ 1024 px tiles, or one viewport-clipped region per visible page).
- Text: custom `hayro_interpret::Device`; `draw_glyph_run` → `PositionedGlyph::transform()`, `as_unicode() -> Option<BfString>`.
  `Glyph` has no `advance_width()` (only `OutlineGlyph` / `Type3Glyph`), and the glyph transform hides a `glyph_stretch`.
  **Build char boxes from origin to next origin in device space**; use the advance only for a run's last glyph. A text-only device does not decode images.
  Driver: `Context::new(initial_transform, bbox, &InterpreterCache::new(), page.xref(), settings)` + `interpret_page`; reference `hayro-interpret/examples/extract_html.rs`.
- Fonts: `InterpreterSettings.font_resolver` gets `FontQuery::{Standard, Fallback(f)}` (`character_collection`, `is_bold`, …) → system-font hook.
- Memory: `Page` caches its decoded content in a `OnceLock` inside `Pdf` → to bound memory, **recycle the `Pdf`** (and its `RenderCache`), e.g. every ~64 distinct pages per worker.
- Licences: all hayro deps already pass `deny.toml` (glifo already in Cargo.lock). Embedded data for THIRD-PARTY-NOTICES: 14 Foxit Type 1 fonts (BSD-3), Adobe CMaps (BSD-3), two ICC profiles (CC0). MSRV 1.92 ≤ workspace 1.95.
- No cancellation hook: a runaway render can only be abandoned.

## 2. Architecture

### 2.1 `crates/soyuz-appcore/src/services/pdf/` (new)
| File | Contents |
|---|---|
| `mod.rs` | `EngineKind {Hayro, Pdfium}`, `PdfSrc` (moved here, re-exported from `preview_pdf`), `PdfError {Password{wrong}, Invalid, TooLarge, TimedOut, Panicked, Stale, Other(String)}`, `DocInfo`, `PageGeom {w_pt, h_pt, rotation}`, `RenderOpts {annotations, invert, system_fonts}`, `Region {page, scale, rot_quarter, x, y, w, h}`, `Bitmap {w, h, rgba}`. One-shot API: `open_info`, `render_page_fit(src, engine, page, max_w, max_h, opts, stale)`, `page_text`, `document_text(src, engine, max_pages, max_chars, budget)`. |
| `hayro_backend.rs` | Only file importing hayro: `load`, `render_region`, `text(&Page) -> PageText`, metadata mapping. |
| `pdfium_backend.rs` | Today's `preview_pdf::pdfium` module moved (`#[cfg(feature = "pdf")]`), `pub use` shims left in `preview_pdf`. No region rendering (viewer is hayro-only). |
| `fonts.rs` | `FontQuery::Fallback` resolver over `soyuz_fs::os_fonts` (`installed`, `find_in`), CJK collection → known system families per OS, bold/italic; font bytes cached per process (`Arc`), lookup only on workers. |
| `text.rs` | `PageText {text, chars: Vec<CharBox {byte, quad: [f32; 8]}>, lines}` in page points; collector Device; line/space reconstruction; `find(&PageText, &Query {text, case, whole_word}) -> Vec<Match {chars, rects}>`; hit test; word/line ranges; `slice_text`. |
| `nav.rs` | Outline + links from raw objects: `Outline {title, dest, uri, children}`, `Dest {page, top, left}`, `Link {rect, target: Dest \| Uri}`; explicit + named dests; caps (64 levels, 20 000 nodes), cycle guard. |
| `guard.rs` | `run_guarded<T: Send>(budget, stale, f) -> Result<T, PdfError>`: own thread (16 MiB stack), `catch_unwind`, timeout → abandon. A panicked/abandoned worker recreates its `Pdf`. |
| `session.rs` | `PdfSession::open(src, password, cfg: SessionCfg {threads, timeout, cache_bytes, opts}, wake, up)`; `info()`, `geoms()`, `request(Job)`, `poll() -> Vec<Event>`, `set_generation(u64)`, `outline()`, `links(page)`. `Job::{Region(TileKey), Base(page), Thumb(page), Text(page), Find(Query)}` with priority. `Event::{Opened, Failed, Region, Base, Thumb, Text(page, Arc<PageText>), FindProgress, PageFailed}`. Workers: fixed `min(4, cores-1).max(1)`; each holds an `Arc<Pdf>`-free own `Pdf` + stack `RenderCache`, recycled every ~64 pages. Opening (file read, engine probe) happens on a worker, never the UI thread. |
| `tiles.rs` | `TileKey {page, scale_milli, rot, tx, ty, invert}` (tiles ≥ 1024 px), visible-set maths, `TexCache` byte-budget LRU (evicts farthest from viewport first). |
| `fixtures.rs` (cfg(test), `pub(crate)`) | lopdf-built PDFs: multi-page text, mixed page sizes, `/Rotate 90`, outline, URI + GoTo links, encrypted (RC4-128 via lopdf). |

`preview_pdf.rs` remains the facade (password store, `info_lopdf`, `extract_text`, `search_text`, `pdfium_dirs`, `pdfium_available`).

Pixel caps: region ≤ 2048², page bitmap ≤ 8192 per side. File size cap: `[preview] max_document_mib` (existing key, reused).

### 2.2 Engine selection
`pick(cfg_engine, dirs) -> EngineKind` runs on a worker (it probes the filesystem). Pdfium only when configured, feature `pdf` on, and the library is available; else Hayro.

### 2.3 Viewer data flow
F4 on a PDF → `editors::open_path` → `io::load` checks for `%PDF-` in the first 1 KiB **before** `looks_like_text` → `LoadResult::Pdf` → `DocKind::Pdf(Box<PdfDoc>)` holding a `PdfSession`.
Each frame the UI hands the model the viewport; the model computes layout, visible pages and wanted regions, diffs the cache, requests jobs. Workers render, upload via the `Uploader` alias (`services/preview_gfx.rs`, premultiplied), send an event, wake. `PdfDoc::pump()` moves textures into `TexCache`; the UI paints a base layer (≤ 1400 px long edge) plus sharp regions on top.
Priority: visible regions centre-out > visible bases > visible text > ±1 page prefetch > sidebar thumbnails > find sweep. Zoom/rotation bumps the generation; stale jobs are dropped at dequeue, stale results discarded unless still wanted. Text lazily per visible page (`Arc`, LRU ~200 pages); find sweeps the document in the background with progressive counts. Dark pages: CPU lightness inversion on the worker; flag is part of the key.

### 2.4 Existing consumers
- Preview pane / Quick Look: `preview_rich::pdf()` and `pdf_page_async` use `pdf::open_info` / `render_page_fit` with the picked engine (WHITE background). `PdfView.engine`; `no_render` only on real failure; pdfium card only when pdfium was chosen and failed. "Open in viewer" button uses the existing `PreviewAction::EditFile(PathBuf)`.
- TUI preview: consumes `PdfView.first` — no change.
- Thumbnails: `thumbs::set_pdf(on, pdfium_dir, engine)`; drop the `cfg!(feature = "pdf")` gate (`thumbs.rs:152`); `pdf_available()` true whenever thumbnails are on; render under the guard (5 s); `thumb_disk::cache_key_with_source(.., "hayro-0.9" | "pdfium")`.
- `search_text` / index: pdfium if chosen and loaded, else the hayro text device, lopdf last on error/timeout; 30 s per file.
- Config doc comments to fix: `schema.rs` `[preview] pdf` ("Needs cargo feature pdf" → what `pdf = false` now means), `pdf_thumbnails` ("through pdfium"), `settings-preview-pdf-desc` (en-US + tr-TR).

## 3. Configuration
| Key | Type | Default | Meaning |
|---|---|---|---|
| `[preview] pdf_engine` | `"hayro" \| "pdfium"` | `"hayro"` | pdfium = use it when it loads, else hayro (preview, Quick Look, thumbnails, document text) |
| `[pdf] default_zoom` | `fit_width \| fit_page \| actual` | `fit_width` | zoom of a newly opened document |
| `[pdf] sidebar` | `none \| thumbnails \| outline` | `none` | sidebar on open |
| `[pdf] sidebar_width` | f32 120–400 | 180 | |
| `[pdf] page_gap` | f32 0–64 | 12 | gap between pages |
| `[pdf] dark_pages` | `off \| invert \| follow_theme` | `off` | initial state; the toggle action is per tab (no config write) |
| `[pdf] annotations` | bool | true | draw annotation appearances |
| `[pdf] system_fonts` | bool | true | use installed fonts for non-embedded fonts (CJK etc.) |
| `[pdf] cache_mib` | u64 32–4096 | 256 | texture budget |
| `[pdf] render_timeout_secs` | u64 2–300 | 20 | per job |
| `[pdf] external_links` | `ask \| open \| off` | `ask` | http / https / mailto only |
| `[pdf] remember_position` | bool | true | page/zoom/rotation per file this session |
| `[open] pdf_in_viewer` | bool | false | Enter / Open on a PDF opens the viewer instead of the default app |

Cargo feature `pdf` = pdfium backend only. hayro is non-optional.

### 3.1 Keymap
New `KeyContext::Pdf`. `editors::commands::binding` gets the active document kind: for PDF documents it looks up `Pdf` first, then `Editor` (reused `EditorFind`, `EditorFindNext/Prev`, `EditorGoTo`, `EditorMatchCase`, `EditorWholeWord`, `EditorCloseDocument`, `EditorNext/PrevDocument`, `EditorMaximize`). Update both TUI call sites (`soyuz-tui/src/overlay.rs` ~560, ~588). `app.rs::key_context` is not involved.
**Unmodified-key Pdf bindings are skipped while a text field (find / page field) has focus** (pass a "field focused" flag).
`Action::is_pdf()` excludes the existing `Action::PdfPassword`.

| Action | Default (all presets) |
|---|---|
| `PdfZoomIn` / `PdfZoomOut` / `PdfZoomActual` | `+` / `-` / `0` (plus Ctrl+wheel); not Ctrl+= etc. (global UiZoom) |
| `PdfFitWidth` / `PdfFitPage` | `W` / `F` |
| `PdfNextPage` / `PdfPrevPage` | `N` / `P` |
| `PdfFirstPage` / `PdfLastPage` | `Ctrl+Home` / `Ctrl+End` |
| `PdfRotateCw` / `PdfRotateCcw` | `R` / `Shift+R` |
| `PdfToggleSidebar` | `S` |
| `PdfToggleDarkPages` | unbound |

Fixed keys like the text editor: arrows, PgUp/PgDn, Space/Shift+Space, Home/End, Ctrl+A, Ctrl+C, Esc. Copy runs through the host: add it to `commands::needs_host` (or route via the frontend's `EditInput::Copy`).
Also: `keys/conflicts.rs` (~86) exempts Pdf like Editor; TUI `overlay.rs::is_free` (~376) and ~683 treat Pdf like Editor; Pdf actions are `gui_only` until task G removes that; Windows Terminal blocks some chords (`WT_BLOCKED`) — check the defaults.

### 3.2 Settings UI
`ui/settings/pages.rs` `preview`: engine choice after `settings-preview-pdf`; pdfium folder row says "only used when the engine is pdfium"; new "PDF viewer" group with all `[pdf]` keys; `[open] pdf_in_viewer` on the Open page. `appearance.rs` (~305) drops "needs pdfium". `models/settings_schema.rs` HINTS ranges. A test like `tui_page_covers_every_key` (`pages.rs` ~2227) for every `[pdf]` key.

## 4. Viewer UI
Host: `DocKind::Pdf` tab in the editor panel (`ui/editors/panel.rs`), like the image editor (tabs, dock, maximize, session restore, multi-window via `models::owners`). Save hidden; "Open as hex" in the tab menu.
- Toolbar 38 px (`bg_surface1`, `border_subtle` line): sidebar toggle, prev/next, page field "12 / 340", zoom out, zoom dropdown (`ui/widgets/dropdown.rs`), zoom in, rotate, dark pages, find, overflow menu (copy, select all, open as hex, reveal). Toolbar segmented control: `ui/widgets.rs::segmented`.
- Find bar: the editor find-bar look (`ui/editors/widgets.rs`).
- Body: optional sidebar (segmented Thumbnails / Outline; thumbnails virtualized with an `accent` ring on the current page; outline tree with `ROW_COMPACT` rows; `ui::empty_state::EmptyState` when there is no outline) + canvas.
- Status bar 24 px: page size (mm), zoom, engine, selection length, warnings.
- Canvas: `bg_sunken`; white pages with the `tokens::shadow::POPUP` shadow; blurred base while regions load; spinner only without a base; a failed page shows a `locked_card`-style card with Retry.
- Interactions: wheel scroll; Ctrl+wheel / pinch zoom around the pointer (maths from `ui/editors/image.rs`); middle-drag pan; left-drag selects text (I-beam), double = word, triple = line, Shift+click extends; selection `accent` ~35 %, matches `warning` ~35 %, current match `accent` outline; links: pointer cursor, internal scroll to dest, external per `[pdf] external_links` via `models::terminal::open_web_link` (only http/https/mailto; never file:/Launch/JavaScript); copy via `Host::copy_text` (off the system clipboard under `SOYUZ_NO_SYSTEM_CLIPBOARD`); context menu: Copy, Select all, Copy page as text, Find selection, Go to page; rotation view-only, whole document; encrypted → locked card → `models::archive::ask_pdf_password`, reload on `password_generation()`; file watched, reload keeps the page.
- Accessibility: `a11y::container(Role::Document)`, one `a11y::item` per visible page "Page n of m" (page text as description when a11y is active), `a11y::button`/`toggle` in the toolbar, `a11y::tree_item` in the outline; every pointer action has a key; focus ids registered in the panel; instant zoom under reduced motion; palette colours only.
- Devscript: `edit:<file.pdf>`; `editor:pdf=<page=N|zoom=150|fit=width|fit=page|rotate=cw|find=text|next|sidebar=outline|select=all|copy>`.

## 5. Work breakdown
Every agent: **no git write commands, the orchestrator commits**; shared central files (`actions.rs`, `ops.rs`, `app.rs`, `cmd.rs`, `ui/panels/mod.rs`, `shell.rs`) minimal and additive. Each branch must pass `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check` and the no-shell32 grep on its own, and re-bless generated docs (`SOYUZ_BLESS_DOCS=1 cargo test -p soyuz-app --test docs_sync`) when it changes keys or config docs (the orchestrator re-blesses after each merge). Fluent keys land together with the code using them, in **en-US and tr-TR**.

**Wave 1 (parallel)**
- **A — contract (soyuz-core + exhaustive fallout)**: schema (`PdfEngine`, `Preview.pdf_engine`, `struct Pdf` + enums, `Open.pdf_in_viewer`), defaults (values + bindings in all 4 presets), `Config.pdf`, actions (`Pdf*`, `is_pdf`, `gui_only`), keys (`KeyContext::Pdf`, `ALL`, `label`, `home_context`), `docs/config.example.toml`; fallout: `soyuz-appcore/src/palette/catalog.rs::meta` (icons/keywords), `actions.ftl` (`action-pdf-*` + `-desc`, en-US + tr-TR), `i18n/labels.rs` + `core.ftl` (key-context label), `keys/conflicts.rs`, `models/settings.rs` `Section::tables` (add `"pdf"`), TUI `KeyContext` matches + `overlay.rs::is_free`/~683; bless docs. Tests: config round trip, `string_round_trip_all`, `lookup(Pdf, "R") == PdfRotateCw` in all presets, no new conflicts, `every_action_is_translated`, `every_key_shows_on_exactly_one_page`.
- **B — engine (`services/pdf/**`)**: `hayro = "0.9.0"` workspace + appcore deps, `pub mod pdf;`, pdfium module move with shims. Plain parameters, no `Config`. Tests: Letter page 612×792 at scale 1 on white; region == crop of full render; `/Rotate 90`; PageText order and `find` rects over dark pixels; outline + URI/GoTo links; encrypted none/wrong/right; guard TimedOut/Panicked; TexCache eviction; session generation drops stale; fonts resolver returns a face for a CJK fallback query when one is installed (skip otherwise); `tests/boundary.rs`; `cargo deny check`.

**Wave 2 (parallel, after A + B)**
- **C — consumers**: `preview_pdf.rs`, PDF parts of `preview_rich.rs`, `thumbs.rs` PDF parts + `set_pdf` call site, `thumb_disk.rs` doc, `search.rs` ~143–180, `services.ftl` keys, schema doc-comment fixes; "Open in viewer" via `PreviewAction::EditFile`. Tests: adapted preview_pdf tests; preview of `sample_pdf()` without pdfium renders and reports "Renderer: hayro"; `search_text` via hayro; thumbnail without pdfium; pdfium-without-library falls back; 300-page `document_text` timing; bless docs.
- **D — viewer model (`soyuz-appcore/src/editors/pdf/**`)**: `PdfDoc` (layout, zoom modes, anchor, rotation, sidebar, find, selection, `pump`, `command`, `close_bars`, `focus_request`, not dirty), `layout.rs`, `select.rs`; additive `editors/mod.rs` (`DocKind::Pdf`, focus, Esc, devscript, limits, `(LoadResult::Pdf, DocKind::Pdf) if reload` arm ~402), `editors/io.rs` (sniff before `looks_like_text`), `editors/commands.rs` (`binding` with doc kind + field-focus flag; `needs_host` for copy; dispatch), TUI `overlay.rs` call sites, `actions.rs` arm, `ops.rs` `[open] pdf_in_viewer`, session position memory; new `locales/*/pdf.ftl` registered with `ftl!("pdf")` in `i18n/mod.rs` `FILES`; **stub arms** in `ui/editors/panel.rs` (`kind_glyph` ~59, colour ~132, body ~265) and `soyuz-tui/src/view/editor/mod.rs` (~278, ~354). Tests with `no_upload()`: open → `DocKind::Pdf`; fit width; anchor zoom; page clamp; go-to; find wraps + scrolls; select all + copy on `NullHost`; rotation; devscript verbs; R → `PdfRotateCw` on a PDF tab, typing on a text tab, typing in the PDF find field.
- **F — settings**: PDF rows in `pages.rs`, `appearance.rs` row, `[open] pdf_in_viewer` row, HINTS, `settings.ftl` (en + tr). Tests: hints; every `[pdf]` key on a page; no dead settings (engine reaches `thumbs::set_pdf` and `RichLimits`).

**Wave 3 (parallel, after D)**
- **E — viewer UI (`soyuz-app/src/ui/editors/pdf/**`: mod, canvas, toolbar, sidebar, findbar)**; replaces D's panel stubs; additive `ui/editors/mod.rs`, `ui/preview/rich.rs` (open-in-viewer button, card text), `quicklook.rs`; `pdf-ui-*` keys in `pdf.ftl`. Tests: model-level tests + scripted off-screen run (`SOYUZ_NO_SYSTEM_CLIPBOARD=1 SOYUZ_WINDOW_POS=-4000,-4000 SOYUZ_SCRIPT="edit:<fixture>;editor:pdf=zoom=200;editor:pdf=find=Hello;screenshot:…;quit"`), `frame_budget` PDF step, `uia-audit` clean. No egui_kittest.
- **G — TUI**: replaces D's stubs; text-only page view, PgUp/PgDn, go-to page; removes `gui_only` from Pdf actions it supports; `tui-editor.ftl` keys. Tests: headless snapshots, `sr_snapshots`.

**Wave 4 (orchestrator) — H**: docs (USER_GUIDE viewer chapter + PDF/search/Linux notes, ROADMAP F3 + line 585, INTEGRATION_NOTES section — non-UTF-8 bytes, edit carefully, README, CLAUDE.md known gap), `scripts/notices.ps1` + `about.hbs` (Foxit fonts, Adobe CMaps, ICC), `scripts/changelog.ps1`, full gates, `cargo deny check`, Linux cross-check, release size before/after, manual pass on real PDFs (scanned JBIG2/JPX, CJK without embedded fonts, vector map, forms, 1000-page book).

## 6. Risks
1. Decoder OOM aborts the app (out-of-process worker deferred by owner).
2. Abandoned threads keep burning CPU until their job ends.
3. Fidelity gaps (knockout groups, some blending, colour-key masking).
4. Region re-interprets the page: vector-heavy pages at high zoom may be slow — measure.
5. `as_unicode` is experimental upstream; content-stream order; no RTL; no OCR.
6. Whole file in RAM (no mmap: it would lock the file on Windows).
7. `soyuz-appcore` builds with `codegen-units = 16` (rustc inliner workaround): watch release builds after adding hayro.
