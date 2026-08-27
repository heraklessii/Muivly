import { useState } from 'react'

import Thumb from '../components/Thumb'
import {
  displayName,
  engine,
  hasOverrides,
  noOverrides,
  type EngineStatus,
  type Fit,
  type Monitor,
  type Overrides,
} from '../api'
import { assignmentLabel, type Assignment, type Store } from '../store'

type Props = {
  store: Store
  monitors: Monitor[]
  status: EngineStatus | null
  onAssign: (monitorName: string, assignment: Assignment) => void
  onRefresh: () => void
}

const FIT_LABELS: { value: Fit | ''; label: string }[] = [
  { value: '', label: 'Masaüstüyle aynı' },
  { value: 'cover', label: 'Doldur' },
  { value: 'contain', label: 'Sığdır' },
  { value: 'stretch', label: 'Ger' },
]

/**
 * Frame rates a single screen can be held to.
 *
 * Worth having because the decode is shared: a second monitor showing the
 * same wallpaper at 10 fps costs a tenth of the flips and no extra decoding
 * at all. On an integrated GPU pushing a 4K panel it is most of the cost.
 */
const OWN_RATES: { value: number; label: string }[] = [
  { value: 0, label: 'Masaüstüyle aynı' },
  { value: 30, label: '30 fps' },
  { value: 15, label: '15 fps' },
  { value: 10, label: '10 fps' },
  { value: 5, label: '5 fps' },
]

/** The per-screen panel: everything this monitor does differently. */
function OwnSettings({
  monitor,
  own,
  onSave,
}: {
  monitor: string
  own: Overrides
  onSave: (next: Overrides) => void
}) {
  const graded = own.brightness !== null

  return (
    <details className="own" open={hasOverrides(own)}>
      <summary>
        Bu ekrana özel
        {hasOverrides(own) && <span className="badge">açık</span>}
      </summary>

      <div className="row">
        <label className="slider-label">Ölçekleme</label>
        <select
          value={own.fit ?? ''}
          onChange={(e) => onSave({ ...own, fit: (e.target.value || null) as Fit | null })}
        >
          {FIT_LABELS.map((fit) => (
            <option key={fit.value} value={fit.value}>
              {fit.label}
            </option>
          ))}
        </select>

        <label className="slider-label">Kare hızı</label>
        <select
          value={own.fps ?? 0}
          onChange={(e) => onSave({ ...own, fps: Number(e.target.value) || null })}
        >
          {OWN_RATES.map((rate) => (
            <option key={rate.value} value={rate.value}>
              {rate.label}
            </option>
          ))}
        </select>
      </div>

      <div className="row">
        <label className="toggle">
          <input
            type="checkbox"
            checked={graded}
            onChange={(e) =>
              onSave(
                e.target.checked
                  ? { ...own, brightness: 1, saturation: 1, blur: 0 }
                  : { ...own, brightness: null, saturation: null, blur: null },
              )
            }
          />
          <span>Kendi parlaklık / doygunluk / bulanıklığı</span>
        </label>
      </div>

      {graded && (
        <>
          <div className="row">
            <label className="slider-label">Parlaklık</label>
            <input
              type="range"
              min={0.2}
              max={1.6}
              step={0.05}
              value={own.brightness ?? 1}
              onChange={(e) => onSave({ ...own, brightness: Number(e.target.value) })}
            />
            <span className="fps-value">{Math.round((own.brightness ?? 1) * 100)}%</span>
          </div>
          <div className="row">
            <label className="slider-label">Doygunluk</label>
            <input
              type="range"
              min={0}
              max={2}
              step={0.05}
              value={own.saturation ?? 1}
              onChange={(e) => onSave({ ...own, saturation: Number(e.target.value) })}
            />
            <span className="fps-value">{Math.round((own.saturation ?? 1) * 100)}%</span>
          </div>
          <div className="row">
            <label className="slider-label">Bulanıklık</label>
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={own.blur ?? 0}
              onChange={(e) => onSave({ ...own, blur: Number(e.target.value) })}
            />
            <span className="fps-value">{Math.round((own.blur ?? 0) * 100)}%</span>
          </div>
        </>
      )}

      {hasOverrides(own) && (
        <div className="row">
          <button onClick={() => onSave(noOverrides)}>
            {displayName(monitor)} masaüstünü izlesin
          </button>
        </div>
      )}
    </details>
  )
}

