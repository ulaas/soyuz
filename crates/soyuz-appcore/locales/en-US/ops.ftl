### Soyuz UI strings (en-US source): ops
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/ops.ftl.

## Navigation, launching and app commands (ops.rs)

ops-bad-address = Can't open this address
# $name: file name.
ops-open-failed = Couldn't open { $name }
ops-launcher-failed = Couldn't start the launcher
ops-folder-gone = Folder no longer exists
# $name: folder title.
ops-folder-gone-body = { $name } was removed or unmounted.
# $action: action title.
ops-coming-soon = { $action } is coming soon
ops-coming-soon-body = File operations arrive with the jobs engine integration.
# $app: application name.
ops-open-with-failed = Couldn't open with { $app }
ops-copied-paths = Copied { $count } paths
ops-copied-to-clipboard = Copied to clipboard
ops-reveal-failed = Couldn't reveal in the system file manager
ops-terminal-failed = Couldn't open a terminal
# $name: folder name.
ops-favorite-added = Added “{ $name }” to Favorites
ops-view-applied = View applied to all folders

## Job titles (cards, toasts, history). $state: "active" (in progress) | "done".
## $what: “name” for one item, otherwise "1,204 items". $dest: folder path.

ops-job-copy-to = { $state ->
    [active] Copying { $what } to { $dest }
   *[done] Copied { $what } to { $dest }
}
ops-job-move-to = { $state ->
    [active] Moving { $what } to { $dest }
   *[done] Moved { $what } to { $dest }
}
# $dest: the new name, quoted.
ops-job-rename-to = { $state ->
    [active] Renaming { $what } to { $dest }
   *[done] Renamed { $what } to { $dest }
}
ops-job-copy = { $state ->
    [active] Copying { $what }
   *[done] Copied { $what }
}
ops-job-move = { $state ->
    [active] Moving { $what }
   *[done] Moved { $what }
}
ops-job-rename = { $state ->
    [active] Renaming { $what }
   *[done] Renamed { $what }
}
ops-job-duplicate = { $state ->
    [active] Duplicating { $what }
   *[done] Duplicated { $what }
}
ops-job-trash = { $state ->
    [active] Moving { $what } to the trash
   *[done] Moved { $what } to the trash
}
ops-job-delete = { $state ->
    [active] Deleting { $what }
   *[done] Deleted { $what }
}
ops-job-wipe = { $state ->
    [active] Wiping { $what }
   *[done] Wiped { $what }
}
ops-job-set-meta = { $state ->
    [active] Updating properties of { $what }
   *[done] Updated properties of { $what }
}
ops-job-restore = { $state ->
    [active] Restoring { $what }
   *[done] Restored { $what }
}
ops-job-empty-trash = { $state ->
    [active] Emptying trash
   *[done] Emptied trash
}
# Media tools (Phase V): $what is the source, $dest the output file name.
ops-job-media-convert = { $state ->
    [active] Converting { $what } to { $dest }
   *[done] Converted { $what } to { $dest }
}
ops-job-media-extract = { $state ->
    [active] Extracting audio from { $what } to { $dest }
   *[done] Extracted audio from { $what } to { $dest }
}
ops-job-media-trim = { $state ->
    [active] Trimming { $what } to { $dest }
   *[done] Trimmed { $what } to { $dest }
}
ops-job-media-frame = { $state ->
    [active] Saving a frame of { $what } to { $dest }
   *[done] Saved a frame of { $what } to { $dest }
}
# $has_name: "yes" | "no"; $name: the new item's name, quoted (empty when unknown).
ops-job-new-folder = { $state ->
    [active] { $has_name ->
        [yes] Creating folder { $name }
       *[no] Creating folder
    }
   *[done] { $has_name ->
        [yes] Created folder { $name }
       *[no] Created folder
    }
}
ops-job-new-file = { $state ->
    [active] { $has_name ->
        [yes] Creating file { $name }
       *[no] Creating file
    }
   *[done] { $has_name ->
        [yes] Created file { $name }
       *[no] Created file
    }
}
ops-job-link = { $state ->
    [active] { $has_name ->
        [yes] Creating link { $name }
       *[no] Creating link
    }
   *[done] { $has_name ->
        [yes] Created link { $name }
       *[no] Created link
    }
}
# $label: the engine's description of the original operation (English, first letter lowercased).
ops-job-undo = { $state ->
    [active] Undoing: { $label }
   *[done] Undone: { $label }
}
ops-job-redo = { $state ->
    [active] Redoing: { $label }
   *[done] Redone: { $label }
}
ops-job-retry = { $state ->
    [active] Retrying: { $label }
   *[done] Retried: { $label }
}
ops-job-other = { $state ->
    [active] Working on { $label }
   *[done] Finished { $label }
}

