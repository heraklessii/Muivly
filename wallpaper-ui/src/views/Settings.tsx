import { createContext, useContext, useEffect, useMemo, useState } from 'react'

import {
  contextMenu,
  disk,
  engine,
  longDuration,
  pack,
  pickPackage,
  startup,
  type EngineStatus,
  type Fit,
  type Rule,
} from '../api'
import Automation from '../components/Automation'
import { currentSettings, levelOf, LEVELS, type Level } from '../levels'
import type { Store } from '../store'

type Props = {
  store: Store
  status: EngineStatus | null
  onChange: (next: Store) => void
  onRefresh: () => void
}

/**
 * The groups the settings are filed under.
 *
 * There are twenty-three cards on this page and they used to be one column,
 * which meant reaching the sound settings was ten cards of scrolling past
 * things the user was not looking for. Worse, the five cards that answer the
 * same question — when should the wallpaper get out of the way — were spread
 * across the whole page rather than sitting together.
 *
 * The groups are by *when you would go looking*, not by which part of the
 * engine the setting reaches. "Tasarruf" is everything that costs less; that
 * it lands in four different modules is our problem, not the user's.
 */
const GROUPS = [
  { id: 'perf', label: 'Performans' },
  { id: 'playback', label: 'Oynatma' },
  { id: 'look', label: 'Görünüm' },
  { id: 'system', label: 'Sistem' },
  { id: 'status', label: 'Durum' },
] as const

type GroupId = (typeof GROUPS)[number]['id']

/**
 * Whether one card belongs on screen right now.
 *
 * Through a context rather than a prop, so `Card` can live at module scope:
 * a component defined inside `Settings` would be a new type on every render,
 * and React would throw away and rebuild every card under it — which is
 * exactly the input the user is typing an application name into.
 */
const Showing =
  createContext<(group: GroupId, title: string, words: string, advanced?: boolean) => boolean>(
    () => true,
  )

/**
 * One settings card, filed under a group and findable by search.
 *
 * `words` is what the user might type looking for this card when it is not
 * what the title is called: "fps" for the frame rate, "batarya" for the
 * battery. Nobody searches for the word the designer chose.
 *
 * `advanced` marks a card the level picker already decides. Those stay out of
 * the way until somebody asks for them — but a search still finds them,
 * because a person who typed "bellek" has asked.
 */
function Card({
  group,
  title,
  words = '',
  advanced = false,
  children,
}: {
  group: GroupId
  title: string
  words?: string
  advanced?: boolean
  children: React.ReactNode
}) {
  const showing = useContext(Showing)
  if (!showing(group, title, words, advanced)) return null

  return (
    <section className="card">
      <h2 className="card-title">{title}</h2>
      {children}
    </section>
  )
}

/** Case- and accent-insensitive enough for a Turkish settings search. */
function fold(text: string): string {
  return text
    .toLocaleLowerCase('tr')
    .replace(/[ıi̇]/g, 'i')
    .replace(/ş/g, 's')
    .replace(/ğ/g, 'g')
    .replace(/ü/g, 'u')
    .replace(/ö/g, 'o')
    .replace(/ç/g, 'c')
}

const FITS: { value: Fit; label: string; help: string }[] = [
  { value: 'cover', label: 'Doldur', help: 'Ekranı kaplar, taşan kenarlar kırpılır' },
  { value: 'contain', label: 'Sığdır', help: 'Videonun tamamı görünür, kenarlarda siyah bant kalır' },
  { value: 'stretch', label: 'Ger', help: 'Ekranı kaplar ama görüntü hafif ezilir' },
]

/**
 * The handlers that mean "the user has finished moving this slider".
 *
 * There is more than one way to let go of a range input, and `mouseup` is
 * only the most obvious: an arrow key never produces one, a drag released
 * off the end of the track fires it somewhere else entirely, and a touch
 * produces none at all. Listening for that alone meant a slider could be
 * moved with the keyboard all day without a single value reaching the
 * engine, and a drag that overshot the track was silently thrown away.
 *
 * Commits are idempotent — each checks whether anything actually changed —
 * so several of these firing for one gesture costs nothing.
 */
function releases(commit: () => void) {
  return {
    onPointerUp: commit,
    // Fires when a drag ends anywhere, including outside the element.
    onLostPointerCapture: commit,
    onKeyUp: commit,
    onBlur: commit,
  }
}

/** What to drop to while unplugged. 0 means "the same as on the charger". */
const BATTERY_RATES: { value: number; label: string }[] = [
  { value: 0, label: 'Aynı kalsın' },
  { value: 30, label: '30 fps' },
  { value: 24, label: '24 fps' },
  { value: 15, label: '15 fps' },
]

const SPEEDS: { value: number; label: string }[] = [
  { value: 0.5, label: '0.5×' },
  { value: 0.75, label: '0.75×' },
  { value: 1, label: 'Normal' },
  { value: 1.5, label: '1.5×' },
  { value: 2, label: '2×' },
]

const FADES: { value: number; label: string }[] = [
  { value: 0, label: 'Anında' },
  { value: 250, label: 'Hızlı' },
  { value: 400, label: 'Normal' },
  { value: 800, label: 'Yavaş' },
]

