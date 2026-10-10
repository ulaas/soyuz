### Soyuz arayüz metinleri (tr-TR): tools
### Anahtarlar en-US/tools.ftl ile birebir aynı olmalı.

## Araç başlatma bildirimleri

tools-toast-open-folder-treemap = Disk kullanımını incelemek için bir klasör açın
tools-toast-open-folder-compare = Karşılaştırmak için bir klasör açın
tools-toast-select-file-split = Bölmek için tek bir dosya seçin
tools-toast-select-first-part = Bölünmüş bir dosyanın ilk parçasını (.001) seçin
tools-toast-select-versions = Önceki sürümlerini görmek için tek bir dosya veya klasör seçin
tools-toast-no-profile = “{ $name }” adlı bir eşitleme profili yok
tools-toast-versions-off = Önceki sürümler kapalı
tools-toast-versions-off-body = Ayarlar › İşler › Eşitleme ve sürümler bölümünden açabilirsiniz.
tools-toast-tagged = { $count } öğeye “{ $tag }” etiketi eklendi
tools-toast-untagged = { $count } öğeden “{ $tag }” etiketi kaldırıldı

## Etiketler

tools-tags-menu = Etiketler
tools-tags-section = Etiketler
tools-tags-clear = Etiketleri temizle
tools-tags-count = { $count } etiket
tools-tags-rename = Etiketi yeniden adlandır…
tools-tags-reset-name = Adı sıfırla
tools-tags-color-tip = { $color } etiket
tools-recent = Son kullanılanlar

## Bağlam menüsü satırları

tools-menu-add-to-shelf = Rafa ekle
tools-menu-analyze-disk-usage = Disk kullanımını incele
tools-menu-find-duplicates = Yinelenen dosyaları bul
tools-menu-compare-folders = Klasörleri karşılaştır
tools-menu-compare-folders-dialog = Klasörleri karşılaştır…
tools-menu-compare-other-pane = Diğer bölmeyle karşılaştır
tools-menu-join-parts = Parçaları birleştir…
tools-menu-split-file = Dosyayı böl…
tools-menu-clear-recent = Son kullanılanları temizle
tools-menu-remove-from-recent = Son kullanılanlardan kaldır

## Git süslemeleri

tools-git-ignored = Yok sayıldı
tools-git-untracked = İzlenmiyor
tools-git-type-changed = Türü değişti
tools-git-renamed = Yeniden adlandırıldı
tools-git-added = Eklendi
tools-git-deleted = Silindi
tools-git-modified = Değiştirildi
tools-git-conflict = Çakışma
tools-git-staged = { $status } (hazırlandı)
tools-git-detached = bağımsız
tools-git-tip-repo = Git deposu: { $root }
    Dal: { $branch }
tools-git-tip-counts = { $changed } değişen · { $staged } hazırlanan · { $untracked } izlenmeyen
tools-git-tip-counts-conflicts = { $changed } değişen · { $staged } hazırlanan · { $untracked } izlenmeyen · { $conflicts } çakışma
tools-git-tip-click = Depo kök klasörünü açmak için tıklayın

## Raf

tools-shelf-title = Raf
tools-shelf-tip-count = Raf · { $count } öğe
tools-shelf-tip-empty = Raf (toplamak için dosyaları buraya bırakın)
tools-shelf-hide = Rafı gizle
tools-shelf-clear = Rafı temizle
tools-shelf-empty-title = Dosyaları buraya bırakın
tools-shelf-empty-body = Farklı klasörlerdeki öğeleri burada toplayın, sonra sürükleyip bırakın ya da hepsini tek seferde kopyalayın.
tools-shelf-missing = Bulunamadı
tools-shelf-item-sub = { $size } · { $folder }
tools-shelf-item-tip = { $path }
    Kopyalamak veya taşımak için sürükleyin · göstermek için çift tıklayın
tools-shelf-reveal = Bölmede göster
tools-shelf-remove = Raftan kaldır
tools-shelf-footer = { $which ->
    [selected] Seçili
   *[other] Tümü
    } ({ $count }) → { $dest }
tools-shelf-copy-here = Buraya kopyala
tools-shelf-move-here = Buraya taşı

## Ortak araç sözcükleri

tools-stop = Durdur
tools-rescan = Yeniden tara
tools-open = Aç
tools-open-in-pane = Bölmede aç
tools-reveal-in-system = Sistem dosya yöneticisinde göster
tools-copy-path = Yolu kopyala
tools-move-to-trash = Çöp Kutusu'na taşı
tools-move-to-trash-dialog = Çöp Kutusu'na taşı…
tools-percent = %{ $pct }
tools-files = { $count } dosya

