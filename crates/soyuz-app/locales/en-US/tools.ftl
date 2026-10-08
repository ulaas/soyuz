### Soyuz UI strings (en-US source): tools
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/tools.ftl.

## Tool launch toasts

tools-toast-open-folder-treemap = Open a folder to analyze its disk usage
tools-toast-open-folder-compare = Open a folder to compare it
tools-toast-select-file-split = Select one file to split
tools-toast-select-first-part = Select the first part of a split file (.001)
tools-toast-select-versions = Select one file or folder to see its previous versions
# $name: sync profile name.
tools-toast-no-profile = No sync profile named “{ $name }”
tools-toast-versions-off = Previous versions are turned off
tools-toast-versions-off-body = Turn them on in Settings › Jobs › Sync & versions.
# $tag: tag color name (tools-tag-*).
tools-toast-tagged = Tagged { $count ->
    [one] 1 item
   *[other] { $count } items
    } { $tag }
tools-toast-untagged = Removed { $tag } from { $count ->
    [one] 1 item
   *[other] { $count } items
}

## Tags

tools-tag-red = Red
tools-tag-orange = Orange
tools-tag-yellow = Yellow
tools-tag-green = Green
tools-tag-cyan = Cyan
tools-tag-blue = Blue
tools-tag-purple = Purple
tools-tag-gray = Gray
tools-tags-menu = Tags
tools-tags-section = Tags
tools-tags-clear = Clear tags
tools-tags-count = { $count } tags
tools-recent = Recent

## Context menu rows

tools-menu-add-to-shelf = Add to shelf
tools-menu-analyze-disk-usage = Analyze disk usage
tools-menu-find-duplicates = Find duplicates
tools-menu-compare-folders = Compare folders
tools-menu-compare-folders-dialog = Compare folders…
tools-menu-compare-other-pane = Compare with other pane
tools-menu-join-parts = Join parts…
tools-menu-split-file = Split file…
tools-menu-clear-recent = Clear recent items
tools-menu-remove-from-recent = Remove from Recent

## Git decorations

tools-git-ignored = Ignored
tools-git-untracked = Untracked
tools-git-type-changed = Type changed
tools-git-renamed = Renamed
tools-git-added = Added
tools-git-deleted = Deleted
tools-git-modified = Modified
tools-git-conflict = Conflict
# $status: one of the tools-git-* status names.
tools-git-staged = { $status } (staged)
tools-git-detached = detached
# Status bar git chip tooltip. $root: repository folder.
tools-git-tip-repo = Git repository: { $root }
    Branch: { $branch }
tools-git-tip-counts = { $changed } changed · { $staged } staged · { $untracked } untracked
tools-git-tip-counts-conflicts = { $changed } changed · { $staged } staged · { $untracked } untracked · { $conflicts } conflicts
tools-git-tip-click = Click to open the repository root

## Shelf

tools-shelf-title = Shelf
tools-shelf-tip-count = Shelf · { $count } items
tools-shelf-tip-empty = Shelf (drop files here to collect them)
tools-shelf-hide = Hide shelf
tools-shelf-clear = Clear shelf
tools-shelf-empty-title = Drop files here
tools-shelf-empty-body = Collect items from different folders, then drag them out or copy them all at once.
tools-shelf-missing = Missing
# $size: formatted file size; $folder: parent folder name.
tools-shelf-item-sub = { $size } · { $folder }
tools-shelf-item-tip = { $path }
    Drag out to copy or move · double-click to reveal
tools-shelf-reveal = Reveal in pane
tools-shelf-remove = Remove from shelf
# $which: "all" | "selected"; $dest: target folder name.
tools-shelf-footer = { $which ->
    [selected] Selected
   *[other] All
    } { $count } → { $dest }
tools-shelf-copy-here = Copy here
tools-shelf-move-here = Move here

## Shared tool words

