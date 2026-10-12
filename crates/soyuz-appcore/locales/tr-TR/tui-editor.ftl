### Soyuz UI strings (tr-TR): tui-editor
### The terminal UI: the editors and "Save changes?" (Phase T4).
### Keys: <area>-<thing>, kebab-case. Keep keys identical in en-US/tui-editor.ftl.

# The replace bar's key hint ($all, $case, $word, $regex: the commands' chords).
tui-editor-replace-keys = Enter: değiştir · { $all }: tümünü değiştir · { $case } / { $word } / { $regex }: büyük-küçük harf, sözcük, regex
# "Save as" bar.
tui-editor-save-as = Farklı kaydet
tui-editor-saving = Kaydediliyor…
tui-editor-save-as-exists = Bu adda bir dosya zaten var
tui-editor-save-as-text-only = Farklı kaydet metin ve görüntü belgelerinde çalışır
tui-editor-save-as-format = Soyuz bu görüntü biçimini yazamaz (.png, .jpg, .bmp, .tif, .webp, .gif veya .ico kullanın)
# The notice's key ($key: its chord; lossy text: edit anyway; mixed line endings: OK; large hex file: allow editing).
tui-editor-notice-edit = { $key }: yine de düzenle
tui-editor-notice-ok = { $key }: Tamam
tui-editor-notice-allow = { $key }: düzenlemeye izin ver
# The hex editor's go-to bar.
tui-editor-offset = Konum
# After "Replace all".
tui-editor-replaced = { $count ->
    [one] { $count } eşleşme değiştirildi
   *[other] { $count } eşleşme değiştirildi
}
# "Save changes?" footer.
tui-editor-unsaved-hints = Enter: seç · ←/→: gezin · Esc: iptal
# The image editor: its keys ($resize, $save: the commands' chords), the status line,
# the crop and resize bars.
tui-editor-image-keys = R / Shift+R: döndür · H / V: çevir · C: kırp · { $resize }: yeniden boyutlandır · Ctrl+Z: geri al · { $save }: kaydet
tui-editor-image-status = { $width } × { $height } px  ·  { $format }
tui-editor-image-off = { $width } × { $height } px (görüntüler kapalı: [tui] images)
tui-editor-crop-x = X
tui-editor-crop-y = Y
tui-editor-width = Genişlik
tui-editor-height = Yükseklik
tui-editor-crop-keys = Enter: kırp · Tab: sonraki alan · Esc: iptal
tui-editor-resize-keys = Enter: yeniden boyutlandır · boş alan en-boy oranını korur · %50 ölçekler
tui-editor-resize-invalid = Boyutları piksel (1 ile 65535 arası) ya da yüzde olarak girin
tui-editor-resize-too-large = [editor] max_image_mp sınırından büyük ({ $limit } megapiksel)
tui-editor-crop-invalid = Alan görüntünün dışında ya da boş
tui-editor-resize-image-only = Yeniden boyutlandırma görüntülerde çalışır
