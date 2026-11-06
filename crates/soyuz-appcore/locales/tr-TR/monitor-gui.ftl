### Soyuz UI strings (tr-TR): monitor-gui
### Sistem izleyicinin grafik sayfaları ve ayar sayfası.
### Keys: en-US/monitor-gui.ftl ile aynı.

## Rail and top bar

monitor-gui-group-live = Canlı
monitor-gui-group-resources = Kaynaklar
monitor-gui-group-system = Sistem
monitor-gui-group-records = Kayıtlar
monitor-gui-status = Her { $interval } · { $processes } işlem · { $threads } iş parçacığı
monitor-gui-paused = Duraklatıldı
monitor-gui-pause = Okumaları duraklat
monitor-gui-resume = Okumaları sürdür
monitor-gui-take-snapshot = Anlık görüntü al
monitor-gui-interval = Okuma aralığı
monitor-gui-every = her
monitor-gui-waiting = İlk okumalar yolda.
monitor-gui-now = şimdi

## Charts

monitor-gui-avg-max = { $avg } (en çok { $max })
monitor-gui-resize-column = { $column } sütununu yeniden boyutlandır

## Overview

monitor-gui-cpu-sub = { $cores } mantıksal işlemci, { $clock }
monitor-gui-cpu-sub-load = { $cores } mantıksal işlemci · yük { $load }
monitor-gui-cpu-sub-plain = { $cores } mantıksal işlemci
monitor-gui-mem-sub = { $total } içinde { $percent }
monitor-gui-disk-sub = { $read } okuma · { $write } yazma
monitor-gui-net-sub = { $rx } gelen · { $tx } giden
monitor-gui-gpu-sub = { $total } bellekten { $used }
monitor-gui-battery = Pil
monitor-gui-battery-charging = Şarj oluyor
monitor-gui-battery-ac = Fişe takılı
monitor-gui-battery-discharging = Pilde
monitor-gui-hottest = En sıcak algılayıcı
monitor-gui-read = Okuma
monitor-gui-write = Yazma
monitor-gui-received = Alınan
monitor-gui-sent = Gönderilen
monitor-gui-host = Bilgisayar
monitor-gui-os = İşletim sistemi
monitor-gui-kernel-version = Çekirdek
monitor-gui-processor = İşlemci
monitor-gui-cores = Çekirdekler
monitor-gui-cores-value = { $physical } fiziksel · { $logical } mantıksal
monitor-gui-ram = Bellek
monitor-gui-uptime = Çalışma süresi
monitor-gui-load = Yük ortalaması
monitor-gui-virtualization = Sanallaştırma
monitor-gui-top-cpu = İşlemciye göre en üsttekiler
monitor-gui-top-memory = Belleğe göre en üsttekiler
monitor-gui-volumes = Birimler

## Processes

monitor-gui-filter-hint = Süzgeç: ad, user:root, cpu>5, mem>100M, /ifade/
monitor-gui-filter-regex-hint = Ad, komut, yol ve kullanıcı üzerinde düzenli ifade
monitor-gui-filter-help =
    Sözcükler ad, komut satırı, yol, kullanıcı ve PID ile eşleşir.
    "tırnaklı sözcükler" tek terimdir; -sözcük dışlar; a|b ikisinden biri demektir.
    /ifade/ veya /ifade/i bir düzenli ifadedir.
    user:root, name:, cmd:, path:, state:, pid:, ppid:, cgroup:, arch: tek bir sütunu sınar.
    cpu>5, mem>100M, threads>=10, read>1M, gpu>0 sayıları karşılaştırır (K, M, G, T sonekleri).
