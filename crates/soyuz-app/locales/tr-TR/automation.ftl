### Soyuz arayüz metinleri (tr-TR): automation
### Anahtarlar en-US/automation.ftl ile birebir aynı olmalı.

## İş kaynağı rozetleri (etkinlik paneli)

automation-origin-rule = Bir otomasyon kuralı tarafından başlatıldı
automation-origin-script = Bir betik tarafından başlatıldı
automation-origin-cli = soyuz komut satırından başlatıldı

## Betik eylemleri

automation-script-fallback-title = Betik: { $id }
automation-script-action = Betik eylemi
automation-scripts-still-loading = Betikler hâlâ yükleniyor
automation-no-action = “{ $id }” adlı betik eylemi yok
automation-no-action-body = Ayarlar → Betikler bölümünü denetleyin.
automation-start-failed = Betik başlatılamadı
automation-script-failed = “{ $title }” betiği başarısız oldu
automation-script-toast = { $title }: { $text }
automation-unknown-tag = Bilinmeyen etiket rengi: “{ $tag }”
automation-not-running = Otomasyonlar bu pencerede çalışmıyor
automation-reloading = Betikler yeniden yükleniyor
automation-not-loaded = Betikler henüz yüklenmedi
automation-folder-failed = Betikler klasörü oluşturulamadı
automation-open-failed = { $name } açılamadı

## Betik şablonları

automation-template-example = Örnek: eylemler, bir sütun ve bir önizleyici
automation-template-tidy = Kural betiği: uzantıya göre ayır

## Kurallar

automation-secondary-note = Kurallar ikincil pencerelerde değil, ana Soyuz penceresinde çalışır.
automation-another-window = Kuralları başka bir Soyuz penceresi çalıştırıyor.
automation-cli-unavailable = kullanılamıyor: { $error } (başka bir Soyuz penceresi sunuyor olabilir)

## Komut satırından başlatılan işlerin etiketleri

automation-cli-items = { $count ->
    [one] “{ $name }”
   *[other] { $count } öğe
}
automation-cli-copy = { $count ->
    [one] “{ $name }” öğesini “{ $dest }” klasörüne kopyala
   *[other] { $count } öğeyi “{ $dest }” klasörüne kopyala
}
automation-cli-move = { $count ->
    [one] “{ $name }” öğesini “{ $dest }” klasörüne taşı
   *[other] { $count } öğeyi “{ $dest }” klasörüne taşı
}
automation-cli-trash = { $count ->
    [one] “{ $name }” öğesini Çöp Kutusu'na taşı
   *[other] { $count } öğeyi Çöp Kutusu'na taşı
}
automation-cli-delete = { $count ->
    [one] “{ $name }” öğesini sil
   *[other] { $count } öğeyi sil
}

## Betik onay kartı

automation-approval-title = “{ $action }” { $count } değişiklik yapmak istiyor
automation-approval-intro = Bir betik bu değişiklikleri istedi. Henüz hiçbir şey yapılmadı.
automation-and-more = …ve { $count } tane daha
automation-approval-note = Her değişiklik duraklatabileceğiniz, iptal edebileceğiniz ve geri alabileceğiniz bir arka plan işi olarak çalışır. Bu kartı Ayarlar → Betikler bölümünden kapatabilirsiniz.
automation-approval-run = Çalıştır

## Ayarlar → Otomasyonlar

