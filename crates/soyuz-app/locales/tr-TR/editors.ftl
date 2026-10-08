### Soyuz arayüz metinleri (tr-TR): editors
### Anahtarlar en-US/editors.ftl ile birebir aynı olmalı.


## Editor panel, documents and saving

editors-select-file = Düzenlemek için bir dosya seçin
editors-image-format-readonly = Bu görüntü biçimi yazılamıyor
editors-reload-failed = { $name } yeniden yüklenemedi
editors-saved = { $name } kaydedildi
editors-saved-no-backup = Çöp Kutusu'nda yedek kopya yok: { $note }
editors-save-failed = { $name } kaydedilemedi
editors-reloaded = { $name } yeniden yüklendi
editors-reloaded-body = Dosya diskte değişti.
editors-error-is-folder = Bu bir klasör
editors-backup-too-large = yedek kopya için çok büyük
editors-backup-no-parent = üst klasör yok

## Editor tabs and panel buttons

editors-tab-unsaved-tip =
    { $path }
    Kaydedilmemiş değişiklikler
editors-tab-reload = Diskten yeniden yükle
editors-tab-reopen-hex = Onaltılık düzenleyicide yeniden aç
editors-tab-reopen-text = Metin olarak yeniden aç
editors-tab-reveal = Dosya listesinde göster
editors-panel-hide = Düzenleyiciyi gizle (belgeler açık kalır)
editors-panel-restore = Geri yükle
editors-panel-maximize = Dosya bölmelerini kaplayacak şekilde büyüt
editors-panel-dock-bottom = Alta yerleştir
editors-panel-dock-right = Sağa yerleştir
editors-external-changed = Bu dosya diskte değişti.
editors-external-reload = Yeniden yükle
editors-external-keep = Benim sürümümü koru
editors-external-deleted = Bu dosya diskte silindi veya yeniden adlandırıldı. Kaydederseniz yeniden yazılır.
editors-open-failed =
    { $name } açılamadı
    { $error }
editors-empty = Açık dosya yok
editors-empty-hint = Bir dosya seçip { $key } tuşuna basın (veya sağ tıklayıp Soyuz'da düzenle'yi seçin)
editors-reopen-dirty = Önce değişiklikleri kaydedin veya atın

## Unsaved changes prompt

editors-unsaved-title = { $name } dosyasındaki değişiklikler kaydedilsin mi?
editors-unsaved-body = Kaydetmezseniz değişiklikleriniz kaybolur.
editors-unsaved-quit-title = { $count } dosyada kaydedilmemiş değişiklik var
editors-unsaved-quit-body = Soyuz kapanıyor. Kaydetmezseniz değişiklikleriniz kaybolur.
editors-unsaved-quit-body-list = Soyuz kapanıyor. { $names }. Kaydetmezseniz değişiklikleriniz kaybolur.
editors-unsaved-save-all = Tümünü kaydet
editors-unsaved-dont-save = Kaydetme

## Text editor

editors-text-lossy = Geçerli UTF-8 değil: geçersiz baytlar {"�"} olarak gösteriliyor. Kaydederseniz bu şekilde yazılır.
editors-text-edit-anyway = Yine de düzenle
editors-text-mixed-eol = Karışık satır sonları: kaydederken en sık kullanılan seçilir.
editors-tb-save = Kaydet (Ctrl+S)
editors-tb-undo = Geri al (Ctrl+Z)
editors-tb-redo = Yinele (Ctrl+Y)
editors-tb-find = Bul (Ctrl+F)
editors-tb-replace = Değiştir (Ctrl+H)
editors-tb-goto = Satıra git (Ctrl+G)
editors-tb-wrap = Uzun satırları kaydır
editors-tb-whitespace = Boşlukları göster
editors-read-only = Salt okunur
editors-find-hint = Bul
editors-find-match-case = Büyük/küçük harf duyarlı
editors-find-whole-word = Tam sözcük
editors-find-regex = Normal ifade
editors-find-invalid-pattern = geçersiz desen
editors-find-no-results = Sonuç yok
editors-find-position = { $current } / { $total }{ $more }
editors-find-results = { $count }{ $more } sonuç
editors-find-close = Kapat (Esc)
editors-find-next = Sonraki (Enter / F3)
editors-find-prev = Önceki (Shift+Enter)
editors-replace-hint = Değiştir
editors-replace-hint-regex = Değiştir (gruplar için $1)
editors-replace-one = Değiştir
editors-replace-all = Tümünü değiştir
editors-goto-hint = Satır[:sütun]
editors-goto-range = 1 – { $count }  ·  Gitmek için Enter
editors-text-status-pos = Sat { $line }, Süt { $col }
editors-text-status-selected = { $count } seçili
editors-text-status-lines = { $count } satır
editors-text-plain = Düz metin
editors-text-indent-tabs = Sekmeler
editors-text-indent-spaces = Boşluk: { $count }
editors-text-eol-tip = Kaydederken kullanılan satır sonları (değiştirmek için tıklayın)
editors-text-empty = Boş dosya