monitor-gui-filter-error = Süzgeç: { $message }
monitor-gui-regex = Süzgecin tümü bir düzenli ifade
monitor-gui-kernel = Çekirdek ve sistem işlemlerini göster
monitor-gui-other-users = Diğer kullanıcıların işlemlerini göster
monitor-gui-scale-machine = İşlemci: %100 tüm makine
monitor-gui-scale-per-core = İşlemci: %100 tek çekirdek
monitor-gui-heat = Hücreleri yüke göre renklendir
monitor-gui-details = Ayrıntı paneli
monitor-gui-export-csv = CSV olarak dışa aktar
monitor-gui-export-json = JSON olarak dışa aktar
monitor-gui-choose-columns = Sütunları seç…
monitor-gui-reset-columns = Sütunları sıfırla
monitor-gui-columns = Sütunlar
monitor-gui-search-columns = Sütunlarda ara
monitor-gui-unknown-user = Bilinmeyen kullanıcı
monitor-gui-marked-count = { $count } işaretli işlem
monitor-gui-end-tree = İşlem ağacını sonlandır
monitor-gui-kill-tree = İşlem ağacını öldür
monitor-gui-priority = Öncelik
monitor-gui-affinity = İşlemciler…
monitor-gui-affinity-title = İşlemciler
monitor-gui-affinity-for = { $name } (PID { $pid }) hangi mantıksal işlemcilerde çalışabilir.
monitor-gui-affinity-count = { $total } içinden { $on } seçili
monitor-gui-apply = Uygula
monitor-gui-open-location = Dosya konumunu aç
monitor-gui-copy-pid = PID
monitor-gui-copy-name = Ad
monitor-gui-copy-path = Yol
monitor-gui-copy-command = Komut satırı
monitor-gui-toggle-group = Grubu daralt veya genişlet
monitor-gui-expand-all = Tümünü genişlet
monitor-gui-collapse-all = Tümünü daralt
monitor-gui-resize-details = Ayrıntı panelini yeniden boyutlandır
monitor-gui-select-process = Ayrıntılarını görmek için bir işlem seçin.

## Details panel

monitor-gui-tab-general = Genel
monitor-gui-tab-threads = İş parçacıkları
monitor-gui-tab-modules = Modüller
monitor-gui-tab-files = Dosyalar
monitor-gui-tab-environment = Ortam
monitor-gui-tab-sockets = Soketler
monitor-gui-no-sockets = Açık soket yok.
monitor-gui-nothing-here = Gösterilecek bir şey yok.
monitor-gui-limited = Bunun bir bölümü okunamadı (erişim engellendi).
monitor-gui-col-proto = Protokol
monitor-gui-col-local = Yerel
monitor-gui-col-remote = Uzak
monitor-gui-col-state = Durum
monitor-gui-col-process = İşlem
monitor-gui-col-tid = TID
monitor-gui-col-thread-name = İş parçacığı
monitor-gui-col-module = Modül
monitor-gui-col-size = Boyut
monitor-gui-col-base = Taban adresi
monitor-gui-col-fd = FD
monitor-gui-col-path = Yol
monitor-gui-col-variable = Değişken
monitor-gui-col-value = Değer

## CPU

monitor-gui-total = Toplam
monitor-gui-user = Kullanıcı
monitor-gui-system = Sistem
monitor-gui-iowait = G/Ç bekleme
monitor-gui-irq = Kesmeler
monitor-gui-sockets = Yuvalar
monitor-gui-clock = Saat
monitor-gui-max-clock = Anma saati
monitor-gui-ctx-switches = Bağlam değişimleri
monitor-gui-interrupts = Kesmeler
monitor-gui-per-sec = { $value } /sn
monitor-gui-temperature = Sıcaklık
monitor-gui-cache-l1 = L1 önbellek
monitor-gui-cache-l2 = L2 önbellek
monitor-gui-cache-l3 = L3 önbellek
monitor-gui-processes = İşlemler
monitor-gui-threads = İş parçacıkları
monitor-gui-handles = Tanıtıcılar
monitor-gui-running = Çalışan
monitor-gui-logical-cpus = { $count } mantıksal işlemci
monitor-gui-core-graphs = Grafikler
monitor-gui-core-heat = Isı
monitor-gui-core-bars = Çubuklar

## Memory

monitor-gui-mem-in-use = Kullanımda
monitor-gui-mem-composition = Dağılım
monitor-gui-mem-cached = Önbellek
monitor-gui-mem-buffers = Arabellekler
monitor-gui-mem-kernel = Çekirdek
monitor-gui-mem-compressed = Sıkıştırılmış
monitor-gui-mem-free = Boş
monitor-gui-mem-shared = Paylaşılan
monitor-gui-swap = Takas
monitor-gui-no-swap = Takas alanı yok.
monitor-gui-commit = Yükleme payı
monitor-gui-commit-used = Yüklenen
monitor-gui-commit-limit = Yükleme sınırı
monitor-gui-available = Kullanılabilir
monitor-gui-page-in = Sayfa girişi
monitor-gui-page-out = Sayfa çıkışı

