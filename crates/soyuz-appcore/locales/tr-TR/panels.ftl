### Soyuz arayüz metinleri (tr-TR): panels
### Anahtarlar en-US/panels.ftl ile birebir aynı olmalı.

## Alt panel (ui/panels/mod.rs)

panels-tab-terminal = Terminal
panels-tab-activity = Etkinlik
panels-close-panel = Paneli kapat
panels-close-activity = Etkinliği kapat

## Etkinlik paneli: araç çubuğu, durum çubuğu göstergesi, boş durum

panels-tab-jobs = İşler
panels-tab-jobs-pending = İşler · { $count }
panels-tab-history = Geçmiş
panels-clear-finished = Bitenleri temizle
panels-cancel-all = Tümünü iptal et
panels-resume-all = Tümünü sürdür
panels-pause-all = Tümünü duraklat
panels-needs-input = Yanıt bekleniyor
panels-jobs-count = { $count } iş
panels-show-activity-tip = Etkinliği göster (Ctrl+Shift+J)
panels-jobs-empty = İşlem yok
panels-jobs-empty-hint = Kopyalama, taşıma ve silme işlemlerinin ilerlemesi burada görünür.

## İş kartları

panels-state-queued-at = Kuyrukta · sıra: { $position }
panels-state-queued = Kuyrukta
panels-state-planning = Öğeler sayılıyor…
panels-state-working = Çalışıyor…
panels-state-paused = Duraklatıldı
panels-state-waiting = Yanıtınız bekleniyor
panels-state-finishing = Tamamlanıyor…
panels-verified = Doğrulandı
panels-verifying = Doğrulanıyor
panels-verified-tip = Kopyalanan her dosya geri okundu ve BLAKE3 sağlama toplamı kaynakla karşılaştırıldı
panels-progress-of = { $done } / { $total }
panels-items-of = { $done } / { $total } öğe
panels-speed-eta = { $speed } · { $eta } kaldı
panels-speed-avg = ort. { $speed }
panels-hide-details = Ayrıntıları gizle
panels-show-log = Günlüğü ve hataları göster
panels-more = Diğer
panels-priority = Öncelik
panels-priority-high = Yüksek
panels-priority-normal = Normal
panels-priority-low = Düşük
panels-move-up = Yukarı taşı
panels-move-down = Aşağı taşı
panels-start-now = Hemen başlat
panels-resume = Sürdür
panels-pause = Duraklat
panels-resolve = Çöz…
panels-resolve-conflict = Çakışmayı çöz
panels-remove-from-list = Listeden kaldır
panels-retry-failed = Yeniden dene
panels-retry-failed-items = Başarısız öğeleri yeniden dene
panels-log-empty = Henüz günlük kaydı yok.

## Geçmiş sekmesi

panels-history-empty = Henüz geçmiş yok
panels-history-empty-hint = Tamamlanan işlemler burada görünür ve geri alınabilir.
panels-history-undone = Geri alındı
panels-history-verified-tip = Kopyalanan her dosyanın sağlama toplamı kaynağıyla karşılaştırıldı (BLAKE3)
panels-history-not-undoable = geri alınamaz
panels-history-not-undoable-tip = Bu işlem geri alınamaz (ör. kalıcı silme).

## Terminal paneli

panels-terminal-default-name = Terminal { $number }
panels-terminal-a11y = Terminal: { $name }
panels-terminal-command-not-run = Komut çalıştırılmadı
panels-terminal-command-not-run-body = Komut satırı veya klasör denetim karakterleri (ör. satır sonu) içeriyor; kabuğa güvenli şekilde yazılamaz.
panels-terminal-clip-title = Terminal panonuza yazmak istiyor:
panels-terminal-clip-empty = (boş)
panels-terminal-clip-warning = Programlar, yapıştırmanız için komut yerleştirebilir. Yalnızca bunu bekliyorsanız izin verin.
panels-terminal-clip-deny = Reddet
panels-terminal-clip-allow-once = Bir kez izin ver
panels-terminal-clip-always = Bu oturumda hep izin ver
panels-terminal-clip-always-tip = Soyuz yeniden başlatılana kadar terminalin panoya yazmasına izin ver
panels-terminal-new = Yeni terminal
panels-terminal-open-here = Terminali burada aç
panels-terminal-open-external = Harici terminali burada aç
panels-terminal-busy = Terminal meşgul — klasör değiştirilmedi
panels-terminal-shell-failed = Kabuk başlatılamadı: { $error }
panels-terminal-unknown-error = bilinmeyen hata
panels-terminal-link-not-found = Bağlantı hedefi bulunamadı
panels-terminal-exe-revealed = Çalıştırılabilir dosya gösterildi, çalıştırılmadı
panels-terminal-exe-revealed-body = Terminal çıktısındaki bağlantılar asla program başlatmaz. Güveniyorsanız dosya listesinden açın.
panels-terminal-reveal-failed = Dosya gösterilemedi
panels-terminal-link-failed = Bağlantı açılamadı
