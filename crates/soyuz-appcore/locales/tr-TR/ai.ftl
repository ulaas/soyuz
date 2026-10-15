### Soyuz arayüz metinleri (tr-TR): ai
### Anahtarlar en-US/ai.ftl ile birebir aynı olmalı.

## Klasör erişimi (çipler, menüler, özellikler penceresi)

ai-access-short-none = Erişim yok
ai-access-short-metadata = Üst veri
ai-access-short-contents = İçerik
ai-access-inherited = { $access } (devralındı)
ai-access-menu-caption = Asistanın bu klasöre erişimi
ai-access-menu-none = Erişim yok (asistandan gizli)
ai-access-menu-metadata = Yalnızca üst veri (adlar, boyutlar, tarihler)
ai-access-menu-contents = İçeriğe izin ver (modele gönderilebilir)
ai-access-inherit = Üst klasörlerden devral
ai-access-tip = Asistanın { $name } içinde görebilecekleri
ai-access-tip-inherited = Asistanın { $name } içinde görebilecekleri (devralındı)
ai-properties-access = YZ erişimi

## Bağlam menüsü

ai-menu-assistant = Asistan
ai-menu-suggest-names = Ad öner
ai-menu-summarize = Özetle
ai-menu-find-related = İlgili dosyaları bul
ai-menu-explain-folder = Bu klasörü açıkla
ai-menu-organize = Düzenle…
ai-menu-smart-search = Akıllı arama…

## Panel başlığı ve çipler

ai-panel-title = Asistan
ai-status-recipe = Tarif çalıştırılıyor…
ai-status-waiting = Onayınız bekleniyor
ai-status-working = Çalışıyor · { $secs } sn
ai-status-idle = Her değişikliği planlar; siz onaylamadan hiçbir şey olmaz
ai-status-external = { $count } dış istek onayınızı bekliyor
ai-close = Asistanı kapat
ai-close-key = Asistanı kapat  ({ $key })
ai-settings = YZ ayarları
ai-new-chat = Yeni sohbet
ai-no-model = Yapılandırılmış model yok
ai-model-caption = Sonraki mesajın modeli
ai-model-auto = Otomatik (hızlı / güçlü yönlendirme)
ai-manage-providers = Sağlayıcıları yönet…
ai-route-fast = Hızlı
ai-route-strong = Güçlü
ai-route-tip = Yazdığınız mesajlar için model yönlendirmesi: hızlı (adlandırma, sınıflandırma) veya güçlü (planlama). Eğik çizgi komutları kendi modelini seçer.
ai-meter-tokens = { $tokens } token
ai-meter-tokens-cost = { $tokens } token · ${ $cost }
ai-meter-tip-capped =
    Bu görevin / oturumun token sayısı ve maliyeti
    Bu ay: ${ $cap } sınırın ${ $spent } kadarı kullanıldı (% { $percent })
ai-meter-tip-month = Bu ay: ${ $spent } (sınır yok)
ai-meter-tip = Token sayısı ve maliyet (fiyatı bilinmeyen modellerde yalnızca token)
ai-tab-chat = Sohbet
ai-tab-recipes = Tarifler
ai-tab-audit = Denetim günlüğü
ai-stop-esc = Durdur  Esc

## Sohbet

