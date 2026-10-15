### Soyuz UI strings (en-US source): dialogs
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/dialogs.ftl.

## Shared

dialogs-done = Done
dialogs-show = Show
dialogs-hide = Hide

## Select by pattern

dialogs-pattern-title = Select by pattern
dialogs-pattern-hint = Glob (*.rs, IMG_??.jpg) or plain text
dialogs-pattern-add = Add to the current selection
dialogs-pattern-select = Select
dialogs-pattern-selected = { $count ->
    [one] Selected { $count } item
   *[other] Selected { $count } items
}

## Columns

dialogs-columns-title = Columns

## About

dialogs-about-title = About Soyuz
dialogs-about-version-line = Version { $version }
dialogs-about-tagline = A keyboard-first, non-blocking file explorer.
dialogs-about-version = Version
dialogs-about-commit = Commit
dialogs-about-build = Build
dialogs-about-platform = Platform
dialogs-about-renderer = Renderer
dialogs-about-config = Config
dialogs-about-memory = Memory
dialogs-about-ffmpeg = FFmpeg
dialogs-about-ffmpeg-license = This software uses libraries from the FFmpeg project under the LGPLv2.1 (loaded at run time; see THIRD-PARTY-NOTICES.md).
# $thumbs / $previews: sizes like "12.5 MiB"; $thumb_count / $preview_count: cached entries.
dialogs-about-memory-line = thumbnails { $thumbs } ({ $thumb_count }) · previews { $previews } ({ $preview_count })
dialogs-about-license = Licensed under the MIT License. Fonts: Inter, Cascadia Mono, IBM Plex, Noto Sans, JetBrains Mono, Source Code Pro (OFL). Icons: Phosphor (MIT).
dialogs-about-copy-info = Copy info

## Keyboard shortcuts

dialogs-keys-title = Keyboard shortcuts
dialogs-keys-filter = Filter shortcuts

## Record shortcut

dialogs-record-title = Record shortcut
dialogs-record-press = Press a key combination…
dialogs-record-sequence-hint = Up to three chords make a sequence (e.g. G then G).
dialogs-record-unused = Not used by anything else.
# $action: action title; $context: where the binding applies ("File list").
dialogs-record-user = { $action } ({ $context })
# $users: comma-separated list of dialogs-record-user entries.
dialogs-record-conflict = Currently bound to { $users }. Saving moves it here.
dialogs-record-keys-hint = Enter saves · Esc cancels
dialogs-record-clear = Clear

## Delete confirmation

# $name: the item's name.
dialogs-delete-trash-one = Move “{ $name }” to the trash?
dialogs-delete-trash-many = { $count ->
    [one] Move { $count } item to the trash?
   *[other] Move { $count } items to the trash?
}
dialogs-delete-permanent-one = Delete “{ $name }” permanently?
dialogs-delete-permanent-many = { $count ->
    [one] Delete { $count } item permanently?
   *[other] Delete { $count } items permanently?
}
dialogs-delete-wipe-one = Securely wipe “{ $name }”?
dialogs-delete-wipe-many = { $count ->
    [one] Securely wipe { $count } item?
   *[other] Securely wipe { $count } items?
}
dialogs-delete-empty-title = Empty the trash?
dialogs-delete-trash-body = You can restore items from the trash later.
dialogs-delete-trash-confirm = Move to trash
dialogs-delete-permanent-body = Permanently deleted items can't be restored.
dialogs-delete-wipe-body = The data is overwritten before deletion. On SSDs and copy-on-write filesystems this cannot guarantee the old blocks are gone.
dialogs-delete-wipe-confirm = Wipe
dialogs-delete-purge-body = These items will be removed from the trash for good.
dialogs-delete-empty-body = { $count ->
    [one] All { $count } item in the trash will be deleted permanently.
   *[other] All { $count } items in the trash will be deleted permanently.
}
dialogs-delete-empty-confirm = Empty trash
# More names than the list shows.
dialogs-delete-and-more = and { $count } more

## Quit while jobs run