/** How long out of sight before the decoders are handed back. */
const HIBERNATES: { value: number; label: string; help: string }[] = [
  { value: 0, label: 'Kapalı', help: 'Bellekte durmaya devam eder' },
  { value: 10, label: '10 sn', help: 'Oyuna geçer geçmez bırakır' },
  { value: 20, label: '20 sn', help: 'Önerilen' },
  { value: 120, label: '2 dk', help: 'Yalnız uzun süre kapalıysa' },
]

/** The memory budget, as the sizes it actually resolves to. */
const BUDGETS: { value: number; label: string; help: string }[] = [
  { value: 0, label: 'Sınırsız', help: 'Video neyse o boyutta açılır' },
  { value: 600, label: '600 MB', help: '4K videolar için' },
  { value: 350, label: '350 MB', help: '2K ekranlar için yeter' },
  { value: 200, label: '200 MB', help: 'Full HD ekranlar için yeter' },
  { value: 120, label: '120 MB', help: 'Küçük ekran, dar bellek' },
]

/** How long the machine may sit untouched before the wallpaper stands still. */
const IDLES: { value: number; label: string; help: string }[] = [
  { value: 0, label: 'Kapalı', help: 'Kimse yokken de oynamaya devam' },
  { value: 120, label: '2 dk', help: 'Masadan kalkar kalkmaz durur' },
  { value: 300, label: '5 dk', help: 'Önerilen' },
  { value: 900, label: '15 dk', help: 'Yalnız uzun aralarda durur' },
]

/** What to fall to while the machine is busy with something else. */
const BUSY_RATES: { value: number; label: string }[] = [
  { value: 0, label: 'Aynı kalsın' },
  { value: 15, label: '15 fps' },
  { value: 10, label: '10 fps' },
  { value: 5, label: '5 fps' },
]

/** How far a photograph drifts on its own. */
const DRIFTS: { value: number; label: string; help: string }[] = [
  { value: 0, label: 'Kapalı', help: 'Fotoğraf hiç kıpırdamaz' },
  { value: 0.35, label: 'Hafif', help: 'Bakmazsan fark etmezsin' },
  { value: 0.7, label: 'Belirgin', help: 'Yavaşça yakınlaşıp uzaklaşır' },
  { value: 1, label: 'Tam', help: 'Gözle görülür şekilde gezinir' },
]

const INTERVALS: { value: number; label: string }[] = [
  { value: 0, label: 'Video bitince' },
  { value: 300, label: '5 dakika' },
  { value: 900, label: '15 dakika' },
  { value: 1800, label: '30 dakika' },
  { value: 3600, label: '1 saat' },
  { value: 21600, label: '6 saat' },
]

