# Accessibility

Soyuz draws its own widgets with egui and exposes them to assistive technology through
**AccessKit** (UI Automation on Windows; AT-SPI on Linux and NSAccessibility on macOS later).
AccessKit is switched on automatically when a screen reader or another UI Automation client
connects. Until then egui builds no accessibility tree, so it costs nothing.

This document covers what is exposed, how keyboard-only use works, the automated checks, and
the manual certification checklist for screen readers.

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

All names are translated (Fluent, see `docs/INTEGRATION_NOTES.md` → F10). They follow the UI
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

### Orca (Linux) and VoiceOver (macOS) — deferred

Linux and macOS are out of scope until the owner lifts the platform deferral (see the project notes).
When they come back, repeat the NVDA list with Orca (AT-SPI through `accesskit_unix`) and
VoiceOver (`accesskit_macos`, VO+arrow navigation).

## Known gaps

- **No screen-reader run has been recorded yet.** The checklist above is the certification
  script, and its results go into this file. The automated UIA audit
  (`scripts/uia-audit.ps1`, results in `ACCESSIBILITY_AUDIT.md`) checks what a screen reader
  receives (names, roles, states) in six states, but not how NVDA / Narrator announce it or
  whether navigation feels right; it does not replace this run.
- Right-to-left layout is deferred (owner decision 2026-10-09); Arabic / Hebrew names render
  with a system fallback font but are laid out in logical order.
- egui exposes text runs, not a rich document model. The text editor reads line by line,
  without paragraph or heading semantics.
- The terminal value is the visible screen only, not the scrollback.
- Live regions: toasts are not announced automatically yet (AccessKit `Live::Polite` on the
  toast host is the planned fix).
- High-contrast: the app has its own high-contrast theme setting. The Windows high-contrast
  mode is not followed automatically.
