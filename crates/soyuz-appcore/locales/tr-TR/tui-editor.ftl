### Soyuz UI strings (tr-TR): tui-editor
### The terminal UI: the editors and "Save changes?" (Phase T4).
### Keys: <area>-<thing>, kebab-case. Keep keys identical in en-US/tui-editor.ftl.

# The replace bar's key hint.
tui-editor-replace-keys = Enter: değiştir · Alt+A: tümünü değiştir · Alt+C / W / R: büyük-küçük harf, sözcük, regex
# "Save as" bar.
tui-editor-save-as = Farklı kaydet
tui-editor-saving = Kaydediliyor…
tui-editor-save-as-exists = Bu adda bir dosya zaten var
tui-editor-save-as-text-only = Farklı kaydet yalnızca metin belgelerinde çalışır
# The notice's key (lossy text: edit anyway; mixed line endings: OK; large hex file: allow editing).
tui-editor-notice-edit = Alt+E: yine de düzenle
tui-editor-notice-ok = Alt+E: Tamam
tui-editor-notice-allow = Alt+E: düzenlemeye izin ver
# The hex editor's go-to bar.
tui-editor-offset = Konum
# After "Replace all".
tui-editor-replaced = { $count ->
    [one] { $count } eşleşme değiştirildi
   *[other] { $count } eşleşme değiştirildi
}
# "Save changes?" footer.
tui-editor-unsaved-hints = Enter: seç · ←/→: gezin · Esc: iptal
