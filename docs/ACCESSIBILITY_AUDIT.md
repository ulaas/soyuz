# Accessibility audit (automated: UI Automation on Windows, AT-SPI on Linux)

`scripts/uia-audit.ps1` is the automated counterpart of a screen-reader pass. It reads the
UI Automation tree that Soyuz exposes through AccessKit, which is the same tree NVDA and
Narrator use, and checks names, roles, selection state and progress values. It does **not**
replace a human run with NVDA and Narrator. That run stays on the checklist in
[ACCESSIBILITY.md](ACCESSIBILITY.md): the audit cannot judge announcement order, verbosity,
focus movement or whether a workflow makes sense by ear.

## How it works

```powershell
cargo build --release -p soyuz-app
./scripts/uia-audit.ps1 -Report $env:TEMP\uia.md      # -Only main,confirm  -Dump elements.jsonl

# With the media states, off screen (nothing appears on the desktop):
$env:SOYUZ_FFMPEG_DIR = '<folder with the FFmpeg 9.0 DLLs and ffmpeg.exe>'
$env:SOYUZ_WINDOW_POS = '-4000,-4000'
./scripts/uia-audit.ps1 -Report $env:TEMP\uia.md
```

- It creates the demo tree (`scripts/demo-fixture.ps1`) and runs the release build in portable
  mode. It drives the app only through `SOYUZ_SCRIPT` and sends no mouse or keyboard input.
- Each state is held with the `until:<file>` script step while the audit reads the tree.
  - It makes one cached `FindAll` over the window's subtree.
  - It then reads the `SelectionItem` / `RangeValue` patterns live, because the cached
    pattern-availability flags are not reliable through the AccessKit provider.
  - Dialog roles come from `LocalizedControlType`, because the managed client has no
    `AriaRole` constant.
- The exit code is the number of failed checks.
- **Media states.** `video` and `mediatools` need the FFmpeg libraries (`SOYUZ_FFMPEG_DIR`,
  or the DLLs next to the exe) and the demo video `Videos\Launch replay.mp4`, which
  `demo-fixture.ps1` renders with an `ffmpeg` program from `SOYUZ_FFMPEG_DIR` or the `PATH`
  (any LGPL build: MPEG-4 Part 2 video, AAC audio). Without them the two states are
  skipped with a warning. The player runs muted (`[media] muted = true`).
