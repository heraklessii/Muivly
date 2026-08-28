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

**Not (2026-08-26 güncellendi):** Pencereyi kapatmak (X) hâlâ yalnız gizliyor,
motor çalışmaya devam ediyor. Ama tepsi menüsündeki **"Çıkış" artık motoru da
durduruyor** — UI çıkmadan önce pipe üzerinden `quit` gönderiyor. Önceki
davranışta (sadece UI kapanıyordu) kullanıcının masaüstünde, Görev
Yöneticisi'nden başka hiçbir yerden erişemediği bir `muivly-core` kalıyordu;
bu arka plan servisi değil, kurtulunamayan bir işlem demek.

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

---

## 2026-08-26 — Render döngüsü kendi ızgarasında değil, videonun temposunda uyanır

**Karar:** Kare hızlandırma iki sınırın geç olanına göre yapılıyor:
(1) tier'ın hedef fps'i, (2) decoder'ın elindeki bir sonraki karenin
zamanı (`VideoDecoder::time_to_next`). Uyku `thread::sleep` ile değil,
yüksek çözünürlüklü bekleme sayacıyla (`compositor::clock::Pacer`).
Ayrıca kare değişmediyse `Present` çağrılmıyor.

**Gerekçe:** Üç ayrı takılma kaynağı vardı ve üçü de burada birleşiyordu:

- **Uyku hassasiyeti.** Windows'ta `thread::sleep` sistem sayaç adımına
  (varsayılan 15.6 ms) yuvarlanıyor. 30 fps'te 33.3 ms isteyip 46.8 ms
  uyuyorduk — her ikinci kare tam bir adım geç. Görüntüde bu doğrudan
  takılma. `timeBeginPeriod(1)` çözerdi ama sistem genelinde sayaç
  çözünürlüğünü yükseltip her işlemin pil tüketimini artırıyor; yüksek
  çözünürlüklü sayaç yalnız bizi ilgilendiriyor (Win10 1803+; daha
  eskisinde sıradan sayaca düşüyor, yani eskisinden kötü değil).
- **Izgara uyumsuzluğu.** Kendi fps ızgaramızda uyanmak, 24 fps'lik bir
  klibin bazı karelerini bir tik bazılarını iki tik göstermek demek. Kare
  kaybı yok ama göz düzensizliği görüyor — izleyici için "takılıyor".
- **Gereksiz sunum.** 30 fps video 60 fps ızgarada saniyede 30 kez aynı
  pikselleri sunuyordu. Artık kare değişmediyse çizim de flip de yok.

**Ayrıca:** Decoder, `IMFSample`'ı kopyalama yapılana kadar canlı tutuyor.
Yalnız `ID3D11Texture2D`'yi tutmak yetmiyordu — havuz slot'u sample'ın
referans sayısına bakıyor, sample bırakılınca decoder aynı slot'un üstüne
yazabiliyordu.

---

## 2026-08-26 — Oturum başına tek motor

**Karar:** `muivly-core` başlarken `Local\muivly-core` adlı bir mutex
alıyor. Alamıyorsa "already running" yazıp çıkıyor. UI de başlatmadan önce
pipe'a bakıyor.

**Gerekçe:** İkinci bir motor aynı WorkerW'a ikinci bir duvar kağıdı
çiziyor, aynı videoyu bir daha decode ediyor (projenin tüm RAM/CPU
iddiasının tersi) ve pipe adına yalnız biri cevap veriyor — sonraki `quit`
birini durdurup diğerini bırakıyor. Mutex'i kernel süreç nasıl biterse
bitsin (çökme dâhil) serbest bırakıyor; kilit dosyası bunu yapmıyor.

---

## 2026-08-26 — Açılmayan dosya motoru düşürmez

**Karar:** `VideoDecoder::open` hata verirse monitör temizleniyor, sebep
`Status.error` alanına yazılıp UI'da gösteriliyor, motor çalışmaya devam
ediyor.

**Gerekçe:** Eskiden hata `compositor::run`'dan yukarı gidip işlemi
`exit(1)` ile bitiriyordu. Donanım decoder'ı olmayan tek bir codec, taşınmış
tek bir dosya bütün ekranlardaki duvar kağıdını düşürüyordu — kullanıcı için
"Muivly çalışmayı durdurdu", oysa yapılması gereken tek şey o dosyayı
oynatmamak.

---

## 2026-08-26 — Decode kendi thread'inde, iki karelik kuyrukla

**Karar:** `IMFSourceReader::ReadSample` artık render thread'inde değil.
Okuyucu thread'i çözülmüş kareleri `sync_channel(2)` üzerinden veriyor.

**Gerekçe:** ReadSample'ın maliyeti sabit değil — keyframe, soğuk dosya,
parçalı moov kutusu ortalama karenin çok üstüne çıkıyor. Bu, render
thread'inde ödendiğinde o kare deadline'ı kaçırıyor; tempolama ne kadar iyi
olursa olsun gözle görülür bir takılma.

