/**
 * Rules that change the wallpaper without being asked.
 *
 * Two triggers, and the difference between them is worth showing rather than
 * explaining: a time rule is a *start* time, so two of them are a day and a
 * night with no gap to leave by mistake, and a theme rule follows the switch
 * Windows already flips at sunset.
 *
 * A rule points at a playlist or at a single wallpaper from the library, not
 * at a path the user types. The engine stores paths, so this translates in
 * both directions — which is also what makes a rule survive a wallpaper being
 * renamed inside Muivly.
 */
import { useState } from 'react'

import { clockLabel, clockMinutes, fileTitle, type Rule } from '../api'
import type { Store } from '../store'

type Props = {
  store: Store
  rules: Rule[]
  onChange: (rules: Rule[]) => void
}

/** A rule's target, as something the library can name. */
function describe(store: Store, items: string[]): string {
  if (items.length === 0) return 'boş'

  const playlist = store.playlists.find((list) => {
    const paths = list.itemIds
      .map((id) => store.items.find((item) => item.id === id)?.path)
      .filter(Boolean)
    return paths.length === items.length && paths.every((path, i) => path === items[i])
  })
  if (playlist) return `${playlist.name} (${items.length} klip)`

  const known = store.items.find((item) => item.path === items[0])
  const name = known?.title ?? fileTitle(items[0])
  return items.length > 1 ? `${name} +${items.length - 1}` : name
}

/** Every choosable target: each playlist, then each single wallpaper. */
function targets(store: Store): { key: string; label: string; paths: string[] }[] {
  const lists = store.playlists.map((list) => ({
    key: `playlist:${list.id}`,
    label: `${list.name} (liste)`,
    paths: list.itemIds
      .map((id) => store.items.find((item) => item.id === id)?.path)
      .filter((path): path is string => Boolean(path)),
  }))

  const singles = store.items.map((item) => ({
    key: `item:${item.id}`,
    label: item.title,
    paths: [item.path],
  }))

  return [...lists, ...singles].filter((target) => target.paths.length > 0)
}

export default function Automation({ store, rules, onChange }: Props) {
  const choices = targets(store)
  const [kind, setKind] = useState<'time' | 'theme'>('time')
  const [at, setAt] = useState('07:00')
  const [dark, setDark] = useState(false)
  const [target, setTarget] = useState('')

  const picked = choices.find((choice) => choice.key === target) ?? choices[0]

  function add() {
    if (!picked) return

    const next: Rule = {
      kind,
      value: kind === 'time' ? clockMinutes(at) : dark ? 1 : 0,
      items: picked.paths,
    }

    // One rule per trigger. A second rule for 07:00 could only ever mean the
    // user is replacing the first, and two of them would leave which one wins
    // up to the order they happen to be in.
    const kept = rules.filter((rule) => !(rule.kind === next.kind && rule.value === next.value))
    onChange([...kept, next].sort((a, b) => a.value - b.value))
  }

  return (
    <section className="card">
      <h2 className="card-title">Otomasyon</h2>
      <p className="card-sub">
        Duvar kağıdı saate ya da Windows'un açık/koyu temasına göre kendisi
        değişsin. Yazdığın saat, o duvar kağıdının <em>başlama</em> saatidir:
        07:00 ve 20:00 koyarsan gündüz–gece olur. Tema kuralı varsa saatten
        önce gelir.
      </p>

      {rules.length > 0 && (
        <div className="chips">
          {rules.map((rule) => (
            <span key={`${rule.kind}${rule.value}`} className="chip removable" data-active="true">
              {rule.kind === 'time'
                ? clockLabel(rule.value)
                : rule.value === 1
                  ? 'Koyu tema'
                  : 'Açık tema'}
              {' → '}
              {describe(store, rule.items)}
              <button
                className="chip-remove"
                title="Kuralı kaldır"
                aria-label="Kuralı kaldır"
                onClick={() =>
                  onChange(rules.filter((other) => other !== rule))
                }
              >
                ×
              </button>
            </span>
          ))}
        </div>
      )}

      {choices.length === 0 ? (
        <p className="card-sub">
          Önce kitaplığa bir duvar kağıdı ekle; kuralın gösterecek bir şeyi
          olmalı.
        </p>
      ) : (
        <>
          <div className="options">
            <button
              className="option compact"
              data-active={kind === 'time'}
              onClick={() => setKind('time')}
            >
              Saat
            </button>
            <button
              className="option compact"
              data-active={kind === 'theme'}
              onClick={() => setKind('theme')}
            >
              Windows teması
            </button>
          </div>

          <div className="row">
            {kind === 'time' ? (
              <input type="time" value={at} onChange={(e) => setAt(e.target.value)} />
            ) : (
              <select value={dark ? 'dark' : 'light'} onChange={(e) => setDark(e.target.value === 'dark')}>
                <option value="light">Açık tema</option>
                <option value="dark">Koyu tema</option>
              </select>
            )}

            <select value={picked?.key ?? ''} onChange={(e) => setTarget(e.target.value)}>
              {choices.map((choice) => (
                <option key={choice.key} value={choice.key}>
                  {choice.label}
                </option>
              ))}
            </select>

            <button className="primary" onClick={add}>
              Kural ekle
            </button>
          </div>
        </>
      )}
    </section>
  )
}