- **Off screen.** With `SOYUZ_WINDOW_POS` set the app never activates its windows, and a
  native dialog window that is never activated does not deliver its AccessKit tree (the
  confirmation and media tools states then see only the dialog's title bar). The script
  therefore sets `[appearance] native_dialogs = false` for off-screen runs, so dialogs are
  drawn inside the main window with the same content and roles. On-screen runs keep native
  dialogs.

| State | What the script sets up |
|---|---|
| main | details view of `Documents\Reports`, one file selected, folder tree (sidebar mode `both`), two tabs |
| palette | command palette with the query `copy` |
| settings | settings window, General |
| appearance | settings window, Appearance (dropdowns, sliders, color swatches) |
| confirm | the "delete permanently" confirmation, cancelled with Esc (the script checks the file still exists) |
| video | Quick Look on `Videos\Launch replay.mp4`, played for two seconds and paused (Quick Look's bar is wide enough for the volume slider) |
| mediatools | the media tools dialog (*Convert media…*) on the same video, with its probe and plan done, cancelled with Esc |
| progress | a throttled copy job (`SOYUZ_THROTTLE_MBPS=2`) with the activity panel |

Every state runs the generic checks below. Some states add their own checks.

| Check | What it verifies |
|---|---|
| G1 | the window has a name |
| G2 | no keyboard-focusable element is unnamed |
| G3 | buttons, menu items, tabs, list and tree items, check boxes, edits, combo boxes, sliders, spinners and progress bars have a name |
| G4 | list, tree and tab items expose their selection state |
| G5 | no name is made only of icon-font glyphs (Private Use Area characters, which screen readers announce as "private use character") |
| M1–M5 (main) | file rows are named list items; the selected file is reported selected; tree rows are tree items; exactly one tab is selected; Back / Forward / Up / Settings are present |
| P1–P2 (palette) | the search field is a named edit; results are named list items |
| S1–S2 (settings) | the settings window is a named window; switches are named |
| A1 (appearance) | combo boxes, sliders, spinners and fields are named |
| C1–C2 (confirm) | the confirmation is a named (alert) dialog; its Delete / Cancel buttons are named |
| V1–V2 (video) | the Position and Volume sliders are named and have a value; the picture ("Video: *name*"), Play / Pause, Mute / Unmute and Loop buttons are present |
| T1–T3 (mediatools) | the dialog is a named window ("Convert media"); the tool switcher (Convert, Extract audio, Trim, Save frame) and the Run button are named; the Preset combo box and the text fields are named |
| R1–R2 (progress) | a job progress bar exists, with a name and a value from 0 to 100 |

## Results (2026-10-09, Windows 11, release build)

**First run: 37 checks, 17 failed.** Some of these were problems with the audit itself, fixed
in the script:

- **Cached pattern flags.** The cached `Is*PatternAvailable` flags came back false. This made
  every list, tree and tab item fail G4, M2 and M4, and the progress bar fail R2, even though
  their states were exposed. The script now reads the patterns live.
- **Sidebar mode.** The sidebar was in `places` mode, which shows no tree, so M3 failed. The
  script now uses mode `both`.
- **Missing `AriaRole`.** The confirmation dialog failed C1 because the managed client has no
  `AriaRole`. The check now uses `LocalizedControlType`, which reports "alert dialog".

**Real findings in the app, all fixed:**

| # | Where | Finding | Fix |
|---|---|---|---|
| 1 | Toolbar | The filter field was an unnamed edit (its "Filter" text is only a placeholder) | `a11y::name` with the filter label |
| 2 | Command palette | The search field was unnamed | named with its hint text |
| 3 | Command palette | Result rows were generic, unnamed elements | role list item, name "title, detail", keyboard shortcut, selected state, position in set |
| 4 | Command palette | Mode chips (`>` `@` `#` …) were unnamed | toggle buttons named with their legend, pressed state = active mode |
| 5 | Settings | The search field was unnamed | named "Search settings" |
| 6 | Settings | Text and code fields were unnamed | named from their settings row label (`row_label`) |
| 7 | Settings | Combo boxes (dropdowns) were unnamed | named from the row label |
| 8 | Settings → Appearance | Slider value boxes (egui's built-in `DragValue`) were unnamed and could not be named | the slider and its value box are now separate widgets, both named from the row |
| 9 | Settings → Appearance | About three dozen color swatches (theme tokens, custom accent) were unnamed buttons | named with the token label / row label |
| 10 | Settings (lists, AI, keys) | Icon-only buttons (remove, move up / down, reset, add, record) used the icon glyph as their name | names derived from the icon ("Remove", "Move up", …) |
| 11 | Settings, dialogs, status bar | Decorative icon labels (window / section icons, dialog title icon, zoom icon) were text nodes named with a private-use glyph | hidden from assistive technology (`widgets::icon_label` → `a11y::decorative`) |
| 12 | File list | Column headers were generic, unnamed elements, and column resize handles were unnamed | role column header with name and sort direction; handles are named splitters ("Resize the Name column") |
| 13 | Settings (AI, MCP, column widths) | Free-standing number boxes were unnamed (not in an audited state) | named with their labels |

**Final run: 48 checks, 0 failed.** Element counts per state: main 105, palette 122, settings
168, appearance 348, confirm 111, progress 122.

## Results (2026-10-15, Windows 11, release build, Phase V media states)

Off-screen run (`SOYUZ_WINDOW_POS=-4000,-4000`) with FFmpeg 9.0 (an LGPL shared build) from
`SOYUZ_FFMPEG_DIR`: **67 checks, 2 failed.** Element counts per state: main 91, palette 108,
settings 162, appearance 333, confirm 97, video 113, mediatools 118, progress 126.

- **Audit fix:** the first off-screen run also failed C1 / C2 and T2 because native dialog
  windows that are never activated expose only their title bar to UIA. The script now
  draws dialogs inside the window when `SOYUZ_WINDOW_POS` is set (see above).
- **Audit fix:** the preview pane's player bar has no volume slider at the pane's default
  width (it is left out below 330 px), so the `video` state uses Quick Look.

**Re-run after the fixes (same day, `-Only video,mediatools`): 19 checks, 0 failed.** Finding 1 is fixed (the four media-tools text fields are named by their captions); the player's Mute and Loop now report "pressed" / "not pressed".

**Real findings in the app (finding 1 fixed, see above):**

| # | Where | Finding | Suggested fix |
|---|---|---|---|
| 1 | Media tools dialog | The output name field is an unnamed edit (G3, T3); its "Output name" caption is a separate label. The Trim start / end and Save frame time fields use the same helper (`text_field` in `ui/dialogs/media_tools.rs`) and are unnamed too (not in an audited state) | `a11y::name` with the caption (`dialogs-media-name`, `-start`, `-end`, `-at`) |
| 2 | Video player in the preview pane | At the default pane width the volume slider is not drawn, so it is missing from the tree; only `-` / `=` change the volume there | keep a compact volume control, or document it (done in ACCESSIBILITY.md) |

Passed: the player's Position slider (value "0:02 / 0:08" plus a 0–100 range value), Volume
("80%"), the picture button "Video: Launch replay.mp4", Play / Pause, Mute / Unmute and
Loop; the media tools dialog as a named dialog with its tool switcher, Preset combo box
and Cancel / Convert buttons named.

## Not covered

- **Other states.** The audit does not visit:
  - the assistant panel and its plan cards
  - editors (text, hex, image)
  - tools (treemap, compare, duplicates, sync, versions)
  - the properties dialog, Quick Look on anything but a video, the terminal and remote
    connections
  - the media tools' Extract audio, Trim and Save frame pages, the Settings → Media page
    and the FFmpeg runtime download
  - the welcome card, toasts and context menus

  Add a state to the script before relying on any of these.
- **Remaining icon labels.** About 60 `ui.label(icons::icon(…))` calls in other panels still
  expose their glyph. Converting them is mechanical (`widgets::icon_label`), but it touches
  many files, so it was left out of this pass. G5 will flag each one as
  soon as its state is audited.
- **What the audit cannot check.** It does not judge reading order, live-region
  announcements (toasts are not announced yet; see ACCESSIBILITY.md), or keyboard focus
  order. Focus order is covered separately by the keyboard-only GUI test
  (`tests/keyboard_only.rs`).
- **Human run still needed.** The NVDA / Narrator certification run is still to do.

## Linux: AT-SPI audit

`scripts/atspi-audit.py` is the Linux counterpart of `uia-audit.ps1`. It reads the AT-SPI
tree that `accesskit_unix` publishes, which is what Orca uses. It drives the same six states
with the same `SOYUZ_SCRIPT` steps and the same `until:<file>` holds.

```sh
cargo build --release -p soyuz-app
python3 scripts/atspi-audit.py --report /tmp/atspi.md   # --only main,confirm  --dump els.jsonl  --exe <binary>
```

- **Needs:** python3-gobject with the `Atspi 2.0` typelib (at-spi2-core). It does not use
  pyatspi and installs nothing.
- **Demo tree:** it creates a small one in a temporary directory, with the file names the
  states rely on (`Documents/Reports/Q3 mission report.md`, `Launch checklist.txt`, a 64 MB
  `Downloads/Telemetry dump.dat`, …). It never touches your folders.
- **Portable mode:** it runs the binary in portable mode. The `portable` marker and
  `portable-data/` sit next to the binary and are removed afterwards. It refuses to start if
  they already exist.
- **Session accessibility:** it turns the session's accessibility on through `org.a11y.Status`
  on the session bus (`IsEnabled`, and `ScreenReaderEnabled` where the bus still has it). It
  restores the previous values at the end. AccessKit builds no tree until this flag is on.
- **Finding the app:** it finds Soyuz in the AT-SPI registry by process id. In each state it
  clears the AT-SPI cache and walks the whole tree, reading role, name, states, the `Value`
  interface, window-relative extents and labelled-by relations.
- **Window:** the window opens briefly on your screen, because Wayland cannot place it off
  screen. The run takes under a minute. The app is always killed at the end.
- **Exit code:** the number of failed checks. Warnings do not count.

It runs the same generic checks as on Windows, with AT-SPI roles (`button`, `toggle button`,
`check box`, `radio button`, `combo box`, `entry`, `list item`, `tree item`, `page tab`,
`slider`, `spin button`, `progress bar`, …), plus four more:

| Check | What it verifies |
|---|---|
| G1–G5 | as on Windows: window name, unnamed focusable elements, unnamed interactive controls, list / tree / tab items are `SELECTABLE`, glyph-only names |
| G6 | no unnamed interactive siblings with the same role (a screen reader cannot tell them apart) |
| G7 | check boxes, radio buttons and switches are `CHECKABLE` (or `CHECKED`); tab strips have exactly one `SELECTED` tab |
| G7e (warning) | combo boxes are `EXPANDABLE` |
| G8 | text inputs (entry, spin button) have a name or a labelled-by relation |
| G9 | focusable, enabled elements are `SENSITIVE` and have a non-empty size |
| M / P / S / A / C / R | the same per-state checks as on Windows. The confirmation must be a `dialog` or `alert`. The progress bar is read through the `Value` interface. |

### Results (2026-10-11, Fedora 45, niri Wayland, at-spi2-core 2.62, release build)

#### at-spi2-core 2.62: the stock `accesskit_unix` never activated

egui 0.36 links `accesskit_unix` 0.21.1. That version activates only when
`org.a11y.Status.ScreenReaderEnabled` becomes true, and at-spi2-core 2.62 no longer has that
property (only `IsEnabled`). So AccessKit never turned on, Soyuz never appeared in the AT-SPI
registry, and the first audit stopped with "no Soyuz application found". Upstream fixed it in
`accesskit_unix` 0.23, which needs `accesskit` 0.25 (egui 0.36 pins 0.24).

**Fix:** `third_party/accesskit_unix` is 0.21.1 with the one-line change
(`receive_screen_reader_enabled_changed` → `receive_is_enabled_changed` in `src/context.rs`),
wired in through `[patch.crates-io]` in the workspace `Cargo.toml` (see
`third_party/README.md`). The results below come from the normal release build with that patch.
Since egui 0.37 (`accesskit_unix` 0.24, which has the upstream fix) the patch is no longer needed
and was removed.

#### Findings

**First run: 72 checks, 3 failed.** The first pass over `main` alone failed 6 of 17 checks.
Some of those failures were problems in the script, fixed there:

- **Role names.** at-spi2-core 2.62 calls push buttons `button`, not `push button`, so
  Back / Forward / Up (M5) were not found.
- **Toggle buttons.** An untoggled toggle button has no AT-SPI state at all: AccessKit only
  adds `PRESSED` when it is on, and `CHECKABLE` only for other roles. G7 therefore does not
  require a state on toggle buttons.

**Real findings in the app, all fixed:**

| # | Where | Finding | Fix |
|---|---|---|---|
| 1 | Window | The frame had no name. UIA takes the name from the native title, but AT-SPI reads the AccessKit root node | `a11y::window_name` sets the root label to the window title every frame (`app.rs`, `update_title`) |
| 2 | Status bar | The shelf toggle was an unnamed focusable element | `a11y::toggle` with its tooltip, pressed state = shelf open (`tools/mod.rs`) |
| 3 | Status bar | The jobs indicator ("2 jobs") was unnamed | `a11y::button`, "<count>, Show activity (Ctrl+Shift+J)" (`panels/activity.rs`) |
| 4 | All resizable panels | egui's panel resize handles (sidebar, activity, shelf, preview, assistant) were unnamed focusable elements | `a11y::panel_splitter`: role splitter, "Resize the sidebar", … (one call after each panel's `show`) |
| 5 | Window edges | The four client-side resize strips were unnamed focusable elements | hidden from assistive technology (pointer-only; the window manager resizes from the keyboard) (`titlebar.rs`) |
| 6 | Dual / quad pane | The pane splitters were unnamed | `a11y::splitter`, "Resize the panes" (`shell.rs`) |
| 7 | Dual pane | The pane headers (click to focus a pane) were unnamed | toggle named "<folder>, <path>", pressed = focused pane (`pane_view.rs`) |
| 8 | Activity panel | The Jobs / History switcher was unnamed | role tab, selected = current view (`panels/activity.rs`) |
| 9 | Settings → Appearance | The density cards (radio buttons) reported "selected", not "checked". A radio's checked state is AccessKit's `toggled` on both platforms | `set_toggled` (`settings/density_gallery.rs`) |

Findings 2–9 affect Windows too: UIA reads the same AccessKit nodes. Finding 1 is
specific to AT-SPI.

**Final run: 78 checks, 0 failed, 2 warnings.**

- **Warnings (G7e).** The settings dropdowns (Language, Dates, Color theme, Accent color,
  File icons, Graphics backend) are not `EXPANDABLE`. Soyuz sets AccessKit's `expanded`
  state, but `accesskit_atspi_common` 0.18 does not map it to AT-SPI yet; the same applies
  to the folder tree's expanded state. This needs no app change.
- **Element counts per state:** main 94, palette 111, settings 182, appearance 320,
  confirm 100, progress 115.

### Differences from the UIA audit

| | Windows (UIA) | Linux (AT-SPI) |
|---|---|---|
| Activation | the first UIA client request turns AccessKit on | the session flag `org.a11y.Status` must be on (set by the script, restored afterwards); see the at-spi2-core note above |
| Window name | native title bar | AccessKit root node label (`a11y::window_name`) |
| Selection / toggle state | `SelectionItem` / `Toggle` patterns, read live | state set (`SELECTABLE`, `SELECTED`, `CHECKABLE`, `CHECKED`, `PRESSED`); there are no patterns to read |
| Expanded state | `ExpandCollapse` pattern | not mapped by `accesskit_atspi_common` 0.18 (G7e warning) |
| Confirmation | `alert dialog` via `LocalizedControlType` | role `alert` |
| Progress value | `RangeValue` pattern | `Value` interface |
| Placement | can open off screen (`SOYUZ_WINDOW_POS`) | Wayland opens the window on screen for the duration of the run |
| Demo tree | `scripts/demo-fixture.ps1` (full, procedurally drawn) | a minimal tree created by the script itself |