export default function Settings({ store, status, onChange, onRefresh }: Props) {
  const [statePath, setStatePath] = useState('')
  // Edited locally while dragging so the poll does not fight the slider.
  const [fpsDraft, setFpsDraft] = useState<number | null>(null)
  // Same reason: a slider bound straight to polled state jumps back under
  // the cursor every time the poll lands mid-drag.
  const [visual, setVisual] = useState({
    brightness: store.settings.brightness,
    saturation: store.settings.saturation,
    blur: store.settings.blur,
  })
  const [sound, setSound] = useState(store.settings.sound)
  const [volume, setVolume] = useState(store.settings.volume)
  const [autostart, setAutostart] = useState(false)
  const [startupError, setStartupError] = useState<string | null>(null)
  const [menu, setMenu] = useState(false)
  // Same drag-versus-poll reason as the sliders above.
  const [motion, setMotion] = useState({
    reactive: store.settings.reactive,
    parallax: store.settings.parallax,
  })
  // The application list is edited as one line of text and only sent when
  // the box loses focus: sending on every keystroke would freeze the
  // wallpaper for "p", "ph", "pho"...
  const [appsDraft, setAppsDraft] = useState<string | null>(null)
  // What the last import or export did, shown until the next one.
  const [packNote, setPackNote] = useState<string | null>(null)
  // Which group is open, and what the user is looking for. A search runs
  // across every group rather than inside the open one: somebody who does
  // not know which group holds a setting is precisely who is typing.
  const [group, setGroup] = useState<GroupId>('perf')
  const [search, setSearch] = useState('')
  // Whether the dials behind the level picker are on screen. Off until asked
  // for: the whole point of the levels is that most people never open this.
  const [detailed, setDetailed] = useState(false)

  useEffect(() => {
    void disk.path().then(setStatePath)
    // The registry is the truth here, not our own state file: the user may
    // have removed the entry with any of the several tools Windows offers.
    void startup
      .enabled()
      .then(setAutostart)
      .catch(() => setAutostart(false))
    void contextMenu
      .enabled()
      .then(setMenu)
      .catch(() => setMenu(false))
  }, [])

  const fps = fpsDraft ?? status?.fps ?? store.settings.fps

  // The draft is held until the engine is seen to agree, rather than dropped
  // the moment it is sent: status is polled, so clearing it straight away
  // showed the old value again for up to a poll and read as the slider
  // snapping back under the cursor.
  useEffect(() => {
    if (fpsDraft !== null && status?.fps === fpsDraft) setFpsDraft(null)
  }, [status?.fps, fpsDraft])

  async function apply<T>(work: () => Promise<T>, next: Partial<Store['settings']>) {
    await work()
    onChange({ ...store, settings: { ...store.settings, ...next } })
    onRefresh()
  }

  function pushFps() {
    if (fpsDraft === null || fpsDraft === status?.fps) return
    void apply(() => engine.setFps(fpsDraft), { fps: fpsDraft })
  }

  function pushVisual() {
    if (
      status &&
      status.brightness === visual.brightness &&
      status.saturation === visual.saturation &&
      status.blur === visual.blur
    ) {
      return
    }
    void apply(
      () => engine.setVisual(visual.brightness, visual.saturation, visual.blur),
      visual,
    )
  }

  function pushMotion() {
    if (status && status.reactive === motion.reactive && status.parallax === motion.parallax) {
      return
    }
    void apply(() => engine.setMotion(motion.reactive, motion.parallax), motion)
  }

  const apps = status?.apps ?? store.settings.apps

  function pushApps() {
    if (appsDraft === null) return
    const names = appsDraft
      .split(',')
      .map((name) => name.trim())
      .filter(Boolean)
    setAppsDraft(null)
    if (names.join('|') === apps.join('|')) return
    void apply(() => engine.setApps(names), { apps: names })
  }

  const duck = status?.duck ?? store.settings.duck

  function pushSound() {
    if (status && status.sound === sound && status.volume === volume) return
    void apply(() => engine.setSound(sound, volume, duck), { sound, volume })
  }

  const wanted = fold(search.trim())
  // Rebuilt only when the search or the group moves, so it is a stable value
  // for the provider rather than a new function on every keystroke elsewhere
  // on the page.
  const showing = useMemo(
    () => (owner: GroupId, title: string, words: string, advanced = false) => {
      // A search reaches everything, open group or not, folded away or not:
      // somebody who typed the name of a setting has asked for it.
      if (wanted) return fold(`${title} ${words}`).includes(wanted)
      if (owner !== group) return false
      return !advanced || detailed
    },
    [wanted, group, detailed],
  )

  // What the seven dials are set to, and which level that is.
  const tuning = currentSettings(status, store.settings)
  const level = levelOf(tuning)

  /**
   * Apply one level: seven values to the engine, the same seven to the state
   * file.
   *
   * The engine going away halfway through would leave the machine on a set
   * of values that is nobody's level, which the picker would then honestly
   * report as "Özel". That is the right thing to show, so the failure needs
   * nothing beyond not being thrown into the console.
   */
  async function pickLevel(next: Level) {
    const chosen = next.settings
    try {
      await engine.setFps(chosen.fps)
      await engine.setMemory(chosen.memoryMb)
      await engine.setHibernate(chosen.hibernateSecs)
      await engine.setIdle(chosen.idleSecs)
      await engine.setBusyFps(chosen.busyFps)
      await engine.setPower(chosen.batteryFps, chosen.pauseOnSaver)
      onChange({ ...store, settings: { ...store.settings, ...chosen } })
    } finally {
      onRefresh()
    }
  }

  return (
    <Showing.Provider value={showing}>
      <header className="view-head">
        <div>
          <h1 className="view-title">Ayarlar</h1>
          <p className="view-sub">Değişiklikler anında uygulanır.</p>
        </div>
      </header>

      <div className="settings-bar">
        {/* A group of filters rather than tabs: `role="tab"` is a promise
            about `aria-controls` and a tabpanel, and a half-kept promise
            reads worse to a screen reader than none. */}
        <div className="chips" role="group" aria-label="Ayar grupları">
          {GROUPS.map((entry) => (
            <button
              key={entry.id}
              className="chip"
              aria-pressed={!wanted && group === entry.id}
              data-active={!wanted && group === entry.id}
              onClick={() => {
                setGroup(entry.id)
                setSearch('')
              }}
            >
              {entry.label}
            </button>
          ))}
        </div>
        <div className="spacer" />
        <input
          type="search"
          value={search}
          placeholder="Ayar ara"
          aria-label="Ayarlarda ara"
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>

      {/* `display: contents`, so the cards lay out exactly as they did when
          they were siblings — the wrapper exists only so that `:empty` can
          answer "did anything match" without a second list of every card to
          keep in step with this one. */}
      <div className="settings-cards">

        <Card group="perf" title="Seviye" words="performans kalite hafif dengeli tam preset ayar">
          <p className="card-sub">
            Duvar kağıdının bilgisayarından ne kadar isteyeceği. Aşağıdakilerin
            hepsi tek tek de ayarlanabilir ama gerekmez — buradan birini seç,
            gerisi kendiliğinden ayarlanır.
          </p>
          <div className="options">
            {LEVELS.map((option) => (
              <button
                key={option.id}
                className="option"
                data-active={level?.id === option.id}
                onClick={() => {
                  void pickLevel(option).catch(() => {
                    // The banner already says the engine is not running.
                  })
                }}
              >
                <span className="option-label">{option.label}</span>
                <span className="option-help">{option.help}</span>
              </button>
            ))}
          </div>

          <div className="row">
            {!level && <span className="badge">Özel</span>}
            {!level && (
              <span className="muted">Ayarları kendin değiştirdin.</span>
            )}
            <div className="spacer" />
            <button aria-expanded={detailed} onClick={() => setDetailed(!detailed)}>
              {detailed ? 'Ayrıntıları gizle' : 'Tek tek ayarla'}
            </button>
          </div>
        </Card>

        <Card group="perf" title="Kare hızı" words="fps kare hiz akicilik" advanced>
          <p className="card-sub">
            Saniyede kaç kare çizilsin. Yükseldikçe daha akıcı görünür, daha
            çok işlemci ister. Bu bir üst sınır: ekran görünmüyorken ve pilde
            zaten kendiliğinden düşüyor.
          </p>
          <div className="row">
            <input
              type="range"
              min={10}
              max={120}
              step={5}
              value={fps}
              onChange={(e) => setFpsDraft(Number(e.target.value))}
              {...releases(pushFps)}
            />
            <span className="fps-value">{fps} fps</span>
          </div>
        </Card>

        <Card group="perf" title="Pil" words="batarya sarj tasarruf fis guc" advanced>
          <p className="card-sub">
            Fişten çekilince duvar kağıdı yavaşlayan ilk şey olsun —{' '}
            {status
              ? status.on_battery
                ? `şu an pilde (%${status.battery_percent})`
                : 'şu an prizde'
              : 'motor çalışmıyor'}
            {status?.saver && ', pil tasarrufu açık'}.
          </p>
          <div className="options">
            {BATTERY_RATES.map((rate) => (
              <button
                key={rate.value}
                className="option compact"
                data-active={(status?.battery_fps ?? store.settings.batteryFps) === rate.value}
                onClick={() =>
                  void apply(
                    () =>
                      engine.setPower(
                        rate.value,
                        status?.pause_on_saver ?? store.settings.pauseOnSaver,
                      ),
                    { batteryFps: rate.value },
                  )
                }
              >
                {rate.label}
              </button>
            ))}
          </div>
          <div className="row">
            <label className="toggle">
              <input
                type="checkbox"
                checked={status?.pause_on_saver ?? store.settings.pauseOnSaver}
                onChange={(e) =>
                  void apply(
                    () =>
                      engine.setPower(
                        status?.battery_fps ?? store.settings.batteryFps,
                        e.target.checked,
                      ),
                    { pauseOnSaver: e.target.checked },
                  )
                }
              />
              <span>Pil tasarrufu açıkken tamamen dursun</span>
            </label>
          </div>
          <p className="card-sub">
            Dondurmak duvar kağıdını kaldırmaz: son kare ekranda öylece kalır,
            sadece hareket durur.
          </p>
        </Card>

        <Card group="perf" title="Masadan kalkınca" words="bosta idle uzakta klavye fare dur" advanced>
          <p className="card-sub">
            Bilgisayarın başında kimse yokken duvar kağıdının oynamasının bir
            anlamı yok. Klavyeye ve fareye bu kadar süre dokunulmazsa durur,
            ilk tuşa basınca geri gelir.
            {status?.away && ' Şu an duruyor.'}
          </p>
          <div className="options">
            {IDLES.map((option) => (
              <button
                key={option.value}
                className="option"
                data-active={(status?.idle_secs ?? store.settings.idleSecs) === option.value}
                onClick={() =>
                  void apply(() => engine.setIdle(option.value), { idleSecs: option.value })
                }
              >
                <span className="option-label">{option.label}</span>
                <span className="option-help">{option.help}</span>
              </button>
            ))}
          </div>
          <p className="card-sub">
            Windows'un kendi sayacına bakar. Muivly hiçbir tuşu görmez, hiçbir
            şey kaydetmez.
          </p>
        </Card>

        <Card group="perf" title="Makine meşgulken" words="cpu yuk mesgul derleme oyun busy" advanced>
          <p className="card-sub">
            Oyun yüklenirken, bir güncelleme inerken ya da ağır bir iş
            dönerken duvar kağıdı yoldan çekilsin.
            {status &&
              (status.busy
                ? ` Şu an geri çekilmiş durumda; bilgisayarın %${Math.round(
                    status.load,
                  )} kadarı kullanılıyor.`
                : ` Şu an bilgisayarın %${Math.round(status.load)} kadarı kullanılıyor.`)}
          </p>
          <div className="options">
            {BUSY_RATES.map((rate) => (
              <button
                key={rate.value}
                className="option compact"
                data-active={(status?.busy_fps ?? store.settings.busyFps) === rate.value}
                onClick={() =>
                  void apply(() => engine.setBusyFps(rate.value), { busyFps: rate.value })
                }
              >
                {rate.label}
              </button>
            ))}
          </div>
          <div className="row">
            <label className="toggle">
              <input
                type="checkbox"
                checked={status?.reduce_motion ?? store.settings.reduceMotion}
                onChange={(e) =>
                  void apply(() => engine.setReduceMotion(e.target.checked), {
                    reduceMotion: e.target.checked,
                  })
                }
              />
              <span>Windows'ta animasyonlar kapalıysa duvar kağıdı da dursun</span>
            </label>
          </div>
        </Card>

        <Card group="playback" title="Ölçekleme" words="fit doldur sigdir ger en boy orani kirp">
          <p className="card-sub">
            Videonun şekli ekranınla tutmadığında ne olsun.
          </p>
          <div className="options">
            {FITS.map((fit) => (
              <button
                key={fit.value}
                className="option"
                data-active={(status?.fit ?? store.settings.fit) === fit.value}
                onClick={() => void apply(() => engine.setFit(fit.value), { fit: fit.value })}
              >
                <span className="option-label">{fit.label}</span>
                <span className="option-help">{fit.help}</span>
              </button>
            ))}
          </div>
        </Card>

        <Card group="playback" title="Oynatma" words="hiz speed yavas hizli">
          <p className="card-sub">
            Videoyu yavaşlatmak ya da hızlandırmak. Yavaşlatmak bilgisayarı
            daha da az yorar.
          </p>
          <div className="options">
            {SPEEDS.map((speed) => (
              <button
                key={speed.value}
                className="option compact"
                data-active={(status?.speed ?? store.settings.speed) === speed.value}
                onClick={() =>
                  void apply(() => engine.setSpeed(speed.value), { speed: speed.value })
                }
              >
                {speed.label}
              </button>
            ))}
          </div>
        </Card>

        <Card group="playback" title="Geçiş" words="crossfade gecis kesme fade">
          <p className="card-sub">
            Bir duvar kağıdından diğerine geçerken ne olsun.
          </p>
          <div className="options">
            {FADES.map((fade) => (
              <button
                key={fade.value}
                className="option compact"
                data-active={(status?.fade_ms ?? store.settings.fadeMs) === fade.value}
                onClick={() =>
                  void apply(() => engine.setFade(fade.value), { fadeMs: fade.value })
                }
              >
                {fade.label}
              </button>
            ))}
          </div>
        </Card>

        <Card group="playback" title="Liste geçişi" words="playlist liste aralik karisik shuffle siradaki">
          <p className="card-sub">
            Bir ekrana liste atandığında sıradaki klibe ne zaman geçilecek.
          </p>
          <div className="options">
            {INTERVALS.map((interval) => (
              <button
                key={interval.value}
                className="option compact"
                data-active={
                  (status?.interval_secs ?? store.settings.intervalSecs) === interval.value
                }
                onClick={() =>
                  void apply(() => engine.setInterval(interval.value), {
                    intervalSecs: interval.value,
                  })
                }
              >
                {interval.label}
              </button>
            ))}
          </div>

          <div className="row">
            <label className="toggle">
              <input
                type="checkbox"
                checked={status?.shuffle ?? store.settings.shuffle}
                onChange={(e) =>
                  void apply(() => engine.setShuffle(e.target.checked), {
                    shuffle: e.target.checked,
                  })
                }
              />
              <span>Listeyi karışık oynat</span>
            </label>
          </div>
          <p className="card-sub">
            Liste baştan sona bir kez dolaşılır, sonra sıra yeniden çekilir —
            aynı duvar kağıdı, diğerleri sırasını almadan tekrar gelmez. Bu
            ayarı değiştirmek ekranda olanı değiştirmez, sadece sonrasını.
          </p>
        </Card>

        <Card group="look" title="Görünüm" words="parlaklik doygunluk bulaniklik renk grade">
          <p className="card-sub">
            Dosyaya dokunulmaz; bunlar yalnız ekrana çizilirken uygulanır ve
            her ekranda aynıdır.
          </p>

          <div className="row">
            <label className="slider-label">Parlaklık</label>
            <input
              type="range"
              min={0.2}
              max={1.6}
              step={0.05}
              value={visual.brightness}
              onChange={(e) => setVisual({ ...visual, brightness: Number(e.target.value) })}
              {...releases(pushVisual)}
            />
            <span className="fps-value">{Math.round(visual.brightness * 100)}%</span>
          </div>

          <div className="row">
            <label className="slider-label">Doygunluk</label>
            <input
              type="range"
              min={0}
              max={2}
              step={0.05}
              value={visual.saturation}
              onChange={(e) => setVisual({ ...visual, saturation: Number(e.target.value) })}
              {...releases(pushVisual)}
            />
            <span className="fps-value">{Math.round(visual.saturation * 100)}%</span>
          </div>

          <div className="row">
            <label className="slider-label">Bulanıklık</label>
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={visual.blur}
              onChange={(e) => setVisual({ ...visual, blur: Number(e.target.value) })}
              {...releases(pushVisual)}
            />
            <span className="fps-value">{Math.round(visual.blur * 100)}%</span>
          </div>

          <div className="row">
            <button
              onClick={() => {
                const plain = { brightness: 1, saturation: 1, blur: 0 }
                setVisual(plain)
                void apply(() => engine.setVisual(1, 1, 0), plain)
              }}
            >
              Sıfırla
            </button>
          </div>
        </Card>

        <Card group="look" title="Ses" words="sound volume ses seviye sessiz duck">
          <p className="card-sub">
            Videonun kendi sesi, ana ekrandaki duvar kağıdından. Duvar kağıdı
            görünmez olduğunda (tam ekran oyun, kilit ekranı) kendiliğinden
            susar.
          </p>
          <div className="row">
            <button
              className={sound ? 'primary' : undefined}
              onClick={() => {
                const next = !sound
                setSound(next)
                void apply(() => engine.setSound(next, volume, duck), { sound: next })
              }}
            >
              {sound ? 'Açık' : 'Kapalı'}
            </button>
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={volume}
              disabled={!sound}
              onChange={(e) => setVolume(Number(e.target.value))}
              {...releases(pushSound)}
            />
            <span className="fps-value">{Math.round(volume * 100)}%</span>
          </div>

          <div className="row">
            <label className="toggle">
              <input
                type="checkbox"
                checked={duck}
                onChange={(e) =>
                  void apply(() => engine.setSound(sound, volume, e.target.checked), {
                    duck: e.target.checked,
                  })
                }
              />
              <span>Başka bir şey ses çalarken sessize geç</span>
            </label>
            {status?.ducking && <span className="muted">şu an geri çekildi</span>}
          </div>
        </Card>

        <Card group="perf" title="Bellek" words="ram memory butce mb cozunurluk" advanced>
          <p className="card-sub">
            Duvar kağıdının kapladığı belleği belirleyen tek şey videonun
            çözünürlüğü. Sınır koymak, videoyu ekranına yetecek kadar küçük
            açmak demek — ekranda zaten görünmeyen pikseller.
            {status && ` Muivly şu an ${status.ram_mb} MB kullanıyor.`}
          </p>
          <div className="options">
            {BUDGETS.map((budget) => (
              <button
                key={budget.value}
                className="option"
                data-active={(status?.memory_mb ?? store.settings.memoryMb) === budget.value}
                onClick={() =>
                  void apply(() => engine.setMemory(budget.value), { memoryMb: budget.value })
                }
              >
                <span className="option-label">{budget.label}</span>
                <span className="option-help">{budget.help}</span>
              </button>
            ))}
          </div>
          <p className="card-sub">
            Değiştirince oynayan videolar baştan başlar, kısa bir duraklama
            görürsün. Kalıcı çözüm kitaplıktaki <strong>Hafiflet</strong>:
            videoyu bir kez küçük kaydeder, sonrası bedava.
          </p>
        </Card>

        <Card group="perf" title="Görünmezken" words="hibernate uyku cozucu bellek ortulu gizli" advanced>
          <p className="card-sub">
            Tam ekran bir oyun açıkken duvar kağıdı zaten çizilmiyor, ama video
            hâlâ bellekte duruyor. Bu süre dolunca o da bırakılır — belleği
            oyuna kalır. Masaüstü göründüğünde video baştan başlar.
            {status?.hibernating && ' Şu an bırakılmış durumda.'}
          </p>
          <div className="options">
            {HIBERNATES.map((option) => (
              <button
                key={option.value}
                className="option"
                data-active={(status?.hibernate_secs ?? store.settings.hibernateSecs) === option.value}
                onClick={() =>
                  void apply(() => engine.setHibernate(option.value), {
                    hibernateSecs: option.value,
                  })
                }
              >
                <span className="option-label">{option.label}</span>
                <span className="option-help">{option.help}</span>
              </button>
            ))}
          </div>
        </Card>

        <Card group="look" title="Hareket" words="motion sese tepki paralaks imlec">
          <p className="card-sub">
            Duvar kağıdı sese ve fareye tepki versin. İkisi de neredeyse
            bedava: fazladan hiçbir şey çizilmiyor, sadece resmin kadrajı
            kayıyor. Sıfırdayken hiçbir ölçüm yapılmaz.
          </p>

          <div className="row">
            <label className="slider-label">Sese tepki</label>
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={motion.reactive}
              onChange={(e) => setMotion({ ...motion, reactive: Number(e.target.value) })}
              {...releases(pushMotion)}
            />
            <span className="fps-value">{Math.round(motion.reactive * 100)}%</span>
          </div>
          <p className="card-sub">
            Bilgisayardan çıkan sesin tamamını dinler — müzik, video, oyun,
            hepsi. Muivly'nin kendi sesi kapalı olsa bile çalışır.
          </p>

          <div className="row">
            <label className="slider-label">İmleç paralaksı</label>
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={motion.parallax}
              onChange={(e) => setMotion({ ...motion, parallax: Number(e.target.value) })}
              {...releases(pushMotion)}
            />
            <span className="fps-value">{Math.round(motion.parallax * 100)}%</span>
          </div>
        </Card>

        <Card group="look" title="Duran görselde sürüklenme" words="drift ken burns fotograf yakinlasma">
          <p className="card-sub">
            Fotoğraf duvar kağıdı hiç kıpırdamaz. Bu ayar kadrajı çok yavaş
            kaydırıp yakınlaştırır, fotoğraf canlı durur. Video açmakla aynı
            şey değil — hiçbir şey çözülmüyor, sadece resmin neresine
            baktığımız yavaşça değişiyor.
          </p>
          <div className="options">
            {DRIFTS.map((option) => (
              <button
                key={option.value}
                className="option"
                data-active={(status?.drift ?? store.settings.drift) === option.value}
                onClick={() =>
                  void apply(() => engine.setDrift(option.value), { drift: option.value })
                }
              >
                <span className="option-label">{option.label}</span>
                <span className="option-help">{option.help}</span>
              </button>
            ))}
          </div>
          <p className="card-sub">
            Yalnız fotoğraflara uygulanır. Video, GIF ve shader zaten hareketli
            — üstüne bir hareket daha binse iyi durmazdı.
          </p>
        </Card>

        <Card group="look" title="Vurgu rengi" words="accent renk windows tema">
          <p className="card-sub">
            Windows'un vurgu rengi duvar kağıdından gelsin. Ekrandaki resmin
            ortalama rengi alınır; yazılar okunur kalsın diye çok koyu ya da
            çok açık olmaması sağlanır.
          </p>
          <div className="row">
            <label className="toggle">
              <input
                type="checkbox"
                checked={status?.accent ?? store.settings.accent}
                onChange={(e) =>
                  void apply(() => engine.setAccent(e.target.checked), {
                    accent: e.target.checked,
                  })
                }
              />
              <span>Vurgu rengi duvar kağıdını izlesin</span>
            </label>
          </div>
          <p className="card-sub">
            Eski renklerin bir kenara yazılıyor. Bu ayarı kapattığında, motoru
            kapattığında ya da motor çökerse kendi renklerin geri gelir. Görev
            çubuğu bazen bir sonraki açılışı bekleyebiliyor.
          </p>
        </Card>

        {status && status.uptime_secs > 0 && (
          <Card group="status" title="Ne kadar dinlendi" words="dinlenme resting bosa calisma suresi istatistik">
            <p className="card-sub">
              Muivly <strong>{longDuration(status.uptime_secs)}</strong> açıktı,
              bunun <strong>{longDuration(status.resting_secs)}</strong>{' '}
              kadarında hiçbir şey çizmedi: ekran kapalıydı, üstü bir
              pencereyle örtülüydü ya da başında kimse yoktu. Bu projenin
              bütün mesele bu sayı.
            </p>
            <div className="row">
              <div className="rest-bar" aria-hidden="true">
                <span
                  style={{
                    width: `${Math.min(
                      100,
                      Math.round((status.resting_secs / Math.max(1, status.uptime_secs)) * 100),
                    )}%`,
                  }}
                />
              </div>
              <span className="fps-value">
                %{Math.round((status.resting_secs / Math.max(1, status.uptime_secs)) * 100)}
              </span>
            </div>
            <p className="card-sub">Muivly'nin bu açılışından beri.</p>
          </Card>
        )}

        <Card group="perf" title="Uygulama kuralları" words="app uygulama dondur photoshop oyun on plan">
          <p className="card-sub">
            Buradaki uygulamalardan biri öndeyken duvar kağıdı durur. Tam ekran
            oyunlar için gerekmez, onlar zaten masaüstünü kapatıyor — bu, ekranı
            kaplamayan ama bilgisayarı yoran işler için: render alma, görüntülü
            görüşme, video düzenleme.
          </p>
          <div className="row">
            <input
              type="text"
              className="grow"
              placeholder="photoshop, blender, obs64"
              value={appsDraft ?? apps.join(", ")}
              onChange={(e) => setAppsDraft(e.target.value)}
              onBlur={pushApps}
              onKeyDown={(e) => {
                if (e.key === "Enter") e.currentTarget.blur()
              }}
            />
          </div>
          <p className="card-sub">
            Virgülle ayır. <code>.exe</code> yazmasan da olur.
          </p>
        </Card>

        {/* Its own card, drawn by the component rather than by `Card`, so it
            is filed by hand rather than by the wrapper. */}
        {showing('playback', 'Otomasyon', 'kural saat tema zamanlama otomatik') && (
          <Automation
            store={store}
            rules={status?.rules ?? []}
            onChange={(rules: Rule[]) => {
              void engine.setRules(rules).then(onRefresh)
            }}
          />
        )}

        <Card group="system" title="Kısayollar ve menü" words="hotkey kisayol sag tik explorer menu">
          <p className="card-sub">
            Kısayollar her yerde çalışır. Bir tuş birleşimini başka bir
            uygulama kapmışsa yalnız o çalışmaz, diğerleri çalışmaya devam
            eder.
          </p>
          <div className="row">
            <label className="toggle">
              <input
                type="checkbox"
                checked={status?.hotkeys ?? store.settings.hotkeys}
                onChange={(e) =>
                  void apply(() => engine.setHotkeys(e.target.checked), {
                    hotkeys: e.target.checked,
                  })
                }
              />
              <span>
                Ctrl+Alt+→ sonraki · Ctrl+Alt+P dondur · Ctrl+Alt+M ses
              </span>
            </label>
          </div>
          <div className="row">
            <label className="toggle">
              <input
                type="checkbox"
                checked={menu}
                onChange={async (e) => {
                  const next = e.target.checked
                  try {
                    await contextMenu.set(next)
                    setMenu(next)
                  } catch (err) {
                    setPackNote(String(err))
                  }
                }}
              />
              <span>Explorer'da sağ tık → "Muivly duvar kağıdı yap"</span>
            </label>
          </div>
          <p className="card-sub">
            Menüden seçtiğin dosya bütün ekranlara uygulanır. Bu pencere
            açılmaz.
          </p>
        </Card>

        <Card group="system" title="Paketler" words="muivly paket disa aktar ice aktar zip">
          <p className="card-sub">
            <code>.muivly</code> bir duvar kağıdını adı ve künyesiyle birlikte
            tek dosyada taşır — arkadaşına yollamak için. Aslında bir zip
            dosyası, Muivly'si olmayan biri de içini açabilir. Kitaplıkta her
            duvar kağıdının kendi "Paket yap" düğmesi var.
          </p>
          <div className="row">
            <button
              onClick={async () => {
                const file = await pickPackage()
                if (!file) return
                try {
                  const imported = await pack.import(file)
                  onChange({
                    ...store,
                    items: [
                      ...store.items,
                      {
                        id: crypto.randomUUID(),
                        path: imported.path,
                        title: imported.title,
                        added: Date.now(),
                      },
                    ],
                  })
                  setPackNote(
                    `"${imported.title}" kitaplığa eklendi${
                      imported.author ? ` · ${imported.author}` : ''
                    }`,
                  )
                } catch (err) {
                  setPackNote(String(err))
                }
              }}
            >
              Paket içe aktar
            </button>
            {packNote && <span className="muted">{packNote}</span>}
          </div>
        </Card>

        <Card group="system" title="Windows ile başlat" words="autostart baslangic acilis oturum">
          <p className="card-sub">
            Bilgisayar açılınca yalnız duvar kağıdı başlar, bu pencere değil.
            Son duvar kağıdını ve ayarlarını kendisi hatırlıyor.
          </p>
          <div className="row">
            <button
              className={autostart ? 'primary' : undefined}
              onClick={async () => {
                const next = !autostart
                try {
                  await startup.set(next)
                  setAutostart(next)
                } catch (e) {
                  setStartupError(String(e))
                }
              }}
            >
              {autostart ? 'Açık' : 'Kapalı'}
            </button>
            {startupError && <span className="error-text">{startupError}</span>}
          </div>
        </Card>

        <Card group="status" title="Performans" words="cpu ram fps olcum bellek">
          <p className="card-sub">
            Muivly'nin kendi ölçümü, saniyede bir yenilenir. Görev
            Yöneticisi'nde <code>muivly-core</code> satırında aynı sayıları
            görürsün.
          </p>
          <div className="stats">
            <div className="stat">
              <span className="stat-value">{status ? status.cpu.toFixed(1) : '—'}%</span>
              <span className="stat-label">İşlemci (bir çekirdeğin payı)</span>
            </div>
            <div className="stat">
              <span className="stat-value">{status ? status.ram_mb : '—'} MB</span>
              <span className="stat-label">Bellek</span>
            </div>
            <div className="stat">
              <span className="stat-value">{status ? status.real_fps.toFixed(0) : '—'}</span>
              <span className="stat-label">
              Saniyedeki kare (sınır {status?.fps ?? '—'})
            </span>
            </div>
          </div>
        </Card>

        <Card group="system" title="Motor" words="engine yeniden baslat cikis surum">
          <p className="card-sub">
            Duvar kağıdı bu pencereden bağımsız çalışır. Pencereyi kapatmak
            onu durdurmaz; X tuşu uygulamayı sistem tepsisine küçültür.
          </p>
          <div className="row">
            <button
              className={status?.frozen ? 'primary' : undefined}
              disabled={!status}
              title="Son kare ekranda kalır, hareket durur"
              onClick={async () => {
                try {
                  await engine.setFrozen(!status?.frozen)
                } catch {
                  /* the next poll reports the engine is gone */
                }
                onRefresh()
              }}
            >
              {status?.frozen ? 'Donduruldu' : 'Dondur'}
            </button>
            <button
              className="danger"
              disabled={!status}
              onClick={async () => {
                try {
                  await engine.quit()
                } catch {
                  // Already gone is the outcome asked for, not a failure.
                }
                // The engine answers `ok` and then tears its windows down, so
                // asking straight away would still find it listening and the
                // panel would claim it is running.
                setTimeout(onRefresh, 400)
              }}
            >
              Motoru durdur
            </button>
            <span className="muted">
              {status ? `Çalışıyor · ${status.monitors.length} ekran` : 'Çalışmıyor'}
            </span>
          </div>
        </Card>

        <Card group="system" title="Veri" words="state json kitaplik dosya yolu">
          <p className="card-sub">
            Kitaplığın, listelerin ve hangi ekranda ne olduğu burada tutuluyor.
            Videoların kopyalanmaz, yalnız nerede oldukları yazılır.
          </p>
          <div className="path muted">{statePath}</div>
        </Card>
      </div>

      {wanted && (
        <div className="card empty settings-none">
          <p>
            <strong>{search.trim()}</strong> diye bir ayar yok — başka bir
            kelime dene, ya da yukarıdaki gruplara bak.
          </p>
        </div>
      )}
    </Showing.Provider>
  )
}