automation-rules-group = Otomasyonlar
automation-rules-run = Kuralları çalıştır
automation-rules-run-desc = Soyuz açıkken izlenen klasörlerdeki yeni öğeler üzerinde işlem yap.
automation-rules-dry-run = Deneme çalıştırması
automation-rules-dry-run-desc = Her kural yalnızca ne yapacağını çalışma günlüğüne yazar.
automation-rules-log-size = Çalışma günlüğü boyutu
automation-rules-log-size-desc = Kural başına saklanan kayıt sayısı.
automation-rules-cli = Terminalden gelen komutları kabul et
automation-rules-cli-desc = soyuz cp | mv | rm | trash | open | reveal | jobs komutları işlerini bu pencereye gönderir (yalnızca geçerli kullanıcı).
automation-rules-new = Yeni kural
automation-rules-default-name = Kural { $n }
automation-rules-start-from = veya şununla başlayın:
automation-rules-preset-pdf = İndirilenler'deki PDF'ler → Belgeler/PDF + mavi etiket
automation-rules-preset-pdf-name = PDF'leri dosyala
automation-rules-preset-installers = Eski yükleyicileri Çöp Kutusu'na taşı
automation-rules-preset-installers-name = Eski yükleyiciler
automation-rules-intro = Her adım bir iş olarak çalışır (etkinlik paneli, geri alma). Kurallar bir öğenin değişmesi bitene kadar bekler ve yarım kalmış indirmelere asla dokunmaz.
automation-rules-name-hint = Kural adı
automation-rules-status-off = Kapalı
automation-rules-status-watching = İzleniyor
automation-rules-status-not-running = Henüz çalışmıyor
automation-rules-delete = Kuralı sil
automation-rules-move-up = Yukarı taşı
automation-rules-move-down = Aşağı taşı
automation-rules-run-now = Şimdi çalıştır
automation-rules-run-now-tip = Şu anda eşleşen her şey üzerinde işlem yap
automation-rules-watch = İzle
automation-rules-subfolders = Alt klasörler
automation-rules-names = Adlar
automation-rules-glob-hint = *.pdf;*.epub  (boş = tümü)
automation-rules-regex-hint = regex (isteğe bağlı)
automation-rules-items = Öğeler
automation-rules-size = Boyut (KiB)
automation-rules-no-limit = 0 = sınır yok
automation-rules-age = Yaş (dakika)
automation-rules-by-mtime = değiştirilme zamanına göre
automation-rules-wait = Bekle
automation-rules-seconds-suffix = {" "}sn
automation-rules-after-change = son değişiklikten sonra
automation-rules-existing = Mevcut öğelere de uygula
automation-rules-conflicts = Çakışmalar
automation-rules-then = Ardından
automation-rules-run-log = ÇALIŞMA GÜNLÜĞÜ
automation-rules-clear = Temizle
automation-rules-matches-now = ŞU ANKİ EŞLEŞMELER
automation-rules-no-matches = Klasörde şu anda eşleşen bir şey yok.
automation-rules-dest-hint = hedef klasör
automation-rules-script-hint = betik adı
automation-rules-script-tip = Betikler klasöründe fn rule(path, ctx) tanımlayan bir betik
automation-rules-remove-step = Adımı kaldır
automation-rules-add-step = Adım ekle
automation-step-move = Taşı
automation-step-copy = Kopyala
automation-step-rename = Yeniden adlandır
automation-step-tag = Etiketle
automation-step-trash = Çöp Kutusu
automation-step-script = Betik

## Etiket renkleri (kural adımları)

automation-tag-red = Kırmızı
automation-tag-orange = Turuncu
automation-tag-yellow = Sarı
automation-tag-green = Yeşil
automation-tag-cyan = Camgöbeği
automation-tag-blue = Mavi
automation-tag-purple = Mor
automation-tag-gray = Gri

## Ayarlar → Betikler

automation-scripts-group = Betik çalıştırma
automation-scripts-run = Betikleri çalıştır
automation-scripts-run-desc = scripts/*.rhai dosyalarını yükle: eylemleri, sütunları, önizleyicileri ve kural adımları.
automation-scripts-approve = Betik değişikliklerini onayla
automation-scripts-approve-desc = Bir eylemin neyi değiştirmek istediğini çalışmadan önce göster. Kapalıysa değişiklikler hemen başlar (yine de geri alınabilir).
automation-scripts-read = Betiklerin dosya okumasına izin ver
automation-scripts-read-desc = list, stat, search ve read_text. Kapalıysa betikler yalnızca kendilerine verilen yolları görür.
automation-scripts-timeout = Süre sınırı
automation-scripts-timeout-desc = Bir eylemin veya kural betiğinin en uzun çalışma süresi.
automation-scripts-cell-timeout = Sütun ve önizleme süre sınırı
automation-scripts-cell-timeout-desc = Betik sütununun her değeri veya her önizleyici çağrısı için.
automation-scripts-ops = İşlem sınırı
automation-scripts-ops-desc = Bir çağrı durdurulmadan önce yapılabilecek betik işlemi sayısı.
automation-scripts-read-max = Okunabilecek en büyük dosya
automation-scripts-read-max-desc = read_text en fazla bu kadarını döndürür.
automation-scripts-list-max = Listelenecek en fazla girdi
automation-scripts-list-max-desc = list() ve search() en fazla bu kadar girdi döndürür.
automation-scripts-show-folder = Betikler klasörünü göster
automation-scripts-reload = Yeniden yükle
automation-scripts-new-from = Şablondan yeni:
automation-scripts-folder-note = Betikler kaydettiğinizde yeniden yüklenir. Korumalı alanda çalışırlar (işlem başlatma, ağ veya doğrudan dosya değişikliği yok): istedikleri her değişiklik geri alınabilir bir iş olur. Eylemleri Klavye bölümünde “run_script:<file>/<id>” olarak atayın.
automation-scripts-loading = Betikler yükleniyor…
automation-scripts-empty = Henüz betik yok. Yukarıdaki bir şablonla başlayın ya da betikler klasörüne .rhai dosyaları koyun.
automation-scripts-summary = { $scripts } betik · { $actions } eylem · { $columns } sütun · { $previewers } önizleyici
automation-scripts-columns-capped = Yalnızca ilk 4 sütun gösterilir.
automation-scripts-last-run = Son çalıştırma: { $error }
automation-scripts-context-menu = bağlam menüsü
automation-scripts-column = Sütun: “{ $title }”
automation-scripts-previewer = “{ $title }” önizleyicisi (.{ $exts })
automation-scripts-rule-step = Kural adımı: fn rule(path, ctx)
