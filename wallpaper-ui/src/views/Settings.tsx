import { useEffect, useState } from 'react'

import {
  contextMenu,
  disk,
  engine,
  pack,
  pickPackage,
  startup,
  type EngineStatus,
  type Fit,
} from '../api'
import type { Store } from '../store'

type Props = {
  store: Store
  status: EngineStatus | null
  onChange: (next: Store) => void
  onRefresh: () => void
}

const FITS: { value: Fit; label: string; help: string }[] = [
  { value: 'cover', label: 'Doldur', help: 'Ekranı kaplar, taşan kenarları kırpar' },
  { value: 'contain', label: 'Sığdır', help: 'Tamamını gösterir, kenarlarda siyah bant' },
  { value: 'stretch', label: 'Ger', help: 'Ekranı kaplar, oranı bozar' },
]

/**
 * The handlers that mean "the user has finished moving this slider".
 *
 * There is more than one way to let go of a range input, and `mouseup` is
 * only the most obvious: an arrow key never produces one, a drag released
 * off the end of the track fires it somewhere else entirely, and a touch
 * produces none at all. Listening for that alone meant a slider could be
 * moved with the keyboard all day without a single value reaching the
 * engine, and a drag that overshot the track was silently thrown away.
 *
 * Commits are idempotent — each checks whether anything actually changed —
 * so several of these firing for one gesture costs nothing.
 */
function releases(commit: () => void) {
  return {
    onPointerUp: commit,
    // Fires when a drag ends anywhere, including outside the element.
    onLostPointerCapture: commit,
    onKeyUp: commit,
    onBlur: commit,
  }
}

/** What to drop to while unplugged. 0 means "the same as on the charger". */
const BATTERY_RATES: { value: number; label: string }[] = [
  { value: 0, label: 'Düşürme' },
  { value: 30, label: '30 fps' },
  { value: 24, label: '24 fps' },
  { value: 15, label: '15 fps' },
]

const SPEEDS: { value: number; label: string }[] = [
  { value: 0.5, label: '0.5×' },
  { value: 0.75, label: '0.75×' },
  { value: 1, label: 'Normal' },
  { value: 1.5, label: '1.5×' },
  { value: 2, label: '2×' },
]

const FADES: { value: number; label: string }[] = [
  { value: 0, label: 'Sert kesme' },
  { value: 250, label: 'Hızlı' },
  { value: 400, label: 'Normal' },
  { value: 800, label: 'Yavaş' },
]

const INTERVALS: { value: number; label: string }[] = [
  { value: 0, label: 'Klip bittiğinde' },
  { value: 300, label: '5 dakika' },
  { value: 900, label: '15 dakika' },
  { value: 1800, label: '30 dakika' },
  { value: 3600, label: '1 saat' },
  { value: 21600, label: '6 saat' },
]

