/**
 * Everything that crosses into Rust.
 *
 * Two destinations behind one surface: the engine (over the named pipe) and
 * the state file on disk. Keeping them in one module means a component never
 * has to know which is which.
 */
import { invoke } from '@tauri-apps/api/core'

export type MonitorState = {
  name: string
  enabled: boolean
  /** Which item of the playlist is currently on screen. */
  index: number
  items: string[]
}

export type EngineStatus = {
  fps: number
  paused: boolean
  fit: Fit
  interval_secs: number
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
  quit: () => invoke<void>('quit_engine'),
  start: () => invoke<void>('start_engine', { video: null }),
  installed: () => invoke<boolean>('engine_installed'),
}

export const disk = {
  load: () => invoke<string | null>('load_state'),
  save: (json: string) => invoke<void>('save_state', { json }),
  path: () => invoke<string>('state_path'),
  exists: (path: string) => invoke<boolean>('file_exists', { path }),
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
