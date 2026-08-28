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

/** One automation rule: when the wallpaper changes itself, and to what. */
export type Rule = {
  kind: 'time' | 'theme'
  /** Minutes since midnight for `time`; 1 = dark, 0 = light for `theme`. */
  value: number
  items: string[]
}

/** One saved arrangement of wallpapers across the screens. */
export type Scene = {
  name: string
  /** Device name, and what that screen was showing. */
  monitors: [string, string[]][]
}

/** One setting a shader file declares for itself, and where it is set. */
export type ShaderParam = {
  name: string
  min: number
  max: number
  default: number
  value: number
  /** What to call it on screen. The name, when the file did not say. */
  label: string
}

/** A shader on screen, with the settings it asked for. */
export type ShaderFile = {
  path: string
  params: ShaderParam[]
}

/** A clip being rewritten smaller, or the one that just finished. */
export type Optimize = {
  source: string
  percent: number
  /** Where the smaller copy landed, once it has. */
  output: string | null
  error: string | null
}

export type EngineStatus = {
  fps: number
  paused: boolean
  /** Stopped on purpose, rather than because nothing is visible. */
  frozen: boolean
  fit: Fit
  interval_secs: number
  /** Whether a playlist plays in a drawn order rather than as written. */
  shuffle: boolean
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
  /** How long out of sight before the engine hands its decoders back. */
  hibernate_secs: number
  /** Whether they are handed back right now — the memory is not in use. */
  hibernating: boolean
  /** How far the wallpaper answers to sound, and to the cursor. 0-1 each. */
  reactive: number
  parallax: number
  /** A memory budget in megabytes; 0 is none. */
  memory_mb: number
  /** Applications that freeze the wallpaper while they are in front. */
  apps: string[]
  rules: Rule[]
  /** Named arrangements of wallpapers across the screens. */
  scenes: Scene[]
  /** The settings each shader on screen declares for itself. */
  shaders: ShaderFile[]
  /** How long the machine may sit untouched before the wallpaper stands
   *  still, and whether it is standing still right now. */
  idle_secs: number
  away: boolean
  /** The frame rate while the machine is busy with something else, whether it
   *  is, and how busy the last sample found the whole machine. */
  busy_fps: number
  busy: boolean
  load: number
  /** Whether Windows' reduce-motion setting is honoured. */
  reduce_motion: boolean
  /** How far a photograph drifts on its own, 0-1. */
  drift: number
  /** Whether the Windows accent colour follows the wallpaper. */
  accent: boolean
  /** How long the engine has been up, and how much of that it spent drawing
   *  nothing at all — the number this whole project is about. */
  uptime_secs: number
  resting_secs: number
  optimize: Optimize | null
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
  setShuffle: (shuffle: boolean) => invoke<void>('set_shuffle', { shuffle }),
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
  /** How long out of sight before the decoders are handed back. 0 never. */
  setHibernate: (seconds: number) => invoke<void>('set_hibernate', { seconds }),
  /** How far the wallpaper answers to sound and to the cursor. */
  setMotion: (reactive: number, parallax: number) =>
    invoke<void>('set_motion', { reactive, parallax }),
  /** A memory budget in megabytes; 0 leaves the tier's own cap in place. */
  setMemory: (megabytes: number) => invoke<void>('set_memory', { megabytes }),
  setApps: (names: string[]) => invoke<void>('set_apps', { names }),
  setRules: (rules: Rule[]) => invoke<void>('set_rules', { rules }),
  /** Rewrite one clip at the size of the largest screen, once. */
  optimize: (path: string) => invoke<void>('optimize', { path }),
  /** Stand still after this long with nobody touching the machine. 0 never. */
  setIdle: (seconds: number) => invoke<void>('set_idle', { seconds }),
  /** The frame rate while the machine is busy. 0 keeps one rate. */
  setBusyFps: (fps: number) => invoke<void>('set_busy_fps', { fps }),
  /** Honour Windows' "show animations" setting. */
  setReduceMotion: (enabled: boolean) => invoke<void>('set_reduce_motion', { enabled }),
  /** How far a photograph drifts on its own, 0-1. */
  setDrift: (drift: number) => invoke<void>('set_drift', { drift }),
  /** Let the Windows accent colour follow the wallpaper. */
  setAccent: (enabled: boolean) => invoke<void>('set_accent', { enabled }),
  /** One shader file's own settings, all of them at once. */
  setShaderParams: (path: string, values: [string, number][]) =>
    invoke<void>('set_shader_params', { path, values }),
  /** Save what is on every screen under a name, recall it, or forget it. */
  saveScene: (name: string) => invoke<void>('scene', { action: 'save', name }),
  loadScene: (name: string) => invoke<void>('scene', { action: 'load', name }),
  deleteScene: (name: string) => invoke<void>('scene', { action: 'delete', name }),
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

/** Whether this path is a shader rather than something to decode. */
export function isShader(path: string): boolean {
  return SHADER_EXTENSIONS.includes(path.split('.').pop()?.toLowerCase() ?? '')
}

/** Whether this is something the engine can put on a screen at all. The same
 *  set the file picker offers, for the paths that arrive without one — a
 *  file dropped on the window. */
export function isPlayable(path: string): boolean {
  const extension = path.split('.').pop()?.toLowerCase() ?? ''
  return (
    VIDEO_EXTENSIONS.includes(extension) ||
    IMAGE_EXTENSIONS.includes(extension) ||
    SHADER_EXTENSIONS.includes(extension)
  )
}

/** Whether rewriting this file smaller could help. Only video: a photo is
 *  decoded once and a shader is never decoded at all. */
export function canOptimize(path: string): boolean {
  return VIDEO_EXTENSIONS.includes(path.split('.').pop()?.toLowerCase() ?? '')
}

/**
 * How much bigger this clip is than the screen it will be shown on.
 *
 * The whole reason "Lighten" exists: a decoder's memory is its frame size
 * times its reference frames, and a 4K loop on a 1080p laptop is decoding
 * four times the pixels that screen can show, forever. Below the threshold
 * the rewrite is not worth the quality it costs.
 *
 * Returns 1 when there is nothing to gain, so a caller can treat the number
 * as "times too big" and show it.
 */
export function oversizeFactor(
  video: { width: number; height: number },
  screen: { width: number; height: number },
): number {
  if (!video.width || !video.height || !screen.width || !screen.height) return 1
  const factor = (video.width * video.height) / (screen.width * screen.height)
  return factor > 1 ? factor : 1
}

/** Whether rewriting is worth suggesting: a third again as many pixels as the
 *  screen can show is where the saving starts to be worth a re-encode. */
export function worthLightening(
  video: { width: number; height: number },
  screen: { width: number; height: number },
): boolean {
  return oversizeFactor(video, screen) >= 1.35
}

/** Seconds as `4 sa 12 dk`, or `12 dk` — for durations measured in hours. */
export function longDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return '0 dk'

  const hours = Math.floor(seconds / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  if (hours > 0) return `${hours} sa ${minutes} dk`
  if (minutes > 0) return `${minutes} dk`
  return `${Math.round(seconds)} sn`
}

/** `420` as `07:00` — minutes since midnight, the way a rule stores them. */
export function clockLabel(minutes: number): string {
  const clamped = Math.max(0, Math.min(24 * 60 - 1, Math.round(minutes)))
  const hh = String(Math.floor(clamped / 60)).padStart(2, '0')
  const mm = String(clamped % 60).padStart(2, '0')
  return `${hh}:${mm}`
}

/** `07:00` back to 420. Anything unparseable is midnight. */
export function clockMinutes(label: string): number {
  const [hh, mm] = label.split(':').map((part) => Number.parseInt(part, 10))
  if (!Number.isFinite(hh)) return 0
  return Math.max(0, Math.min(24 * 60 - 1, hh * 60 + (Number.isFinite(mm) ? mm : 0)))
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

/** Pixel shaders, which are not decoded at all — the lightest wallpaper
 *  Muivly can show. One `mainImage(float2 uv)` function per file. `.glsl` and
 *  `.frag` are Shadertoy shaders, translated by the engine on the way in. */
const SHADER_EXTENSIONS = ['hlsl', 'fx', 'glsl', 'frag']

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
      {
        name: 'Duvar kağıdı',
        extensions: [...VIDEO_EXTENSIONS, ...IMAGE_EXTENSIONS, ...SHADER_EXTENSIONS],
      },
      { name: 'Video', extensions: VIDEO_EXTENSIONS },
      { name: 'Görsel', extensions: IMAGE_EXTENSIONS },
      { name: 'Shader', extensions: SHADER_EXTENSIONS },
    ],
  })
  if (!picked) return []
  return Array.isArray(picked) ? picked : [picked]
}
