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
