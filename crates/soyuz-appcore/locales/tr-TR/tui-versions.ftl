### Soyuz UI strings (tr-TR): tui-versions
### Terminal arayüzü: önceki sürümler (Aşama T7).
### Keys: <area>-<thing>, kebab-case. Keep keys identical in en-US/tui-versions.ftl.

# Dialog title; $name: the file or folder name.
tui-versions-title = Önceki sürümler: { $name }
tui-versions-hints = Enter aç · C karşılaştır · T kopyala · R geri yükle · F5 yeniden ara · Esc kapat
tui-versions-hints-folder = Enter aç · T kopyala · R geri yükle · F5 yeniden ara · Esc kapat
# Windows, copies denied: a message only (the terminal UI never restarts itself elevated).
# $kind: "file" | "folder".
tui-versions-admin-hint = Bu birimde yalnızca yöneticilerin okuyabildiği { $count } gölge kopya var. Bu { $kind ->
    [folder] klasörün
   *[other] dosyanın
    } eski sürümlerini görmek için Soyuz'u kapatın, bir terminali “Yönetici olarak çalıştır” ile açın ve orada soyuz --tui komutunu başlatın.
# The Properties dialog's button.
tui-versions-button = Önceki sürümler…