## Disk kullanımı (ağaç haritası)

tools-treemap-title = Disk kullanımı
tools-treemap-up = Bir üst düzey
tools-treemap-totals = { $size }  ·  { $count } dosya
tools-treemap-listing = Listeleniyor…
tools-treemap-scanning = Taranıyor: { $done } / { $total } klasör · şu ana kadar { $size } · { $current }
tools-treemap-failed = Bu klasör taranamıyor: { $error }
tools-treemap-scanned = { $secs } sn'de tarandı
tools-treemap-scanned-errors = { $secs } sn'de tarandı · { $errors } klasör okunamadı
tools-treemap-legend-folders = Klasörler
tools-treemap-legend-images = Resimler
tools-treemap-legend-video = Video
tools-treemap-legend-audio = Ses
tools-treemap-legend-archives = Arşivler
tools-treemap-legend-code = Kod
tools-treemap-legend-documents = Belgeler
tools-treemap-legend-programs = Programlar
tools-treemap-legend-other = Diğer
tools-treemap-confirm-trash = “{ $name }” ({ $size }) Çöp Kutusu'na taşınsın mı?
tools-treemap-measuring = Ölçülüyor…
tools-treemap-empty-folder = Bu klasör boş
tools-treemap-single-file = Tek bir dosya
tools-treemap-tab-contents = İçerik
tools-treemap-tab-largest = En büyük dosyalar
tools-treemap-no-files = Bu düzeyde dosya yok
tools-treemap-many-small = Çok sayıda küçük öğe
tools-treemap-zoom-in = Klasöre yakınlaş
tools-treemap-smaller-items = Daha küçük öğeler
tools-treemap-share = Üst klasörün { $parent } kadarı · görünümün { $view } kadarı
tools-treemap-measure-deeper = Daha derin düzeyleri ölçmek için tıklayın
tools-treemap-tile-a11y = { $name }, { $size }
tools-add = Ekle
tools-clear = Temizle

## Yinelenen dosya bulucu

tools-dupes-title = Yinelenen dosya bulucu
tools-dupes-folders = { $count } klasör
tools-dupes-search-in = Aranacak yer
tools-dupes-root-tip = { $path }
    Kaldırmak için tıklayın
tools-dupes-add-path-hint = Klasör yolu ekleyin…
tools-dupes-add-other-pane = Diğer bölmeyi ekle
tools-dupes-min-size = En küçük boyut
tools-dupes-size-any = Tümü
tools-dupes-hashing = Karma hesaplanıyor: { $done } / { $total } aday · { $size } okundu · şu ana kadar { $groups } grup · { $secs } sn
tools-dupes-summary = { $groups } yinelenen dosya grubu · { $size } geri kazanılabilir
tools-dupes-summary-secs = { $groups } yinelenen dosya grubu · { $size } geri kazanılabilir · { $secs } sn
tools-dupes-summary-ms = { $groups } yinelenen dosya grubu · { $size } geri kazanılabilir · { $ms } ms
tools-dupes-empty-running = Yinelenen dosyalar aranıyor…
tools-dupes-empty-running-body = Gruplar doğrulanır doğrulanmaz burada görünür.
tools-dupes-empty-handled = Tüm yinelenen dosyalar işlendi
tools-dupes-empty-handled-body = İşaretlenen kopyalar Çöp Kutusu'na taşındı (Geri al bildirimde).
tools-dupes-empty-none = Yinelenen dosya bulunamadı
tools-dupes-empty-none-body = Bu klasörlerdeki her dosya benzersiz.
tools-dupes-empty-start = Aynı dosyaları bulun
tools-dupes-empty-start-body = Klasörleri ve en küçük boyutu seçip başlatın.
tools-dupes-select-every-group = Her grupta seç:
tools-dupes-keep-newest = En yeniyi tut
tools-dupes-keep-oldest = En eskiyi tut
tools-dupes-keep-shortest = En kısa yolu tut
tools-dupes-newest = En yeni
tools-dupes-oldest = En eski
tools-dupes-shortest = En kısa yol
tools-dupes-keep = Tut
tools-dupes-keep-one = En az bir kopya kalmalı
tools-dupes-mark-hint = Kaldırmak istediğiniz kopyaları işaretleyin
tools-dupes-marked = { $count } dosya işaretlendi · { $size }
tools-dupes-move-n-to-trash = { $count } dosyayı Çöp Kutusu'na taşı
tools-dupes-toast-moving = { $count } yinelenen dosya Çöp Kutusu'na taşınıyor
tools-dupes-delete-marked = İşaretlileri sil…
tools-dupes-group-title = { $count } kopya · her biri { $size }
tools-dupes-reclaimable = { $size } geri kazanılabilir
tools-dupes-row-tip = { $path }
    İşaretlemek için tıklayın · göstermek için çift tıklayın