**Kuyruk neden 2:** Yeterince derin ki ani sıçramayı yutsun, yeterince sığ ki
kapalı bir monitörde decoder bir iki kare içinde dursun — thread dolu kanalda
`send`'de bloke oluyor, yani "görünmeyen şey çalışmaz" kuralı ayrıca
kodlanmadan kendiliğinden sağlanıyor.

**Timestamp'ler:** Klip başa sardığında sıfırlanıyor, bu yüzden her kare bir
`pass` numarası taşıyor. Tüketici `pass` değişince klip saatini sıfırlıyor —
"tekrar ilk kare" ile "çok geç kalmış kare" aksi hâlde ayırt edilemiyordu.

---

## 2026-08-26 — Ölçek kapağı yalnız kaynak 4 kat büyükse uygulanır

**Karar:** `max_scale` artık gerçekten uygulanıyor ama eşikli: kaynağın
piksel sayısı kapağın 4 katından fazlaysa Media Foundation'dan küçük kare
isteniyor, değilse dosya kendi boyutunda çözülüyor.

**Gerekçe — ölçüm:** Kapak koşulsuz uygulandığında beklenen bellek kazancı
çıkmadı, tersi oldu. Sebep: kodek zaten native boyutta çözmek zorunda; küçük
kare istemek araya bir video processor ve **ikinci bir tampon havuzu**
sokuyor. İlker'in makinesinde 4K klip / 1440p kapak ile ölçüldü:

| | CPU (tek çekirdek payı) | private bellek |
|---|---|---|
| Kapak yok | %6.6 | 784 MB |
| Kapak koşulsuz | %8.9 | 834-967 MB |

Kazanılan tek şey kare başına blit ve shader örneklemesi — koca bir havuza
değmesi için kaynağın ekrana göre absürt büyük olması gerekiyor. 8K klibin
1440p masaüstünde ölçeklenmesi mantıklı, herkesin elindeki 4K klibin değil.

**Not:** Ölçekleme `MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING`
olmadan sessizce reddediliyor — kapak bir süre hesaplanıp raporlanıp hiç
uygulanmıyordu.

---

## 2026-08-26 — Görsel ve GIF WIC ile, CPU'da

**Karar:** png/jpg/bmp/webp/gif/tiff `decoder/still.rs`'te WIC ile çözülüyor.
GIF kareleri gerektikçe bir tuvale kompoze ediliyor, hepsi baştan açılıp
bellekte tutulmuyor.

**Gerekçe:** "Decode her zaman GPU'da" kuralı video için. PNG çözen GPU yok;
duran görsel bir kez çözülüp bir kez yükleniyor ve sonra hiç maliyeti yok —
ne decode, ne çizim, ne flip. Bu bir istisna değil, Muivly'nin en hafif
duvar kağıdı. GIF kareleri baştan açılsa 1080p/60 kare ~500 MB ederdi; GIF
zaten kareyi yalnız değişen dikdörtgen olarak saklıyor.

---

## 2026-08-26 — Ses tek akış, birincil ekranı takip eder

**Karar:** WASAPI shared mode + MF ses decode, kendi thread'inde. Varsayılan
kapalı. Her monitör kapandığında (tam ekran oyun, kilit ekranı) susuyor.

**Gerekçe:** Monitör başına ses iki farklı klipte iki şarkı demek. Aynı klip
iki ekranda ise zaten tek şarkı. İkisi de tek akışa çıkıyor, o da birincil
ekranı takip ediyor. Shared mode, ses kartını ele geçirmeden diğer her şeyle
karışabilmek için.

---

## 2026-08-26 — Motor kendi oturumunu hatırlıyor

**Karar:** `%APPDATA%\Muivly\session.txt` — açık duvar kağıtları ve ayarlar.
Kitaplık değil; o frontend'in `state.json`'ında kalıyor.

**Gerekçe:** Otomatik başlatma bunu zorunlu kıldı. Açılışta motor tek başına
geliyor ve ekranında hiçbir şey olmayan bir duvar kağıdı motoru kimsenin
açık bırakacağı bir şey değil. Alternatif — UI'ı gizli başlatmak — açılışa
bir WebView'ın ~100 MB'ını yazmak demekti; projenin tam olarak reddettiği
şey.

**Run anahtarı UI'ı değil motoru gösteriyor**, HKCU'da (HKLM yönetici hakkı
isterdi, bu bir kullanıcının kendi masaüstü tercihi).

---

## 2026-08-26 — Keşfet: motionbgs.com, tek host, kullanıcı tetikli

**Karar:** UI'da bir keşfet görünümü. Rust yalnız indiriyor ve kaydediyor
(`web.rs`), HTML frontend'de `DOMParser` ile ayrıştırılıyor.

