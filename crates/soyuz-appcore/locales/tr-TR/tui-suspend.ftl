### Soyuz UI strings (tr-TR): tui-suspend
### Terminal arayüzü: bir kabuğa ya da metin düzenleyiciye geçiş (Aşama T5 / T6).
### Keys: <area>-<thing>, kebab-case. Keep keys identical in en-US/tui-suspend.ftl.

# Printed in the terminal when a shell starts in place of the terminal UI.
tui-suspend-banner = Soyuz: dosya listesine dönmek için `exit` yazın.
# Printed before a command line runs in place of the terminal UI ($command: the line).
tui-suspend-running = Soyuz: { $command } çalıştırılıyor
# Printed after that command ended.
tui-suspend-press-key = Soyuz'a dönmek için Enter'a basın…
# No shell configured ([terminal] shell) and none found.
tui-suspend-no-shell = Kabuk bulunamadı: ayarlardan [terminal] shell değerini belirleyin
# No console text editor found ($names: the commands tried, comma-separated).
tui-suspend-no-editor = Metin düzenleyici bulunamadı
tui-suspend-no-editor-tried = Denenenler: { $names }. [tui] editor ayarını ya da VISUAL veya EDITOR ortam değişkenini belirleyin.
# A program could not be started ($program: its file name; the body is the system error).
tui-suspend-failed = { $program } başlatılamadı
# A command or an editor ended with an error ($program: its file name, $code: the exit code).
tui-suspend-exit = { $program } { $code } çıkış koduyla sona erdi
