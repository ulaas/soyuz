### Soyuz arayüz metinleri (tr-TR): remote
### Anahtarlar en-US/remote.ftl ile birebir aynı olmalı.

## Bildirimler ve istemler

remote-sign-in-failed = Oturum açılamadı. Kullanıcı adını ve parolayı denetleyin.
remote-disconnected = { $server } bağlantısı kesildi
remote-downloading = { $name } indiriliyor…
remote-downloading-body = İndirme bitince açılacak.
remote-download-failed = { $name } indirilemedi
remote-copied = { $count } uzak öğe kopyalandı
remote-cut = { $count } uzak öğe kesildi
remote-clipboard-body = Ctrl+V ile herhangi bir klasöre veya sunucuya yapıştırın.
remote-nothing-to-paste = Yapıştırılacak bir şey yok
remote-nothing-to-paste-body = Önce dosyaları kopyalayın veya kesin (Ctrl+C / Ctrl+X).
remote-no-other-folder = Diğer bölmede klasör yok
remote-delete-title = Sunucudan silinsin mi?
remote-delete-message = { $count ->
    [one] “{ $name }” öğesi { $server } sunucusundan kalıcı olarak silinecek. Sunucularda Çöp Kutusu olmadığından bu işlem geri alınamaz.
   *[other] { $count } öğe { $server } sunucusundan kalıcı olarak silinecek. Sunucularda Çöp Kutusu olmadığından bu işlem geri alınamaz.
}
remote-new-folder-name = Yeni klasör
remote-unknown-time = bilinmiyor
remote-props-folder = Klasör · değiştirilme: { $when }
remote-props-file = { $size } · değiştirilme: { $when }
remote-unavailable = { $what ->
    [new-file] Uzak konumlarda yeni dosya oluşturulamaz
    [duplicate] Çoğaltma uzak konumlarda kullanılamaz
    [links] Bağlantılar uzak konumlarda kullanılamaz
    [terminal] Terminaller uzak konumlarda kullanılamaz
    [reveal] Sistem dosya yöneticisi uzak konumlarda kullanılamaz
   *[other] Bu komut uzak konumlarda kullanılamaz
}

## Kenar çubuğu (Ağ bölümü)

remote-network = Ağ
remote-state-connected = Bağlı
remote-state-connecting = Bağlanıyor…
remote-state-failed = Başarısız: { $error }
remote-state-idle = Bağlı değil
remote-open-new-tab = Yeni sekmede aç
remote-edit-connection = Bağlantıyı düzenle…
remote-disconnect = Bağlantıyı kes
remote-copy-address = Adresi kopyala
remote-remove = Kenar çubuğundan kaldır
remote-connect = Sunucuya bağlan…
remote-connect-tip = SFTP, WebDAV, FTP / FTPS veya S3 uyumlu depolama

## Bağlan iletişim kutusu

remote-dialog-connect-title = Sunucuya bağlan
remote-dialog-edit-title = Bağlantıyı düzenle
remote-protocol = Protokol
remote-bucket = Kova (bucket)
remote-server = Sunucu
remote-region = Bölge
remote-port = Bağlantı noktası
remote-endpoint = Uç nokta
remote-sign-in-with = Oturum açma yöntemi
remote-access-key = Erişim anahtarı
remote-user = Kullanıcı
remote-user-hint = kullanıcı adı
remote-secret-key = Gizli anahtar
remote-key-passphrase = Anahtar parolası
remote-key-passphrase-hint = anahtarın parolası varsa
remote-password = Parola
remote-secret-unchanged = değişmedi
remote-key-file-label = Özel anahtar dosyası
remote-agent-note = Windows OpenSSH aracısını (veya Pageant'ı) kullanır. Anahtarları ssh-add ile ekleyin.
remote-prefix = Ön ek (isteğe bağlı)
remote-folder = Klasör (isteğe bağlı)
remote-save = Kenar çubuğuna kaydet (Ağ)
remote-name-hint = Ad (isteğe bağlı)
remote-remember = Parolayı hatırla (Windows Kimlik Bilgileri Yöneticisi)
remote-connect-button = Bağlan
remote-auth-password = Parola
remote-auth-key-file = Anahtar dosyası
remote-auth-agent = SSH aracısı
remote-auth-access-keys = Erişim anahtarları
remote-auth-public = Herkese açık
remote-auth-anonymous = Anonim
remote-error-bucket = Kova adını girin.
remote-error-server = Sunucu adını girin.
remote-error-port = “{ $port }” geçerli bir bağlantı noktası değil.
remote-error-sftp-user = SFTP için kullanıcı adı gerekir.

## Ana bilgisayar anahtarı istemi

remote-hostkey-changed-title = Sunucu anahtarı değişti!
remote-hostkey-trust-title = Bu sunucuya güvenilsin mi?
remote-hostkey-changed-body = { $host }, daha önce güvendiğiniz anahtardan farklı bir anahtar sundu. Bu, sunucunun yeniden kurulduğu ya da birinin bağlantıyı dinlediği anlamına gelebilir.
remote-hostkey-first-body = { $host } ile ilk kez bağlantı kuruluyor. Güvenmeden önce parmak izinin sunucununkiyle eşleştiğini denetleyin.
remote-hostkey-previous = Daha önce güvenilen
remote-hostkey-trust-new = Yeni anahtara güven
remote-hostkey-trust = Güven ve bağlan

## Sunucuda yeni klasör / yeniden adlandırma

remote-new-folder-title = Yeni klasör
remote-name = Ad
remote-create = Oluştur
remote-error-name = Eğik çizgi içermeyen bir ad girin.

## Linux (data)

remote-secret-not-saved = Parola kaydedilmedi
remote-smb-unsupported = SMB paylaşımları henüz doğrudan açılamıyor. Paylaşımı masaüstünüzle bağlayın (örneğin `gio mount smb://sunucu/paylasim`), sonra Aygıtlar'dan veya /run/user altındaki klasöründen açın.
