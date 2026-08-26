import { useCallback, useEffect, useRef, useState } from 'react'

import { engine, type EngineStatus, type Monitor } from './api'
import {
  emptyStore,
  loadStore,
  resolve,
  saveStore,
  type Assignment,
  type Store,
} from './store'
import Library from './views/Library'
import Monitors from './views/Monitors'
import Playlists from './views/Playlists'
import Settings from './views/Settings'

type View = 'library' | 'playlists' | 'monitors' | 'settings'

const NAV: { id: View; label: string }[] = [
  { id: 'library', label: 'Kitaplık' },
  { id: 'playlists', label: 'Listeler' },
  { id: 'monitors', label: 'Ekranlar' },
  { id: 'settings', label: 'Ayarlar' },
]

/** How often to ask the engine how it is doing. */
const POLL_MS = 1500

export default function App() {
  const [view, setView] = useState<View>('library')
  const [store, setStore] = useState<Store>(emptyStore)
  const [loaded, setLoaded] = useState(false)
  const [status, setStatus] = useState<EngineStatus | null>(null)
  const [monitors, setMonitors] = useState<Monitor[]>([])
  const [error, setError] = useState<string | null>(null)
  const [starting, setStarting] = useState(false)

  useEffect(() => {
    void loadStore().then((next) => {
      setStore(next)
      setLoaded(true)
    })
  }, [])

  // Kept in a ref as well as state so `refresh` can read it without being
  // rebuilt on every change, which would restart the poll timer.
  const monitorsRef = useRef<Monitor[]>([])

  const refresh = useCallback(async () => {
    try {
      setStatus(await engine.status())
    } catch {
      setStatus(null)
      setMonitors([])
      monitorsRef.current = []
      return
    }

    // Fetched separately, and never allowed to invalidate the status above:
    // the engine serves one connection at a time, so a second request can
    // legitimately arrive a moment too early and fail on its own.
    if (monitorsRef.current.length === 0) {
      try {
        const list = await engine.monitors()
        monitorsRef.current = list
        setMonitors(list)
      } catch {
        // The next poll tries again.
      }
    }
  }, [])

  useEffect(() => {
    void refresh()
    const timer = setInterval(() => void refresh(), POLL_MS)
    return () => clearInterval(timer)
  }, [refresh])

  // Persisting is debounced: renaming a wallpaper types one character at a
  // time and each keystroke would otherwise be a disk write.
  const saveTimer = useRef<number | null>(null)
  const persist = useCallback(
    (next: Store) => {
      setStore(next)
      if (saveTimer.current !== null) clearTimeout(saveTimer.current)
      saveTimer.current = window.setTimeout(() => void saveStore(next), 300)
    },
    [],
  )

  /** Record an assignment and tell the engine about it in one step. */
  const assign = useCallback(
    async (monitorName: string, assignment: Assignment) => {
      const next: Store = {
        ...store,
        assignments: { ...store.assignments, [monitorName]: assignment },
      }
      persist(next)

      try {
        await engine.setPlaylist(monitorName, resolve(next, assignment))
        setError(null)
      } catch (e) {
        setError(String(e))
      }
      await refresh()
    },
    [store, persist, refresh],
  )

  // Re-apply everything once the engine appears, so a wallpaper survives the
  // engine being restarted without the user touching anything.
  const applied = useRef(false)
  useEffect(() => {
    if (!loaded || !status) {
      applied.current = false
      return
    }
    if (applied.current) return
    applied.current = true

    void (async () => {
      for (const [monitorName, assignment] of Object.entries(store.assignments)) {
        if (!assignment) continue
        try {
          await engine.setPlaylist(monitorName, resolve(store, assignment))
        } catch {
          // The engine went away again; the next appearance retries.
        }
      }
    })()
  }, [loaded, status, store])

  if (!loaded) {
    return <div className="app-loading" />
  }

  return (
    <div className="shell">
      <nav className="sidebar">
        <div className="wordmark">
          Mui<span>vly</span>
        </div>

        <div className="nav">
          {NAV.map((entry) => (
            <button
              key={entry.id}
              className="nav-item"
              data-active={view === entry.id}
              onClick={() => setView(entry.id)}
            >
              {entry.label}
            </button>
          ))}
        </div>

        <div className="pill sidebar-status" data-state={!status ? 'off' : status.paused ? 'paused' : 'playing'}>
          <span className="dot" />
          {!status ? 'Motor kapalı' : status.paused ? 'Duraklatıldı' : 'Oynatılıyor'}
        </div>
      </nav>

      <main className="content">
        {!status ? (
          <div className="card empty">
            <h2 className="card-title">Motor çalışmıyor</h2>
            <p>
              Duvar kağıdı motoru ayrı bir işlem olarak çalışır. Bu pencereyi
              kapatsan da açık kalır.
            </p>
            <button
              className="primary"
              disabled={starting}
              onClick={async () => {
                setStarting(true)
                try {
                  await engine.start()
                  setError(null)
                  // Give it a moment to open its pipe before the next poll.
                  setTimeout(() => void refresh(), 800)
                } catch (e) {
                  setError(String(e))
                } finally {
                  setStarting(false)
                }
              }}
            >
              Motoru başlat
            </button>
            {error && <p className="error-text">{error}</p>}
          </div>
        ) : (
          <>
            {error && <p className="error-text">{error}</p>}

            {view === 'library' && (
              <Library
                store={store}
                monitors={monitors}
                onChange={persist}
                onAssign={(monitorName, itemId) =>
                  void assign(monitorName, { kind: 'item', id: itemId })
                }
              />
            )}

            {view === 'playlists' && (
              <Playlists
                store={store}
                monitors={monitors}
                onChange={persist}
                onAssignPlaylist={(monitorName, playlistId) =>
                  void assign(monitorName, { kind: 'playlist', id: playlistId })
                }
              />
            )}

            {view === 'monitors' && (
              <Monitors
                store={store}
                monitors={monitors}
                status={status}
                onAssign={(monitorName, assignment) => void assign(monitorName, assignment)}
                onRefresh={() => void refresh()}
              />
            )}

            {view === 'settings' && (
              <Settings
                store={store}
                status={status}
                onChange={persist}
                onRefresh={() => void refresh()}
              />
            )}
          </>
        )}
      </main>
    </div>
  )
}