## Disks

monitor-gui-no-disks = Fiziksel disk bildirilmedi.
monitor-gui-busy = Meşgul
monitor-gui-busy-value = { $value } meşgul
monitor-gui-queue = Kuyruk
monitor-gui-iops = IOPS o/y
monitor-gui-read-total = Toplam okunan
monitor-gui-write-total = Toplam yazılan
monitor-gui-disk-read = Disk okuma
monitor-gui-disk-write = Disk yazma

## Network

monitor-gui-net-total = Tüm arabirimler
monitor-gui-net-received = Ağ gelen
monitor-gui-net-sent = Ağ giden
monitor-gui-interfaces = { $count } arabirim
monitor-gui-show-virtual = Sanal arabirimler
monitor-gui-show-virtual-n = { $count } sanal gizli
monitor-gui-up = Açık
monitor-gui-down = Kapalı
monitor-gui-virtual = Sanal
monitor-gui-rx-total = Toplam alınan
monitor-gui-tx-total = Toplam gönderilen
monitor-gui-packets = Paket/sn gelen / giden
monitor-gui-errors = Hatalar
monitor-gui-no-interfaces = Gösterilecek ağ arabirimi yok.

## GPU

monitor-gui-no-gpu = Grafik bağdaştırıcısı yok
monitor-gui-no-gpu-sub = Bu sistem GPU bildirmiyor ya da sürücüsü okuma vermiyor.
monitor-gui-load-label = Yük
monitor-gui-engines = Motorlar
monitor-gui-gpu-memory = GPU belleği
monitor-gui-gpu-shared = Paylaşılan sistem belleği
monitor-gui-not-reported = Sürücü bildirmiyor.
monitor-gui-power = Güç
monitor-gui-fan = Fan
monitor-gui-mem-clock = Bellek saati

## Sensors

monitor-gui-no-sensors = Algılayıcı yok
monitor-gui-no-sensors-sub = Bu sistem sıcaklık, fan ya da güç okuması sunmuyor.
monitor-gui-time-left = Kalan süre
monitor-gui-health = Sağlık
monitor-gui-cycles = Şarj döngüsü

## Lists

monitor-gui-list-filter-hint = Süz, ya da /ifade/
monitor-gui-count-of = { $total } içinden { $shown }
monitor-gui-no-connections = Açık bağlantı yok.
monitor-gui-no-services = Hizmet bulunamadı.
monitor-gui-no-startup = Sistemle birlikte başlayan bir şey yok.
monitor-gui-no-matches = Süzgeçle eşleşen bir şey yok.
monitor-gui-go-to-process = İşleme git
monitor-gui-copy-remote = Uzak adresi kopyala
monitor-gui-copy-local = Yerel adresi kopyala
monitor-gui-copy-source = Kaynağı kopyala
monitor-gui-col-service = Hizmet
monitor-gui-col-service-name = Ad
monitor-gui-col-start-mode = Başlatma
monitor-gui-col-description = Açıklama
monitor-gui-col-enabled = Etkin
monitor-gui-col-source = Kaynak
monitor-gui-enabled = Etkin
monitor-gui-disabled = Devre dışı

## History

monitor-gui-live = Canlı
monitor-gui-recorded-since = { $time } tarihinden beri kayıtta
monitor-gui-nothing-recorded = Bu aralık için kayıt yok.
monitor-gui-at-cursor = İmleçte
monitor-gui-latest = En son
monitor-gui-no-tops = Bu an için işlem listesi saklanmadı.

## Snapshots

