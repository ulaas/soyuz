### Soyuz UI strings (en-US source): editors
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/editors.ftl.


## Editor panel, documents and saving

editors-select-file = Select a file to edit
editors-image-format-readonly = This image format can't be written
# $name: file name.
editors-reload-failed = Couldn't reload { $name }
editors-saved = Saved { $name }
# $note: why the backup copy could not be made.
editors-saved-no-backup = No backup copy in the trash: { $note }
editors-save-failed = Couldn't save { $name }
editors-reloaded = Reloaded { $name }
editors-reloaded-body = It changed on disk.
editors-error-is-folder = This is a folder
editors-backup-too-large = too large for a backup copy
editors-backup-no-parent = no parent folder

## Editor tabs and panel buttons

# $path: full path of the document.
editors-tab-unsaved-tip =
    { $path }
    Unsaved changes
editors-tab-reload = Reload from disk
editors-tab-reopen-hex = Reopen in hex editor
editors-tab-reopen-text = Reopen as text
editors-tab-reveal = Show in file list
editors-panel-hide = Hide the editor (documents stay open)
editors-panel-restore = Restore
editors-panel-maximize = Maximize over the file panes
editors-panel-dock-bottom = Dock at the bottom
editors-panel-dock-right = Dock on the right
editors-external-changed = This file changed on disk.
editors-external-reload = Reload
editors-external-keep = Keep my version
editors-external-deleted = This file was deleted or renamed on disk. Saving writes it again.
# $error: the reason (from the system).
editors-open-failed =
    Couldn't open { $name }
    { $error }
editors-empty = No files open
# $key: the shortcut of "Edit in Soyuz", e.g. F4.
editors-empty-hint = Select a file and press { $key } (or right-click → Edit in Soyuz)
editors-reopen-dirty = Save or discard the changes first

## Unsaved changes prompt

editors-unsaved-title = Save changes to { $name }?
editors-unsaved-body = Your changes are lost if you don't save them.
editors-unsaved-quit-title = { $count ->
    [one] { $count } file has unsaved changes
   *[other] { $count } files have unsaved changes
}
editors-unsaved-quit-body = Soyuz is closing. Your changes are lost if you don't save them.
# $names: comma-separated file names.
editors-unsaved-quit-body-list = Soyuz is closing. { $names }. Your changes are lost if you don't save them.
editors-unsaved-save-all = Save all
editors-unsaved-dont-save = Don't save

## Text editor

editors-text-lossy = Not valid UTF-8: invalid bytes are shown as {"�"}. Saving would write them that way.
editors-text-edit-anyway = Edit anyway
editors-text-mixed-eol = Mixed line endings: saving uses the most common one.
editors-tb-save = Save (Ctrl+S)
editors-tb-undo = Undo (Ctrl+Z)
editors-tb-redo = Redo (Ctrl+Y)
editors-tb-find = Find (Ctrl+F)
editors-tb-replace = Replace (Ctrl+H)
editors-tb-goto = Go to line (Ctrl+G)
editors-tb-wrap = Wrap long lines
editors-tb-whitespace = Show whitespace
editors-read-only = Read-only
editors-find-hint = Find
editors-find-match-case = Match case
editors-find-whole-word = Whole word
editors-find-regex = Regular expression
editors-find-invalid-pattern = invalid pattern
editors-find-no-results = No results
# $current: number of the selected match; $total: number of matches; $more: "+" when the count is capped, else empty.
editors-find-position = { $current } of { $total }{ $more }
editors-find-results = { $count ->
    [one] { $count }{ $more } result
   *[other] { $count }{ $more } results
}
editors-find-close = Close (Esc)
editors-find-next = Next (Enter / F3)
editors-find-prev = Previous (Shift+Enter)
editors-replace-hint = Replace
editors-replace-hint-regex = Replace ($1 for groups)
editors-replace-one = Replace
editors-replace-all = Replace all
editors-goto-hint = Line[:column]
# $count: number of lines in the document.
editors-goto-range = 1 – { $count }  ·  Enter to jump
editors-text-status-pos = Ln { $line }, Col { $col }
editors-text-status-selected = { $count } selected
editors-text-status-lines = { $count ->
    [one] { $count } line
   *[other] { $count } lines
}
editors-text-plain = Plain text
editors-text-indent-tabs = Tabs
# $count: indent width in spaces.
editors-text-indent-spaces = Spaces: { $count }
editors-text-eol-tip = Line endings used when saving (click to switch)
editors-text-empty = Empty file

