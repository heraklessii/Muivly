# Worklog

Amaç: "Geçen sefer nereye kadar gelmiştik" sorusunun cevabı. Kronolojik,
kısa girdiler — uzun açıklama yok, decisions.md'nin tekrarı yok (sadece
"ne yapıldı", gerekçe için decisions.md'ye referans ver).

Yeni girdi eklerken: tarih, 2-4 madde, gerekirse ilgili decisions.md
kararına referans. Rutin/küçük işler için girdi açmaya gerek yok — sadece
mimari etkisi olan veya sonraki session'ın bilmesi gereken şeyler.

---

## 2026-08-26 — Proje kurulumu ve doküman yapısı

- CLAUDE.md, docs/project_overview.md, docs/decisions.md, docs/worklog.md
  oluşturuldu.
- Mimari yön netleşti: iki process (native core + Tauri UI), Media
  Foundation + D3D11VA video decode, paylaşımlı texture ile multi-monitor.
  Detay: `docs/decisions.md` (2026-08-26 girdileri).
- Hedef kitle netleşti: düşük donanım kullanıcıları (ilk faz).
- **Sonraki adım (henüz yapılmadı):** GPU capability detection tasarımı
  (kullanıcının donanımına göre otomatik fps/çözünürlük önerisi) ve/veya
  WorkerW injection + shared D3D11 texture prototipi. İkisinden hangisiyle
  başlanacağı henüz seçilmedi.
## 2026-08-26 — GPU capability detection tasarımı

- `docs/gpu_capability.md` yazıldı: probe sinyalleri, adapter sınıflandırma
  (VRAM önce, vendor tiebreak), tier→fps/ölçek politika tablosu, `GpuProfile`
  API şekli, cache/invalidation, kullanıcı override.