dialogs-quit-title = { $count ->
    [one] A file operation is still running
   *[other] { $count } file operations are still running
}
dialogs-quit-body = Quitting now cancels them and may leave partially copied files behind. You can keep Soyuz open, or let it close by itself once they finish.
dialogs-quit-cancel-jobs = Cancel jobs and quit
dialogs-quit-when-finished = Quit when finished
dialogs-quit-keep-open = Keep open

## Job prompts (conflict, error, space, trash, read-only, password)

# $total: prompts waiting, this one included.
dialogs-prompt-waiting = 1 of { $total } waiting
dialogs-prompt-cancel-op = Cancel operation
dialogs-conflict-title-file = Replace or skip?
dialogs-conflict-title-dir = Merge folders?
dialogs-conflict-title-kind = That name is taken
dialogs-conflict-title-same = Source and destination are the same
dialogs-conflict-title-into-self = Can't move a folder into itself
dialogs-conflict-title-case = Name differs only in case
dialogs-prompt-title-archive-limit = Archive expands beyond safety limit
dialogs-prompt-title-error = Something went wrong
dialogs-prompt-title-space = Not enough space
dialogs-prompt-title-trash = Can't move to the trash
dialogs-prompt-title-read-only = Read-only item
dialogs-prompt-title-password = Password required
dialogs-conflict-file-exists = The destination already has a file named “{ $name }”.
dialogs-conflict-dir-exists = The destination already has a folder named “{ $name }”.
dialogs-conflict-kind-differs = “{ $name }” exists as a different kind of item.
dialogs-conflict-same = The source and the destination are the same item.
dialogs-conflict-into-self = “{ $name }” can't go inside itself.
dialogs-conflict-case = “{ $name }” differs only by upper/lower case.
dialogs-conflict-source = Source
dialogs-conflict-destination = Destination
dialogs-conflict-folder = Folder
dialogs-conflict-larger = Larger
dialogs-conflict-newer = Newer
dialogs-conflict-new-name = New name
dialogs-conflict-apply-all = Do this for all remaining conflicts in this operation
dialogs-conflict-back = Back
dialogs-conflict-keep-both = Keep both
dialogs-conflict-merge = Merge
dialogs-conflict-replace = Replace
dialogs-conflict-more = More…
dialogs-conflict-replace-folder = Replace folder
dialogs-conflict-replace-newer = Replace if newer
dialogs-conflict-replace-larger = Replace if larger
dialogs-conflict-replace-different = Replace if size differs
dialogs-conflict-rename = Rename…
dialogs-prompt-archive-limit = “{ $name }” expands beyond the safety limit.
dialogs-prompt-archive-limit-hint = Archives built to fill the disk (decompression bombs) look like this. Limits are in Settings › File operations.
dialogs-prompt-abort = Abort
dialogs-prompt-abort-extract-tip = Stop extracting
dialogs-prompt-abort-tip = Stop the whole operation
dialogs-prompt-continue = Continue
dialogs-prompt-continue-tip = Ignore this limit for this extraction
dialogs-prompt-error = Couldn't process “{ $name }”.
dialogs-prompt-skip-all = Skip all
dialogs-prompt-skip-all-tip = Skip this and every later error
# $needed / $free: formatted sizes.
dialogs-prompt-space = “{ $name }” needs { $needed } but only { $free } is free.
dialogs-prompt-space-hint = Some items will fail when the disk fills up.
dialogs-prompt-continue-anyway = Continue anyway
dialogs-prompt-trash = “{ $name }” can't be moved to the trash.
dialogs-prompt-trash-permanent = Delete it permanently instead? This can't be undone.
dialogs-prompt-apply-all-items = Do this for all items in this operation
dialogs-prompt-delete-permanently = Delete permanently
dialogs-prompt-read-only = “{ $name }” is read-only.
dialogs-prompt-read-only-ask = Change it anyway?
dialogs-prompt-apply-all-read-only = Do this for all read-only items in this operation
dialogs-prompt-change-anyway = Change anyway
dialogs-prompt-encrypted = “{ $name }” is encrypted.
dialogs-prompt-wrong-password = That password didn't work. Try again.
dialogs-prompt-password = Password
dialogs-prompt-unlock = Unlock

## Properties

