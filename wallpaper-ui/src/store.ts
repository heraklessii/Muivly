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
  /** 1 leaves the picture alone; below dims, above brightens. */
  brightness: number
  /** 1 leaves the picture alone; 0 is greyscale. */
  saturation: number
  /** 0 is sharp, 1 is the widest blur on offer. */
  blur: number
  /** Off until asked for. A desktop that makes noise unprompted is a bug. */
  sound: boolean
  volume: number
  /** Stand down while another application is making sound. */
  duck: boolean
  /** Playback rate; 1 is the speed the file was authored at. */
  speed: number
  /** How long one wallpaper takes to replace another, in milliseconds. */
  fadeMs: number
  /** One wallpaper stretched across every screen. */
  span: boolean
  /** The three desktop-wide shortcuts. */
  hotkeys: boolean
  /** Frame rate cap while unplugged; 0 keeps the plugged-in rate. */
  batteryFps: number
  /** Freeze the wallpaper entirely while Windows battery saver is on. */
  pauseOnSaver: boolean
  /** How long the desktop must stay out of sight before the engine hands
   *  its decoders back. 0 keeps them open. */
  hibernateSecs: number
  /** How far the wallpaper answers to the sound coming out of the machine. */
  reactive: number
  /** How far it shifts under the cursor. */
  parallax: number
  /** A memory budget in megabytes; 0 leaves the detected tier's cap alone. */
  memoryMb: number
  /** Applications that freeze the wallpaper while they are in front. */
  apps: string[]
  /** How long the machine may sit untouched before the wallpaper stands
   *  still. 0 never does. */
  idleSecs: number
  /** The frame rate while the machine is busy with something else. 0 keeps
   *  one rate whatever else is running. */
  busyFps: number
  /** Honour Windows' "show animations" setting. */
  reduceMotion: boolean
  /** How far a photograph drifts on its own. 0 leaves it still. */
  drift: number
  /** Let the Windows accent colour follow the wallpaper. */
  accent: boolean
}

export type Store = {
  version: 1
  items: Item[]
  playlists: Playlist[]
  /** Keyed by monitor device name. */
  assignments: Record<string, Assignment>
  settings: Settings
  /** Whether the first-run walkthrough has been finished or skipped. */
  onboarded: boolean
}

export const emptyStore: Store = {
  version: 1,
  items: [],
  playlists: [],
  assignments: {},
  settings: {
    fps: 30,
    fit: 'cover',
    intervalSecs: 0,
    brightness: 1,
    saturation: 1,
    blur: 0,
    sound: false,
    volume: 0.5,
    duck: true,
    speed: 1,
    fadeMs: 400,
    span: false,
    hotkeys: true,
    batteryFps: 24,
    pauseOnSaver: true,
    hibernateSecs: 20,
    reactive: 0,
    parallax: 0,
    memoryMb: 0,
    apps: [],
    idleSecs: 300,
    busyFps: 10,
    reduceMotion: true,
    drift: 0,
    accent: false,
  },
  onboarded: false,
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
      // A state file written before onboarding existed belongs to someone who
      // already found their way around; a library is proof enough of that.
      onboarded: parsed.onboarded ?? (parsed.items?.length ?? 0) > 0,
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
 * Add files to the library, ignoring any already there.
 *
 * Adding the same file twice would give two entries that behave identically
 * and cannot be told apart in a playlist. Returns the store unchanged when
 * every path was a duplicate, so the caller can skip a needless write.
 */
export function withPaths(store: Store, paths: string[]): Store {
  const fresh = paths
    .filter((path) => !store.items.some((item) => item.path === path))
    .map(itemFromPath)

  return fresh.length > 0 ? { ...store, items: [...store.items, ...fresh] } : store
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