tools-stop = Stop
tools-rescan = Rescan
tools-open = Open
tools-open-in-pane = Open in pane
tools-reveal-in-system = Reveal in system file manager
tools-copy-path = Copy path
tools-move-to-trash = Move to trash
tools-move-to-trash-dialog = Move to trash…
# $pct: formatted decimal number.
tools-percent = { $pct }%
tools-files = { $count ->
    [one] { $count } file
   *[other] { $count } files
}

## Disk usage (treemap)

tools-treemap-title = Disk usage
tools-treemap-up = Up one level
# $size: formatted size.
tools-treemap-totals = { $size }  ·  { $count ->
    [one] { $count } file
   *[other] { $count } files
}
tools-treemap-listing = Listing…
# $current: name of the folder being scanned.
tools-treemap-scanning = Scanning { $done } of { $total } folders · { $size } so far · { $current }
tools-treemap-failed = Can't scan this folder: { $error }
# $secs: formatted seconds.
tools-treemap-scanned = Scanned in { $secs } s
tools-treemap-scanned-errors = Scanned in { $secs } s · { $errors } folders could not be read
tools-treemap-legend-folders = Folders
tools-treemap-legend-images = Images
tools-treemap-legend-video = Video
tools-treemap-legend-audio = Audio
tools-treemap-legend-archives = Archives
tools-treemap-legend-code = Code
tools-treemap-legend-documents = Documents
tools-treemap-legend-programs = Programs
tools-treemap-legend-other = Other
tools-treemap-confirm-trash = Move “{ $name }” ({ $size }) to the trash?
tools-treemap-measuring = Measuring…
tools-treemap-empty-folder = This folder is empty
tools-treemap-single-file = A single file
tools-treemap-tab-contents = Contents
tools-treemap-tab-largest = Largest files
tools-treemap-no-files = No files at this level
tools-treemap-many-small = Many small items
tools-treemap-zoom-in = Zoom into folder
tools-treemap-smaller-items = Smaller items
# $parent / $view: formatted percentages (tools-percent).
tools-treemap-share = { $parent } of parent · { $view } of view
tools-treemap-measure-deeper = Click to measure deeper levels
# Screen reader name of a treemap tile.
tools-treemap-tile-a11y = { $name }, { $size }
tools-add = Add
tools-clear = Clear

## Duplicate finder

tools-dupes-title = Duplicate finder
tools-dupes-folders = { $count } folders
tools-dupes-search-in = Search in
tools-dupes-root-tip = { $path }
    Click to remove
tools-dupes-add-path-hint = Add a folder path…
tools-dupes-add-other-pane = Add other pane
tools-dupes-min-size = Minimum size
tools-dupes-size-any = Any
# $size: bytes read so far; $secs: elapsed seconds.
tools-dupes-hashing = Hashing { $done } of { $total } candidates · { $size } read · { $groups } groups so far · { $secs } s
# $groups: number of groups; $size: reclaimable size.
tools-dupes-summary = { $groups ->
    [one] { $groups } group of duplicates
   *[other] { $groups } groups of duplicates
    } · { $size } reclaimable
tools-dupes-summary-secs = { $groups ->
    [one] { $groups } group of duplicates
   *[other] { $groups } groups of duplicates
    } · { $size } reclaimable · { $secs } s
tools-dupes-summary-ms = { $groups ->
    [one] { $groups } group of duplicates
   *[other] { $groups } groups of duplicates
    } · { $size } reclaimable · { $ms } ms
tools-dupes-empty-running = Looking for duplicates…
tools-dupes-empty-running-body = Groups appear here as soon as they are confirmed.
tools-dupes-empty-handled = All duplicates handled
tools-dupes-empty-handled-body = The marked copies were moved to the trash (Undo is in the toast).
tools-dupes-empty-none = No duplicates found
tools-dupes-empty-none-body = Every file under these folders is unique.
tools-dupes-empty-start = Find identical files
tools-dupes-empty-start-body = Choose folders and a minimum size, then start.
tools-dupes-select-every-group = Select in every group:
tools-dupes-keep-newest = Keep newest
tools-dupes-keep-oldest = Keep oldest
tools-dupes-keep-shortest = Keep shortest path
tools-dupes-newest = Newest
tools-dupes-oldest = Oldest
tools-dupes-shortest = Shortest path
tools-dupes-keep = Keep
tools-dupes-keep-one = Keep at least one copy
tools-dupes-mark-hint = Mark the copies you want to remove
tools-dupes-marked = { $count ->
    [one] { $count } file
   *[other] { $count } files
    } marked · { $size }
