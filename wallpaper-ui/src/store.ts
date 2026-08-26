/**
 * The library: what the user has collected, how they have grouped it, and
 * which monitor shows what.
 *
 * This is the frontend's own schema. Rust stores it as an opaque JSON blob
 * (see src-tauri/src/store.rs), so adding a field here needs no Rust change.
 * Nothing the engine needs lives in here — the engine is told about
 * wallpapers over the pipe.
 */
import { disk, fileTitle, type Fit } from './api'

export type Item = {
  id: string
  path: string
  title: string
  /** Epoch milliseconds, for sorting by "recently added". */
  added: number
}

export type Playlist = {
  id: string
  name: string
  /** Item ids, in playback order. */
  itemIds: string[]
}

export type Assignment =
  | { kind: 'item'; id: string }
  | { kind: 'playlist'; id: string }
  | null

export type Settings = {
  fps: number
  fit: Fit
  /** 0 means "advance when the clip ends" rather than on a clock. */
  intervalSecs: number
}

export type Store = {
  version: 1
  items: Item[]
  playlists: Playlist[]
  /** Keyed by monitor device name. */
  assignments: Record<string, Assignment>
  settings: Settings
}

export const emptyStore: Store = {
  version: 1,
  items: [],
  playlists: [],
  assignments: {},
  settings: { fps: 30, fit: 'cover', intervalSecs: 0 },
}

export function newId(): string {
  return crypto.randomUUID()
}

export async function loadStore(): Promise<Store> {
  const raw = await disk.load()
  if (!raw) return emptyStore

  try {
    const parsed = JSON.parse(raw) as Partial<Store>
    // Merged rather than trusted: a state file from an older build is
    // missing whatever was added since, and a library that vanishes because
    // one field moved is not a trade worth making.
    return {
      ...emptyStore,
      ...parsed,
      settings: { ...emptyStore.settings, ...parsed.settings },
      items: parsed.items ?? [],
      playlists: parsed.playlists ?? [],
      assignments: parsed.assignments ?? {},
    }
  } catch {
    return emptyStore
  }
}

export async function saveStore(store: Store): Promise<void> {
  await disk.save(JSON.stringify(store, null, 2))
}

/** Turn a file path into a library entry, titled from its file name. */
export function itemFromPath(path: string): Item {
  return { id: newId(), path, title: fileTitle(path), added: Date.now() }
}

/**
 * The list of file paths an assignment resolves to, in playback order.
 * A missing item or an emptied playlist resolves to nothing, which clears
 * that monitor rather than leaving it on a wallpaper the user deleted.
 */
export function resolve(store: Store, assignment: Assignment): string[] {
  if (!assignment) return []

  if (assignment.kind === 'item') {
    const item = store.items.find((i) => i.id === assignment.id)
    return item ? [item.path] : []
  }

  const playlist = store.playlists.find((p) => p.id === assignment.id)
  if (!playlist) return []

  return playlist.itemIds
    .map((id) => store.items.find((i) => i.id === id))
    .filter((i): i is Item => i !== undefined)
    .map((i) => i.path)
}

/** A human label for what a monitor is set to. */
export function assignmentLabel(store: Store, assignment: Assignment): string {
  if (!assignment) return 'Atanmadı'

  if (assignment.kind === 'item') {
    return store.items.find((i) => i.id === assignment.id)?.title ?? 'Eksik dosya'
  }

  const playlist = store.playlists.find((p) => p.id === assignment.id)
  return playlist ? `${playlist.name} (liste)` : 'Eksik liste'
}