monitor-gui-snapshot-label-hint = Sonraki anlık görüntünün etiketi
monitor-gui-take = Al
monitor-gui-no-snapshots = Henüz anlık görüntü yok
monitor-gui-no-snapshots-sub = Anlık görüntü, bir anın tüm okumalarını ve işlemlerini sonradan karşılaştırmak için saklar.
monitor-gui-auto = otomatik
monitor-gui-procs-short = işlem
monitor-gui-cpu-short = işlemci
monitor-gui-compare-from = Buradan karşılaştır (A)
monitor-gui-compare-with = Bununla karşılaştır (B)
monitor-gui-rename = Yeniden adlandır
monitor-gui-compare-a = Anlık görüntü A
monitor-gui-compare-b = Anlık görüntü B
monitor-gui-pick-snapshot = Bir anlık görüntü seçin
monitor-gui-by-program = Programa göre
monitor-gui-by-process = İşleme göre
monitor-gui-compare = Karşılaştır
monitor-gui-compare-title = İki anı karşılaştır
monitor-gui-compare-sub = A ve B anlık görüntülerini (ya da şimdiyi) seçip Karşılaştır'a basın.
monitor-gui-diff-summary = { $added } eklendi · { $removed } kaldırıldı · { $changed } değişti
monitor-gui-show-unchanged = Değişmeyenleri göster
monitor-gui-col-change = Değişim
monitor-gui-col-count = Sayı
monitor-gui-added = Eklendi
monitor-gui-removed = Kaldırıldı
monitor-gui-changed = Değişti
monitor-gui-same = Aynı

## Settings page

