import { useEffect, useState } from 'react'

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
import type { Store } from '../store'

type Props = {
  store: Store
  status: EngineStatus | null
  onChange: (next: Store) => void
  onRefresh: () => void
}

const FITS: { value: Fit; label: string; help: string }[] = [
  { value: 'cover', label: 'Doldur', help: 'Ekranı kaplar, taşan kenarları kırpar' },
  { value: 'contain', label: 'Sığdır', help: 'Tamamını gösterir, kenarlarda siyah bant' },
  { value: 'stretch', label: 'Ger', help: 'Ekranı kaplar, oranı bozar' },
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
  { value: 0, label: 'Düşürme' },
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
  { value: 0, label: 'Sert kesme' },
  { value: 250, label: 'Hızlı' },
  { value: 400, label: 'Normal' },
  { value: 800, label: 'Yavaş' },
]

/** How long out of sight before the decoders are handed back. */
const HIBERNATES: { value: number; label: string; help: string }[] = [
  { value: 0, label: 'Kapalı', help: 'Çözücüler hep açık kalır' },
  { value: 10, label: '10 sn', help: 'Alt+Tab yapar yapmaz' },
  { value: 20, label: '20 sn', help: 'Önerilen' },
  { value: 120, label: '2 dk', help: 'Sadece uzun sürelerde' },
]

/** The memory budget, as the sizes it actually resolves to. */
const BUDGETS: { value: number; label: string; help: string }[] = [
  { value: 0, label: 'Sınırsız', help: 'Donanıma göre seçilen tavan' },
  { value: 600, label: '600 MB', help: 'Klibin kendi çözünürlüğü' },
  { value: 350, label: '350 MB', help: 'En çok 1440p' },
  { value: 200, label: '200 MB', help: 'En çok 1080p' },
  { value: 120, label: '120 MB', help: 'En çok 720p' },
]

/** How long the machine may sit untouched before the wallpaper stands still. */
const IDLES: { value: number; label: string; help: string }[] = [
  { value: 0, label: 'Kapalı', help: 'Kimse yokken de oynar' },
  { value: 120, label: '2 dk', help: 'Masadan kalkar kalkmaz' },
  { value: 300, label: '5 dk', help: 'Önerilen' },
  { value: 900, label: '15 dk', help: 'Sadece uzun aralarda' },
]

/** What to fall to while the machine is busy with something else. */
const BUSY_RATES: { value: number; label: string }[] = [
  { value: 0, label: 'Düşürme' },
  { value: 15, label: '15 fps' },
  { value: 10, label: '10 fps' },
  { value: 5, label: '5 fps' },
]

/** How far a photograph drifts on its own. */
const DRIFTS: { value: number; label: string; help: string }[] = [
  { value: 0, label: 'Kapalı', help: 'Görsel hiç kıpırdamaz' },
  { value: 0.35, label: 'Hafif', help: 'Fark edilir etmez' },
  { value: 0.7, label: 'Belirgin', help: 'Yavaş bir yakınlaşma' },
  { value: 1, label: 'Tam', help: 'Kadraj gözle görülür gezinir' },
]

