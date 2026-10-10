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

### Navigation

| Action | Shortcut | What it does |
|---|---|---|
| Back | <kbd>Alt</kbd>+<kbd>←</kbd> | Go back in this pane's history |
| Forward | <kbd>Alt</kbd>+<kbd>→</kbd> | Go forward in this pane's history |
| Up | <kbd>Alt</kbd>+<kbd>↑</kbd>, <kbd>Backspace</kbd> *(file list)* | Go to the parent folder |
| Focus address bar | <kbd>Ctrl</kbd>+<kbd>L</kbd> | Edit the path in the address bar |
| Refresh | <kbd>Ctrl</kbd>+<kbd>R</kbd>, <kbd>F5</kbd> | Reload the folder listing |
| Toggle sidebar | <kbd>Ctrl</kbd>+<kbd>B</kbd> | Show or hide the sidebar |
| Next tab | <kbd>Ctrl</kbd>+<kbd>Tab</kbd> | Switch to the next tab |
| Prev tab | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Tab</kbd> | Switch to the previous tab |
| Go to folder… | <kbd>Ctrl</kbd>+<kbd>P</kbd> | Fuzzy-jump to a recent or frequent folder |
| Open bookmark 1–9 | <kbd>Alt</kbd>+<kbd>1</kbd> … <kbd>Alt</kbd>+<kbd>9</kbd> | Navigate to a configured bookmark |
| Go to tab 1–8 | <kbd>Ctrl</kbd>+<kbd>1</kbd> … <kbd>Ctrl</kbd>+<kbd>8</kbd> | Switch to a tab by position |
| Go to last tab | <kbd>Ctrl</kbd>+<kbd>9</kbd> | Switch to a tab by position |

### Tabs & panes

| Action | Shortcut | What it does |
|---|---|---|
| New tab | <kbd>Ctrl</kbd>+<kbd>T</kbd> | Open a new tab |
| Close tab | <kbd>Ctrl</kbd>+<kbd>W</kbd> | Close the current tab |
| Reopen closed tab | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>T</kbd> | Reopen the most recently closed tab |
| Toggle dual pane | <kbd>F9</kbd> | Show or hide the second pane |
| New window | <kbd>Ctrl</kbd>+<kbd>N</kbd> | Open a new Soyuz window |
| Toggle quad layout (2×2) | <kbd>Shift</kbd>+<kbd>F9</kbd> | Show four panes in a 2×2 grid |
| Workspaces… | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>W</kbd> | Open, save and manage named workspaces |

### View

| Action | Shortcut | What it does |
|---|---|---|
| View: Details | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>1</kbd> | Show the folder as a detailed table |
| View: Grid | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>2</kbd> | Show icons or thumbnails in a grid |
| View: List | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>3</kbd> | Show a compact multi-column list |
| View: Columns (Miller) | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>4</kbd> | Browse with cascading columns |
| View: Gallery | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>5</kbd> | Large preview with a filmstrip |
| Larger icons | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>+</kbd>, <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>=</kbd> | Make icons and thumbnails larger |
| Smaller icons | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>-</kbd> | Make icons and thumbnails smaller |
| Reset icon size | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>0</kbd> | Reset the icon size |
| Zoom in (interface) | <kbd>Ctrl</kbd>+<kbd>+</kbd>, <kbd>Ctrl</kbd>+<kbd>=</kbd> | Scale the whole interface up |
| Zoom out (interface) | <kbd>Ctrl</kbd>+<kbd>-</kbd> | Scale the whole interface down |
| Reset interface zoom | <kbd>Ctrl</kbd>+<kbd>0</kbd> | Back to 100 % interface scale |
| Toggle hidden files | <kbd>Ctrl</kbd>+<kbd>H</kbd> | Show or hide hidden files |
| Toggle preview pane | <kbd>Alt</kbd>+<kbd>P</kbd> | Show or hide the preview pane |
| Toggle terminal | <kbd>Ctrl</kbd>+<kbd>&#96;</kbd> | Show or hide the integrated terminal |
| Toggle activity panel | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>J</kbd> | Show or hide running jobs |
| Toggle AI assistant | <kbd>Ctrl</kbd>+<kbd>I</kbd> | Show or hide the AI assistant panel |
| Toggle disk tree | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>E</kbd> | Switch the sidebar between places and the disk tree |