ai-empty-title = Bu klasör hakkında soru sorun
ai-empty-body = Asistan okuma araçlarıyla etrafa bakar ve her değişikliği onaylamanız için bir plan olarak önerir. Onaylanan planlar normal, geri alınabilir işler olarak çalışır.
ai-idea-organize = Bu klasörü düzenle
ai-idea-explain = Burada neler var?
ai-idea-dedupe = Yinelenen dosyaları bul
ai-idea-rename = Seçili öğeler için daha iyi adlar öner
ai-thinking = Düşünüyor
ai-confirm-cost = Bu görevin maliyeti yaklaşık ${ $cost } olabilir
ai-confirm-body = { $model } için tam bir çalışma üzerinden tahmin edildi. İstediğiniz an Esc ile durdurabilirsiniz.
ai-start = Başlat
ai-end-changes = { $count } değişiklik
ai-end-tokens = { $tokens } token
ai-end-cost = ${ $cost }
ai-open-settings = YZ ayarlarını aç
ai-undo-all = Asistanın yaptığı her şeyi geri al
ai-undo-all-tip = Bu görevin tüm işlerini en yeniden başlayarak geri al (geçmişten geri alma).
ai-undoing = Geri alınıyor…
ai-save-recipe = Tarif olarak kaydet
ai-save-recipe-tip = Bu adımları, daha sonra model olmadan yeniden çalıştırabileceğiniz bir TOML tarifi olarak kaydedin (aynı önizleme ve onay).
ai-recipe-name-hint = Tarif adı
ai-saving = Kaydediliyor…
ai-pending-plans = { $count } plan onayınızı bekliyor
ai-approve-all = Tümünü onayla
ai-mention-selection = seçim ({ $count } öğe)
ai-mention-folder = bu klasör: { $name }
ai-mention-tab = sekme: { $name }
ai-composer-busy = Asistan çalışıyor…  (Esc ile durdurun)
ai-composer-hint = Bir soru sorun veya görev verin…   / komutlar · @ bahsetme
ai-composer-keys = Göndermek için Enter · yeni satır için Shift+Enter
ai-stop = Durdur
ai-send = Gönder
ai-smart-chip = Akıllı arama
ai-drop-mention = { $count } öğeden bahset
ai-use-tool = `{ $name }` aracını kullanarak

## Eğik çizgi komutları

ai-slash-organize = Düzenli bir klasör yapısı planla ve dosyaları içine taşı
ai-slash-dedupe = Yinelenen dosyaları bul ve fazla kopyaları Çöp Kutusu'na taşımayı öner
ai-slash-rename = Tutarlı adlar öner (tek bir toplu yeniden adlandırma)
ai-slash-explain = Bu klasörün içeriğini açıkla
ai-slash-find = Bir tarife uyan dosyaları bul

## Araçlar menüsü

ai-tools = Araçlar
ai-tools-connected = Araçlar · { $count } bağlı
ai-tools-tip = Asistanın kullanabileceği araçlar: Soyuz'un kendi araçları ve bağlı MCP sunucularınınkiler
ai-tools-builtin = { $count } yerleşik araç (dosyalar, arama, medya, işler, arayüz) · risk sınırı: { $risk ->
    [read] okuma
    [write] yazma
   *[destructive] yıkıcı
}
ai-tools-caption-connected = Bağlı araçlar (MCP sunucuları)
ai-tools-none = Dış MCP sunucusu yok. Ayarlar → Yapay zekâ asistanı → Bağlantılar bölümünden ekleyebilirsiniz.
ai-tools-count = { $count } araç
ai-tools-disabled = devre dışı
ai-tools-connecting = bağlanıyor…
ai-tools-not-connected = bağlı değil
ai-risk-read = okuma
ai-risk-write = yazma
ai-risk-destructive = yıkıcı
ai-tools-gate-ceiling = asistanın risk sınırının üzerinde
ai-tools-gate-free = sormadan çalışır
ai-tools-gate-ask = onay ister
ai-tools-risk-line = Risk: { $risk } · { $gate }
ai-manage-connections = Bağlantıları yönet…

## Plan kartları

