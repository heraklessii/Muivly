/**
 * The library: everything the user has collected, as a wall of moving
 * previews.
 *
 * The grid is the whole view — a wallpaper is picked by how it looks, not by
 * its file name, so a tile is mostly picture and its controls stay out of the
 * way until the cursor is on one. Only the hovered tile plays.
 *
 * Three things are known about a file that the state file does not store:
 * its size and date (asked of Rust in one batch), and its resolution and
 * running time (reported by the preview that was going to decode it anyway).
 * Neither is worth a probe of its own, and both are what makes a tile
 * something you can judge rather than just look at.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import Thumb, { type Meta } from '../components/Thumb'
import {
  canOptimize,
  disk,
  displayName,
  duration,
  engine,
  fileSize,
  isShader,
  pack,
  pickPackageDestination,
  pickVideos,
  resolutionLabel,
  steam,
  worthLightening,
  oversizeFactor,
  type FileInfo,
  type Monitor,
  type Optimize,
  type ShaderFile,
} from '../api'
import { newId, withPaths, type Item, type Store } from '../store'

type Props = {
  store: Store
  monitors: Monitor[]
  /** A clip being rewritten smaller, or the one that just finished. */
  optimize: Optimize | null
  /** The settings each shader on screen declares for itself. Empty for
   *  everybody who has never put one on a screen. */
  shaders: ShaderFile[]
  onChange: (next: Store) => void
  onAssign: (monitorName: string, itemId: string) => void
  /** Ask for a fresh status — a shader setting is only visible once the
   *  engine has been asked about it again. */
  onRefresh: () => void
}

type Sort = 'added' | 'title' | 'size' | 'length'
type Filter = 'all' | 'video' | 'still' | 'shader' | 'live' | 'missing'

const STILL = /\.(png|jpe?g|bmp|webp|gif)$/i

const FILTERS: { id: Filter; label: string }[] = [
  { id: 'all', label: 'Tümü' },
  { id: 'video', label: 'Video' },
  { id: 'still', label: 'Görsel' },
  { id: 'shader', label: 'Shader' },
  { id: 'live', label: 'Ekranda' },
  { id: 'missing', label: 'Eksik' },
]

const SORTS: { id: Sort; label: string }[] = [
  { id: 'added', label: 'Son eklenen' },
  { id: 'title', label: 'İsme göre' },
  { id: 'size', label: 'Dosya boyutu' },
  { id: 'length', label: 'Süre' },
]

/** `.mp4` from a path, upper-cased, for the format badge. */
function extension(path: string): string {
  return (path.split('.').pop() ?? '').toUpperCase()
}

function date(epochMs: number): string {
  return new Date(epochMs).toLocaleDateString('tr-TR', {
    day: 'numeric',
    month: 'long',
    year: 'numeric',
  })
}