### Selection

| Action | Shortcut | What it does |
|---|---|---|
| Select all | <kbd>Ctrl</kbd>+<kbd>A</kbd> *(file list)* | Select every item |
| Select none | <kbd>Esc</kbd> *(file list)* | Clear the selection |
| Invert selection | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>I</kbd> *(file list)* | Select everything that isn't selected |
| Cursor up | <kbd>↑</kbd> *(file list)* | Move the cursor up |
| Cursor down | <kbd>↓</kbd> *(file list)* | Move the cursor down |
| Cursor page up | <kbd>PageUp</kbd> *(file list)* | Move the cursor one page up |
| Cursor page down | <kbd>PageDown</kbd> *(file list)* | Move the cursor one page down |
| Cursor home | <kbd>Home</kbd> *(file list)* | Jump to the first item |
| Cursor end | <kbd>End</kbd> *(file list)* | Jump to the last item |
| Extend up | <kbd>Shift</kbd>+<kbd>↑</kbd> *(file list)* | Extend the selection upwards |
| Extend down | <kbd>Shift</kbd>+<kbd>↓</kbd> *(file list)* | Extend the selection downwards |
| Filter this folder | <kbd>Ctrl</kbd>+<kbd>F</kbd> | Filter the list as you type |

### File

| Action | Shortcut | What it does |
|---|---|---|
| Open | <kbd>Enter</kbd> *(file list)* | Open the selection |
| Edit in Soyuz | <kbd>F4</kbd> *(file list)* | Open the file in the built-in text, hex or image editor |
| Open in new tab | <kbd>Ctrl</kbd>+<kbd>Enter</kbd> *(file list)* | Open the selected folders in new tabs |
| Copy | <kbd>Ctrl</kbd>+<kbd>C</kbd> *(file list)* | Copy the selection to the clipboard |
| Cut | <kbd>Ctrl</kbd>+<kbd>X</kbd> *(file list)* | Cut the selection to the clipboard |
| Paste | <kbd>Ctrl</kbd>+<kbd>V</kbd> *(file list)* | Paste the clipboard into this folder |
| Paste as link | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> *(file list)* | Paste the clipboard as links |
| Copy to other pane | <kbd>F5</kbd> *(file list)* | Copy the selection to the other pane |
| Move to other pane | <kbd>F6</kbd> *(file list)* | Move the selection to the other pane |
| Duplicate | <kbd>Ctrl</kbd>+<kbd>D</kbd> *(file list)* | Make a copy next to the original |
| Rename | <kbd>F2</kbd> *(file list)* | Rename the item under the cursor |
| Batch rename… | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>R</kbd> *(file list)* | Rename many items with patterns |
| New folder | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>N</kbd> *(file list)* | Create a new folder here |
| New file | <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>N</kbd> *(file list)* | Create a new empty file here |
| Move to trash | <kbd>Del</kbd> *(file list)* | Move the selection to the trash |
| Delete permanently | <kbd>Shift</kbd>+<kbd>Del</kbd> *(file list)* | Delete without the trash |
| Properties | <kbd>Alt</kbd>+<kbd>Enter</kbd> *(file list)* | Show size, dates and attributes |
| Copy path | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> *(file list)* | Copy the full path to the clipboard |
| Undo | <kbd>Ctrl</kbd>+<kbd>Z</kbd> | Undo the last file operation |
| Redo | <kbd>Ctrl</kbd>+<kbd>Y</kbd>, <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>Z</kbd> | Redo the last undone operation |

### Tools

| Action | Shortcut | What it does |
|---|---|---|
| Command palette | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd>, <kbd>Ctrl</kbd>+<kbd>K</kbd> | Search and run any command |
| Search | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>F</kbd>, <kbd>F3</kbd> | Search this folder and its subfolders |
| Settings | <kbd>Ctrl</kbd>+<kbd>,</kbd> | Open the settings |
| Keyboard shortcuts | <kbd>Ctrl</kbd>+<kbd>/</kbd> | Browse and edit keyboard shortcuts |
| Quit | <kbd>Ctrl</kbd>+<kbd>Q</kbd> | Quit Soyuz |
| Quick Look | <kbd>Space</kbd> *(file list)* | Preview the item in a popup |

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