dialogs-menu-batch-rename = Batch rename…
dialogs-menu-attributes = Attributes & timestamps…
dialogs-menu-checksum = Checksum
dialogs-menu-compare = Compare with…
dialogs-props-owner-changed = Ownership changed
dialogs-props-owner-failed = Couldn't change ownership
dialogs-props-unavailable = Properties are available for files and folders on disk
dialogs-props-select-first = Select one or more items first
dialogs-props-tab-general = General
dialogs-props-tab-attributes = Attributes
dialogs-props-tab-security = Security
dialogs-props-tab-checksums = Checksums
dialogs-props-tab-details = Details
dialogs-props-tab-versions = Previous versions
dialogs-props-title-one = { $name } properties
dialogs-props-created = Created
dialogs-props-modified = Modified
dialogs-props-accessed = Accessed
dialogs-props-attributes = Attributes
dialogs-props-timestamps = Timestamps
# $size: formatted size; $bytes: exact byte count.
dialogs-props-exact-size = { $size } ({ $bytes } bytes)
# $kind: file type ("PNG image"); $size: formatted size.
dialogs-props-kind-size = { $kind } · { $size }
dialogs-props-selected = { $count } items selected
# $kind: the file type all selected items share.
dialogs-props-all-kind = All { $kind }
dialogs-props-mixed-types = Mixed types
# $counts: "3 files, 2 folders"; $kinds: dialogs-props-all-kind or dialogs-props-mixed-types.
dialogs-props-summary = { $counts } · { $kinds }
dialogs-props-type = Type
dialogs-props-location = Location
dialogs-props-size = Size
dialogs-props-size-on-disk = Size on disk
dialogs-props-contains = { $files ->
        [one] { $files } file
       *[other] { $files } files
    }, { $dirs ->
        [one] { $dirs } folder
       *[other] { $dirs } folders
    }
dialogs-props-contains-errors = { $files ->
        [one] { $files } file
       *[other] { $files } files
    }, { $dirs ->
        [one] { $dirs } folder
       *[other] { $dirs } folders
    } · { $errors } unreadable
dialogs-props-contains-label = Contains
dialogs-props-total = Total
dialogs-props-link-target = Link target
dialogs-props-dates = Dates
dialogs-props-touch-tip = Set modified and accessed to now
dialogs-props-multiple = Multiple values
dialogs-props-unknown = Unknown
dialogs-props-changed = changed
dialogs-props-revert = Revert
dialogs-props-no-created-linux = Linux file systems don't allow changing the creation time.
dialogs-props-reading = Reading metadata…
dialogs-props-copy-path = Copy full path
# $error: why the name is invalid.
dialogs-props-invalid-name = Invalid name: { $error }
dialogs-props-unsaved = Unsaved changes
dialogs-props-undo-note = Changes run as a background job and can be undone (Ctrl+Z).

## Attributes

