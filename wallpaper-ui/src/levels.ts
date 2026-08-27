/**
 * The four levels a user actually picks between.
 *
 * Seven separate dials decide what Muivly costs: the frame rate, the memory
 * budget, how long out of sight before the video is let go, how long the desk
 * must be empty, what to fall to while the machine is busy, and two for the
 * battery. Every one of them is a real choice with a real reason, and every
 * one of them asks the user a question they should not have to answer —
 * nobody opens a wallpaper app wanting to think about frame buffers.
 *
 * So they are offered as one choice with four answers, from "barely touch
 * this machine" to "make it look as good as it can". The dials are still
 * there, behind "Tek tek ayarla", for whoever wants them — and a set of
 * values that matches no level is simply shown as "Özel" rather than being
 * corrected.
 *
 * `balanced` is deliberately the engine's own defaults, so a fresh install
 * reads as "Dengeli" rather than as "Özel".
 */
import type { EngineStatus } from './api'
import type { Settings } from './store'

/** The settings one level decides. Everything else is left alone. */
export type LevelSettings = Pick<
  Settings,
  'fps' | 'memoryMb' | 'hibernateSecs' | 'idleSecs' | 'busyFps' | 'batteryFps' | 'pauseOnSaver'
>

export type Level = {
  id: string
  label: string
  /** One line, in the words of somebody choosing rather than tuning. */
  help: string
  settings: LevelSettings
}

export const LEVELS: Level[] = [
  {
    id: 'minimum',
    label: 'En hafif',
    help: 'Eski makineler için. Kıpırdadığından çok durur.',
    settings: {
      fps: 20,
      memoryMb: 120,
      hibernateSecs: 5,
      idleSecs: 120,
      busyFps: 5,
      batteryFps: 15,
      pauseOnSaver: true,
    },
  },
  {
    id: 'light',
    label: 'Hafif',
    help: 'Makineyi hiç zorlamaz, yine de akıcı görünür.',
    settings: {
      fps: 24,
      memoryMb: 200,
      hibernateSecs: 10,
      idleSecs: 300,
      busyFps: 5,
      batteryFps: 15,
      pauseOnSaver: true,
    },
  },
  {
    id: 'balanced',
    label: 'Dengeli',
    help: 'Önerilen. Çoğu bilgisayarda doğru cevap bu.',
    settings: {
      fps: 30,
      memoryMb: 0,
      hibernateSecs: 20,
      idleSecs: 300,
      busyFps: 10,
      batteryFps: 24,
      pauseOnSaver: true,
    },
  },
  {
    id: 'full',
    label: 'Tam',
    help: 'Güçlü makineler için. En akıcı görüntü, en çok kaynak.',
    settings: {
      fps: 60,
      memoryMb: 0,
      hibernateSecs: 120,
      idleSecs: 900,
      busyFps: 0,
      batteryFps: 30,
      pauseOnSaver: false,
    },
  },
]

/**
 * What the engine is set to right now, as the seven values a level decides.
 *
 * The engine is the truth when it is running; the state file is what is left
 * to go on when it is not.
 */
export function currentSettings(
  status: EngineStatus | null,
  saved: Settings,
): LevelSettings {
  return {
    fps: status?.fps ?? saved.fps,
    memoryMb: status?.memory_mb ?? saved.memoryMb,
    hibernateSecs: status?.hibernate_secs ?? saved.hibernateSecs,
    idleSecs: status?.idle_secs ?? saved.idleSecs,
    busyFps: status?.busy_fps ?? saved.busyFps,
    batteryFps: status?.battery_fps ?? saved.batteryFps,
    pauseOnSaver: status?.pause_on_saver ?? saved.pauseOnSaver,
  }
}

/**
 * Which level these settings are, or `null` for a set that is nobody's.
 *
 * Worked out from the values rather than remembered, so a level stays
 * accurate after the user moves one dial by hand, after a settings file from
 * an older build, and after the engine restores a session of its own.
 */
export function levelOf(settings: LevelSettings): Level | null {
  return (
    LEVELS.find((level) =>
      (Object.keys(level.settings) as (keyof LevelSettings)[]).every(
        (key) => level.settings[key] === settings[key],
      ),
    ) ?? null
  )
}
