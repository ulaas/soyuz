# Accessibility audit (automated, UI Automation)

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

| State | What the script sets up |
|---|---|
| main | details view of `Documents\Reports`, one file selected, folder tree (sidebar mode `both`), two tabs |
| palette | command palette with the query `copy` |
| settings | settings window, General |
| appearance | settings window, Appearance (dropdowns, sliders, color swatches) |
| confirm | the "delete permanently" confirmation, cancelled with Esc (the script checks the file still exists) |
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

## Not covered

- **Other states.** The audit does not visit:
  - the assistant panel and its plan cards
  - editors (text, hex, image)
  - tools (treemap, compare, duplicates, sync, versions)
  - the properties dialog, Quick Look, the terminal and remote connections
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
