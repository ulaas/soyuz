### Soyuz UI strings (en-US source): common
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/common.ftl.

## Shared buttons and words. Area-specific strings belong in their own file.

common-ok = OK
common-cancel = Cancel
common-close = Close
common-apply = Apply
common-save = Save
common-delete = Delete
common-rename = Rename
common-copy = Copy
common-move = Move
common-undo = Undo
common-redo = Redo
common-yes = Yes
common-no = No
common-retry = Retry
common-skip = Skip
common-reset = Reset
common-loading = Loading…

# Count of items; numbers are formatted with the locale's digit grouping.
common-items = { $count ->
    [one] { $count } item
   *[other] { $count } items
}
common-folders = { $count ->
    [one] { $count } folder
   *[other] { $count } folders
}
common-files = { $count ->
    [one] { $count } file
   *[other] { $count } files
}
# $kind: "file" | "folder" | "link" | other.
common-kind = { $kind ->
    [file] File
    [folder] Folder
    [link] Link
   *[other] Item
}

## Dropdown widget
dropdown-search = Search…
dropdown-no-matches = No matches
dropdown-use-typed = Use “{ $text }”
dropdown-custom = Custom…