dialogs-attr-readonly = Read-only
dialogs-attr-readonly-hint = Programs can read but not change the file
dialogs-attr-hidden = Hidden
dialogs-attr-hidden-hint = Not shown unless hidden files are visible
dialogs-attr-hidden-finder-hint = Hidden in Finder
dialogs-attr-system = System
dialogs-attr-system-hint = Used by the operating system
dialogs-attr-archive = Archive
dialogs-attr-archive-hint = Changed since the last backup
dialogs-attr-not-indexed = Not content indexed
dialogs-attr-not-indexed-hint = Excluded from the search index
dialogs-attr-temporary = Temporary
dialogs-attr-temporary-hint = Kept in memory when possible
dialogs-attr-offline = Offline
dialogs-attr-offline-hint = Data is not immediately available
dialogs-attr-compressed = Compressed (NTFS)
dialogs-attr-compressed-hint = Stored compressed by the file system
dialogs-attr-locked = Locked (immutable)
dialogs-attr-locked-hint = Cannot be changed, renamed or deleted
dialogs-attr-immutable = Immutable
dialogs-attr-immutable-hint = Cannot be changed, renamed or deleted (needs root)
dialogs-inode-title = Inode flags
dialogs-inode-append = Append-only
dialogs-inode-append-hint = Data can only be added; no rename or delete (needs root)
dialogs-inode-nodump = No dump
dialogs-inode-nodump-hint = Skipped by dump-style backups
dialogs-inode-noatime = No access time
dialogs-inode-noatime-hint = Reading doesn't update the access time
dialogs-inode-nocow = No copy-on-write
dialogs-inode-nocow-hint = btrfs: only takes effect on empty files, and on new files in a folder
dialogs-inode-compress = Compress
dialogs-inode-compress-hint = Stored compressed by the file system (btrfs)
dialogs-inode-root-note = Only root can change Immutable and Append-only.
dialogs-inode-unsupported = This file system has no inode flags.
# $error: the system's error message.
dialogs-inode-unreadable = Inode flags can't be read: { $error }
dialogs-inode-changed = Inode flags changed
dialogs-inode-failed = Couldn't change the inode flags
dialogs-inode-needs-root = Immutable and Append-only can only be changed by root (CAP_LINUX_IMMUTABLE), for example with sudo chattr.
dialogs-inode-denied = Only the owner (or root) can change these flags, and an immutable file must be made mutable first.
dialogs-inode-read-only-fs = The file system is mounted read-only.
dialogs-attr-encrypted = Encrypted
dialogs-attr-sparse = Sparse
dialogs-attr-reparse = Reparse point
dialogs-attr-cloud = Cloud placeholder
dialogs-attrs-title-one = Attributes of { $name }
dialogs-attrs-title-many = Attributes of { $count } items
dialogs-attrs-collecting = Collecting folder contents…
dialogs-attrs-all-now = All to now
dialogs-attrs-recursive = Also apply to everything inside the selected folders

## Unix permissions

dialogs-mode-permissions = Permissions
dialogs-mode-read = Read
dialogs-mode-write = Write
dialogs-mode-execute = Execute
dialogs-mode-owner = Owner
dialogs-mode-group = Group
dialogs-mode-others = Others
dialogs-mode-setuid = Set user ID
dialogs-mode-setgid = Set group ID
dialogs-mode-sticky = Sticky
dialogs-mode-octal = Octal
dialogs-mode-mixed = mixed

## Security

dialogs-sec-full-control = Full control
dialogs-sec-special = Special
dialogs-sec-unknown-user = unknown user “{ $name }”
dialogs-sec-unknown-group = unknown group “{ $name }”
dialogs-sec-ownership = Ownership
dialogs-sec-owner-admin = Changing ownership usually requires administrator rights.
dialogs-sec-owner-root = Only root can give a file to another user; the owner may change the group to one they belong to.
dialogs-sec-reading = Reading permissions…
dialogs-sec-unavailable = Permissions unavailable
dialogs-sec-your-access = Your access
dialogs-sec-entries = Access entries
dialogs-sec-no-entries = No entries (everyone has full access)
dialogs-sec-inherited = inherited
dialogs-sec-read-only-view = Read-only view. Advanced permission editing is not available.
dialogs-sec-read-only-view-unix = The access-control list is shown read-only (change it with setfacl). Owner, group and permission bits can be changed here and on the Attributes tab.

## Checksums

dialogs-hash-no-thread = cannot start a worker thread
dialogs-hash-compute-all = Compute all
dialogs-hash-not-computed = Not computed
dialogs-hash-compute = Compute
# $percent: whole number without the % sign.
dialogs-hash-percent = { $percent }%
dialogs-hash-compare = Compare
dialogs-hash-compare-hint = Paste a checksum to verify (MD5, SHA-1, SHA-256, BLAKE3 or CRC-32)
dialogs-hash-not-hex = Not a hexadecimal checksum
dialogs-hash-bad-length = Unrecognized checksum length
dialogs-hash-computing = Computing…
# $algo: algorithm name ("SHA-256").
dialogs-hash-match = Match · { $algo }
dialogs-hash-mismatch = Mismatch · the file differs

## Details

