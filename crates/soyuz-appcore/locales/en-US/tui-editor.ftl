### Soyuz UI strings (en-US source): tui-editor
### The terminal UI: the editors and "Save changes?" (Phase T4).
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/tui-editor.ftl.

# The replace bar's key hint ($all, $case, $word, $regex: the commands' chords).
tui-editor-replace-keys = Enter: replace · { $all }: replace all · { $case } / { $word } / { $regex }: case, word, regex
# "Save as" bar.
tui-editor-save-as = Save as
tui-editor-saving = Saving…
tui-editor-save-as-exists = A file with this name already exists
tui-editor-save-as-text-only = Save as works for text and image documents
tui-editor-save-as-format = Soyuz cannot write this image format (use .png, .jpg, .bmp, .tif, .webp, .gif or .ico)
# The notice's key ($key: its chord; lossy text: edit anyway; mixed line endings: OK; large hex file: allow editing).
tui-editor-notice-edit = { $key }: edit anyway
tui-editor-notice-ok = { $key }: OK
tui-editor-notice-allow = { $key }: allow editing
# The hex editor's go-to bar.
tui-editor-offset = Offset
# After "Replace all".
tui-editor-replaced = { $count ->
    [one] Replaced { $count } match
   *[other] Replaced { $count } matches
}
# "Save changes?" footer.
tui-editor-unsaved-hints = Enter: choose · ←/→: move · Esc: cancel
# The image editor: its keys ($resize, $save: the commands' chords), the status line,
# the crop and resize bars.
tui-editor-image-keys = R / Shift+R: rotate · H / V: flip · C: crop · { $resize }: resize · Ctrl+Z: undo · { $save }: save
tui-editor-image-status = { $width } × { $height } px  ·  { $format }
tui-editor-image-off = { $width } × { $height } px (images are off: [tui] images)
tui-editor-crop-x = X
tui-editor-crop-y = Y
tui-editor-width = Width
tui-editor-height = Height
tui-editor-crop-keys = Enter: crop · Tab: next field · Esc: cancel
tui-editor-resize-keys = Enter: resize · an empty field keeps the aspect ratio · 50% scales
tui-editor-resize-invalid = Enter sizes in pixels (1 to 65535) or a percentage
tui-editor-resize-too-large = Larger than [editor] max_image_mp ({ $limit } megapixels)
tui-editor-crop-invalid = The area is outside the image or empty
tui-editor-resize-image-only = Resize works for images