## Hex editor

editors-hex-cancelled = iptal edildi
editors-hex-invalid = geçersiz
editors-hex-binary = ikili
editors-hex-need-text = Aranacak bir şey yazın
editors-hex-need-bytes = Onaltılık baytları yazın, ör. DE AD BE EF
editors-hex-bad-digit = '{ $char }' onaltılık bir basamak değil
editors-hex-odd-digits = Onaltılık basamak sayısı tek
editors-hex-searching = Aranıyor…
editors-hex-found = 0x{ $offset } konumunda bulundu
editors-hex-not-found = Bulunamadı
editors-hex-too-large = Metin düzenleyici için çok büyük: onaltılık görünümde salt okunur açıldı.
editors-hex-allow-editing = Düzenlemeye izin ver
editors-hex-read-failed = Dosya okunamadı: { $error }
editors-hex-tb-find = Bayt veya metin bul (Ctrl+F)
editors-hex-tb-goto = Konuma git (Ctrl+G)
editors-hex-tb-read-only = Salt okunur (düzenleme yok)
editors-hex-big-endian = Big-endian
editors-hex-little-endian = Little-endian
editors-hex-endian-tip = Veri denetçisinin kullandığı bayt sırası
editors-hex-goto-hint = Konum: 0x1F, 1Fh veya 31, +16
editors-hex-find-hint-hex = Onaltılık baytlar, ör. DE AD BE EF
editors-hex-find-hint-text = Metin (UTF-8)
editors-hex-mode-tip = Onaltılık bayt veya metin ara
editors-hex-status = Konum 0x{ $hex } ({ $offset })  ·  { $count } bayt
editors-hex-status-changed = { $count } değişiklik
editors-hex-overwrite = Üzerine yazma
editors-hex-text-column = Metin sütunu
editors-hex-hex-column = Onaltılık sütun
editors-hex-inspector = Veri denetçisi
editors-hex-order-big = big-endian
editors-hex-order-little = little-endian

## Image editor

editors-image-too-large = { $width } × { $height }, { $limit } MP sınırından büyük ([editor] max_image_mp)
editors-image-cant-write = Soyuz bu görüntü biçimini yazamıyor
editors-image-lossy-warning = JPEG burada kayıpsız döndürülemez veya kırpılamaz: kaydederken { $quality } kalitesiyle yeniden kodlanır ve meta veriler atılır.
editors-image-unsupported-warning = Bu biçim yazılamıyor: düzenlemeler kaydedilemez.
editors-image-fit = Görünüme sığdır (0)
editors-image-actual-size = Gerçek boyut (1)
editors-image-zoom-out = Uzaklaştır (-)
editors-image-zoom-in = Yakınlaştır (+)
editors-image-rotate-left = Sola döndür (Shift+R)
editors-image-rotate-right = Sağa döndür (R)
editors-image-flip-h = Yatay çevir (H)
editors-image-flip-v = Dikey çevir (V)
editors-image-crop = Kırp: bir dikdörtgen sürükleyin, Enter uygular (C)
editors-image-apply-crop = Kırpmayı uygula
editors-image-kind = Görüntü
editors-image-status = { $width } × { $height } px  ·  { $format }  ·  %{ $zoom }
editors-image-status-crop = kırpma { $width } × { $height }
editors-image-status-edits = { $count } düzenleme
editors-image-mode-lossless = Kayıpsız kaydedilir
editors-image-mode-lossy = Yeniden kodlanarak kaydedilir (kayıplı)
editors-image-mode-readonly = Salt okunur biçim
editors-image-crop-hint = Korunacak alanı seçmek için sürükleyin
