### Soyuz UI strings (tr-TR): tui-settings-tables
### Terminal arayüzü: ayar düzenleyicide tablo listelerini ve gizli bilgileri düzenleme (Aşama T6).
### Keys: <area>-<thing>, kebab-case. Keep keys identical in en-US/tui-settings-tables.ftl.

## Öğe listesi
tui-tables-title = { $table } düzenle
tui-tables-empty = Henüz kayıt yok: Insert ya da Alt+A ile ekleyin
tui-tables-entry = Kayıt { $n }
tui-tables-duplicate = Çoğalt
tui-tables-hint = Enter düzenle · Insert ekle · Alt+D çoğalt · Delete kaldır · Alt+↑/↓ taşı · Esc kapat
tui-tables-hint-map = Enter düzenle · Insert ekle · Alt+D çoğalt · Delete kaldır · Esc kapat
tui-tables-remove-confirm = “{ $name }” kaldırılsın mı? Ayarları config.toml dosyasından silinir.
tui-tables-key = Ad
tui-tables-key-body = Bu tablodaki her kaydın benzersiz bir adı vardır.
tui-tables-key-hint = Enter ekle · Esc vazgeç
tui-tables-key-needed = Bir ad yazın
tui-tables-name-taken = “{ $name }” bu listede zaten kullanılıyor
tui-tables-editor-after = Bu listedeki değişiklikler şimdi uygulanıyor; config.toml dosyasını düzenlemek için Ctrl+E tuşuna yeniden basın

## Bir öğenin formu
tui-tables-row-title = { $table } · { $name }
tui-tables-row-hint = ↑/↓ alan · Enter uygula ya da düzenle · ←/→ değiştir · Alt+R sıfırla · Esc geri
tui-tables-opaque = { "$" }EDITOR ile düzenleyin (Ctrl+E)
tui-tables-chord = Enter kısayolu kaydeder; Alt+R kaldırır
tui-tables-secret-ref = Yalnızca bir başvuru: credential:<ad> ya da env:<DEĞİŞKEN>. Alt+S gizli bilgiyi işletim sisteminin kimlik bilgisi deposuna kaydeder.
tui-tables-set-secret = Gizli bilgiyi ayarla…

## Gizli bilgiyi ayarla…
tui-tables-secret-title = { $name } için gizli bilgi
tui-tables-secret-field = Gizli bilgi
tui-tables-secret-body = İşletim sisteminin kimlik bilgisi deposunda “{ $name }” adıyla saklanır. config.toml gizli bilgiyi asla tutmaz, yalnızca credential:{ $name } başvurusunu tutar.
tui-tables-secret-hint = Enter kaydet · Alt+R kayıtlı gizli bilgiyi kaldır · Esc vazgeç
tui-tables-secret-remove = Kaldır
tui-tables-secret-empty = Önce gizli bilgiyi yazın
tui-tables-secret-stored = Gizli bilgi kimlik bilgisi deposuna kaydedildi
tui-tables-secret-removed = Gizli bilgi kimlik bilgisi deposundan kaldırıldı
tui-tables-secret-failed = Kimlik bilgisi deposu: { $error }
tui-tables-secret-hint-mcp = Enter kaydet · Alt+G belirteç üret · Alt+R kayıtlı belirteci kaldır · Esc vazgeç
tui-tables-token-hint = Alt+T belirteci kopyala · Alt+C istemci yapılandırmasını kopyala · Esc kapat
