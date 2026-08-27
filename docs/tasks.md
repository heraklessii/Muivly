# Tasks

Amaç: Şu an ne yapılacak, sırada ne var. worklog.md geçmişi tutar, bu dosya
geleceği tutar. Bir görev tamamlandığında buradan silinir/işaretlenir VE
mimari etkisi varsa worklog.md'ye kısa girdi eklenir.

Format: öncelik grubu altında madde. Gerekirse ilgili decisions.md/
project_overview.md bölümüne referans.

---

## Şimdi (aktif faz)

- [ ] **2026-08-27'de yazılan on üç özelliği gerçek makinede ölç.** Hepsi
      derlendi, `cargo clippy -D warnings` ve 149 test geçiyor, hiçbiri
      makinede çalıştırılmadı:
      - [ ] Hafiflet + referans kare: encoder ayarı kabul ediliyor mu
            (çıktıdaki satır), aynı klip için önce/sonra private bytes
      - [ ] Boşta durma: 5 dk sonra gerçekten duruyor mu, ilk tuşta geri
            geliyor mu, gecikme göze batıyor mu
      - [ ] Makine yükü: derleme sırasında kare hızı düşüyor mu, histerezis
            gidip gelmeyi engelliyor mu
      - [ ] Shader parametreleri: `spectrum.hlsl` derleniyor mu (`#define`
            yaklaşımı), kaydırıcı anında etki ediyor mu
      - [ ] Shadertoy çevirisi: gerçek bir `.glsl` ile — hangi yapılar
            kırılıyor, hata mesajı satır numarası doğru mu
      - [ ] Ses bantları: loopback yakalama açılıyor mu, çıkış cihazı
            değişince `stale()` yakalıyor mu, CPU'ya etkisi
      - [ ] Vurgu rengi: renk okunuyor mu, Windows uyguluyor mu, kapatınca
            eski renkler gerçekten geri geliyor mu (asıl risk burada)
      - [ ] Sürüklenme: fotoğrafta kare hızında yeniden çizim CPU'ya ne
            katıyor
      - [ ] Sahne kaydet/yükle/sil, `--benchmark` çıktısı

- [ ] **Bu oturumda yazılanları gerçek makinede ölç.** Sekiz özellik
      derlendi ve testleri geçti, hiçbiri makinede çalıştırılmadı:
      - [ ] Hibernasyon: oyun açıkken Task Manager'da private bytes
            (beklenti ~700 MB → ~50 MB), uyanma süresi göze batıyor mu
      - [ ] Hafiflet: gerçek 4K klipte süre, çıktı boyutu, sesin kalması,
            donanım encoder'ı olmayan makinede hata mesajı
      - [ ] Shader: `examples/shaders/aurora.hlsl` entegre GPU'da kaç fps,
            derleme hatasının satır numarası doğru mu
      - [ ] Sese tepki / paralaks: kapalıyken gerçekten hiç ölçüm yok mu
      - [ ] Otomasyon: saat kuralı geldiğinde geçiş, tema değişince
      - [ ] Uygulama kuralı: adı yazılan uygulama öne gelince donma
      - [ ] Bellek bütçesi: kademeler arası geçişte yeniden açılma

