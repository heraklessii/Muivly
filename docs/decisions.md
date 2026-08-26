# Decisions

Amaç: "Neden X değil de Y yaptık" sorusunun cevabı burada. Yeni bir mimari
öneri gelmeden veya mevcut bir yaklaşımı sorgulamadan önce oku — daha önce
elenmiş bir yolu tekrar önermemek/tartışmamak için.

Format: her karar kısa. Tarih, karar, gerekçe, alternatif (varsa neden elendi).
Eskimiş/geçersiz olan kararı SİLME, "GEÇERSİZ:" ile işaretle ve neden
değiştiğini yaz — geçmiş bağlamı kaybetmemek için.

---

## 2026-08-26 — Render motoru: wgpu/native D3D11, Tauri WebView değil

**Karar:** Wallpaper render'ı Tauri'nin WebView'i (WRY) üzerinden değil,
bağımsız bir Rust binary içinde native D3D11/wgpu ile yapılacak.

**Gerekçe:** Hedef kitle düşük donanım kullanıcıları. WebView/CEF tabanlı
render (Electron veya WE'nin bazı web wallpaper'ları gibi) sürekli ekstra
RAM yer. Native render bu maliyeti ortadan kaldırıyor.

**Alternatif (elendi):** Tüm uygulamayı Tauri WebView içinde canvas/video
tag ile render etmek. Daha basit olurdu ama RAM hedefiyle çelişiyor.

---

## 2026-08-26 — İki process ayrımı: core (native) + ui (Tauri)

**Karar:** Wallpaper motoru ve ayar paneli tamamen ayrı process'ler, IPC
(named pipe) ile haberleşir.

**Gerekçe:** UI kapatılsa/açılsa bile wallpaper render'ının bellek profili
sabit kalsın istiyoruz. WebView maliyeti sadece ayarlar açıkken ödensin.

**Alternatif (elendi):** Tek process içinde her şeyi yönetmek. Daha az IPC
karmaşıklığı olurdu ama UI açıkken RAM artışı wallpaper'ı da etkiler,
"hafiflik" iddiasını zayıflatır.

---

## 2026-08-26 — Video decode: Media Foundation + D3D11VA, ffmpeg değil

**Karar:** Video decode tamamen Windows Media Foundation üzerinden,
hardware-accelerated (D3D11VA). ffmpeg/libav'a decode için bağımlılık yok.

**Gerekçe:** Zero-copy pipeline (GPU'da decode edilen frame CPU'ya
dönmeden direkt texture olarak kullanılır) hem RAM hem CPU tasarrufu
sağlıyor. ffmpeg genelde CPU decode'a düşer veya ekstra bağımlılık/binary
boyutu getirir.

