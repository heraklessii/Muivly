import { useState } from 'react'

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

  const selected = store.playlists.find((p) => p.id === selectedId) ?? null

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

  function remove(playlist: Playlist) {
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

  const inList = new Set(selected?.itemIds ?? [])
  const available = store.items.filter((i) => !inList.has(i.id))

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
            {store.playlists.map((playlist) => (
              <button
                key={playlist.id}
                className="rail-item"
                data-active={playlist.id === selectedId}
                onClick={() => {
                  setSelectedId(playlist.id)
                  setRenaming(false)
                }}
              >
                <span className="rail-name">{playlist.name}</span>
                <span className="rail-count">{playlist.itemIds.length}</span>
              </button>
            ))}
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
                <button className="danger" onClick={() => remove(selected)}>
                  Listeyi sil
                </button>
              </div>

              <h3 className="section-label">Sıra</h3>
              {selected.itemIds.length === 0 ? (
                <p className="muted">
                  Boş. Aşağıdan ekleyerek başla — sıra buradaki gibi oynatılır.
                </p>
              ) : (
                <ol className="ordered">
                  {selected.itemIds.map((id, index) => {
                    const item = store.items.find((i) => i.id === id)
                    if (!item) return null
                    return (
                      <li className="ordered-row" key={id}>
                        <span className="ordinal">{index + 1}</span>
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
                    )
                  })}
                </ol>
              )}

              <h3 className="section-label">Kitaplıktan ekle</h3>
              {available.length === 0 ? (
                <p className="muted">
                  {store.items.length === 0
                    ? 'Kitaplık boş.'
                    : 'Kitaplıktaki her şey zaten bu listede.'}
                </p>
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
