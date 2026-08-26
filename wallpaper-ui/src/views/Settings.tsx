import { useEffect, useState } from 'react'

import { disk, engine, type EngineStatus, type Fit } from '../api'
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

  useEffect(() => {
    void disk.path().then(setStatePath)
  }, [])

  const fps = fpsDraft ?? status?.fps ?? store.settings.fps

  async function apply<T>(work: () => Promise<T>, next: Partial<Store['settings']>) {
    await work()
    onChange({ ...store, settings: { ...store.settings, ...next } })
    onRefresh()
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
            onMouseUp={() => {
              if (fpsDraft !== null && fpsDraft !== status?.fps) {
                void apply(() => engine.setFps(fpsDraft), { fps: fpsDraft })
              }
              setFpsDraft(null)
            }}
          />
          <span className="fps-value">{fps} fps</span>
        </div>
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
        <h2 className="card-title">Motor</h2>
        <p className="card-sub">
          Duvar kağıdı motoru ayrı bir işlem. Bu pencereyi kapatmak onu
          durdurmaz — X tuşu uygulamayı sistem tepsisine küçültür.
        </p>
        <div className="row">
          <button
            className="danger"
            disabled={!status}
            onClick={async () => {
              await engine.quit()
              onRefresh()
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