export default function Monitors({ store, monitors, status, onAssign, onRefresh }: Props) {
  // A scene is named before it is saved, and saving over a name replaces it —
  // which is what somebody means by saving twice.
  const [sceneName, setSceneName] = useState('')
  const [sceneError, setSceneError] = useState<string | null>(null)

  async function saveScene() {
    const name = sceneName.trim()
    if (name.length === 0) return
    setSceneError(null)
    try {
      await engine.saveScene(name)
      setSceneName('')
      onRefresh()
    } catch (e) {
      setSceneError(String(e))
    }
  }

  // The layout preview draws the real desktop arrangement, scaled down. Two
  // screens side by side should look side by side here too, or picking the
  // right one becomes guesswork.
  // Seeded from the first screen rather than from the origin: a desktop
  // whose screens all sit right of (0,0) would otherwise be drawn inside a
  // box stretched back to a corner no screen occupies, and every screen would
  // shrink into one edge of it.
  const first = monitors[0]
  const bounds = monitors.reduce(
    (acc, m) => ({
      left: Math.min(acc.left, m.x),
      top: Math.min(acc.top, m.y),
      right: Math.max(acc.right, m.x + m.width),
      bottom: Math.max(acc.bottom, m.y + m.height),
    }),
    first
      ? { left: first.x, top: first.y, right: first.x + first.width, bottom: first.y + first.height }
      : { left: 0, top: 0, right: 0, bottom: 0 },
  )

  const spanWidth = Math.max(1, bounds.right - bounds.left)
  const spanHeight = Math.max(1, bounds.bottom - bounds.top)
  const scale = Math.min(560 / spanWidth, 200 / spanHeight)

  return (
    <>
      <header className="view-head">
        <div>
          <h1 className="view-title">Ekranlar</h1>
          <p className="view-sub">
            {monitors.length} ekran. Aynı videoyu gösteren ekranlar birbirinin
            işini paylaşır, ikinci ekran neredeyse bedavaya gelir.
          </p>
        </div>
      </header>

      <div className="card">
        <h2 className="card-title">Sahneler</h2>
        <p className="card-sub">
          Hangi ekranda ne olduğunu bir isimle kaydeder, tek tıkla geri
          çağırırsın. Sahne yalnız duvar kağıtlarını hatırlar; parlaklık ve
          kare hızı gibi ayarlar sahneye göre değişmez.
        </p>

        <div className="row">
          <input
            type="text"
            className="grow"
            placeholder="Sahne adı — Çalışma, Gece, Oyun"
            value={sceneName}
            maxLength={40}
            onChange={(e) => setSceneName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') void saveScene()
            }}
          />
          <button
            className="primary"
            disabled={!status || sceneName.trim().length === 0}
            onClick={() => void saveScene()}
          >
            Şu anki hâli kaydet
          </button>
        </div>

        {sceneError && <p className="error-text">{sceneError}</p>}

        {(status?.scenes ?? []).length === 0 ? (
          <p className="card-sub">Henüz kayıtlı sahne yok.</p>
        ) : (
          <div className="options">
            {(status?.scenes ?? []).map((scene) => (
              <div className="scene" key={scene.name}>
                <button
                  className="option compact"
                  onClick={() => {
                    setSceneError(null)
                    void engine
                      .loadScene(scene.name)
                      .then(onRefresh)
                      .catch((e) => setSceneError(String(e)))
                  }}
                  title={scene.monitors
                    .map(
                      ([name, items]) =>
                        `${displayName(name)}: ${items.length === 0 ? 'boş' : `${items.length} öğe`}`,
                    )
                    .join(' · ')}
                >
                  {scene.name}
                </button>
                <button
                  className="icon"
                  aria-label={`${scene.name} sahnesini sil`}
                  onClick={() => {
                    setSceneError(null)
                    void engine
                      .deleteScene(scene.name)
                      .then(onRefresh)
                      .catch((e) => setSceneError(String(e)))
                  }}
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      {monitors.length > 1 && (
        <div className="card">
          <h2 className="card-title">Tek duvar kağıdını ekranlara yay</h2>
          <p className="card-sub">
            Tek bir video bütün ekranlara yayılır, her ekran kendi parçasını
            gösterir. Görüntü ekranlar arasında hizalı kalır ve tek video
            oynadığı için fazladan bir maliyeti olmaz.
          </p>
          <div className="row">
            <label className="toggle">
              <input
                type="checkbox"
                checked={status?.span ?? false}
                disabled={!status}
                onChange={async (e) => {
                  try {
                    await engine.setSpan(e.target.checked)
                  } catch {
                    /* the next poll reports the engine is gone */
                  }
                  onRefresh()
                }}
              />
              <span>Yayılsın</span>
            </label>
            {status?.span && (
              <span className="muted">
                Bir ekrana duvar kağıdı seçmek hepsini birden değiştirir.
              </span>
            )}
          </div>
        </div>
      )}

      {monitors.length > 1 && (
        <div className="card">
          <h2 className="card-title">Yerleşim</h2>
          <p className="card-sub">Ekranlarının dizilişi.</p>
          <div
            className="layout"
            style={{ width: spanWidth * scale, height: spanHeight * scale }}
          >
            {monitors.map((m) => (
              <div
                key={m.name}
                className="layout-screen"
                data-off={
                  status?.monitors.find((s) => s.name === m.name)?.enabled === false
                }
                style={{
                  left: (m.x - bounds.left) * scale,
                  top: (m.y - bounds.top) * scale,
                  width: m.width * scale,
                  height: m.height * scale,
                }}
              >
                <span>{displayName(m.name)}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {monitors.map((monitor) => {
        const live = status?.monitors.find((s) => s.name === monitor.name)
        const assignment = store.assignments[monitor.name] ?? null
        const playing = live?.items[live.index]
        const isPlaylist = (live?.items.length ?? 0) > 1

        return (
          <section className="card" key={monitor.name}>
            <div className="row">
              <div>
                <h2 className="card-title">
                  {displayName(monitor.name)}
                  {monitor.primary && <span className="badge">Birincil</span>}
                </h2>
                <p className="card-sub">
                  {monitor.width}×{monitor.height} · {monitor.refresh_hz} Hz ·{' '}
                  {monitor.adapter}
                </p>
              </div>
              <div className="spacer" />
              <label className="toggle">
                <input
                  type="checkbox"
                  checked={live?.enabled ?? true}
                  onChange={async (e) => {
                    // The engine going away between the poll and the click
                    // is ordinary; the next poll reports it as such.
                    try {
                      await engine.setEnabled(monitor.name, e.target.checked)
                    } catch {
                      /* ignored */
                    }
                    onRefresh()
                  }}
                />
                <span>Açık</span>
              </label>
            </div>

            <div className="assign-row">
              {playing ? (
                <Thumb path={playing} />
              ) : (
                <div className="thumb thumb-empty">
                  <span>—</span>
                </div>
              )}

              <div className="assign-body">
                <div className="assign-label">{assignmentLabel(store, assignment)}</div>
                {isPlaylist && live && (
                  <div className="muted">
                    {live.index + 1} / {live.items.length}
                  </div>
                )}
                {playing && <div className="path muted">{playing}</div>}
              </div>

              <div className="spacer" />

              <select
                className="assign"
                value={assignment ? `${assignment.kind}:${assignment.id}` : ''}
                onChange={(e) => {
                  const value = e.target.value
                  if (!value) return onAssign(monitor.name, null)
                  const [kind, id] = value.split(':')
                  onAssign(monitor.name, { kind: kind as 'item' | 'playlist', id })
                }}
              >
                <option value="">Atanmadı</option>
                {store.items.length > 0 && (
                  <optgroup label="Duvar kağıtları">
                    {store.items.map((item) => (
                      <option key={item.id} value={`item:${item.id}`}>
                        {item.title}
                      </option>
                    ))}
                  </optgroup>
                )}
                {store.playlists.length > 0 && (
                  <optgroup label="Listeler">
                    {store.playlists.map((playlist) => (
                      <option key={playlist.id} value={`playlist:${playlist.id}`}>
                        {playlist.name} ({playlist.itemIds.length})
                      </option>
                    ))}
                  </optgroup>
                )}
              </select>

              <button
                disabled={!isPlaylist}
                title={isPlaylist ? 'Sıradaki klibe geç' : 'Yalnız listelerde'}
                onClick={async () => {
                  try {
                    await engine.next(monitor.name)
                  } catch {
                    /* the next poll reports the engine is gone */
                  }
                  onRefresh()
                }}
              >
                Sonraki
              </button>
            </div>

            <OwnSettings
              monitor={monitor.name}
              own={live?.overrides ?? noOverrides}
              onSave={async (next) => {
                try {
                  await engine.setOverrides(monitor.name, next)
                } catch {
                  /* the next poll reports the engine is gone */
                }
                onRefresh()
              }}
            />
          </section>
        )
      })}
    </>
  )
}