tools-dupes-move-n-to-trash = Move { $count } to trash
tools-dupes-toast-moving = Moving { $count ->
    [one] { $count } duplicate
   *[other] { $count } duplicates
    } to the trash
tools-dupes-delete-marked = Delete marked…
# $size: size of one copy.
tools-dupes-group-title = { $count } copies · { $size } each
tools-dupes-reclaimable = { $size } reclaimable
tools-dupes-row-tip = { $path }
    Click to mark · double-click to reveal
tools-done = Done
tools-cancelled = Cancelled

## Split / join

tools-split-title = Split file
tools-join-title = Join files
tools-split-err-not-part = Not a split part (expected name.001)
tools-split-err-not-found = { $path } not found
tools-split-err-open = Cannot open { $path }: { $error }
tools-split-err-create = Cannot create { $path }: { $error }
tools-split-err-read = Cannot read { $path }: { $error }
tools-split-err-write = Cannot write { $path }: { $error }
tools-split-err-exists = { $path } already exists
tools-split-err-zero-size = Part size must be greater than zero
tools-split-err-no-name = No file name
tools-split-err-invalid-size = Enter a valid part size (e.g. 250 MB)
tools-split-done = File split
# $checksum: checksum file name.
tools-split-done-body = Created { $count } parts and { $checksum }
tools-join-done = Files joined
tools-join-verified = { $name } · checksum verified
tools-join-mismatch = Checksum mismatch
tools-join-mismatch-body = The joined file does not match its checksum; kept as { $name }
tools-join-unverified = { $name } · no checksum file to verify
tools-split-label-file = FILE
tools-split-unknown-size = unknown size
tools-split-label-part-size = PART SIZE
tools-split-custom = Custom
tools-split-custom-hint = e.g. 250 MB
# $size: size of one part.
tools-split-plan = { $count ->
    [one] { $count } part
   *[other] { $count } parts
    } of { $size } (+ a .blake3 checksum file)
tools-split-label-parts = PARTS
# $name: name of the joined file.
tools-join-plan = { $count } parts → { $name }
tools-join-has-checksum = A checksum file was found; the result will be verified.
tools-join-no-checksum = No checksum file; the result will not be verified.
tools-split-label-output = OUTPUT FOLDER
tools-split-folder-hint = Folder
# $done / $total / $speed: formatted sizes.
tools-split-progress = { $done } of { $total }  ·  { $speed }/s
tools-split-start = Split
tools-join-start = Join
tools-back = Back

## Compare & sync

