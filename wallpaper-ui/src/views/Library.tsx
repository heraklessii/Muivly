import { useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'

import Thumb from '../components/Thumb'
import { displayName, type Monitor } from '../api'
import { itemFromPath, type Item, type Store } from '../store'

type Props = {
  store: Store
  monitors: Monitor[]
  onChange: (next: Store) => void
  onAssign: (monitorName: string, itemId: string) => void
}

export default function Library({ store, monitors, onChange, onAssign }: Props) {
  const [editing, setEditing] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  const [query, setQuery] = useState('')

  async function add() {
    const picked = await open({
      multiple: true,
      filters: [{ name: 'Video', extensions: ['mp4', 'webm', 'mkv', 'mov', 'm4v', 'avi'] }],
    })
    if (!picked) return

    const paths = Array.isArray(picked) ? picked : [picked]
    // Adding the same file twice would give two entries that behave
    // identically and cannot be told apart in a playlist.
    const fresh = paths
      .filter((path) => !store.items.some((item) => item.path === path))
      .map(itemFromPath)

    if (fresh.length > 0) {
      onChange({ ...store, items: [...store.items, ...fresh] })
    }
  }

  function remove(item: Item) {
    onChange({
      ...store,
      items: store.items.filter((i) => i.id !== item.id),
      // Drop it from every playlist too, or they keep a dangling id that
      // silently shortens the list at playback time.
      playlists: store.playlists.map((p) => ({
        ...p,
        itemIds: p.itemIds.filter((id) => id !== item.id),
      })),
      assignments: Object.fromEntries(
        Object.entries(store.assignments).map(([monitor, assignment]) => [
          monitor,
          assignment?.kind === 'item' && assignment.id === item.id ? null : assignment,
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

  const needle = query.trim().toLocaleLowerCase('tr')
  const shown = needle
    ? store.items.filter((i) => i.title.toLocaleLowerCase('tr').includes(needle))
    : store.items

  return (
    <>
      <header className="view-head">
        <div>
          <h1 className="view-title">Kitaplık</h1>
          <p className="view-sub">
            {store.items.length} duvar kağıdı. Dosyalar kopyalanmaz, yalnız
            yolları saklanır.
          </p>
        </div>
        <div className="spacer" />
        {store.items.length > 0 && (
          <input
            className="search"
            type="search"
            placeholder="Ara"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        )}
        <button className="primary" onClick={add}>
          Video ekle
        </button>
      </header>

      {store.items.length === 0 ? (
        <div className="card empty">
          <h2 className="card-title">Kitaplık boş</h2>
          <p>
            Video ekleyerek başla. Donanım çözücüsü olan her format çalışır —
            genelde mp4 (H.264/HEVC) ve webm (VP9/AV1).
          </p>
          <button className="primary" onClick={add}>
            Video ekle
          </button>
        </div>
      ) : shown.length === 0 ? (
        <div className="card empty">
          <p>“{query}” ile eşleşen bir şey yok.</p>
        </div>
      ) : (
        <div className="grid">
          {shown.map((item) => (
            <article className="tile" key={item.id}>
              <Thumb path={item.path} />

              <div className="tile-body">
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
                  <button
                    className="tile-title"
                    title="Yeniden adlandır"
                    onClick={() => {
                      setEditing(item.id)
                      setDraft(item.title)
                    }}
                  >
                    {item.title}
                  </button>
                )}
                <div className="tile-path path">{item.path}</div>
              </div>

              <div className="tile-actions">
                {monitors.length === 1 ? (
                  <button onClick={() => onAssign(monitors[0].name, item.id)}>Uygula</button>
                ) : (
                  <select
                    className="assign"
                    value=""
                    onChange={(e) => {
                      if (e.target.value) onAssign(e.target.value, item.id)
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
                <button className="danger" onClick={() => remove(item)}>
                  Kaldır
                </button>
              </div>
            </article>
          ))}
        </div>
      )}
    </>
  )
}
