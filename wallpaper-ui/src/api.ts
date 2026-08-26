/**
 * Everything that crosses into Rust.
 *
 * Two destinations behind one surface: the engine (over the named pipe) and
 * the state file on disk. Keeping them in one module means a component never
 * has to know which is which.
 */
import { invoke } from '@tauri-apps/api/core'
import { open, save } from '@tauri-apps/plugin-dialog'

/**
 * What one screen has chosen not to share with the others.
 *
 * `null` in a field means "follow the desktop", which is what every monitor
 * does until someone opens its own panel.
 */
export type Overrides = {
  fit: Fit | null
  fps: number | null
  brightness: number | null
  saturation: number | null
  blur: number | null
}

export const noOverrides: Overrides = {
  fit: null,
  fps: null,
  brightness: null,
  saturation: null,
  blur: null,
}

/** Whether a monitor differs from the desktop at all. */
export function hasOverrides(own: Overrides | undefined): boolean {
  if (!own) return false
  return own.fit !== null || own.fps !== null || own.brightness !== null
}

export type MonitorState = {
  name: string
  enabled: boolean
  /** Which item of the playlist is currently on screen. */
  index: number
  items: string[]
  overrides: Overrides
}

export type EngineStatus = {
  fps: number
  paused: boolean
  /** Stopped on purpose, rather than because nothing is visible. */
  frozen: boolean
  fit: Fit
  interval_secs: number
  brightness: number
  saturation: number
  blur: number
  sound: boolean
  volume: number
  /** Whether the soundtrack stands down for other applications... */
  duck: boolean
  /** ...and whether it is doing so right now. */
  ducking: boolean
  /** Playback rate; 1 is the speed the file was authored at. */
  speed: number
  /** How long one wallpaper takes to replace another. 0 cuts. */
  fade_ms: number
  /** One wallpaper stretched across every screen. */
  span: boolean
  hotkeys: boolean
  /** The frame rate cap while unplugged; 0 means the same as plugged in. */
  battery_fps: number
  pause_on_saver: boolean
  /** What the machine is running on at the moment. */
  on_battery: boolean
  saver: boolean
  battery_percent: number
  /** Share of one core, 0-100, as the engine measures itself. */
  cpu: number
  ram_mb: number
  /** Frames actually presented per second — not the same as the fps cap. */
  real_fps: number
  /** The last thing the engine could not do, or null. */
  error: string | null
  monitors: MonitorState[]
}

export type Monitor = {
  name: string
  x: number
  y: number
  width: number
  height: number
  refresh_hz: number
  primary: boolean
  adapter: string
}

export type Fit = 'cover' | 'contain' | 'stretch'

/** One importable wallpaper found in the Wallpaper Engine workshop folders. */
export type Found = {
  title: string
  path: string
  preview: string | null
}

export const engine = {
  status: () => invoke<EngineStatus>('status'),
  monitors: () => invoke<Monitor[]>('monitors'),
  setPlaylist: (monitor: string, items: string[]) =>
    invoke<void>('set_playlist', { monitor, items }),
  next: (monitor: string) => invoke<void>('next_item', { monitor }),
  setEnabled: (monitor: string, enabled: boolean) =>
    invoke<void>('set_monitor_enabled', { monitor, enabled }),
  setFps: (fps: number) => invoke<void>('set_fps', { fps }),
  setFit: (fit: Fit) => invoke<void>('set_fit', { fit }),
  setInterval: (seconds: number) => invoke<void>('set_interval', { seconds }),
  setVisual: (brightness: number, saturation: number, blur: number) =>
    invoke<void>('set_visual', { brightness, saturation, blur }),
  setSound: (enabled: boolean, volume: number, duck: boolean) =>
    invoke<void>('set_sound', { enabled, volume, duck }),
  /** What to do about running on a battery. `batteryFps` 0 keeps one rate. */
  setPower: (batteryFps: number, pauseOnSaver: boolean) =>
    invoke<void>('set_power', { batteryFps, pauseOnSaver }),
  setSpeed: (speed: number) => invoke<void>('set_speed', { speed }),
  setFade: (milliseconds: number) => invoke<void>('set_fade', { milliseconds }),
  setSpan: (span: boolean) => invoke<void>('set_span', { span }),
  setHotkeys: (enabled: boolean) => invoke<void>('set_hotkeys', { enabled }),
  /** Stop the wallpaper where it stands, without taking it away. */
  setFrozen: (frozen: boolean) => invoke<void>('set_frozen', { frozen }),
  /** One monitor's own settings, all of them at once. */
  setOverrides: (monitor: string, own: Overrides) =>
    invoke<void>('set_overrides', { monitor, ...own }),
  quit: () => invoke<void>('quit_engine'),
  start: () => invoke<void>('start_engine', { video: null }),
  installed: () => invoke<boolean>('engine_installed'),
}

/** Starting the engine with Windows. The setting lives in the registry, not
 *  in our own state file, because that is where Windows looks for it. */
export const startup = {
  enabled: () => invoke<boolean>('autostart_enabled'),
  set: (enabled: boolean) => invoke<void>('set_autostart', { enabled }),
}

/** Wallpapers the user already owns in Wallpaper Engine. */
export const steam = {
  scan: () => invoke<Found[]>('scan_wallpaper_engine'),
}

/** "Muivly duvar kağıdı yap" in Explorer's right-click menu. */
export const contextMenu = {
  enabled: () => invoke<boolean>('context_menu_enabled'),
  set: (enabled: boolean) => invoke<void>('set_context_menu', { enabled }),
}

