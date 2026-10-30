### Soyuz UI strings (en-US source): app
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/app.ftl.

## File context menu (ui/context_menu.rs)

app-ctx-open = Open
app-ctx-open-in-new-tab = Open in new tab
app-ctx-open-in-other-pane = Open in other pane
app-ctx-open-with = Open with
app-ctx-open-with-looking = Looking for apps…
app-ctx-open-with-none = No registered apps found
app-ctx-open-with-choose = Choose another app…
app-ctx-edit-in-soyuz = Edit in Soyuz
app-ctx-cut = Cut
app-ctx-copy = Copy
app-ctx-paste-into-folder = Paste into folder
app-ctx-previous-versions = Restore previous version…
app-ctx-copy-to-other-pane = Copy to other pane
app-ctx-move-to-other-pane = Move to other pane
app-ctx-rename = Rename
app-ctx-duplicate = Duplicate
app-ctx-create-link = Create link…
app-ctx-move-to-trash = Move to trash
app-ctx-delete-permanently = Delete permanently
app-ctx-copy-path = Copy path
app-ctx-copy-paths = Copy paths
app-ctx-copy-name = Copy name
app-ctx-copy-names = Copy names
app-ctx-more = More
app-ctx-copy-as = Copy as
app-ctx-send-to = Send to
app-ctx-terminal = Terminal
app-ctx-tools = Tools
app-ctx-media = Media
app-ctx-paste-special = Paste special
app-ctx-commands = Commands
app-ctx-run-as-admin = Run as administrator
app-ctx-run-as-root = Run as root
app-ctx-reveal-in-system = Reveal in system file manager
app-ctx-open-terminal-here = Open terminal here
app-ctx-open-external-terminal = Open external terminal here
app-ctx-add-to-favorites = Add to favorites
app-ctx-folder-sizes = Calculate folder sizes
app-ctx-properties = Properties
app-ctx-view = View
app-ctx-sort-by = Sort by
app-ctx-group-by = Group by
app-ctx-refresh = Refresh
app-ctx-paste = Paste
app-ctx-paste-as-link = Paste as link
app-ctx-paste-and-verify = Paste and verify
app-ctx-new-folder = New folder
app-ctx-new-file = New file
app-ctx-new-link = New link…
app-ctx-copy-folder-path = Copy folder path
app-ctx-show-hidden = Show hidden files
app-ctx-item-checkboxes = Item checkboxes
app-ctx-select-all = Select all
app-ctx-columns = Columns
app-ctx-reset-columns = Reset columns

## View modes and sorting

app-view-details = Details
app-view-list = List
app-view-icons = Icons
app-view-gallery = Gallery
app-view-columns = Columns
app-sort-ascending = Ascending
app-sort-descending = Descending

## Folder colors and emblems (ui/looks.rs)

app-looks-folder-color = Folder color
# Screen-reader name of a color dot; $color is a translated color name.
app-looks-color-folder = { $color } folder
app-looks-no-color = No color
app-looks-emblem = Emblem
app-looks-no-emblem = No emblem
app-color-red = Red
app-color-orange = Orange
app-color-yellow = Yellow
app-color-green = Green
app-color-cyan = Cyan
app-color-blue = Blue
app-color-purple = Purple
app-color-gray = Gray
app-emblem-star = Star
app-emblem-heart = Heart
app-emblem-code = Code
app-emblem-image = Pictures
app-emblem-music = Music
app-emblem-film = Video
app-emblem-briefcase = Work
app-emblem-book = Books
app-emblem-cloud = Cloud
app-emblem-lock = Private
app-emblem-game = Games
app-emblem-archive = Archive

## Toolbar (ui/toolbar.rs)

