### Soyuz UI strings (en-US source): settings
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/settings.ftl.

# Language picker: "Automatic (Türkçe)"; $lang = the detected OS language's own name.
settings-language-auto = Automatic ({ $lang })
settings-general-language = Language
settings-general-language-desc = Language of menus, dialogs and messages, and how dates and numbers are written. Applies immediately.

## Window chrome

settings-title = Settings
settings-close = Close settings
settings-close-tooltip = Close (Esc)
settings-search-hint = Search settings
# $query = the search text.
settings-search-results = Results for “{ $query }”
settings-search-empty = No settings match
settings-status-saving = Saving…
settings-status-saved = Saved to config.toml
settings-status-live = Changes apply instantly
settings-edit-config = Edit config.toml
settings-reset-section = Reset section
settings-reset-section-tooltip = Restore the defaults of this section
# $section = section title.
settings-reset-section-done = { $section } reset to defaults
settings-save-failed = Couldn't save config.toml

## Sections (sidebar title and the description under the page header)

settings-section-general = General
settings-section-general-desc = Startup, hidden files, sizes and dates.
settings-section-appearance = Appearance
settings-section-appearance-desc = Theme, accent color, density and fonts. Changes preview live.
settings-section-fonts = Fonts
settings-section-fonts-desc = Fonts for the interface, headings, terminal, code, hex view and numbers.
settings-section-views = Views & layout
settings-section-views-desc = Default view, columns, sorting and window layout.
settings-section-behaviour = Behaviour
settings-section-behaviour-desc = Clicking, confirmations, drag and drop and typing.
settings-section-jobs = File operations
settings-section-jobs-desc = Copy / move engine, verification, sync schedules and previous versions.
settings-section-terminal = Terminal
settings-section-terminal-desc = The integrated terminal panel.
settings-section-terminal-ui = Terminal UI
settings-section-terminal-ui-desc = The text-mode interface that runs inside a terminal (soyuz --tui).
settings-section-preview = Preview & search
settings-section-preview-desc = Preview limits, the FFmpeg libraries, search defaults and git integration.
settings-section-editor = Editor
settings-section-editor-desc = The built-in text, hex and image editors (Edit in Soyuz, F4).
settings-section-keyboard = Keyboard
settings-section-keyboard-desc = Presets and every shortcut. Click a shortcut to change it.
settings-section-commands = Custom commands
settings-section-commands-desc = Your own commands with placeholders, in the palette and context menu.
settings-section-bookmarks = Bookmarks
settings-section-bookmarks-desc = Folders you jump to often, with optional hotkeys.
settings-section-scripts = Scripts
settings-section-scripts-desc = Rhai scripts: actions, context-menu entries, columns and previewers.
settings-section-automations = Automations
settings-section-automations-desc = Rules that file new items in watched folders, and the soyuz command line.
settings-section-ai = AI assistant
settings-section-ai-desc = Model providers, API keys, privacy, scope, budgets and MCP connections.
settings-section-advanced = Advanced
settings-section-advanced-desc = Edit, export or import config.toml, and make Soyuz your default file manager.
settings-section-about = About & updates
settings-section-about-desc = Version, and checking GitHub Releases for a newer Soyuz (only when you ask).

## Export / import

settings-export-title = Export settings
settings-import-title = Import settings
settings-exported = Settings exported
settings-export-failed = Export failed
settings-imported = Settings imported
settings-imported-warnings = Settings imported with warnings
settings-import-failed = Import failed
settings-theme-import-title = Import theme
settings-theme-export-title = Export theme
settings-theme-exported = Theme exported
settings-theme-export-failed = Theme export failed
settings-theme-import-failed = Theme import failed
# $name = theme name.
settings-theme-imported = Theme “{ $name }” imported
settings-theme-imported-warnings = Theme “{ $name }” imported with warnings
settings-theme-invalid = Not a valid theme file

## General

# $sample = 1,288,490,189 bytes formatted in the chosen style.
settings-general-sizes-desc = 1.2 GB file shows as { $sample }
settings-general-dates = Dates
# $preview = a time from yesterday in the chosen format.
settings-general-dates-desc = Yesterday's file shows as “{ $preview }”

## General page

