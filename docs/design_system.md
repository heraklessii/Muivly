# Mui Tasarım Sistemi — Muivly Uyarlaması

Kaynak: `Muita/src/renderer/src/styles.css` ve
`Muitoon/client/public/css/main.css`. İkisi de aynı paleti kullanıyor; Muivly
üçüncü Mui ürünü olarak aynı temayı sürdürür.

Kural (Muita'dan devralındı): **bileşen dosyasına asla renk/ölçü sabiti
yazılmaz.** Her şey buradaki jetonlardan gelir, tema `<html data-theme>` ile
değişir.

---

## Marka özeti

| | Değer |
|---|---|
| Vurgu | teal `#2dd4bf` (koyu tema) / `#0d9488` (aydınlık) |
| Zemin | `#0f1115` |
| Panel | `#181b22` |
| Metin | `#e8eaed` |
| Font | **Outfit** (variable, 100–900), yerele gömülü — CDN yok |
| Ana tema | Koyu. Aydınlık tema türetilmiş varyant. |

Muitoon'da eski pembe vurgu (`#ff4d8d`) kaldırılıp teal'e geçilmiş — yani teal
mevcut Mui kimliği, geri dönülmüş bir renk değil.

## Jeton isimlendirmesi

Muita ve Muitoon isimleri biraz ayrışıyor. Muivly bir **masaüstü uygulaması**
olduğu için Muita'nın şemasını alıyor:

| Muivly (= Muita) | Muitoon karşılığı |
|---|---|
| `--bg-panel` | `--surface` |
| `--bg-elevated` | `--surface-2` |
| `--text` | `--fg` |
| `--text-muted` | `--muted` |

`--bg`, `--accent`, `--border`, `--radius` üçünde de aynı.

## Jetonlar

```css
:root, :root[data-theme='dark'] {
  --bg: #0f1115;
  --bg-panel: #181b22;
  --bg-elevated: #20242d;
  --bg-sunken: #0b0d11;
  --border: #262b35;
  --border-strong: #333a47;
  --text: #e8eaed;
  --text-muted: #8b93a3;

  --accent: #2dd4bf;
  --accent-strong: #5eead4;
  --accent-soft: rgb(45 212 191 / 12%);   /* kart zemini, seçili satır */
  --accent-line: rgb(45 212 191 / 34%);   /* odak halkası */
  --on-accent: #04211d;                   /* teal AÇIK: üstündeki yazı koyu */

  --error: #ff7a6b;
  --warning: #fbbf24;                     /* "bak buraya" ≠ "bu bozuk" */
  --ok: #a3e635;                          /* limon: "bitti" ≠ "aktif" */

  --shadow-1: 0 1px 2px rgb(0 0 0 / 40%);
  --shadow-2: 0 8px 24px -6px rgb(0 0 0 / 60%);
  --shadow-3: 0 24px 64px -12px rgb(0 0 0 / 70%);

  color-scheme: dark;
}

:root[data-theme='light'] {
  --bg: #eef1f5;
  --bg-panel: #ffffff;
  --bg-elevated: #f2f4f8;
  --bg-sunken: #e7eaf0;
  --border: #dde1e9;
  --border-strong: #c3c9d4;
  --text: #12151a;
  --text-muted: #5d6675;

  --accent: #0d9488;                      /* aynı teal, açık zeminde okunur */
  --accent-strong: #0f766e;
  --accent-soft: rgb(13 148 136 / 10%);
  --accent-line: rgb(13 148 136 / 30%);
  --on-accent: #ffffff;

  --error: #c0392b;
  --warning: #b45309;
  --ok: #4d7c0f;

  --shadow-1: 0 1px 2px rgb(18 21 26 / 8%);
  --shadow-2: 0 8px 24px -6px rgb(18 21 26 / 14%);
  --shadow-3: 0 24px 64px -12px rgb(18 21 26 / 20%);

  color-scheme: light;
}

:root {
  --space-1: 4px;  --space-2: 8px;  --space-3: 12px;
  --space-4: 16px; --space-5: 22px; --space-6: 32px;

  --font-ui: 'Outfit', 'Segoe UI', system-ui, sans-serif;
  --font-xs: 11px; --font-sm: 12px; --font-md: 13px;
  --font-lg: 15px; --font-xl: 19px;

  --radius-sm: 8px;    /* kontroller */
  --radius: 12px;      /* kartlar */
  --radius-lg: 16px;   /* yüzeyler */
  --radius-pill: 999px;

  --control-height: 32px;
  --topbar-height: 52px;
  --statusbar-height: 28px;

  --ease: cubic-bezier(0.2, 0.6, 0.3, 1);
  --t-fast: 110ms;
  --t-med: 180ms;
}
```

## Kullanım kuralları

- **Vurgu rengi yalnız arayüz kromunda**: aktif sekme, odak halkası, birincil
  düğme, ilerleme çubuğu. Wallpaper önizleme küçük resimlerinin üstüne teal
  bindirme yapılmaz — "arayüz mü, içerik mi" ayrımı korunur (Muita kuralı).
- `--on-accent` unutma: teal açık bir renk, üstündeki yazı koyu olmak zorunda.
- Gövde: `font-size: var(--font-md)`, `line-height: 1.5`,
  `letter-spacing: 0.005em`, `font-variant-numeric: tabular-nums`.
  Tabular-nums Muivly'de ayrıca gerekli — RAM/CPU/fps sayaçları zıplamasın.
- `user-select: none` kromda; hata mesajı ve dosya yolu gibi **bilgi** olan
  metin `user-select: text` ile istisna tutulur.
- İnce kaydırma çubuğu: `scrollbar-width: thin`,
  `scrollbar-color: var(--border-strong) transparent`.
- Form elemanları fontu miras almaz — `button, input, select, textarea
  { font-family: inherit; }` yazılması şart (Muitoon P118).

## Glass / neon (isteğe bağlı, ölçülü)

Muitoon'da var, Muita'da yok. Muivly'de yalnız wallpaper önizleme üstündeki
bindirme kontrollerinde kullanılabilir:

```css
--glass-bg: rgba(15, 17, 21, 0.72);
--glass-border: rgba(255, 255, 255, 0.07);
--neon-glow-sm: 0 0 12px rgb(45 212 191 / 28%);
```

`backdrop-filter` pahalı — ayar paneli WebView'i zaten "sadece açıkken ödenen"
maliyet, ama yine de tek bir yüzeyle sınırlı tutulur.

## Font dosyaları

Outfit variable woff2, iki alt küme: `latin` + `latin-ext`.
**`latin-ext` Türkçe için şart** (ğ ş İ ı Ş Ğ).

Muita'daki gibi `public/` yerine `assets/` altına konur ki Vite göreli yolu
yeniden yazsın ve paketlenmiş uygulamada da bulunsun. CDN yok — Tauri CSP'si
zaten uzak kaynağa izin vermeyecek.

Dosyalar Muita'dan kopyalanabilir:
`Muita/src/renderer/src/assets/fonts/outfit-latin{,-ext}.woff2`
