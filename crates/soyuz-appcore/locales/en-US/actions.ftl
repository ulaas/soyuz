### Soyuz UI strings (en-US source): actions
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/actions.ftl.

## Action titles and descriptions: action-<name> / action-<name>-desc, where <name> is the
## action's config name with _ replaced by - (looked up dynamically, see palette/catalog.rs).

## Navigation

action-back = Back
action-back-desc = Go back in this pane's history
action-forward = Forward
action-forward-desc = Go forward in this pane's history
action-up = Up
action-up-desc = Go to the parent folder
action-home = Home
action-home-desc = Go to your home folder
action-root = Root
action-root-desc = Go to the root of the current drive
action-go-to = Go to path…
action-go-to-desc = Type a path into the address bar
action-focus-address-bar = Focus address bar
action-focus-address-bar-desc = Edit the path in the address bar
action-edit-path = Edit path
action-edit-path-desc = Edit the current path as text
action-refresh = Refresh
action-refresh-desc = Reload the folder listing
action-toggle-sidebar = Toggle sidebar
action-toggle-sidebar-desc = Show or hide the sidebar
action-focus-sidebar = Focus sidebar
action-focus-sidebar-desc = Move keyboard focus to the sidebar
action-focus-activity-panel = Focus jobs panel
action-focus-activity-panel-desc = Move keyboard focus to the running and finished operations (terminal UI)
action-focus-list = Focus list
action-focus-list-desc = Move keyboard focus to the file list
action-focus-other-pane = Focus other pane
action-focus-other-pane-desc = Switch focus to the other pane
action-focus-terminal = Focus terminal
action-focus-terminal-desc = Move keyboard focus to the terminal
action-next-tab = Next tab
action-next-tab-desc = Switch to the next tab
action-prev-tab = Prev tab
action-prev-tab-desc = Switch to the previous tab
action-jump-to-folder-dialog = Go to folder…
action-jump-to-folder-dialog-desc = Fuzzy-jump to a recent or frequent folder
action-open-recent = Recent folders
action-open-recent-desc = Pick from recently visited folders
action-open-timeline = Timeline: changed files
action-open-timeline-desc = Files changed today and this week
action-tab-history = Where was I? (tab history graph)
action-tab-history-desc = Every place this tab visited, with branches
action-connect-to-server = Connect to server…
action-connect-to-server-desc = Browse an SFTP, WebDAV, FTP / FTPS or S3 server

## Tabs

action-new-tab = New tab
action-new-tab-desc = Open a new tab
action-close-tab = Close tab
action-close-tab-desc = Close the current tab
action-reopen-closed-tab = Reopen closed tab
action-reopen-closed-tab-desc = Reopen the most recently closed tab
action-duplicate-tab = Duplicate tab
action-duplicate-tab-desc = Open the current folder in a new tab
action-pin-tab = Pin tab
action-pin-tab-desc = Pin or unpin the current tab
action-move-tab-left = Move tab left
action-move-tab-left-desc = Move the tab one position left
action-move-tab-right = Move tab right
action-move-tab-right-desc = Move the tab one position right
action-tab-to-new-window = Move tab to new window
action-tab-to-new-window-desc = Tear the tab off into its own window
action-toggle-dual-pane = Toggle dual pane
action-toggle-dual-pane-desc = Show or hide the second pane
action-swap-panes = Swap panes
action-swap-panes-desc = Swap the left and right panes
action-mirror-pane-to-other = Mirror folder in other pane
action-mirror-pane-to-other-desc = Show this folder in the other pane too
action-open-in-other-pane = Open in other pane
action-open-in-other-pane-desc = Open the selected folder in the other pane
action-sync-navigation = Toggle synchronized browsing
action-sync-navigation-desc = Navigate both panes together
action-new-window = New window
action-new-window-desc = Open a new Soyuz window
action-close-window = Close window
action-close-window-desc = Close this window
action-toggle-quad-pane = Toggle quad layout (2×2)
action-toggle-quad-pane-desc = Show four panes in a 2×2 grid
action-new-tab-group = Add tab to new group
action-new-tab-group-desc = Put the current tab into a new tab group
action-ungroup-tab = Remove tab from group
action-ungroup-tab-desc = Take the current tab out of its group
action-merge-windows = Merge all windows into this one
action-merge-windows-desc = Move the tabs of every other window here
action-workspaces = Workspaces…
action-workspaces-desc = Open, save and manage named workspaces
action-save-workspace = Save workspace as…
action-save-workspace-desc = Save this window's tabs and layout under a name
action-update-workspace = Update current workspace
action-update-workspace-desc = Save this window over the workspace last opened
action-manage-workspaces = Manage workspaces…
action-manage-workspaces-desc = Open, rename, reorder, delete saved workspaces and set their hotkeys

