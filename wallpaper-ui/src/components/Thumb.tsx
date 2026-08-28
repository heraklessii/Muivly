import { useCallback, useEffect, useRef, useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'

/** Extensions the engine decodes with WIC — a still, so no `<video>` for it.
 *
 *  `.gif` belongs here even though it moves: no browser plays one through a
 *  `<video>` element, so leaving it out meant every animated GIF in the
 *  library — a format the engine supports and the file picker offers — drew
 *  itself as the "could not read this file" tile. An `<img>` animates it. */
const STILL = /\.(png|jpe?g|bmp|webp|gif)$/i

/** Shaders. There is nothing for the WebView to show: the picture does not
 *  exist until the engine's GPU runs the program. A tile that says so is a
 *  better answer than a `<video>` failing and reading as a broken file. */
const SHADER = /\.(hlsl|fx)$/i

/** What the file turns out to be, once the WebView has looked at it. */
export type Meta = {
  width: number
  height: number
  /** Zero for a still. */
  seconds: number
}

type Props = {
  path: string
  /** Where to freeze a video, in seconds. */
  seconds?: number
  /** Let the clip run — the grid turns this on for the card under the cursor. */
  play?: boolean
  /**
   * Called once the dimensions are known.
   *
   * The decoder the library needs is the engine's, but the WebView has one
   * too and it has already opened the file to draw this frame — so the
   * resolution and the running time come for free rather than from a media
   * probe of our own.
   */
  onMeta?: (meta: Meta) => void
}

/**
 * How far outside the viewport a tile still counts as worth loading.
 *
 * Roughly a screenful in either direction: far enough that scrolling at a
 * normal speed never reaches an empty tile, close enough that a library of
 * hundreds only ever has a dozen or so files open.
 */
const NEAR = '800px'

/** How big a captured poster is allowed to be. A tile is a few hundred
 *  pixels wide on any screen this app runs on; anything past this is memory
 *  spent on detail nobody can see. */
const POSTER_WIDTH = 640

/** How long one capture may take before it is abandoned. Generous: a 4K clip
 *  on a slow disk is not a broken file. */
const CAPTURE_TIMEOUT = 10000

/**
 * Posters already captured, by path, and what the file turned out to be.
 *
 * Module level rather than component state: tiles unmount as they scroll out
 * of view, and re-decoding a file the user has merely scrolled past twice is
 * the cost this whole component exists to avoid.
 */
const posters = new Map<string, { url: string; meta: Meta }>()

/** Enough for a large library; past it the oldest poster is released rather
 *  than holding every frame the user has ever scrolled past. */
const POSTER_CAP = 400

/**
 * Captures run one at a time.
 *
 * A grid coming into view would otherwise open a dozen decoders at once,
 * which is the burst this is meant to prevent — briefly, but on the machines
 * Muivly is for a brief dozen 4K decoders is a stutter the user feels.
 */
let queue: Promise<unknown> = Promise.resolve()

/**
 * One frame of a video file, as a small still, with the video thrown away.
 *
 * A `<video>` element is not a picture: it is a decoder plus a compositing
 * layer, and the layer is re-composited for as long as the element is in the
 * document — at the refresh rate of whatever screen it is on. Measured on a
 * 3840x2160 clip in front of a 180 Hz panel, one paused tile cost 59% of a
 * core and about 450 MB, for a picture that never changed. Twelve tiles is
 * not twelve times better behaved.
 *
 * So the frame is copied into a canvas a tile's worth of pixels wide and the
 * video is released. `crossOrigin` is what makes that legal: Tauri's asset
 * protocol answers with `Access-Control-Allow-Origin` set to the window's own
 * origin, so the canvas is not tainted and `toBlob` is allowed. Without the
 * attribute the request is not a CORS request at all, the canvas is tainted,
 * and this throws — which is why the fallback below matters.
 */
function capture(path: string, seconds: number): Promise<{ url: string; meta: Meta }> {
  return new Promise((resolve, reject) => {
    const video = document.createElement('video')
    video.crossOrigin = 'anonymous'
    video.muted = true
    video.playsInline = true
    video.preload = 'metadata'

    // Two pixels, almost transparent, in the corner — and genuinely on
    // screen, which is the part that matters.
    //
    // The browser does not render video nobody can see, and it is thorough
    // about it: detached, off-viewport and `opacity:0` all count as nobody.
    // An element in any of those states still reports its duration and
    // dimensions, so `drawImage` succeeds and paints black — the tile ends
    // up with the right resolution and running time under a picture of
    // nothing, which looks far more like a broken file than a broken
    // thumbnailer. Two visible pixels is the smallest price for a real
    // frame, and it is paid for a fraction of a second per file.
    video.style.cssText =
      'position:fixed;left:0;top:0;width:2px;height:2px;opacity:0.02;pointer-events:none;z-index:2147483647'
    document.body.appendChild(video)

    let settled = false
    let giveUp = 0
    const done = (outcome: { url: string; meta: Meta } | Error) => {
      if (settled) return
      settled = true
      clearTimeout(giveUp)
      // Drop the decoder and the buffered data with it.
      video.removeAttribute('src')
      video.load()
      video.remove()
      if (outcome instanceof Error) reject(outcome)
      else resolve(outcome)
    }

    // Nothing may wedge the queue. A file the decoder never answers about
    // would otherwise stop every later tile from being captured at all, and
    // leave its own decoder running the whole time it waited.
    giveUp = window.setTimeout(() => done(new Error(`gave up reading ${path}`)), CAPTURE_TIMEOUT)

    video.onerror = () => done(new Error(`could not read ${path}`))

    video.onloadeddata = () => {
      video.currentTime = Math.min(seconds, Math.max(0, video.duration - 0.1))
    }

    // `seeked` says the decoder has the frame; a frame is given to the
    // compositor on the next one. Two of them, because the first is the
    // frame the seek was requested on.
    //
    // Not `requestVideoFrameCallback`, which is the precise answer to the
    // same question and useless here: it fires when a *new* frame is
    // presented, and a video paused on the frame it was just seeked to will
    // never present another. It simply never calls back.
    const whenPainted = (then: () => void) =>
      requestAnimationFrame(() => requestAnimationFrame(then))

    video.onseeked = () => whenPainted(() => {
      const meta: Meta = {
        width: video.videoWidth,
        height: video.videoHeight,
        seconds: Number.isFinite(video.duration) ? video.duration : 0,
      }

      try {
        const scale = Math.min(1, POSTER_WIDTH / Math.max(1, meta.width))
        const canvas = document.createElement('canvas')
        canvas.width = Math.max(1, Math.round(meta.width * scale))
        canvas.height = Math.max(1, Math.round(meta.height * scale))

        const context = canvas.getContext('2d')
        if (!context) throw new Error('no 2d context')
        context.drawImage(video, 0, 0, canvas.width, canvas.height)

        canvas.toBlob(
          (blob) => {
            if (!blob) {
              done(new Error('the frame would not encode'))
              return
            }
            done({ url: URL.createObjectURL(blob), meta })
          },
          'image/jpeg',
          0.82,
        )
      } catch (e) {
        done(e instanceof Error ? e : new Error(String(e)))
      }
    })

    // Assigned last, so none of the handlers can be missed.
    video.src = convertFileSrc(path)
  })
}

/** Remember a poster, releasing the oldest once there are too many. */
function remember(path: string, entry: { url: string; meta: Meta }) {
  posters.set(path, entry)
  while (posters.size > POSTER_CAP) {
    const oldest = posters.keys().next()
    if (oldest.done) break
    const gone = posters.get(oldest.value)
    if (gone) URL.revokeObjectURL(gone.url)
    posters.delete(oldest.value)
  }
}

/**
 * A frame from a wallpaper file.
 *
 * At rest a tile is a still image — see `capture` for why it is emphatically
 * not a `<video>`. The video element only exists while the tile is actually
 * being played, which the grid allows for one tile at a time: the one under
 * the cursor.
 *
 * The media element only exists while the tile is on screen or nearly so.
 * A library big enough to be worth organising is exactly the one that would
 * otherwise hold hundreds of decoders; unmounting what has scrolled away is
 * the whole of the fix, and the grid keeps its real height so scroll position
 * and keyboard navigation stay honest.
 */
export default function Thumb({ path, seconds = 1, play = false, onMeta }: Props) {
  const [poster, setPoster] = useState<string | null>(() => posters.get(path)?.url ?? null)
  const [failed, setFailed] = useState(false)
  const [near, setNear] = useState(false)
  const watcher = useRef<IntersectionObserver | null>(null)
  const video = useRef<HTMLVideoElement | null>(null)
  const still = STILL.test(path)
  const shader = SHADER.test(path)

  // Held in a ref rather than depended on: the grid hands down a fresh
  // closure on every render, and a decode must not be re-run for that.
  const report = useRef(onMeta)
  report.current = onMeta

  /**
   * Watch whichever element is currently standing in for this tile — the
   * placeholder before it loads, the picture afterwards. Both fill the tile,
   * so either answers the only question being asked.
   */
  const watch = useCallback((element: HTMLElement | null) => {
    watcher.current?.disconnect()
    if (!element) return

    const observer = new IntersectionObserver(
      (entries) => setNear(entries[0].isIntersecting),
      { rootMargin: NEAR },
    )
    observer.observe(element)
    watcher.current = observer
  }, [])

  useEffect(() => () => watcher.current?.disconnect(), [])

  // A different file in the same tile — the grid re-sorts under the cursor,
  // a wallpaper is renamed — starts over rather than showing the old picture.
  useEffect(() => {
    setPoster(posters.get(path)?.url ?? null)
    setFailed(false)
  }, [path])

  // The poster is captured once per file, on the queue, and only for a tile
  // that is actually near the viewport.
  useEffect(() => {
    if (still || shader || !near || failed) return

    const known = posters.get(path)
    if (known) {
      setPoster(known.url)
      report.current?.(known.meta)
      return
    }

    let live = true
    queue = queue
      .then(() => {
        // The tile scrolled away, or another copy of it won the race, while
        // this one waited its turn.
        if (!live) return
        const raced = posters.get(path)
        if (raced) {
          setPoster(raced.url)
          report.current?.(raced.meta)
          return
        }
        return capture(path, seconds).then((entry) => {
          remember(path, entry)
          if (!live) return
          setPoster(entry.url)
          report.current?.(entry.meta)
        })
      })
      .catch(() => {
        if (live) setFailed(true)
      })

    return () => {
      live = false
    }
  }, [path, seconds, near, still, shader, failed])

  // Playing is the one time a real decoder is wanted. It is created when the
  // cursor arrives and destroyed when it leaves, so the expensive thing only
  // exists while somebody is looking straight at it.
  useEffect(() => {
    const element = video.current
    if (!element || !play) return
    // A rejected play() is normal here: the element can be torn down between
    // the hover and the promise settling.
    void element.play().catch(() => {})
  }, [play])

  // The poster stays up until the clip has something of its own to show, or
  // hovering a tile would blink it black while the decoder opens.
  const [running, setRunning] = useState(false)
  useEffect(() => {
    if (!play) setRunning(false)
  }, [play])

  // Nothing to load, so no observer and no loading state: the tile is its
  // own final answer.
  if (shader) {
    return (
      <div className="thumb thumb-shader" title="Shader — önizlemesi motorda çizilir">
        <span>&lt;/&gt;</span>
      </div>
    )
  }

  if (failed) {
    return (
      <div ref={watch} className="thumb thumb-missing" title="Dosya okunamadı">
        <span>!</span>
      </div>
    )
  }

  // A photograph is already a picture: an `<img>` is the cheapest thing the
  // WebView has and there is no frame to capture.
  if (still) {
    return near ? (
      <img
        ref={watch}
        className="thumb"
        src={convertFileSrc(path)}
        alt=""
        onLoad={(e) =>
          report.current?.({
            width: e.currentTarget.naturalWidth,
            height: e.currentTarget.naturalHeight,
            seconds: 0,
          })
        }
        onError={() => setFailed(true)}
      />
    ) : (
      <div ref={watch} className="thumb" />
    )
  }

  return (
    <>
      {/* The still the tile shows whenever it is not being played. It also
          carries the observer, so a tile that has scrolled far away stops
          asking to be captured. A tile merely off screen holds its space
          quietly: the loading pulse is an animation, and a few hundred of
          them running out of sight is the sort of thing this grid is trying
          to stop.

          Taken out of the layout rather than merely hidden while the clip
          is running: both fill the tile, so leaving it in would push the
          video below it. */}
      {poster ? (
        <img
          ref={watch}
          className="thumb"
          src={poster}
          alt=""
          style={running ? { display: 'none' } : undefined}
        />
      ) : (
        <div ref={watch} className={near ? 'thumb thumb-loading' : 'thumb'} />
      )}

      {play && (
        <video
          ref={video}
          className="thumb"
          style={running ? undefined : { display: 'none' }}
          src={convertFileSrc(path)}
          muted
          loop
          playsInline
          preload="auto"
          onLoadedData={() => setRunning(true)}
          onError={() => setFailed(true)}
        />
      )}
    </>
  )
}
