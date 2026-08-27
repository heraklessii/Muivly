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

/**
 * A frame from a wallpaper file.
 *
 * A paused `<video>` seeked a little way in, rather than a frame copied onto
 * a canvas: local files are served over Tauri's asset protocol, which is a
 * different origin from the app, so drawing one to a canvas taints it and
 * `toDataURL` throws. Showing the element itself sidesteps that entirely.
 *
 * The media element only exists while the tile is on screen or nearly so.
 * `preload="metadata"` keeps each one to a header read and a single decoded
 * frame, but a `<video>` is a decoder however little it decodes, and browsers
 * cap how many may exist at once — somewhere around fifty, after which every
 * further tile silently fails to load. A library big enough to be worth
 * organising is exactly the one that hits that. Unmounting what has scrolled
 * away is the whole of the fix: the grid keeps its real height, so scroll
 * position and keyboard navigation stay honest, and only the decoders go.
 *
 * Only the hovered tile is ever allowed to play, so the cost of the preview
 * is one software-decoded clip — never a wall of them.
 */
export default function Thumb({ path, seconds = 1, play = false, onMeta }: Props) {
  const [state, setState] = useState<'loading' | 'ready' | 'failed'>('loading')
  const [near, setNear] = useState(false)
  const video = useRef<HTMLVideoElement | null>(null)
  const watcher = useRef<IntersectionObserver | null>(null)
  const still = STILL.test(path)
  const shader = SHADER.test(path)

  /**
   * Watch whichever element is currently standing in for this tile — the
   * placeholder before it loads, the media itself afterwards. Both fill the
   * tile, so either answers the only question being asked.
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

  /** The video element is both what plays and what is watched. */
  const attachVideo = useCallback(
    (element: HTMLVideoElement | null) => {
      video.current = element
      watch(element)
    },
    [watch],
  )

  useEffect(() => () => watcher.current?.disconnect(), [])

  // A tile that scrolled away lost its element; when it comes back it starts
  // over rather than claiming to be ready with nothing on screen.
  useEffect(() => {
    if (!near) setState('loading')
  }, [near])

  useEffect(() => {
    const element = video.current
    if (!element || still || state !== 'ready') return

    if (play) {
      // A rejected play() is normal here: the element can be torn down
      // between the hover and the promise settling.
      void element.play().catch(() => {})
    } else {
      element.pause()
      element.currentTime = Math.min(seconds, Math.max(0, element.duration - 0.1))
    }
  }, [play, state, seconds, still])

  // Nothing to load, so no observer and no loading state: the tile is its
  // own final answer.
  if (shader) {
    return (
      <div className="thumb thumb-shader" title="Shader — önizlemesi motorda çizilir">
        <span>&lt;/&gt;</span>
      </div>
    )
  }

  if (state === 'failed') {
    return (
      <div ref={watch} className="thumb thumb-missing" title="Dosya okunamadı">
        <span>!</span>
      </div>
    )
  }

  const hidden = state === 'loading' ? { display: 'none' } : undefined

  return (
    <>
      {/* Also what carries the observer while there is no media to carry it.
          A tile that is merely off screen holds its space quietly: the
          loading pulse is an animation, and a few hundred of them running
          out of sight is the sort of thing this grid is trying to stop. */}
      {(!near || state === 'loading') && (
        <div
          ref={near ? undefined : watch}
          className={near ? 'thumb thumb-loading' : 'thumb'}
        />
      )}

      {near &&
        (still ? (
          <img
            ref={watch}
            className="thumb"
            style={hidden}
            src={convertFileSrc(path)}
            alt=""
            onLoad={(e) => {
              setState('ready')
              onMeta?.({
                width: e.currentTarget.naturalWidth,
                height: e.currentTarget.naturalHeight,
                seconds: 0,
              })
            }}
            onError={() => setState('failed')}
          />
        ) : (
          <video
            ref={attachVideo}
            className="thumb"
            style={hidden}
            src={convertFileSrc(path)}
            muted
            loop
            playsInline
            preload="metadata"
            // Seeking a little way in avoids the black or blank first frame
            // most encoders produce.
            onLoadedData={(e) => {
              const element = e.currentTarget
              element.currentTime = Math.min(seconds, Math.max(0, element.duration - 0.1))
              onMeta?.({
                width: element.videoWidth,
                height: element.videoHeight,
                seconds: element.duration,
              })
            }}
            onSeeked={() => setState('ready')}
            onError={() => setState('failed')}
          />
        ))}
    </>
  )
}
