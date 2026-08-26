# CLAUDE.md

Bu dosya her session'da otomatik okunur. Sadece **her zaman geçerli olan** kural ve
mimari bilgi burada tutulur. Detay/geçmiş için `docs/` klasörüne bak — orayı sen
gerektiğinde oku, ben proaktif linklemeyeceğim.

## Proje Özeti

İsim: **Muivly** (Mui ailesi: Muita, Muitoon, Muivly)
Repo: https://github.com/heraklessii/Muivly — **açık kaynak, Apache-2.0**
Ne: Native, çok hafif live wallpaper uygulaması — Wallpaper Engine alternatifi.
Hedef kitle: **Düşük donanım / eski PC kullanıcıları** (entegre GPU, 4-8GB RAM).
Öncelikli içerik tipi: Video wallpaper (mp4/webm). Shader/web desteği ileride.

Fark yaratma stratejisi: RAM/CPU kullanımını ölçülebilir şekilde WE'den düşük tutmak.
Bu, her mimari kararın önceliği olmalı — "çalışır" yetmez, "hafif çalışır" olmalı.

## Mimari (özet — detay için docs/project_overview.md)

İki ayrı process, IPC ile haberleşir:

```
wallpaper-core/   Rust native binary. Asıl motor. UI kapansa da RAM'de kalmaz,
                  bağımsız arka plan servisi olarak çalışır.
  caps/           GPU/sistem probe. Core başında BİR KEZ çalışır, sonucu
                  Arc<GpuProfile> olarak diğer modüllere geçer.
  decoder/        Media Foundation + D3D11VA hardware decode. ffmpeg YOK
                  (demux için istisna olabilir, decode asla CPU'da yapılmaz).
  compositor/     WorkerW injection, multi-monitor shared D3D11 texture.
  power/          Fullscreen/occlusion detection → render throttle/pause.
  ipc/            Named pipe, UI ile konuşur.

wallpaper-ui/     Tauri v2 + React. SADECE ayar paneli. Wallpaper render'ı
                  buradan asla yapılmaz — WebView RAM'i wallpaper'a karışmaz.
```

## Kesin Kurallar

- **Video decode her zaman GPU'da** (D3D11VA/Media Foundation). CPU fallback
  eklemeden önce mutlaka sor — RAM/CPU bütçesini bozar. Codec'in HW desteği
  yoksa video oynatılmaz: ilk frame statik gösterilir, UI'da neden bildirilir.
- **Zero-copy**: decode edilen frame CPU'ya round-trip yapmadan D3D11 texture
  olarak kalır. Yeni bir video pipeline kodu yazarken bunu bozma.
- **Çoklu monitör**: aynı video birden fazla ekranda oynuyorsa decode TEK SEFER
  yapılır, texture paylaşılır (`IDXGIResource`). Monitör başına ayrı decode YASAK.
  Tek istisna: monitörler farklı GPU'lara bağlıysa **adapter başına** bir decode
  (cross-adapter paylaşım zero-copy'yi bozuyor).
- **Render, çıkışın sahibi olan adapter'da.** Hibrit laptop'ta dGPU seçilmez —
  pil ve cross-adapter kopya nedeniyle.
- **Idle/occluded durumda render durur.** Fullscreen uygulama açıkken veya
  WorkerW görünmüyorken CPU/GPU kullanımı ~0'a yakın olmalı.
- **wallpaper-ui'de asla wallpaper render'ı yapma.** WebView sadece UI için.
- Yeni bağımlılık (crate/npm paketi) eklemeden önce RAM/binary boyutu etkisini
  değerlendir, büyükse sor.

## Açık Kaynak — Bu Kod Yabancılar Tarafından Okunacak

Muivly herkese açık bir repo ve insanlar GitHub Releases'ten indirip
kullanacak. Bunun günlük işe yansıması:

- **Dışa dönük her şey İngilizce**: README, CONTRIBUTING, issue şablonları,
  CI adım isimleri, commit mesajları, kod yorumları, CLI çıktısı ve hata
  mesajları. `docs/` ve CLAUDE.md Türkçe kalır — onlar iç dosyalar.
- **CI yeşil olmadan bitmiş sayılmaz**: `cargo fmt --all -- --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test --all`. Uyarı
  bırakma.
- **Telemetri yok.** Analytics, crash reporting, sessiz ağ çağrısı yok.
  README bunu vaat ediyor.
- **Saf mantık test edilir.** `caps/policy.rs` ve `caps/adapter.rs`'teki
  sınıflandırma gibi donanımdan bağımsız kod her zaman testli olmalı;
  donanıma dokunan kod için `--caps` çıktısı worklog'a yazılır.
- **Sürüm**: semver. `v*` tag'i push'lanınca `release.yml` taslak release
  açıyor. Sürüm numarasını kendi başına artırma, İlker söyler.
- **Kırılan kural = reddedilen PR.** "Kesin Kurallar" başlığı
  CONTRIBUTING.md'de İngilizce olarak da yazılı; ikisi birbiriyle tutarlı
  kalmalı. Birini değiştirirsen diğerini de değiştir.

## Konvansiyonlar

- Rust: `wallpaper-core` içinde modül başına klasör (yukarıdaki yapı).
- Commit mesajları: Türkçe veya İngilizce fark etmez, kısa ve net.
- Kod yorumları: İngilizce (paylaşılabilir/açık kaynak ihtimaline karşı).
- İlker ile konuşma dili: Türkçe, doğrudan, uzun açıklamadan çok aksiyon.
- **Arayüz**: Muivly, Muita/Muitoon ile ortak Mui temasını kullanır (teal
  `#2dd4bf`, zemin `#0f1115`, Outfit font). Bileşen dosyasına renk/ölçü sabiti
  YAZILMAZ, hepsi CSS jetonlarından gelir — bkz. `docs/design_system.md`.

## docs/ Dosyaları — Ne Zaman Oku/Yaz

Varsayılan: **okuma/yazma yok.** Küçük değişiklik, bug fix, soru-cevap gibi
rutin işlerde bu dosyalara dokunma. Aşağıdaki dosyalar sadece belirtilen
durumda devreye girer:

| Dosya | Ne zaman OKU | Ne zaman YAZ |
|---|---|---|
| `docs/decisions.md` | Mimariyi değiştiren bir öneri gelince (ör. "ffmpeg'e geçelim mi") | Yeni bir mimari karar kesinleşince, kısa madde |
| `docs/project_overview.md` | Yeni bir modül tasarlarken / "bu neden böyle" sorusunda | Mimari gerçekten değişince |
| `docs/tasks.md` | "Sırada ne var" sorulunca veya yeni bir çalışmaya başlarken | Bir görev bitince/eklenince, tek satır |
| `docs/worklog.md` | Nadiren, geçmiş bir kararın tarihini bulmak gerekirse | Sadece mimariyi etkileyen bir iş bitince, 2-4 madde |
| `docs/design_system.md` | wallpaper-ui'de CSS/bileşen yazarken | Jeton eklenince/değişince |
| `docs/gpu_capability.md` | `caps/`, fps politikası veya codec desteği işine dokununca | Probe/tier mantığı değişince |

Belirsizsen yazma — İlker gerekirse zaten "worklog'a ekle" diye söyler.