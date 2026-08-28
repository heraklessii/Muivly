/**
 * Playlists: a handful of wallpapers in an order, handed to a screen as one.
 *
 * The order is the whole content of the thing, so it is draggable — arrow
 * buttons stay for the keyboard, but nobody reorders twelve clips one step at
 * a time with a mouse.
 *
 * Dragging is done with pointer events rather than the HTML5 drag API: the
 * window accepts dropped files (see App.tsx), and Tauri's file drop handler
 * takes the platform's drag events for itself on Windows, which leaves
 * `dragstart` never firing. Pointer capture answers to nobody.
 */
import { useMemo, useRef, useState } from 'react'

import Thumb from '../components/Thumb'
import { displayName, type Monitor } from '../api'
import { newId, type Playlist, type Store } from '../store'

type Props = {
  store: Store
  monitors: Monitor[]
  onChange: (next: Store) => void
  onAssignPlaylist: (monitorName: string, playlistId: string) => void
}

export default function Playlists({ store, monitors, onChange, onAssignPlaylist }: Props) {
  const [selectedId, setSelectedId] = useState<string | null>(store.playlists[0]?.id ?? null)
  const [renaming, setRenaming] = useState(false)
  const [draft, setDraft] = useState('')
  /** Deletion asks once; this is the list waiting for the second click. */
  const [confirming, setConfirming] = useState<string | null>(null)
  /** Narrows the "add from library" row, which is otherwise every wallpaper
   *  the user owns as a wall of chips. */
  const [query, setQuery] = useState('')
  /** Which row is being dragged, as a position in the rendered list. */
  const [dragging, setDragging] = useState<number | null>(null)
  const rows = useRef<HTMLOListElement>(null)

  const selected = store.playlists.find((p) => p.id === selectedId) ?? null

  /** Playlist id to the screens playing it. */
  const onScreen = useMemo(() => {
    const map = new Map<string, string[]>()

    for (const [monitorName, assignment] of Object.entries(store.assignments)) {
      if (assignment?.kind !== 'playlist') continue
      map.set(assignment.id, [...(map.get(assignment.id) ?? []), displayName(monitorName)])
    }

    return map
  }, [store.assignments])

  function update(next: Playlist) {
    onChange({
      ...store,
      playlists: store.playlists.map((p) => (p.id === next.id ? next : p)),
    })
  }

  function create() {
    const playlist: Playlist = {
      id: newId(),
      name: `Liste ${store.playlists.length + 1}`,
      itemIds: [],
    }
    onChange({ ...store, playlists: [...store.playlists, playlist] })
    setSelectedId(playlist.id)
  }

  /** A second list with the same contents, to change without losing the
   *  first — the usual way somebody makes a variant of a working order. */
  function duplicate(playlist: Playlist) {
    const copy: Playlist = {
      id: newId(),
      name: `${playlist.name} kopyası`,
      itemIds: [...playlist.itemIds],
    }
    onChange({ ...store, playlists: [...store.playlists, copy] })
    setSelectedId(copy.id)
  }

  function remove(playlist: Playlist) {
    setConfirming(null)
    onChange({
      ...store,
      playlists: store.playlists.filter((p) => p.id !== playlist.id),
      assignments: Object.fromEntries(
        Object.entries(store.assignments).map(([monitor, assignment]) => [
          monitor,
          assignment?.kind === 'playlist' && assignment.id === playlist.id ? null : assignment,
        ]),
      ),
    })
    setSelectedId(null)
  }

  /** Move an entry one step, without letting it fall off either end. */
  function move(playlist: Playlist, index: number, delta: number) {
    const target = index + delta
    if (target < 0 || target >= playlist.itemIds.length) return

    const itemIds = [...playlist.itemIds]
    ;[itemIds[index], itemIds[target]] = [itemIds[target], itemIds[index]]
    update({ ...playlist, itemIds })
  }

  /** Lift one entry out and put it back down at another position. Not a
   *  swap: dropping the last clip at the top should push the rest down, not
   *  send whatever was at the top to the bottom. */
  function reorder(playlist: Playlist, from: number, to: number) {
    if (from === to || from < 0 || to < 0) return
    const itemIds = [...playlist.itemIds]
    const [moved] = itemIds.splice(from, 1)
    itemIds.splice(to, 0, moved)
    update({ ...playlist, itemIds })
  }

  /** Which rendered row the cursor is over, by its box rather than by any
   *  event of its own — the pointer is captured by the handle, so no other
   *  row hears about it. */
  function rowAt(y: number): number | null {
    const list = rows.current
    if (!list) return null

    const boxes = [...list.children].map((row) => row.getBoundingClientRect())
    const found = boxes.findIndex((box) => y < box.bottom)
    return found === -1 ? boxes.length - 1 : found
  }

  const byId = new Map(store.items.map((item) => [item.id, item]))

  // The rows actually drawn. An id whose wallpaper has since been removed
  // from the library draws nothing, so a position on screen is not the same
  // as a position in `itemIds`, and dragging has to translate between them.
  const entries = (selected?.itemIds ?? []).flatMap((id, index) => {
    const item = byId.get(id)
    return item ? [{ id, item, index }] : []
  })

  const inList = new Set(selected?.itemIds ?? [])
  const needle = query.trim().toLocaleLowerCase('tr')
  const available = store.items.filter(
    (item) =>
      !inList.has(item.id) &&
      (needle === '' || item.title.toLocaleLowerCase('tr').includes(needle)),
  )
  const outside = store.items.filter((item) => !inList.has(item.id)).length

  return (
    <>
      <header className="view-head">
        <div>
          <h1 className="view-title">Listeler</h1>
          <p className="view-sub">
            Sırayla oynatılacak duvar kağıtları. Bir listeyi ekrana atadığında
            sıradaki klibe kendi geçer.
          </p>
        </div>
        <div className="spacer" />
        <button className="primary" onClick={create}>
          Liste oluştur
        </button>
      </header>

      {store.playlists.length === 0 ? (
        <div className="card empty">
          <h2 className="card-title">Henüz liste yok</h2>
          <p>
            Bir liste, kitaplığındaki videolardan seçtiklerini sırayla oynatır.
            Geçiş anı Ayarlar'dan seçiliyor: klip bittiğinde ya da belirli bir
            süre sonra.
          </p>
          <button className="primary" onClick={create}>
            Liste oluştur
          </button>
        </div>
      ) : (
        <div className="split">
          <aside className="list-rail">
            {store.playlists.map((playlist) => {
              const screens = onScreen.get(playlist.id)

              return (
                <button
                  key={playlist.id}
                  className="rail-item"
                  data-active={playlist.id === selectedId}
                  title={screens ? `Ekranda: ${screens.join(', ')}` : undefined}
                  onClick={() => {
                    setSelectedId(playlist.id)
                    setRenaming(false)
                    setConfirming(null)
                    setQuery('')
                  }}
                >
                  {screens && <span className="dot" />}
                  <span className="rail-name">{playlist.name}</span>
                  <span className="rail-count">{playlist.itemIds.length}</span>
                </button>
              )
            })}
          </aside>

          {selected ? (
            <section className="card list-detail">
              <div className="row">
                {renaming ? (
                  <input
                    className="rename"
                    autoFocus
                    value={draft}
                    onChange={(e) => setDraft(e.target.value)}
                    onBlur={() => {
                      if (draft.trim()) update({ ...selected, name: draft.trim() })
                      setRenaming(false)
                    }}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') e.currentTarget.blur()
                      if (e.key === 'Escape') setRenaming(false)
                    }}
                  />
                ) : (
                  <button
                    className="card-title as-button"
                    title="Yeniden adlandır"
                    onClick={() => {
                      setRenaming(true)
                      setDraft(selected.name)
                    }}
                  >
                    {selected.name}
                  </button>
                )}
                <div className="spacer" />
                {monitors.length > 0 && (
                  <select
                    className="assign"
                    value=""
                    onChange={(e) => {
                      if (e.target.value) onAssignPlaylist(e.target.value, selected.id)
                      e.target.value = ''
                    }}
                  >
                    <option value="">Ekrana uygula…</option>
                    {monitors.map((m) => (
                      <option key={m.name} value={m.name}>
                        {displayName(m.name)}
                        {m.primary ? ' (birincil)' : ''}
                      </option>
                    ))}
                  </select>
                )}
                <button onClick={() => duplicate(selected)}>Çoğalt</button>
                <button
                  className="danger"
                  onClick={() =>
                    confirming === selected.id ? remove(selected) : setConfirming(selected.id)
                  }
                  onBlur={() => setConfirming(null)}
                >
                  {confirming === selected.id ? 'Emin misin?' : 'Listeyi sil'}
                </button>
              </div>

              {onScreen.get(selected.id) && (
                <p className="card-sub">
                  Şu an ekranda: {onScreen.get(selected.id)?.join(', ')}
                </p>
              )}

              <h3 className="section-label">Sıra</h3>
              {selected.itemIds.length === 0 ? (
                <p className="muted">
                  Boş. Aşağıdan ekleyerek başla — sıra buradaki gibi oynatılır.
                </p>
              ) : (
                <ol className="ordered" ref={rows}>
                  {entries.map(({ id, item, index }, position) => (
                    <li
                      className="ordered-row"
                      key={id}
                      data-dragging={dragging === position}
                    >
                      {/* The handle, so the buttons on the row still work as
                          buttons. Reordering happens as the cursor crosses a
                          row rather than on release: the list is short, and
                          watching it rearrange is the whole feedback. */}
                      <span
                        className="ordinal handle"
                        title="Sürükleyerek taşı"
                        onPointerDown={(e) => {
                          e.preventDefault()
                          e.currentTarget.setPointerCapture(e.pointerId)
                          setDragging(position)
                        }}
                        onPointerMove={(e) => {
                          if (dragging === null) return
                          const target = rowAt(e.clientY)
                          if (target === null || target === dragging) return
                          reorder(selected, entries[dragging].index, entries[target].index)
                          setDragging(target)
                        }}
                        onPointerUp={() => setDragging(null)}
                        onPointerCancel={() => setDragging(null)}
                      >
                        {index + 1}
                      </span>
                      <Thumb path={item.path} />
                      <span className="ordered-title">{item.title}</span>
                      <div className="spacer" />
                      <button
                        className="icon"
                        title="Yukarı"
                        aria-label={`${item.title} — bir yukarı taşı`}
                        disabled={index === 0}
                        onClick={() => move(selected, index, -1)}
                      >
                        ↑
                      </button>
                      <button
                        className="icon"
                        title="Aşağı"
                        aria-label={`${item.title} — bir aşağı taşı`}
                        disabled={index === selected.itemIds.length - 1}
                        onClick={() => move(selected, index, 1)}
                      >
                        ↓
                      </button>
                      <button
                        className="icon danger"
                        title="Listeden çıkar"
                        aria-label={`${item.title} — listeden çıkar`}
                        onClick={() =>
                          update({
                            ...selected,
                            itemIds: selected.itemIds.filter((x) => x !== id),
                          })
                        }
                      >
                        ×
                      </button>
                    </li>
                  ))}
                </ol>
              )}

              <h3 className="section-label">Kitaplıktan ekle</h3>
              {outside === 0 ? (
                <p className="muted">
                  {store.items.length === 0
                    ? 'Kitaplık boş.'
                    : 'Kitaplıktaki her şey zaten bu listede.'}
                </p>
              ) : (
                <>
                  <div className="row">
                    <input
                      className="search grow"
                      type="search"
                      placeholder={`${outside} duvar kağıdı arasında ara`}
                      value={query}
                      onChange={(e) => setQuery(e.target.value)}
                    />
                    <button
                      disabled={available.length === 0}
                      onClick={() =>
                        update({
                          ...selected,
                          itemIds: [...selected.itemIds, ...available.map((item) => item.id)],
                        })
                      }
                    >
                      {needle ? `Eşleşenleri ekle (${available.length})` : 'Hepsini ekle'}
                    </button>
                  </div>

                  {available.length === 0 ? (
                    <p className="muted">“{query}” ile eşleşen bir şey yok.</p>
                  ) : (
                    <div className="chips">
                      {available.map((item) => (
                        <button
                          key={item.id}
                          className="chip"
                          onClick={() =>
                            update({ ...selected, itemIds: [...selected.itemIds, item.id] })
                          }
                        >
                          + {item.title}
                        </button>
                      ))}
                    </div>
                  )}
                </>
              )}
            </section>
          ) : (
            <section className="card empty">
              <p>Soldan bir liste seç.</p>
            </section>
          )}
        </div>
      )}
    </>
  )
}