tools-compare-title = Compare & sync
tools-compare-only-left = Only left
tools-compare-newer-left = Newer left
tools-compare-different = Different
tools-compare-newer-right = Newer right
tools-compare-only-right = Only right
tools-preset-update = Two-way update
tools-preset-mirror-right = Mirror left → right
tools-preset-mirror-left = Mirror right → left
tools-preset-two-way = Two-way sync
tools-preset-two-way-tip = Remembers the last sync, so deletions and edits on both sides are told apart. Changes on both sides are asked as conflicts.
tools-method-date-size = Date & size
tools-method-size = Size
tools-method-content = Content
tools-reason-deleted-right = Deleted right
tools-reason-deleted-left = Deleted left
tools-reason-new-left = New left
tools-reason-new-right = New right
tools-reason-changed-left = Changed left
tools-reason-changed-right = Changed right
tools-conflict-both-changed = Changed on both sides
tools-conflict-both-created = Different on both sides
tools-conflict-changed-left-deleted-right = Changed left · deleted right
tools-conflict-deleted-left-changed-right = Deleted left · changed right
tools-resolution-left = Keep left
tools-resolution-right = Keep right
tools-resolution-both = Keep both
tools-compare-err-choose = Choose two folders to compare.
tools-compare-err-same = Both sides are the same folder.
tools-compare-state-save-failed = Couldn't save the sync state
# Job name in the jobs panel; $left / $right: folder names.
tools-compare-job-label = Sync { $left } ↔ { $right }
tools-compare-left-hint = Left folder
tools-compare-right-hint = Right folder
tools-compare-swap = Swap sides
tools-compare-by = Compare by
tools-compare-sync-mode = Sync mode
tools-compare-start = Compare
tools-compare-empty = Pick two folders and press Compare
tools-compare-empty-body = Differences show up here, ready to sync.
# $secs: formatted elapsed seconds.
tools-compare-running = Comparing… { $secs } s
tools-compare-syncing = Syncing… the folders are compared again when the job finishes.
tools-compare-recording = Recording the sync state…
tools-compare-failed = Can't compare these folders
tools-compare-identical = · { $count } identical
tools-compare-filter-hint = Filter names
tools-compare-first-run = First two-way sync of these folders: newer files win, unclear ones are conflicts. From now on deletions and edits on both sides are tracked.
tools-compare-in-sync = The folders are in sync
tools-compare-in-sync-body = No differences were found.
tools-compare-review = Review sync…
tools-compare-resolve-n = { $count ->
    [one] Resolve { $count } conflict…
   *[other] Resolve { $count } conflicts…
}
tools-compare-conflicts-button = Conflicts…
tools-profiles = Profiles
tools-profiles-none = No saved profiles
# $method / $mode: compare method and sync mode names.
tools-profile-tip = { $left }
    ↔ { $right }
    { $method } · { $mode }
tools-profiles-save-as = Save current as
tools-profiles-name-hint = Profile name
tools-profiles-delete-hint = Right-click a profile to delete it
tools-profiles-schedules = Schedules & reports…
tools-compare-col-left = LEFT
tools-compare-col-name = NAME
tools-compare-col-right = RIGHT
# Appended in parentheses to the bulk action tooltips.
tools-compare-all-visible = all visible rows
# $modified: formatted date.
tools-compare-meta-folder = Folder  ·  { $modified }
tools-compare-meta-folder-only = Folder
tools-compare-meta-deleted = Deleted
# $meta: size and date, "Folder" or "Deleted".
tools-compare-meta-left = Left: { $meta }
tools-compare-meta-right = Right: { $meta }
tools-compare-resolved = Resolved
tools-compare-plan-title = Review the sync plan
tools-compare-plan-trash-note = Deleted items go to the trash when the volume supports it.
tools-compare-plan-open-conflicts = { $count ->
    [one] { $count } unresolved conflict will be skipped and asked again next time.
   *[other] { $count } unresolved conflicts will be skipped and asked again next time.
}
tools-compare-copy-right = Copy to right
tools-compare-copy-left = Copy to left
tools-compare-delete-left = Delete on left
tools-compare-delete-right = Delete on right
tools-compare-conflict-copy-left = Conflict copy ←
tools-compare-conflict-copy-right = Conflict copy →
tools-compare-start-sync = Start sync ({ $count })
tools-compare-conflicts-title = Resolve conflicts
tools-compare-conflicts-body = { $count ->
    [one] { $count } item changed on both sides since the last sync. Choose which version wins; skipped items are asked again next time.
   *[other] { $count } items changed on both sides since the last sync. Choose which version wins; skipped items are asked again next time.
}
tools-compare-apply-all = Apply to all
tools-compare-keep-newer = Keep newer
tools-compare-keep-newer-tip = For each item, the side modified last wins
tools-compare-all-decided = All decided
tools-compare-n-open = { $count } open
tools-compare-res-delete-right = Delete it on the right too
tools-compare-res-delete-left = Delete it on the left too
tools-compare-res-left = Copy the left version over the right one
tools-compare-res-right = Copy the right version over the left one
tools-compare-res-both = Left keeps the name; the right version is kept as a conflict copy on both sides
tools-compare-res-skip = Decide later
tools-plan-to-right = { $count } → right
tools-plan-to-left = { $count } ← left
tools-plan-to-delete = { $count } to delete
# $size: formatted size.
tools-plan-to-copy = { $size } to copy