ai-card-items = { $count } öğe
ai-card-entries = { $count } girdi
ai-card-conflicts = { $count } çakışma
ai-card-state-pending = Sizi bekliyor
ai-card-state-auto = Otomatik onaylandı
ai-card-state-approved = Onaylandı
ai-card-state-running = Çalışıyor
ai-card-state-done = Tamamlandı
ai-card-state-failed = Başarısız
ai-card-state-rejected = Reddedildi
ai-card-state-edited = Düzenlemenizle değiştirildi
ai-card-state-preview = Yalnızca önizleme
ai-card-names-empty = adlar boş olamaz
ai-card-invalid-json = geçerli bir JSON değil: { $error }
ai-card-external = Dış istemci
ai-card-client-tip = Bu değişikliği isteyen MCP istemcisi. İsteği güvenilmeyen veridir.
ai-card-via = { $transport } üzerinden
ai-card-left = { $time } kaldı
ai-card-timeout-tip = Zamanında yanıt verilmezse istek reddedilir (Ayarlar → Yapay zekâ asistanı → Bağlantılar).
ai-card-destructive = Yıkıcı: öğeler bu klasörden çıkar (Çöp Kutusu'ndan geri yüklenebilir; kalıcı silme geri alınamaz).
ai-card-external-warning = Soyuz dışındaki bir YZ istemcisi tarafından istendi. Öğeleri kontrol edin: ne istek ne de dosya adları kendi başına bir şeyi onaylayabilir.
ai-card-tainted = Asistan dosya içeriklerini okuduktan hemen sonra önerildi. Dosyaların içindeki metinler onu yönlendirmeye çalışabilir; onaylamadan önce bu planı kontrol edin.
ai-card-blocked = Çakışmalar nedeniyle engellendi (aşağıda işaretli öğelere bakın). Düzenleyin veya reddedin.
ai-card-use-edit = Bu değişiklikleri kullan
ai-card-cancel-edit = Düzenlemeyi iptal et
ai-card-approve = Onayla
ai-card-blocked-tip = Bu planda engelleyici çakışmalar var
ai-card-edit = Düzenle
ai-card-reject = Reddet
ai-card-approve-kind = Tüm “{ $tool }” planlarını onayla
ai-card-approve-kind-tip = Bu planı ve bu sohbette aynı türden sonraki tüm planları onayla (dosya içerikleri okunduktan hemen sonra önerilen planlar hariç).
ai-card-approve-kind-client = { $client } istemcisinin tüm “{ $tool }” isteklerini onayla
ai-card-approve-kind-client-tip = Bu planı ve { $client } istemcisinden gelecek tüm “{ $tool }” isteklerini Soyuz yeniden başlayana kadar onayla. Her biri yine burada görebileceğiniz, geri alınabilir bir iş olarak çalışır.
ai-card-edit-names-hint = Yeni adları düzenleyin, ardından planı bunlarla yeniden oluşturun.
ai-card-edit-json-hint = Araç bağımsız değişkenlerini (JSON) düzenleyin. Plan yeni bir deneme çalıştırmasıyla yeniden oluşturulur.

## Plan önizlemeleri

ai-diff-added = eklendi
ai-diff-moved = taşındı
ai-diff-renamed = yeniden adlandırıldı
ai-diff-deleted = silindi
ai-diff-changed = değiştirildi
ai-diff-reveal-tip =
    { $path }
    Bölmede göstermek için tıklayın
ai-diff-more = … { $count } tane daha
ai-diff-before = Önce
ai-diff-after = Sonra
ai-diff-current-name = Geçerli ad
ai-diff-new-name = Yeni ad

## Dış istekler (MCP onay alanı)

ai-mcp-title = Dış istekler
ai-mcp-clients = MCP istemcileri
ai-mcp-waiting = { $count } istek sizi bekliyor
ai-mcp-clear = Temizle
ai-mcp-clear-tip = Tamamlanan istekleri gizle
ai-mcp-undo-client = { $client } istemcisinin yaptığı her şeyi geri al
ai-mcp-undo-client-tip = { $client } istemcisinin bu oturumda çalıştırdığı { $count } işlemi en yeniden başlayarak geri al (her biri bir geri alma işi olarak).
ai-mcp-timed-out = Zaman aşımı
ai-mcp-cancelled-by-client = İstemci tarafından iptal edildi
ai-mcp-job-gone = İş, etkinlik listesinden kaldırılmış
ai-mcp-job-done = Tamamlandı: { $ok } öğe, { $skipped } atlandı
ai-mcp-job-errors = Hatalarla tamamlandı: { $ok } başarılı, { $failed } başarısız
ai-mcp-job-failed = Başarısız: { $error }
ai-mcp-job-cancelled = İptal edildi
ai-mcp-new-client = Yeni MCP istemcisi: “{ $name }”
ai-mcp-new-client-body = { $transport } üzerinden bağlandı; Ayarlar → Yapay zekâ asistanı → Bağlantılar bölümünden onaylayana kadar yalnızca okuyabilir.
ai-mcp-pipe-unavailable = kullanılamıyor: { $error }
ai-mcp-http-failed = 127.0.0.1:{ $port } dinlenemiyor: { $error }

## Tarifler sayfası

ai-recipes-intro = Tarifler, kaydedilen adımları geçerli klasörde yeni planlarla yeniden oynatır; model gerekmez, onaylar aynıdır.
ai-recipes-empty-title = Henüz tarif yok
ai-recipes-empty-body = Asistan bir görevi bitirdikten sonra adımlarını saklamak için “Tarif olarak kaydet”i seçin.
ai-recipe-delete-tip = Bu tarifi sil
ai-recipe-run = Burada çalıştır
ai-recipe-run-tip = Her adımı geçerli klasörde yeniden planla; her plan onayınızı bekler.
ai-recipe-unknown-tool = bilinmeyen araç: “{ $tool }”
ai-recipe-read-tool = “{ $tool }” bir okuma aracı; tarifler yalnızca değişiklik içerir

## Denetim günlüğü sayfası

ai-audit-filter-hint = Metne göre filtrele
ai-audit-source = Kaynak
ai-audit-session = Oturum
ai-audit-model = Model
ai-audit-chip-all = { $label }: tümü
ai-audit-chip = { $label }: { $value }
ai-audit-all = Tümü
ai-audit-reload = Yeniden yükle
ai-audit-count = { $total } olaydan { $shown } tanesi · { $path }
ai-audit-job = iş { $id }
ai-audit-badge-assistant = Asistan
ai-audit-badge-recipe = Tarif
ai-audit-source-assistant = Asistan
ai-audit-source-recipe = Asistan (tarif)

## Akıllı arama

ai-smart-title = Akıllı arama
ai-smart-hint = ör. photos from last summer larger than 5 MB (İngilizce yazın)
ai-smart-empty = Yazdıkça filtreler burada görünür.
ai-smart-query = Sorgu
ai-smart-search = Ara
ai-smart-ask = Asistana sor
ai-smart-ask-tip = Sorguyu modelin oluşturmasına ve en iyi eşleşmeleri seçmesine izin verin
ai-smart-remove-tip = Bu filtreyi kaldır
ai-smart-ask-shown = Akıllı arama: { $text }
ai-smart-unknown-kind = bilinmeyen tür: “{ $kind }” (image, video, audio, document, archive, code, folder deneyin)
ai-smart-bad-size = geçersiz boyut: “{ $value }” (ör. >5MB)
ai-smart-bad-date = geçersiz tarih: “{ $value }” (ör. 2026-06..2026-08)

## Sohbet balonları, bildirimler

ai-user-summarize-selection = Seçili dosyaları özetle
ai-user-summarize-folder = Bu klasörü özetle
ai-user-find-related = Seçili öğelerle ilgili dosyaları bul
ai-user-suggest-names = { $count } dosya için ad öner
ai-user-run-recipe = “{ $name }” tarifini çalıştır. Konum: { $folder }
ai-notice-select-files = Ad önerilecek dosyaları seçin.
ai-notice-busy-previous = Asistan hâlâ önceki görev üzerinde çalışıyor. Önce onu durdurun (Esc).
ai-notice-disabled = YZ özellikleri Ayarlar → Yapay zekâ asistanı bölümünde kapalı.
ai-notice-busy = Asistan meşgul; önce onu durdurun.
ai-notice-cancelled = Başlamadan iptal edildi.
ai-notice-compacted = Sohbeti küçük tutmak için { $count } eski araç sonucu özetlendi.
ai-notice-budget = “{ $profile }” aylık bütçesinin (${ $cap }) % { $percent } kadarını kullandı.
ai-notice-edit-invalid = Düzenlenen plan geçersiz: { $error }
ai-notice-needs-approval = Bu plan, asistan dosya içeriklerini okuduktan hemen sonra geldiği için açıkça onaylamanız gerekiyor.
ai-toast-open-folder = Tarifi çalıştırmak için bir klasör açın
ai-saved-as = Kaydedildi: { $file }
ai-save-failed = Kaydedilemedi: { $error }
ai-undo-done = { $count } işlem geri alındı.
ai-undo-partial = { $count } işlem geri alındı; { $failed } işlem geri alınamadı: { $errors }
ai-undo-client-done = { $client } istemcisinin { $count } işlemi geri alındı.
ai-undo-client-partial = { $client } istemcisinin { $count } işlemi geri alındı; { $failed } işlem geri alınamadı: { $errors }
ai-undo-entry-unavailable = { $id } numaralı geçmiş kaydı şu anda geri alınamıyor
ai-undo-entry-failed = { $id } numaralı geçmiş kaydı: { $outcome }
ai-settings-word = Ayarlar

## Görev sonu

ai-finish-done = Tamamlandı
ai-finish-cancelled = Durduruldu
ai-finish-step-limit = Durduruldu: adım sınırına ulaşıldı
ai-finish-token-budget = Durduruldu: bu görevin token bütçesi tükendi
ai-finish-time-limit = Durduruldu: süre sınırına ulaşıldı
ai-finish-monthly-cap = Durduruldu: aylık bütçe sınırına ulaşıldı
ai-finish-error = Hata: { $error }
ai-error-disabled = YZ özellikleri kapalı (Ayarlar → Yapay zekâ asistanı).
ai-error-no-provider = Henüz bir YZ sağlayıcısı yapılandırılmadı. Ayarlar → Yapay zekâ asistanı bölümünden ekleyin.
ai-error-no-model-id = “{ $name }” profilinde model kimliği yok.

## Araç satırları ve plan sonuçları

ai-tool-attach-detail = { $texts } metin alıntısı, { $pics } resim
ai-tool-attach-brief = { $count } dosyaya bakıldı
ai-tool-read-limit = okuma aracı sınırına ulaşıldı
ai-brief-entries = { $count } girdi
ai-brief-matches = { $count } eşleşme
ai-brief-groups = { $count } yinelenen grubu
ai-brief-items = { $count } öğe
ai-brief-folders = { $count } klasör
ai-brief-tabs = { $count } sekme
ai-brief-lines = { $count } satır okundu
ai-brief-checksum = sağlama toplamı hesaplandı
ai-brief-hidden = { $count } öğe klasör izinleri nedeniyle gizlendi
ai-brief-ok = tamam
ai-brief-args-items = { $count } öğe
ai-exec-blocked = Çakışmalar nedeniyle engellendi
ai-exec-done = Tamamlandı
ai-exec-server-error = Sunucu bir hata bildirdi
ai-outcome-done = tamamlandı: { $ok } öğe, { $skipped } atlandı
ai-outcome-errors = hatalarla tamamlandı: { $ok } başarılı, { $failed } başarısız
ai-outcome-errors-first = hatalarla tamamlandı: { $ok } başarılı, { $failed } başarısız (ilki: { $path }: { $error })
ai-outcome-failed = başarısız: { $error }
ai-outcome-cancelled = iptal edildi
ai-outcome-timed-out = hâlâ çalışıyor (süre sınırına ulaşıldı)
ai-note-tags-added = { $count } öğeye etiket eklendi
ai-note-tags-removed = { $count } öğeden etiket kaldırıldı
ai-note-undo-started = { $id } numaralı geçmiş kaydının geri alınması başladı
ai-note-command-started = “{ $name }” özel komutu başlatıldı
ai-note-nothing = yapılacak bir şey yok
ai-recipe-step = Adım { $step }/{ $total }: { $tool } { $args }
ai-recipe-step-rejected = Adım { $step } reddedildi; tarif durdu.
ai-recipe-step-failed = Adım { $step } tamamlanmadı; tarif durdu.
ai-recipe-step-error = adım { $step } ({ $tool }): { $error }

## Dosya görünümlerindeki canlı vurgular

ai-touch-reading = Asistan buna baktı
ai-touch-planned = Onayınızı bekleyen bir planda
ai-touch-changing = Bir asistan işi tarafından değiştiriliyor
ai-touch-done = Asistan tarafından değiştirildi