/** What a `.muivly` package says about itself. */
export type Manifest = {
  name: string
  author: string
  file: string
  preview: string | null
  license: string
  source: string
}

/** Where an imported package landed, and what it called itself. */
export type Imported = {
  path: string
  title: string
  author: string
  preview: string | null
}

/**
 * `.muivly` packages: a wallpaper plus its name and credit, in one file to
 * hand somebody. A zip underneath, so anyone can open one without Muivly.
 */
export const pack = {
  export: (source: string, destination: string, name: string, author: string, license: string) =>
    invoke<void>('export_package', { source, destination, name, author, license }),
  import: (packageFile: string) => invoke<Imported>('import_package', { package: packageFile }),
  inspect: (packageFile: string) => invoke<Manifest>('inspect_package', { package: packageFile }),
}

/**
 * motionbgs.com, which is a place to get wallpapers from rather than
 * anything Muivly depends on. Nothing here runs until the user opens the
 * browse view or presses download.
 */
export const web = {
  fetch: (url: string) => invoke<string>('web_fetch', { url }),
  download: (url: string, name: string) => invoke<string>('web_download', { url, name }),
  folder: () => invoke<string>('wallpapers_path'),
}

/** What the disk knows about a wallpaper file that the library does not. */
export type FileInfo = {
  size: number
  /** Last modified, epoch milliseconds. */
  modified: number
}

export const disk = {
  load: () => invoke<string | null>('load_state'),
  save: (json: string) => invoke<void>('save_state', { json }),
  path: () => invoke<string>('state_path'),
  exists: (path: string) => invoke<boolean>('file_exists', { path }),
  /** Sizes and dates for many files at once. A path that is gone is simply
   *  missing from the answer — which is how a moved file is spotted. */
  infos: (paths: string[]) => invoke<Record<string, FileInfo>>('file_infos', { paths }),
  reveal: (path: string) => invoke<void>('reveal', { path }),
}

/** `1.4 GB`, `812 MB`, `96 KB`. */
export function fileSize(bytes: number): string {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`
  if (bytes >= 1024 ** 2) return `${Math.round(bytes / 1024 ** 2)} MB`
  return `${Math.max(1, Math.round(bytes / 1024))} KB`
}

/** Seconds as `1:04`, or `12:03:40` for something absurdly long. */
export function duration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return '—'

  const whole = Math.round(seconds)
  const parts = [Math.floor(whole / 3600), Math.floor((whole % 3600) / 60), whole % 60]

  return (parts[0] > 0 ? parts : parts.slice(1))
    .map((part, index) => (index === 0 ? String(part) : String(part).padStart(2, '0')))
    .join(':')
}

/** `3840x2160` as `4K`, and the rest by height. */
export function resolutionLabel(width: number, height: number): string {
  if (!width || !height) return '—'
  if (height >= 4320) return '8K'
  if (height >= 2160) return '4K'
  if (height >= 1440) return '1440p'
  if (height >= 1080) return '1080p'
  if (height >= 720) return '720p'
  return `${width}×${height}`
}

/** Strip the `\\.\` prefix Windows puts on display device names. */
export function displayName(deviceName: string): string {
  return deviceName.replace(/^\\\\\.\\/, '')
}

/** Last path segment, without the extension — a sensible default title. */
export function fileTitle(path: string): string {
  const file = path.split(/[\\/]/).pop() ?? path
  return file.replace(/\.[^.]+$/, '')
}

/** Container formats worth offering — the engine hardware-decodes what is inside. */
const VIDEO_EXTENSIONS = ['mp4', 'webm', 'mkv', 'mov', 'm4v', 'avi']

/** Still images and GIFs, which the engine decodes with WIC instead. A photo
 *  costs nothing at all once it is on screen. */
const IMAGE_EXTENSIONS = ['gif', 'png', 'jpg', 'jpeg', 'bmp', 'webp']

/**
 * Ask the user for video files. Cancelling gives an empty array rather than
 * throwing, so a caller only has to handle a real failure.
 *
 * This is a plugin command, which means it needs `dialog:allow-open` in
 * src-tauri/capabilities. Without it the promise rejects and the button
 * appears dead — surface the error rather than swallowing it.
 */
/** Ask for one `.muivly` package to unpack. Cancelling gives null. */
export async function pickPackage(): Promise<string | null> {
  const picked = await open({
    multiple: false,
    filters: [{ name: 'Muivly paketi', extensions: ['muivly'] }],
  })
  if (!picked) return null
  return Array.isArray(picked) ? (picked[0] ?? null) : picked
}

/** Ask where to write a package. Cancelling gives null. */
export async function pickPackageDestination(title: string): Promise<string | null> {
  return await save({
    defaultPath: `${title}.muivly`,
    filters: [{ name: 'Muivly paketi', extensions: ['muivly'] }],
  })
}

export async function pickVideos(): Promise<string[]> {
  const picked = await open({
    multiple: true,
    filters: [
      { name: 'Duvar kağıdı', extensions: [...VIDEO_EXTENSIONS, ...IMAGE_EXTENSIONS] },
      { name: 'Video', extensions: VIDEO_EXTENSIONS },
      { name: 'Görsel', extensions: IMAGE_EXTENSIONS },
    ],
  })
  if (!picked) return []
  return Array.isArray(picked) ? picked : [picked]
}