## Sync schedules and reports

tools-sched-manual = Manual
tools-sched-every-hours = { $count ->
    [one] Every hour
   *[other] Every { $count } hours
}
tools-sched-every-minutes = Every { $count } min
# $time: HH:MM.
tools-sched-daily = Daily at { $time }
tools-sched-on-change = When files change
tools-sched-on-start = When Soyuz starts
tools-trigger-manual = manual
tools-trigger-schedule = scheduled
tools-trigger-change = on change
tools-trigger-start = at start
tools-status-in-sync = In sync
tools-status-dry-run = Dry run
tools-status-applied = Synced
tools-status-partial = Partly synced
tools-status-failed = Failed
tools-report-keep-copy = Keep a copy
# Report summary parts, joined with " · ".
tools-report-nothing = nothing to change
tools-report-copy = copy { $count }
tools-report-would-copy = would copy { $count }
tools-report-delete = delete { $count }
tools-report-would-delete = would delete { $count }
tools-report-actions-total = { $count } actions in all
tools-report-failed = { $count } failed
tools-report-conflicts = { $count ->
    [one] { $count } conflict skipped
   *[other] { $count } conflicts skipped
}
tools-report-state-not-saved = synced, but the sync state could not be saved: { $error }
tools-activity-planning = Planning…
tools-activity-syncing = Syncing…
tools-activity-recording = Recording state…
# Job name in the jobs panel; $name: profile name.
tools-sched-job-label = Sync profile “{ $name }”
tools-sched-toast-failed = Sync “{ $name }” failed
tools-sched-toast-conflicts = { $count ->
    [one] “{ $name }”: { $count } conflict to resolve
   *[other] “{ $name }”: { $count } conflicts to resolve
}
tools-sched-toast-conflicts-body = Changed on both sides, so they were skipped. Open the profile to choose.
tools-sched-open-profiles = Open sync profiles
tools-sched-toast-dry-run = Dry run of “{ $name }”
tools-sched-toast-in-sync = “{ $name }” is in sync
# $side: "left" | "right".
tools-sched-err-unavailable = the { $side ->
    [left] left
   *[other] right
    } folder { $path } is not available
tools-sched-watch-failed = Can't watch sync folders
tools-next-scheduler-off = Scheduler off (Settings › Jobs)
tools-next-manual = Runs when you start it
tools-next-on-change = Next: when a file changes
tools-next-ran = Ran this session
tools-next-now = Next: now
tools-next-minute = Next: in a minute
tools-next-in-minutes = Next: in { $count } min
tools-next-today = Next: today { $time }
tools-next-tomorrow = Next: tomorrow { $time }
# $month: abbreviated month name; $day: day of month.
tools-next-date = Next: { $month } { $day }

## Sync profiles window

tools-profiles-title = Sync profiles
tools-profiles-count = { $count ->
    [one] { $count } profile
   *[other] { $count } profiles
}
tools-profiles-scheduler-on = Scheduled profiles run while Soyuz is open; each run is a normal job in the activity panel.
tools-profiles-scheduler-off = The scheduler is off (Settings › Jobs › Sync & versions); profiles only run when you start them.
tools-profiles-empty = No saved sync profiles
tools-profiles-empty-body = Open Compare & sync, pick two folders and save them under Profiles. Schedules and reports show up here.
tools-profiles-expand = Schedule & reports
tools-profiles-run-now = Run now
tools-profiles-dry-run-tip = Write a report of what would change, without changing anything
tools-profiles-open-tip = Compare the folders and choose actions (and resolve conflicts) by hand
# $mode / $method: sync mode and compare method names.
tools-profiles-card-paths = { $left }  ↔  { $right }  ·  { $mode } · { $method }
tools-profiles-card-paths-tip = { $left }
    ↔ { $right }