tools-done = Bitti
tools-cancelled = İptal edildi

## Bölme / birleştirme

tools-split-title = Dosyayı böl
tools-join-title = Dosyaları birleştir
tools-split-err-not-part = Bölünmüş bir parça değil (ad.001 bekleniyordu)
tools-split-err-not-found = { $path } bulunamadı
tools-split-err-open = { $path } açılamıyor: { $error }
tools-split-err-create = { $path } oluşturulamıyor: { $error }
tools-split-err-read = { $path } okunamıyor: { $error }
tools-split-err-write = { $path } yazılamıyor: { $error }
tools-split-err-exists = { $path } zaten var
tools-split-err-zero-size = Parça boyutu sıfırdan büyük olmalıdır
tools-split-err-no-name = Dosya adı yok
tools-split-err-invalid-size = Geçerli bir parça boyutu girin (ör. 250 MB)
tools-split-done = Dosya bölündü
tools-split-done-body = { $count } parça ve { $checksum } oluşturuldu
tools-join-done = Dosyalar birleştirildi
tools-join-verified = { $name } · sağlama toplamı doğrulandı
tools-join-mismatch = Sağlama toplamı uyuşmuyor
tools-join-mismatch-body = Birleştirilen dosya sağlama toplamıyla uyuşmuyor; { $name } adıyla saklandı
tools-join-unverified = { $name } · doğrulanacak sağlama toplamı dosyası yok
tools-split-label-file = DOSYA
tools-split-unknown-size = boyut bilinmiyor
tools-split-label-part-size = PARÇA BOYUTU
tools-split-custom = Özel
tools-split-custom-hint = ör. 250 MB
tools-split-plan = { $size } boyutunda { $count } parça (+ bir .blake3 sağlama toplamı dosyası)
tools-split-label-parts = PARÇALAR
tools-join-plan = { $count } parça → { $name }
tools-join-has-checksum = Bir sağlama toplamı dosyası bulundu; sonuç doğrulanacak.
tools-join-no-checksum = Sağlama toplamı dosyası yok; sonuç doğrulanmayacak.
tools-split-label-output = ÇIKTI KLASÖRÜ
tools-split-folder-hint = Klasör
tools-split-progress = { $done } / { $total }  ·  { $speed }/sn
tools-split-start = Böl
tools-join-start = Birleştir
tools-back = Geri

## Karşılaştır ve eşitle