export default function Library({
  store,
  monitors,
  optimize,
  shaders,
  onChange,
  onAssign,
  onRefresh,
}: Props) {
  const [editing, setEditing] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  const [query, setQuery] = useState('')
  const [sort, setSort] = useState<Sort>('added')
  const [filter, setFilter] = useState<Filter>('all')
  const [error, setError] = useState<string | null>(null)
  const [importing, setImporting] = useState(false)
  /** The tile playing its preview, and the one with its monitor menu open. */
  const [hovered, setHovered] = useState<string | null>(null)
  const [menu, setMenu] = useState<string | null>(null)
  /** Removal asks once; this is the tile waiting for the second click. */
  const [confirming, setConfirming] = useState<string | null>(null)
  /** The wallpaper whose details are open in the side panel. */
  const [detailId, setDetailId] = useState<string | null>(null)
  /** The last rewrite whose output has already been added to the library.
   *  The engine keeps reporting a finished job until the next one starts,
   *  so without this the same file would be added on every poll. */
  const [added, setAdded] = useState<string | null>(null)
  const [selected, setSelected] = useState<string[]>([])

  /** Size and date per path, and whether that answer has arrived yet. */
  const [infos, setInfos] = useState<Record<string, FileInfo>>({})
  const [scanned, setScanned] = useState(false)

  /** Resolution and length per item id, as the previews report them. */
  const [meta, setMeta] = useState<Record<string, Meta>>({})

  // Joined on a newline, which a Windows path cannot contain — a space can.
  // The joined string is the effect's dependency, so the scan reruns when the
  // set of files changes rather than on every render that rebuilds the array.
  const paths = store.items.map((item) => item.path).join('\n')

  useEffect(() => {
    if (paths === '') {
      setInfos({})
      setScanned(true)
      return
    }

    let live = true
    void disk
      .infos(paths.split('\n'))
      .then((found) => {
        if (!live) return
        setInfos(found)
        setScanned(true)
      })
      .catch(() => setScanned(true))

    return () => {
      live = false
    }
  }, [paths])

  const noteMeta = useCallback((id: string, next: Meta) => {
    // Only ever set once per file: the callback fires on every remount of a
    // tile, and an unconditional write would re-render the whole grid.
    setMeta((current) => (current[id] ? current : { ...current, [id]: next }))
  }, [])

  /** Item id to the monitors currently showing it. */
  const onScreen = useMemo(() => {
    const map = new Map<string, string[]>()

    for (const [monitorName, assignment] of Object.entries(store.assignments)) {
      const ids =
        assignment?.kind === 'item'
          ? [assignment.id]
          : assignment?.kind === 'playlist'
            ? (store.playlists.find((p) => p.id === assignment.id)?.itemIds ?? [])
            : []

      for (const id of ids) {
        map.set(id, [...(map.get(id) ?? []), displayName(monitorName)])
      }
    }

    return map
  }, [store.assignments, store.playlists])

  const missing = useCallback(
    (item: Item) => scanned && infos[item.path] === undefined,
    [scanned, infos],
  )

  /**
   * Pull in whatever Wallpaper Engine wallpapers are already on this machine.
   *
   * Nothing is copied or converted — a workshop item is a folder with a video
   * in it, and this adds that path the same way the file picker would. Scene
   * and web wallpapers are skipped in Rust; they are Wallpaper Engine's own
   * runtime and there is nothing here that could play them.
   */
  async function importSteam() {
    setImporting(true)
    try {
      const found = await steam.scan()
      if (found.length === 0) {
        setError('Wallpaper Engine kitaplığında oynatılabilir bir şey bulunamadı.')
        return
      }

      const next = withPaths(store, found.map((item) => item.path))
      if (next === store) {
        setError('Bulunanların hepsi zaten kitaplıkta.')
        return
      }

      // The workshop titles are better than the file names they would
      // otherwise get, so they are carried across for the newly added ones.
      const titles = new Map(found.map((item) => [item.path, item.title]))
      onChange({
        ...next,
        items: next.items.map((item) =>
          store.items.some((existing) => existing.id === item.id)
            ? item
            : { ...item, title: titles.get(item.path) ?? item.title },
        ),
      })
      setError(null)
    } catch (e) {
      setError(String(e))
    } finally {
      setImporting(false)
    }
  }

  async function add() {
    try {
      const next = withPaths(store, await pickVideos())
      if (next !== store) onChange(next)
      setError(null)
    } catch (e) {
      // A picker that fails silently reads as a dead button.
      setError(`Dosya seçici açılamadı: ${e}`)
    }
  }

  /** Drop wallpapers from the library, and from everything pointing at them. */
  function removeMany(ids: string[]) {
    const gone = new Set(ids)
    setConfirming(null)
    setSelected((current) => current.filter((id) => !gone.has(id)))
    setDetailId((current) => (current && gone.has(current) ? null : current))

    onChange({
      ...store,
      items: store.items.filter((item) => !gone.has(item.id)),
      // Drop them from every playlist too, or those keep a dangling id that
      // silently shortens the list at playback time.
      playlists: store.playlists.map((p) => ({
        ...p,
        itemIds: p.itemIds.filter((id) => !gone.has(id)),
      })),
      assignments: Object.fromEntries(
        Object.entries(store.assignments).map(([monitor, assignment]) => [
          monitor,
          assignment?.kind === 'item' && gone.has(assignment.id) ? null : assignment,
        ]),
      ),
    })
  }

  function commitRename(item: Item) {
    const title = draft.trim()
    if (title) {
      onChange({
        ...store,
        items: store.items.map((i) => (i.id === item.id ? { ...i, title } : i)),
      })
    }
    setEditing(null)
  }

  /** One monitor needs no menu — the button applies straight away. */
  function apply(item: Item) {
    if (monitors.length <= 1) {
      const only = monitors[0]?.name
      if (only) onAssign(only, item.id)
      return
    }
    setMenu((current) => (current === item.id ? null : item.id))
  }

  /** Append to a playlist, skipping whatever is already in it. */
  function addToPlaylist(playlistId: string, ids: string[]) {
    onChange({
      ...store,
      playlists: store.playlists.map((p) =>
        p.id === playlistId
          ? { ...p, itemIds: [...p.itemIds, ...ids.filter((id) => !p.itemIds.includes(id))] }
          : p,
      ),
    })
  }

  /** A new playlist named after how many there already are. */
  function addToNewPlaylist(ids: string[]) {
    onChange({
      ...store,
      playlists: [
        ...store.playlists,
        { id: newId(), name: `Liste ${store.playlists.length + 1}`, itemIds: ids },
      ],
    })
  }

  function toggleSelected(id: string) {
    setSelected((current) =>
      current.includes(id) ? current.filter((other) => other !== id) : [...current, id],
    )
  }

  const needle = query.trim().toLocaleLowerCase('tr')

  const shown = useMemo(() => {
    const list = store.items.filter((item) => {
      if (needle && !item.title.toLocaleLowerCase('tr').includes(needle)) return false
      if (filter === 'video') return !STILL.test(item.path) && !isShader(item.path)
      if (filter === 'still') return STILL.test(item.path)
      if (filter === 'shader') return isShader(item.path)
      if (filter === 'live') return onScreen.has(item.id)
      if (filter === 'missing') return scanned && infos[item.path] === undefined
      return true
    })

    return list.sort((a, b) => {
      if (sort === 'title') return a.title.localeCompare(b.title, 'tr')
      if (sort === 'size') return (infos[b.path]?.size ?? 0) - (infos[a.path]?.size ?? 0)
      if (sort === 'length') return (meta[b.id]?.seconds ?? 0) - (meta[a.id]?.seconds ?? 0)
      return b.added - a.added
    })
  }, [store.items, needle, filter, sort, onScreen, infos, meta, scanned])

  const totals = useMemo(() => {
    let bytes = 0
    let stills = 0
    let gone = 0

    for (const item of store.items) {
      bytes += infos[item.path]?.size ?? 0
      if (STILL.test(item.path)) stills += 1
      if (scanned && infos[item.path] === undefined) gone += 1
    }

    return { bytes, stills, gone }
  }, [store.items, infos, scanned])

  // A finished rewrite goes straight into the library. The whole point of
  // the button is a lighter copy to use, and a file the user then has to go
  // and find is a button that did not finish its job.
  useEffect(() => {
    const output = optimize?.output
    if (!output || output === added) return
    setAdded(output)

    const next = withPaths(store, [output])
    if (next !== store) onChange(next)
  }, [optimize?.output, added, store, onChange])

  const detail = store.items.find((item) => item.id === detailId) ?? null
  const rewriting = optimize && !optimize.output && !optimize.error

  // The screen a clip would actually be rewritten for: the biggest one, which
  // is what the engine picks. A clip that fits the big screen is not too big
  // for the small one either.
  const biggest = useMemo(() => {
    return monitors.reduce(
      (best, monitor) =>
        monitor.width * monitor.height > best.width * best.height ? monitor : best,
      { width: 0, height: 0 } as { width: number; height: number },
    )
  }, [monitors])

  /** Whether this clip is decoding pixels the largest screen cannot show. */
  const oversized = useCallback(
    (id: string, path: string) => {
      const facts = meta[id]
      if (!facts || !canOptimize(path)) return false
      return worthLightening(facts, biggest)
    },
    [meta, biggest],
  )

  // Whichever shader is being looked at, with the settings its own file
  // declared. Only reported while it is on a screen: the settings come from
  // the open decoder, and a file nobody is showing has none open.
  const shaderOnScreen = detail
    ? (shaders.find((shader) => shader.path === detail.path) ?? null)
    : null

  /**
   * Held locally while a slider is being dragged, for the same reason the
   * settings panel does it: status is polled, and a slider bound straight to
   * the polled value jumps back under the cursor every time a poll lands.
   */
  const [shaderDraft, setShaderDraft] = useState<Record<string, number>>({})

  // Dropped when the drawer moves to a different file, so one shader's
  // values are never shown on another's sliders.
  useEffect(() => {
    setShaderDraft({})
  }, [detailId])

  const pushShader = useCallback(
    (path: string) => {
      const values = Object.entries(shaderDraft) as [string, number][]
      if (values.length === 0) return
      void engine
        .setShaderParams(path, values)
        .then(onRefresh)
        .catch((e) => setError(String(e)))
    },
    [shaderDraft, onRefresh],
  )

  // Escape closes whatever is open, innermost first.
  const escapeState = useRef({ detailId, menu, selected })
  escapeState.current = { detailId, menu, selected }
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key !== 'Escape') return
      const current = escapeState.current
      if (current.menu) setMenu(null)
      else if (current.detailId) setDetailId(null)
      else if (current.selected.length > 0) setSelected([])
    }

    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  return (
    <>
      <header className="view-head">
        <div>
          <h1 className="view-title">Kitaplık</h1>
          <p className="view-sub">
            {store.items.length} duvar kağıdı
            {totals.stills > 0 ? `, ${totals.stills} görsel` : ''}
            {totals.bytes > 0 ? ` · ${fileSize(totals.bytes)}` : ''}
            {totals.gone > 0 ? ` · ${totals.gone} dosya eksik` : ''}
          </p>
        </div>
        <div className="spacer" />
        <button onClick={() => void importSteam()} disabled={importing}>
          {importing ? 'Taranıyor…' : "Wallpaper Engine'den al"}
        </button>
        <button className="primary" onClick={add}>
          Ekle
        </button>
      </header>

      {error && <p className="error-text">{error}</p>}

      {store.items.length > 0 && (
        <div className="toolbar">
          <div className="chips">
            {FILTERS.map((entry) => {
              // "Eksik" only turns up when something actually is.
              if (entry.id === 'missing' && totals.gone === 0) return null

              return (
                <button
                  key={entry.id}
                  className="chip"
                  data-active={filter === entry.id}
                  onClick={() => setFilter(entry.id)}
                >
                  {entry.label}
                </button>
              )
            })}
          </div>

          <div className="spacer" />

          <input
            className="search"
            type="search"
            placeholder="Ara"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />

          <select value={sort} onChange={(e) => setSort(e.target.value as Sort)}>
            {SORTS.map((entry) => (
              <option key={entry.id} value={entry.id}>
                {entry.label}
              </option>
            ))}
          </select>
        </div>
      )}

      {store.items.length === 0 ? (
        <div className="card empty">
          <h2 className="card-title">Kitaplık boş</h2>
          <p>
            Video ekleyerek başla. Donanım çözücüsü olan her format çalışır —
            genelde mp4 (H.264/HEVC) ve webm (VP9/AV1). Duran görseller de
            olur; onlar ekrana geldikten sonra hiçbir maliyet çıkarmaz.
          </p>
          <div className="row center-row">
            <button className="primary" onClick={add}>
              Video ekle
            </button>
            <button onClick={() => void importSteam()} disabled={importing}>
              {importing ? 'Taranıyor…' : "Wallpaper Engine'den al"}
            </button>
          </div>
        </div>
      ) : shown.length === 0 ? (
        <div className="card empty">
          <p>
            {needle
              ? `“${query}” ile eşleşen bir şey yok.`
              : 'Bu filtreye uyan bir şey yok.'}
          </p>
        </div>
      ) : (
        <div className="wall">
          {shown.map((item) => {
            const screens = onScreen.get(item.id)
            const info = infos[item.path]
            const facts = meta[item.id]
            const isSelected = selected.includes(item.id)

            return (
              <article
                className="wp"
                key={item.id}
                data-live={screens !== undefined}
                data-selected={isSelected}
                data-missing={missing(item)}
                onMouseEnter={() => setHovered(item.id)}
                onMouseLeave={() => {
                  setHovered((current) => (current === item.id ? null : current))
                  setConfirming((current) => (current === item.id ? null : current))
                }}
              >
                <div className="wp-media">
                  <Thumb
                    path={item.path}
                    play={hovered === item.id && !missing(item)}
                    onMeta={(next) => noteMeta(item.id, next)}
                  />

                  {missing(item) ? (
                    <span className="wp-flag" data-tone="error">
                      Dosya yok
                    </span>
                  ) : (
                    screens && (
                      <span className="wp-flag" title={screens.join(', ')}>
                        <span className="dot" />
                        {screens.length > 1 ? `${screens.length} ekran` : screens[0]}
                      </span>
                    )
                  )}

                  <span className="wp-kind">
                    {facts ? resolutionLabel(facts.width, facts.height) : extension(item.path)}
                  </span>

                  {/* The clip is decoding pixels no screen here can show,
                      every frame, for as long as it plays. Said on the tile
                      because nobody goes looking for a rewrite button they
                      have no reason to suspect they need. */}
                  {oversized(item.id, item.path) && (
                    <span
                      className="wp-flag"
                      data-tone="warning"
                      data-corner="bottom"
                      title="Ekranından büyük — Hafiflet belleği kalıcı olarak düşürür"
                    >
                      Ekranından büyük
                    </span>
                  )}

                  {/* Selection is always reachable, and stays visible for
                      anything already picked. */}
                  <button
                    className="wp-check"
                    data-on={isSelected}
                    aria-label={isSelected ? 'Seçimi kaldır' : 'Seç'}
                    onClick={() => toggleSelected(item.id)}
                  >
                    {isSelected ? '✓' : ''}
                  </button>

                  <div className="wp-overlay">
                    <button
                      className="primary"
                      // With no engine there are no monitors to apply to;
                      // everything else on the card still works.
                      disabled={missing(item) || monitors.length === 0}
                      title={monitors.length === 0 ? 'Motor kapalı' : undefined}
                      onClick={() => apply(item)}
                    >
                      {monitors.length <= 1 ? 'Uygula' : 'Ekrana uygula'}
                    </button>

                    <div className="wp-overlay-row">
                      <button onClick={() => setDetailId(item.id)}>Ayrıntı</button>
                      <button
                        onClick={() => {
                          setEditing(item.id)
                          setDraft(item.title)
                        }}
                      >
                        Adlandır
                      </button>
                      <button
                        className="danger"
                        onClick={() =>
                          confirming === item.id
                            ? removeMany([item.id])
                            : setConfirming(item.id)
                        }
                      >
                        {confirming === item.id ? 'Emin misin?' : 'Kaldır'}
                      </button>
                    </div>
                  </div>

                  {menu === item.id && (
                    <>
                      {/* Anywhere-else click closes the menu. */}
                      <button
                        className="wp-scrim"
                        aria-label="Kapat"
                        onClick={() => setMenu(null)}
                      />
                      <div className="wp-menu">
                        {monitors.map((m) => (
                          <button
                            key={m.name}
                            onClick={() => {
                              setMenu(null)
                              onAssign(m.name, item.id)
                            }}
                          >
                            <span className="wp-menu-name">{displayName(m.name)}</span>
                            {m.primary && <span className="badge">birincil</span>}
                          </button>
                        ))}
                      </div>
                    </>
                  )}
                </div>

                <div className="wp-meta">
                  {editing === item.id ? (
                    <input
                      className="rename"
                      autoFocus
                      value={draft}
                      onChange={(e) => setDraft(e.target.value)}
                      onBlur={() => commitRename(item)}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') commitRename(item)
                        if (e.key === 'Escape') setEditing(null)
                      }}
                    />
                  ) : (
                    <>
                      <div className="wp-name" title={item.title}>
                        {item.title}
                      </div>
                      <div className="wp-facts">
                        {facts && facts.seconds > 0 && <span>{duration(facts.seconds)}</span>}
                        {info && <span>{fileSize(info.size)}</span>}
                        <span>{extension(item.path)}</span>
                      </div>
                    </>
                  )}
                </div>
              </article>
            )
          })}
        </div>
      )}

      {/* --------------------------------------------------------------
          Selection bar — only there while something is selected.
          -------------------------------------------------------------- */}
      {selected.length > 0 && (
        <div className="bulk">
          <span className="bulk-count">{selected.length} seçildi</span>

          <select
            value=""
            onChange={(e) => {
              const value = e.target.value
              if (value === 'new') addToNewPlaylist(selected)
              else if (value) addToPlaylist(value, selected)
              e.target.value = ''
            }}
          >
            <option value="">Listeye ekle…</option>
            {store.playlists.map((playlist) => (
              <option key={playlist.id} value={playlist.id}>
                {playlist.name}
              </option>
            ))}
            <option value="new">+ Yeni liste</option>
          </select>

          <button onClick={() => setSelected(shown.map((item) => item.id))}>
            Hepsini seç
          </button>
          <button className="danger" onClick={() => removeMany(selected)}>
            Kaldır
          </button>
          <button onClick={() => setSelected([])}>Vazgeç</button>
        </div>
      )}

      {/* --------------------------------------------------------------
          Detail panel — everything known about one wallpaper.
          -------------------------------------------------------------- */}
      {detail && (
        <>
          <button
            className="drawer-scrim"
            aria-label="Kapat"
            onClick={() => setDetailId(null)}
          />
          <aside className="drawer">
            <div className="drawer-head">
              <h2 className="card-title">Ayrıntı</h2>
              <div className="spacer" />
              <button className="icon" aria-label="Kapat" onClick={() => setDetailId(null)}>
                ×
              </button>
            </div>

            <div className="drawer-preview">
              <Thumb
                path={detail.path}
                play={!missing(detail)}
                onMeta={(next) => noteMeta(detail.id, next)}
              />
            </div>

            <input
              className="rename"
              value={editing === detail.id ? draft : detail.title}
              onFocus={() => {
                setEditing(detail.id)
                setDraft(detail.title)
              }}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={() => commitRename(detail)}
              onKeyDown={(e) => {
                if (e.key === 'Enter') e.currentTarget.blur()
              }}
            />

            <dl className="facts">
              <dt>Çözünürlük</dt>
              <dd>
                {meta[detail.id]
                  ? `${meta[detail.id].width}×${meta[detail.id].height}`
                  : 'okunuyor…'}
              </dd>

              <dt>Süre</dt>
              <dd>
                {meta[detail.id]
                  ? meta[detail.id].seconds > 0
                    ? duration(meta[detail.id].seconds)
                    : 'duran görsel'
                  : '—'}
              </dd>

              <dt>Boyut</dt>
              <dd>{infos[detail.path] ? fileSize(infos[detail.path].size) : '—'}</dd>

              <dt>Biçim</dt>
              <dd>{extension(detail.path)}</dd>

              <dt>Eklendi</dt>
              <dd>{date(detail.added)}</dd>

              <dt>Değiştirildi</dt>
              <dd>
                {infos[detail.path]?.modified ? date(infos[detail.path].modified) : '—'}
              </dd>

              <dt>Ekranda</dt>
              <dd>{onScreen.get(detail.id)?.join(', ') ?? 'hayır'}</dd>

              <dt>Listelerde</dt>
              <dd>
                {store.playlists
                  .filter((p) => p.itemIds.includes(detail.id))
                  .map((p) => p.name)
                  .join(', ') || 'hiçbiri'}
              </dd>
            </dl>

            <div className="drawer-path path">{detail.path}</div>

            {missing(detail) && (
              <p className="error-text">
                Bu dosya bulunduğu yerde değil. Taşındıysa yeniden eklemen
                gerekiyor.
              </p>
            )}

            <div className="section-label">Ekrana uygula</div>
            <div className="drawer-actions">
              {monitors.length === 0 ? (
                <p className="muted">Motor kapalıyken ekran listesi yok.</p>
              ) : (
                monitors.map((m) => (
                  <button
                    key={m.name}
                    disabled={missing(detail)}
                    onClick={() => onAssign(m.name, detail.id)}
                  >
                    {displayName(m.name)}
                    {m.primary && <span className="badge">birincil</span>}
                  </button>
                ))
              )}
            </div>

            <div className="section-label">Liste</div>
            <select
              value=""
              onChange={(e) => {
                const value = e.target.value
                if (value === 'new') addToNewPlaylist([detail.id])
                else if (value) addToPlaylist(value, [detail.id])
                e.target.value = ''
              }}
            >
              <option value="">Listeye ekle…</option>
              {store.playlists.map((playlist) => (
                <option key={playlist.id} value={playlist.id}>
                  {playlist.name}
                </option>
              ))}
              <option value="new">+ Yeni liste</option>
            </select>

            {shaderOnScreen && (
              <>
                <div className="section-label">Bu shader'ın ayarları</div>
                <p className="muted">
                  Dosyanın kendi başındaki <code>// param</code> satırlarından
                  geliyor. Değiştirmek anında uygulanır — yeniden derleme yok,
                  yeniden açma yok; sabit tamponda bir sayı değişir.
                </p>
                {shaderOnScreen.params.map((param) => (
                  <div className="row" key={param.name}>
                    <label className="slider-label" title={param.name}>
                      {param.label}
                    </label>
                    <input
                      type="range"
                      min={param.min}
                      max={param.max}
                      step={(param.max - param.min) / 100}
                      value={shaderDraft[param.name] ?? param.value}
                      onChange={(e) =>
                        setShaderDraft((current) => ({
                          ...current,
                          [param.name]: Number(e.target.value),
                        }))
                      }
                      onPointerUp={() => pushShader(detail.path)}
                      onLostPointerCapture={() => pushShader(detail.path)}
                      onKeyUp={() => pushShader(detail.path)}
                      onBlur={() => pushShader(detail.path)}
                    />
                    <span className="fps-value">
                      {(shaderDraft[param.name] ?? param.value).toFixed(2)}
                    </span>
                  </div>
                ))}
                <div className="drawer-actions">
                  <button
                    onClick={() => {
                      const defaults = Object.fromEntries(
                        shaderOnScreen.params.map((p) => [p.name, p.default]),
                      )
                      setShaderDraft(defaults)
                      void engine
                        .setShaderParams(
                          detail.path,
                          Object.entries(defaults) as [string, number][],
                        )
                        .then(onRefresh)
                        .catch((e) => setError(String(e)))
                    }}
                  >
                    Dosyanın kendi değerlerine dön
                  </button>
                </div>
              </>
            )}

            {canOptimize(detail.path) && (
              <>
                <div className="section-label">Hafiflet</div>

                {/* The number, before the explanation: "four times bigger
                    than your screen" is the whole argument, and it is
                    different for every clip and every desktop. */}
                {oversized(detail.id, detail.path) && (
                  <p className="notice">
                    Bu klip en büyük ekranının{' '}
                    <strong>
                      {oversizeFactor(meta[detail.id], biggest).toFixed(1)} katı
                    </strong>{' '}
                    piksel çözüyor — her kare, oynadığı sürece. Hafifletmek bunu
                    kalıcı olarak bitirir.
                  </p>
                )}

                <p className="muted">
                  Klibi en büyük ekranının çözünürlüğünde bir kez yeniden
                  yazar. Çözücünün belleğini belirleyen iki şey var: kare
                  boyutu ve kodlayıcının tuttuğu referans kare sayısı. Yeniden
                  yazarken ikisi de düşürülür — 4K bir klip 1080p ekranda
                  görünmeyen pikselleri her kare yeniden çözüyor. Bir kez
                  sürer, sonra hep ucuzdur.
                </p>
                <div className="drawer-actions">
                  <button
                    disabled={missing(detail) || Boolean(rewriting)}
                    onClick={() =>
                      void engine.optimize(detail.path).catch((e) => setError(String(e)))
                    }
                  >
                    {rewriting && optimize?.source === detail.path
                      ? `Yazılıyor… %${optimize.percent}`
                      : rewriting
                        ? "Başka bir klip yazılıyor"
                        : "Hafiflet"}
                  </button>
                </div>
                {optimize?.error && optimize.source === detail.path && (
                  <p className="error-text">{optimize.error}</p>
                )}
                {optimize?.output && optimize.source === detail.path && (
                  <p className="muted">Bitti — hafif kopya kitaplığa eklendi.</p>
                )}
              </>
            )}

            <div className="drawer-foot">
              <button
                onClick={() =>
                  void disk.reveal(detail.path).catch((e) => setError(String(e)))
                }
              >
                Klasörde göster
              </button>
              <button
                title="Adı ve künyesiyle birlikte tek dosyaya koy"
                onClick={async () => {
                  const destination = await pickPackageDestination(detail.title)
                  if (!destination) return
                  try {
                    await pack.export(detail.path, destination, detail.title, '', '')
                  } catch (e) {
                    setError(String(e))
                  }
                }}
              >
                Paket yap
              </button>
              <div className="spacer" />
              <button className="danger" onClick={() => removeMany([detail.id])}>
                Kitaplıktan kaldır
              </button>
            </div>
          </aside>
        </>
      )}
    </>
  )
}