## View

action-view-details = View: Details
action-view-details-desc = Show the folder as a detailed table
action-view-grid = View: Grid
action-view-grid-desc = Show icons or thumbnails in a grid
action-view-list = View: List
action-view-list-desc = Show a compact multi-column list
action-view-miller = View: Columns (Miller)
action-view-miller-desc = Browse with cascading columns
action-view-gallery = View: Gallery
action-view-gallery-desc = Large preview with a filmstrip
action-zoom-in = Larger icons
action-zoom-in-desc = Make icons and thumbnails larger
action-zoom-out = Smaller icons
action-zoom-out-desc = Make icons and thumbnails smaller
action-zoom-reset = Reset icon size
action-zoom-reset-desc = Reset the icon size
action-ui-zoom-in = Zoom in (interface)
action-ui-zoom-in-desc = Scale the whole interface up
action-ui-zoom-out = Zoom out (interface)
action-ui-zoom-out-desc = Scale the whole interface down
action-ui-zoom-reset = Reset interface zoom
action-ui-zoom-reset-desc = Back to 100 % interface scale
action-toggle-high-contrast = Toggle high contrast
action-toggle-high-contrast-desc = Stronger text, borders and focus rings
action-toggle-hidden = Toggle hidden files
action-toggle-hidden-desc = Show or hide hidden files
action-toggle-flat-view = Toggle flat view
action-toggle-flat-view-desc = List the contents of all subfolders
action-toggle-sort-dir = Reverse sort order
action-toggle-sort-dir-desc = Flip ascending / descending order
action-choose-columns = Choose columns…
action-choose-columns-desc = Pick the columns shown in details view
action-reset-folder-view = Reset folder view
action-reset-folder-view-desc = Forget the view settings of this folder
action-apply-view-to-all = Apply view to all folders
action-apply-view-to-all-desc = Use this layout for every folder
action-toggle-preview-pane = Toggle preview pane
action-toggle-preview-pane-desc = Show or hide the preview pane
action-toggle-info-pane = Toggle info pane
action-toggle-info-pane-desc = Show or hide the info pane
action-toggle-terminal = Toggle terminal
action-toggle-terminal-desc = Show or hide the integrated terminal
action-toggle-activity-panel = Toggle activity panel
action-toggle-activity-panel-desc = Show or hide running jobs
action-preview-scroll-up = Scroll preview up
action-preview-scroll-up-desc = Scroll the preview pane up one line (terminal UI)
action-preview-scroll-down = Scroll preview down
action-preview-scroll-down-desc = Scroll the preview pane down one line (terminal UI)
action-preview-page-up = Preview page up
action-preview-page-up-desc = Scroll the preview pane up one page (terminal UI)
action-preview-page-down = Preview page down
action-preview-page-down-desc = Scroll the preview pane down one page (terminal UI)
action-context-menu = Open context menu
action-context-menu-desc = Show the menu of the focused item, like a right click (terminal UI)
action-new-smart-folder = New smart folder
action-new-smart-folder-desc = Save an index query as a folder in the sidebar that stays up to date
action-save-search-as-smart-folder = Save search as smart folder
action-save-search-as-smart-folder-desc = Keep the search shown in this pane as a smart folder in the sidebar
action-index-settings = Index settings
action-index-settings-desc = Index status, indexed folders and smart folder options
action-rebuild-index = Rebuild index
action-rebuild-index-desc = Forget the file index and crawl the indexed folders again
action-toggle-index = Turn file indexing on or off
action-toggle-index-desc = A local index of the indexed folders makes searches instant and keeps smart folders live
action-toggle-ai-panel = Toggle AI assistant
action-toggle-ai-panel-desc = Show or hide the AI assistant panel
action-toggle-status-bar = Toggle status bar
action-toggle-status-bar-desc = Show or hide the status bar
action-toggle-tree-sidebar = Toggle disk tree
action-toggle-tree-sidebar-desc = Switch the sidebar between places and the disk tree
action-toggle-checkboxes = Toggle item checkboxes
action-toggle-checkboxes-desc = Select items with checkboxes
action-toggle-shelf = Toggle shelf
action-toggle-shelf-desc = Show or hide the drop shelf for collecting files
action-toggle-editor-panel = Toggle editor panel
action-toggle-editor-panel-desc = Show or hide the built-in editor panel