dialogs-details-full-path = Full path
dialogs-details-resolves-to = Resolves to
dialogs-details-target-missing = Target missing
dialogs-details-hard-links = Hard links
dialogs-details-file-id = File ID
dialogs-details-inode = Inode
dialogs-details-volume-serial = Volume serial
dialogs-details-device = Device
dialogs-details-reparse-tag = Reparse tag
dialogs-details-mode = Mode
dialogs-details-file-system = File system
dialogs-details-shortcut = Shortcut
dialogs-details-target = Target
dialogs-details-dimensions = Dimensions
dialogs-details-megapixels = Megapixels
dialogs-details-image = Image
dialogs-details-media = Media
dialogs-exif-camera = Camera
dialogs-exif-lens = Lens
dialogs-exif-taken = Taken
dialogs-exif-exposure = Exposure
dialogs-exif-aperture = Aperture
dialogs-exif-focal = Focal length
dialogs-exif-orientation = Orientation
dialogs-exif-software = Software
dialogs-exif-orient-2 = Mirrored
dialogs-exif-orient-3 = Rotated 180°
dialogs-exif-orient-4 = Mirrored vertically
dialogs-exif-orient-5 = Mirrored, rotated 90° CCW
dialogs-exif-orient-6 = Rotated 90° CW
dialogs-exif-orient-7 = Mirrored, rotated 90° CW
dialogs-exif-orient-8 = Rotated 90° CCW

## Date picker

dialogs-date-pick = Pick a date
dialogs-date-prev-month = Previous month
dialogs-date-next-month = Next month
dialogs-date-now = Now

## Create link

dialogs-link-title = Create link
dialogs-link-symlink = Symbolic link
dialogs-link-hardlink = Hard link
dialogs-link-junction = Junction
dialogs-link-symlink-hint = Points to a file or folder by path and may cross volumes. On Windows this needs Developer Mode or elevation.
dialogs-link-symlink-hint-unix = Points to a file or folder by path and may cross volumes.
dialogs-link-hardlink-hint = A second name for the same file data. Files only, on the same volume.
dialogs-link-junction-hint = A Windows folder link that needs no special rights. Folders only, local volumes.
dialogs-link-need-folder = Links can only be created in a folder on disk
dialogs-link-parent = Parent folder
dialogs-link-home = Home
dialogs-link-use-folder = Use this folder
dialogs-link-empty-folder = Empty folder
dialogs-link-picker-hint = Click to choose, double-click a folder to open it.
dialogs-link-target = Target
dialogs-link-target-hint = Path of the file or folder to link to
dialogs-link-browse = Browse
dialogs-link-checking = Checking target…
dialogs-link-missing = Target does not exist
dialogs-link-file = File
dialogs-link-relative = Store a relative path (the link keeps working when both move together)
dialogs-link-name = Link name
dialogs-link-name-hint = Name of the new link
# $dir: the folder the link is created in.
dialogs-link-created-in = Created in { $dir }
dialogs-link-in = In { $dir }
dialogs-link-choose-target = Choose a target.
dialogs-link-need-target = Enter a target path.
dialogs-link-exists = “{ $name }” already exists here.
dialogs-link-hard-files-only = Hard links can only point to files.
dialogs-link-hard-needs-file = A hard link needs an existing file.
dialogs-link-hard-same-volume = Hard links must be on the same volume as the target.
dialogs-link-junction-folders-only = Junctions can only point to folders.
dialogs-link-junction-needs-folder = A junction needs an existing folder.
dialogs-link-create = Create

## Copy to / Move to

dialogs-transfer-move-title = Move to folder
dialogs-transfer-copy-title = Copy to folder
dialogs-transfer-move-one = Move “{ $name }” to:
dialogs-transfer-copy-one = Copy “{ $name }” to:
dialogs-transfer-move-many = Move { $count } items to:
dialogs-transfer-copy-many = Copy { $count } items to:
dialogs-transfer-dest = Destination folder
dialogs-transfer-dest-hint = Folder path

## Open with

dialogs-openwith-title = Open “{ $name }” with
dialogs-openwith-looking = Looking up applications…
dialogs-openwith-none = No registered applications for this file type
# $app: application name.
dialogs-openwith-default-handler = { $app } (opens with the default handler)
dialogs-openwith-other = Or another program (full path):
dialogs-openwith-open = Open
dialogs-openwith-always = Always use this app for .{ $ext } files
dialogs-openwith-always-system = This application opens through the system, so it can't be the default in Soyuz
# $path: the typed program path.
dialogs-openwith-not-program = { $path } is not a program file
dialogs-openwith-choose = Choose an application or type a program path