# $when: relative time; $trigger: tools-trigger-*; $summary: report summary.
tools-profiles-last-run = { $when } ({ $trigger }) · { $summary }
tools-profiles-never-run = Never run
tools-profiles-reports = Run reports
tools-profiles-no-runs = No runs yet. Use Dry run to see what would change.
tools-profiles-report-row = { $status } · { $trigger }
tools-sched-label = Schedule
tools-sched-kind-interval = Every…
tools-sched-kind-daily = Daily
tools-sched-kind-change = On change
tools-sched-kind-start = At start
tools-sched-every = Every
tools-sched-at = At
tools-unit-min = min
tools-unit-hour = h
tools-sched-daily-note = A run missed while Soyuz was closed is caught up at the next start.
tools-sched-change-note = Runs a few seconds after files change in either folder (Settings › Jobs › change delay).
tools-report-would-change = Would change: { $summary }
tools-report-changed = Changed: { $summary }
tools-report-nothing-to-change = Nothing to change.
tools-report-more = … and { $count } more

## Previous versions

tools-versions-title = Previous versions
tools-versions-off = Previous versions are turned off (Settings › Jobs › Sync & versions).
tools-versions-intro = Earlier versions from shadow copies (snapshots Windows takes for restore points and backups).
tools-versions-look-again = Look again
tools-versions-looking = Looking for shadow copies…
tools-versions-list-failed = Can't list previous versions
tools-versions-unsupported = Not available here
tools-versions-elevate = Open Soyuz as administrator
tools-versions-elevate-tip = Starts another Soyuz window with administrator rights (Windows asks first)
tools-versions-denied-more = { $count ->
    [one] { $count } more shadow copy could not be read without administrator rights.
   *[other] { $count } more shadow copies could not be read without administrator rights.
}
tools-versions-copy-to = Copy to…
tools-versions-restore = Restore
tools-versions-restore-tip = Copy this version back to the original location (asks before replacing; undoable)
tools-versions-current = Current
tools-versions-this = This version
tools-versions-deleted = deleted
# $date: formatted date.
tools-versions-modified = Modified { $date }
tools-versions-comparing = Comparing contents…
tools-versions-same = Same contents as the current file.
# $note: tools-versions-size-diff / -different-bytes, or empty.
tools-versions-differ = Contents differ. { $note }
tools-versions-admin-needed = Administrator rights needed
# $kind: "file" | "folder".
tools-versions-admin-needed-body = This volume has { $count ->
    [one] { $count } shadow copy
   *[other] { $count } shadow copies
    }, but Windows only lets administrators read them. Start Soyuz as administrator to see earlier versions of this { $kind ->
    [folder] folder
   *[other] file
    }.
tools-versions-none = No earlier versions
tools-versions-none-body = None of the { $count ->
    [one] { $count } shadow copy
   *[other] { $count } shadow copies
    } of this volume holds a different version of this { $kind ->
    [folder] folder
   *[other] file
    }.
tools-versions-no-copies = No shadow copies on this volume
tools-versions-no-copies-body = Windows creates shadow copies for restore points, Windows Backup and File History (System Protection must be on for the drive). None exist yet.
tools-versions-shadow-copy = Shadow copy
# $provider: snapshot kind ("Shadow copy").
tools-versions-snapshot-time = { $provider }  ·  { $date }
tools-versions-snapshot-seq = { $provider } #{ $seq }
# $modified: formatted date (may be empty).
tools-versions-detail-folder = Folder  ·  modified { $modified }
tools-versions-detail-file = { $size }  ·  modified { $modified }
tools-versions-unreadable = One of the files can't be read.
# $dir: "larger" | "smaller".
tools-versions-size-diff = The current file is { $size } { $dir ->
    [larger] larger
   *[other] smaller
    }.
tools-versions-too-large = Same size; too large to compare quickly.
tools-versions-different-bytes = Same size, different bytes.
tools-versions-job-label = Restore previous version of “{ $name }”
tools-versions-elevate-failed = Couldn't start Soyuz as administrator
tools-versions-open-failed = Couldn't open that version