## Selection

action-select-all = Select all
action-select-all-desc = Select every item
action-select-none = Select none
action-select-none-desc = Clear the selection
action-invert-selection = Invert selection
action-invert-selection-desc = Select everything that isn't selected
action-select-by-pattern = Select by pattern…
action-select-by-pattern-desc = Select items matching a glob or text
action-select-same-extension = Select same extension
action-select-same-extension-desc = Select files with the same extension
action-select-same-name = Select same name
action-select-same-name-desc = Select items with the same base name
action-mark-toggle = Toggle mark
action-mark-toggle-desc = Toggle selection of the item under the cursor
action-mark-and-move-down = Mark and move down
action-mark-and-move-down-desc = Toggle selection and move to the next item
action-cursor-up = Cursor up
action-cursor-up-desc = Move the cursor up
action-cursor-down = Cursor down
action-cursor-down-desc = Move the cursor down
action-cursor-page-up = Cursor page up
action-cursor-page-up-desc = Move the cursor one page up
action-cursor-page-down = Cursor page down
action-cursor-page-down-desc = Move the cursor one page down
action-cursor-home = Cursor home
action-cursor-home-desc = Jump to the first item
action-cursor-end = Cursor end
action-cursor-end-desc = Jump to the last item
action-extend-up = Extend up
action-extend-up-desc = Extend the selection upwards
action-extend-down = Extend down
action-extend-down-desc = Extend the selection downwards
action-filter = Filter this folder
action-filter-desc = Filter the list as you type
action-clear-filter = Clear filter
action-clear-filter-desc = Remove the current filter
action-type-ahead-toggle = Toggle type-ahead mode
action-type-ahead-toggle-desc = Switch typing between jump and filter

## FileOps

