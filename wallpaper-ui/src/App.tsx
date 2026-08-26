import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'

type Status = {
  fps: number
  paused: boolean
  video: string | null
}

type Monitor = {
  name: string
  x: number
  y: number
  width: number
  height: number
  refresh_hz: number
  primary: boolean
  adapter: string
}

/** How often to ask the engine how it is doing. */
const POLL_MS = 1500

export default function App() {
  const [status, setStatus] = useState<Status | null>(null)
  const [monitors, setMonitors] = useState<Monitor[]>([])
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  // `fps` is edited locally while dragging so the slider does not fight the
  // poll, and only sent to the engine on release.
  const [fpsDraft, setFpsDraft] = useState<number | null>(null)

  const refresh = useCallback(async () => {
    try {
      setStatus(await invoke<Status>('status'))
    } catch {
      setStatus(null)
      setMonitors([])
      return
    }

    // Fetched separately, and never allowed to invalidate the status above:
    // the engine serves one connection at a time, so a second request can
    // legitimately arrive a moment too early and fail on its own.
    if (monitors.length === 0) {
      try {
        setMonitors(await invoke<Monitor[]>('monitors'))
      } catch {
        // Left empty; the next poll tries again.
      }
    }
  }, [monitors.length])

  useEffect(() => {
    void refresh()
    const timer = setInterval(() => void refresh(), POLL_MS)
    return () => clearInterval(timer)
  }, [refresh])

  async function act(work: () => Promise<unknown>) {
    setBusy(true)
    try {
      await work()
      setError(null)
      await refresh()
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  async function chooseVideo() {
    const picked = await open({
      multiple: false,
      filters: [{ name: 'Video', extensions: ['mp4', 'webm', 'mkv', 'mov', 'm4v'] }],
    })
    if (typeof picked === 'string') {
      await act(() => invoke('set_video', { path: picked }))
    }
  }

  const running = status !== null
  const playing = running && !status.paused
  const state = !running ? 'off' : status.paused ? 'paused' : 'playing'
  const stateLabel = !running
    ? 'Motor kapalı'
    : status.paused
      ? 'Duraklatıldı'
      : 'Oynatılıyor'

  return (
    <div className="app">
      <header className="topbar">
        <div className="wordmark">
          Mui<span>vly</span>
        </div>
        <div className="spacer" />
        <div className="pill" data-state={state}>
          <span className="dot" />
          {stateLabel}
        </div>
      </header>

      {!running ? (
        <section className="card">
          <div className="empty">
            <h2 className="card-title">Motor çalışmıyor</h2>
            <p>
              Wallpaper motoru ayrı bir işlem olarak çalışır. Bu pencereyi
              kapatsan da açık kalır.
            </p>
            <button
              className="primary"
              disabled={busy}
              onClick={() => act(() => invoke('start_engine', { video: null }))}
            >
              Motoru başlat
            </button>
            {error && <p className="error-text">{error}</p>}
          </div>
        </section>
      ) : (
        <>
          <section className="card">
            <h2 className="card-title">Duvar kağıdı</h2>
            <p className="card-sub">
              Video GPU'da çözülür. Ekran görünmüyorken çözme tamamen durur.
            </p>

            <div className="current">
              {status.video ? (
                // Direction is rtl in CSS so a long path is clipped from the
                // start and the file name stays readable.
                <span className="path">{status.video}</span>
              ) : (
                <span className="none">Seçili video yok — yer tutucu gradyan gösteriliyor</span>
              )}
            </div>

            <div className="row">
              <button className="primary" disabled={busy} onClick={chooseVideo}>
                Video seç
              </button>
              <button
                disabled={busy || !status.video}
                onClick={() => act(() => invoke('clear_video'))}
              >
                Kaldır
              </button>
            </div>

            {error && <p className="error-text">{error}</p>}
          </section>

          <section className="card">
            <h2 className="card-title">Kare hızı</h2>
            <p className="card-sub">
              Donanımına göre otomatik seçildi. Pilde ve ekran görünmüyorken
              zaten düşürülüyor.
            </p>

            <div className="row">
              <input
                type="range"
                min={10}
                max={120}
                step={5}
                value={fpsDraft ?? status.fps}
                onChange={(e) => setFpsDraft(Number(e.target.value))}
                onMouseUp={() => {
                  if (fpsDraft !== null && fpsDraft !== status.fps) {
                    void act(() => invoke('set_fps', { fps: fpsDraft }))
                  }
                  setFpsDraft(null)
                }}
              />
              <span className="fps-value">{fpsDraft ?? status.fps} fps</span>
            </div>
          </section>

          <section className="card">
            <h2 className="card-title">Ekranlar</h2>
            <p className="card-sub">
              {monitors.length} ekran. Aynı GPU'ya bağlı ekranlar tek bir
              çözme işlemini paylaşır.
            </p>

            <div className="monitors">
              {monitors.map((m) => (
                <div className="monitor" key={m.name}>
                  <div>
                    <div className="monitor-name">
                      {m.name.replace(/^\\\\\.\\/, '')}
                    </div>
                    <div className="monitor-meta">
                      {m.width}×{m.height} · {m.refresh_hz} Hz · {m.adapter}
                    </div>
                  </div>
                  <div className="spacer" />
                  {m.primary && <span className="badge">Birincil</span>}
                </div>
              ))}
            </div>
          </section>

          <p className="hint">
            {playing
              ? 'Pencereyi kapatmak uygulamayı sistem tepsisine küçültür; duvar kağıdı çalışmaya devam eder.'
              : 'Bir pencere ekranı tamamen kapattığı için çözme durduruldu.'}
          </p>
        </>
      )}
    </div>
  )
}