app-toolbar-back = Back
app-toolbar-forward = Forward
app-toolbar-up = Up
app-toolbar-settings = Settings
app-toolbar-dual-pane = Dual pane (right-click: layouts, sync)
app-toolbar-preview-pane = Preview pane
app-toolbar-search = Search subfolders
app-toolbar-command-palette = Command palette
app-toolbar-sort = Sort
app-toolbar-view = View
app-toolbar-no-history = No history
app-toolbar-layout = Layout
app-toolbar-hidden-files = Hidden files
app-toolbar-sidebar = Sidebar
app-toolbar-choose-columns = Choose columns…
app-toolbar-reset-folder-view = Reset folder view
app-toolbar-apply-view-to-all = Apply view to all folders…
app-toolbar-folders-first = Folders first
app-toolbar-natural-order = Natural order (file2 < file10)
app-toolbar-sync-browsing = Synchronized browsing
app-toolbar-swap-panes = Swap panes
app-toolbar-mirror-pane = Mirror to other pane
app-toolbar-focus-other-pane = Focus other pane
app-toolbar-filter = Filter

## Status bar (ui/statusbar.rs)

app-status-items = { $count ->
    [one] { $count } item
   *[other] { $count } items
}
# Live filter active: $visible matching out of $count items.
app-status-items-filtered = { $count ->
    [one] { $visible } of { $count } item
   *[other] { $visible } of { $count } items
}
app-status-hidden = · { $count } hidden
app-status-selected = { $count } selected
# $size is a formatted size ("1.5 MB").
app-status-selected-size = { $count } selected  ({ $size })
# $size is the icon size in pixels.
app-status-icon-size = Icon size { $size }
app-status-free = { $free } free
app-status-free-of = { $free } free of { $total }
app-status-theme-system = Follow system theme
app-status-theme-light = Light theme
app-status-theme-dark = Dark theme

## Sidebar (ui/sidebar.rs)

app-sidebar-favorites = Favorites
app-sidebar-devices = Devices
app-sidebar-cloud = Cloud
app-sidebar-tags = Tags
app-sidebar-network = Network
app-sidebar-other = Other
app-sidebar-folders = Disk tree
app-sidebar-smart = Smart folders
app-sidebar-settings = Settings
# $key is a keyboard shortcut ("Ctrl+,").
app-sidebar-settings-tip = Settings  ({ $key })
app-sidebar-remove-favorite = Remove from favorites
app-sidebar-this-pc = This PC
app-sidebar-computer = Computer
# $path is the mount point; $free / $total are formatted sizes.
app-sidebar-volume-tip = { $path }
    { $free } free of { $total }
app-sidebar-scanning = Scanning…
app-sidebar-trash = Trash
# Icon bar at the top of the sidebar: switch between the sections and the disk tree.
app-sidebar-view-bar = Sidebar view
app-sidebar-view-places = Places
app-sidebar-view-places-tip = Places: favorites, devices, portal, tags and more
app-sidebar-view-tree = Disk tree
app-sidebar-view-tree-tip = Disk tree: your drives and home folder as one folder tree
# $tip is one of the tips above, $key a keyboard shortcut ("Ctrl+Shift+E").
app-sidebar-view-tip-key = { $tip }  ({ $key })

## Portal: folders pinned by dropping them on the sidebar section (ui/portal.rs)

app-sidebar-portal = Portal
app-portal-empty = Drop folders here to pin them
app-portal-missing = missing
# $path is the pinned folder.
app-portal-missing-tip = { $path }
    The folder was not found (moved, renamed or on a drive that is not connected).