action-open = Open
action-open-desc = Open the selection
action-open-with = Open with…
action-open-with-desc = Open with a chosen application
action-open-in-editor = Edit in Soyuz
action-open-in-editor-desc = Open the file in the built-in text, hex or image editor
action-open-in-new-tab = Open in new tab
action-open-in-new-tab-desc = Open the selected folders in new tabs
action-open-in-terminal = Open in terminal
action-open-in-terminal-desc = Open the selected folder in the terminal
action-open-terminal-here = Open terminal here
action-open-terminal-here-desc = Open a terminal in the current folder
action-run-elevated = Run elevated
action-run-elevated-desc = Run the selection as administrator
action-reveal-in-system = Reveal in system file manager
action-reveal-in-system-desc = Show the item in the system file manager
action-copy = Copy
action-copy-desc = Copy the selection to the clipboard
action-cut = Cut
action-cut-desc = Cut the selection to the clipboard
action-paste = Paste
action-paste-desc = Paste the clipboard into this folder
action-paste-as-link = Paste as link
action-paste-as-link-desc = Paste the clipboard as links
action-paste-shortcut = Paste shortcut
action-paste-shortcut-desc = Paste the clipboard as shortcuts
action-paste-and-verify = Paste and verify
action-paste-and-verify-desc = Paste, then hash-compare every copy
action-copy-to-other-pane = Copy to other pane
action-copy-to-other-pane-desc = Copy the selection to the other pane
action-move-to-other-pane = Move to other pane
action-move-to-other-pane-desc = Move the selection to the other pane
action-copy-to = Copy to…
action-copy-to-desc = Copy the selection to a chosen folder
action-move-to = Move to…
action-move-to-desc = Move the selection to a chosen folder
action-duplicate = Duplicate
action-duplicate-desc = Make a copy next to the original
action-rename = Rename
action-rename-desc = Rename the item under the cursor
action-batch-rename = Batch rename…
action-batch-rename-desc = Rename many items with patterns
action-new-folder = New folder
action-new-folder-desc = Create a new folder here
action-new-file = New file
action-new-file-desc = Create a new empty file here
action-new-symlink = New symbolic link…
action-new-symlink-desc = Create a symbolic link
action-new-hardlink = New hard link…
action-new-hardlink-desc = Create a hard link
action-new-junction = New junction…
action-new-junction-desc = Create an NTFS junction
action-delete = Move to trash
action-delete-desc = Move the selection to the trash
action-delete-permanent = Delete permanently
action-delete-permanent-desc = Delete without the trash
action-secure-wipe = Secure wipe
action-secure-wipe-desc = Overwrite and delete the selection
action-properties = Properties
action-properties-desc = Show size, dates and attributes
action-copy-path = Copy path
action-copy-path-desc = Copy the full path to the clipboard
action-copy-path-posix = Copy path (forward slashes)
action-copy-path-posix-desc = Copy the path with forward slashes
action-copy-path-quoted = Copy path (quoted)
action-copy-path-quoted-desc = Copy the path in double quotes
action-copy-name = Copy name
action-copy-name-desc = Copy the file name to the clipboard
action-copy-as-file-list = Copy as file list
action-copy-as-file-list-desc = Copy all selected paths, one per line
action-create-archive = Create archive…
action-create-archive-desc = Compress the selection into an archive
action-extract-here = Extract here
action-extract-here-desc = Extract the archive into this folder
action-extract-to-folder = Extract to folder
action-extract-to-folder-desc = Extract into a folder named after the archive
action-extract-to = Extract to…
action-extract-to-desc = Extract the archive to a chosen folder
action-test-archive = Test archive
action-test-archive-desc = Verify the archive's integrity
action-compare-checksum = Compare checksum…
action-compare-checksum-desc = Compare a file against a known checksum
action-set-attributes = Set attributes…
action-set-attributes-desc = Change read-only / hidden / system attributes
action-touch-timestamps = Touch timestamps
action-touch-timestamps-desc = Set modification times to now
action-split-file = Split file
action-split-file-desc = Split a large file into parts
action-join-files = Join files
action-join-files-desc = Join split parts back together
action-clear-tags = Clear tags
action-clear-tags-desc = Remove all tags from the selection
action-undo = Undo
action-undo-desc = Undo the last file operation
action-redo = Redo
action-redo-desc = Redo the last undone operation
action-previous-versions = Restore previous version…
action-previous-versions-desc = Browse and restore earlier versions from shadow copies
action-media-convert = Convert media…
action-media-convert-desc = Convert a video or audio file to another format (FFmpeg)
action-media-extract-audio = Extract audio…
action-media-extract-audio-desc = Save the sound track of a video as an audio file (FFmpeg)
action-media-trim = Trim media…
action-media-trim-desc = Keep only a part of a video or audio file (FFmpeg)
action-media-save-frame = Save video frame…
action-media-save-frame-desc = Save one frame of a video as a picture (FFmpeg)

## Tools

