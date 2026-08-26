/**
 * First run.
 *
 * Muivly has one thing to explain that most apps do not: the wallpaper is a
 * separate process, and this window is only its settings panel. Everything
 * else here is the shortest path from an empty install to something moving on
 * the desktop — start the engine, add a video, put it on a screen.
 *
 * Every step can be skipped. Someone who closes this and pokes around the
 * sidebar instead should not be stuck.
 */
import { useEffect, useState } from 'react'

import Thumb from '../components/Thumb'
import { displayName, engine, pickVideos, type EngineStatus, type Monitor } from '../api'
import { withPaths, type Store } from '../store'

type Props = {
  store: Store
  status: EngineStatus | null
  monitors: Monitor[]
  onChange: (next: Store) => void
  onStartEngine: () => Promise<void>
  onApply: (monitorName: string, itemId: string) => Promise<void>
  onFinish: () => void
}

const STEPS = ['Merhaba', 'Motor', 'Video', 'Ekran', 'Hazır']

export default function Onboarding({
  store,
  status,
  monitors,
  onChange,
  onStartEngine,
  onApply,
  onFinish,
}: Props) {
  const [step, setStep] = useState(0)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [installed, setInstalled] = useState<boolean | null>(null)
  const [chosen, setChosen] = useState<string | null>(null)
  const [monitorName, setMonitorName] = useState<string | null>(null)
  const [applied, setApplied] = useState(false)

  // Only worth asking once, and only on the step where it changes the wording.
  useEffect(() => {
    if (step === 1 && installed === null) {
      void engine
        .installed()
        .then(setInstalled)
        .catch(() => setInstalled(null))
    }
  }, [step, installed])

  // Default the selection to whatever is most likely wanted: the wallpaper
  // just added, and the primary screen.
  const lastAdded = store.items[store.items.length - 1] ?? null
  const item = store.items.find((i) => i.id === chosen) ?? lastAdded
  const target =
    monitors.find((m) => m.name === monitorName) ??
    monitors.find((m) => m.primary) ??
    monitors[0] ??
    null

  async function addVideos() {
    setBusy(true)
    try {
      const next = withPaths(store, await pickVideos())
      if (next !== store) onChange(next)
      setError(null)
    } catch (e) {
      setError(`Dosya seçici açılamadı: ${e}`)
    } finally {
      setBusy(false)
    }
  }

  async function startEngine() {
    setBusy(true)
    try {
      await onStartEngine()
      setError(null)
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  async function apply() {
    if (!item || !target) return
    setBusy(true)
    try {
      await onApply(target.name, item.id)
      setApplied(true)
      setError(null)
      setStep(4)
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="onboarding">
      <div className="onboard-panel">
        <div className="onboard-rail">
          <div className="wordmark">
            Mui<span>vly</span>
          </div>

          <ol className="onboard-steps">
            {STEPS.map((label, i) => (
              <li
                key={label}
                className="onboard-step"
                data-state={i === step ? 'now' : i < step ? 'done' : 'next'}
              >
                <span className="onboard-bullet">{i < step ? '✓' : i + 1}</span>
                {label}
              </li>
            ))}
          </ol>

          <button className="onboard-skip" onClick={onFinish}>
            Turu atla
          </button>
        </div>

        <div className="onboard-body">
          {step === 0 && (
            <>
              <h1 className="onboard-title">Muivly&apos;ye hoş geldin</h1>
              <p className="onboard-lead">
                Masaüstüne video duvar kağıdı koyar. Tek hedefi hafif olmak:
                çözme işi baştan sona ekran kartında yapılır, tam ekran bir
                uygulama açtığında çizim tamamen durur.
              </p>
              <ul className="onboard-facts">
                <li>
                  <strong>Duvar kağıdı ayrı bir işlem.</strong> Bu pencereyi
                  kapatsan da çalışmaya devam eder, tepsi simgesinden geri
                  açılır.
                </li>
                <li>
                  <strong>Dosyaların kopyalanmaz.</strong> Kitaplık yalnızca
                  yolları saklar, diskte ikinci bir kopya oluşmaz.
                </li>
                <li>
                  <strong>Telemetri yok.</strong> Hiçbir şey ölçülmez,
                  gönderilmez; uygulama tamamen çevrimdışı çalışır.
                </li>
              </ul>
            </>
          )}

          {step === 1 && (
            <>
              <h1 className="onboard-title">Motoru başlat</h1>
              <p className="onboard-lead">
                Duvar kağıdını çizen program bu. Bir kez başlar, arka planda
                kalır ve ayar penceresi kapansa da çalışmaya devam eder.
              </p>

              {status ? (
                <p className="onboard-ok">Motor çalışıyor.</p>
              ) : installed === false ? (
                <p className="error-text">
                  muivly-core.exe uygulamanın yanında bulunamadı. Kurulumdan
                  geldiysen bu bir hata; kaynaktan çalıştırıyorsan önce{' '}
                  <code>cargo build --release</code> gerekiyor.
                </p>
              ) : (
                <button className="primary" disabled={busy} onClick={() => void startEngine()}>
                  Motoru başlat
                </button>
              )}
            </>
          )}

          {step === 2 && (
            <>
              <h1 className="onboard-title">İlk videonu ekle</h1>
              <p className="onboard-lead">
                Donanım çözücüsü olan her format çalışır — genelde mp4
                (H.264/HEVC) ve webm (VP9/AV1). Birden fazla dosya
                seçebilirsin.
              </p>

              <button className="primary" disabled={busy} onClick={() => void addVideos()}>
                Video ekle
              </button>

              {store.items.length > 0 && (
                <p className="onboard-ok">
                  Kitaplıkta {store.items.length} duvar kağıdı var.
                </p>
              )}
            </>
          )}

          {step === 3 && (
            <>
              <h1 className="onboard-title">Bir ekrana uygula</h1>
              <p className="onboard-lead">
                {monitors.length > 1
                  ? 'Her ekran kendi duvar kağıdını gösterebilir. Şimdilik birini seç, gerisini Ekranlar sekmesinden ayarlarsın.'
                  : 'Seçtiğin video, uygulandığı anda masaüstünde oynamaya başlar.'}
              </p>

              {store.items.length === 0 ? (
                <p className="muted">Önce bir video eklemen gerekiyor.</p>
              ) : (
                <div className="pick-grid">
                  {store.items.map((candidate) => (
                    <button
                      key={candidate.id}
                      className="pick-tile"
                      data-active={item?.id === candidate.id}
                      onClick={() => setChosen(candidate.id)}
                    >
                      <Thumb path={candidate.path} />
                      <span className="pick-name">{candidate.title}</span>
                    </button>
                  ))}
                </div>
              )}

              {monitors.length > 1 && (
                <div className="row onboard-target">
                  <span className="muted">Ekran</span>
                  <select
                    value={target?.name ?? ''}
                    onChange={(e) => setMonitorName(e.target.value)}
                  >
                    {monitors.map((m) => (
                      <option key={m.name} value={m.name}>
                        {displayName(m.name)}
                        {m.primary ? ' (birincil)' : ''}
                      </option>
                    ))}
                  </select>
                </div>
              )}

              {monitors.length === 0 && (
                <p className="muted">
                  Motor ekranları henüz bildirmedi; bir saniye içinde gelir.
                </p>
              )}
            </>
          )}

          {step === 4 && (
            <>
              <h1 className="onboard-title">{applied ? 'Hazır' : 'Kurulum tamam'}</h1>
              <p className="onboard-lead">
                {applied
                  ? 'Duvar kağıdın masaüstünde oynuyor. Bundan sonrası zevk meselesi.'
                  : 'İstediğin zaman kitaplığa dönüp bir duvar kağıdı uygulayabilirsin.'}
              </p>
              <ul className="onboard-facts">
                <li>
                  <strong>Listeler</strong> birkaç videoyu sırayla oynatır;
                  geçiş süresini Ayarlar belirler.
                </li>
                <li>
                  <strong>Ayarlar</strong> kare hızını ve videonun ekrana nasıl
                  oturacağını tutar — düşük kare hızı daha az güç harcar.
                </li>
                <li>
                  <strong>Tepsi simgesi</strong> bu pencereyi geri getirir;
                  pencereyi kapatmak duvar kağıdını durdurmaz.
                </li>
              </ul>
            </>
          )}

          {error && <p className="error-text">{error}</p>}

          <div className="onboard-foot">
            {step > 0 && (
              <button disabled={busy} onClick={() => setStep(step - 1)}>
                Geri
              </button>
            )}
            <div className="spacer" />
            {step === 3 ? (
              <button
                className="primary"
                disabled={busy || !item || !target}
                onClick={() => void apply()}
              >
                Uygula
              </button>
            ) : step === 4 ? (
              <button className="primary" onClick={onFinish}>
                Bitir
              </button>
            ) : (
              <button
                className="primary"
                disabled={
                  busy || (step === 1 && !status) || (step === 2 && store.items.length === 0)
                }
                onClick={() => setStep(step + 1)}
              >
                Devam
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}
