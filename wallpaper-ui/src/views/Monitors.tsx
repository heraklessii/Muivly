import Thumb from '../components/Thumb'
import { displayName, engine, type EngineStatus, type Monitor } from '../api'
import { assignmentLabel, type Assignment, type Store } from '../store'

type Props = {
  store: Store
  monitors: Monitor[]
  status: EngineStatus | null
  onAssign: (monitorName: string, assignment: Assignment) => void
  onRefresh: () => void
}

export default function Monitors({ store, monitors, status, onAssign, onRefresh }: Props) {
  // The layout preview draws the real desktop arrangement, scaled down. Two
  // screens side by side should look side by side here too, or picking the
  // right one becomes guesswork.
  const bounds = monitors.reduce(
    (acc, m) => ({
      left: Math.min(acc.left, m.x),
      top: Math.min(acc.top, m.y),
      right: Math.max(acc.right, m.x + m.width),
      bottom: Math.max(acc.bottom, m.y + m.height),
    }),
    { left: 0, top: 0, right: 0, bottom: 0 },
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
            {monitors.length} ekran. Aynı GPU'ya bağlı ekranlar aynı videoyu
            gösteriyorsa tek bir çözme işlemini paylaşır.
          </p>
        </div>
      </header>

      {monitors.length > 1 && (
        <div className="card">
          <h2 className="card-title">Yerleşim</h2>
          <p className="card-sub">Masaüstü düzenin, ölçekli.</p>
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
                    await engine.setEnabled(monitor.name, e.target.checked)
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
                  await engine.next(monitor.name)
                  onRefresh()
                }}
              >
                Sonraki
              </button>
            </div>
          </section>
        )
      })}
    </>
  )
}