## Hex editor

editors-hex-cancelled = cancelled
editors-hex-invalid = invalid
editors-hex-binary = binary
editors-hex-need-text = Type something to find
editors-hex-need-bytes = Type hex bytes, e.g. DE AD BE EF
# $char: the offending character.
editors-hex-bad-digit = '{ $char }' is not a hex digit
editors-hex-odd-digits = Odd number of hex digits
editors-hex-searching = Searching…
# $offset: hexadecimal offset without the 0x prefix.
editors-hex-found = Found at 0x{ $offset }
editors-hex-not-found = Not found
editors-hex-too-large = Too large for the text editor: opened read-only in hex.
editors-hex-allow-editing = Allow editing
editors-hex-read-failed = Couldn't read the file: { $error }
editors-hex-tb-find = Find bytes or text (Ctrl+F)
editors-hex-tb-goto = Go to offset (Ctrl+G)
editors-hex-tb-read-only = Read-only (no editing)
editors-hex-big-endian = Big endian
editors-hex-little-endian = Little endian
editors-hex-endian-tip = Byte order used by the data inspector
editors-hex-goto-hint = Offset: 0x1F, 1Fh or 31, +16
editors-hex-find-hint-hex = Hex bytes, e.g. DE AD BE EF
editors-hex-find-hint-text = Text (UTF-8)
editors-hex-mode-tip = Search hex bytes or text
# $hex: cursor offset in hexadecimal (no 0x); $offset: the same in decimal; $count: file size in bytes.
editors-hex-status = Offset 0x{ $hex } ({ $offset })  ·  { $count ->
    [one] { $count } byte
   *[other] { $count } bytes
}
editors-hex-status-changed = { $count } changed
editors-hex-overwrite = Overwrite
editors-hex-text-column = Text column
editors-hex-hex-column = Hex column
editors-hex-inspector = Data inspector
editors-hex-order-big = big endian
editors-hex-order-little = little endian

## Image editor

# $width, $height: image size in pixels; $limit: megapixel limit from the settings.
editors-image-too-large = { $width } × { $height } is larger than the { $limit } MP limit ([editor] max_image_mp)
editors-image-cant-write = Soyuz can't write this image format
# $quality: JPEG quality setting (1–100).
editors-image-lossy-warning = JPEG can't be rotated or cropped losslessly here: saving re-encodes it at quality { $quality } and drops metadata.
editors-image-unsupported-warning = This format can't be written: edits can't be saved.
editors-image-fit = Fit to view (0)
editors-image-actual-size = Actual size (1)
editors-image-zoom-out = Zoom out (-)
editors-image-zoom-in = Zoom in (+)
editors-image-rotate-left = Rotate left (Shift+R)
editors-image-rotate-right = Rotate right (R)
editors-image-flip-h = Flip horizontally (H)
editors-image-flip-v = Flip vertically (V)
editors-image-crop = Crop: drag a rectangle, Enter applies (C)
editors-image-apply-crop = Apply crop
editors-image-kind = Image
# $format: image format (PNG, JPEG…); $zoom: zoom level in percent.
editors-image-status = { $width } × { $height } px  ·  { $format }  ·  { $zoom } %
editors-image-status-crop = crop { $width } × { $height }
editors-image-status-edits = { $count ->
    [one] { $count } edit
   *[other] { $count } edits
}
editors-image-mode-lossless = Saves losslessly
editors-image-mode-lossy = Saves re-encoded (lossy)
editors-image-mode-readonly = Read-only format
editors-image-crop-hint = Drag to select the area to keep
