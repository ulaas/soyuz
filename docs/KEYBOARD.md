# Keyboard shortcuts

<!-- Generated from soyuz-core's default key maps by crates/soyuz-app/tests/docs_sync.rs.
     Do not edit by hand: run `SOYUZ_BLESS_DOCS=1 cargo test -p soyuz-app --test docs_sync`. -->

Soyuz ships four key presets. Pick one in **Settings → Keyboard** (or `[keys] preset` in
`config.toml`), then override single actions there or under `[keys.bindings]`; see
[CONFIG.md](CONFIG.md#keys). The command palette (<kbd>Ctrl</kbd>+<kbd>K</kbd>) lists every
action with its current shortcut, and <kbd>Ctrl</kbd>+<kbd>/</kbd> shows the active map in the app.
A printable one-page version is in [keyboard.html](keyboard.html) (open it in a browser and print to PDF).

Shortcuts marked *(file list)*, *(sidebar)* and so on only apply while that part of the window has
keyboard focus; the rest work everywhere. Text fields, the terminal and the built-in editors keep
their usual editing keys.

| Preset | Style |
|---|---|
| Explorer | Windows Explorer conventions (the default). |
| Commander (F-keys) | Total Commander / Norton Commander F-keys for two-pane work. |
| Finder | macOS Finder conventions (Cmd is Ctrl on Windows and Linux). |
| Vim-ish | Vim / ranger-style letter keys and two-key sequences. |

## Explorer preset (default)

The *Terminal UI* column is for `soyuz --tui`: *same* means the same keys as in the window,
— that the terminal UI does not have the action. Terminals that cannot send some chords get
extra ones, see [Terminal UI on legacy terminals and in Windows Terminal](#terminal-ui-on-legacy-terminals-and-in-windows-terminal).

### Navigation

| Action | Shortcut | Terminal UI | What it does |
|---|---|---|---|
| Back | <kbd>Alt</kbd>+<kbd>←</kbd> | same | Go back in this pane's history |
| Forward | <kbd>Alt</kbd>+<kbd>→</kbd> | same | Go forward in this pane's history |
| Up | <kbd>Alt</kbd>+<kbd>↑</kbd>, <kbd>Backspace</kbd> *(file list)* | same | Go to the parent folder |
| Focus address bar | <kbd>Ctrl</kbd>+<kbd>L</kbd> | same | Edit the path in the address bar |
| Refresh | <kbd>Ctrl</kbd>+<kbd>R</kbd>, <kbd>F5</kbd> | same | Reload the folder listing |
| Toggle sidebar | <kbd>Ctrl</kbd>+<kbd>B</kbd> | same | Show or hide the sidebar |
| Focus sidebar | — | <kbd>F12</kbd> | Move keyboard focus to the sidebar |
| Focus terminal | — | <kbd>Alt</kbd>+<kbd>T</kbd> | Move keyboard focus to the terminal |
| Next tab | <kbd>Ctrl</kbd>+<kbd>Tab</kbd> | same | Switch to the next tab |
| Prev tab | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Tab</kbd> | same | Switch to the previous tab |
| Go to folder… | <kbd>Ctrl</kbd>+<kbd>P</kbd> | same | Fuzzy-jump to a recent or frequent folder |
| Focus jobs panel | — *(terminal UI only)* | <kbd>Alt</kbd>+<kbd>J</kbd> | Move keyboard focus to the running and finished operations (terminal UI) |
| Focus editor | — | <kbd>Alt</kbd>+<kbd>E</kbd> | Give the keyboard back to the open editor document |
| Open bookmark 1–9 | <kbd>Alt</kbd>+<kbd>1</kbd> … <kbd>Alt</kbd>+<kbd>9</kbd> | same | Navigate to a configured bookmark |
| Go to tab 1–8 | <kbd>Ctrl</kbd>+<kbd>1</kbd> … <kbd>Ctrl</kbd>+<kbd>8</kbd> | same | Switch to a tab by position |
| Go to last tab | <kbd>Ctrl</kbd>+<kbd>9</kbd> | same | Switch to a tab by position |

### Tabs & panes

| Action | Shortcut | Terminal UI | What it does |
|---|---|---|---|
| New tab | <kbd>Ctrl</kbd>+<kbd>T</kbd> | same | Open a new tab |
| Close tab | <kbd>Ctrl</kbd>+<kbd>W</kbd> | same | Close the current tab |
| Reopen closed tab | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd> | same | Reopen the most recently closed tab |
| Toggle dual pane | <kbd>F9</kbd> | same | Show or hide the second pane |
| New window | <kbd>Ctrl</kbd>+<kbd>N</kbd> | same | Open a new Soyuz window |
| Toggle quad layout (2×2) | <kbd>Shift</kbd>+<kbd>F9</kbd> | same | Show four panes in a 2×2 grid |
| Workspaces… | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>W</kbd> | same | Open, save and manage named workspaces |

### View

| Action | Shortcut | Terminal UI | What it does |
|---|---|---|---|
| View: Details | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>1</kbd> | same | Show the folder as a detailed table |
| View: Grid | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>2</kbd> | same | Show icons or thumbnails in a grid |
| View: List | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>3</kbd> | same | Show a compact multi-column list |
| View: Columns (Miller) | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>4</kbd> | same | Browse with cascading columns |
| View: Gallery | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>5</kbd> | same | Large preview with a filmstrip |
| Larger icons | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>+</kbd>, <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>=</kbd> | same | Make icons and thumbnails larger |
| Smaller icons | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>-</kbd> | same | Make icons and thumbnails smaller |
| Reset icon size | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>0</kbd> | same | Reset the icon size |
| Zoom in (interface) | <kbd>Ctrl</kbd>+<kbd>+</kbd>, <kbd>Ctrl</kbd>+<kbd>=</kbd> | same | Scale the whole interface up |
| Zoom out (interface) | <kbd>Ctrl</kbd>+<kbd>-</kbd> | same | Scale the whole interface down |
| Reset interface zoom | <kbd>Ctrl</kbd>+<kbd>0</kbd> | same | Back to 100 % interface scale |
| Toggle hidden files | <kbd>Ctrl</kbd>+<kbd>H</kbd> | same | Show or hide hidden files |
| Toggle preview pane | <kbd>Alt</kbd>+<kbd>P</kbd> | same | Show or hide the preview pane |
| Toggle terminal | <kbd>Ctrl</kbd>+<kbd>&#96;</kbd> | same | Show or hide the integrated terminal |
| Toggle activity panel | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>J</kbd> | same | Show or hide running jobs |
| Toggle AI assistant | <kbd>Ctrl</kbd>+<kbd>I</kbd> | same | Show or hide the AI assistant panel |
| Toggle disk tree | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>E</kbd> | same | Switch the sidebar between places and the disk tree |
| Scroll preview up | — *(terminal UI only)* | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>↑</kbd> | Scroll the preview pane up one line (terminal UI) |
| Scroll preview down | — *(terminal UI only)* | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>↓</kbd> | Scroll the preview pane down one line (terminal UI) |
| Preview page up | — *(terminal UI only)* | <kbd>Alt</kbd>+<kbd>PageUp</kbd> | Scroll the preview pane up one page (terminal UI) |
| Preview page down | — *(terminal UI only)* | <kbd>Alt</kbd>+<kbd>PageDown</kbd> | Scroll the preview pane down one page (terminal UI) |

### Selection

| Action | Shortcut | Terminal UI | What it does |
|---|---|---|---|
| Select all | <kbd>Ctrl</kbd>+<kbd>A</kbd> *(file list)* | same | Select every item |
| Select none | <kbd>Esc</kbd> *(file list)* | same | Clear the selection |
| Invert selection | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>I</kbd> *(file list)* | same | Select everything that isn't selected |
| Cursor up | <kbd>↑</kbd> *(file list)* | same | Move the cursor up |
| Cursor down | <kbd>↓</kbd> *(file list)* | same | Move the cursor down |
| Cursor page up | <kbd>PageUp</kbd> *(file list)* | same | Move the cursor one page up |
| Cursor page down | <kbd>PageDown</kbd> *(file list)* | same | Move the cursor one page down |
| Cursor home | <kbd>Home</kbd> *(file list)* | same | Jump to the first item |
| Cursor end | <kbd>End</kbd> *(file list)* | same | Jump to the last item |
| Extend up | <kbd>Shift</kbd>+<kbd>↑</kbd> *(file list)* | same | Extend the selection upwards |
| Extend down | <kbd>Shift</kbd>+<kbd>↓</kbd> *(file list)* | same | Extend the selection downwards |
| Filter this folder | <kbd>Ctrl</kbd>+<kbd>F</kbd> | same | Filter the list as you type |

### File

| Action | Shortcut | Terminal UI | What it does |
|---|---|---|---|
| Open | <kbd>Enter</kbd> *(file list)* | same | Open the selection |
| Open with… | <kbd>Shift</kbd>+<kbd>Enter</kbd> *(file list)* | same | Open with a chosen application |
| Edit in Soyuz | <kbd>F4</kbd> *(file list)* | same | Open the file in the built-in text, hex or image editor |
| Open in new tab | <kbd>Ctrl</kbd>+<kbd>Enter</kbd> *(file list)* | same | Open the selected folders in new tabs |
| Copy | <kbd>Ctrl</kbd>+<kbd>C</kbd> *(file list)* | same | Copy the selection to the clipboard |
| Cut | <kbd>Ctrl</kbd>+<kbd>X</kbd> *(file list)* | same | Cut the selection to the clipboard |
| Paste | <kbd>Ctrl</kbd>+<kbd>V</kbd> *(file list)* | same | Paste the clipboard into this folder |
| Paste as link | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> *(file list)* | same | Paste the clipboard as links |
| Copy to other pane | <kbd>F5</kbd> *(file list)* | same | Copy the selection to the other pane |
| Move to other pane | <kbd>F6</kbd> *(file list)* | same | Move the selection to the other pane |
| Duplicate | <kbd>Ctrl</kbd>+<kbd>D</kbd> *(file list)* | same | Make a copy next to the original |
| Rename | <kbd>F2</kbd> *(file list)* | same | Rename the item under the cursor |
| Batch rename… | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>R</kbd> *(file list)* | same | Rename many items with patterns |
| New folder | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>N</kbd> *(file list)* | same | Create a new folder here |
| New file | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>N</kbd> *(file list)* | same | Create a new empty file here |
| Move to trash | <kbd>Del</kbd> *(file list)* | same | Move the selection to the trash |
| Delete permanently | <kbd>Shift</kbd>+<kbd>Del</kbd> *(file list)* | same | Delete without the trash |
| Properties | <kbd>Alt</kbd>+<kbd>Enter</kbd> *(file list)* | same | Show size, dates and attributes |
| Copy path | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> *(file list)* | same | Copy the full path to the clipboard |
| Undo | <kbd>Ctrl</kbd>+<kbd>Z</kbd> | same | Undo the last file operation |
| Redo | <kbd>Ctrl</kbd>+<kbd>Y</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd> | same | Redo the last undone operation |

### Tools

| Action | Shortcut | Terminal UI | What it does |
|---|---|---|---|
| Command palette | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd>, <kbd>Ctrl</kbd>+<kbd>K</kbd> | same | Search and run any command |
| Search | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd>, <kbd>F3</kbd> | same | Search this folder and its subfolders |
| Settings | <kbd>Ctrl</kbd>+<kbd>,</kbd> | same | Open the settings |
| Keyboard shortcuts | <kbd>Ctrl</kbd>+<kbd>/</kbd> | same | Browse and edit keyboard shortcuts |
| Quit | <kbd>Ctrl</kbd>+<kbd>Q</kbd> | same | Quit Soyuz |
| Quick Look | <kbd>Space</kbd> *(file list)* | same | Preview the item in a popup |
| Open context menu | — *(terminal UI only)* | <kbd>Shift</kbd>+<kbd>F10</kbd> | Show the menu of the focused item, like a right click (terminal UI) |
| Editor: Save | <kbd>Ctrl</kbd>+<kbd>S</kbd> *(editor)* | same | Save the document in the editor |
| Editor: Save as | — *(terminal UI only)* | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> *(editor)*, <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> *(editor)* | Save a copy of the document under a new name and edit the copy (terminal UI) |
| Editor: Find | <kbd>Ctrl</kbd>+<kbd>F</kbd> *(editor)* | same | Find text in the document |
| Editor: Replace | <kbd>Ctrl</kbd>+<kbd>H</kbd> *(editor)* | <kbd>Ctrl</kbd>+<kbd>R</kbd> *(editor)* | Find and replace text in the document |
| Editor: Go to line | <kbd>Ctrl</kbd>+<kbd>G</kbd> *(editor)* | same | Jump to a line (text) or an offset (hex) |
| Editor: Find next | <kbd>F3</kbd> *(editor)* | same | Select the next match |
| Editor: Find previous | <kbd>Shift</kbd>+<kbd>F3</kbd> *(editor)* | same | Select the previous match |
| Editor: Close document | <kbd>Ctrl</kbd>+<kbd>W</kbd> *(editor)* | same | Close the document (asks to save changes first) |
| Editor: Next document | <kbd>Ctrl</kbd>+<kbd>PageDown</kbd> *(editor)* | same | Switch to the next editor tab |
| Editor: Previous document | <kbd>Ctrl</kbd>+<kbd>PageUp</kbd> *(editor)* | same | Switch to the previous editor tab |
| Editor: Toggle word wrap | — | <kbd>Alt</kbd>+<kbd>Z</kbd> *(editor)* | Wrap long lines at the panel's edge or scroll sideways |
| Editor: Maximize | — *(terminal UI only)* | <kbd>Alt</kbd>+<kbd>M</kbd> *(editor)* | Let the editor fill the file panes' area, or restore it (terminal UI) |
| Editor: Switch dock side | — *(terminal UI only)* | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>M</kbd> *(editor)* | Dock the editor on the right or at the bottom (terminal UI) |
| Editor: Answer notice | — *(terminal UI only)* | <kbd>Alt</kbd>+<kbd>E</kbd> *(editor)* | Run the action of the document's notice: reload a changed file, edit a lossy file anyway, allow editing a large file (terminal UI) |
| Editor: Toggle match case | — *(terminal UI only)* | <kbd>Alt</kbd>+<kbd>C</kbd> *(editor)* | Match upper and lower case in find (terminal UI) |
| Editor: Toggle whole word | — *(terminal UI only)* | <kbd>Alt</kbd>+<kbd>W</kbd> *(editor)* | Find whole words only (terminal UI) |
| Editor: Toggle regular expression | — *(terminal UI only)* | <kbd>Alt</kbd>+<kbd>R</kbd> *(editor)* | Find with a regular expression (terminal UI) |
| Editor: Replace all | — *(terminal UI only)* | <kbd>Alt</kbd>+<kbd>A</kbd> *(editor)* | Replace every match in the document (terminal UI) |
| Editor: Resize image | — *(terminal UI only)* | <kbd>Alt</kbd>+<kbd>S</kbd> *(editor)* | Scale the image to a new size in pixels (terminal UI) |
| Player: Play / pause | <kbd>P</kbd> *(player)*, <kbd>K</kbd> *(player)* | same | Start or pause the video or audio playing in the preview or Quick Look |
| Player: Back | <kbd>J</kbd> *(player)*, <kbd>←</kbd> *(player)* | same | Jump back by the seek step ([media] seek_step_secs) |
| Player: Forward | <kbd>L</kbd> *(player)*, <kbd>→</kbd> *(player)* | same | Jump forward by the seek step ([media] seek_step_secs) |
| Player: Louder | <kbd>=</kbd> *(player)* | same | Raise the player volume |
| Player: Quieter | <kbd>-</kbd> *(player)* | same | Lower the player volume |
| Player: Mute / unmute | <kbd>M</kbd> *(player)* | same | Turn the player sound off or on |

## Terminal UI on legacy terminals and in Windows Terminal

A terminal without the kitty keyboard protocol (most terminals on Linux and macOS) sends
<kbd>Ctrl</kbd>+<kbd>Shift</kbd>+letter as <kbd>Ctrl</kbd>+letter and <kbd>Ctrl</kbd>+<kbd>Tab</kbd> as
<kbd>Tab</kbd>, and nothing for <kbd>Ctrl</kbd> with most punctuation. Windows Terminal keeps some
chords for itself (new tab, split pane, its own palette, …). The terminal UI detects both and adds
the chords below to the Explorer preset; the original chords keep working wherever they arrive.
The Windows console and terminals with the kitty protocol need none of them. An extra chord is
only added while it is free, so your own bindings win.

| Action | Legacy terminal | Windows Terminal | WSL in Windows Terminal |
|---|---|---|---|
| Next tab | <kbd>Ctrl</kbd>+<kbd>PageDown</kbd> | <kbd>Ctrl</kbd>+<kbd>PageDown</kbd> | <kbd>Ctrl</kbd>+<kbd>PageDown</kbd> |
| Prev tab | <kbd>Ctrl</kbd>+<kbd>PageUp</kbd> | <kbd>Ctrl</kbd>+<kbd>PageUp</kbd> | <kbd>Ctrl</kbd>+<kbd>PageUp</kbd> |
| Reopen closed tab | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd> | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd> | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd> |
| View: Details | <kbd>Ctrl</kbd>+<kbd>F1</kbd> | <kbd>Ctrl</kbd>+<kbd>F1</kbd> | <kbd>Ctrl</kbd>+<kbd>F1</kbd> |
| View: Grid | <kbd>Ctrl</kbd>+<kbd>F2</kbd> | <kbd>Ctrl</kbd>+<kbd>F2</kbd> | <kbd>Ctrl</kbd>+<kbd>F2</kbd> |
| View: List | <kbd>Ctrl</kbd>+<kbd>F3</kbd> | <kbd>Ctrl</kbd>+<kbd>F3</kbd> | <kbd>Ctrl</kbd>+<kbd>F3</kbd> |
| View: Columns (Miller) | <kbd>Ctrl</kbd>+<kbd>F4</kbd> | <kbd>Ctrl</kbd>+<kbd>F4</kbd> | <kbd>Ctrl</kbd>+<kbd>F4</kbd> |
| View: Gallery | <kbd>Ctrl</kbd>+<kbd>F5</kbd> | <kbd>Ctrl</kbd>+<kbd>F5</kbd> | <kbd>Ctrl</kbd>+<kbd>F5</kbd> |
| Toggle terminal | <kbd>Alt</kbd>+<kbd>&#96;</kbd> | — | <kbd>Alt</kbd>+<kbd>&#96;</kbd> |
| Toggle activity panel | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>J</kbd> | — | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>J</kbd> |
| Toggle AI assistant | <kbd>Alt</kbd>+<kbd>I</kbd> | — | <kbd>Alt</kbd>+<kbd>I</kbd> |
| Toggle disk tree | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>E</kbd> | — | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>E</kbd> |
| Invert selection | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>I</kbd> *(file list)* | — | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>I</kbd> *(file list)* |
| Paste | — | <kbd>Alt</kbd>+<kbd>V</kbd> *(file list)* | — |
| Paste as link | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> *(file list)* | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> *(file list)* | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> *(file list)* |
| Batch rename… | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>R</kbd> *(file list)* | — | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>R</kbd> *(file list)* |
| New folder | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>N</kbd> *(file list)* | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>N</kbd> *(file list)* | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>N</kbd> *(file list)* |
| Properties | — | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>Enter</kbd> *(file list)* | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>Enter</kbd> *(file list)* |
| Copy path | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> *(file list)* | — | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> *(file list)* |
| Redo | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd> | — | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd> |
| Command palette | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd> | — | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd> |
| Search | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd> | — | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd> |
| Settings | <kbd>Alt</kbd>+<kbd>,</kbd> | <kbd>Alt</kbd>+<kbd>,</kbd> | <kbd>Alt</kbd>+<kbd>,</kbd> |
| Keyboard shortcuts | <kbd>Alt</kbd>+<kbd>/</kbd> | — | <kbd>Alt</kbd>+<kbd>/</kbd> |

Inside Windows Terminal the terminal UI also lists a working chord first in menus, hints and
the palette. These default chords never reach it there: <kbd>Alt</kbd>+<kbd>Enter</kbd>, <kbd>Ctrl</kbd>+<kbd>,</kbd>, <kbd>Ctrl</kbd>+<kbd>-</kbd>, <kbd>Ctrl</kbd>+<kbd>0</kbd>, <kbd>Ctrl</kbd>+<kbd>=</kbd>, <kbd>Ctrl</kbd>+<kbd>+</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>1</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>2</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>3</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>4</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>5</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>N</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Tab</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd>, <kbd>Ctrl</kbd>+<kbd>Tab</kbd>, <kbd>Ctrl</kbd>+<kbd>V</kbd>. Unbind them in Windows Terminal's settings (Actions) to use them in Soyuz. Windows Terminal's
paste chords paste text; under WSL a paste into the file list runs Soyuz's Paste.

## Commander (F-keys) preset

Total Commander / Norton Commander F-keys for two-pane work. Only the bindings that differ from the Explorer preset are listed.

| Action | Explorer | Commander (F-keys) |
|---|---|---|
| Refresh | <kbd>Ctrl</kbd>+<kbd>R</kbd>, <kbd>F5</kbd> | <kbd>Ctrl</kbd>+<kbd>R</kbd> |
| Focus other pane | — | <kbd>Tab</kbd> *(file list)* |
| Toggle dual pane | <kbd>F9</kbd> | — |
| Swap panes | — | <kbd>Ctrl</kbd>+<kbd>U</kbd> |
| Toggle mark | — | <kbd>Space</kbd> *(file list)* |
| Mark and move down | — | <kbd>Ins</kbd> *(file list)* |
| Filter this folder | <kbd>Ctrl</kbd>+<kbd>F</kbd> | <kbd>Ctrl</kbd>+<kbd>F</kbd>, <kbd>Ctrl</kbd>+<kbd>S</kbd> *(file list)* |
| Rename | <kbd>F2</kbd> *(file list)* | <kbd>Shift</kbd>+<kbd>F6</kbd> *(file list)* |
| New folder | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>N</kbd> *(file list)* | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>N</kbd> *(file list)*, <kbd>F7</kbd> *(file list)* |
| Move to trash | <kbd>Del</kbd> *(file list)* | <kbd>Del</kbd> *(file list)*, <kbd>F8</kbd> *(file list)* |
| Search | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd>, <kbd>F3</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd>, <kbd>Alt</kbd>+<kbd>F7</kbd> |
| Quick Look | <kbd>Space</kbd> *(file list)* | <kbd>F3</kbd> *(file list)* |

## Finder preset

macOS Finder conventions (Cmd is Ctrl on Windows and Linux). Only the bindings that differ from the Explorer preset are listed.

| Action | Explorer | Finder |
|---|---|---|
| Back | <kbd>Alt</kbd>+<kbd>←</kbd> | <kbd>Alt</kbd>+<kbd>←</kbd>, <kbd>Ctrl</kbd>+<kbd>[</kbd> |
| Forward | <kbd>Alt</kbd>+<kbd>→</kbd> | <kbd>Alt</kbd>+<kbd>→</kbd>, <kbd>Ctrl</kbd>+<kbd>]</kbd> |
| Up | <kbd>Alt</kbd>+<kbd>↑</kbd>, <kbd>Backspace</kbd> *(file list)* | <kbd>Alt</kbd>+<kbd>↑</kbd>, <kbd>Ctrl</kbd>+<kbd>↑</kbd> |
| Home | — | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>H</kbd> |
| Go to path… | — | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>G</kbd> |
| Refresh | <kbd>Ctrl</kbd>+<kbd>R</kbd>, <kbd>F5</kbd> | <kbd>Ctrl</kbd>+<kbd>R</kbd> |
| Go to tab 1 | <kbd>Ctrl</kbd>+<kbd>1</kbd> | — |
| Go to tab 2 | <kbd>Ctrl</kbd>+<kbd>2</kbd> | — |
| Go to tab 3 | <kbd>Ctrl</kbd>+<kbd>3</kbd> | — |
| Go to tab 4 | <kbd>Ctrl</kbd>+<kbd>4</kbd> | — |
| Go to tab 5 | <kbd>Ctrl</kbd>+<kbd>5</kbd> | — |
| Go to tab 6 | <kbd>Ctrl</kbd>+<kbd>6</kbd> | — |
| Go to tab 7 | <kbd>Ctrl</kbd>+<kbd>7</kbd> | — |
| Go to tab 8 | <kbd>Ctrl</kbd>+<kbd>8</kbd> | — |
| Go to last tab | <kbd>Ctrl</kbd>+<kbd>9</kbd> | — |
| View: Details | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>1</kbd> | <kbd>Ctrl</kbd>+<kbd>2</kbd> |
| View: Grid | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>2</kbd> | <kbd>Ctrl</kbd>+<kbd>1</kbd> |
| View: List | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>3</kbd> | <kbd>Ctrl</kbd>+<kbd>5</kbd> |
| View: Columns (Miller) | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>4</kbd> | <kbd>Ctrl</kbd>+<kbd>3</kbd> |
| View: Gallery | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>5</kbd> | <kbd>Ctrl</kbd>+<kbd>4</kbd> |
| Toggle hidden files | <kbd>Ctrl</kbd>+<kbd>H</kbd> | <kbd>Ctrl</kbd>+<kbd>H</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>.</kbd> |
| Open | <kbd>Enter</kbd> *(file list)* | <kbd>Ctrl</kbd>+<kbd>↓</kbd> *(file list)*, <kbd>Ctrl</kbd>+<kbd>O</kbd> *(file list)* |
| Copy to other pane | <kbd>F5</kbd> *(file list)* | — |
| Move to other pane | <kbd>F6</kbd> *(file list)* | — |
| Rename | <kbd>F2</kbd> *(file list)* | <kbd>Enter</kbd> *(file list)* |
| Move to trash | <kbd>Del</kbd> *(file list)* | <kbd>Del</kbd> *(file list)*, <kbd>Ctrl</kbd>+<kbd>Backspace</kbd> *(file list)* |
| Properties | <kbd>Alt</kbd>+<kbd>Enter</kbd> *(file list)* | <kbd>Alt</kbd>+<kbd>Enter</kbd> *(file list)*, <kbd>Ctrl</kbd>+<kbd>I</kbd> *(file list)* |
| Copy path | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> *(file list)* | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> *(file list)*, <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>C</kbd> *(file list)* |
| Search | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd>, <kbd>F3</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd> |

## Vim-ish preset

Vim / ranger-style letter keys and two-key sequences. Only the bindings that differ from the Explorer preset are listed.

| Action | Explorer | Vim-ish |
|---|---|---|
| Back | <kbd>Alt</kbd>+<kbd>←</kbd> | <kbd>Alt</kbd>+<kbd>←</kbd>, <kbd>Shift</kbd>+<kbd>H</kbd> *(file list)* |
| Forward | <kbd>Alt</kbd>+<kbd>→</kbd> | <kbd>Alt</kbd>+<kbd>→</kbd>, <kbd>Shift</kbd>+<kbd>L</kbd> *(file list)* |
| Up | <kbd>Alt</kbd>+<kbd>↑</kbd>, <kbd>Backspace</kbd> *(file list)* | <kbd>Alt</kbd>+<kbd>↑</kbd>, <kbd>H</kbd> *(file list)*, <kbd>Backspace</kbd> *(file list)* |
| Home | — | <kbd>G</kbd> <kbd>H</kbd> *(file list)* |
| Refresh | <kbd>Ctrl</kbd>+<kbd>R</kbd>, <kbd>F5</kbd> | <kbd>Ctrl</kbd>+<kbd>R</kbd> |
| Next tab | <kbd>Ctrl</kbd>+<kbd>Tab</kbd> | <kbd>Ctrl</kbd>+<kbd>Tab</kbd>, <kbd>G</kbd> <kbd>T</kbd> *(file list)* |
| Prev tab | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Tab</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Tab</kbd>, <kbd>G</kbd> <kbd>Shift</kbd>+<kbd>T</kbd> *(file list)* |
| Toggle hidden files | <kbd>Ctrl</kbd>+<kbd>H</kbd> | <kbd>Ctrl</kbd>+<kbd>H</kbd>, <kbd>Z</kbd> <kbd>H</kbd> *(file list)* |
| Invert selection | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>I</kbd> *(file list)* | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>I</kbd> *(file list)*, <kbd>V</kbd> *(file list)* |
| Mark and move down | — | <kbd>Space</kbd> *(file list)* |
| Cursor up | <kbd>↑</kbd> *(file list)* | <kbd>↑</kbd> *(file list)*, <kbd>K</kbd> *(file list)* |
| Cursor down | <kbd>↓</kbd> *(file list)* | <kbd>↓</kbd> *(file list)*, <kbd>J</kbd> *(file list)* |
| Cursor home | <kbd>Home</kbd> *(file list)* | <kbd>Home</kbd> *(file list)*, <kbd>G</kbd> <kbd>G</kbd> *(file list)* |
| Cursor end | <kbd>End</kbd> *(file list)* | <kbd>End</kbd> *(file list)*, <kbd>Shift</kbd>+<kbd>G</kbd> *(file list)* |
| Extend up | <kbd>Shift</kbd>+<kbd>↑</kbd> *(file list)* | <kbd>Shift</kbd>+<kbd>↑</kbd> *(file list)*, <kbd>Shift</kbd>+<kbd>K</kbd> *(file list)* |
| Extend down | <kbd>Shift</kbd>+<kbd>↓</kbd> *(file list)* | <kbd>Shift</kbd>+<kbd>↓</kbd> *(file list)*, <kbd>Shift</kbd>+<kbd>J</kbd> *(file list)* |
| Filter this folder | <kbd>Ctrl</kbd>+<kbd>F</kbd> | <kbd>Ctrl</kbd>+<kbd>F</kbd>, <kbd>/</kbd> *(file list)* |
| Open | <kbd>Enter</kbd> *(file list)* | <kbd>Enter</kbd> *(file list)*, <kbd>L</kbd> *(file list)* |
| Copy | <kbd>Ctrl</kbd>+<kbd>C</kbd> *(file list)* | <kbd>Ctrl</kbd>+<kbd>C</kbd> *(file list)*, <kbd>Y</kbd> <kbd>Y</kbd> *(file list)* |
| Cut | <kbd>Ctrl</kbd>+<kbd>X</kbd> *(file list)* | <kbd>Ctrl</kbd>+<kbd>X</kbd> *(file list)*, <kbd>D</kbd> <kbd>D</kbd> *(file list)* |
| Paste | <kbd>Ctrl</kbd>+<kbd>V</kbd> *(file list)* | <kbd>Ctrl</kbd>+<kbd>V</kbd> *(file list)*, <kbd>P</kbd> <kbd>P</kbd> *(file list)* |
| Paste as link | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> *(file list)* | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> *(file list)*, <kbd>P</kbd> <kbd>L</kbd> *(file list)* |
| Copy to other pane | <kbd>F5</kbd> *(file list)* | — |
| Move to other pane | <kbd>F6</kbd> *(file list)* | — |
| Rename | <kbd>F2</kbd> *(file list)* | <kbd>F2</kbd> *(file list)*, <kbd>C</kbd> <kbd>W</kbd> *(file list)* |
| New folder | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>N</kbd> *(file list)* | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>N</kbd> *(file list)*, <kbd>G</kbd> <kbd>N</kbd> *(file list)* |
| Move to trash | <kbd>Del</kbd> *(file list)* | <kbd>Del</kbd> *(file list)*, <kbd>D</kbd> <kbd>Del</kbd> *(file list)* |
| Copy path | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> *(file list)* | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> *(file list)*, <kbd>Y</kbd> <kbd>P</kbd> *(file list)* |
| Copy name | — | <kbd>Y</kbd> <kbd>N</kbd> *(file list)* |
| Undo | <kbd>Ctrl</kbd>+<kbd>Z</kbd> | <kbd>Ctrl</kbd>+<kbd>Z</kbd>, <kbd>U</kbd> *(file list)* |
| Command palette | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd>, <kbd>Ctrl</kbd>+<kbd>K</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd>, <kbd>Ctrl</kbd>+<kbd>K</kbd>, <kbd>Shift</kbd>+<kbd>;</kbd> *(file list)* |
| Search | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd>, <kbd>F3</kbd> | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd> |
| Quick Look | <kbd>Space</kbd> *(file list)* | <kbd>I</kbd> *(file list)* |

## Not bound by default

Every other action (folder tools, checksums, sort and group choices, AI commands, …) has no
default shortcut. Run it from the command palette, or bind it in Settings → Keyboard.