tools-compare-title = Karşılaştır ve eşitle
tools-compare-only-left = Yalnızca solda
tools-compare-newer-left = Solda daha yeni
tools-compare-different = Farklı
tools-compare-newer-right = Sağda daha yeni
tools-compare-only-right = Yalnızca sağda
tools-preset-update = İki yönlü güncelleme
tools-preset-mirror-right = Yansıt: sol → sağ
tools-preset-mirror-left = Yansıt: sağ → sol
tools-preset-two-way = İki yönlü eşitleme
tools-preset-two-way-tip = Son eşitlemeyi hatırlar; böylece iki taraftaki silme ve düzenlemeler birbirinden ayırt edilir. Her iki tarafta da yapılan değişiklikler çakışma olarak sorulur.
tools-method-date-size = Tarih ve boyut
tools-method-size = Boyut
tools-method-content = İçerik
tools-reason-deleted-right = Sağda silindi
tools-reason-deleted-left = Solda silindi
tools-reason-new-left = Solda yeni
tools-reason-new-right = Sağda yeni
tools-reason-changed-left = Solda değişti
tools-reason-changed-right = Sağda değişti
tools-conflict-both-changed = Her iki tarafta da değişti
tools-conflict-both-created = Her iki tarafta farklı
tools-conflict-changed-left-deleted-right = Solda değişti · sağda silindi
tools-conflict-deleted-left-changed-right = Solda silindi · sağda değişti
tools-resolution-left = Solu tut
tools-resolution-right = Sağı tut
tools-resolution-both = İkisini de tut
tools-compare-err-choose = Karşılaştırılacak iki klasör seçin.
tools-compare-err-same = İki taraf da aynı klasör.
tools-compare-state-save-failed = Eşitleme durumu kaydedilemedi
tools-compare-job-label = Eşitle: { $left } ↔ { $right }
tools-compare-left-hint = Sol klasör
tools-compare-right-hint = Sağ klasör
tools-compare-swap = Tarafları değiştir
tools-compare-by = Karşılaştırma ölçütü
tools-compare-sync-mode = Eşitleme modu
tools-compare-start = Karşılaştır
tools-compare-empty = İki klasör seçip Karşılaştır'a basın
tools-compare-empty-body = Farklar eşitlemeye hazır olarak burada görünür.
tools-compare-running = Karşılaştırılıyor… { $secs } sn
tools-compare-syncing = Eşitleniyor… İş bitince klasörler yeniden karşılaştırılacak.
tools-compare-recording = Eşitleme durumu kaydediliyor…
tools-compare-failed = Bu klasörler karşılaştırılamıyor
tools-compare-identical = · { $count } aynı
tools-compare-filter-hint = Adları filtrele
tools-compare-first-run = Bu klasörlerin ilk iki yönlü eşitlemesi: daha yeni dosyalar kazanır, belirsiz olanlar çakışma sayılır. Bundan sonra iki taraftaki silme ve düzenlemeler izlenir.
tools-compare-in-sync = Klasörler eşit
tools-compare-in-sync-body = Hiç fark bulunamadı.
tools-compare-review = Eşitlemeyi gözden geçir…
tools-compare-resolve-n = { $count } çakışmayı çöz…
tools-compare-conflicts-button = Çakışmalar…
tools-profiles = Profiller
tools-profiles-none = Kayıtlı profil yok
tools-profile-tip = { $left }
    ↔ { $right }
    { $method } · { $mode }
tools-profiles-save-as = Geçerli ayarları farklı kaydet
tools-profiles-name-hint = Profil adı
tools-profiles-delete-hint = Silmek için profile sağ tıklayın
tools-profiles-schedules = Zamanlamalar ve raporlar…
tools-compare-col-left = SOL
tools-compare-col-name = AD
tools-compare-col-right = SAĞ
tools-compare-all-visible = tüm görünür satırlar
tools-compare-meta-folder = Klasör  ·  { $modified }
tools-compare-meta-folder-only = Klasör
tools-compare-meta-deleted = Silindi
tools-compare-meta-left = Sol: { $meta }
tools-compare-meta-right = Sağ: { $meta }
tools-compare-resolved = Çözüldü
tools-compare-plan-title = Eşitleme planını gözden geçirin
tools-compare-plan-trash-note = Birim destekliyorsa silinen öğeler Çöp Kutusu'na gider.
tools-compare-plan-open-conflicts = Çözülmemiş { $count } çakışma atlanacak ve bir dahaki sefere yeniden sorulacak.
tools-compare-copy-right = Sağa kopyala
tools-compare-copy-left = Sola kopyala
tools-compare-delete-left = Solda sil
tools-compare-delete-right = Sağda sil
tools-compare-conflict-copy-left = Çakışma kopyası ←
tools-compare-conflict-copy-right = Çakışma kopyası →
tools-compare-start-sync = Eşitlemeyi başlat ({ $count })
tools-compare-conflicts-title = Çakışmaları çözün
tools-compare-conflicts-body = { $count } öğe son eşitlemeden bu yana her iki tarafta da değişti. Hangi sürümün geçerli olacağını seçin; atlanan öğeler bir dahaki sefere yeniden sorulur.
tools-compare-apply-all = Tümüne uygula
tools-compare-keep-newer = Daha yeniyi tut
tools-compare-keep-newer-tip = Her öğe için en son değiştirilen taraf kazanır
tools-compare-all-decided = Tümü karara bağlandı
tools-compare-n-open = { $count } açık
tools-compare-res-delete-right = Sağda da sil
tools-compare-res-delete-left = Solda da sil
tools-compare-res-left = Sol sürümü sağdakinin üzerine kopyala
tools-compare-res-right = Sağ sürümü soldakinin üzerine kopyala
tools-compare-res-both = Sol taraf adını korur; sağ sürüm her iki tarafta çakışma kopyası olarak saklanır
tools-compare-res-skip = Sonra karar ver
tools-plan-to-right = { $count } → sağa
tools-plan-to-left = { $count } ← sola
tools-plan-to-delete = { $count } silinecek
tools-plan-to-copy = { $size } kopyalanacak