## MCP approval

# $client: name of the MCP client.
dialogs-mcp-title-destructive = { $client } wants to delete or overwrite files
dialogs-mcp-title = { $client } wants to change files
dialogs-mcp-title-unknown = An AI client wants to change files
dialogs-mcp-note = Approved plans run as normal background jobs you can pause, cancel and undo.

## Crash recovery

dialogs-recovery-title = { $count ->
    [one] Soyuz closed during an operation
   *[other] Soyuz closed during { $count } operations
}
# $percent: whole number.
dialogs-recovery-done = { $percent }% done
dialogs-recovery-progress-unknown = progress unknown
# $started: date and time; $done: dialogs-recovery-done or dialogs-recovery-progress-unknown.
dialogs-recovery-job-line = Started { $started } · { $done }
dialogs-recovery-job-line-undo = Started { $started } · { $done } · { $count } completed part(s) can be rolled back
dialogs-recovery-body = These operations were still running when Soyuz stopped. Resume them, roll back what they already did (moved to the trash where possible), or leave things as they are.
dialogs-recovery-scanning = Looking for leftover temporary files…
dialogs-recovery-no-leftovers = No leftover temporary files found.
dialogs-recovery-clean = { $count ->
    [one] Move { $count } leftover temporary file to the trash
   *[other] Move { $count } leftover temporary files to the trash
}
dialogs-recovery-later = Decide later
dialogs-recovery-later-tip = Ask again next time Soyuz starts
dialogs-recovery-resume = Resume
dialogs-recovery-resume-tip = Finish what is left
dialogs-recovery-rollback = Roll back
dialogs-recovery-rollback-tip = Undo the parts that were completed
dialogs-recovery-rollback-none = Nothing was completed yet
dialogs-recovery-discard = Discard
dialogs-recovery-discard-tip = Leave everything as it is
dialogs-crash-title = { $count ->
    [one] Soyuz closed unexpectedly
   *[other] Soyuz recorded { $count } crash reports
}
dialogs-crash-truncated = … (open the file to see the rest)
dialogs-crash-body = A crash report was saved on this computer. It contains the version, the operating system, where the problem happened and the last log lines. It is never sent anywhere.
dialogs-crash-view = View
dialogs-crash-delete = Delete reports
dialogs-crash-open-folder = Open folder
dialogs-crash-open-folder-tip = Show the crashes folder in a new tab
dialogs-crash-safe-mode = Restart in safe mode
dialogs-crash-safe-mode-tip = Default settings; no session restore, watchers, thumbnails or git

## Batch rename