**Alternatif (kısmi/olası istisna):** ffmpeg sadece container demux için
kullanılabilir (MF'in desteklemediği formatlarda), decode adımı yine
D3D11VA'da kalır. Bu tamamen kapanmış bir kapı değil, gerekirse tekrar
değerlendirilebilir.

---

## 2026-08-26 — Multi-monitor: paylaşılan texture, monitör başına decode değil

**Karar:** Aynı video birden fazla monitörde oynuyorsa tek decode yapılır,
`IDXGIResource` ile texture tüm WorkerW instance'larına paylaşılır.

**Gerekçe:** Monitör başına ayrı decode, kaynak kullanımını monitör
sayısıyla doğru orantılı büyütür — hafiflik hedefiyle doğrudan çelişir.

---

## 2026-08-26 — Hedef kitle: düşük donanım kullanıcıları (ilk faz)

**Karar:** İlk pazarlama/ürün odağı "eski PC / entegre GPU / 8GB RAM altı"
kullanıcı kitlesi. Geliştirici/açık kaynak ve "sade video kullanıcısı"
kitleleri ikinci öncelik.

**Gerekçe:** WE'nin bu kitlede bilinen zayıflığı var (RAM/CPU şikayetleri).
Ölçülebilir, kanıtlanabilir bir fark burada daha kolay gösterilebilir
(bkz. worklog'da ileride eklenecek benchmark girdileri).

**Not:** Bu karar tüm mimari önceliklerini etkiler — "çalışır" yeterli
değil, "düşük donanımda akıcı çalışır" bar'ı esas alınmalı.
---

## 2026-08-26 — Wallpaper, çıkışın sahibi olan adapter'da çalışır

**Karar:** Hibrit sistemde (iGPU + dGPU) render, monitörün fiziksel olarak
bağlı olduğu adapter'da yapılır. `IDXGIFactory6::EnumAdapterByGpuPreference`
`MINIMUM_POWER` ile sıralanır, `IDXGIAdapter::EnumOutputs` ile eşleme çıkarılır.

**Gerekçe:** Laptop'ta dGPU'yu wallpaper için uyanık tutmak pil ömrünü yakar.
Ayrıca çıkışlar iGPU'daysa dGPU'da render cross-adapter kopya doğurur ve
zero-copy kuralını bozar.

**Alternatif (elendi):** `HIGH_PERFORMANCE` ile en güçlü GPU'yu seçmek.
Oyun mantığı; arka planda sürekli çalışan bir wallpaper için yanlış.

---

## 2026-08-26 — Cross-adapter: adapter başına bir decode

**Karar:** "Monitör başına ayrı decode YASAK" kuralı duruyor, ama monitörler
farklı GPU'lara bağlıysa **adapter başına bir decode** yapılır. Bu, kuralın
yazılı istisnası.

**Gerekçe:** Cross-adapter paylaşılan texture
(`D3D11_RESOURCE_MISC_SHARED_CROSS_ADAPTER`) sistem belleği üzerinden kopya
gerektiriyor — zero-copy kuralını doğrudan bozar. İki adapter'da iki decode,
her frame'i RAM'e indirip geri yüklemekten ucuz.

**Kapsam:** Tek adapter durumunda (yaygın hâl) hiçbir şey değişmez: tek decode,
`IDXGIResource` ile paylaşılan texture.

---

## 2026-08-26 — HW decode yoksa: statik ilk frame

**Karar:** Video codec'inin donanım decode desteği yoksa video oynatılmaz;
ilk frame statik gösterilir ve UI'da neden bildirilir.

**Gerekçe:** CPU decode yasak (RAM/CPU bütçesi). Sessizce hiçbir şey
göstermemek kullanıcıya "bozuk" hissi verir; statik frame en azından seçilen
wallpaper'ı gösterir ve sorunun ne olduğu UI'dan okunur.

**Alternatif (elendi):** Wallpaper'ı hiç yüklememek. Daha net ama kullanıcı
neyi seçtiğini göremiyor.

---

## 2026-08-26 — `caps/` ayrı modül

**Karar:** GPU capability detection `wallpaper-core/src/caps/` altında ayrı
bir modül. Core başlangıcında bir kez probe edilir, `Arc<GpuProfile>` olarak
decoder/power/compositor'a geçirilir.

**Gerekçe:** Hem `decoder/` (codec desteği) hem `power/` (fps politikası)
bu bilgiye ihtiyaç duyuyor. `power/` altına koymak `decoder → power`
bağımlılığı doğururdu; bu ters yönlü bir bağımlılık.

**Detay:** `docs/gpu_capability.md`

---

## 2026-08-26 — Arayüz teması: ortak Mui tasarım sistemi

**Karar:** wallpaper-ui, Muita ve Muitoon ile aynı paleti ve fontu kullanır:
teal `#2dd4bf` vurgu, `#0f1115` zemin, Outfit variable font (yerele gömülü).
Jeton isimlendirmesi Muita'nınkiyle aynı (`--bg-panel`, `--text`, ...).

**Gerekçe:** Üçü de İlker'in ürünü; ortak kimlik. Muita bir masaüstü
uygulaması olduğu için jeton şeması Muivly'ye daha yakın.

**Detay:** `docs/design_system.md`

---

## 2026-08-26 — Açık kaynak, Apache-2.0

**Karar:** Muivly `heraklessii/Muivly` altında açık kaynak, lisans
**Apache-2.0**. Dağıtım GitHub Releases üzerinden (portable zip + ileride
installer).

**Gerekçe:** İlker ileride Steam'de yayınlama ihtimalini açık tutmak istiyor.
Bu GPL'i eliyor: Steamworks SDK tescilli, GPL koduna linklenemez; ayrıca
dışarıdan gelen GPL katkılar ticari bir sürümü bloklardı. Apache-2.0 izin
verici, §5 sayesinde katkılar otomatik aynı lisansla geliyor (katkıcılardan
tek tek izin istemeye gerek kalmıyor) ve MIT'te olmayan patent koruması var.

**Alternatifler (elendi):**
- **GPL-3.0**: kapalı fork'u engellerdi ama Steam yolunu kapatıyor.
- **MIT**: Apache ile aynı serbestlik, patent koruması ve açık katkı şartı yok.
- **MPL-2.0**: dosya bazlı copyleft; Steam için Apache'ten bir avantajı yok,
  karşılığında karmaşıklık getiriyor.

**Not:** İlker telif sahibi olduğu için kendi kodunu istediği lisansla ayrıca
dağıtabilir; bu lisans dışarıdan gelen katkılar için önemli.

---

## 2026-08-26 — WorkerW: önce Progman'ın çocuğuna bak, sonra kardeşe

**Karar:** Wallpaper katmanı şu sırayla aranır: (1) Progman'ın `WorkerW`
çocuğu, (2) klasik "SHELLDLL_DefView sahibinin arkasındaki üst seviye
WorkerW", (3) Progman'ın kendisi. Seçilen aday **masaüstü boyutuna karşı
doğrulanır** (görünür + en az 640x480).

**Gerekçe:** Windows 11'de (bu makinede ölçüldü) görünür WorkerW,
Progman'ın çocuğu. Klasik algoritma burada görünmez 166x47 stub'lardan
birini buluyor ve hata **sessiz**: pencere oluşuyor, render başarılı
dönüyor, ekranda hiçbir şey çıkmıyor. Doğrulama adımı bu sessiz hatayı
gürültülü hâle getiriyor.

**Not:** `muivly-core --diag` masaüstü pencere ağacını döküyor. Bir kullanıcı
"wallpaper görünmüyor" derse ilk istenecek şey bu.

---

## 2026-08-26 — Occlusion: tüm pencereleri tara, foreground yetmez

**Karar:** Bir monitörün kapalı olup olmadığı, tüm üst seviye pencereler
taranarak belirlenir (250ms cache). Minimize ve **cloaked** pencereler
elenir. DXGI'ın `DXGI_STATUS_OCCLUDED` cevabı ikincil sinyal olarak kalır.

**Gerekçe:** İki alternatif de tek başına yetersiz kaldı:
- `DXGI_STATUS_OCCLUDED` wallpaper child pencereleri için güvenilir
  tetiklenmiyor.
- `GetForegroundWindow` tek pencere döndürüyor; çoklu monitörde ikinci
  ekranı kaplayan pencere foreground değilse o monitör boşa render ediyor.

**Cloaked kontrolü şart:** askıya alınmış UWP pencereleri
`IsWindowVisible=true` ve tam ekran rect ile geliyor; elenmezlerse wallpaper
görünürken yanlışlıkla duruyor.

**Ölçüm:** iki monitör, 30fps — görünürken %3.12 CPU (tek çekirdek),
tamamen kapalıyken %0.00.

---

## 2026-08-26 — Decode edilen frame örneklenebilir texture'a GPU içi kopyalanır

**Karar:** Media Foundation'ın verdiği decode edilmiş texture doğrudan
örneklenmez; `CopySubresourceRegion` ile `D3D11_BIND_SHADER_RESOURCE` bind'li
kendi NV12 texture'ımıza kopyalanır ve shader onu örnekler.

**Gerekçe:** Decoder çıktısı `D3D11_BIND_DECODER` ile oluşturuluyor ve
genelde `SHADER_RESOURCE` bind'i yok — SRV oluşturulamıyor. Kopya GPU
içinde kalıyor; frame hiçbir noktada sistem belleğine inmiyor, yani
zero-copy kuralı korunuyor. "Zero-copy" burada "CPU'ya round-trip yok"
demek, "hiç kopya yok" değil.

**Alternatif (elendi):** `MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING`
ile BGRA çıktı istemek. Shader basitleşirdi ama araya bir video processor
girip fazladan GPU işi ve bellek getiriyor.

---

## 2026-08-26 — Görünürlük decode'dan ÖNCE hesaplanır

**Karar:** `Renderer::draw` önce hangi monitörlerin görünür olduğunu
belirler; hiçbiri görünmüyorsa decoder hiç çağrılmaz.

**Gerekçe:** İlk implementasyonda decode, occlusion kontrolünden önce
yapılıyordu — monitör tamamen kapalıyken bile her frame çözülüyordu. Bu,
projenin en temel vaadinin ("görünmüyorsa iş yok") sessizce ihlaliydi.
Ölçüm farkı: kapalıyken %13.65 → %0.39 CPU.

---

## 2026-08-26 — IPC protokolü: satır tabanlı metin, JSON değil

**Karar:** wallpaper-core ↔ wallpaper-ui protokolü satır başına bir UTF-8
mesaj. Komutlar: `status`, `monitors`, `set <path>`, `clear`, `fps <n>`,
`quit`. Cevaplar `ok ...` veya `err ...`.

**Gerekçe:** Mesaj kümesi altı komut. serde + serde_json core'a ~300 KB
binary ve bir bağımlılık ekliyor; karşılığında bu boyutta bir protokol için
kazanç yok. Ayrıca metin protokolü elle test edilebiliyor (PowerShell'den
pipe'a bağlanıp yazmak yeterli), bu da hata ayıklamayı kolaylaştırıyor.
Core'un bağımlılık sayısı 1'de kalıyor (`windows`).

**Ne zaman değişir:** komut sayısı bir düzineyi geçerse veya iç içe/yapısal
veri taşımak gerekirse serde'ye geçilir. UI tarafında serde zaten var —
oradaki WebView maliyeti yanında hiç kalır.

**Not:** `set <path>` argümanı satır sonuna kadar okunuyor, tırnak yok.
Windows yollarında boşluk kural, istisna değil.

---

## 2026-08-26 — X'e basmak pencereyi tepsiye küçültür

**Karar:** Ayar penceresinin kapatma düğmesi uygulamayı kapatmaz; pencereyi
gizler ve sistem tepsisinde kalır. Çıkış tepsi menüsünden.

**Gerekçe:** Motor zaten ayrı bir işlem, yani UI kapansa da duvar kağıdı
çalışmaya devam ediyor. Ama X'e basan kullanıcı uygulamanın "hâlâ orada"
olmasını bekliyor — her tepsi uygulaması böyle davranıyor. Gerçekten
kapatmak, sonra tekrar açmak için Başlat menüsüne gitmek gerektirirdi.

**Not:** Tepsi menüsündeki "Çıkış" yalnız UI'ı kapatıyor. Motoru durdurmak
ayrı bir eylem (`quit` komutu) — kullanıcı ayar panelini kapattı diye
duvar kağıdı kaybolmamalı.

---

## 2026-08-26 — Decoder'lar dosya yoluna göre anahtarlanır

**Karar:** `Renderer` decoder'ları `HashMap<PathBuf, VideoDecoder>` içinde
tutuyor. Bir monitöre video atanınca, o yol için decoder yoksa açılıyor;
kimse kullanmıyorsa düşürülüyor.

**Gerekçe:** "Aynı video birden fazla ekranda oynuyorsa decode tek sefer"
kuralı böylece kendiliğinden sağlanıyor — ayrı bir paylaşım mantığı yok,
aynı anahtar aynı decoder demek. Farklı videolar zaten farklı decode
gerektiriyor; bu da tier politikasının `allow_distinct_videos` ile
sınırladığı şey.

---

## 2026-08-26 — Kitaplık şeması frontend'in, Rust'ın değil

**Karar:** `%APPDATA%\Muivly\state.json` Rust tarafında tipsiz bir JSON blob.
Rust yalnız (1) BOM kırpıyor, (2) geçerli JSON mu diye bakıyor, (3) atomik
yazıyor (geçici dosya + rename).

**Gerekçe:** Bu dosyada motorun ihtiyaç duyduğu hiçbir şey yok — motora
duvar kağıtları pipe üzerinden söyleniyor. Şemayı Rust'ta da tanımlamak, her
UI alanı eklendiğinde eşlik eden bir Rust değişikliği demekti.

**Bozuk dosya:** Üstüne yazılmıyor. `state.corrupt-<zaman>.json` olarak kenara
alınıp UI boştan başlıyor. Aksi hâlde parse hatası → boş kitaplık → ilk kayıt
iyi dosyayı siliyordu; bu gerçekten yaşandı (BOM'lu bir dosyayla).