app-portal-drop-badge = Pin to Portal
app-portal-open = Open
app-portal-rename = Rename label…
app-portal-reset-name = Use the folder's name
app-portal-unpin = Unpin from Portal
# $name is a folder name.
app-portal-pinned = Pinned “{ $name }” to Portal
app-portal-pinned-many = Pinned { $count } folders to Portal
app-portal-folders-only = Only folders can be pinned to Portal
app-portal-already = Already pinned to Portal
app-place-home = Home
app-place-desktop = Desktop
app-place-documents = Documents
app-place-downloads = Downloads
app-place-pictures = Pictures
app-place-music = Music
app-place-videos = Videos
# System folders below the favorites ([sidebar] system_folders).
app-sysfolders-section = System
app-sysfolders-hide = Hide system folders…
app-sysfolders-hide-tip = Hide the system folders
app-sysfolders-hide-title = Hide system folders?
app-sysfolders-hide-message = The system folders below your favorites leave the sidebar. You can show them again any time in Settings → Views & layout → System folders, where you can also choose which ones appear.
app-sysfolders-hide-confirm = Hide
app-sysfolder-appdata = AppData (Roaming)
app-sysfolder-local-appdata = AppData (Local)
app-sysfolder-locallow = AppData (LocalLow)
app-sysfolder-programdata = ProgramData
app-sysfolder-program-files = Program Files
app-sysfolder-program-files-x86 = Program Files (x86)
app-sysfolder-temp = Temp
app-sysfolder-public = Public
app-sysfolder-start-menu = Start menu programs
app-sysfolder-startup = Startup
app-sysfolder-fonts = Fonts
app-sysfolder-hosts = drivers\etc (hosts)
app-sysfolder-windows = Windows
app-sysfolder-soyuz-config = Soyuz settings
app-sysfolder-xdg-config = .config
app-sysfolder-xdg-data = .local/share
app-sysfolder-xdg-cache = .cache
app-sysfolder-xdg-state = .local/state
app-sysfolder-local-bin = .local/bin
app-sysfolder-applications = Applications
app-sysfolder-autostart = Autostart
app-sysfolder-flatpak-apps = Flatpak apps
app-sysfolder-etc = /etc
app-sysfolder-opt = /opt
app-sysfolder-var-log = Logs (/var/log)

## Address bar (ui/breadcrumb.rs)

app-crumb-address = Address
app-crumb-hidden-parents = Hidden parent folders
app-crumb-edit-path = { $name } (edit path)
app-crumb-go-to = Go to { $name }
app-crumb-refresh = Refresh
app-crumb-refresh-tip = Refresh (F5)
app-crumb-subfolders = Subfolders
app-crumb-cant-read = Can't read this folder
app-crumb-no-folders = No folders

## Title bar (ui/titlebar.rs)

app-titlebar-menu = Soyuz menu
app-titlebar-new-window = New window
app-titlebar-settings = Settings…
app-titlebar-shortcuts = Keyboard shortcuts
app-titlebar-about = About Soyuz
app-titlebar-quit = Quit
app-titlebar-minimize = Minimize
app-titlebar-maximize = Maximize
app-titlebar-restore = Restore
app-titlebar-close = Close

## Tab strip (ui/tabstrip.rs)

app-tabs-list = Tabs
app-tabs-group = Tab group
app-tabs-group-named = Tab group { $name }
app-tabs-expand-group = Expand group
app-tabs-collapse-group = Collapse group (right-click for more)
app-tabs-close-named = Close tab { $title }
app-tabs-new = New tab
# $key is a keyboard shortcut ("Ctrl+T").
app-tabs-new-tip = New tab ({ $key })
app-tabs-drop-new-window = Release to open in a new window
app-tabs-drop-other-window = Release to move to that window
app-tabs-drop-keep-one = A window keeps at least one tab
app-tabs-group-name = Group name
app-tabs-duplicate = Duplicate tab
app-tabs-pin = Pin tab
app-tabs-unpin = Unpin tab
app-tabs-close = Close tab
app-tabs-close-others = Close other tabs
app-tabs-close-right = Close tabs to the right
app-tabs-reopen-closed = Reopen closed tab

## Welcome card (ui/welcome.rs)

app-welcome-title = Welcome to Soyuz
app-welcome-dismiss = Dismiss
app-welcome-intro = Keyboard-first and never blocking. A few shortcuts to start with:
app-welcome-tip-palette = Every command, searchable
app-welcome-tip-jump = Jump to any folder you visited
app-welcome-tip-search = Search names and contents
app-welcome-tip-dual = Two folders side by side
app-welcome-tip-terminal = A terminal that follows you
app-welcome-tip-preview = Preview the selected file
app-welcome-got-it = Got it
app-welcome-settings = Settings