**Gerekçe:** HTML ayrıştırıcı crate'i, WebView'da hazır duran bir şeyi daha
kötü yapmak için büyük bir bağımlılık olurdu. Rust tarafında **host
allowlist** var ve bu bir formalite değil: onsuz bu komutlar, WebView'a
erişebilen her şeyin kullanabileceği, dosya sistemine bağlı genel amaçlı bir
proxy olurdu. Dosya adları da temizleniyor (`safe_name`) — bir sayfadan gelip
yola dönüşen tek şey o.

**Offline kalıyor:** Görünüm açılmadan hiçbir istek yok. README ve
CONTRIBUTING bunu açıkça yazıyor.

---

## 2026-08-26 — IPC: üç pipe instance'ı, uzak istemci yok

**Karar:** `CreateNamedPipeW` üç ayrı thread'de üç instance açıyor ve mod'a
`PIPE_REJECT_REMOTE_CLIENTS` eklendi.

**Gerekçe:** Tek instance ile UI düzenli olarak kaybediyordu. İstemci her
istek için yeni bağlantı açıyor ve durumu zamanlayıcıyla yokluyor; kullanıcı
tıklaması bir yoklamanın üstüne geldiğinde ikincisi `ERROR_PIPE_BUSY` alıyor,
UI bunu ancak "motor çalışmıyor" diye okuyabiliyordu. Aynı boşluk bir
konuşmanın bitişi ile bir sonraki instance'ın oluşturulması arasında da
vardı. Üç dinleyici ikisini de kapatıyor; bağlantı başına thread ise 1.5
saniyede bir sonsuza kadar thread doğurmak demekti.

**Uzak istemci:** İsimli pipe, aksi söylenmedikçe SMB üzerinden
`\<makine>\pipe\muivly` olarak erişilebilir. Bu protokolün ağa çıkacak hiçbir
parçası yok.

**İstemci tarafı:** `connect` meşgul pipe'ı 20 ms aralıkla on kez deniyor —
"meşgul", "yok"un tam tersi bir cevap.

---

## 2026-08-26 — Tauri komutları async, main thread'de değil

**Karar:** Pipe, disk ve Steam taraması yapan her `#[tauri::command]`
`(async)` ile işaretlendi.

**Gerekçe:** Tauri senkron bir komutu main thread'de çalıştırıyor. Bu
komutların hepsi bloklayan G/Ç yapıyor, yani pencerenin kendi thread'ini
motorun cevap verme süresi kadar tutuyorlardı. Motor hızlı — ta ki olmayana
kadar (ekran değişimi, uyuyan diskten açılan dosya); o anda ayar penceresi
boyanmayı bırakıyor. Bu dosyalardaki hiçbir şey pencereye dokunmuyor.

---

## 2026-08-26 — Video processor yalnız gerçekten gerekince

**Karar:** `MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING` artık koşulsuz
değil. Okuyucu önce processor'sız kuruluyor; NV12 kabul edilirse öyle
kalıyor. Yalnız iki durumda processor'lu bir okuyucu ile yeniden açılıyor:
(1) NV12 reddedildi (10-bit HEVC P010 veriyor), (2) kaynak kapağın dört
katından büyük, yani ölçekleme yapılacak.

**Gerekçe:** Processor bir tampon havuzu daha demek ve duvar kağıdının ömrü
boyunca duruyor — ölçüm zaten `docs/decisions.md`'de (784 MB → 834+ MB).
Yaygın hâl (8-bit H.264/HEVC, native NV12) bunun hiçbirine ihtiyaç duymuyor.
Fazladan açma maliyeti yalnız nadir hâlde ödeniyor.

**Ayrıca:** `MF_SOURCE_READER_DISABLE_CAMERA_PLUGINS` eklendi (burada kamera
yok, eklenti zinciri boşuna yükleniyordu) ve `MFStartup` süreç başına bir
kez çağrılıyor.

---

## 2026-08-26 — Görünmeyince ses susmuyor, duruyor

**Karar:** Her monitör kapandığında ses thread'i akışı `Stop`+`Reset` ile
durduruyor ve 200 ms'lik yoklamaya geçiyor.

**Gerekçe:** Eskiden sessizlik, kazancı sıfıra çarpmakla elde ediliyordu —
yani tam ekran oyun açıkken kimsenin duymadığı bir parça CPU'da çözülmeye
devam ediyordu. "Görünmüyorsa iş yok" kuralının ses tarafındaki karşılığı
eksikti.

---

## 2026-08-26 — Kitaplık ızgarası: ekran dışındaki karo çözücü tutmaz

**Karar:** `Thumb` medya elemanını yalnız karo görünürken (ya da 800 piksel
yakınındayken) oluşturuyor; uzaklaşınca eleman söküyor.

