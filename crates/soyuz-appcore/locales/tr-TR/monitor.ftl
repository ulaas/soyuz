### Soyuz UI strings (tr-TR): monitor
### Sistem izleyici, iki arayüz için.
### Keys: en-US/monitor.ftl ile aynı.

## Pages

monitor-page-overview = Genel bakış
monitor-page-processes = İşlemler
monitor-page-cpu = İşlemci
monitor-page-memory = Bellek
monitor-page-disks = Diskler
monitor-page-network = Ağ
monitor-page-gpu = GPU
monitor-page-sensors = Algılayıcılar
monitor-page-connections = Bağlantılar
monitor-page-services = Hizmetler
monitor-page-startup = Başlangıç
monitor-page-history = Geçmiş
monitor-page-snapshots = Anlık görüntüler

## Process table columns

monitor-col-name = Ad
monitor-col-pid = PID
monitor-col-parent = Üst işlem
monitor-col-user = Kullanıcı
monitor-col-state = Durum
monitor-col-cpu = İşlemci
monitor-col-cpu-time = İşlemci süresi
monitor-col-memory = Bellek
monitor-col-private = Özel
monitor-col-shared = Paylaşılan
monitor-col-virtual = Sanal
monitor-col-peak-memory = En yüksek bellek
monitor-col-memory-percent = Bellek %
monitor-col-threads = İş parçacıkları
monitor-col-handles = Tanıtıcılar
monitor-col-disk-read = Okuma/sn
monitor-col-disk-write = Yazma/sn
monitor-col-disk-read-total = Toplam okunan
monitor-col-disk-write-total = Toplam yazılan
monitor-col-gpu = GPU
monitor-col-gpu-memory = GPU belleği
monitor-col-page-faults = Sayfa hataları
monitor-col-priority = Öncelik
monitor-col-nice = Nice
monitor-col-session = Oturum
monitor-col-uptime = Çalışma süresi
monitor-col-started = Başlangıç
monitor-col-path = Yol
monitor-col-command = Komut satırı
monitor-col-arch = Mimari
monitor-col-elevated = Yönetici
monitor-col-cgroup = Denetim grubu
monitor-col-description = Açıklama

monitor-coldesc-name = Programın adı
monitor-coldesc-pid = İşlem kimliği
monitor-coldesc-parent = Onu başlatan işlemin kimliği
monitor-coldesc-user = Çalıştığı hesap
monitor-coldesc-state = Çalışıyor, uyuyor, askıda ve benzeri
monitor-coldesc-cpu = İşlemci yükü, bu tablo için seçilen ölçekte
monitor-coldesc-cpu-time = Başladığından beri kullandığı işlemci süresi
monitor-coldesc-memory = Fiziksel bellek (çalışma kümesi)
monitor-coldesc-private = Yalnızca bu işlemin tuttuğu bellek
monitor-coldesc-shared = Diğer işlemlerle paylaşılan fiziksel bellek
monitor-coldesc-virtual = Adres alanı veya ayrılan bellek
monitor-coldesc-peak-memory = Tuttuğu en yüksek fiziksel bellek
monitor-coldesc-memory-percent = Fiziksel belleğin makinenin belleğine oranı
monitor-coldesc-threads = İş parçacığı sayısı
monitor-coldesc-handles = Açık tanıtıcılar veya dosya tanımlayıcılar
monitor-coldesc-disk-read = Okuma hızı. Windows'ta borular ve soketler dahil tüm girişidir
monitor-coldesc-disk-write = Yazma hızı. Windows'ta borular ve soketler dahil tüm çıkışıdır
monitor-coldesc-disk-read-total = Başladığından beri okunan bayt
monitor-coldesc-disk-write-total = Başladığından beri yazılan bayt
monitor-coldesc-gpu = Grafik işlemci yükü
monitor-coldesc-gpu-memory = Tuttuğu grafik belleği
monitor-coldesc-page-faults = Başladığından beri sayfa hataları
monitor-coldesc-priority = Zamanlama önceliği
monitor-coldesc-nice = Nice değeri, Windows'ta temel öncelik numarası
monitor-coldesc-session = Oturum veya uçbirim
monitor-coldesc-uptime = Ne kadar süredir çalıştığı
monitor-coldesc-started = Ne zaman başladığı
monitor-coldesc-path = Çalıştırılabilir dosya
monitor-coldesc-command = Komut satırının tamamı
monitor-coldesc-arch = Programın işlemci mimarisi
monitor-coldesc-elevated = Yönetici haklarıyla çalışır
monitor-coldesc-cgroup = Linux denetim grubu (kapsayıcı veya hizmet)
monitor-coldesc-description = Varsa programın açıklaması