action-command-palette = Command palette
action-command-palette-desc = Search and run any command
action-search = Search
action-search-desc = Search this folder and its subfolders
action-search-content = Search file contents
action-search-content-desc = Search inside files
action-folder-size = Calculate folder sizes
action-folder-size-desc = Compute the size of folders
action-disk-usage-treemap = Disk usage treemap
action-disk-usage-treemap-desc = Visualize what takes up space
action-find-duplicates = Find duplicate files
action-find-duplicates-desc = Find files with identical content
action-compare-folders = Compare folders
action-compare-folders-desc = Compare the contents of two folders
action-sync-folders = Synchronize folders
action-sync-folders-desc = Make two folders identical
action-sync-profiles = Sync profiles & schedules
action-sync-profiles-desc = Saved sync profiles, schedules and run reports
action-settings = Settings
action-settings-desc = Open the settings
action-edit-config-file = Edit config file
action-edit-config-file-desc = Open config.toml in your editor
action-reload-config = Reload config
action-reload-config-desc = Re-read config.toml from disk
action-open-trash = Open trash
action-open-trash-desc = Browse the trash
action-empty-trash = Empty trash
action-empty-trash-desc = Permanently delete everything in the trash
action-restore-from-trash = Restore from trash
action-restore-from-trash-desc = Put trashed items back
action-computer = Computer
action-computer-desc = Show drives and devices
action-show-keybindings = Keyboard shortcuts
action-show-keybindings-desc = Browse and edit keyboard shortcuts
action-about = About Soyuz
action-about-desc = Version and credits
action-download-ffmpeg-runtime = Download FFmpeg runtime
action-download-ffmpeg-runtime-desc = Download the FFmpeg libraries for video and audio previews and load them without a restart
action-welcome = Welcome tips
action-welcome-desc = Show the getting-started tips again
action-quit = Quit
action-quit-desc = Quit Soyuz
action-quick-look = Quick Look
action-quick-look-desc = Preview the item in a popup
action-quick-look-next = Quick Look: next item
action-quick-look-next-desc = Preview the next item
action-quick-look-prev = Quick Look: previous item
action-quick-look-prev-desc = Preview the previous item
action-toggle-hex-preview = Toggle hex preview
action-toggle-hex-preview-desc = Show the preview as hex
action-ai-organize = AI: Organize this folder…
action-ai-organize-desc = Ask the assistant to plan a tidy-up of this folder
action-ai-suggest-names = AI: Suggest names
action-ai-suggest-names-desc = Ask the assistant for better names (preview before renaming)
action-ai-summarize = AI: Summarize
action-ai-summarize-desc = Summarize the selected files (contents only where allowed)
action-ai-explain-folder = AI: Explain this folder
action-ai-explain-folder-desc = Ask the assistant what this folder contains
action-ai-find-related = AI: Find related files
action-ai-find-related-desc = Find files related to the selection
action-ai-smart-search = AI: Smart search
action-ai-smart-search-desc = Type a search in plain words; edit the query before it runs
action-ai-audit-log = AI: Audit log
action-ai-audit-log-desc = Every prompt, tool call, plan, approval and job of the assistant
action-ai-recipes = AI: Recipes
action-ai-recipes-desc = Saved assistant plans you can re-run without a model
action-ai-stop = AI: Stop the assistant
action-ai-stop-desc = Cancel the running assistant task
action-open-scripts = Scripts…
action-open-scripts-desc = Your Rhai scripts: actions, columns, previewers; errors
action-reload-scripts = Reload scripts
action-reload-scripts-desc = Load the scripts folder again
action-open-automations = Automations…
action-open-automations-desc = Rules that act on new files in watched folders

## Jobs

action-jobs-pause-all = Pause all jobs
action-jobs-pause-all-desc = Pause every running job
action-jobs-resume-all = Resume all jobs
action-jobs-resume-all-desc = Resume every paused job
action-jobs-cancel-all = Cancel all jobs
action-jobs-cancel-all-desc = Cancel every job
action-jobs-clear-finished = Clear finished jobs
action-jobs-clear-finished-desc = Remove finished jobs from the list
action-jobs-show-history = Show job history
action-jobs-show-history-desc = Browse completed operations and undo

## Parameterised
action-open-bookmark-desc = Navigate to a configured bookmark
action-select-tab-desc = Switch to a tab by position
action-open-workspace-desc = Restore a saved workspace
action-sort-by-desc = Sort the folder by this column
action-set-group-by-desc = Group items under headers
action-add-tag-desc = Label the selection with a color tag
action-checksum-desc = Compute a checksum of the selection
action-custom-command-desc = Custom command
action-run-script-desc = Script action

## Titles of actions with a payload (looked up by catalog.rs).
# $n: bookmark / tab / workspace slot number (1–9).
action-open-bookmark = Open bookmark { $n }
action-select-tab = Go to tab { $n }
action-select-tab-last = Go to last tab
action-open-workspace = Open workspace { $n }
# $tag: a color tag name (actions-tag-*).
action-add-tag = Add tag: { $tag }
# $group: a grouping name (actions-group-*).
action-set-group-by = Group by: { $group }
# $column: a column name.
action-sort-by = Sort by: { $column }
# $algo: a hash algorithm name (SHA-256, BLAKE3, …).
action-checksum = Checksum: { $algo }

## Color tags
actions-tag-red = Red
actions-tag-orange = Orange
actions-tag-yellow = Yellow
actions-tag-green = Green
actions-tag-cyan = Cyan
actions-tag-blue = Blue
actions-tag-purple = Purple
actions-tag-gray = Gray
actions-tag-other = Tag

## Group-by choices
actions-group-none = None
actions-group-kind = Type
actions-group-date = Date modified
actions-group-size = Size
actions-group-extension = Extension
actions-group-tag = Tag
actions-group-first-letter = First letter