**Gerekçe:** `preload="metadata"` her karoyu bir başlık okumasına indiriyor
ama `<video>` ne kadar az çözerse çözsün bir çözücü, ve tarayıcı aynı anda
kaç tane olabileceğini sınırlıyor (elli civarı). Sonrasında her yeni karo
sessizce yüklenemiyor. Izgara gerçek yüksekliğini koruduğu için kaydırma
konumu ve klavye gezinmesi bozulmuyor; giden yalnız çözücüler.

**Alternatif (elendi):** Tam sanallaştırma (görünen aralığı hesaplayıp
yalnız onu render etmek). Daha çok kod, aynı kazanç; asıl sınır DOM düğümü
sayısı değil çözücü sayısıydı.

---

## 2026-08-26 — Pil ayrı bir bütçe

**Karar:** `power/battery.rs`. `GetSystemPowerStatus` iki saniyede bir
okunuyor; pilde kare hızı ayrı bir tavana (varsayılan 24) çekiliyor, Windows
pil tasarrufu açıkken çizim tamamen duruyor.

**Gerekçe:** `caps/policy.rs` zaten pilde 30 fps'e iniyordu ama bunu yalnız
**açılışta bir kez** yapıyordu — kablo çekildiğinde hiçbir şey değişmiyordu.
Hedef kitle laptop kullanıcıları; ölçülebilir farkın en görünür olduğu yer
burası.

**Dondurmak, kapatmak değil:** Tepsideki "duraklat" monitörleri kapatıp
Windows duvar kağıdını gösteriyor. Pil tasarrufundaki davranış farklı: yüzey
duruyor, son kare ekranda kalıyor, yalnız hareket duruyor. Kabloyu her
çekişte masaüstünün bir anlığına Windows duvar kağıdına dönmesi kabul
edilebilir değildi.

---

## 2026-08-26 — Ses, başkası konuşurken geri çekilir

**Karar:** `audio/duck.rs`. Varsayılan çıkış aygıtındaki diğer oturumlar
`IAudioSessionManager2` ile taranıyor; herhangi biri **duyulur** ses
üretiyorsa duvar kağıdının sesi 200 ms içinde %12'ye iniyor.

**Gerekçe:** Arka plan sesi ancak arka planda kaldığı sürece hoş. Video
açıldığı anda üstüne binen bir ses, kullanıcının sesi bir daha hiç açmaması
demek.

**Oturum durumu değil, ölçüm:** `AudioSessionStateActive` yalanı çok: bir
tarayıcı sekmesi hiçbir şey çalmadan dakikalarca "aktif" duruyor. Oturum
başına tepe ölçer (`IAudioMeterInformation`) doğru sinyal. Sistem sesleri
oturumu muaf — bir bildirim sesi, fade bitmeden bitiyor.

---

## 2026-08-26 — Ekran değişince her şey yeniden kuruluyor

**Karar:** Gizli, üst düzey bir pencere (`compositor/notify.rs`)
`WM_DISPLAYCHANGE`, `WM_SETTINGCHANGE`, uyanma bildirimi ve `WM_HOTKEY`
alıyor. Yerleşim gerçekten değiştiyse (ya da WorkerW gittiyse) `caps` yeniden
sorgulanıyor ve bütün sahne yeniden kuruluyor.

**Neden ayrı bir pencere:** Duvar kağıdı yüzeyleri WorkerW'in **çocuğu**.
Windows bu yayınları yalnız üst düzey pencerelere gönderiyor, `RegisterHotKey`
de bir çocuk pencereye teslim edemiyor. `HWND_MESSAGE` de işe yaramıyor:
message-only pencereler yayın mesajı almıyor — tam da alması gereken şeyi.

**Yerleşim karşılaştırması:** `WM_SETTINGCHANGE` çok şey için tetikleniyor,
o yüzden yeniden kurmadan önce monitör listesi (ad, konum, boyut) eskisiyle
karşılaştırılıyor. Değişmediyse hiçbir maliyet yok.

---

## 2026-08-26 — Monitör başına ayar, tek çözme

**Karar:** `fit`, görünüm (parlaklık/doygunluk/bulanıklık) ve kare hızı
monitör başına geçersiz kılınabiliyor. Kare hızı tavanı sunumu atlıyor,
çözmeyi değil.

**Gerekçe:** Ultrawide + dikey ekran karışımında tek bir `fit` doğru olamaz.
Kare hızının monitör başına olmasının ayrı bir değeri var: çözme zaten
paylaşılıyor, dolayısıyla ikinci ekranı 10 fps'e çekmek çizim ve flip'lerin
onda dokuzunu siliyor, çözmeden hiçbir şey eksilmiyor.

---

## 2026-08-26 — Yayma (span): kırpma masaüstünün tamamına göre