dialogs-rename-unavailable = Batch rename works on files and folders on disk
dialogs-rename-nothing = Nothing to rename
dialogs-rename-title = Rename { $count } items
dialogs-rename-rules = Rules
dialogs-rename-editor = Text editor
dialogs-rename-presets = Presets
dialogs-rename-builtin = Built-in
dialogs-rename-saved = Saved
dialogs-rename-preset-name = Preset name
dialogs-rename-delete-preset = Delete saved preset
dialogs-rename-edit = Edit
dialogs-rename-remove-rule = Remove rule
dialogs-rename-move-down = Move down
dialogs-rename-move-up = Move up
dialogs-rename-add-rule = + Add rule
dialogs-rename-insert-prefix = Insert prefix
dialogs-rename-insert-suffix = Insert suffix
# $token: the counter token "{n}" (kept as is).
dialogs-rename-counter = Counter  { $token }
# $tokens: list of rename tokens; $group / $named: regex capture references ("$1", "${name}").
dialogs-rename-tokens-hint = Tokens: { $tokens }. Regex replacements can use { $group } or { $named }.
dialogs-rename-one-per-line = One name per line
dialogs-rename-editor-hint = Edit freely; line N renames item N. Paste from a spreadsheet works too.
dialogs-rename-reset-names = Reset to current names
dialogs-rename-current-name = Current name
dialogs-rename-duplicate = Another item gets the same name
dialogs-rename-exists = A file with this name already exists
dialogs-rename-position = Position
dialogs-rename-pos-start = Start
dialogs-rename-pos-end = End
dialogs-rename-pos-at = At
dialogs-rename-pos-index-tip = Character index; negative counts from the end
dialogs-rename-find = Find
dialogs-rename-find-hint = text to find
dialogs-rename-find-regex-hint = e.g. IMG_(\d+)
dialogs-rename-replace = Replace
dialogs-rename-replace-hint = replacement (tokens allowed)
dialogs-rename-replace-regex-hint = e.g. Photo $1
dialogs-rename-regex = Regex
dialogs-rename-match-case = Match case
dialogs-rename-first-only = First only
dialogs-rename-text = Text
# $token: an example token ("{date:%Y%m%d}_").
dialogs-rename-text-hint = e.g. { $token } or Draft
dialogs-rename-remove-count = Remove
dialogs-rename-remove-start = chars starting at
dialogs-rename-from-end = from the end
dialogs-rename-separator = Separator
dialogs-rename-separator-hint = e.g. _ or space
dialogs-rename-pattern = Pattern
dialogs-rename-collapse = Also collapse repeated spaces
dialogs-rename-counter-start = Start
dialogs-rename-counter-step = Step
dialogs-rename-counter-digits = Digits
dialogs-rename-per-folder = Per folder
dialogs-rename-to-rename = { $count } to rename
dialogs-rename-problems = { $count ->
    [one] { $count } problem
   *[other] { $count } problems
}
dialogs-rename-cycles = { $count ->
    [one] { $count } swap cycle · temporary names used
   *[other] { $count } swap cycles · temporary names used
}
dialogs-rename-go = { $count ->
    [one] Rename { $count } item
   *[other] Rename { $count } items
}
dialogs-rename-target-stem = Name only
dialogs-rename-target-ext = Extension only
dialogs-rename-target-whole = Name + extension
dialogs-rename-case-lower = lowercase
dialogs-rename-case-upper = UPPERCASE
dialogs-rename-case-title = Title Case
dialogs-rename-case-sentence = Sentence case
dialogs-rename-case-invert = iNVERT cASE
dialogs-rename-rule-regex = Replace (regex)
dialogs-rename-rule-replace = Replace
dialogs-rename-rule-case = Change case
dialogs-rename-rule-insert = Insert text
dialogs-rename-rule-remove = Remove characters
dialogs-rename-rule-number = Numbering
dialogs-rename-rule-template = Template
dialogs-rename-rule-trim = Trim spaces
dialogs-rename-preset-photos = Photos by capture date
dialogs-rename-preset-folder-number = Folder name + number
dialogs-rename-preset-underscores = Spaces to underscores
dialogs-rename-preset-lower-ext = Lower-case extensions
dialogs-rename-preset-date-prefix = Prefix with modified date
# $format: the strftime pattern the user typed.
dialogs-rename-err-date-format = invalid date format “{ $format }”
dialogs-rename-err-unmatched = unmatched “{"}"}”
dialogs-rename-err-unclosed = unclosed “{"{"}”
# $token: the token text without braces.
dialogs-rename-err-padding = bad padding in {"{"}{ $token }{"}"}
dialogs-rename-err-token = unknown token {"{"}{ $token }{"}"}
# $rule: rule number (1-based); $error: what is wrong.
dialogs-rename-err-rule = rule { $rule }: { $error }
dialogs-rename-err-no-target = no target
# $lines: lines in the editor; $count: items being renamed.
dialogs-rename-err-lines = { $lines } lines for { $count } items: keep exactly one name per line

## Linux (devices)
# $name: the volume's name.
dialogs-unlock-title = Unlock { $name }
dialogs-unlock-intro = This volume is encrypted. Enter its passphrase to unlock and mount it.
dialogs-unlock-intro-bitlocker = This volume is encrypted with BitLocker. Enter its password or recovery key to unlock and mount it.
dialogs-unlock-hint = Passphrase
dialogs-unlock-hint-bitlocker = Password or recovery key
dialogs-unlock-show = Show passphrase
dialogs-unlock-hide = Hide passphrase
dialogs-unlock-working = Unlocking…
dialogs-unlock-recovery-note = A recovery key is the 48-digit key Windows showed when BitLocker was turned on (eight groups of six digits, dashes optional).
dialogs-unlock-button = Unlock