settings-general-startup = Startup
settings-general-start-in = Start in
settings-general-start-in-desc = Folder shown when Soyuz starts without a saved session.
settings-general-start-home = Home
settings-general-start-last = Last folder
settings-general-start-folder = Folder
settings-general-restore-tabs = Restore tabs on startup
settings-general-restore-tabs-desc = Reopen the tabs, panes and folders of the last session.
settings-general-files = Files
settings-linux = Linux desktop
settings-linux-window-backend = Window system
settings-linux-window-backend-desc = Automatic uses Wayland when available, else X11. X11 runs through XWayland under a Wayland compositor, which makes drag and drop with X11 apps work. Takes effect after a restart.
settings-linux-file-manager1 = Answer “Show in folder”
settings-linux-file-manager1-desc = While Soyuz runs, it owns the org.freedesktop.FileManager1 D-Bus service, so “Show in folder” in browsers and other apps opens Soyuz.
settings-linux-clipboard-tools = Clipboard helper programs
settings-linux-clipboard-tools-desc = When the built-in clipboard fails, run wl-copy or xclip instead.
settings-general-show-hidden = Show hidden files
settings-general-show-hidden-desc = Dotfiles and items with the hidden attribute (Ctrl+H toggles).
settings-general-show-hidden-desc-unix = Dotfiles and the names listed in a folder's .hidden file (Ctrl+H toggles).
settings-general-dot-hidden = Honour .hidden files
settings-general-dot-hidden-desc = Also hide the names listed in a folder's .hidden file, as GNOME Files and Dolphin do.
settings-general-show-system = Show system files
settings-general-show-system-desc = Windows protected operating-system files.
settings-general-show-ext = Show file extensions
settings-general-show-ext-desc = Display “.txt”, “.rs” … after names.
settings-general-strict-names = Strict Windows names
settings-general-strict-names-desc = Apply Windows file-name rules (no <>:"|?*, reserved names) on every OS.
settings-general-strict-names-desc-unix = Apply Windows file-name rules (no <>:"|?*, reserved names) everywhere. Drives formatted FAT, exFAT or NTFS get these rules automatically.
settings-general-formats = Formats
settings-general-sizes = File sizes
settings-general-sizes-bytes = Bytes
settings-general-date-relative = Relative
settings-general-date-short = Short
settings-general-date-long = Long
settings-general-date-custom = Custom…

## Views & layout page

settings-view-details = Details
settings-view-grid = Grid
settings-view-list = List
settings-view-columns = Columns
settings-view-gallery = Gallery
settings-views-folders = Folders
settings-views-default-view = Default view
settings-views-default-view-desc = Used for folders you haven't customized.
settings-views-folders-first = Folders first
settings-views-folders-first-desc = Keep folders above files when sorting.
settings-views-natural-sort = Natural sorting
settings-views-natural-sort-desc = file2 comes before file10.
settings-views-case-sort = Case-insensitive sorting
settings-views-case-sort-desc = “apple” and “Apple” sort together.
settings-views-remember = Remember view per folder
settings-views-remember-desc = Each folder keeps its own view, sort and grouping.
settings-views-details-columns = Details columns
settings-views-details-columns-desc = Columns of the details view for folders without their own layout. Drag widths in the view itself.
settings-views-hide-column = Hide column
settings-views-add-column = Add:
settings-views-window-layout = Window layout
settings-views-sidebar = Sidebar
settings-views-sidebar-desc = Favorites, devices, portal and the disk tree.
settings-views-sidebar-content = Sidebar content
settings-views-sidebar-content-desc = Places sections, the disk tree of your drives and home folder filling the sidebar, or both with the tree as a section. The icon bar at the top of the sidebar and Ctrl+Shift+E switch between places and the disk tree.
settings-views-sidebar-places = Places
settings-views-folder-tree = Disk tree
settings-views-sidebar-both = Both
settings-views-tree-follow = Tree follows the current folder
settings-views-tree-follow-desc = Expand the disk tree to the active pane's folder and highlight it.
settings-views-tree-drag = Expand tree on drag hover
settings-views-tree-drag-desc = Hold a drag over a collapsed folder to open it. 0 turns this off.
settings-views-status-bar = Status bar
settings-views-status-bar-desc = Item counts, selection size and free space.
settings-views-preview-pane = Preview pane
settings-views-preview-pane-desc = Show the preview pane at startup.
settings-views-dual-pane = Dual pane at startup
settings-views-dual-pane-desc = Open new windows with two panes.
settings-views-dual-orientation = Dual pane orientation
settings-views-side-by-side = Side by side
settings-views-stacked = Stacked
settings-views-tab-strip = Tab strip
settings-always = Always
settings-views-tab-strip-multiple = With 2+ tabs
settings-views-activity-panel = Activity panel
settings-views-activity-panel-desc = Where running file operations are shown.
settings-views-activity-right = Right
settings-views-activity-bottom = Bottom
settings-views-activity-floating = Floating
settings-views-terminal-height = Terminal height
settings-views-terminal-height-desc = Default height of the bottom panel.
settings-views-windows = Windows & workspaces
settings-views-new-window = New window opens
settings-views-new-window-desc = Ctrl+N: another window of this Soyuz (shares jobs and settings), or a separate process.
settings-views-new-window-window = Window
settings-views-new-window-process = Process
settings-views-tear-off = Tear tabs off
settings-views-tear-off-desc = Drag a tab out of the strip to open it in its own window.
settings-views-tear-off-distance = Tear-off distance
settings-views-tear-off-distance-desc = How far a tab has to leave the strip first.
settings-views-close-window = Closing a window with tabs
settings-views-close-window-desc = Move its tabs to another window, close them, or ask each time.
settings-ask = Ask
settings-views-move-tabs = Move tabs
settings-views-close-tabs = Close tabs
settings-views-restore-windows = Restore windows
settings-views-restore-windows-desc = Reopen the other windows, where they were, with the session.
settings-views-sync-relative = Relative synchronized browsing
settings-views-sync-relative-desc = Synced panes follow sibling and multi-level moves, not only into a child folder or up.
settings-views-history-graph = History graph size
settings-views-history-graph-desc = Places remembered per tab for “Where was I?”.
settings-views-timeline-range = Timeline range
settings-views-timeline-range-desc = Days of changes the Timeline shows (at least this week).
settings-views-timeline-folders = Timeline folders
settings-views-timeline-folders-desc = Recently visited folders scanned for changed files.
settings-views-timeline-items = Timeline items
settings-views-timeline-items-desc = The most files the Timeline lists.
settings-views-timeline-index = Timeline uses the search index
settings-views-timeline-index-desc = Also ask the search index, when one is available, for changed files.
settings-views-sidebar-sections = Sidebar sections
settings-views-sidebar-sections-desc = Choose and order the sections of the sidebar.
settings-views-gtk-bookmarks = Show GTK bookmarks
settings-views-gtk-bookmarks-desc = Show the GTK bookmarks (~/.config/gtk-3.0/bookmarks, shared with Nautilus and other GTK file managers) under Favorites.
settings-views-tag-names = Tag names
settings-views-tag-names-desc = Give the color tags your own names. Items stay tagged by color, so renaming changes nothing on disk. Leave a field empty to use the color name.
settings-views-tags-xdg = Share tags with other apps
settings-views-tags-xdg-desc = Mirror tags to the user.xdg.tags extended attribute so Dolphin and other apps see them.
settings-views-section-favorites = Favorites
settings-views-section-devices = Devices
settings-views-section-cloud = Cloud folders
settings-views-section-tags = Tags
settings-views-section-network = Network
settings-views-section-trash = Other (Recent, Timeline, Trash)
settings-views-section-portal = Portal
settings-views-section-tree = Disk tree (only in “Both”)
settings-views-section-smart = Smart folders
settings-views-portal = Portal
settings-views-portal-desc = Folders pinned to the Portal section of the sidebar. Drop folders on the section to pin them; rename a label or leave it empty to use the folder's name.
settings-views-portal-empty = No pinned folders yet. Drag folders onto Portal in the sidebar.
settings-views-portal-unpin = Unpin
settings-views-portal-up = Move up
settings-views-portal-down = Move down
settings-views-toolbar = Toolbar & context menu
settings-views-toolbar-items = Toolbar items
settings-views-toolbar-items-desc = Action names (e.g. back, up, new_tab), plus | separator, spacer, breadcrumb, filter, view_menu, sort_menu, activity. One per line.
settings-views-ctx-hide = Hidden context-menu entries
settings-views-ctx-hide-desc = Action names to hide from right-click menus. One per line.

## Behaviour page

settings-behaviour-opening = Opening
settings-behaviour-single-click = Open items with a single click
settings-behaviour-single-click-desc = Otherwise a double click opens.
settings-behaviour-double-click = Double-click speed
settings-behaviour-double-click-desc = Maximum time between the two clicks.
settings-behaviour-open-folders-in = Open folders in
settings-behaviour-same-tab = Same tab
settings-behaviour-new-tab = New tab
settings-behaviour-middle-click = Middle click on a folder
settings-behaviour-other-pane = Other pane
settings-behaviour-nothing = Nothing
settings-behaviour-confirmations = Confirmations
settings-behaviour-confirm-trash = Confirm moving to trash
settings-behaviour-confirm-trash-desc = Ask before Delete sends items to the trash.
settings-behaviour-confirm-permanent = Confirm permanent delete
settings-behaviour-confirm-permanent-desc = Ask before Shift+Delete removes items for good.
settings-behaviour-paste-conflict = When pasted items already exist
settings-behaviour-overwrite = Overwrite
settings-skip = Skip
settings-behaviour-keep-both = Keep both
settings-behaviour-drag-drop = Drag and drop
settings-behaviour-drop-action = Default drop action
settings-behaviour-drop-action-desc = Ctrl copies, Shift moves, Alt links regardless of this.
settings-auto = Auto
settings-behaviour-drop-copy = Copy
settings-behaviour-drop-move = Move
settings-behaviour-spring = Spring-loaded folders
settings-behaviour-spring-desc = While dragging, hovering a folder, breadcrumb or tab opens it so you can drop deeper.
settings-behaviour-spring-delay = Spring-load delay
settings-behaviour-spring-delay-desc = How long to hover before the folder opens.
settings-behaviour-drag-out = Drag to other apps
settings-behaviour-drag-out-desc = Dragging items out of the window hands them to the desktop, Explorer or other apps.
settings-behaviour-drag-out-desc-unix = Dragging items out of the window hands them to the desktop, other file managers or other apps.
settings-behaviour-typing = Typing & renaming
settings-behaviour-type-ahead = Typing in the list
settings-behaviour-type-ahead-desc = Jump to the first match, or filter the list.
settings-behaviour-jump = Jump
settings-behaviour-filter = Filter
settings-behaviour-rename-selects = Rename selects
settings-behaviour-rename-selects-desc = What is selected when renaming starts (F2 cycles).
settings-behaviour-name-only = Name only
settings-behaviour-everything = Everything
settings-behaviour-refreshing = Refreshing
settings-behaviour-auto-refresh = Refresh automatically
settings-behaviour-auto-refresh-desc = Watch open folders for changes made by other programs.
settings-behaviour-debounce = Change debounce
settings-behaviour-debounce-desc = How long to wait for a burst of changes to settle.
settings-behaviour-watch-poll = Polling interval
settings-behaviour-watch-poll-desc = When the system's file watch limit (fs.inotify.max_user_watches) is used up, open folders are checked for changes this often instead.

## File operations page

settings-remote-hosts-desc = { $count ->
    [one] { $count } host key trusted. Forget one to be asked again.
   *[other] { $count } host keys trusted. Forget one to be asked again.
}
settings-remote-shares = List shares for \\server

settings-jobs-queue = Queue
settings-jobs-parallel = Parallel jobs
settings-jobs-parallel-desc = How many copy / move / delete jobs run at once. Applies after restart.
settings-jobs-queue-mode = Queue mode
settings-jobs-queue-mode-desc = Per device runs one job per disk pair at a time, which is fastest on HDDs. Applies after restart.
settings-jobs-per-device = Per device
settings-jobs-parallel-mode = Parallel
settings-jobs-auto-activity = Open the activity panel automatically
settings-jobs-auto-activity-desc = When a job runs longer than a second.
settings-jobs-reset-apply-all = Reset “apply to all” between jobs
settings-jobs-reset-apply-all-desc = Conflict answers don't carry over to the next job. Applies after restart.
settings-jobs-history = History size
settings-jobs-history-desc = Completed operations kept for undo. Applies after restart.
settings-jobs-copying = Copying
settings-jobs-verify = Verify after copy
settings-jobs-verify-desc = Hash source and destination (BLAKE3) and compare.
settings-jobs-reflink = Copy-on-write clones
settings-jobs-reflink-desc = Instant copies on ReFS / Btrfs / APFS when possible.
settings-never = Never
settings-jobs-buffer = Copy buffer
settings-jobs-unbuffered = Unbuffered I/O above
settings-jobs-unbuffered-desc = Large files bypass the OS cache.
settings-jobs-preserve = Preserve
settings-jobs-preserve-desc = Metadata carried over to copies.
settings-jobs-preserve-times = Timestamps
settings-jobs-preserve-attrs = Attributes
settings-jobs-preserve-xattr = Extended attributes
settings-jobs-preserve-acl = ACLs
settings-jobs-preserve-owner = Owner
settings-jobs-preserve-sparse = Sparse files
settings-jobs-archive = Archive safety
settings-jobs-bomb-ratio = Expansion limit
settings-jobs-bomb-ratio-desc = Pause extraction when the output grows past this multiple of the archive's size (0 = off).
settings-jobs-bomb-floor = …but not below
settings-jobs-bomb-floor-desc = The expansion limit never pauses before this much output.
settings-jobs-max-entries = Most entries
settings-jobs-max-entries-desc = Pause extraction past this many entries (0 = no limit).
settings-jobs-max-entry = Largest entry
settings-jobs-max-entry-desc = Pause on a single entry larger than this (0 = no limit).
settings-jobs-zip-level = Compression when adding to a zip
settings-jobs-zip-level-desc = Deflate level for files pasted into a zip you are browsing (0 = store, 9 = smallest).
settings-jobs-special-bits = Keep setuid / setgid / sticky bits
settings-jobs-special-bits-desc = Off strips these special permission bits from extracted files (Linux / macOS).
settings-jobs-trash-unsupported = When the trash isn't available
settings-jobs-trash-unsupported-desc = e.g. network drives and FAT volumes.
settings-jobs-trash-delete = Delete
settings-jobs-sync = Sync & versions
settings-jobs-scheduler = Run scheduled sync profiles
settings-jobs-scheduler-desc = Profiles with a schedule run while Soyuz is open (no background service).
settings-jobs-dry-run = Scheduled runs are dry runs
settings-jobs-dry-run-desc = Only write a report of what would change; apply it yourself from Sync profiles.
settings-jobs-change-delay = “On change” delay
settings-jobs-change-delay-desc = Wait this long after the last change before syncing.
settings-jobs-reports = Reports kept per profile
settings-jobs-reports-desc = Run and dry-run reports.
settings-jobs-previous-versions = Previous versions
settings-jobs-previous-versions-desc = List earlier copies of files from Windows shadow copies (Properties, context menu).
settings-jobs-previous-versions-desc-linux = List earlier copies of files from Snapper, ZFS and Timeshift snapshots (Properties, context menu).
settings-jobs-shadow-probe = Shadow copies to probe
settings-jobs-shadow-probe-desc = Fallback when the kernel device list can't be read.
settings-remote = Remote locations
settings-remote-cache = Reuse folder listings for
settings-remote-cache-desc = Remote folders are listed again after this long (Refresh always re-lists).
settings-remote-poll = Check open remote folders every
settings-remote-poll-desc = 0 = only on Refresh.
settings-remote-connect-timeout = Connection timeout
settings-remote-connect-timeout-desc = Give up connecting after this long.
settings-remote-io-timeout = Server timeout
settings-remote-io-timeout-desc = Give up when a server stops answering mid-operation.
settings-remote-download-cache = Download cache
settings-remote-download-cache-desc = Remote files opened or previewed are kept in a temp folder up to this size.
settings-remote-preview-max = Preview files up to
settings-remote-preview-max-desc = Larger remote files are not downloaded just to preview them.
# $platform: "windows" | "other".
settings-remote-shares-desc = { $platform ->
    [windows] Typing a bare server name (\\server) lists its shared folders.
   *[other] Typing \\server or smb://server lists its shared folders (asked over SMB, signed in like any other server).
}
settings-remote-trusted = Trusted SSH servers
settings-remote-forget = Forget

## Terminal page

settings-terminal-shell-desc = A detected shell, Automatic, or Custom… for any program. Applies to new sessions.
settings-terminal-shell = Shell
settings-terminal-args = Arguments
settings-terminal-args-desc = Extra arguments for the shell.
settings-terminal-integration = Shell integration
settings-terminal-integration-desc = Inject a prompt hook so the terminal reports its folder.
settings-terminal-scrollback = Scrollback
settings-terminal-scrollback-desc = Lines kept per session.
settings-terminal-look = Look
settings-terminal-font-size = Font size
settings-terminal-colors = Colors
settings-terminal-colors-desc = Auto derives the palette from the app theme.
settings-dark = Dark
settings-light = Light
settings-terminal-following = Following
settings-terminal-follow-nav = Follow navigation
settings-terminal-follow-nav-desc = cd the terminal when the focused pane changes folder.
settings-terminal-follow-shell = Follow the shell
settings-terminal-follow-shell-desc = Navigate the pane when you cd in the shell.
settings-terminal-cd-tab = cd on tab switch
settings-terminal-per-tab = One terminal per tab
settings-terminal-per-tab-desc = Otherwise all tabs share the same sessions.
settings-terminal-esc = Esc returns focus
settings-terminal-esc-desc = Esc on an empty prompt goes back to the file list.
settings-terminal-security = Security
settings-terminal-osc52 = Programs setting the clipboard
settings-terminal-osc52-desc = OSC 52 lets terminal output replace your clipboard, e.g. with a command for you to paste. Reading the clipboard is always blocked.
settings-allow = Allow
settings-tui-input = Mouse and keyboard
settings-tui-mouse = Mouse
settings-tui-mouse-desc = Click to select and use the wheel to scroll. Turn it off to keep the terminal's own text selection working.
settings-tui-keyboard-protocol = Keyboard protocol
settings-tui-keyboard-protocol-desc = Auto uses the kitty keyboard protocol where the terminal supports it (never on Windows), so shortcuts like Ctrl+Shift+letter arrive exactly. Kitty always asks for it; Legacy never does.
settings-tui-legacy = Legacy
settings-tui-appearance = Appearance
settings-tui-color-mode = Colors
settings-tui-color-mode-desc = Auto detects what the terminal supports (COLORTERM, TERM, NO_COLOR). Pick a fixed mode when colors look wrong, e.g. over SSH or in an old console.
settings-tui-truecolor = True color (24-bit)
settings-tui-ansi256 = 256 colors
settings-tui-ansi16 = 16 colors
settings-tui-mono = No color
settings-tui-icons = Icons
settings-tui-icons-desc = Nerd Font icons need a Nerd Font set in the terminal. Unicode symbols work with every font; ASCII uses plain characters only.
settings-tui-icons-nerd = Nerd Font
settings-tui-icons-unicode = Unicode
settings-tui-icons-ascii = ASCII
settings-tui-borders = Borders
settings-tui-borders-desc = How panels are outlined. ASCII works in any terminal.
settings-tui-borders-rounded = Rounded
settings-tui-borders-plain = Square
settings-tui-borders-ascii = ASCII
settings-tui-sidebar-width = Sidebar width
settings-tui-sidebar-width-desc = Width of the sidebar in terminal columns (12–60).
settings-tui-jobs-rows = Jobs panel height
settings-tui-jobs-rows-desc = Height of the bottom panel with running and finished operations, in terminal rows (3–30). It hides when the terminal is too short.
settings-tui-preview-ratio = Preview width
settings-tui-preview-ratio-desc = Share of the width the preview pane takes (0.2–0.6); of the height when it sits below the files on a narrow terminal.
settings-tui-terminal-rows = Terminal height
settings-tui-terminal-rows-desc = Height of the embedded terminal pane in terminal rows (3–40). It hides when the terminal is too short.
settings-tui-ai-ratio = AI panel width
settings-tui-ai-ratio-desc = Share of the width the AI assistant panel takes beside the files (0.25–0.6). Below 100 columns the panel takes the whole area.
settings-tui-programs = Shell and editor
settings-tui-shell-mode = Terminal actions
settings-tui-shell-mode-desc = What toggling the terminal, "Open terminal here" and "Run in terminal" do. Embedded pane shows a terminal in the bottom panel; Suspend leaves the terminal UI and runs your shell in the real terminal until you type exit.
settings-tui-shell-mode-pane = Embedded pane
settings-tui-shell-mode-suspend = Suspend to shell
settings-tui-editor = Text editor
settings-tui-editor-desc = Program the terminal UI runs to edit the config file and other text, with its arguments (for example nvim or code --wait). Empty uses the VISUAL environment variable, then EDITOR, then the first of nvim, vim, nano, micro, hx and vi found on PATH.
settings-tui-images-group = Images
settings-tui-images = Images
settings-tui-images-desc = How previews and thumbnails show pictures. Auto asks the terminal; Kitty, Sixel and iTerm2 force a graphics protocol; Half blocks draws coloured text cells that work everywhere at a low resolution; Off shows no pictures.
settings-tui-images-halfblocks = Half blocks
settings-tui-cell-pixels = Cell size
settings-tui-cell-pixels-desc = Size of one terminal cell in pixels (width × height), used to scale sixel images when the terminal does not report it.
settings-tui-clipboard = Clipboard
settings-tui-osc52 = Copy through the terminal (OSC 52)
settings-tui-osc52-desc = Copied text is sent to the terminal, which puts it on the clipboard of the computer it runs on; this also works over SSH. Auto sends it over SSH or when the local clipboard cannot take the text; Always sends it with every copy; Never keeps copies on the local clipboard only.
settings-tui-accessibility = Accessibility
settings-tui-screen-reader = Screen-reader mode
settings-tui-screen-reader-desc = Draws plain text a screen reader can follow: no box drawing, one pane at a time, text markers instead of icons, the terminal cursor on the focused item and a status line that reads as a sentence. Start with --screen-reader to turn it on for one run only.
settings-deny = Deny

## Preview & search page

settings-preview = Preview
settings-preview-text-limit = Text preview limit
settings-preview-text-limit-desc = Larger files show the first part only.
settings-preview-image-limit = Image size limit
settings-preview-image-limit-desc = Larger images aren't decoded for previews.
settings-preview-syntax = Syntax theme
settings-preview-syntax-desc = Colors of highlighted code in previews and the editor.
settings-preview-pdf = PDF preview
settings-preview-pdf-desc = Pages render with pdfium.dll (not bundled; put it next to soyuz.exe or in the config folder). Without it, PDFs show their text.
settings-preview-pdf-desc-unix = Pages render with libpdfium.so (not bundled; put it next to the soyuz program, in the config folder or in a system library folder such as /usr/lib64). Without it, PDFs show their text.
settings-preview-pdfium = pdfium folder
settings-preview-pdfium-desc = Where pdfium.dll lives. Empty: next to soyuz.exe, then the config folder.
settings-preview-pdfium-desc-unix = Where libpdfium.so lives. Empty: next to the soyuz program, then the config folder, then the system library folders.
settings-preview-pdfium-hint = next to soyuz.exe
settings-preview-pdfium-hint-unix = next to the soyuz program
settings-preview-office = Office documents
settings-preview-office-desc = Word, Excel, PowerPoint and OpenDocument structure and text.
settings-preview-sheet-rows = Spreadsheet rows
settings-preview-sheet-rows-desc = Rows read from the first sheet.
settings-preview-epub = E-books
settings-preview-epub-desc = EPUB title, author, cover and contents.
settings-preview-doc-limit = Document size limit
settings-preview-doc-limit-desc = Office, EPUB and PDF files (and the parts inside them) above this aren't parsed.
settings-preview-fonts = Fonts
settings-preview-fonts-desc = A specimen and glyph grid for TTF, OTF, TTC and WOFF.
settings-preview-models = 3D models
settings-preview-models-desc = Quick render of glTF, GLB, STL and OBJ; drag to rotate.
settings-preview-spin = Spin models
settings-preview-spin-desc = Turn models slowly until you drag them (off with reduced motion).
settings-preview-model-limit = Model size limit
settings-preview-model-limit-desc = Larger model files aren't loaded.
settings-preview-waveform = Audio waveform
settings-preview-waveform-desc = Decode audio files to draw their waveform.
settings-preview-audio-limit = Audio size limit
settings-preview-audio-limit-desc = Larger audio files get no waveform.
settings-preview-playback = Audio playback
settings-preview-playback-desc = Play, pause and seek inside the preview.
settings-preview-video = Video previews
settings-preview-video-desc = Container metadata, plus the streams and a poster frame when the FFmpeg libraries are loaded (Media, below).

## Media (FFmpeg libraries) group of the Preview page

settings-media = Media
settings-media-status = FFmpeg libraries
settings-media-status-ok = Loaded
settings-media-status-missing-short = Not loaded
settings-media-status-loading-short = Loading…
settings-media-status-off = Off: videos show their container metadata only, audio plays through the built-in decoder.
settings-media-status-loading = Looking for the libraries…
# $version: "9.0 (avcodec 63.2.100)"; $dir: the folder; $decoders / $encoders: counts.
settings-media-status-loaded = { $version } from { $dir } · { $decoders } decoders, { $encoders } encoders.
settings-media-status-gpl = A GPL build: it works, but Soyuz can't be redistributed with it.
# $reason: the loader's message.
settings-media-status-missing = { $reason }.
# $tag: the GitHub release (ffmpeg-runtime-9.0.2); $dir: the config folder's ffmpeg/ sub-folder.
settings-media-status-get = Soyuz ships the libraries next to the program. To add or replace them, get the "{ $tag }" release from GitHub and unpack it into { $dir }, or choose their folder below.
# $dirs: one folder per line.
settings-media-status-searched = Searched:
    { $dirs }
settings-media-loader-path = (the system's library search path)
settings-media-retry = Try again
settings-media-ffmpeg = Use the FFmpeg libraries
settings-media-ffmpeg-desc = Video posters, thumbnails and stream details through FFmpeg when its libraries are found. Off: metadata-only video previews, no player or media tools.
settings-media-dir = Library folder
settings-media-dir-desc = Searched first for avcodec-63.dll and the other four. Empty: next to soyuz.exe, then the config folder's ffmpeg sub-folder. Applies to the next load (after a restart once libraries were loaded).
settings-media-dir-desc-unix = Searched first for libavcodec.so.63 / libavcodec.63.dylib and the other four. Empty: next to the soyuz program, then the config folder's ffmpeg sub-folder, then the system library folders. Applies to the next load (after a restart once libraries were loaded).
settings-media-dir-hint = next to soyuz.exe
settings-media-dir-hint-unix = next to the soyuz program
settings-media-browse = Choose…
# $file: the library file to pick (avcodec-63.dll).
settings-media-pick-title = Choose { $file } in the libraries' folder
settings-media-threads = Decoder threads
settings-media-threads-desc = Threads per decoded file; 0 uses one per core. Thumbnails always use one.
settings-media-max-px = Frame size
settings-media-max-px-desc = Longest edge of poster frames (and of the player's frames) in pixels; larger sources are scaled down while decoding.
settings-media-poster = Poster time
settings-media-poster-desc = Seconds into the video for the poster frame and the thumbnail, clamped to its length. 0: a tenth of the length, at most 5 s.
settings-media-audio-backend = Audio decoder
settings-media-audio-backend-desc = Which decoder analyses and plays audio previews. Auto: FFmpeg when its libraries are loaded (Opus, WMA, APE, AC-3, DSD, MKA and more), otherwise the built-in Symphonia decoder (mp3, flac, wav, ogg, aac, alac).
settings-media-backend-auto = Auto
settings-media-player = Video player
settings-media-player-desc = Play videos in the preview pane and Quick Look. Off: a poster frame only.
settings-media-autoplay = Autoplay
settings-media-autoplay-desc = Start playing as soon as a video is previewed.
settings-media-loop = Loop
settings-media-loop-desc = Restart from the beginning at the end.
settings-media-volume = Volume
settings-media-volume-desc = Player volume for videos and audio previews. Changed from the player too (written back here).
settings-media-muted = Muted
settings-media-muted-desc = Start muted. Toggled from the player too (written back here).
settings-media-seek-step = Seek step
settings-media-seek-step-desc = Seconds the player skips on Left / Right.
settings-media-tui-fps = Terminal frame rate
settings-media-tui-fps-desc = Video frames per second the terminal UI draws as pictures (applies to the next video opened).
settings-preview-diff = Compare two files
settings-preview-diff-desc = With exactly two text files selected, the preview shows their diff.
settings-preview-diff-limit = Diff size limit
settings-preview-diff-limit-desc = Each file is compared up to this size.
settings-preview-thumb-mem = Thumbnail memory
settings-preview-thumb-mem-desc = Decoded thumbnails kept in memory; the least recently shown go first. Current use is in About.
settings-preview-preview-mem = Preview memory
settings-preview-preview-mem-desc = Finished previews kept so going back to a file is instant. Current use is in About.
settings-search = Search
settings-search-threads = Search threads
settings-search-threads-desc = 0 uses every core.
settings-search-gitignore = Respect .gitignore
settings-search-gitignore-desc = Skip ignored files by default.
settings-search-content-limit = Content search limit
settings-search-content-limit-desc = Files larger than this are skipped when searching contents.
settings-search-saved = Saved searches from config.toml
settings-git-status-column = Git status column
settings-git-status-column-desc = Show modified / untracked markers in repositories.
settings-git-refresh = Refresh interval

## Advanced page

settings-advanced-config-desc = Every setting lives in one TOML file. Edits made in an editor apply within a second; parse errors show where they are and keep the previous settings.
settings-advanced-edit = Edit config file
settings-advanced-reload = Reload from disk
settings-advanced-copy = Copy as TOML
settings-advanced-transfer = Export & import
settings-advanced-export-desc = Save a copy of config.toml anywhere you like.
settings-advanced-export = Export…
settings-advanced-import-desc = Load a config file (replaces the current settings).
settings-advanced-import = Import…
settings-advanced-danger = Danger zone
settings-advanced-reset-all = Reset everything
settings-advanced-reset-all-desc = Restore every setting to its default.
settings-advanced-reset-all-button = Reset all settings
settings-advanced-reset-all-done = All settings reset to defaults

## Appearance: theme color tokens (labels of [theme.colors] keys)

settings-token-bg-window = Window background
settings-token-bg-surface1 = Sidebar, toolbars
settings-token-bg-surface2 = Hover rows, inputs, cards
settings-token-bg-surface3 = Popups, menus, dialogs
settings-token-bg-sunken = Terminal, hex view
settings-token-border-subtle = Separators
settings-token-border-strong = Focused controls
settings-token-text-primary = Primary text
settings-token-text-secondary = Secondary text
settings-token-text-tertiary = Hints, placeholders
settings-token-accent = Accent
settings-token-accent-hover = Accent (hover)
settings-token-accent-subtle = Selection fill
settings-token-success = Success
settings-token-warning = Warning
settings-token-danger = Danger
settings-token-info = Info
settings-token-glyph-folder = Folder icons
settings-token-glyph-image = Image icons
settings-token-glyph-video = Video icons
settings-token-glyph-audio = Audio icons
settings-token-glyph-archive = Archive icons
settings-token-glyph-code = Code icons
settings-token-glyph-doc = Document icons
settings-token-glyph-exe = Program icons
settings-token-tag-red = Tag red
settings-token-tag-orange = Tag orange
settings-token-tag-yellow = Tag yellow
settings-token-tag-green = Tag green
settings-token-tag-cyan = Tag cyan
settings-token-tag-blue = Tag blue
settings-token-tag-purple = Tag purple
settings-token-tag-gray = Tag gray

## Appearance page

# $version = the loaded FFmpeg libraries ("9.0 (avcodec 63.2.100)").
settings-appearance-video-thumbs-desc = A keyframe of each video through the FFmpeg libraries ({ $version }).
settings-appearance-video-thumbs-missing = A keyframe of each video through the FFmpeg libraries — they aren't loaded (Preview → Media says why), so videos show their length instead.
settings-appearance-overrides = { $count ->
    [one] { $count } override
   *[other] { $count } overrides
}

settings-appearance-theme = Theme
settings-appearance-color-theme = Color theme
settings-appearance-color-theme-desc = Each theme has a dark and a light variant. Theme files live in the themes folder; edits apply live.
settings-appearance-mode = Mode
settings-appearance-mode-desc = Follow the system dark / light switch live, or force dark or light.
settings-appearance-mode-system = System
settings-appearance-accent = Accent color
settings-appearance-accent-desc = Selection, focus rings, primary buttons and links. System follows the Windows accent color live.
settings-appearance-accent-desc-unix = Selection, focus rings, primary buttons and links. System follows the desktop's accent color.
settings-appearance-reduce-motion = Reduce motion
settings-appearance-reduce-motion-desc = Turn off animations (panels, dialogs, toasts).
settings-appearance-toast-time = Notification time
settings-appearance-toast-time-desc = How long notifications stay up, as a multiple of their standard time (info 4 s, warning 6 s, error 8 s). Hovering one pauses it.
settings-appearance-high-contrast = High contrast
settings-appearance-high-contrast-desc = Black / white surfaces, stronger text, borders and focus rings.
settings-appearance-density-text = Density & text
settings-appearance-density = Density
settings-appearance-density-desc = Compact, cozy, comfortable or touch: list rows of 24, 28, 32 or 40 px; touch also enlarges buttons and menus for fingers. Click a preview.
settings-appearance-font-size-desc = Base size of all UI text.
settings-appearance-scale = Interface scale
settings-appearance-scale-desc = Zoom the whole window (Ctrl+= / Ctrl+- / Ctrl+0; applies when you release the slider).
settings-appearance-fonts-desc = Interface, headings, terminal, code, hex and number fonts, with size offsets and previews.
settings-appearance-fonts-button = Fonts…
settings-appearance-fallback = Fallback fonts
settings-appearance-fallback-desc = System fonts for Chinese, Japanese, Korean, Arabic, Hebrew and emoji names, loaded in the background. Automatic: when a name needs them; at startup: also covers terminal output.
settings-automatic = Automatic
settings-appearance-fallback-startup = At startup
settings-off = Off
settings-appearance-icons = Icons & thumbnails
settings-appearance-file-icons = File icons
settings-appearance-file-icons-desc = Colored by type, or monochrome.
settings-appearance-colored = Colored
settings-appearance-mono = Mono
settings-appearance-badges = Extension badges
settings-appearance-badges-desc = A small colored label with the file extension on icons in the grid and gallery views.
settings-appearance-thumbs = Thumbnails
settings-appearance-thumbs-desc = Show image and video thumbnails instead of icons.
settings-appearance-folder-thumbs = Folder thumbnails
settings-appearance-folder-thumbs-desc = Grid views show a 2×2 mosaic of the first pictures inside each folder.
settings-appearance-video-thumbs = Video thumbnails
settings-appearance-pdf-thumbs = PDF thumbnails
settings-appearance-os-thumbs = Windows thumbnail cache
settings-appearance-os-thumbs-desc = Show the thumbnails Explorer already made (thumbcache_*.db, read-only) before decoding files. Also gives videos and PDFs a picture without the FFmpeg libraries or pdfium when Explorer has one.
settings-appearance-os-thumbs-linux = Shared thumbnail cache
settings-appearance-os-thumbs-desc-linux = Show the thumbnails other apps already made in the freedesktop thumbnail cache (~/.cache/thumbnails, read-only) before decoding files. Also gives videos and PDFs a picture without the FFmpeg libraries or pdfium when one exists.
settings-appearance-pdf-thumbs-desc = First page of PDFs through pdfium (pdfium.dll next to soyuz.exe, in the config folder or in the folder set under Preview). Without it PDFs keep their icon.
settings-appearance-pdf-thumbs-desc-unix = First page of PDFs through pdfium (libpdfium.so next to the soyuz program, in the config folder, in the folder set under Preview or in a system library folder). Without it PDFs keep their icon.
settings-appearance-icon-pack = Icon pack
settings-appearance-icon-pack-desc = Pictures for file types and folders. Put your own packs in the icons folder, one sub-folder each with a pack.toml (see the user guide); anything a pack leaves out keeps the built-in icon.
settings-appearance-icon-pack-default = Built-in (Phosphor)
settings-appearance-icon-pack-flat = Flat (built-in)
settings-appearance-icon-pack-folder = Icons folder
settings-appearance-thumb-cache = Thumbnail disk cache
settings-appearance-thumb-cache-desc = Keeps thumbnails in the thumbs folder so they survive restarts (0 = off). Oldest are removed first.
settings-clear = Clear
settings-appearance-thumb-size = Default thumbnail size
settings-appearance-thumb-size-desc = Starting zoom of grid views.
settings-appearance-sidebar-width = Sidebar width
settings-appearance-titlebar-desc = Soyuz's own title bar with the logo, the tabs and themed window controls. Off: the system title bar.
settings-appearance-titlebar-desc-restart = Soyuz's title bar with the tabs under the window buttons. Off: the system title bar. Takes effect after a restart.
settings-appearance-titlebar = Custom title bar
settings-appearance-backdrop = Backdrop
settings-appearance-backdrop-desc = Windows 11: show the system Mica or Acrylic material behind the window. Takes effect after a restart.
settings-none = None
settings-appearance-snap = Snap layouts
settings-appearance-snap-desc = Windows 11: hovering the maximize button opens the Snap Layouts flyout. Takes effect after a restart.
settings-appearance-native-dialogs = Dialogs as separate windows
settings-appearance-native-dialogs-desc = Dialogs and the settings window open as their own windows that can move outside Soyuz and onto other monitors; menus that do not fit the window open outside it. Off: everything stays inside the window.
settings-appearance-theme-colors = Theme colors
settings-appearance-theme-colors-desc = Override any design token. Overrides apply to both dark and light themes; a dot marks overridden tokens.
settings-appearance-reset-colors = Reset colors
settings-appearance-token-reset = Use the theme's color
settings-appearance-preview-home = Home
settings-appearance-preview-projects = Projects
settings-appearance-preview-downloads = Downloads

settings-appearance-preview-crumb = Home  ›  Projects

## Density gallery

settings-density-compact = Compact
settings-density-cozy = Cozy
settings-density-comfortable = Comfortable
settings-density-touch = Touch
# Screen-reader name of a density card; $name = density name.
settings-density-a11y = { $name } density

## Editor page

settings-editor-panel = Panel
settings-editor-dock = Dock
settings-editor-dock-desc = Where the editor panel opens. It can also be maximized over the file panes.
settings-editor-text = Text
settings-editor-font-size-desc = Text and hex editor font.
settings-editor-tab-width = Tab width
settings-editor-tab-width-desc = Columns per tab stop.
settings-editor-wrap = Wrap long lines
settings-editor-wrap-desc = Soft-wrap at the panel edge instead of scrolling sideways.
settings-editor-whitespace = Show whitespace
settings-editor-whitespace-desc = Draw spaces as dots and tabs as arrows.
settings-editor-max-text = Largest text file
settings-editor-max-text-desc = Larger files open read-only in the hex editor.
settings-editor-saving = Saving
settings-editor-backup = Keep the previous version in the trash
settings-editor-backup-desc = Before a save overwrites a file, a copy (<name>.bak) goes to the trash so you can restore it.
settings-editor-jpeg = JPEG quality
settings-editor-jpeg-desc = Rotated or cropped JPEGs are re-encoded (there is no lossless path); other formats save losslessly.
settings-editor-max-image = Largest image
settings-editor-max-image-desc = Bigger images aren't decoded by the image editor.

## Fonts page

# Preview text of the interface font (a pangram of the language works best).
settings-fonts-sample-ui = The quick brown fox jumps over the lazy dog — Documents, Pictures, Ünïcödé
settings-fonts-sample-headings = Properties · Downloads · Settings
# $font = font family, e.g. "Segoe UI" or "Inter (embedded)".
settings-fonts-using = Using { $font }

settings-fonts-interface = Interface
settings-fonts-interface-desc = Labels, menus, buttons and file names. Default: the embedded Inter; installed fonts such as Segoe UI work too.
settings-fonts-headings = Headings
settings-fonts-headings-desc = Titles, dialog headers and section names. Empty: the semibold face of the interface font.
settings-fonts-headings-hint = Semibold of the interface font
settings-fonts-terminal-desc = The integrated terminal. Default: the embedded Cascadia Mono (no ligatures, so != and -> stay as typed).
settings-fonts-code = Code editor
settings-fonts-code-desc = The text editor, code previews and other monospace text. Default: Cascadia Mono.
settings-fonts-hex = Hex view
settings-fonts-hex-desc = The hex editor and hex preview.
settings-fonts-numbers = Numbers
settings-fonts-numbers-desc = Sizes, dates and progress figures in the file list and the activity panel. Empty: the interface font. IBM Plex Sans, Noto Sans and the monospace fonts have equal-width digits, so columns line up.
settings-fonts-numbers-hint = Same as the interface font
settings-fonts-monospace = Monospace
settings-fonts-rendering = Rendering
settings-fonts-hinting = Hinting
settings-fonts-hinting-desc = Snap letter stems to the pixel grid: crisper text at small sizes and 100 % scaling.
settings-fonts-subpixel = Sub-pixel positioning
settings-fonts-subpixel-desc = Place letters at fractional pixel positions: more even spacing, slightly softer stems.
settings-fonts-hinting-strength = Hinting strength
settings-fonts-hinting-strength-desc = Crisp also fits letter widths to the pixel grid (sharpest at 100 % scaling); Normal is egui's default; Light keeps the designed shapes.
settings-fonts-hinting-crisp = Crisp
settings-fonts-hinting-normal = Normal
settings-fonts-hinting-light = Light
settings-fonts-gamma-dark = Text weight (dark themes)
settings-fonts-gamma-light = Text weight (light themes)
settings-fonts-gamma-desc = How anti-aliased edges turn into opacity: lower values draw text heavier, 1.00 is linear. Automatic uses the renderer's curve.
settings-fonts-reset-all = Reset all fonts
settings-fonts-reset-all-desc = Back to Inter / Cascadia Mono with crisp hinting and no size offsets.
settings-fonts-embedded = Embedded
settings-fonts-reset-one = Reset this font
settings-fonts-offset = Size offset (points)

## Font sources and notices (theme/fonts.rs)

# $weight = variable-font weight axis value, e.g. 600.
settings-font-source-weight = { $family } (weight { $weight })
settings-font-source-embedded = { $font } (embedded)
# $missing = configured font family; $used = the font used instead.
settings-font-not-installed = { $missing } is not installed; using { $used }
settings-font-no-semibold = { $family } has no semibold face; headings use the regular one

## Theme gallery and accent

settings-accent-orbit-blue = Orbit blue
settings-accent-violet = Violet
settings-accent-pink = Pink
settings-accent-red = Red
settings-accent-amber = Amber
settings-accent-green = Green
settings-accent-teal = Teal
settings-accent-sky = Sky
settings-theme-a11y = { $name } theme
# Theme card tooltip; $author = theme author from the theme file.
settings-theme-tooltip = { $name } — { $author }
settings-theme-high-contrast = High Contrast

settings-theme-hc-on = High contrast is on: it replaces the color theme until you turn it off.
settings-theme-import = Import theme…
settings-theme-export = Export current…
settings-theme-export-tooltip = Save the current colors (theme, accent and overrides) as a theme file
settings-theme-folder = Themes folder
settings-theme-custom-tag = custom
settings-custom = Custom

## Keyboard page

settings-keys-preset-explorer-desc = Windows Explorer conventions: F2 rename, Del, Alt+arrows, Ctrl+1…9 tabs.
settings-keys-preset-commander-desc = Total Commander style: F3 view, F5 copy, F6 move, F7 folder, F8 delete, Tab switches panes.
settings-keys-preset-finder-desc = macOS Finder: Enter renames, Cmd+Down opens, Cmd+Backspace trashes, Cmd+1…4 views.
settings-keys-preset-vim-desc = hjkl, gg / G, yy copy, dd cut, pp paste, / filter, marks with m and '.
settings-keys-shortcuts = Shortcuts
settings-keys-search-hint = Search actions or keys
settings-keys-reset-changes = { $count ->
    [one] Reset { $count } change
   *[other] Reset { $count } changes
}
# Title of the shortcut recorder; $action = action title.
settings-keys-record-title = Shortcut for “{ $action }”

settings-keys-preset = Preset
settings-keys-key-preset = Key preset
settings-keys-cmd-ctrl = Treat Cmd as Ctrl
settings-keys-cmd-ctrl-desc = Bindings written with Ctrl also answer to Cmd on macOS.
settings-keys-marks = Marks
settings-keys-marks-desc = m + key marks the current folder, ' + key jumps back to it (always on in Vim-ish).
settings-keys-conflicts-only = Conflicts only
settings-keys-changed-only = Changed only
settings-keys-empty = No shortcuts match
settings-keys-reset-one = Reset to the preset
settings-add = Add
settings-keys-record = Record a shortcut

## Custom commands page

settings-commands-template-vscode = Open in VS Code
settings-commands-template-git = Git status here
settings-commands-template-copy-names = Copy names to clipboard
settings-commands-template-list = List folder in terminal
settings-commands-ph-focused = focused item (or the folder)
settings-commands-ph-selected = selected paths
settings-commands-ph-names = selected names
settings-commands-ph-dir = current folder
settings-commands-ph-other = other pane's folder
settings-commands-ph-percent = a literal %
settings-commands-new = New command
# $name = command or bookmark name.
settings-commands-record-title = Hotkey for “{ $name }”
settings-bookmarks-record-title = Hotkey for bookmark “{ $name }”
settings-bookmarks-default-name = Bookmark
settings-bookmarks-icon-default = default

settings-commands-placeholders = Placeholders
settings-commands-help = An argument that is exactly %s or %n expands to one argument per item. Commands appear in the palette (Ctrl+Shift+P), in the right-click menu and can have a hotkey.
settings-commands-or-template = or start from
settings-name = Name
settings-commands-delete = Delete command
settings-commands-run = Run
settings-commands-command = Command
settings-commands-command-hint = program or command line
settings-commands-args-hint = %s (quotes group words)
settings-commands-show-for = Show for
settings-commands-background-ctx = Folder background
settings-commands-run-in = Run in
settings-commands-run-background = Background
settings-commands-run-terminal = Terminal panel
settings-commands-hotkey = Hotkey
settings-commands-record = Record…
settings-commands-empty = No custom commands yet
settings-bookmarks-add-current = Bookmark current folder
settings-bookmarks-add-empty = Empty bookmark
settings-bookmarks-help = Bookmarks 1–9 also open with Alt+1…9 and with their own hotkey.
settings-bookmarks-path-hint = Folder path
settings-bookmarks-record = Record a hotkey
settings-bookmarks-empty = No bookmarks yet

## About & updates page

settings-about-version = Version { $version }
# $kind = "debug" | "release"; $os / $arch = platform ids (windows / x86_64).
settings-about-build = { $kind ->
    [debug] Debug
   *[release] Release
    } build for { $os } / { $arch }. Licensed under the MIT License.
settings-updates-privacy = Soyuz only asks GitHub for the public release list; no data about you or your files is sent. Downloads are verified (SHA-256, and the installer's signature) and never installed without your click.
settings-updates-up-to-date = You're up to date. The newest release is { $latest }.
settings-updates-failed = Update check failed: { $error }
settings-updates-download-tooltip = Save { $name } to your Downloads folder
settings-updates-downloading = Downloading { $name }…
# Download progress; $done / $total = sizes like "3.2 MiB".
settings-updates-progress = { $done } of { $total }
settings-updates-saved-to = Saved to { $path }
settings-updates-sha-ok = SHA-256 verified: { $hash }
settings-updates-sig-valid = Authenticode signature: valid
settings-updates-sig-unsigned = Authenticode signature: none (unsigned build)
# $code = Windows error code, e.g. 0x800B0100.
settings-updates-sig-invalid = Authenticode signature: INVALID ({ $code })
settings-updates-sig-unknown = No installer signature to check (portable zip).
settings-updates-available = { $version } is available (you have { $current })
settings-updates-prerelease = pre-release

## Default file manager (Advanced page)

settings-fm-registered = Soyuz now opens folders and drives
settings-fm-restored = File Explorer restored as the folder handler
settings-fm-desc = Folders and drives opened from other apps, the desktop, shortcuts or Win+R open in Soyuz instead of File Explorer. Soyuz adds an “Open in Soyuz” verb for Directory and Drive under HKEY_CURRENT_USER\Software\Classes and makes it the default — for your account only, no admin rights, nothing system-wide. Special folders (This PC, Control Panel, Recycle Bin) stay with Explorer. “Restore Explorer” removes exactly what Soyuz added and puts back anything that was there before.
settings-fm-checking = Checking…
settings-fm-status-explorer = Currently: File Explorer opens folders.
# $command = the registered command line.
settings-fm-status-soyuz = Currently: Soyuz opens folders ({ $command }).
settings-fm-status-modified = Another program changed the folder handler after Soyuz registered. “Restore Explorer” removes Soyuz's entries and leaves that program's choice in place.
settings-fm-status-unsupported = Not available on this platform yet.
settings-fm-temp-copy = Note: this copy runs from { $path }; if you move or delete it, folders will fail to open until you restore Explorer.
# $handler = name of the app that opens folders now (e.g. Files, Dolphin).
settings-fm-desc-linux = Sets Soyuz as the folder handler in ~/.config/mimeapps.list for your account (no root rights, nothing system-wide), so folders opened from other apps and the desktop open in Soyuz. Restore puts back the previous handler (currently: { $handler }).
settings-fm-handler-unknown = none set
settings-fm-status-other = Currently: { $handler } opens folders.
settings-fm-status-modified-linux = Another program changed the folder handler after Soyuz registered. “Restore previous” removes Soyuz's entries and leaves that program's choice in place.
settings-fm-temp-copy-linux = Note: this copy runs from { $path }; if you move or delete it, folders will fail to open until you restore the previous handler.
settings-fm-restored-linux = Previous folder handler restored
settings-fm-restore-linux = Restore previous
settings-fm1 = “Show in folder” when Soyuz isn't running
settings-fm1-desc = Lets other apps' “Show in folder” open Soyuz even when it isn't running: installs a D-Bus service file (org.freedesktop.FileManager1) in ~/.local/share/dbus-1/services for your account. Remove puts back whatever was there before.
settings-fm1-status-on = Installed: “Show in folder” starts Soyuz.
settings-fm1-status-off = Not installed.
settings-fm1-install = Install service
settings-fm1-remove = Remove
settings-fm1-installed = “Show in folder” now starts Soyuz
settings-fm1-removed = D-Bus service file removed

settings-about-button = About Soyuz…
settings-updates = Updates
settings-updates-repo = Release repository
settings-updates-repo-desc = GitHub repository Soyuz updates come from (owner/name or URL). Empty turns the update check off.
settings-updates-weekly = Check once a week
settings-updates-weekly-desc = At startup, at most once every 7 days. Only shows a notification; nothing is downloaded.
settings-updates-prereleases = Include pre-releases
settings-updates-prereleases-desc = Offer release candidates and betas too.
settings-updates-check = Check for updates
settings-updates-no-repo = Set a release repository to enable the check.
settings-updates-status = Update status
settings-updates-checking = Checking GitHub Releases…
settings-updates-download = Download & verify
settings-updates-no-asset = No download for this platform in this release.
settings-updates-copy-link = Copy release link
settings-cancel = Cancel
settings-updates-detached-sig = A detached signature file is published; Soyuz does not verify it yet.
settings-updates-run = Run installer…
settings-updates-reveal = Show in Soyuz
settings-updates-run-note = The installer opens in its own window. It may ask you to close Soyuz to finish the update.
settings-updates-run-now = Run it now
settings-updates-started = Installer started
settings-updates-start-failed = Cannot start the installer
settings-fm-failed = Folder handler change failed
settings-fm = Default file manager
settings-fm-open-with = Open folders with Soyuz
settings-fm-reregister = Re-register this copy
settings-fm-make-default = Make Soyuz the default
settings-fm-restore = Restore Explorer

## Index group (Preview & search page)

settings-index-usn-active = NTFS change journal in use: changes made while Soyuz was closed are applied within seconds of starting.
settings-index-usn-admin = Reading the NTFS change journal needs administrator rights, so after each start Soyuz re-checks the indexed folders in the background instead (searches keep using the index meanwhile). Run Soyuz as administrator to use the journal.
# $reason = technical reason from the index service.
settings-index-usn-unsupported = Change journal not available ({ $reason }); folders are re-checked after each start.
settings-index-usn-off = Change journal not used; folders are re-checked after each start.
# $root: indexed folder; $minutes: re-check interval in minutes.
settings-index-polled-root = Change watching is off for { $root }: the system's file watch limit (fs.inotify.max_user_watches) is used up; re-checked every { $minutes } min.
settings-index-enabled-desc = Keeps names, sizes, dates and tags of the indexed folders in a local database: searches there are instant, and smart folders update live.
settings-index-content-files = { $count ->
    [one] { $count } file with indexed contents
   *[other] { $count } files with indexed contents
}
settings-index-journal-changes = { $count ->
    [one] { $count } change applied from the journal at startup
   *[other] { $count } changes applied from the journal at startup
}
settings-index-last-write = Last change written { $seconds } s ago
settings-index-database = Database: { $path }
settings-index-root-hint = Folder to index, e.g. D:\Photos
settings-index-usn-desc = Apply changes made while Soyuz was closed straight from the volume's change journal (Windows, NTFS). Needs administrator rights; otherwise folders are re-checked after each start.

settings-index = Index
settings-index-enabled = Background index
settings-index-status = Status
settings-index-starting = Starting…
settings-index-off = Indexing is off. Searches walk the disk.
settings-index-rebuild = Rebuild index
settings-index-rebuild-tooltip = Forget everything and index the folders again
settings-index-roots = Indexed folders
settings-index-roots-desc = ~ is your home folder. Folders inside another indexed folder are covered by it.
settings-index-remove-root = Stop indexing this folder
settings-index-add-root = Add folder
settings-index-skip-desc = One per line: a name (node_modules, *.tmp) or a path ending (AppData/Local/Temp, AppData/Local/**/Cache).
settings-index-linux-exclude = Also skip on Linux
settings-index-linux-exclude-desc = Same syntax, used only on Linux: caches, the trash, container and game stores, and the kernel's pseudo file systems. A pattern starting with / or ~/ is anchored there (/proc, ~/snap).
settings-index-content = Index file contents
settings-index-content-desc = Words of small text files, for content: searches. Uses more disk and time.
settings-index-content-limit = Content size limit
settings-index-content-limit-desc = Larger files are indexed by name only.
settings-index-content-types = Content file types
settings-index-content-types-desc = Extensions whose contents are indexed.
settings-index-usn = Use the NTFS change journal
settings-index-throttle = Crawl pause
settings-index-rescan = Re-check unwatched folders
settings-index-rescan-desc = When the system's file watch limit is used up (Linux: fs.inotify.max_user_watches), indexed folders that can't be watched are re-checked this often; only folders whose modification time changed are re-read.
settings-index-throttle-desc = Pause after every 1000 files while indexing; higher is gentler on the disk.
settings-index-smart-refresh = Smart folder refresh
settings-index-smart-refresh-desc = Smart folders update at most this often while files change.
settings-index-use-for-search = Use the index for searches
settings-index-use-for-search-desc = Off: the search bar always walks the disk (smart folders still use the index).

## AI assistant page

# $what = the credential operation, e.g. "Key deleted".
settings-ai-op-failed = { $what } failed: { $error }
settings-ai-profiles = Provider profiles
settings-ai-profile-removed = Removed “{ $name }”. Its stored key stays in the credential store until you delete it.
settings-ai-fast-model = Fast model
settings-ai-fast-model-desc = Naming and classifying. A local model is a good fit.
settings-ai-strong-model = Strong model
settings-ai-strong-model-desc = Planning multi-step file work.
# $store = name of the OS credential store.
settings-ai-key-stored = Stored in { $store }
settings-ai-key-env = From environment ({ $var })
settings-ai-key-env-missing = { $var } is not set
settings-ai-key-saved = Key saved to { $store }
settings-ai-key-deleted = Key deleted
settings-ai-store-windows = Windows Credential Manager
settings-ai-store-other = the OS keychain
settings-ai-store-linux = the Secret Service keyring (GNOME Keyring / KWallet)
# $store: settings-ai-store-linux / -other.
settings-ai-store-unavailable = The credential store ({ $store }) is not available, so keys can't be saved. Start or unlock GNOME Keyring or KWallet, or use an environment variable (env:<VAR>) instead.
settings-ai-loading = loading…
# This month's spend of a profile; $cost = "$1.23".
settings-ai-spend = { $cost } · { $tokens ->
    [one] { $tokens } token
   *[other] { $tokens } tokens
    } · { $requests ->
    [one] { $requests } request
   *[other] { $requests } requests
}
settings-ai-spend-unpriced = model not in the price table
# $percent = whole number.
settings-ai-spend-share = { $percent }% of cap
settings-ai-spend-exceeded = cap reached, requests are blocked
settings-ai-test-models-error = (model list unavailable: { $error })
settings-ai-tokens = { $count ->
    [one] { $count } token
   *[other] { $count } tokens
}
settings-ai-model = Model
settings-ai-price-input = Input
settings-ai-price-output = Output
settings-ai-price-cache-read = Cache read
settings-ai-price-cache-write = Cache write

settings-ai-empty = No AI providers yet
settings-ai-empty-hint = Add a local model (Ollama, LM Studio …) to keep everything on this machine, or a cloud provider.
settings-ai-assistant = Assistant
settings-ai-enabled = Enable AI features
settings-ai-enabled-desc = Off hides the assistant and blocks every model request.
settings-ai-privacy = What the assistant may see
settings-ai-privacy-desc = Default for every folder. Metadata only: names, sizes and dates; file contents never leave this machine.
settings-ai-metadata-only = Metadata only
settings-ai-contents = Contents
settings-ai-default-profile = Default profile
settings-ai-default-profile-desc = Used when a task names no profile.
settings-ai-first-profile = First profile
settings-ai-scope = Scope
settings-ai-roots = Allowed folders
settings-ai-roots-desc = One folder per line. AI tools can only see and change items inside them (empty = your home folder).
settings-ai-blocked = Extra blocked patterns
settings-ai-blocked-desc = Globs, one per line (e.g. **/private/**). .ssh, .env, keychains, browser profiles and wallets are always blocked.
settings-ai-sensitive = Allow sensitive locations
settings-ai-sensitive-desc = Lets AI tools into .ssh, .env files, keychains, browser profiles and wallets. Not recommended.
settings-ai-read-limit = Text read limit
settings-ai-read-limit-desc = Largest part of a file the assistant can read at once.
settings-ai-network = Network and budget
settings-ai-timeout = Response timeout
settings-ai-timeout-desc = Seconds to wait for a provider to start answering.
settings-ai-idle-timeout = Stream idle timeout
settings-ai-idle-timeout-desc = Give up when a streaming answer stalls this long.
settings-ai-retries = Retries
settings-ai-retries-desc = On rate limits (429), overload and server errors; honours retry-after.
settings-ai-budget-warn = Budget warning
settings-ai-budget-warn-desc = Warn when a profile has used this share of its monthly cap.
settings-ai-tasks = Assistant tasks
settings-ai-approvals = Approvals
settings-ai-approvals-desc = Always ask, or let small conflict-free write plans run unasked. Never after the assistant read file contents; destructive plans always ask.
settings-ai-always-ask = Always ask
settings-ai-auto-writes = Auto small writes
settings-ai-max-risk = Highest tool class
settings-ai-max-risk-desc = What the assistant is offered at all.
settings-ai-read-only = Read only
settings-ai-write = Write
settings-ai-destructive = Trash / delete
settings-ai-steps = Steps per task
settings-ai-steps-desc = Model turns before a task stops.
settings-ai-token-budget = Token budget per task
settings-ai-token-budget-desc = Input + output tokens of all turns (0 = unlimited).
settings-ai-time-limit = Time limit per task
settings-ai-time-limit-desc = Wall clock; Esc stops a task any time.
settings-ai-auto-reads = Automatic reads
settings-ai-auto-reads-desc = Read tools the assistant may run on its own per task.
settings-ai-compact = Compact after
settings-ai-compact-desc = Older tool results are summarised once the conversation is this large.
settings-ai-keep-recent = Keep recent results
settings-ai-keep-recent-desc = Newest tool results kept verbatim when compacting.
settings-ai-listing = Folder entries in context
settings-ai-listing-desc = Entries of the current folder summarised for the model.
settings-ai-confirm-cost = Ask before costly tasks
settings-ai-confirm-cost-desc = Confirm when the estimated cost is above this (0 = never ask).
settings-ai-redact = Redact secrets for cloud models
settings-ai-redact-desc = Mask e-mail addresses, API keys and tokens in file data sent to cloud providers (local models see it unchanged).
settings-ai-name-excerpt = Suggest names: text per file
settings-ai-name-excerpt-desc = Characters of each text or document file sent when suggesting names (only in folders with “contents” access; 0 = names and metadata only).
settings-ai-vision-px = Suggest names: image size
settings-ai-vision-px-desc = Longest side of the downscaled picture sent to vision models (profiles with “Accepts images”, folders with “contents” access; 0 = never send images).
settings-ai-name-files = Suggest names: files
settings-ai-name-files-desc = Most files looked at per request.
settings-ai-folders = Folder permissions
settings-ai-per-folder = Per-folder access
settings-ai-per-folder-desc = Overrides “What the assistant may see” for a folder and everything below it (the deepest rule wins). Also set from the assistant panel or a folder's Properties.
settings-ai-folder-hint = folder path
settings-ai-metadata = Metadata
settings-ai-remove-rule = Remove this rule
settings-ai-no-rules = No folder rules: every folder uses the default above.
settings-ai-add-rule = Add folder rule
settings-ai-task-model-hint = profile's model
settings-ai-name-unique = needs a unique name
settings-ai-delete-profile = Delete profile
settings-ai-default-tag = default
settings-ai-make-default = Make default
settings-ai-base-url = Base URL
settings-ai-model-hint = model id
settings-ai-api-key = API key
settings-ai-tools = Tools
settings-ai-native = Native
settings-ai-json = JSON fallback
settings-ai-max-tokens = Max output tokens
settings-ai-vision = Accepts images
settings-ai-vision-desc = vision model: “Suggest names” may send downscaled pictures where contents are allowed
settings-ai-budget = Monthly budget
settings-ai-budget-hint = 0 = no cap
settings-ai-this-month = This month
settings-ai-key-not-needed = Not needed (local server)
settings-ai-no-key = No key
settings-ai-key-missing = Not stored yet
settings-ai-key-hint = paste a key to store it
settings-ai-save-key = Save key
settings-ai-forget-key = Delete the stored key
settings-ai-key-ref = Key reference
settings-ai-key-ref-hint = credential:<name> or env:<VAR>
settings-ai-key-ref-desc = config.toml stores only this reference, never the key.
settings-ai-headers = Extra headers
settings-ai-header-value = value
settings-ai-add-header = Add header
settings-ai-headers-desc = Header values are stored in config.toml; put secrets in the API key field instead.
settings-ai-testing = Testing…
settings-ai-test = Test connection
settings-ai-test-tooltip = Lists models and sends one tiny request that asks the model to call a test tool (uses a few tokens)
settings-ai-model-unlisted = model not in the server's list
settings-ai-prices = Prices
settings-ai-price-table = Price table
settings-ai-price-table-desc = USD per million tokens, used for the cost meter and budget caps. The model is a glob; the first match wins.
settings-ai-add-price = Add price
settings-restore-defaults = Restore defaults

## AI → Connections (MCP)

settings-mcp-risk-write = Read + write
settings-mcp-risk-delete = Incl. delete
# $app = product name, e.g. "Claude Desktop".
settings-mcp-installed = Added Soyuz to { $app }. Restart { $app } to pick it up.
settings-mcp-connections = Connections (MCP)
settings-mcp-intro = Let AI apps such as Claude Desktop, Claude Code or VS Code use Soyuz through the Model Context Protocol. They work only inside the folders you allow; changes show up here as a plan you approve, then run as undoable jobs. Every call is written to ai-audit.jsonl.
# $name = named-pipe name or an error.
settings-mcp-pipe = pipe { $name }
settings-mcp-pipe-off = pipe off
settings-mcp-waiting = { $count ->
    [one] { $count } waiting for approval
   *[other] { $count } waiting for approval
}
settings-mcp-token-stored = Token stored in the credential store
settings-mcp-token-revoked = Token revoked
settings-mcp-client-removed = Removed “{ $name }” and its token
settings-mcp-starts-with = starts with: { $command }
settings-mcp-clients = MCP clients
settings-mcp-add-to = Add Soyuz to …
settings-mcp-file-changes = Changes to the file
settings-mcp-new-file = New file
settings-mcp-write-to = Write to { $app }
settings-mcp-servers = External MCP servers
settings-mcp-servers-intro = Tools from other MCP servers the in-app assistant may use. Their descriptions and results are treated as untrusted; tools above “read” ask for approval first.

settings-mcp-server = Soyuz MCP server
settings-mcp-attach = Let `soyuz --mcp` attach to this window
settings-mcp-attach-desc = Tool calls then show in this window and write tools get an approval card. Off = `soyuz --mcp` runs headless.
settings-mcp-http = HTTP transport
settings-mcp-http-desc = Streamable HTTP on 127.0.0.1 for clients that cannot start programs. Each client needs a token.
settings-mcp-port = HTTP port
settings-mcp-port-desc = Endpoint: http://127.0.0.1:<port>/mcp
settings-mcp-origins = Allowed browser origins
settings-mcp-origins-desc = One per line. Loopback origins are always allowed; anything else is refused (DNS-rebinding defence).
settings-mcp-register = Register unknown clients
settings-mcp-register-desc = A new client is added as “pending” and can only read until you approve it below. Off = refuse it.
settings-mcp-approval-timeout = Approval timeout
settings-mcp-approval-timeout-desc = Refuse a change when nobody answers its approval card.
settings-mcp-approval-ui = Approval cards
settings-mcp-approval-ui-desc = Where a client's plan card appears: in the assistant panel (it opens when a request arrives) or as a modal card over the window.
settings-mcp-approval-panel = Assistant panel
settings-mcp-approval-modal = Modal card
settings-mcp-pending = pending · read-only
settings-mcp-approve = Approve
settings-mcp-approved = approved
settings-mcp-remove-client = Remove client
settings-mcp-may = May
settings-mcp-ask = Ask before changes
settings-mcp-no-ask = changes run without asking
settings-mcp-folders-hint = empty = the assistant's allowed folders
settings-mcp-token = HTTP token
settings-mcp-rotate = Rotate token
settings-mcp-generate = Generate token
settings-mcp-revoke = Revoke
settings-mcp-token-once = Copy this token now; Soyuz will not show it again.
settings-mcp-copy-token = Copy token
settings-mcp-copy-config = Copy config
settings-mcp-client-hint = client name, e.g. my-agent
settings-mcp-add-client = Add client
settings-mcp-no-clients = No clients yet. Use “Add Soyuz to …” below, or connect one: it appears here as pending.
settings-mcp-snippet = Snippet
settings-mcp-already = Already set up: the file contains this entry.
settings-mcp-copy-snippet = Copy snippet
settings-close-button = Close
settings-mcp-backup = The old file is kept as .soyuz-backup next to it.
settings-mcp-remove-server = Remove server
settings-mcp-command-hint = program (stdio), e.g. uvx
settings-mcp-args-hint = one per line
settings-mcp-url = or URL
settings-mcp-server-token = Token
settings-mcp-tools-may = Its tools may
settings-mcp-ask-run = Ask before running
settings-mcp-trust-hints = Trust read-only hints
settings-mcp-trust-hints-desc = only for servers you trust
settings-mcp-timeout = Timeout
settings-mcp-test = Test
settings-mcp-server-hint = server name
settings-mcp-add-server = Add server

## Slider units (after a number)

settings-unit-chars = chars
settings-unit-days = days
settings-unit-lines = lines
settings-unit-rows = rows
settings-unit-columns = columns

## Accessible names of icon-only buttons
settings-a11y-remove = Remove
settings-a11y-move-up = Move up
settings-a11y-move-down = Move down
settings-a11y-record-key = Record a shortcut
settings-a11y-button = Button
## Dropdowns (fonts, shells, themes, models, renderer)
settings-fonts-system-group = System fonts
settings-fonts-embedded-only-reason = “Embedded” is on: only fonts built into Soyuz are used
settings-fonts-default = Default
settings-fonts-loading = Loading installed fonts…
settings-fonts-custom-hint = Family name or font file path
settings-terminal-detected = Detected on this computer
settings-terminal-detecting = Looking for shells…
settings-preview-syntax-auto = Follows the app theme
settings-accent-presets = Presets
settings-accent-custom-a11y = { $name }: custom color
settings-theme-builtin-group = Built in
settings-theme-custom-group = Your themes
settings-ai-models-known = Known models
settings-ai-models-default = default
settings-ai-models-listed = From the provider
settings-ai-models-loading = Loading the provider’s model list…
settings-ai-models-failed = Could not list models: { $error }
settings-appearance-gpu-backend = Graphics backend
settings-appearance-gpu-backend-desc = The graphics API Soyuz draws with. Try another one if the window flickers or stays blank with your graphics driver. Applies at the next start; if Soyuz then fails to start, run it with --safe-mode.
settings-appearance-gpu-windows-only = Only available on Windows
settings-appearance-gpu-compat = compatibility
settings-bookmarks-icon = Bookmark icon

## Linux (glue)
settings-updates-privacy-linux = Soyuz only asks GitHub for the public release list; no data about you or your files is sent. Downloads are checked against the release's SHA-256 checksum and never installed without your click.
settings-updates-sig-unknown-linux = Linux releases carry no installer signature; the SHA-256 checksum above was verified.
settings-updates-install-restart = Install & restart
settings-updates-install-restart-tooltip = Replace this copy of Soyuz with the downloaded version and start it again. Your settings are kept.
settings-updates-installing = Installing the update…
settings-linux-fm1-serving = Serving
settings-linux-fm1-pending = Starting…
settings-linux-fm1-off = Off
settings-fonts-interface-desc-linux = Labels, menus, buttons and file names. Default: the embedded Inter; installed fonts such as Noto Sans or Cantarell work too.

## Linux (devices)
settings-views-unmounted-devices = Unmounted devices
settings-views-unmounted-devices-desc = Disks and partitions that aren't mounted, listed under Devices: click one to mount it, or to unlock a BitLocker / LUKS volume. Mounting goes through UDisks2, which may ask for your password.
settings-views-unmounted-all = All
settings-views-unmounted-removable = Removable only
settings-views-unmounted-off = Off
settings-views-hidden-devices = Hidden devices
settings-views-hidden-devices-desc = Devices hidden with "Hide from sidebar", one filesystem UUID or device path (/dev/sdb1) per line.
settings-views-hidden-devices-clear = Show all again

## thumbs
settings-appearance-write-os-thumbs = Share thumbnails with other apps
settings-appearance-write-os-thumbs-desc = Save the thumbnails Soyuz makes into the freedesktop thumbnail cache (~/.cache/thumbnails), so file managers and file choosers can show them without decoding the files again.