## Eşitleme zamanlamaları ve raporları

tools-sched-manual = Elle
tools-sched-every-hours = { $count ->
    [one] Saatte bir
   *[other] { $count } saatte bir
}
tools-sched-every-minutes = { $count } dakikada bir
tools-sched-daily = Her gün { $time }
tools-sched-on-change = Dosyalar değiştiğinde
tools-sched-on-start = Soyuz başladığında
tools-trigger-manual = elle
tools-trigger-schedule = zamanlanmış
tools-trigger-change = değişiklikte
tools-trigger-start = başlangıçta
tools-status-in-sync = Eşit
tools-status-dry-run = Deneme çalıştırması
tools-status-applied = Eşitlendi
tools-status-partial = Kısmen eşitlendi
tools-status-failed = Başarısız
tools-report-keep-copy = Kopya sakla
tools-report-nothing = değişecek bir şey yok
tools-report-copy = { $count } kopyalandı
tools-report-would-copy = { $count } kopyalanacak
tools-report-delete = { $count } silindi
tools-report-would-delete = { $count } silinecek
tools-report-actions-total = toplam { $count } işlem
tools-report-failed = { $count } başarısız
tools-report-conflicts = { $count } çakışma atlandı
tools-report-state-not-saved = eşitlendi, ancak eşitleme durumu kaydedilemedi: { $error }
tools-activity-planning = Planlanıyor…
tools-activity-syncing = Eşitleniyor…
tools-activity-recording = Durum kaydediliyor…
tools-sched-job-label = “{ $name }” eşitleme profili
tools-sched-toast-failed = “{ $name }” eşitlemesi başarısız oldu
tools-sched-toast-conflicts = “{ $name }”: çözülmesi gereken { $count } çakışma
tools-sched-toast-conflicts-body = Her iki tarafta da değiştikleri için atlandılar. Seçim yapmak için profili açın.
tools-sched-open-profiles = Eşitleme profillerini aç
tools-sched-toast-dry-run = “{ $name }” deneme çalıştırması
tools-sched-toast-in-sync = “{ $name }” eşit
tools-sched-err-unavailable = { $side ->
    [left] Sol
   *[other] Sağ
    } klasör ({ $path }) kullanılamıyor
tools-sched-watch-failed = Eşitleme klasörleri izlenemiyor
tools-next-scheduler-off = Zamanlayıcı kapalı (Ayarlar › İşler)
tools-next-manual = Siz başlattığınızda çalışır
tools-next-on-change = Sonraki: bir dosya değiştiğinde
tools-next-ran = Bu oturumda çalıştı
tools-next-now = Sonraki: şimdi
tools-next-minute = Sonraki: bir dakika içinde
tools-next-in-minutes = Sonraki: { $count } dk içinde
tools-next-today = Sonraki: bugün { $time }
tools-next-tomorrow = Sonraki: yarın { $time }
tools-next-date = Sonraki: { $day } { $month }

## Eşitleme profilleri penceresi

tools-profiles-title = Eşitleme profilleri
tools-profiles-count = { $count } profil
tools-profiles-scheduler-on = Zamanlanmış profiller Soyuz açıkken çalışır; her çalıştırma etkinlik panelinde normal bir iştir.
tools-profiles-scheduler-off = Zamanlayıcı kapalı (Ayarlar › İşler › Eşitleme ve sürümler); profiller yalnızca siz başlattığınızda çalışır.
tools-profiles-empty = Kayıtlı eşitleme profili yok
tools-profiles-empty-body = Karşılaştır ve eşitle penceresini açın, iki klasör seçip Profiller altında kaydedin. Zamanlamalar ve raporlar burada görünür.
tools-profiles-expand = Zamanlama ve raporlar
tools-profiles-run-now = Şimdi çalıştır
tools-profiles-dry-run-tip = Hiçbir şeyi değiştirmeden nelerin değişeceğini gösteren bir rapor yazar
tools-profiles-open-tip = Klasörleri karşılaştırın, işlemleri elle seçin (ve çakışmaları çözün)
tools-profiles-card-paths = { $left }  ↔  { $right }  ·  { $mode } · { $method }
tools-profiles-card-paths-tip = { $left }
    ↔ { $right }