- Yeni mimari nokta: wallpaper **çıkışın sahibi olan adapter'da** çalışır
  (hibrit laptop'ta iGPU), dGPU tercih edilmez — pil ve cross-adapter kopya
  nedeniyle. Karar olarak kesinleşirse decisions.md'ye taşınmalı.
- Bekleyen 3 açık soru tasarım dosyasının sonunda; ikisi CLAUDE.md'deki
  kesin kuralları etkiliyor (cross-adapter decode, CPU fallback yasağı).
- **Blokaj:** Rust + MSVC/Windows SDK kurulu değil → WorkerW/D3D11 prototipi
  ve tüm `wallpaper-core` implementasyonu bekliyor.

## 2026-08-26 — caps/ modülü çalışıyor, toolchain kuruldu

- Toolchain kuruldu: rustup 1.29 / rustc 1.98, VS 2022 Build Tools
  (VCTools + Windows 11 SDK). Cargo workspace açıldı, `wallpaper-core`
  derleniyor ve koşuyor. Tek bağımlılık: `windows` 0.62.
- `caps/` implement edildi (`adapter.rs`, `decode.rs`, `system.rs`,
  `policy.rs`). Tasarım: `docs/gpu_capability.md`.
- Onaylanan 3 karar decisions.md'ye ve CLAUDE.md'ye işlendi (cross-adapter
  decode istisnası, HW decode yoksa statik frame, `caps/` ayrı modül).
- Ortak Mui teması `docs/design_system.md`'ye çıkarıldı (Muita + Muitoon'dan:
  teal #2dd4bf, zemin #0f1115, Outfit). CLAUDE.md konvansiyonlarına eklendi.
- **Gerçek donanımda iki hata yakalandı ve düzeltildi:**
  - HEVC Main10, NV12 ile sorgulanınca destekleyen GPU'da bile "yok" diyor;
    10-bit profiller P010 ile sorgulanmalı.
  - `DXGI_OUTPUT_DESC.DesktopCoordinates` DPI-sanallaştırılmış geliyor
    (%125'te 2560x1440 → 2048x1152). `SetProcessDpiAwarenessContext` +
    `DEVMODEW.dmPels*` ile gerçek piksele geçildi.
- İlker'in makinesi cross-adapter test yatağı: AMD iGPU (primary, 2560x1440
  @180Hz) + RTX 4050 (1920x1080 @144Hz), iki ayrı adapter'da birer monitör.
  Probe çıktısı tier=Mid, 30fps öneriyor.

## 2026-08-26 — Açık kaynak altyapısı

- Proje adı netleşti: **Muivly**. Repo `heraklessii/Muivly`, lisans
  **Apache-2.0** (gerekçe: Steam ihtimali — bkz. decisions.md).
- `git init` + ilk commit. LICENSE, README.md, CONTRIBUTING.md yazıldı
  (hepsi İngilizce — dışa dönük).
- `.github/`: `ci.yml` (windows-latest; fmt + clippy -D warnings + test +
  build + `--caps` duman testi), `release.yml` (`v*` tag → portable zip +
  SHA256 + taslak release), issue şablonları, dependabot.
- Shipped binary adı `muivly-core.exe` oldu (crate ve klasör adı
  `wallpaper-core` kalıyor) — kullanıcı Task Manager'da ürün adını görsün.
- CLI eklendi: `--caps`, `--version`, `--help`. `--caps` çıktısı bug report
  şablonunda zorunlu alan.
- 13 birim testi yazıldı (`caps/adapter.rs` sınıflandırma, `caps/policy.rs`
  tier kararları). Hepsi geçiyor, clippy temiz.
- CLAUDE.md'ye "Açık Kaynak" bölümü eklendi: dışa dönük dil İngilizce, CI
  yeşil olmadan iş bitmez, telemetri yok, semver.

## 2026-08-26 — Compositor çalışıyor: masaüstünde gerçek render

- `compositor/` yazıldı: `workerw.rs` (wallpaper katmanını bulma),
  `window.rs` (monitör başına child pencere), `render.rs` (adapter başına
  D3D11 cihazı + monitör başına swap chain), `shader.rs` (geçici gradient),
  `diag.rs` (masaüstü pencere ağacı dökümü).
- `power/` yazıldı: monitör kapalı mı kontrolü.
- **Windows 11'de WorkerW düzeni farklı çıktı.** Klasik algoritma
  ("SHELLDLL_DefView sahibinin kardeşi olan üst seviye WorkerW") bu makinede
  görünmez 166x47 stub'lardan birini buluyordu — pencere oluşuyor, render
  başarılı dönüyor, ekranda hiçbir şey görünmüyor. Sessiz hata.
  Gerçek düzen: WorkerW, **Progman'ın çocuğu** (4480x1440, görünür).
  Çözüm: önce Progman'ın WorkerW çocuğuna bak, sonra klasik kardeş araması,
  ve seçilen adayı masaüstü boyutuna karşı doğrula. `--diag` bu ağacı
  dökmek için eklendi — bir daha tahmin etmeye gerek kalmasın.
- **Occlusion'da iki aşama gerekti.** DXGI'ın `DXGI_STATUS_OCCLUDED` cevabı
  wallpaper child pencereleri için güvenilir tetiklenmiyor. `GetForegroundWindow`
  tek başına da yetmedi: çoklu monitörde ikinci ekranı kaplayan pencere
  foreground değilse görülmüyor, o monitör boşa render etmeye devam ediyor.
  Nihai çözüm: tüm üst seviye pencereleri tara (250ms cache ile), minimize/
  cloaked olanları ele. **Cloaked kontrolü şart** — askıya alınmış UWP
  pencereleri `IsWindowVisible=true` ve tam ekran rect ile geliyor.
- **Ölçüm (bu makine, iki monitör, 30fps):** masaüstü görünürken %3.12 CPU
  (tek çekirdek), her iki monitör kapalıyken **%0.00**, RAM ~64 MB working
  set / ~82 MB private. RAM hedefin (düşük onlarca MB) üstünde; iki ayrı
  D3D11 cihazı (adapter başına bir tane) bunun bir kısmını açıklıyor.
- Test sayısı 17. CI kapısı (fmt/clippy/test) temiz.

## 2026-08-26 — Video oynuyor: Media Foundation + D3D11VA

- `decoder/` yazıldı: `IMFSourceReader` + `IMFDXGIDeviceManager`, NV12
  çıktı, `IMFDXGIBuffer` üzerinden decode edilmiş texture alınıyor.
  Shader NV12'yi iki plane (R8 luma + R8G8 chroma) olarak örnekleyip
  BT.709 limited range ile RGB'ye çeviriyor.
- Zero-copy korunuyor: tek hareket `CopySubresourceRegion` — decoder
  çıktısı `D3D11_BIND_DECODER` ile geliyor ve örneklenemiyor, o yüzden
  `SHADER_RESOURCE` bind'li kendi texture'ımıza GPU içi blit yapılıyor.
  Frame hiçbir noktada sistem belleğine inmiyor.
- Cihaz oluşturmaya `D3D11_CREATE_DEVICE_VIDEO_SUPPORT` ve
  `ID3D10Multithread::SetMultithreadProtected` eklendi. İkincisi şart:
  MF kendi thread'lerinden bu device'a dokunuyor.
- Aspect: "cover" fit (kırp, çubuk gösterme). Monitör başına ayrı hesap —
  aynı video 16:9 ve 16:10 ekranda farklı kırpılıyor.
- **Kendi kuralımızı ihlal eden hata bulundu ve düzeltildi:** ilk hâlde
  `draw()` occlusion kontrolünden ÖNCE decode ediyordu, yani monitör
  kapalıyken bile frame çözülüyordu. Artık görünürlük önce hesaplanıyor;
  hiçbir monitör görünmüyorsa decode hiç çağrılmıyor.
- **Ölçüm (2 monitör, 2 decoder, 1080p30):** masaüstü görünürken %13.65
  CPU (tek çekirdek), tamamen kapalıyken **%0.39**.
- **Zayıf nokta: RAM ~265 MB ve 75 thread.** Hedefin çok üstünde.
  Büyük kısmı MF source reader'ın thread havuzu ve decode buffer'ları;
  bu makinede iki decoder çalıştığı için en kötü durum. Sıradaki iş bu.

## 2026-08-26 — Ayar paneli (Tauri v2) ve IPC

- `ipc/` yazıldı: named pipe sunucusu (`\.\pipe\muivly`), satır tabanlı
  metin protokolü. Komutlar: `status`, `monitors`, `set`, `clear`, `fps`,
  `quit`. serde YOK — gerekçe decisions.md'de.
- Compositor artık çalışırken komut alıyor: video değiştirme cihazı/pencereyi
  yıkmadan yapılıyor (`Renderer::set_video`), fps anında değişiyor.
- `wallpaper-ui/` kuruldu: Tauri v2 + React + Vite. Ortak Mui teması
  (`docs/design_system.md`) `src/styles.css`'e uygulandı, Outfit fontu
  Muita'dan kopyalandı (`latin` + `latin-ext`).
- **X tuşu pencereyi tepsiye küçültüyor**, uygulamayı kapatmıyor. Tepsi
  menüsü: "Muivly'yi aç" / "Çıkış". Sol tık pencereyi geri getiriyor.
  Doğrulandı: WM_CLOSE sonrası pencere gizli, işlem ayakta, motor etkilenmedi.
- İkon üretildi (koyu yuvarlak kare + teal oynat üçgeni), png + ico.
- `wallpaper-ui` kök workspace'ten `exclude` edildi ve kendi workspace'i oldu:
  CI motoru kontrol ederken Tauri'nin bağımlılık ağacını çekmesin.
- CI'a ikinci job (`ui`) eklendi: npm ci + tsc/vite build + fmt + clippy.
  release.yml artık NSIS installer da üretiyor; core exe'si `--config` ile
  resource olarak enjekte ediliyor (tauri.conf.json'a yazılırsa `cargo build`
  dosya yokken kırılıyor — build script resource'ları doğruluyor).

**Yolda çıkan iki hata:**
- Pipe sunucusu istemci kopunca 200ms uyuyordu; UI o boşlukta bağlanamıyordu.
  Artık normal kopma hata sayılmıyor (`ERROR_BROKEN_PIPE` → EOF) ve döngü
  anında yeni instance açıyor.
- UI'da `monitors` isteği başarısız olunca `catch` bloğu `status`'ü de
  siliyordu → "Motor çalışmıyor" görünüyordu. İki istek ayrıldı.
- `cargo build --release` Tauri'yi dev moduna sokuyor (`cfg(dev)`), uygulama
  `localhost:5183`'ü yüklemeye çalışıyor. Üretim derlemesi `npx tauri build`
  ile yapılmalı.

## 2026-08-26 — Kitaplık, listeler, monitör başına duvar kağıdı

**Motor:**
- `Renderer` artık monitör başına video tutuyor. Decoder'lar **dosya yoluna
  göre** anahtarlanıyor (`HashMap<PathBuf, VideoDecoder>`) — aynı videoyu
  gösteren iki monitör tek decode'u kendiliğinden paylaşıyor, kimsenin
  ayarlaması gerekmiyor. Atama değişince kullanılmayan decoder düşürülüyor.
- Oynatma listesi compositor'da: monitör başına `items` + `index`. Geçiş ya
  süreyle ya klip bitince (decoder'a `loops()` sayacı eklendi).
- Ölçekleme kipi: cover / contain / stretch. Contain'de shader kenar dışını
  siyah boyuyor — sampler clamp ettiği için yoksa kenar pikseli banda
  yayılıyordu.
- IPC genişledi: `set <monitor> <path>|<path>`, `next`, `enable`, `fit`,
  `interval`. Yollar `|` ile ayrılıyor (Windows yolunda boşluk kural, `|`
  imkânsız).

**Arayüz:**
- Kenar çubuklu kabuk: Kitaplık / Listeler / Ekranlar / Ayarlar.
- Kitaplık: ekleme (çoklu seçim), kaldırma, yeniden adlandırma, arama,
  ekrana uygulama. Küçük resimler gerçek video karesinden.
- Listeler: oluştur/sil/yeniden adlandır, sıra değiştirme, kitaplıktan ekleme.
- Ekranlar: gerçek masaüstü yerleşiminin ölçekli önizlemesi, monitör başına
  atama (tek video veya liste), aç/kapa, "Sonraki".
- Ayarlar: fps, ölçekleme, liste geçiş aralığı, motoru durdurma, veri yolu.
- Durum `%APPDATA%\Muivly\state.json`'da. Şema frontend'in; Rust yalnız
  geçerli JSON olduğunu doğrulayıp atomik yazıyor (yaz-ve-yeniden-adlandır).

**Yolda çıkan hatalar:**
- `state.json` BOM'lu yazılınca `JSON.parse` patlıyor, UI boş kitaplıkla
  başlıyor ve **ilk kayıt iyi dosyanın üstüne yazıyordu** — sessiz veri kaybı.
  Artık Rust BOM'u kırpıyor; JSON yine de bozuksa dosya
  `state.corrupt-<zaman>.json` olarak kenara alınıyor, üstüne yazılmıyor.
- Küçük resimler canvas'a çizilip `toDataURL` ile alınıyordu; asset protokolü
  farklı origin olduğu için canvas "tainted" olup atıyordu. Artık duraklatılmış
  `<video>` elemanı gösteriliyor.
- **Ekran görüntüsü yöntemi yanıltıcıydı** (bu bir uygulama hatası değil, ama
  saat kaybettirdi): `BitBlt`/`CopyFromScreen` DWM ile kompoze edilen pencereleri
  yakalayamıyor, ayrıca DPI-farkında olmayan bir işlemden alınan pencere ölçüsü
  gerçek boyutun %80'i çıkıp görüntüyü kırpıyor. Doğrusu:
  `SetProcessDpiAwarenessContext` + `PrintWindow(..., PW_RENDERFULLCONTENT)`.
- `--diag` artık her pencerenin gerçek ebeveynini yazıyor. Bir ara duvar
  kağıdının ikonların üstüne çıktığını sandım; ebeveyn çıktısı `parent=0x101e8`
  (WorkerW) göstererek yanlış alarmı bir komutta kapattı.

## 2026-08-26 — Takılan oynatma, kapanmayan motor

- **Kare hızlandırma yeniden yazıldı.** `compositor::clock::Pacer` (yüksek
  çözünürlüklü bekleme sayacı) + videonun kendi kare zamanına göre uyanma +
  kare değişmediyse `Present` etmeme. Üç bağımsız takılma kaynağı vardı;
  detay `docs/decisions.md`.
- **Decoder sample sızıntısı düzeltildi.** `IMFSample` kopyalama bitene
  kadar tutuluyor; önceden bırakılıyordu ve decoder aynı texture slot'unun
  üstüne yazabiliyordu.
- **Tepsi > Çıkış artık motoru da durduruyor** (pipe üzerinden `quit`),
  ve motor oturum başına tek örnek (adlandırılmış mutex). İkisi birlikte
  "muivly-core kapanmıyor" şikâyetinin iki ayrı sebebiydi.
- **Açılmayan dosya artık motoru düşürmüyor**; sebep `Status.error` ile
  UI'a taşınıyor (`status` yanıtına `error <message>` satırı eklendi).

## 2026-08-26 — Dokuz maddelik toplu geliştirme

Hepsi tek turda; detaylar `docs/decisions.md`.

- **Decode kendi thread'inde** (2 karelik kuyruk, `pass` numaralı kareler).
- **Ölçek kapağı** gerçekten uygulanıyor ama 4x eşikle — koşulsuz hâli
  ölçümde CPU'yu %6.6'dan %8.9'a, belleği 784 MB'dan 834+ MB'a çıkardı.
- **Görsel + animasyonlu GIF** (`decoder/still.rs`, WIC).
- **Ses** (WASAPI shared, varsayılan kapalı, görünmeyince susuyor).
- **Parlaklık / doygunluk / bulanıklık** shader'da, `visual` komutu ile.
- **Windows ile başlat** — Run anahtarı motoru gösteriyor, motor da
  `session.txt`'ten son oturumu geri yüklüyor.
- **Tepsi menüsü**: sonraki, duraklat, sesi aç/kapat, çıkış.
- **Wallpaper Engine içe aktarma** (Steam kütüphaneleri + `project.json`).
- **Keşfet** — motionbgs.com, tek host allowlist'i, kullanıcı tetikli.
- **Performans göstergesi** — motorun kendi CPU/bellek/fps ölçümü.

**Yolda çıkan hatalar:**
- Sabit tampon 40 bayta çıkınca `CreateBuffer` E_INVALIDARG verdi; D3D11
  16'nın katını şart koşuyor. Motor açılışta ölüyordu, mesaj yalnız
  "Parametre hatalı" idi.
- Ölçek kapağı `MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING` olmadan
  sessizce reddediliyordu — hesaplanıyor, raporlanıyor, uygulanmıyordu.
- `realfps` iki monitörde 60 gösteriyordu (30 fps klip için). Flip sayısı ile
  kare hızı aynı şey değil; artık sunum yapan döngü geçişleri sayılıyor.
- `safe_name` testi arka arkaya noktaları elemiyordu — kendi testi yakaladı.

## 2026-08-26 — Güvenlik, kararlılık ve ölçüm turu

Kod okunarak çıkarılan hatalar; hepsi tek turda. Gerekçeler
`docs/decisions.md`'de.

- **Keşfet indirmesinde iki açık kapatıldı.** (1) reqwest varsayılan olarak
  yönlendirme takip ediyordu, yani host allowlist'i yalnız ilk URL'i
  koruyordu — tek bir `Location` başlığı bu komutları yeniden "dosya sistemi
  bağlı genel amaçlı proxy" hâline getiriyordu. Artık her sıçrama aynı
  kontrolden geçiyor (en fazla 5). (2) Gövde `bytes()` ile tümüyle belleğe
  alınıyordu — 4K klip için birkaç yüz MB, üstelik sınırsız. Artık geldikçe
  diske yazılıyor, 1 GiB tavanı var, yarım kalan `.part` siliniyor.
- **IPC:** üç pipe instance'ı + `PIPE_REJECT_REMOTE_CLIENTS`; istemcide
  meşgul pipe için yeniden deneme. "Motor çalışmıyor" yanılması bitti.
- **Tauri komutları `(async)`** — bloklayan pipe/disk G/Ç artık main
  thread'de değil.
- **Video okuyucu processor'sız açılıyor**, yalnız gerekince (P010 ya da
  ölçek) processor'lu yeniden kuruluyor. `MFStartup` süreç başına bir kez,
  kamera eklentileri kapalı.
- **Ses görünmeyince gerçekten duruyor** (eskiden sessizce çözmeye devam
  ediyordu).
- **`session.txt`'te tek bozuk satır tüm oturumu çöpe atıyordu** (`?` döngü
  içindeydi) — boş bir satır yeterdi. Ayrıştırma `parse()`'a ayrıldı ve
  testlendi.

**UI tarafı:**
- Kitaplık ızgarası ekran dışındaki karoların medya elemanını söküyor —
  elli üstü öğede tarayıcı çözücü sınırı.
- `.gif` `<video>`'ya gidiyordu, yani motorun oynattığı her GIF kitaplıkta
  "okunamadı" karosu olarak duruyordu.
- Kaydırıcılar yalnız `mouseup` ile işleniyordu: klavyeyle oynatmak hiçbir
  şey göndermiyordu, iz dışında bırakılan sürükleme kayboluyordu.
- Pencere tepsideyken yoklama duruyor (gizli WebView zamanlayıcılarını
  çalıştırmaya devam ediyor).
- Ekran listesi artık `status`'taki ekran sayısı değiştiğinde yenileniyor.

---

## 2026-08-26 — On bir iş, tek oturum

- **Pil politikası** (`power/battery.rs`): pilde ayrı fps tavanı, pil
  tasarrufunda tamamen dondurma. Gerekçe: decisions.md.
- **Ses ducking** (`audio/duck.rs`): başka bir uygulama duyulur ses
  çalarken duvar kağıdının sesi %12'ye iniyor, 200 ms rampayla.
- **Gizli bildirim penceresi** (`compositor/notify.rs`): ekran değişimi,
  uyanma ve üç global kısayol. Yerleşim gerçekten değiştiyse bütün sahne
  (caps dahil) yeniden kuruluyor; Explorer yeniden başlarsa WorkerW da
  yeniden bulunuyor.
- **Monitör başına** fit/görünüm/kare hızı; **span** (tek duvar kağıdı bütün
  masaüstüne); **crossfade**; **oynatma hızı** 0.25-2.0x.
- **Bulunan hata:** `redraw` bayrağı hiçbir yerde temizlenmiyordu — bir kez
  set edilen hedef sonsuza dek her tick'te present ediyordu. Başarılı
  sunumdan sonra temizleniyor artık; "duran kare bedava" sözü ancak şimdi
  doğru.
- **Explorer sağ tık menüsü** (`shell.rs`) + `--set <dosya>`: pencere
  açmadan, yalnız motora bir satır.
- **`.muivly` paketi** (`pack.rs`): elle yazılmış store-only zip, bağımlılık
  yok, akışlı okuma/yazma.
- **Kodek farkındalıklı hata mesajı**: AV1/HEVC için "Store eklentisini kur"
  ile "bu GPU çözemiyor" ayrıldı.
- **winget manifestleri** (`packaging/winget/`), release iş akışında sürüm ve
  SHA256 ile dolduruluyor.
- IPC protokolü genişledi: `power`, `speed`, `fade`, `span`, `hotkeys`,
  `freeze`, `own`; status'a pil/duck/hız alanları eklendi. Oturum dosyası
  hepsini saklıyor ve eski dosyaları hâlâ okuyor.
- Test sayısı 49 → 60 (çekirdek) + 14 (UI). CI artık UI crate'inde de
  `cargo test` çalıştırıyor.

## 2026-08-27 — Ölçüm ve gerçek makinede doğrulama

- RAM ölçüldü, `MF_LOW_LATENCY` girdi: en kötü hâlde 790 → 706 MB private
  (%11). Havuz öznitelikleri denendi ve çıkarıldı. Tablo ve gerekçe
  decisions.md'de.
- Sıra bozulmasına karşı kalıcı bekçi: `read_loop` zaman damgalarını izliyor,
  sıra bozulursa bir kez uyarıyor. Dört 4K klipte tetiklenmedi.
- **Bulunan hata:** UI görünüm geçersiz kılmasını temizlerken `-1` gönderiyor
  (status satırı ve oturum dosyası da öyle yazıyor) ama `own` ayrıştırıcısı
  yalnız `-` kabul ediyordu. "Masaüstünü izlesin" düğmesi sessizce `err`
  alıyordu. Ayrıştırma `parse_overrides`'a çıkarıldı ve testlendi.
- Gerçek makinede doğrulandı: `speed`/`fade`/`power`/`span`/`freeze`/`own`
  komutları, geçersiz değerlerin reddi, geçersiz kılmanın kurulup
  temizlenmesi, dondurmanın CPU'yu %0'a indirmesi ve çözülüp devam etmesi.
- Test sayısı 60 → 65.
