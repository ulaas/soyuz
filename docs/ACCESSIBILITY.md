# Accessibility

Soyuz draws its own widgets with egui and exposes them to assistive technology through
**AccessKit** (UI Automation on Windows; AT-SPI on Linux and NSAccessibility on macOS later).
AccessKit is switched on automatically when a screen reader or another UI Automation client
connects. Until then egui builds no accessibility tree, so it costs nothing.

This document covers what is exposed, how keyboard-only use works, the automated checks, and
the manual certification checklist for screen readers. The terminal UI has its own section:
[Terminal UI](#terminal-ui) (screen-reader mode and its checklist).

## What is exposed

The helpers live in `crates/soyuz-app/src/ui/a11y.rs`. Every custom-painted widget calls one of
them so it gets a role, a name and its state.

| Widget | Role | Name | State / extra |
|---|---|---|---|
| Toolbar / titlebar icon buttons | Button | tooltip text (translated) | enabled |
| Toggle buttons (hidden files, dual pane, …) | Button | label | toggled |
| Tabs (incl. tab groups) | Tab | tab title (group name for group chips) | selected |
| File list rows (details, list, grid, gallery, Miller) | ListBoxOption inside a ListBox named after the folder | file name | selected, position *n of m* |
| Folder tree rows | TreeItem | folder name | selected, expanded, level, position |
| Breadcrumb segments | Button | folder name | — |
| Dialogs (conflict, delete, properties, rename, …) | Dialog / AlertDialog, modal | dialog title | modal |
| Activity cards (jobs) | Group + ProgressIndicator | job title | progress 0–100 % and state line |
| AI plan cards / MCP request cards | Group | card title | buttons Approve / Reject named |
| Terminal | Terminal | "Terminal n" | visible screen text as value |
| Text / hex editors | multiline text input (egui) | file name | — |
| Treemap tiles | ListItem | file / folder name | size as description |
| Settings controls | the matching egui role (CheckBox for switches, ComboBox, Slider, TextInput) | row label | value |

All names are translated (Fluent). They follow the UI
language, so a Turkish UI speaks Turkish names.

## Keyboard-only operation

Everything is reachable without a pointer. The defaults below are for the Explorer key preset.
Settings → Keys shows the full table, which you can change.

- **Move around:** `Tab` / `Shift+Tab` moves focus between widgets. `F6` (or `Ctrl+L`) jumps to
  the address bar, and the arrow keys move inside lists and trees. Typing a name jumps to it
  (type-ahead; the I/İ/ı/i letters match each other).
- **Open, go back:** `Enter` opens, `Backspace` goes up a level, `Alt+Left` / `Alt+Right` go back
  and forward.
- **File operations:** `Ctrl+C` / `Ctrl+X` / `Ctrl+V`, `Ctrl+D` duplicates, `F2` renames,
  `Delete` moves to the Recycle Bin, `Shift+Delete` deletes permanently (asks first), and
  `Ctrl+Z` undoes.
- **Command palette:** `Ctrl+Shift+P` or `Ctrl+K`. Type, use the arrow keys, then `Enter`.
- **Settings:** `Ctrl+,`. Use `Tab` through the controls and `Esc` to close.
- **Dialogs:** dialogs are modal, and focus is trapped inside (egui `Modal`). `Tab` /
  `Shift+Tab` cycle through the dialog's controls only. `Enter` triggers the primary button
  (for destructive confirmations it is the button named after the action, e.g. "Delete"), and
  `Esc` cancels. When a dialog has a text field, the field gets focus when the dialog opens.

### Automated keyboard-only test

`crates/soyuz-app/tests/keyboard_only.rs` launches the real binary in portable mode on a
temporary folder. It sends **only synthetic key events** (the `SOYUZ_SCRIPT` `keys:` step) to
run through this sequence: open a folder and go back, duplicate a file, rename inline, delete
permanently with a keyboard confirmation, open and close Settings, and run "New folder" from the
command palette. It checks state through `expect:` steps and the result on disk. It runs once in
English and once in Turkish (where the palette query is typed in Turkish).

```sh
cargo test -p soyuz-app --test keyboard_only -- --ignored --nocapture
```

The test needs a desktop session, the same as the frame-budget tour. It never touches the user's
folders, settings or the Recycle Bin.

### AccessKit tree tests

Unit tests in `ui/a11y.rs` run egui frames with AccessKit enabled and check the resulting tree:
roles, names, selection, expansion, level, position in set, progress value, the terminal value
and dialog modality.

## Manual screen-reader checklist

Run this before each release, and after any change to a custom widget. Use a clean portable
build (an empty `portable` file next to `soyuz.exe`). Check it in both English and Turkish
(Settings → General → Language).

### NVDA (Windows, primary)

1. Start NVDA, then Soyuz. NVDA announces the window title "Soyuz – <folder>".
2. **File list:** arrow down through the files. Each row announces the name, then "selected" and
   "n of m". `Space` / `Ctrl+Space` changes the selection and NVDA announces the change.
3. **Folder tree:** `F6` until you reach the tree. NVDA announces the folder name, "expanded" /
   "collapsed" and the level. `Right` / `Left` expands and collapses.
4. **Tabs:** `Ctrl+Tab` announces the tab title and "selected". Tab groups announce their name.
5. **Toolbar:** `Tab` to the toolbar. Every icon button has a spoken name (no "button" without a
   name), and toggles announce "pressed" / "not pressed".
6. **Copy a large folder:** the activity card announces its title. In browse mode the progress
   bar reports its percentage, and Pause / Resume / Cancel are named buttons.
7. **Conflict dialog:** copy onto an existing file. NVDA announces "dialog" and its title, and
   focus is inside the dialog. `Tab` cycles Replace / Skip / Keep both / Apply to all and never
   leaves the dialog. `Esc` cancels.
8. **Delete confirmation:** `Shift+Delete`. It is announced as an alert dialog. `Enter`
   confirms and `Esc` cancels.
9. **Rename:** `F2`. NVDA announces an edit field containing the name with the stem selected.
10. **Settings:** `Ctrl+,`. Every row control is announced with its label (e.g. "Show hidden
    files, toggle button, not pressed"), and drop-downs announce their value.
11. **Command palette:** `Ctrl+Shift+P`. The input is announced, and the results list announces
    the title of the highlighted command.
12. **Terminal:** focus the terminal. NVDA review mode (`NVDA+Numpad 7/8/9`) reads the visible
    screen text.
13. **AI panel:** plan cards announce their title and steps, and Approve / Reject are named.
14. **Treemap:** tiles announce the name and size.
15. **Language switch:** change to Türkçe in Settings. Names are spoken in Turkish at once, with
    no restart.

### Narrator (Windows)

Repeat steps 2–11 with Narrator (`Win+Ctrl+Enter`), using scan mode (`Caps Lock+Space`) for the
dialogs. Also check with Narrator's "Inspect" mode (or Accessibility Insights for Windows →
FastPass) that no focusable element is unnamed.

### Orca (Linux)

AccessKit reaches Orca through AT-SPI (`accesskit_unix`). The automated AT-SPI audit
(`scripts/atspi-audit.py`, results in [ACCESSIBILITY_AUDIT.md](ACCESSIBILITY_AUDIT.md#linux-at-spi-audit))
covers the same six states as the UIA audit and is clean, but only with a patched
`accesskit_unix` (see below). The human Orca run is still to do: repeat the NVDA list with
Orca (`Super+Alt+S`; on niri / other compositors start `orca` from a terminal), using
`Orca+Space` preferences for verbosity and flat-review (`KP_Add`/`KP_8`) for the status bar.

Linux notes:

- **Activation.** AccessKit builds no tree until the session's accessibility flag is on
  (`org.a11y.Status` on the session bus). Orca turns it on. Check it with
  `busctl --user get-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled`.
- **at-spi2-core 2.62+ (Fedora 45).** egui 0.36 links `accesskit_unix` 0.21, which waits for
  the `ScreenReaderEnabled` property that newer at-spi2-core removed. Soyuz vendors that
  crate with a one-line fix that follows `IsEnabled` instead (`third_party/accesskit_unix`, the
  same change upstream made in 0.23), so Orca sees Soyuz on old and new distributions alike.
  Drop the patch once egui moves to `accesskit` 0.25.
- **Window name.** AT-SPI reads the window name from the AccessKit root node, not from the
  Wayland / X11 title, so Soyuz sets the root label to the window title every frame.
- **Expanded state.** `accesskit_atspi_common` 0.18 does not map AccessKit's `expanded`
  state yet, so Orca does not announce dropdowns and folder-tree rows as expanded or
  collapsed.

### VoiceOver (macOS) — deferred

macOS is out of scope for now. When it comes back, repeat the NVDA list with VoiceOver
(`accesskit_macos`, VO+arrow navigation).

## Terminal UI

The terminal UI (`soyuz-tui`, `soyuz --tui`) has no accessibility tree: a screen reader reads
the terminal's text, as it does for any console program. Its **screen-reader mode** draws that
text so it reads well. Turn it on with *Toggle screen-reader mode* in the command palette, with
`[tui] screen_reader = true` (*Settings → Terminal UI → Accessibility* in the window, the
*Terminal UI* section of the terminal UI's settings editor), or for one run with
`soyuz-tui --screen-reader` (never saved). The code is in `crates/soyuz-tui/src/a11y.rs`;
the user guide describes it in [Screen-reader mode](USER_GUIDE.md#screen-reader-mode).

| Part | Default look | Screen-reader mode |
|---|---|---|
| Lines, icons, pictures | box drawing, icons, images, bars, spinners | none: plain lines; any box, block or braille character left over is blanked |
| File rows | icon, highlight, `>` marker | `*` for selected rows, `/` after folders, no icons or markers |
| Panes | every pane of the tab | the focused pane only, plus an "Other pane: <folder>" line |
| Terminal cursor | hidden (a drawn caret) | on the focused row, field or button (the editors, the AI composer and the terminal pane keep their own cursor) |
| Status line | counts, free space, view and sort with symbols | one sentence led by the focused item ("Q3 mission report.md, file, 556 B, 3 of 3, selected; 3 items, …; Details, sorted by Name, ascending"), rewritten in full whenever it changes |
| Toasts | boxes in a corner | text in the status line ("Warning: title. body"), at least their unscaled duration |
| Dialogs, menus, palette | boxes over the panes | the rows they cover are cleared first; form labels end with a colon |
| Job progress | live bar and numbers | the percentage only, updated per 10 % or every 2 s |
| Miller columns, settings editor | several columns | one column |

### Automated checks

- `crates/soyuz-tui/src/view/sr_tests.rs`: the look, the caret for each kind of focus, the
  one-pane layout, the status sentence and the toasts, on hand-built view models.
- Snapshots of screen-reader screens: `view__*__sr.snap` (tier A, fixed models) and
  `sr__*.snap` (tier B, the real app on a fixture) in `crates/soyuz-tui/tests/snapshots/`;
  each tier-B shot also checks that the cursor is on the focused row.
- `crates/soyuz-tui/tests/headless/screen_reader.rs`: the palette toggle (saved), the
  `--screen-reader` override (never saved) and the keyboard tour in screen-reader mode.
- `crates/soyuz-tui/tests/headless/tour.rs`: the keyboard-only tour (browse, file operations,
  a dialog, the context menu, tabs, dual panes, the palette, the jobs panel, search,
  settings, quit) in English and Turkish, with the default and the legacy-terminal key map,
  keys only.
- `crates/soyuz-tui/tests/headless/binary.rs`: the shipped binary run headless with
  `--screen-reader`; no box drawing character may be left on its screen.

```sh
cargo test -p soyuz-tui --lib view::sr_tests
cargo test -p soyuz-tui --test headless_tui headless::screen_reader
cargo test -p soyuz-tui --test headless_tui headless::sr_snapshots
cargo test -p soyuz-tui --test headless_tui headless::tour
```

These check what is written to the terminal, not what a screen reader says about it. That
part needs the manual run below.

### Manual checklist (terminal UI)

**Not run by a human yet.** The owner deferred the screen-reader run (2026-10-09, confirmed for
Phase T8 on 2026-10-14). This is the script for it; record the results in this file.

Use a portable copy (an empty `portable` file next to a copy of `soyuz-tui`), a terminal of
at least 100×30, and both English and Turkish (`[general] language`). Start with
`soyuz-tui --screen-reader <folder>`; once, start without the flag and use the palette's
*Toggle screen-reader mode* instead.

| Reader | Terminal | Notes |
|---|---|---|
| Narrator | Windows Terminal | Narrator reads new text as the terminal reports it; scan mode (`Caps Lock+Space`) reads line by line. |
| NVDA | Windows Terminal, then the classic console (`conhost`) | Turn on *Report dynamic content changes*; the review cursor should follow the system caret. |
| Orca | GNOME Terminal (VTE), then Konsole | Orca speaks changed text; flat review (`KP_8`) reads the cursor line. |
| VoiceOver | Terminal.app | Optional while macOS screen-reader testing is deferred for the window too. |

Steps (for each: what you hear should match the status line or the line under the cursor):

1. **Start.** The first status sentence is read: the focused item, the item count, the free
   space, the view and the sort.
2. **File list.** Arrow down through a folder. Each move reads the new item ("name, file,
   size, n of m"). `Space` (or your selection key) adds ", selected", and the row shows `*`.
3. **Review the cursor line.** With the reader's review cursor, read the current line: it is
   the focused row, not the status line or another pane.
4. **Panes.** Turn on dual panes and switch panes. Only one pane is drawn; the "Other pane"
   line names the other folder, and switching reads the other pane's focused item.
5. **Command palette.** `Ctrl+K`, type a few letters, arrow through the results. The focused
   command is read; `Esc` closes the palette and the list's sentence comes back.
6. **Conflict dialog.** Copy a file onto an existing one. The dialog title and its focused
   button are read; `Tab` moves between the buttons and each one is read; `Esc` decides later.
7. **Delete confirmation.** `Shift+Del` on a test file in the sandbox. The question and the
   focused button are read; `Esc` cancels.
8. **Rename and forms.** `F2`, then a *New folder* dialog: the field's label ("Name:") and its
   text are read; typing does not make the status line repeat itself on every key.
9. **Jobs.** Copy a large file (start with `SOYUZ_THROTTLE_MBPS=4` to slow copies down).
   Progress is read about every 10 percent, not on every frame; `Alt+J` moves to the jobs
   panel and its rows are read.
10. **Toasts.** Undo something (`Ctrl+Z`). The toast is read once, in full, and stays long
    enough to finish.
11. **Settings.** `Ctrl+,` (`Alt+,` in Windows Terminal). One column is shown; sections and
    keys are read with their values; toggling `[tui] mouse` reads the new value.
12. **Menus.** `Shift+F10` on a file. The focused entry is read; the arrows move, `Esc`
    closes.
13. **Language.** Switch to Turkish (settings): the sentences change language at once.
14. **Quit.** `Ctrl+Q`. The terminal is restored and the reader is back at the shell prompt.

Known limits to note, not to fail: pictures are not drawn (the preview says "Picture (not
shown in screen-reader mode)"); the editors, the AI composer and the embedded terminal place
the cursor themselves; screen-reader mode is not turned on automatically when a screen reader
runs.

## Known gaps

- **Linux: vendored `accesskit_unix` patch** for at-spi2-core 2.62+ (see "Orca (Linux)"
  above); remove it once egui moves to `accesskit` 0.25.
- **No screen-reader run has been recorded yet.** The checklist above is the certification
  script, and its results go into this file. The automated UIA audit
  (`scripts/uia-audit.ps1`, results in `ACCESSIBILITY_AUDIT.md`) checks what a screen reader
  receives (names, roles, states) in six states, but not how NVDA / Narrator announce it or
  whether navigation feels right; it does not replace this run.
- **Terminal UI:** no human run of its checklist yet (see [Terminal UI](#terminal-ui)); the
  terminal's cursor marks the focus only in screen-reader mode (the default look keeps its
  drawn caret), and the mode is not turned on automatically when a screen reader runs.
- Right-to-left layout is not supported yet; Arabic / Hebrew names render
  with a system fallback font but are laid out in logical order.
- egui exposes text runs, not a rich document model. The text editor reads line by line,
  without paragraph or heading semantics.
- The terminal value is the visible screen only, not the scrollback.
- Live regions: toasts are not announced automatically yet (AccessKit `Live::Polite` on the
  toast host is the planned fix).
- High-contrast: the app has its own high-contrast theme setting. The Windows high-contrast
  mode is not followed automatically.