tools-profiles-last-run = { $when } ({ $trigger }) · { $summary }
tools-profiles-never-run = Hiç çalışmadı
tools-profiles-reports = Çalıştırma raporları
tools-profiles-no-runs = Henüz çalıştırma yok. Nelerin değişeceğini görmek için Deneme çalıştırması'nı kullanın.
tools-profiles-report-row = { $status } · { $trigger }
tools-sched-label = Zamanlama
tools-sched-kind-interval = Her…
tools-sched-kind-daily = Günlük
tools-sched-kind-change = Değişiklikte
tools-sched-kind-start = Başlangıçta
tools-sched-every = Her
tools-sched-at = Saat
tools-unit-min = dk
tools-unit-hour = sa
tools-sched-daily-note = Soyuz kapalıyken kaçırılan çalıştırma bir sonraki açılışta telafi edilir.
tools-sched-change-note = Klasörlerden birinde dosyalar değiştikten birkaç saniye sonra çalışır (Ayarlar › İşler › değişiklik gecikmesi).
tools-report-would-change = Değişecekler: { $summary }
tools-report-changed = Değişenler: { $summary }
tools-report-nothing-to-change = Değişecek bir şey yok.
tools-report-more = … ve { $count } tane daha

## Önceki sürümler

tools-versions-title = Önceki sürümler
tools-versions-off = Önceki sürümler kapalı (Ayarlar › İşler › Eşitleme ve sürümler).
tools-versions-intro = Gölge kopyalardaki eski sürümler (Windows'un geri yükleme noktaları ve yedekler için aldığı anlık görüntüler).
tools-versions-look-again = Yeniden ara
tools-versions-looking = Gölge kopyalar aranıyor…
tools-versions-list-failed = Önceki sürümler listelenemiyor
tools-versions-unsupported = Burada kullanılamaz
tools-versions-elevate = Soyuz'u yönetici olarak aç
tools-versions-elevate-tip = Yönetici haklarıyla yeni bir Soyuz penceresi açar (Windows önce onay ister)
tools-versions-denied-more = { $count } gölge kopya daha yönetici hakları olmadan okunamadı.
tools-versions-copy-to = Şuraya kopyala…
tools-versions-restore = Geri yükle
tools-versions-restore-tip = Bu sürümü özgün konumuna geri kopyalar (değiştirmeden önce sorar; geri alınabilir)
tools-versions-current = Geçerli
tools-versions-this = Bu sürüm
tools-versions-deleted = silinmiş
tools-versions-modified = Değiştirilme: { $date }
tools-versions-comparing = İçerikler karşılaştırılıyor…
tools-versions-same = İçerik geçerli dosyayla aynı.
tools-versions-differ = İçerikler farklı. { $note }
tools-versions-admin-needed = Yönetici hakları gerekiyor
tools-versions-admin-needed-body = Bu birimde { $count } gölge kopya var, ancak Windows bunları yalnızca yöneticilerin okumasına izin veriyor. Bu { $kind ->
    [folder] klasörün
   *[other] dosyanın
    } eski sürümlerini görmek için Soyuz'u yönetici olarak başlatın.
tools-versions-none = Eski sürüm yok
tools-versions-none-body = Bu birimdeki { $count } gölge kopyanın hiçbiri bu { $kind ->
    [folder] klasörün
   *[other] dosyanın
    } farklı bir sürümünü içermiyor.
tools-versions-no-copies = Bu birimde gölge kopya yok
tools-versions-no-copies-body = Windows; geri yükleme noktaları, Windows Yedekleme ve Dosya Geçmişi için gölge kopyalar oluşturur (sürücüde Sistem Koruması açık olmalıdır). Henüz hiç gölge kopya yok.
tools-versions-shadow-copy = Gölge kopya
tools-versions-snapshot-time = { $provider }  ·  { $date }
tools-versions-snapshot-seq = { $provider } #{ $seq }
tools-versions-detail-folder = Klasör  ·  değiştirilme { $modified }
tools-versions-detail-file = { $size }  ·  değiştirilme { $modified }
tools-versions-unreadable = Dosyalardan biri okunamıyor.
tools-versions-size-diff = Geçerli dosya { $size } daha { $dir ->
    [larger] büyük
   *[other] küçük
    }.
tools-versions-too-large = Boyutlar aynı; hızlı karşılaştırmak için çok büyük.
tools-versions-different-bytes = Boyutlar aynı, baytlar farklı.
tools-versions-job-label = “{ $name }” öğesinin önceki sürümünü geri yükle
tools-versions-elevate-failed = Soyuz yönetici olarak başlatılamadı
tools-versions-open-failed = Bu sürüm açılamadı
