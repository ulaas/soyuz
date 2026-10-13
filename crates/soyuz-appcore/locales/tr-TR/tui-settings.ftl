### Soyuz UI strings (tr-TR): tui-settings
### Terminal arayüzü: ayar düzenleyici ve tuş atamaları sayfası (Aşama T6).
### Keys: <area>-<thing>, kebab-case. Keep keys identical in en-US/tui-settings.ftl.

## The editor
tui-settings-title = Ayarlar
tui-settings-results = “{ $query }” için sonuçlar
tui-settings-saving = Kaydediliyor…
tui-settings-saved = Kaydedildi
tui-settings-search-hint = Tüm ayarlarda aramak için yazın
tui-settings-filter-hint = Kısayolları süzmek için yazın
tui-settings-no-results = Eşleşen bir şey yok.
tui-settings-hint-rows = ↑↓ git · Enter düzenle · ←→ değiştir · Del sıfırla · / ara · Tab bölümler · Ctrl+R bölümü sıfırla · Ctrl+E dosyayı düzenle · F1 yardım · Esc kapat
tui-settings-hint-nav = ↑↓ bölüm · Enter/Tab ayarlar · Ctrl+E dosyayı düzenle · Esc kapat
tui-settings-hint-search = Enter/↓ sonuçlara geç · Esc temizle
tui-settings-hint-edit = Enter kaydet · Esc vazgeç
tui-settings-hint-keys = Enter kısayol ekle · ←→ seç · Del kaldır · Backspace hazır ayar · Alt+C çakışmalar · Alt+M değişenler · Esc kapat
tui-settings-choose-hint = ↑↓ seç · Enter uygula · Esc vazgeç

## Values and descriptions
tui-settings-unset = (ayarlanmamış)
tui-settings-empty = (boş)
tui-settings-table-summary = { $count } öğe — düzenlemek için Enter
tui-settings-table-hint = Enter kayıtların listesini açar
tui-settings-opaque-summary = { $count } kayıt
tui-settings-opaque-hint = Bunu config.toml içinde düzenleyin (Ctrl+E)
tui-settings-list-desc = Enter listeyi düzenler
tui-settings-range = Aralık: { $min } – { $max }
tui-settings-values = Değerler: { $values }
tui-settings-default = Varsayılan: { $value }
tui-settings-restart-note = Yeniden başlatınca geçerli olur

## Edits
tui-settings-invalid = Geçersiz değer: { $error }
tui-settings-not-a-number = “{ $text }” bir sayı değil
tui-settings-restart = Yeniden başlatınca geçerli olur
tui-settings-reset-title = { $section } sıfırlansın mı?
tui-settings-reset-body = Bu sayfadaki her ayar varsayılanına döner. Kendi kayıtlarınız (profiller, kurallar, kayıtlı aramalar, sabitlenmiş klasörler) kalır.
tui-settings-section-reset = { $section } varsayılanlara döndü

## The list editor
tui-settings-list-item = Öğe
tui-settings-list-add = Ekle
tui-settings-list-remove = Kaldır
tui-settings-list-empty = (boş liste)
tui-settings-list-hint = ↑↓ seç · Enter alanı öğeye yaz · Insert yeni öğe · Alt+↑↓ taşı · Alt+S kaydet · Esc vazgeç
tui-settings-list-map-entry = “{ $entry }” ad = değer biçiminde olmalı

## The key bindings page and the shortcut recorder
tui-settings-keys-problems = Kısayollarda { $count } sorun:
tui-settings-keys-more = …ve { $count } tane daha
tui-settings-keys-conflicts-only = Yalnızca çakışmalar
tui-settings-keys-modified-only = Yalnızca değişen kısayollar
tui-settings-keys-filter-desc = Enter ya da Boşluk süzgeci açıp kapatır.
tui-settings-keys-contexts = Nerede: { $contexts }
tui-settings-keys-overlay = Asıl tuşu gönderemeyen terminaller için terminal arayüzünün eklediği: { $keys }
tui-settings-keys-overlay-fixed = { $keys } terminal arayüzü tarafından eklenir, kaldırılamaz
tui-settings-record-title = { $action } için kısayol
tui-settings-record-waiting = Tuşlara basın (dizi için en fazla üç)…
tui-settings-record-keys = Tuşlar: { $keys }
tui-settings-record-used = Zaten kullanan: { $actions }
tui-settings-record-hint = Enter kaydeder · Esc vazgeçer · diğer her tuş kaydedilir
tui-settings-record-clear = Temizle