## Command palette: modes
actions-mode-commands = Commands
actions-mode-folders = Go to folder
actions-mode-path = Path
actions-mode-bookmarks = Bookmarks
actions-mode-search = Search
actions-mode-shell = Shell
actions-mode-tabs = Tabs
# Footer legend next to the prefix character (lowercase).
actions-legend-path = path
actions-legend-bookmarks = bookmarks
actions-legend-search = search
actions-legend-shell = shell
actions-legend-tabs = tabs
actions-placeholder-commands = Type a command, or : path  @ bookmark  / search  > shell  # tab
actions-placeholder-folders = Jump to a recent folder or bookmark…
actions-placeholder-path = Type a path — Tab completes folders
actions-placeholder-bookmarks = Bookmarks, favorites and recent folders
actions-placeholder-search = Search this folder and its subfolders
actions-placeholder-shell = Run a shell command in this folder
actions-placeholder-tabs = Switch to an open tab

## Command palette: sections and row tags
actions-section-recent = Recent
actions-section-all-commands = All commands
actions-section-commands = Commands
actions-section-folders = Folders
actions-section-recent-frequent = Recent & frequent
actions-section-bookmarks-favorites = Bookmarks & favorites
actions-section-path = Path
actions-section-search = Search
actions-section-shell = Shell
actions-section-recent-commands = Recent commands
actions-section-tabs = Tabs
actions-tag-recent = recent
actions-tag-current = current
actions-tag-bookmark = bookmark
actions-tag-favorite = favorite
actions-tag-here = here

## Command palette: rows
# $command: the command line of a custom command run in the terminal.
actions-palette-terminal-detail = Terminal · { $command }
# $path: the typed location.
actions-palette-go-to = Go to { $path }
actions-palette-go-hint = Enter to open · Ctrl+Enter opens in a new tab
actions-palette-listing = Listing folder…
# Used as $dir when the pane shows no real folder.
actions-palette-this-folder = this folder
actions-palette-search-empty = Type what to search for
actions-palette-search-empty-detail = Names in { $dir } and below · globs like *.rs work
actions-palette-search-names = Search names for “{ $query }”
actions-palette-search-contents = Search file contents for “{ $query }”
actions-palette-in-subfolders = in { $dir } and subfolders
actions-palette-run = Run  { $command }
actions-palette-run-detail = in { $dir } · output in the terminal panel
actions-palette-shell-recent-hint = Tab to edit · Enter to run
actions-palette-shell-empty = Type a command to run
actions-palette-in-dir = in { $dir }
actions-palette-no-matches = No matches
actions-palette-hint-move = move
actions-palette-hint-run = run
actions-palette-hint-complete = complete
# Between the chords of a key sequence ("G then G").
actions-palette-then = then

## Key sequences and marks
actions-keyseq-set-mark = m … set mark
actions-keyseq-jump-mark = ' … jump to mark
actions-mark-needs-folder = Marks need a real folder
# $mark: the mark character.
actions-mark-set = Mark '{ $mark }' set
actions-mark-missing = No mark '{ $mark }'

## Key-binding conflicts (settings)
# $keys: the shortcut ("Ctrl+K"); $actions: action titles; $context: where the keys apply.
actions-conflict-duplicate = { $keys } is bound to { $actions } in { $context }
actions-conflict-prefix = { $keys } waits for more keys, delaying { $actions }
# $other runs instead of the global $action.
actions-conflict-shadowed = { $keys } runs { $other } in the { $context } instead of { $action }
# Joins two action titles in a list.
actions-conflict-and = { $first } and { $second }

## Action feedback (toasts)
actions-sync-nav-on = Synchronized navigation on
actions-sync-nav-off = Synchronized navigation off
actions-showing-hidden = Showing hidden files
actions-hiding-hidden = Hiding hidden files
actions-folder-view-reset = Folder view reset
actions-apply-view-title = Apply this view to all folders?
actions-apply-view-message = Every folder will use the current layout, sorting and grouping. Folders you customized are reset.
actions-apply-view-confirm = Apply to all
actions-type-ahead-jump = Typing jumps to names
actions-type-ahead-filter = Typing filters the list
actions-open-with-needs-file = Select a file to open with another application
actions-ui-zoom = Interface zoom
# $percent: the new scale, e.g. "125".
actions-ui-zoom-body = { $percent } %  ·  Ctrl+0 resets
actions-high-contrast-on = High contrast on
actions-high-contrast-off = High contrast off
actions-screen-reader-on = Screen-reader mode on
actions-screen-reader-off = Screen-reader mode off
actions-config-create-failed = Couldn't create the config file
actions-index-on = File indexing is on
actions-index-off = File indexing is off
# $action: an action title.
actions-coming-soon = { $action } is coming soon