## Views and states

monitor-view-list = Liste
monitor-view-tree = Ağaç
monitor-view-by-name = Programa göre
monitor-view-by-user = Kullanıcıya göre

monitor-state-running = Çalışıyor
monitor-state-sleeping = Uyuyor
monitor-state-disk-wait = Disk bekliyor
monitor-state-stopped = Durduruldu
monitor-state-zombie = Zombi
monitor-state-idle = Boşta
monitor-state-suspended = Askıda
monitor-state-unknown = Bilinmiyor

monitor-priority-idle = Boşta
monitor-priority-below-normal = Normalin altında
monitor-priority-normal = Normal
monitor-priority-above-normal = Normalin üstünde
monitor-priority-high = Yüksek
monitor-priority-realtime = Gerçek zamanlı

monitor-yes = Evet
monitor-no = Hayır

## Process actions

monitor-action-end = İşlemi sonlandır
monitor-action-kill = İşlemi öldür
monitor-action-suspend = Askıya al
monitor-action-resume = Sürdür
monitor-action-hangup = Kapatma sinyali gönder
monitor-action-interrupt = Kesme sinyali gönder

monitor-signal-done = { $what }: { $count ->
    [one] tamamlandı
   *[other] { $count } işlem için tamamlandı
}
monitor-signal-failed = { $what } başarısız oldu
monitor-no-windows = { $count ->
    [one] Bu işlemin kapatılacak penceresi yok
   *[other] { $count } işlemin kapatılacak penceresi yok
}
monitor-signal-confirm-title = { $what }: { $count ->
    [one] bu işlem?
   *[other] bu { $count } işlem?
}
monitor-signal-confirm = { $names }. Kaydedilmemiş çalışmalar kaybolabilir.
monitor-signal-confirm-tree = { $names } ve başlattığı tüm işlemler. Kaydedilmemiş çalışmalar kaybolabilir.
monitor-risk-critical = İşletim sistemi bu işleme bağlıdır. Durdurmak bilgisayarı dondurabilir veya yeniden başlatabilir.
monitor-risk-other-user = Bu işlem başka bir kullanıcıya ait.

monitor-priority-set = { $name }: öncelik { $what } olarak ayarlandı
monitor-priority-failed = { $name } önceliği değiştirilemedi
monitor-affinity-set = { $name }: işlemci kümesi değiştirildi
monitor-affinity-failed = { $name } işlemcileri değiştirilemedi
monitor-affinity-empty = En az bir işlemci seçin
monitor-no-path = Bu programın yolu bilinmiyor

monitor-error-gone = İşlem zaten sonlanmış.
monitor-error-denied = Erişim reddedildi.
monitor-error-unsupported = Bu sistem bunu yapamıyor.

monitor-no-data = Henüz ölçüm yok

## Services

monitor-service-start = Başlat
monitor-service-stop = Durdur
monitor-service-restart = Yeniden başlat
monitor-service-starting = Başlıyor
monitor-service-stopping = Duruyor
monitor-service-paused = Duraklatıldı
monitor-service-failed-state = Hatalı
monitor-service-confirm-title = { $what }: { $name }?
monitor-service-confirm = Başka programlar bu hizmete bağlı olabilir.
monitor-service-done = { $what }: { $name }
monitor-service-failed = { $what }: { $name } başarısız oldu

## Snapshots and exports

monitor-snapshot-taken = Anlık görüntü kaydedildi: { $label }
monitor-snapshot-failed = Anlık görüntüler değiştirilemedi
monitor-snapshot-delete-title = Bu anlık görüntü silinsin mi?
monitor-snapshot-delete = "{ $label }" kalıcı olarak silinir.
monitor-exported = Dışa aktarıldı
monitor-export-failed = Dışa aktarma başarısız oldu
monitor-show-folder = Klasörü göster

## Alerts

monitor-alert-open = İzleyiciyi aç
monitor-alert-cpu = İşlemci yükü yüksek
monitor-alert-memory = Bellek neredeyse dolu
monitor-alert-temperature = Bir sıcaklık yüksek
monitor-alert-body = { $value } değeri { $limit } sınırının üzerinde kaldı.
monitor-alert-snapshot = Bir anlık görüntü kaydedildi.