**Karar:** `span` açıkken UV eşlemesi tek monitöre değil, bütün masaüstünün
sınırlayıcı kutusuna göre hesaplanıyor; her monitör kendi dikdörtgenine
düşen dilimi örnekliyor.

**Gerekçe:** Kırpma monitör başına yapılsaydı her ekran videoyu kendi
oranına göre kırpardı ve görüntü çerçeveler arasında kaymış olurdu — yani
yaymanın tek amacı kaybolurdu.

**Playlist:** `span` açıkken bir ekrana duvar kağıdı seçmek hepsini birden
değiştiriyor. Dilimlerin tek bir resimden gelmesi şart; aksi hâlde her ekran
başka bir videonun bir parçasını gösterirdi.

---

## 2026-08-26 — Geçişte eski kare bir kopya olarak tutuluyor

**Karar:** Duvar kağıdı değiştiğinde, eski çözücü kapanmadan hemen önce
ekrandaki kare ekran boyutunda bir BGRA dokusuna çiziliyor; yeni kağıdın
üstüne azalan alfa ile bindiriliyor ve geçiş biter bitmez doku bırakılıyor.

**Gerekçe:** Eski çözücü zaten kapanıyor — o kareyi yakalamanın son anı
burası. Kopya ekran uzayında alınıyor (fit, grade ve blur uygulanmış hâlde),
böylece geçişin taşıması gereken tek şey alfa; ayrı bir durum yok.

**Maliyet:** 1080p'de 8 MB, geçiş süresince. Bitince düşüyor — RAM sözü
verilen bir projede süslemenin bellekte kalması kabul edilemezdi.

---

## 2026-08-26 — `.muivly` paketi: zip, elle yazıldı

**Karar:** Paket, `manifest.json` + medya içeren bir zip. Sıkıştırma yok
(yalnız "stored"), zip okuma/yazma `wallpaper-ui/src-tauri/src/pack.rs`
içinde elle yazıldı.

**Gerekçe:** Yük zaten H.264 — sıkıştırılacak bir şey yok. Zip formatı
seçildi çünkü Muivly'si olmayan biri de açabilsin. Kütüphane yerine ~200
satır: bağımlılık ağacı, derleme süresi ve ikili boyutu yok. Hiçbir yerde
dosyanın tamamı belleğe alınmıyor (64 KB parçalar).

**Güvenlik sınırı:** Paketteki her ad `safe_name`'den geçiyor — ayraçlar ve
sürücü iki noktası siliniyor. Formatın tek gerçek saldırı yüzeyi bu.

---

## 2026-08-26 — Kodek adı, HRESULT değil

**Karar:** Bir dosya açılamadığında `decoder::why_not` dosyanın video
akışının alt türünü okuyup mesajı ona göre yazıyor: "AV1 — Store'dan ücretsiz
AV1 Video Extension'ı kur" ile "bu GPU'da AV1 çözücü yok, H.264'e dönüştür"
birbirinden ayrılıyor.

**Gerekçe:** Media Foundation ikisi için de aynı HRESULT'ı veriyor, oysa
biri ücretsiz bir indirmeyle, diğeri dosyayı dönüştürmekle çözülüyor.
Kullanıcının yapabileceği iki şey var ve hangisi olduğunu söylemek gerekiyor.

---

## 2026-08-27 — RAM: ölçüldü, `MF_LOW_LATENCY` girdi, havuz ayarı çıktı

**Makine:** AMD Radeon (entegre, birincil çıkış 2560x1440) + RTX 4050
(1920x1080). 15654 MB RAM. **En kötü hâl:** 4K H.264 klip, iki adapter, yani
iki decoder. Ölçüm: 25 sn ısınma, sonra 4-5 örnek.

| | working set | private | thread |
|---|---|---|---|
| Önce | ~610 MB | ~790 MB | 80-82 |
| `MF_LOW_LATENCY` | ~540 MB | ~706 MB | 79-81 |
| + havuz öznitelikleri | ~515 MB | ~703 MB | 79-81 |

**Karar 1 — `MF_LOW_LATENCY` girdi.** ~70 MB working set, ~85 MB private
(%11). Duvar kağıdı için "gecikme" ile ilgisi yok: decoder'a film oynatmak
için tuttuğu çözülmüş kare kuyruğunu kurmamasını söylüyor.

**Riski nasıl kapattık:** Bu bayrağın bilinen tehlikesi bazı decoder'ların
B-frame'leri sunum sırasına geri dizmeyi bırakması — sonuç, hiçbir hata
vermeyen ince bir titreme. `read_loop` artık aldığı zaman damgalarını izliyor
ve sıra bozulursa bir kez uyarıyor. Dört ayrı 4K klipte (H.264) hiç
tetiklenmedi. Tetiklendiğine dair bir rapor, bu satırı geri almanın gerekçesi.