## Custom commands
# $name: the custom command's name.
actions-command-not-found = No custom command named “{ $name }”
actions-command-empty = “{ $name }” has no command
actions-command-run-failed = Couldn't run “{ $name }”
actions-command-start-failed = Couldn't start the command
actions-command-running = Running “{ $name }”

## Linux (glue)
action-root-desc-linux = Go to the file system root (/)
action-run-elevated-desc-linux = Run the selection as root (asks through polkit)
action-paste-shortcut-desc-linux = Paste the clipboard as symbolic links
action-set-attributes-desc-linux = Change permissions, file flags and timestamps
action-previous-versions-desc-linux = Browse and restore earlier versions from Btrfs / ZFS snapshots (Snapper, Timeshift)

# The built-in editors' commands (key context "Editor"; the terminal UI binds more of them).
action-focus-editor = Focus editor
action-focus-editor-desc = Give the keyboard back to the open editor document
action-editor-save = Editor: Save
action-editor-save-desc = Save the document in the editor
action-editor-save-as = Editor: Save as
action-editor-save-as-desc = Save a copy of the document under a new name and edit the copy (terminal UI)
action-editor-find = Editor: Find
action-editor-find-desc = Find text in the document
action-editor-replace = Editor: Replace
action-editor-replace-desc = Find and replace text in the document
action-editor-go-to = Editor: Go to line
action-editor-go-to-desc = Jump to a line (text) or an offset (hex)
action-editor-find-next = Editor: Find next
action-editor-find-next-desc = Select the next match
action-editor-find-prev = Editor: Find previous
action-editor-find-prev-desc = Select the previous match
action-editor-close-document = Editor: Close document
action-editor-close-document-desc = Close the document (asks to save changes first)
action-editor-next-document = Editor: Next document
action-editor-next-document-desc = Switch to the next editor tab
action-editor-prev-document = Editor: Previous document
action-editor-prev-document-desc = Switch to the previous editor tab
action-editor-toggle-wrap = Editor: Toggle word wrap
action-editor-toggle-wrap-desc = Wrap long lines at the panel's edge or scroll sideways
action-editor-maximize = Editor: Maximize
action-editor-maximize-desc = Let the editor fill the file panes' area, or restore it (terminal UI)
action-editor-toggle-dock = Editor: Switch dock side
action-editor-toggle-dock-desc = Dock the editor on the right or at the bottom (terminal UI)
action-editor-notice-action = Editor: Answer notice
action-editor-notice-action-desc = Run the action of the document's notice: reload a changed file, edit a lossy file anyway, allow editing a large file (terminal UI)
action-editor-match-case = Editor: Toggle match case
action-editor-match-case-desc = Match upper and lower case in find (terminal UI)
action-editor-whole-word = Editor: Toggle whole word
action-editor-whole-word-desc = Find whole words only (terminal UI)
action-editor-regex = Editor: Toggle regular expression
action-editor-regex-desc = Find with a regular expression (terminal UI)
action-editor-replace-all = Editor: Replace all
action-editor-replace-all-desc = Replace every match in the document (terminal UI)
action-editor-resize-image = Editor: Resize image
action-editor-resize-image-desc = Scale the image to a new size in pixels (terminal UI)
action-player-play-pause = Player: Play / pause
action-player-play-pause-desc = Start or pause the video or audio playing in the preview or Quick Look
action-player-seek-back = Player: Back
action-player-seek-back-desc = Jump back by the seek step ([media] seek_step_secs)
action-player-seek-forward = Player: Forward
action-player-seek-forward-desc = Jump forward by the seek step ([media] seek_step_secs)
action-player-volume-up = Player: Louder
action-player-volume-up-desc = Raise the player volume
action-player-volume-down = Player: Quieter
action-player-volume-down-desc = Lower the player volume
action-player-mute = Player: Mute / unmute
action-player-mute-desc = Turn the player sound off or on
action-toggle-screen-reader = Toggle screen-reader mode
action-toggle-screen-reader-desc = Plain text for screen readers: no box drawing, one pane at a time, the cursor on the focused item (terminal UI)