const INTERVALS: { value: number; label: string }[] = [
  { value: 0, label: 'Klip bittiğinde' },
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


  return (
    <>
      <header className="view-head">
        <div>
          <h1 className="view-title">Ayarlar</h1>
          <p className="view-sub">Motor çalışırken anında uygulanır.</p>
        </div>
      </header>

      <section className="card">
        <h2 className="card-title">Kare hızı</h2>
        <p className="card-sub">
          Donanımına göre otomatik seçildi. Ekran görünmüyorken ve pilde zaten
          düşürülüyor — bu tavan.
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
      </section>

      <section className="card">
        <h2 className="card-title">Pil</h2>
        <p className="card-sub">
          Priz varken yukarıdaki tavan geçerli. Fişten çekilince duvar kağıdı
          ilk geri çekilen şey olmalı —{' '}
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
            <span>Pil tasarrufu açıkken tamamen dondur</span>
          </label>
        </div>
        <p className="card-sub">
          Dondurmak duvar kağıdını kaldırmaz — son kare ekranda kalır, çözme
          ve çizme durur.
        </p>
      </section>

      <section className="card">
        <h2 className="card-title">Masadan kalkınca</h2>
        <p className="card-sub">
          Masaüstü kapalıyken çizim zaten duruyor. Kapalı olmadığı halde
          kimsenin bakmadığı durumu hiçbir şey yakalamıyordu: bilgisayarın
          başında kimse yokken duvar kağıdı tam hızda çalışmaya devam
          ediyordu. Klavye ve fareye bu süre boyunca dokunulmazsa son kare
          ekranda kalır, ilk tuşta geri gelir.
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
          Windows'un kendi giriş sayacını okur — tuş kaydeden bir şey
          kurulmaz, hiçbir tuş görülmez.
        </p>
      </section>

      <section className="card">
        <h2 className="card-title">Makine meşgulken</h2>
        <p className="card-sub">
          Duvar kağıdı pahalı hale gelmez; geri kalan her şey pahalı hale
          gelir. Derleme, güncelleme, oyun yüklenirken duvar kağıdı yoldan
          çekilsin. Ölçüm makinenin tamamına ait ve saniyede bir okunuyor.
          {status &&
            (status.busy
              ? ` Şu an geri çekildi (makinenin %${Math.round(status.load)}'i kullanımda).`
              : ` Şu an makinenin %${Math.round(status.load)}'i kullanımda.`)}
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
            <span>Windows "animasyonları göster" kapalıysa hiç oynatma</span>
          </label>
        </div>
      </section>

      <section className="card">
        <h2 className="card-title">Ölçekleme</h2>
        <p className="card-sub">
          Video ile ekranın en boy oranı tutmadığında ne olacağı.
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
      </section>

      <section className="card">
        <h2 className="card-title">Oynatma</h2>
        <p className="card-sub">
          Hız videonun kendi karelerini yeniden zamanlar — çözücüden fazladan
          hiçbir şey istenmez, yavaşlatmak işi azaltır bile.
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
      </section>

      <section className="card">
        <h2 className="card-title">Geçiş</h2>
        <p className="card-sub">
          Bir duvar kağıdından diğerine geçerken. Geçiş sırasında eski karenin
          bir kopyası tutulur ve biter bitmez bırakılır.
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
      </section>

      <section className="card">
        <h2 className="card-title">Liste geçişi</h2>
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
      </section>

      <section className="card">
        <h2 className="card-title">Görünüm</h2>
        <p className="card-sub">
          Duvar kağıdının kendisi değişmez — bunlar ekrana çizilirken
          uygulanır, her ekranda aynı.
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
      </section>

      <section className="card">
        <h2 className="card-title">Ses</h2>
        <p className="card-sub">
          Videonun kendi sesi, birincil ekrandaki duvar kağıdından. Her ekran
          kapandığında (tam ekran oyun, kilit ekranı) kendiliğinden susar.
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
            <span>Başka bir uygulama ses çalarken geri çekil</span>
          </label>
          {status?.ducking && <span className="muted">şu an geri çekildi</span>}
        </div>
      </section>

      <section className="card">
        <h2 className="card-title">Bellek</h2>
        <p className="card-sub">
          Bir videonun belleği neredeyse tamamen çözücünün kare tamponudur ve
          onu belirleyen tek şey karenin boyutudur. Bütçe seçmek, çözücüden
          daha küçük kare istemek demek — küçük ekranda zaten görünmeyen
          pikseller.
          {status && ` Motor şu an ${status.ram_mb} MB kullanıyor.`}
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
          Değiştirmek oynayan her klibi baştan açar — kısa bir duraklama
          görürsün. Kalıcı çözüm kitaplıktaki <strong>Hafiflet</strong>:
          klibi bir kez küçük yazar, sonra hiçbir maliyeti olmaz.
        </p>
      </section>

      <section className="card">
        <h2 className="card-title">Görünmezken</h2>
        <p className="card-sub">
          Masaüstü kapalıyken çizim zaten duruyor. Duran şey çözücü değil —
          tam ekran oyun açıkken kare tamponları bellekte kalmaya devam
          ediyor. Bu süre dolunca motor onları da bırakır; ekran son karede
          kalır, masaüstü göründüğünde geri açılır.
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
      </section>

      <section className="card">
        <h2 className="card-title">Hareket</h2>
        <p className="card-sub">
          İkisi de kareyi nereden örneklediğimizi değiştirir — fazladan geçiş,
          doku veya bellek yok. Kapalıyken hiçbir şey ölçülmez: ne ses
          ölçeri açılır ne imleç sorulur.
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
          Makineden çıkan sesin tamamını ölçer — çalan neyse odur, Muivly
           kendi sesi kapalı olsa bile.
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
      </section>

      <section className="card">
        <h2 className="card-title">Duran görselde sürüklenme</h2>
        <p className="card-sub">
          Bir fotoğraf Muivly'nin gösterebileceği en ucuz duvar kağıdı: bir
          kez çözülür, bir kez yüklenir, sonra hiçbir maliyeti kalmaz. Bu
          ayar onu hareketlendirir ama çözücü eklemez — kareden hangi
          bölgenin örneklendiği çok yavaş kayar, doku yok, geçiş yok. Tek
          bedeli ekranın kare hızında yeniden çizilmesi.
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
          Yalnız fotoğraflara uygulanır. Video, GIF ve shader zaten
          hareketli; ikisi üst üste binerse tek bir resim değil, birbiriyle
          kavga eden iki hareket olur.
        </p>
      </section>

      <section className="card">
        <h2 className="card-title">Vurgu rengi</h2>
        <p className="card-sub">
          Windows'un vurgu rengi duvar kağıdından gelsin. Renk, ekrandaki
          karenin 16×9 küçültülmüş halinden okunuyor — duvar kağıdı
          değiştiğinde, kare başına değil. Okunabilir kalması için parlaklığı
          bir aralığa çekiliyor.
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
          Yazılan her şey <code>HKEY_CURRENT_USER</code> altında ve
          kendinden önceki değerler bir dosyaya yedekleniyor. Kapattığında,
          motoru kapattığında ya da motor çökerse bir sonraki açılışta senin
          kendi renklerin geri konur. Görev çubuğu bazen bir sonraki oturum
          açılışını bekler — orası Windows'un bileceği iş.
        </p>
      </section>

      {status && status.uptime_secs > 0 && (
        <section className="card">
          <h2 className="card-title">Ne kadarı boşa gitmedi</h2>
          <p className="card-sub">
            Bu projenin tamamı bu sayı için var ve şimdiye kadar görebildiğin
            tek yer Görev Yöneticisi'ydi. Motor{' '}
            <strong>{longDuration(status.uptime_secs)}</strong> çalıştı, bunun{' '}
            <strong>{longDuration(status.resting_secs)}</strong> kadarında
            hiçbir şey çizmedi — ekran kapalıydı, üstü örtülüydü, donmuştu ya
            da başında kimse yoktu.
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
          <p className="card-sub">
            Motorun bu açılışından beri. Kendi makinende ölçmek istersen{' '}
            <code>muivly-core --benchmark &lt;dosya&gt;</code> yarım dakika
            oynatıp CPU, bellek ve kare tablosunu basar.
          </p>
        </section>
      )}

      <section className="card">
        <h2 className="card-title">Uygulama kuralları</h2>
        <p className="card-sub">
          Buradaki uygulamalardan biri öndeyken duvar kağıdı donar: son kare
          ekranda kalır, çözme ve çizme durur. Tam ekran oyun zaten masaüstünü
          kapattığı için gerekmez — bu, masaüstünü kapatmayan ama makineyi
          isteyen işler için: render, derleme, görüntülü görüşme.
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
          Virgülle ayır. <code>.exe</code> yazmak zorunda değilsin.
        </p>
      </section>

      <Automation
        store={store}
        rules={status?.rules ?? []}
        onChange={(rules: Rule[]) => {
          void engine.setRules(rules).then(onRefresh)
        }}
      />

      <section className="card">
        <h2 className="card-title">Kısayollar ve menü</h2>
        <p className="card-sub">
          Kısayollar masaüstünün her yerinde çalışır. Bir kombinasyon başka bir
          uygulamada kayıtlıysa yalnız o çalışmaz, diğerleri çalışır.
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
          Menüden seçilen dosya her ekrana atanır. Pencere açılmaz — yalnız
          motora bir satır gider.
        </p>
      </section>

      <section className="card">
        <h2 className="card-title">Paketler</h2>
        <p className="card-sub">
          <code>.muivly</code> bir duvar kağıdını adı ve künyesiyle birlikte tek
          dosyada taşır. İçi zip, yani Muivly olmayan biri de açabilir.
          Kitaplıktaki her duvar kağıdının kendi "Paket yap" düğmesi var.
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
      </section>

      <section className="card">
        <h2 className="card-title">Windows ile başlat</h2>
        <p className="card-sub">
          Açılışta yalnız motor başlar, bu pencere değil — son duvar kağıdını
          ve ayarlarını kendisi hatırlıyor. Ayar paneli açılmadığı için
          açılışta WebView belleği de harcanmaz.
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
      </section>

      <section className="card">
        <h2 className="card-title">Performans</h2>
        <p className="card-sub">
          Motorun kendi ölçümü, saniyede bir yenilenir. Görev
          Yöneticisi'ndeki `muivly-core` ile aynı sayılar.
        </p>
        <div className="stats">
          <div className="stat">
            <span className="stat-value">{status ? status.cpu.toFixed(1) : '—'}%</span>
            <span className="stat-label">CPU (tek çekirdek payı)</span>
          </div>
          <div className="stat">
            <span className="stat-value">{status ? status.ram_mb : '—'} MB</span>
            <span className="stat-label">Bellek</span>
          </div>
          <div className="stat">
            <span className="stat-value">{status ? status.real_fps.toFixed(0) : '—'}</span>
            <span className="stat-label">Gerçek fps (tavan {status?.fps ?? '—'})</span>
          </div>
        </div>
      </section>

      <section className="card">
        <h2 className="card-title">Motor</h2>
        <p className="card-sub">
          Duvar kağıdı motoru ayrı bir işlem. Bu pencereyi kapatmak onu
          durdurmaz — X tuşu uygulamayı sistem tepsisine küçültür.
        </p>
        <div className="row">
          <button
            className={status?.frozen ? 'primary' : undefined}
            disabled={!status}
            title="Son kare ekranda kalır, çözme ve çizme durur"
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
      </section>

      <section className="card">
        <h2 className="card-title">Veri</h2>
        <p className="card-sub">
          Kitaplık, listeler ve atamalar burada. Videolar kopyalanmaz, yalnız
          yolları saklanır.
        </p>
        <div className="path muted">{statePath}</div>
      </section>
    </>
  )
}
