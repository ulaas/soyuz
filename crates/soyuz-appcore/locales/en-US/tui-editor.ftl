### Soyuz UI strings (en-US source): tui-editor
### The terminal UI: the editors and "Save changes?" (Phase T4).
### Keys: <area>-<thing>, kebab-case. Keep keys identical in tr-TR/tui-editor.ftl.

# The replace bar's key hint.
tui-editor-replace-keys = Enter: replace · Alt+A: replace all · Alt+C / W / R: case, word, regex
# "Save as" bar.
tui-editor-save-as = Save as
tui-editor-saving = Saving…
tui-editor-save-as-exists = A file with this name already exists
tui-editor-save-as-text-only = Save as works for text documents
# The notice's key (lossy text: edit anyway; mixed line endings: OK; large hex file: allow editing).
tui-editor-notice-edit = Alt+E: edit anyway
tui-editor-notice-ok = Alt+E: OK
tui-editor-notice-allow = Alt+E: allow editing
# The hex editor's go-to bar.
tui-editor-offset = Offset
# After "Replace all".
tui-editor-replaced = { $count ->
    [one] Replaced { $count } match
   *[other] Replaced { $count } matches
}
# "Save changes?" footer.
tui-editor-unsaved-hints = Enter: choose · ←/→: move · Esc: cancel