**Karar 2 — havuz öznitelikleri çıktı.** `MF_SA_MINIMUM_OUTPUT_SAMPLE_COUNT`
ve `MF_SA_REQUIRED_SAMPLE_COUNT` decoder MFT'sine (IMFSourceReaderEx →
GetTransformForStream) `Ok` dönerek yazılıyor ve **private bytes'ı hiç
değiştirmiyor** (706 → 703, gürültü sınırında). İkisi de taban belirliyor,
tavan değil; asıl tabanı codec'in referans kare gereksinimi koyuyor.
Working set'teki ~25 MB fark sayfalama gürültüsü.

**Kalan:** ~700 MB private hâlâ hedefin çok üstünde ve neredeyse tamamı iki
4K decoder'ın DPB'si. Buradan sonrası mimari: (a) ölçek kapağını monitör
başına gerçek çözünürlüğe indirmek — ama processor bir havuz daha ekliyor,
ölçülmüştü, net zarar; (b) 80 thread'in kaynağı MF'in paylaşılan iş kuyruğu
havuzu, ve bunu küçültmek `ReadSample`'dan asenkron bir tasarıma geçmek
demek. İkisi de ayrı bir iş.

---

## 2026-08-27 — Görünmezken çözücüyü bırakmak (hibernasyon)

**Karar:** Masaüstü belli bir süre (varsayılan 20 sn) hiç görünmediyse motor
tüm çözücüleri `drop` ediyor. Ekran son karede kalıyor, masaüstü görününce
çözücüler yeniden açılıyor (klip baştan başlıyor).

**Gerekçe:** "Görünmüyorken CPU ~0" zaten sağlanıyordu ama *bellek* değil.
Tam ekran oyun açıkken iki 4K decoder'ın DPB'si (≈700 MB private, bkz. bir
üstteki ölçüm) olduğu gibi duruyordu — yani hafiflik iddiasının en çok
önemsendiği anda en pahalı olduğumuz an. `sync_decoders` yalnız `enabled` ve
`video` alanlarına bakıyordu; `occluded`/`frozen` hiç okunmuyordu.