settings-monitor-readings = Okumalar
settings-monitor-interval = Okuma aralığı
settings-monitor-interval-desc = Bir izleyici sekmesi ekrandayken okumalar arasındaki milisaniye.
settings-monitor-page = Başlangıç sayfası
settings-monitor-page-desc = İzleyici sekmesinin açıldığı sayfa.
settings-monitor-graph-seconds = Canlı grafik süresi
settings-monitor-graph-seconds-desc = Canlı grafiklerin gösterdiği saniye.
settings-monitor-cpu-scale = İşlemci ölçeği
settings-monitor-cpu-scale-desc = Bir işlemin işlemci yükü tüm makinenin mi, tek çekirdeğin mi payı.
settings-monitor-core-view = Mantıksal işlemciler
settings-monitor-core-view-desc = İşlemci sayfası mantıksal işlemcileri nasıl çizer.
settings-monitor-temperature-unit = Sıcaklık birimi
settings-monitor-virtual-networks = Sanal ağ arabirimleri
settings-monitor-virtual-networks-desc = Ağ sayfasında geri döngü, tünel ve köprüleri de listele.
settings-monitor-table = İşlem tablosu
settings-monitor-view = Düzen
settings-monitor-view-desc = Düz liste, üst ve alt işlem ağacı ya da programa veya kullanıcıya göre gruplar.
settings-monitor-sort = Sıralama
settings-monitor-sort-desc = İşlem tablosunun sıralandığı sütun.
settings-monitor-sort-descending = Büyükten küçüğe
settings-monitor-sort-descending-desc = Seçilen sütunu en büyük değerden başlayarak sırala.
settings-monitor-columns = Sütunlar
settings-monitor-columns-desc = İşlem tablosunun sütunları. Sırayı değiştirmek için tabloda başlıkları sürükleyin; boyut için kenarlarını sürükleyin.
settings-monitor-columns-note = Ad sütunu her zaman gösterilir.
settings-monitor-heat = Hücreleri yüke göre renklendir
settings-monitor-heat-desc = Yük sütunları ısı haritası gibi, meşgul olduğu ölçüde daha koyu bir ton alır.
settings-monitor-icons = Program simgeleri
settings-monitor-icons-desc = Her programın kendi simgesini adının yanında göster.
settings-monitor-process-gpu = İşlem başına GPU kullanımı
settings-monitor-process-gpu-desc = Her işlemin GPU kullanımını oku (en pahalı okuma; birkaç saniyede bir yenilenir).
settings-monitor-kernel = Çekirdek ve sistem işlemleri
settings-monitor-kernel-desc = Çekirdek iş parçacıklarını ve işletim sisteminin iç sözde işlemlerini listele.
settings-monitor-other-users = Diğer kullanıcıların işlemleri
settings-monitor-other-users-desc = Başka hesaplar altında çalışan işlemleri listele.
settings-monitor-follow = Seçimi izle
settings-monitor-follow-desc = Satırlar yeniden sıralanırken seçili işlemi görünür tut.
settings-monitor-details = Ayrıntı paneli
settings-monitor-details-desc = Seçili işlemin ayrıntılarını tablonun altında göster.
settings-monitor-details-height = Ayrıntı paneli yüksekliği
settings-monitor-confirm-kill = İşlemi sonlandırmadan önce sor
settings-monitor-confirm-kill-desc = Bir işlemi sonlandırmadan, öldürmeden ya da askıya almadan önce onay iste. Kritik işlemler her zaman sorar.
settings-monitor-remember-filter = Süzgeci hatırla
settings-monitor-remember-filter-desc = İşlem süzgecini oturumlar arasında sakla.
settings-monitor-filter = Süzgeç
settings-monitor-filter-desc = Hatırlanan süzgeç. Sözcükler her metin sütunuyla eşleşir; /ifade/ düzenli ifadedir; user:root, cpu>5, mem>100M tek sütunu sınar.
settings-monitor-filter-regex = Süzgeç bir düzenli ifade
settings-monitor-filter-regex-desc = Süzgecin tümünü tek bir düzenli ifade olarak ele al.
settings-monitor-keep-parents = Ağacı süzerken üst işlemleri koru
settings-monitor-keep-parents-desc = Eşleşen işlemlerin üstleri soluk olarak ekranda kalır.
settings-monitor-history = Geçmiş
settings-monitor-background = Arka planda kaydet
settings-monitor-background-desc = İzleyici sekmesi açık değilken de geçmişi kaydetmeyi sürdür.
settings-monitor-background-interval = Arka plan aralığı
settings-monitor-background-interval-desc = İzleyici sekmesi açık değilken okumalar arasındaki saniye.
settings-monitor-persist = Geçmişi diskte sakla
settings-monitor-persist-desc = Geçmişi diske yaz; yeniden başlatmada kaybolmaz.
settings-monitor-history-days = Saklanan gün
settings-monitor-history-days-desc = Diskte saklanan geçmiş gün sayısı.
settings-monitor-snapshots = Anlık görüntüler
settings-monitor-snapshots-kept = Saklanan anlık görüntü
settings-monitor-snapshots-kept-desc = Bu sayı aşılınca en eski otomatik anlık görüntüler silinir. Kendi aldıklarınız hiç silinmez.
settings-monitor-commands = Anlık görüntü ve dışa aktarmalarda komut satırları
settings-monitor-commands-desc = Komut satırları parola ve belirteç içerebilir. Maskelenmiş, gizli görünen seçeneklerin değerlerini örter.
settings-monitor-commands-none = Dışarıda bırak
settings-monitor-commands-redacted = Maskelenmiş
settings-monitor-commands-full = Tam
settings-monitor-alerts = Uyarılar
settings-monitor-alert-cpu = İşlemci şunun üstünde
settings-monitor-alert-memory = Bellek şunun üstünde
settings-monitor-alert-temperature = Sıcaklık şunun üstünde
settings-monitor-alert-off-hint = Bu değerin üstünde kaldığında bildirim ver. 0 uyarıyı kapatır.
settings-monitor-alert-sustain = En az şu kadar süre
settings-monitor-alert-sustain-desc = Uyarı verilmeden önce değerin sınırın üstünde kalması gereken süre.
settings-monitor-alert-snapshot = Uyarıda anlık görüntü
settings-monitor-alert-snapshot-desc = Bir uyarı verildiğinde kendiliğinden anlık görüntü al.

monitor-gui-statusbar = { $page } · { $processes } işlem, { $running } çalışıyor

## Geçmiş kaydı

monitor-gui-record = Kaydet
monitor-gui-record-tip = Okumaları canlı grafiklerin son dakikalarının ötesinde, diskte de tutar. Siz durdurana kadar açık kalır.
monitor-gui-record-pause = Duraklat
monitor-gui-record-resume = Sürdür
monitor-gui-record-stop = Durdur
monitor-gui-recording = Kaydediliyor
monitor-gui-recording-paused = Kayıt duraklatıldı
monitor-gui-not-recording = Kayıt yok: yalnızca canlı okumalar
settings-monitor-record = Geçmişi kaydet
settings-monitor-record-desc = Okumaları canlı grafiklerin son dakikalarının ötesinde tutar. Geçmiş sayfası başlatır, duraklatır ve durdurur.
