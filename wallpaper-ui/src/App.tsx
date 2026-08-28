import { useCallback, useEffect, useRef, useState } from 'react'
import { getCurrentWebview } from '@tauri-apps/api/webview'

import { engine, isPlayable, pack, type EngineStatus, type Monitor } from './api'
import {
  emptyStore,
  loadStore,
  resolve,
  saveStore,
  withPaths,
  type Assignment,
  type Store,
} from './store'
import Browse from './views/Browse'
import Library from './views/Library'
import Monitors from './views/Monitors'
import Onboarding from './views/Onboarding'
import Playlists from './views/Playlists'
import Settings from './views/Settings'

type View = 'library' | 'browse' | 'playlists' | 'monitors' | 'settings'

/** One 16px line icon. The paths are drawn on a 16-unit grid so they line up
 *  with the text next to them without any per-icon nudging. */
function Icon({ id }: { id: View }) {
  const paths: Record<View, string> = {
    library: 'M2.5 3.5h11v9h-11z M2.5 6.5h11',
    browse: 'M8 2a6 6 0 1 0 0 12A6 6 0 0 0 8 2 M2.4 8h11.2 M8 2.2a9 9 0 0 1 0 11.6 M8 2.2a9 9 0 0 0 0 11.6',
    playlists: 'M2.5 4h7 M2.5 8h7 M2.5 12h5 M11.5 6.5v6 M11.5 6.5l2.5-1v6',
    monitors: 'M2.5 3.5h11v7h-11z M6 13h4 M8 10.5V13',
    settings: 'M8 5.6a2.4 2.4 0 1 0 0 4.8 2.4 2.4 0 0 0 0-4.8 M8 1.6v1.6 M8 12.8v1.6 M1.6 8h1.6 M12.8 8h1.6 M3.5 3.5l1.1 1.1 M11.4 11.4l1.1 1.1 M12.5 3.5l-1.1 1.1 M4.6 11.4l-1.1 1.1',
  }

  return (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
      <path
        d={paths[id]}
        fill="none"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  )
}