**Neden son kare için kopya tutmuyoruz:** Yüzeyler ayakta kalıyor ve sunum
yapılmıyor, dolayısıyla ön tampon zaten son kareyi gösteriyor. Ayrı bir
yakalama dokusu (4K'da 32 MB) tam da geri vermeye çalıştığımız belleği
tutardı.

**Neden konum geri yüklenmiyor:** Duvar kağıdı döngüde. Geri sarmak
`SetCurrentPosition` + yeniden çözme demek; kazandırdığı şey "aynı yerden
devam etti" hissi, maliyeti uyanma anındaki tek pahalı iş.

**Alternatif (elendi):** Süreyi sıfır yapmak, yani alt+tab'da anında
bırakmak. Oyun–masaüstü arası geçiş yapan biri her seferinde yeniden açılışı
görür; 20 sn bir alt+tab'ın ötesinde, bir yükleme ekranının içinde.

---

## 2026-08-27 — "Hafiflet": tek seferlik yeniden yazma (`optimize/`)

**Karar:** Kitaplıktaki bir klibi, en büyük ekranın çözünürlüğünde ve tier
kare hızında bir kez H.264'e yeniden yazan bir iş eklendi. Çıktı
`%APPDATA%\Muivly\light\` altına gidiyor ve bitince kitaplığa ekleniyor.
Donanım decode → donanım encode, tek D3D11 cihazı paylaşılıyor.

**Gerekçe:** Ölçümün bıraktığı yerden devam: kalan ~700 MB'ın tamamı kare
boyutu × referans kare sayısı. Kare sayısını codec belirliyor, boyutu ise
dosya. Oynatma sırasında ölçeklemek net zarar (processor bir havuz daha
ekliyor — 2026-08-27 ölçümü). Dosyayı bir kez küçük yazmak aynı kazancı
kalıcı ve bedava veriyor: boru hattında processor yok, ikinci havuz yok.

**CPU decode kuralıyla ilişkisi:** İhlal değil. Okuyucu ve yazıcı MF DXGI
device manager ile bizim cihazımıza bağlı; donanım encoder yoksa iş
"bu dosya için donanım encoder yok" diye başarısız oluyor, CPU'ya düşmüyor.

**Ses:** Düşürülmüyor, AAC 96 kbit/s olarak yeniden yazılıyor. Sesli bir
duvar kağıdının "optimize" edildikten sonra sessizleşmesi kullanıcının geri
alamayacağı bir hata olurdu.

**Alternatif (elendi):** Kaynak dosyanın yanına yazmak. Kitaplıklar salt
okunur paylaşımlarda ve kullanıcının topladığı klasörlerde duruyor.

---

## 2026-08-27 — Shader duvar kağıtları (`decoder/procedural.rs`)

**Karar:** `.hlsl`/`.fx` dosyaları bir duvar kağıdı türü. Kullanıcı tek bir
`float4 mainImage(float2 uv)` yazıyor; prelude (cbuffer + vertex shader) ve
entry point motor tarafından ekleniyor. Shader ekran dışı bir dokuya çiziyor
ve aşağıya `Frame::Bgra` olarak gidiyor.

**Gerekçe:** Video için bellek tablosu hep kötü kalacak — DPB fizik. Shader'da
decoder yok, DPB yok, MF thread havuzu yok: bir doku (1080p'de 8 MB) ve bir
program. "Muivly'nin gösterebileceği en hafif duvar kağıdı" kategorisi.

**Neden ekran dışı dokuya, doğrudan swap chain'e değil:** Bir tam ekran geçiş
maliyetine karşılık fit, ölçek, parlaklık/doygunluk/bulanıklık, crossfade ve
span'in tamamı bedava geliyor — aşağıdaki her şey için bu bir fotoğraftan
ayırt edilemiyor.

**Derleme hataları:** `D3DCompile` prelude'un da içinde olduğu metni
numaralıyor. Satır numaraları kullanıcının dosyasına geri kaydırılıyor
(`shift_line_numbers`, testli) — aksi hâlde 12 satırlık dosyada "satır 43"
hatası veriliyor.

---

## 2026-08-27 — Bellek bütçesi: kullanıcı ayarı olarak kare boyutu tavanı

**Karar:** Ayarlarda MB cinsinden bir bütçe var; `caps::capped` bunu bir kare
boyutu tavanına çeviriyor (600+ → doğal, 350 → 1440p, 200 → 1080p, altı →
720p) ve tier'ın kendi tavanıyla **küçük olan** kazanıyor.

**Gerekçe:** Tek gerçek kaldıraç kare boyutu, ve kullanıcıya MB sormak
"1440p mi 1080p mi" sormaktan daha dürüst — istediği şey sayı, tercihi değil.
Kaba kademeli, çünkü altındaki tahminin kendisi kaba.

**Maliyeti açıkça söyleniyor:** Değiştirmek çözücüleri yeniden açıyor, yani
oynayan her klip baştan başlıyor. UI bunu yazıyor ve kalıcı çözüm olarak
"Hafiflet"i gösteriyor.

---

## 2026-08-27 — Hafiflet artık referans kare sayısını da düşürüyor

**Karar:** `optimize/encode.rs` yazıcının encoder MFT'sini `IMFSinkWriterEx`
ile alıp `ICodecAPI` üzerinden `AVEncVideoMaxNumRefFrame = 1` ve
`AVEncMPVGOPSize = fps` ayarlıyor. Encoder söz dinlemezse iş yine bitiyor,
sadece eski hâliyle.

**Gerekçe:** DPB = referans kare sayısı × kare boyutu. Hafiflet şimdiye kadar
çarpanın yalnız bir yarısına dokunuyordu. H.264 encoder'ları varsayılan
olarak dört ya da daha fazla referans tutuyor çünkü ileri geri sarılan
filmler için tasarlandılar; döngüye giren bir duvar kağıdında ilkinden
sonraki her referans, duvar kağıdı ekranda olduğu sürece tutulan bir tam kare.

**Neden en iyi ihtimalle bir tahmin:** Hangi transform'un encoder olduğu
sabit değil (önünde bir dönüştürücü olabilir), o yüzden ilk dört transform
denenip referans ayarını kabul eden encoder sayılıyor. Gerçek makinede
ölçülmedi — `tasks.md`'de duruyor.

---

## 2026-08-27 — Ses spektrumu: thread yok, kare başına boşaltma var

**Karar:** `audio/spectrum.rs` WASAPI loopback yakalaması açıyor ama kendi
thread'i ya da zamanlayıcısı yok: tampon render döngüsünden, motorun zaten
uyanık olduğu bir geçişte boşaltılıyor. Tampon 200 ms, yani 10 fps çizen bir
duvar kağıdı bile taşırmıyor. Bantlar sekiz Goertzel filtresiyle çıkarılıyor
(FFT kütüphanesi yok, bağımlılık yok).

**`meter.rs`'in reddettiği şeyle ilişkisi:** Çelişmiyor. `meter.rs` bir efekt
için "bir thread ve birkaç milisaniyede bir uyanma"yı reddediyordu; burada
ikisi de yok. Ayrıca tembel: `iBand` yazmayan bir shader ses yığınına hiç
dokunmuyor, ekrandan çıkınca yakalama bırakılıyor.

**Neden tek seviye yetmedi:** Spektrum çizen bir duvar kağıdı örneklerin
kendisini istiyor. Uydurma bantlar üretmek — tek seviyeden sekiz sayı
türetmek — dürüst olmazdı.

---

## 2026-08-27 — Vurgu rengi: geri alınabilir olduğu için var

**Karar:** İsteğe bağlı bir ayar duvar kağıdının ortalama rengini Windows
vurgu rengine yazıyor (`compositor/accent.rs`). Renk GPU'dan 16×9 okunuyor
(`Renderer::dominant_colour`) — projedeki tek geri okuma, ve kare başına
değil duvar kağıdı değişince.

**Gerekçe ve sınır:** Registry kuralı "kapatılabilir" diyordu; bu, kullanıcının
zaten sahip olduğu bir değerin üstüne yazan ilk yer. O yüzden eski değerler
ilk yazmadan önce `accent-backup.txt`'ye alınıyor ve ayar kapanınca, motor
çıkınca ya da motor öldürülmüşse bir sonraki açılışta geri konuyor.

**Dürüst kalan taraf:** `AccentPalette`'in biçimi belgelenmiş değil. Yazılan
düzen bu işi yapan her aracın oturduğu düzen; bu, özelliğin varsayılan olarak
kapalı ve kullanıcının açtığı bir şey olmasının sebebi.

---

## 2026-08-27 — Boşta durma ve makine yükü: iki yeni "çizmeye değmez" sinyali

**Karar:** İki ayar daha aynı soruyu başka yönlerden soruyor. `power/idle.rs`
`GetLastInputInfo` ile "makinenin başında kimse yok" durumunu yakalıyor
(varsayılan 5 dk → son kare kalıyor) ve Windows'un "animasyonları göster"
ayarını okuyor. `power/load.rs` `GetSystemTimes` ile makinenin tamamının
meşguliyetini saniyede bir ölçüp duvar kağıdını düşük kare hızına indiriyor
(varsayılan %80'in üstünde iki örnek → 10 fps, %60'ın altında geri).

**Gerekçe:** Örtülme tespiti "görülebilir mi" diye soruyor ve boş sandalyenin
önündeki açık masaüstünü hiç yakalamıyor — README'nin makinesinde bu bir
çekirdeğin %13.7'si, kimse için. Yük tarafında ise duvar kağıdı pahalı hâle
gelmiyor; geri kalan her şey geliyor.

**Histerezis şart:** Aralıksız tek eşik, duvar kağıdının dakikada birkaç kez
iki kare hızı arasında gidip gelmesi demek — bu tasarruf gibi değil takılma
gibi görünüyor.

**Tuş kaydı yok:** Windows'un kendi sayacı okunuyor, kanca kurulmuyor.

## 2026-08-28 — Kitaplık karosu video değil, yakalanmış tek kare

**Karar:** `components/Thumb.tsx` artık ızgarada `<video>` tutmuyor. Dosya
bir kez açılıyor, 1. saniyedeki kare 640px genişliğinde bir canvas'a çizilip
JPEG blob'una çevriliyor, video bırakılıyor. Karo bir `<img>`. Gerçek video
yalnızca imleç karonun üstündeyken var oluyor. Yakalama sırayla yapılıyor
(tek seferde bir dosya) ve sonuçlar yol bazında bellekte tutuluyor.

**Gerekçe — ölçüm:** Bu makinede (2560x1440 @180Hz, entegre GPU) kitaplıkta
tek bir 3840x2160 klip varken ayar penceresi **bir çekirdeğin %60'ını ve
~870 MB** harcıyordu; kitaplık boşken %1 / 405 MB. `<video>` bir resim değil,
bir kompozit katmanı: elemanın kendisi durdurulmuş olsa bile ekranın
tazeleme hızında yeniden kompozit ediliyor. Düzeltmeden sonra **on** 4K klip
%1 / 537 MB. Motor bu sırada donmuş hâlde %0.1 harcıyordu — yani kullanıcının
şikâyet ettiği kasma duvar kağıdından değil, ayar panelinden geliyordu.

**Üç tuzak, üçü de ölçülerek bulundu:** (1) DOM'a bağlı olmayan bir `<video>`
süresini ve boyutunu bildiriyor ama kare üretmiyor — `drawImage` siyah
çiziyor; eleman gerçekten görünür olmalı (2px, %2 opaklık yetiyor).
(2) `requestVideoFrameCallback` işe yaramıyor: yeni kare sunulduğunda
tetikleniyor, seek'lenip duraklatılmış video bir daha kare sunmuyor.
(3) CSP'de `img-src` içinde `blob:` yoktu; poster üretiliyor ama engelleniyordu.

**Alternatif (reddedilmedi, ertelendi):** Kareyi motora ya da UI'nin Rust
tarafına donanımda çözdürmek mimari olarak daha temiz. WebView zaten dosyayı
bir kez açıyor ve maliyet tutmakta, çözmekte değil — bu yüzden şimdilik
gerekmedi.