## fix-ui

dialogs-inode-applying = Applying…
# $failed / $total: item counts.
dialogs-inode-failed-some = Couldn't change the inode flags of { $failed } of { $total } items
dialogs-unlock-interrupted = The unlock didn't start: another operation is still running on this device. Type the passphrase again and retry.

## Media tools (Phase V: Convert, Extract audio, Trim, Save frame)

dialogs-media-title-convert = Convert media
dialogs-media-title-extract = Extract audio
dialogs-media-title-trim = Trim media
dialogs-media-title-frame = Save video frame
dialogs-media-tool-convert = Convert
dialogs-media-tool-extract = Extract audio
dialogs-media-tool-trim = Trim
dialogs-media-tool-frame = Save frame
dialogs-media-go-convert = Convert
dialogs-media-go-extract = Extract
dialogs-media-go-trim = Trim
dialogs-media-go-frame = Save
# $count files are selected; $first is the one shown in the dialog.
dialogs-media-sources-many = { $count } files (showing { $first })
dialogs-media-tool = Tool
dialogs-media-input = Input
dialogs-media-reading = Reading the file…
dialogs-media-input-failed = The file can't be read: { $error }
dialogs-media-preset = Preset
dialogs-media-codec = Audio codec
dialogs-media-codec-copy = Keep as is
dialogs-media-start = Start
dialogs-media-end = End
dialogs-media-time-hint = seconds or m:ss
dialogs-media-stream-copy = Cut on keyframes (no re-encoding, fast)
dialogs-media-at = Time
dialogs-media-format = Picture format
dialogs-media-name = Output name
dialogs-media-name-hint = file name beside the source
dialogs-media-exists = A file with this name exists
dialogs-media-conflict-ask = Ask
dialogs-media-conflict-keep-both = Keep both
dialogs-media-conflict-overwrite = Replace
dialogs-media-conflict-skip = Skip
dialogs-media-plan = Plan
dialogs-media-planning = Planning…
dialogs-media-plan-failed = { $error }
dialogs-media-stream-n = stream { $n }
dialogs-media-action-copy = copy
dialogs-media-action-encode = encode with { $encoder }
dialogs-media-action-drop = dropped ({ $why })
dialogs-media-estimated = about { $size }
dialogs-media-per-file = One job per file, { $count } in all
dialogs-media-name-empty = Type an output name
dialogs-media-name-invalid = The output name isn't a valid file name
dialogs-media-name-is-source = The output would replace the source file; choose another name
dialogs-media-time-invalid = Times are seconds (12.5) or minutes and seconds (1:05.5)
dialogs-media-trim-order = The end must come after the start
dialogs-media-time-beyond = The file is only { $duration } long
dialogs-media-no-video = The file has no video stream
dialogs-media-no-audio = The file has no audio stream
dialogs-media-no-presets = This FFmpeg build has no usable encoder
dialogs-media-tools-off = The media tools are off (Settings → Preview → Media)
dialogs-media-no-runtime = The FFmpeg libraries aren't loaded
dialogs-media-select-file = Select a video or audio file first
dialogs-media-preset-mkv-copy = Matroska (copy streams)
dialogs-media-preset-mp4-copy = MP4 (copy streams)
dialogs-media-preset-mp4-h264-hw = MP4 / H.264 (hardware encoder)
dialogs-media-preset-mp4-hevc-hw = MP4 / HEVC (hardware encoder)
dialogs-media-preset-webm-vp9 = WebM / VP9 + Opus
dialogs-media-preset-webm-vp8 = WebM / VP8 + Opus
dialogs-media-preset-mov-prores = MOV / ProRes HQ
dialogs-media-preset-mp4-mpeg4 = MP4 / MPEG-4 + AAC
dialogs-media-preset-avi-mpeg4 = AVI / MPEG-4 + MP2
dialogs-media-preset-audio-flac = FLAC
dialogs-media-preset-audio-aac = AAC (M4A)
dialogs-media-preset-audio-opus = Opus
dialogs-media-preset-audio-wav = WAV (16-bit PCM)
dialogs-media-preset-audio-alac = Apple Lossless (M4A)