## Job outcomes. Summary parts are joined with " · ".

# $error: engine error text.
ops-outcome-failed = Failed: { $error }
ops-outcome-cancelled = Cancelled
# $time: duration from format_eta ("12s").
ops-summary-done-in = Done in { $time }
ops-summary-skipped = { $count } skipped
ops-summary-failed = { $count } failed
# $what: a job title with its first letter lowercased.
ops-job-failed = Failed: { $what }
ops-details = Details

## Default names. $stem: target name without extension; $ext: ".txt" or empty.

ops-link-default-name = { $stem } - link{ $ext }
ops-default-folder-name = New folder
ops-default-file-name = New file.txt

## Clipboard, paste, undo (fileops.rs)

ops-clip-copied = { $count ->
    [one] Copied { $count } item
   *[other] Copied { $count } items
}
ops-clip-cut = { $count ->
    [one] Cut { $count } item
   *[other] Cut { $count } items
}
ops-clip-paste-hint = Paste with Ctrl+V.
ops-nothing-to-paste = Nothing to paste
ops-nothing-to-paste-body = Copy or cut files first (Ctrl+C / Ctrl+X).
ops-no-other-pane = No other pane
ops-no-other-pane-body = Open the dual pane view (F9) to copy or move between folders.
ops-drop-into-itself = Can't drop a folder into itself
ops-drop-onto-folder = Drop files onto a folder view to copy them there
ops-nothing-to-undo = Nothing to undo in this session
ops-still-undoing = That operation is still being undone
ops-nothing-to-redo = Nothing to redo
ops-not-available = Not available right now

## Drag and drop. $target: the drop target's name (quoted folder name).

ops-dnd-move-here = Move here
ops-dnd-copy-here = Copy here
ops-dnd-link-here = Create link here
ops-dnd-trash = Trash
ops-dnd-badge-trash = Move to { $target }
ops-dnd-badge-copy = Copy to { $target }
ops-dnd-badge-move = Move to { $target }
ops-dnd-badge-link = Link to { $target }
ops-dnd-badge-drop = Drop into { $target }

## Rename validation

ops-rename-required = A name is required.
# $name: the reserved name (CON, NUL, …).
ops-rename-reserved = “{ $name }” is a reserved name.
# $char: the offending character.
ops-rename-invalid-char = Names can't contain “{ $char }”.
ops-rename-trailing-dot = Names can't end with a dot or a space.
ops-rename-too-long = The name is too long ({ $count } characters).
ops-rename-exists = An item named “{ $name }” already exists here.

## Windows and elevation (ops/window.rs)

ops-new-window-failed = Couldn't open a new window
ops-window-keeps-tab = A window keeps at least one tab
ops-elevate-select = Select a program to run as administrator
# $platform: "windows" | "linux" | "other" (ops/window.rs elevate_platform).
ops-elevate-needs = { $platform ->
    [windows] Run as administrator needs an .exe, .msi, .bat or .cmd file
    [linux] Run as administrator needs a program file with execute permission (it runs through pkexec)
   *[other] Run as administrator needs an executable file
}
# $name: program file name.
ops-elevate-failed = Couldn't run { $name } as administrator

## Place names (breadcrumbs, tab and window titles) and timeline buckets

ops-place-home = Home
ops-place-trash = Trash
ops-place-computer = This PC
ops-place-computer-unix = Computer
ops-place-recent = Recent
ops-place-timeline = Timeline
# $name: tag name.
ops-place-tag = Tag: { $name }
ops-place-search = Search
ops-bucket-today = Today
ops-bucket-yesterday = Yesterday
ops-bucket-this-week = This week
ops-bucket-earlier = Earlier

## Linux (platform)

ops-elevate-no-pkexec = Running programs as administrator uses polkit's pkexec, which isn't installed. Install the polkit package (it provides pkexec) and try again.