export default function Settings({ store, status, onChange, onRefresh }: Props) {
  const [statePath, setStatePath] = useState('')
  // Edited locally while dragging so the poll does not fight the slider.
  const [fpsDraft, setFpsDraft] = useState<number | null>(null)
  // Same reason: a slider bound straight to polled state jumps back under
  // the cursor every time the poll lands mid-drag.
  const [visual, setVisual] = useState({
    brightness: store.settings.brightness,
    saturation: store.settings.saturation,
    blur: store.settings.blur,
  })
  const [sound, setSound] = useState(store.settings.sound)
  const [volume, setVolume] = useState(store.settings.volume)
  const [autostart, setAutostart] = useState(false)
  const [startupError, setStartupError] = useState<string | null>(null)
  const [menu, setMenu] = useState(false)
  // What the last import or export did, shown until the next one.
  const [packNote, setPackNote] = useState<string | null>(null)

  useEffect(() => {
    void disk.path().then(setStatePath)
    // The registry is the truth here, not our own state file: the user may
    // have removed the entry with any of the several tools Windows offers.
    void startup
      .enabled()
      .then(setAutostart)
      .catch(() => setAutostart(false))
    void contextMenu
      .enabled()
      .then(setMenu)
      .catch(() => setMenu(false))
  }, [])

  const fps = fpsDraft ?? status?.fps ?? store.settings.fps

  // The draft is held until the engine is seen to agree, rather than dropped
  // the moment it is sent: status is polled, so clearing it straight away
  // showed the old value again for up to a poll and read as the slider
  // snapping back under the cursor.
  useEffect(() => {
    if (fpsDraft !== null && status?.fps === fpsDraft) setFpsDraft(null)
  }, [status?.fps, fpsDraft])

  async function apply<T>(work: () => Promise<T>, next: Partial<Store['settings']>) {
    await work()
    onChange({ ...store, settings: { ...store.settings, ...next } })
    onRefresh()
  }

  function pushFps() {
    if (fpsDraft === null || fpsDraft === status?.fps) return
    void apply(() => engine.setFps(fpsDraft), { fps: fpsDraft })
  }

  function pushVisual() {
    if (
      status &&
      status.brightness === visual.brightness &&
      status.saturation === visual.saturation &&
      status.blur === visual.blur
    ) {
      return
    }
    void apply(
      () => engine.setVisual(visual.brightness, visual.saturation, visual.blur),
      visual,
    )
  }

  const duck = status?.duck ?? store.settings.duck

  function pushSound() {
    if (status && status.sound === sound && status.volume === volume) return
    void apply(() => engine.setSound(sound, volume, duck), { sound, volume })
  }


  return (
    <>
      <header className="view-head">
        <div>
          <h1 className="view-title">Ayarlar</h1>
          <p className="view-sub">Motor çalışırken anında uygulanır.</p>
        </div>
      </header>

      <section className="card">
        <h2 className="card-title">Kare hızı</h2>
        <p className="card-sub">
          Donanımına göre otomatik seçildi. Ekran görünmüyorken ve pilde zaten
          düşürülüyor — bu tavan.
        </p>
        <div className="row">
          <input
            type="range"
            min={10}
            max={120}
            step={5}
            value={fps}
            onChange={(e) => setFpsDraft(Number(e.target.value))}
            {...releases(pushFps)}
          />
          <span className="fps-value">{fps} fps</span>
        </div>
      </section>

      <section className="card">
        <h2 className="card-title">Pil</h2>
        <p className="card-sub">
          Priz varken yukarıdaki tavan geçerli. Fişten çekilince duvar kağıdı
          ilk geri çekilen şey olmalı —{' '}
          {status
            ? status.on_battery
              ? `şu an pilde (%${status.battery_percent})`
              : 'şu an prizde'
            : 'motor çalışmıyor'}
          {status?.saver && ', pil tasarrufu açık'}.
        </p>
        <div className="options">
          {BATTERY_RATES.map((rate) => (
            <button
              key={rate.value}
              className="option compact"
              data-active={(status?.battery_fps ?? store.settings.batteryFps) === rate.value}
              onClick={() =>
                void apply(
                  () =>
                    engine.setPower(
                      rate.value,
                      status?.pause_on_saver ?? store.settings.pauseOnSaver,
                    ),
                  { batteryFps: rate.value },
                )
              }
            >
              {rate.label}
            </button>
          ))}
        </div>
        <div className="row">
          <label className="toggle">
            <input
              type="checkbox"
              checked={status?.pause_on_saver ?? store.settings.pauseOnSaver}
              onChange={(e) =>
                void apply(
                  () =>
                    engine.setPower(
                      status?.battery_fps ?? store.settings.batteryFps,
                      e.target.checked,
                    ),
                  { pauseOnSaver: e.target.checked },
                )
              }
            />
            <span>Pil tasarrufu açıkken tamamen dondur</span>
          </label>
        </div>
        <p className="card-sub">
          Dondurmak duvar kağıdını kaldırmaz — son kare ekranda kalır, çözme
          ve çizme durur.
        </p>
      </section>

      <section className="card">
        <h2 className="card-title">Ölçekleme</h2>
        <p className="card-sub">
          Video ile ekranın en boy oranı tutmadığında ne olacağı.
        </p>
        <div className="options">
          {FITS.map((fit) => (
            <button
              key={fit.value}
              className="option"
              data-active={(status?.fit ?? store.settings.fit) === fit.value}
              onClick={() => void apply(() => engine.setFit(fit.value), { fit: fit.value })}
            >
              <span className="option-label">{fit.label}</span>
              <span className="option-help">{fit.help}</span>
            </button>
          ))}
        </div>
      </section>

      <section className="card">
        <h2 className="card-title">Oynatma</h2>
        <p className="card-sub">
          Hız videonun kendi karelerini yeniden zamanlar — çözücüden fazladan
          hiçbir şey istenmez, yavaşlatmak işi azaltır bile.
        </p>
        <div className="options">
          {SPEEDS.map((speed) => (
            <button
              key={speed.value}
              className="option compact"
              data-active={(status?.speed ?? store.settings.speed) === speed.value}
              onClick={() =>
                void apply(() => engine.setSpeed(speed.value), { speed: speed.value })
              }
            >
              {speed.label}
            </button>
          ))}
        </div>
      </section>

      <section className="card">
        <h2 className="card-title">Geçiş</h2>
        <p className="card-sub">
          Bir duvar kağıdından diğerine geçerken. Geçiş sırasında eski karenin
          bir kopyası tutulur ve biter bitmez bırakılır.
        </p>
        <div className="options">
          {FADES.map((fade) => (
            <button
              key={fade.value}
              className="option compact"
              data-active={(status?.fade_ms ?? store.settings.fadeMs) === fade.value}
              onClick={() =>
                void apply(() => engine.setFade(fade.value), { fadeMs: fade.value })
              }
            >
              {fade.label}
            </button>
          ))}
        </div>
      </section>

      <section className="card">
        <h2 className="card-title">Liste geçişi</h2>
        <p className="card-sub">
          Bir ekrana liste atandığında sıradaki klibe ne zaman geçilecek.
        </p>
        <div className="options">
          {INTERVALS.map((interval) => (
            <button
              key={interval.value}
              className="option compact"
              data-active={
                (status?.interval_secs ?? store.settings.intervalSecs) === interval.value
              }
              onClick={() =>
                void apply(() => engine.setInterval(interval.value), {
                  intervalSecs: interval.value,
                })
              }
            >
              {interval.label}
            </button>
          ))}
        </div>
      </section>

      <section className="card">
        <h2 className="card-title">Görünüm</h2>
        <p className="card-sub">
          Duvar kağıdının kendisi değişmez — bunlar ekrana çizilirken
          uygulanır, her ekranda aynı.
        </p>

        <div className="row">
          <label className="slider-label">Parlaklık</label>
          <input
            type="range"
            min={0.2}
            max={1.6}
            step={0.05}
            value={visual.brightness}
            onChange={(e) => setVisual({ ...visual, brightness: Number(e.target.value) })}
            {...releases(pushVisual)}
          />
          <span className="fps-value">{Math.round(visual.brightness * 100)}%</span>
        </div>

        <div className="row">
          <label className="slider-label">Doygunluk</label>
          <input
            type="range"
            min={0}
            max={2}
            step={0.05}
            value={visual.saturation}
            onChange={(e) => setVisual({ ...visual, saturation: Number(e.target.value) })}
            {...releases(pushVisual)}
          />
          <span className="fps-value">{Math.round(visual.saturation * 100)}%</span>
        </div>

        <div className="row">
          <label className="slider-label">Bulanıklık</label>
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={visual.blur}
            onChange={(e) => setVisual({ ...visual, blur: Number(e.target.value) })}
            {...releases(pushVisual)}
          />
          <span className="fps-value">{Math.round(visual.blur * 100)}%</span>
        </div>

        <div className="row">
          <button
            onClick={() => {
              const plain = { brightness: 1, saturation: 1, blur: 0 }
              setVisual(plain)
              void apply(() => engine.setVisual(1, 1, 0), plain)
            }}
          >
            Sıfırla
          </button>
        </div>
      </section>

      <section className="card">
        <h2 className="card-title">Ses</h2>
        <p className="card-sub">
          Videonun kendi sesi, birincil ekrandaki duvar kağıdından. Her ekran
          kapandığında (tam ekran oyun, kilit ekranı) kendiliğinden susar.
        </p>
        <div className="row">
          <button
            className={sound ? 'primary' : undefined}
            onClick={() => {
              const next = !sound
              setSound(next)
              void apply(() => engine.setSound(next, volume, duck), { sound: next })
            }}
          >
            {sound ? 'Açık' : 'Kapalı'}
          </button>
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={volume}
            disabled={!sound}
            onChange={(e) => setVolume(Number(e.target.value))}
            {...releases(pushSound)}
          />
          <span className="fps-value">{Math.round(volume * 100)}%</span>
        </div>

        <div className="row">
          <label className="toggle">
            <input
              type="checkbox"
              checked={duck}
              onChange={(e) =>
                void apply(() => engine.setSound(sound, volume, e.target.checked), {
                  duck: e.target.checked,
                })
              }
            />
            <span>Başka bir uygulama ses çalarken geri çekil</span>
          </label>
          {status?.ducking && <span className="muted">şu an geri çekildi</span>}
        </div>
      </section>

      <section className="card">
        <h2 className="card-title">Kısayollar ve menü</h2>
        <p className="card-sub">
          Kısayollar masaüstünün her yerinde çalışır. Bir kombinasyon başka bir
          uygulamada kayıtlıysa yalnız o çalışmaz, diğerleri çalışır.
        </p>
        <div className="row">
          <label className="toggle">
            <input
              type="checkbox"
              checked={status?.hotkeys ?? store.settings.hotkeys}
              onChange={(e) =>
                void apply(() => engine.setHotkeys(e.target.checked), {
                  hotkeys: e.target.checked,
                })
              }
            />
            <span>
              Ctrl+Alt+→ sonraki · Ctrl+Alt+P dondur · Ctrl+Alt+M ses
            </span>
          </label>
        </div>
        <div className="row">
          <label className="toggle">
            <input
              type="checkbox"
              checked={menu}
              onChange={async (e) => {
                const next = e.target.checked
                try {
                  await contextMenu.set(next)
                  setMenu(next)
                } catch (err) {
                  setPackNote(String(err))
                }
              }}
            />
            <span>Explorer'da sağ tık → "Muivly duvar kağıdı yap"</span>
          </label>
        </div>
        <p className="card-sub">
          Menüden seçilen dosya her ekrana atanır. Pencere açılmaz — yalnız
          motora bir satır gider.
        </p>
      </section>

      <section className="card">
        <h2 className="card-title">Paketler</h2>
        <p className="card-sub">
          <code>.muivly</code> bir duvar kağıdını adı ve künyesiyle birlikte tek
          dosyada taşır. İçi zip, yani Muivly olmayan biri de açabilir.
          Kitaplıktaki her duvar kağıdının kendi "Paket yap" düğmesi var.
        </p>
        <div className="row">
          <button
            onClick={async () => {
              const file = await pickPackage()
              if (!file) return
              try {
                const imported = await pack.import(file)
                onChange({
                  ...store,
                  items: [
                    ...store.items,
                    {
                      id: crypto.randomUUID(),
                      path: imported.path,
                      title: imported.title,
                      added: Date.now(),
                    },
                  ],
                })
                setPackNote(
                  `"${imported.title}" kitaplığa eklendi${
                    imported.author ? ` · ${imported.author}` : ''
                  }`,
                )
              } catch (err) {
                setPackNote(String(err))
              }
            }}
          >
            Paket içe aktar
          </button>
          {packNote && <span className="muted">{packNote}</span>}
        </div>
      </section>

      <section className="card">
        <h2 className="card-title">Windows ile başlat</h2>
        <p className="card-sub">
          Açılışta yalnız motor başlar, bu pencere değil — son duvar kağıdını
          ve ayarlarını kendisi hatırlıyor. Ayar paneli açılmadığı için
          açılışta WebView belleği de harcanmaz.
        </p>
        <div className="row">
          <button
            className={autostart ? 'primary' : undefined}
            onClick={async () => {
              const next = !autostart
              try {
                await startup.set(next)
                setAutostart(next)
              } catch (e) {
                setStartupError(String(e))
              }
            }}
          >
            {autostart ? 'Açık' : 'Kapalı'}
          </button>
          {startupError && <span className="error-text">{startupError}</span>}
        </div>
      </section>

      <section className="card">
        <h2 className="card-title">Performans</h2>
        <p className="card-sub">
          Motorun kendi ölçümü, saniyede bir yenilenir. Görev
          Yöneticisi'ndeki `muivly-core` ile aynı sayılar.
        </p>
        <div className="stats">
          <div className="stat">
            <span className="stat-value">{status ? status.cpu.toFixed(1) : '—'}%</span>
            <span className="stat-label">CPU (tek çekirdek payı)</span>
          </div>
          <div className="stat">
            <span className="stat-value">{status ? status.ram_mb : '—'} MB</span>
            <span className="stat-label">Bellek</span>
          </div>
          <div className="stat">
            <span className="stat-value">{status ? status.real_fps.toFixed(0) : '—'}</span>
            <span className="stat-label">Gerçek fps (tavan {status?.fps ?? '—'})</span>
          </div>
        </div>
      </section>

      <section className="card">
        <h2 className="card-title">Motor</h2>
        <p className="card-sub">
          Duvar kağıdı motoru ayrı bir işlem. Bu pencereyi kapatmak onu
          durdurmaz — X tuşu uygulamayı sistem tepsisine küçültür.
        </p>
        <div className="row">
          <button
            className={status?.frozen ? 'primary' : undefined}
            disabled={!status}
            title="Son kare ekranda kalır, çözme ve çizme durur"
            onClick={async () => {
              try {
                await engine.setFrozen(!status?.frozen)
              } catch {
                /* the next poll reports the engine is gone */
              }
              onRefresh()
            }}
          >
            {status?.frozen ? 'Donduruldu' : 'Dondur'}
          </button>
          <button
            className="danger"
            disabled={!status}
            onClick={async () => {
              try {
                await engine.quit()
              } catch {
                // Already gone is the outcome asked for, not a failure.
              }
              // The engine answers `ok` and then tears its windows down, so
              // asking straight away would still find it listening and the
              // panel would claim it is running.
              setTimeout(onRefresh, 400)
            }}
          >
            Motoru durdur
          </button>
          <span className="muted">
            {status ? `Çalışıyor · ${status.monitors.length} ekran` : 'Çalışmıyor'}
          </span>
        </div>
      </section>

      <section className="card">
        <h2 className="card-title">Veri</h2>
        <p className="card-sub">
          Kitaplık, listeler ve atamalar burada. Videolar kopyalanmaz, yalnız
          yolları saklanır.
        </p>
        <div className="path muted">{statePath}</div>
      </section>
    </>
  )
}