## Path picker (ui/path_picker.rs)

app-picker-parent = Parent folder
app-picker-folder-path = Folder path
app-picker-file-name = File name
# $name is a file name.
app-picker-replace = { $name } already exists. Press Save again to replace it.
app-picker-open = Open
app-picker-no-items = No matching items
# Screen-reader name of a row; $kind is "folder" or "file".
app-picker-row = { $kind ->
    [folder] { $name }, folder
   *[other] { $name }, file
}

## Shared widgets, toasts, app (ui/widgets.rs, ui/toasts.rs, app.rs)

app-hidden-controls-tip = Name contains hidden direction/format characters
app-icon-size = Icon size
app-toast-dismiss = Dismiss
app-quit-when-finished = Soyuz will close when the operations finish

## Linux (data)

app-device-mount = Mount
app-device-mount-failed = Couldn't mount { $name }
app-device-unmount = Unmount
app-device-unmounted = { $name } was unmounted
app-device-unmount-failed = Couldn't unmount { $name }
app-device-eject = Eject
app-device-ejected = { $name } can be removed safely
app-device-eject-failed = Couldn't eject { $name }
app-device-not-authorized = Not allowed. Is a polkit authentication agent running?
app-device-busy = The device is in use. Close the files and programs using it, then try again.
app-device-no-udisks = UDisks2 is not available on this system.

## Linux (devices)
app-device-mount-ro = Mount read-only
app-device-unlock = Unlock…
app-device-lock = Lock
app-device-hide = Hide from sidebar
app-device-copy-path = Copy device path
# $name: device name.
app-device-locked-done = { $name } was locked
app-device-lock-failed = Couldn't lock { $name }
app-device-auth-dismissed = The authentication prompt was cancelled.
app-device-hibernated = Windows didn't shut down fully (hibernation or Fast Startup), so the volume can only be mounted read-only.
# $fs: filesystem name (APFS, ReFS…).
app-device-unsupported-fs = Linux can't mount { $fs } volumes.
app-device-wrong-passphrase = Wrong passphrase or recovery key.
app-device-hibernated-title = { $name } is in use by Windows
app-device-hibernated-body = Windows was hibernated or shut down with Fast Startup, so its files may still be open there. Writing to the volume now could damage it. You can mount it read-only, or start Windows and shut it down completely (turn off Fast Startup in Control Panel › Power Options › Choose what the power buttons do) to use it normally.
app-device-hidden = Device hidden from the sidebar
app-device-hidden-body = Show it again in Settings › Views & layout › Sidebar sections.
# $size: e.g. "1.0 TB"; $fs: filesystem (NTFS, exFAT…).
app-device-volume-name = { $size } { $fs } volume
app-device-volume-name-nosize = { $fs } volume
app-device-removable = removable
app-device-internal = internal disk
app-device-click-mount = Click to mount
app-device-click-unlock = Click to unlock
app-device-unsupported-hint = Linux can't mount this filesystem
# Tooltip of an unmounted device: $name, $device (/dev/sdb1), $fs, $size, $drive (model · internal disk), $hint.
app-device-candidate-tip = { $name }
    { $device } · { $fs } · { $size }
    { $drive }
    { $hint }
app-device-status-unmounted = Not mounted · { $size }
app-device-status-locked = Locked · { $size }
app-device-status-unsupported = Not supported · { $size }
app-device-status-mounting = Mounting…
app-device-status-unlocking = Unlocking…
app-device-status-locking = Locking…
app-device-status-unmounting = Unmounting…

## a11y
app-a11y-resize-sidebar = Resize the sidebar
app-a11y-resize-activity = Resize the activity panel
app-a11y-resize-shelf = Resize the shelf
app-a11y-resize-preview = Resize the preview pane
app-a11y-resize-assistant = Resize the assistant panel
app-a11y-resize-panes = Resize the panes

## fix-svc
app-restart-cancelled = Restart cancelled. The update is installed and is used from the next start.