- [ ] **RAM: kolay kazançlar bitti, kalan mimari.** Ölçüldü (tablo
      decisions.md 2026-08-27'de): en kötü hâlde (4K, iki adapter) 790 →
      706 MB private, `MF_LOW_LATENCY` sayesinde. Havuz öznitelikleri
      denendi, hiçbir şey yapmıyor — taban belirliyorlar, tavan değil.
      Kalan ~700 MB'ın neredeyse tamamı iki decoder'ın DPB'si; 80 thread
      MF'in paylaşılan iş kuyruğu. İkisi de ayrı bir tasarım işi:
      - [ ] Asenkron `IMFSourceReaderCallback`'e geçmenin thread sayısına
            etkisini ölç (şu an `ReadSample` senkron + kendi thread'imiz)
      - [ ] Tek adapter'da iki monitör varken gerçekten tek decoder mı
            çalışıyor, ölçümle doğrula (kod öyle diyor, sayı görülmedi)

## Sırada (henüz başlanmadı)

- [ ] **Karışık oynatmayı ve bu oturumun düzeltmelerini makinede dene.**
      Hepsi derlendi, `cargo clippy -D warnings` temiz, 164 test geçiyor;
      hiçbiri makinede çalıştırılmadı:
      - [ ] Karışık oynatma: üç dört klipli bir listede sıra gerçekten
            değişiyor mu, aynı klip arka arkaya geliyor mu, ayarı açıp
            kapatınca ekrandaki klip yerinde kalıyor mu
      - [ ] Ölen çözücü: dosyayı oynarken silince/çıkarınca UI'da mesaj
            görünüyor mu, ekran o monitörde masaüstüne dönüyor mu
      - [ ] Monitör başına fps sınırı: ikinci ekran 10 fps'te takılı kare
            göstermiyor mu
      - [ ] Ayarlar grupları: her ayar hâlâ ulaşılabilir mi, arama kutusu
            gerçek pencerede sticky duruyor mu
      - [ ] Seviyeler: "Tam" güçsüz bir makinede gerçekten 60 fps'e
            çıkıyor mu ve orada ne kadar yakıyor; "En hafif" gözle
            takılıyor mu

- [ ] `MF_SOURCE_READER_DISABLE_CAMERA_PLUGINS` sonrası P010/10-bit HEVC
      yolunu gerçek dosyayla doğrula — processor'lu yeniden açma yalnız
      teoride sınandı

- [ ] **Gerçek donanımda elden geçirme.** Bu oturumda yazılanların çoğu
      derlendi ve testleri geçti ama makinede çalıştırılmadı: crossfade'in
      görüntüsü, span'in ekranlar arasında hizası, kısayolların gerçekten
      kaydolması, kabloyu çekince fps'in düşmesi, ekran takıp çıkarınca
      yeniden kurulum, sağ tık menüsünün Explorer'da görünmesi.
- [ ] `caps` sonucunun cache'lenmesi (invalidation artık var:
      `compositor/notify.rs` + yerleşim karşılaştırması)
- [ ] Paket formatına önizleme görseli yazma (okuma tarafı hazır, `preview`
      alanı manifest'te duruyor; export şu an yalnız videoyu koyuyor)
- [ ] WE ile RAM/CPU karşılaştırma benchmark videosu/görseli (pazarlama)
- [ ] winget'e ilk gönderim (manifestler `packaging/winget/`, release iş
      akışı dolduruyor — gönderim elle)

## Backlog (öncelik yok, fikir aşaması)

- [ ] Klip başı/sonu kırpma noktaları (playlist yolunda `path#start-end`
      gibi bir şey gerekiyor; protokol değişikliği)
- [ ] Linux/Mac desteği araştırması

## Bloklanmış / Karar Bekliyor

(şu an yok)

## Biten

- [x] **Seviye seçici** (`wallpaper-ui/src/levels.ts`): kare hızı, bellek
      bütçesi, hibernasyon, boşta durma, meşgul kare hızı ve iki pil ayarı
      artık tek bir dörtlü seçim — En hafif / Hafif / Dengeli / Tam.
      Kadranlar "Tek tek ayarla"nın arkasında duruyor, arama yine buluyor.
      Seviye hatırlanmıyor, değerlerden çıkarılıyor; hiçbirine uymayan bir
      set "Özel" olarak gösteriliyor. `Dengeli` bilerek motorun kendi
      varsayılanları — temiz kurulum "Özel" görünmesin diye.
- [x] Arayüz metinleri sadeleştirildi: çözücü/kare tamponu/DPB/WebView gibi
      iç terimler kullanıcıya dönük yazılardan çıkarıldı (Ayarlar, Ekranlar,
      Otomasyon, Kitaplık, Onboarding).
- [x] Ctrl+1..5 ile görünüm değiştirme, kenar çubuğunda ipucu olarak da
      yazıyor.
- [x] Erişilebilirlik: `aria-current` gezinmede, ikon düğmelerine
      `aria-label`, hata satırlarına `role="alert"`.
- [x] `resolve()` kitaplığı liste girdisi başına baştan tarıyordu — bir kez
      indeksleniyor.
- [x] Ayarlar sayfası gruplandı
: yirmi üç kart tek kolondaydı, artık beş
      grup (Oynatma / Tasarruf / Görünüm / Sistem / Durum) ve bir arama
      kutusu var. Arama gruplardan bağımsız, Türkçe harf katlaması yapıyor
      (`gorunum` → Görünüm, `KISAYOL` → Kısayollar). "Hiçbiri eşleşmedi"
      ikinci bir kart listesi tutmadan CSS `:empty` ile cevaplanıyor.
- [x] Karışık oynatma
 (`compositor/order.rs` — torba yöntemi, listeyi bir kez
      dolaşıp sırayı yeniden çekiyor; iki geçiş arasında aynı klip
      tekrarlanmıyor). Bağımlılık yok, kendi xorshift'i var.
- [x] `same_app` `.Exe` gibi karışık yazımı tanımıyordu — kullanıcının yazdığı
      uygulama kuralı sessizce hiç çalışmıyordu
- [x] Ölen çözücü (okuma thread'i hata alıp çıkınca) fark edilmiyordu: ekran
      tek karede kalıyor, döngü hiç dinlenmiyor, kullanıcıya bir şey
      söylenmiyordu
- [x] Monitör başına fps sınırı, taze kareyi düşürüp bir daha çizmiyordu
- [x] GIF kare dikdörtgeni satır sonunu aşınca bir alt satıra taşıyordu
- [x] Çizim döngüsünden kare başına `PathBuf` kopyaları kaldırıldı; oynatma
      listesi yokken `loop_counts` haritası hiç kurulmuyor
- [x] UI ekran listesini yalnız sayı değişince tazeliyordu — ekran takas
      edilince eski liste kalıyordu

- [x] Hafiflet'te referans kare sayısı ve GOP (`ICodecAPI`)
- [x] Kitaplıkta "ekranından büyük" uyarısı ve Hafiflet önerisi
- [x] Boşta durma (`GetLastInputInfo`) ve Windows hareket azaltma ayarı
- [x] Makine meşgulken düşük kare hızı (`GetSystemTimes`, histerezisli)
- [x] Shader parametreleri (`// param`) ve UI kaydırıcıları
- [x] Shadertoy `.glsl`/`.frag` içe aktarma (satır satır çeviri)
- [x] Ses bantları + `examples/shaders/spectrum.hlsl`
- [x] Sahneler (kaydet / geri çağır / sil)
- [x] Fotoğrafta yavaş sürüklenme (Ken Burns)
- [x] Vurgu rengi duvar kağıdından (yedekli, geri alınabilir)
- [x] `muivly-core --benchmark`
- [x] "Ne kadar süre hiç çizilmedi" özeti
- [x] Görünmezken çözücüyü tamamen bırakma (hibernasyon)
- [x] "Hafiflet" — klibi bir kez ekran boyutunda yeniden yazma
- [x] Shader/prosedürel içerik (`.hlsl`), örnek dosyayla
- [x] Sese tepki ve imleç paralaksı
- [x] Saat/tema otomasyonu ve uygulama kuralları
- [x] Bellek bütçesi ayarı

- [x] Pil politikası: pilde ayrı fps, pil tasarrufunda dondurma
- [x] Ses ducking — başka uygulama ses çalarken geri çekilme
- [x] Ekran takma/çıkarma, çözünürlük değişimi, uykudan uyanma ve Explorer
      yeniden başlaması: sahnenin tamamı yeniden kuruluyor
- [x] Monitör başına fit / görünüm / kare hızı
- [x] Tek duvar kağıdını ekranlara yayma (span)
- [x] Duvar kağıtları arası crossfade
- [x] Oynatma hızı (0.25-2.0x)
- [x] Global kısayollar + Explorer sağ tık menüsü + `--set <dosya>`
- [x] `.muivly` paket formatı (zip + manifest, elle yazıldı)
- [x] AV1/HEVC için "Store eklentisi eksik" ile "GPU çözemiyor" ayrımı
- [x] winget manifestleri
- [x] `redraw` bayrağı hiç temizlenmiyordu — duran kare her tick present
      ediliyordu
- [x] İndirmede yönlendirme allowlist'i baypas ediyordu + gövde sınırsız
      belleğe alınıyordu — ikisi de kapatıldı
- [x] IPC yarışı: üç pipe instance'ı, uzak istemci reddi, istemcide yeniden
      deneme ("motor çalışmıyor" yanılması)
- [x] Tauri komutları `(async)` — bloklayan G/Ç main thread'den çıktı
- [x] `session.txt`'te tek bozuk satır tüm oturumu siliyordu
- [x] Ses görünmeyince duruyor (eskiden sessizce çözmeye devam ediyordu)
- [x] Kitaplık ızgarası ekran dışında çözücü tutmuyor (sanallaştırma yerine
      medya elemanını sökmek — gerekçe decisions.md'de)
- [x] `.gif` `<video>`'ya gidiyordu, her GIF "okunamadı" görünüyordu
- [x] Kaydırıcılar klavye/dokunmatik ile hiçbir şey göndermiyordu
- [x] Pencere tepsideyken yoklama duruyor
- [x] Kayıp dosya işareti (`file_infos` ile; `file_exists` artık kullanılmıyor)
- [x] Görünmeyen adapter için decoder açılmıyor (`sync_decoders` `enabled`
      filtresiyle zaten sağlanıyordu — doğrulandı)
- [x] Takılan oynatma — tempolama (yüksek çözünürlüklü sayaç + videonun kendi
      kare zamanı), sample ömrü, kare değişmediyse sunum yok
- [x] Decode kendi thread'inde, iki karelik kuyrukla
- [x] Ölçek kapağı (4x eşikli — ölçüm `docs/decisions.md`'de)
- [x] Görsel + animasyonlu GIF desteği (WIC)
- [x] Ses (WASAPI, varsayılan kapalı)
- [x] Parlaklık / doygunluk / bulanıklık
- [x] Windows ile başlat + motorun oturum hatırlaması
- [x] Zengin tepsi menüsü (sonraki / duraklat / ses / çıkış)
- [x] Wallpaper Engine kitaplığı içe aktarma
- [x] Keşfet — motionbgs.com entegrasyonu
- [x] Performans göstergesi (motorun kendi CPU/RAM/fps ölçümü)
- [x] Tepsiden çıkışta motorun da kapanması + oturum başına tek motor

- [x] GPU capability detection tasarımı → `docs/gpu_capability.md`
- [x] Toolchain: rustup 1.29 + rustc 1.98 + VS 2022 Build Tools (VCTools + Win11 SDK)
- [x] `caps/` modülü implementasyonu — gerçek donanımda çalışıyor