const NAV: { id: View; label: string }[] = [
  { id: 'library', label: 'Kitaplık' },
  { id: 'browse', label: 'Keşfet' },
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
  /** Whether something is being dragged over the window right now. */
  const [dropping, setDropping] = useState(false)

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
    let next: EngineStatus
    try {
      next = await engine.status()
      setStatus(next)
    } catch {
      setStatus(null)
      setMonitors([])
      monitorsRef.current = []
      return
    }

    // Fetched separately, and never allowed to invalidate the status above:
    // a second request can legitimately arrive a moment too early and fail
    // on its own.
    //
    // Refetched when the names disagree as well as when there is nothing:
    // status names every screen the engine knows about, so a display being
    // plugged in or unplugged shows up there first. Without this the screen
    // list stayed as it was at launch and "apply to this monitor" pointed at
    // a display that had been unplugged an hour ago.
    //
    // The names, not the count. Swapping one monitor for another leaves the
    // count where it was, and comparing only that kept a list of screens
    // that no longer exist — with the geometry the panel draws taken from
    // the display that had been unplugged.
    const known = new Set(monitorsRef.current.map((monitor) => monitor.name))
    const changed =
      known.size !== next.monitors.length ||
      next.monitors.some((monitor) => !known.has(monitor.name))

    if (changed) {
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
    // Closing the window only hides it — the app lives in the tray — and a
    // hidden WebView keeps its timers running. Polling an engine nobody is
    // looking at is a pipe round trip and a re-render of the whole library,
    // several times a minute, for a window that is not on screen.
    let timer: number | null = null

    const stop = () => {
      if (timer !== null) clearInterval(timer)
      timer = null
    }

    const start = () => {
      if (timer !== null) return
      void refresh()
      timer = window.setInterval(() => void refresh(), POLL_MS)
    }

    const onVisibility = () => (document.hidden ? stop() : start())

    onVisibility()
    document.addEventListener('visibilitychange', onVisibility)

    return () => {
      stop()
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [refresh])

  // Ctrl+1 to Ctrl+5 for the five views, the way every desktop application
  // with a sidebar works. Only with a modifier and only when the keystroke
  // is not going into a field: a bare "3" belongs to whatever the user is
  // typing in, and stealing it would make the rename box unusable.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!event.ctrlKey || event.altKey || event.shiftKey || event.metaKey) return
      const index = Number(event.key) - 1
      if (!Number.isInteger(index) || index < 0 || index >= NAV.length) return
      event.preventDefault()
      setView(NAV[index].id)
    }

    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

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

  // Read by the drop handler, which is registered once and must not be torn
  // down and rebuilt every time the library changes — a listener replaced
  // mid-drag misses the drop it was waiting for.
  const storeRef = useRef(store)
  storeRef.current = store

  /**
   * Files dropped on the window.
   *
   * The same thing the Ekle button does, minus the trip through the picker.
   * A `.muivly` package is unpacked first — dropping one is the obvious way
   * to open it, and the file inside is what belongs in the library.
   */
  const addDropped = useCallback(
    async (paths: string[]) => {
      const packages = paths.filter((path) => path.toLowerCase().endsWith('.muivly'))
      const files = paths.filter(isPlayable)

      if (packages.length === 0 && files.length === 0) {
        setError('Bırakılan dosyalar arasında oynatılabilir bir şey yok.')
        return
      }

      // The package knows a better name for its wallpaper than the file it
      // unpacked to does.
      const titles = new Map<string, string>()
      for (const packageFile of packages) {
        try {
          const imported = await pack.import(packageFile)
          files.push(imported.path)
          titles.set(imported.path, imported.title)
        } catch (e) {
          setError(String(e))
        }
      }

      const current = storeRef.current
      const next = withPaths(current, files)
      if (next === current) {
        setError('Bırakılanların hepsi zaten kitaplıkta.')
        return
      }

      persist({
        ...next,
        items: next.items.map((item) => ({
          ...item,
          title: titles.get(item.path) ?? item.title,
        })),
      })
      setError(null)
      setView('library')
    },
    [persist],
  )

  useEffect(() => {
    let unlisten: (() => void) | null = null
    let live = true

    void getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === 'enter') setDropping(true)
        else if (event.payload.type === 'leave') setDropping(false)
        else if (event.payload.type === 'drop') {
          setDropping(false)
          void addDropped(event.payload.paths)
        }
      })
      .then((off) => {
        // The window can be gone before the promise settles.
        if (live) unlisten = off
        else off()
      })
      .catch(() => {
        // Not fatal: the Ekle button is still there.
      })

    return () => {
      live = false
      unlisten?.()
    }
  }, [addDropped])

  /**
   * Record an assignment and tell the engine about it in one step.
   * Throws on failure, so a caller with its own place to put an error can
   * show one; `assignShown` below is the version for callers without.
   */
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
      } finally {
        await refresh()
      }
    },
    [store, persist, refresh],
  )

  const assignShown = useCallback(
    (monitorName: string, assignment: Assignment) => {
      void assign(monitorName, assignment).catch((e) => setError(String(e)))
    },
    [assign],
  )

  const startEngine = useCallback(async () => {
    await engine.start()
    // Give it a moment to open its pipe before the next poll.
    setTimeout(() => void refresh(), 800)
  }, [refresh])

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

  // Over whatever is on screen, the walkthrough included: dropping a file is
  // the fastest way through the "add a video" step too.
  const dropHint = dropping ? (
    <div className="dropzone" aria-hidden="true">
      <div className="dropzone-box">
        <strong>Bırak, kitaplığa eklensin</strong>
        <span>Video, görsel, shader ya da .muivly paketi</span>
      </div>
    </div>
  ) : null

  // The walkthrough owns the whole window: before there is a wallpaper the
  // sidebar has nothing worth navigating to.
  if (!store.onboarded) {
    return (
      <>
        {dropHint}
        <Onboarding
          store={store}
          status={status}
          monitors={monitors}
          onChange={persist}
          onStartEngine={startEngine}
          onApply={(monitorName, itemId) => assign(monitorName, { kind: 'item', id: itemId })}
          onFinish={() => persist({ ...store, onboarded: true })}
        />
      </>
    )
  }

  return (
    <div className="shell">
      {dropHint}
      <nav className="sidebar" aria-label="Ana menü">
        <div className="wordmark">
          Mui<span>vly</span>
        </div>

        <div className="nav">
          {NAV.map((entry, index) => (
            <button
              key={entry.id}
              className="nav-item"
              aria-current={view === entry.id ? 'page' : undefined}
              title={`${entry.label} (Ctrl+${index + 1})`}
              data-active={view === entry.id}
              onClick={() => setView(entry.id)}
            >
              <Icon id={entry.id} />
              {entry.label}
            </button>
          ))}
        </div>

        <div className="sidebar-foot">
          <div
            className="pill"
            data-state={!status ? 'off' : status.paused ? 'paused' : 'playing'}
          >
            <span className="dot" />
            {!status ? 'Motor kapalı' : status.paused ? 'Duraklatıldı' : 'Oynatılıyor'}
          </div>

          {/* What the engine is costing, where it is always in sight — the
              whole point of Muivly is that these numbers stay small. */}
          {status && (
            <div
              className="sidebar-meter"
              title={`Saniyede ${status.real_fps.toFixed(0)} kare · işlemcinin bir çekirdeğinin %${status.cpu.toFixed(0)} kadarı · ${status.ram_mb.toFixed(0)} MB bellek`}
            >
              <span>
                {status.real_fps.toFixed(0)}
                <em>fps</em>
              </span>
              <span>
                {status.cpu.toFixed(0)}
                <em>% cpu</em>
              </span>
              <span>
                {status.ram_mb.toFixed(0)}
                <em>MB</em>
              </span>
            </div>
          )}
        </div>
      </nav>

      <main className="content">
        {/* The engine being down stops wallpapers playing, not the app
            working: the library, downloads and playlists are all just files
            and state. So it is a banner rather than a wall. */}
        {!status && (
          <div className="banner">
            <div>
              <div className="banner-title">Motor çalışmıyor</div>
              <p className="muted">
                Duvar kağıdı motoru ayrı bir işlem olarak çalışır. Bu pencereyi
                kapatsan da açık kalır. Kitaplığını şimdi de düzenleyebilirsin;
                ekrana uygulamak için motor gerekiyor.
              </p>
            </div>
            <div className="spacer" />
            <button
              className="primary"
              disabled={starting}
              onClick={async () => {
                setStarting(true)
                try {
                  await startEngine()
                  setError(null)
                } catch (e) {
                  setError(String(e))
                } finally {
                  setStarting(false)
                }
              }}
            >
              {starting ? 'Başlatılıyor…' : 'Motoru başlat'}
            </button>
          </div>
        )}

        {error && (
          <p className="error-text" role="alert">
            {error}
          </p>
        )}

        {/* The engine reports its own failures — a codec with no hardware
            decoder, a file that moved. They belong next to whatever the UI
            itself has to say. */}
        {status?.error && (
          <p className="error-text" role="alert">
            {status.error}
          </p>
        )}

        {view === 'library' && (
          <Library
            store={store}
            monitors={monitors}
            optimize={status?.optimize ?? null}
            shaders={status?.shaders ?? []}
            onChange={persist}
            onRefresh={refresh}
            onAssign={(monitorName, itemId) =>
              assignShown(monitorName, { kind: 'item', id: itemId })
            }
          />
        )}

        {view === 'browse' && (
          <Browse
            have={store.items.map((item) => item.path)}
            onDownloaded={(path, title) => {
              const next = withPaths(store, [path])
              if (next === store) return
              // The site knows a better name for it than the file does.
              persist({
                ...next,
                items: next.items.map((item) =>
                  item.path === path ? { ...item, title } : item,
                ),
              })
            }}
          />
        )}

        {view === 'playlists' && (
          <Playlists
            store={store}
            monitors={monitors}
            onChange={persist}
            onAssignPlaylist={(monitorName, playlistId) =>
              assignShown(monitorName, { kind: 'playlist', id: playlistId })
            }
          />
        )}

        {/* These two are windows onto the running engine, so with no engine
            there is nothing for them to show. */}
        {view === 'monitors' &&
          (status ? (
            <Monitors
              store={store}
              monitors={monitors}
              status={status}
              onAssign={assignShown}
              onRefresh={() => void refresh()}
            />
          ) : (
            <div className="card empty">
              <p>Ekran listesi motordan geliyor. Motoru başlat.</p>
            </div>
          ))}

        {view === 'settings' &&
          (status ? (
            <Settings
              store={store}
              status={status}
              onChange={persist}
              onRefresh={() => void refresh()}
            />
          ) : (
            <div className="card empty">
              <p>Ayarlar motora yazılıyor. Motoru başlat.</p>
            </div>
          ))}
      </main>
    </div>
  )
}
