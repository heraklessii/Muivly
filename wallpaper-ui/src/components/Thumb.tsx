import { useState } from 'react'
import { convertFileSrc } from '@tauri-apps/api/core'

/**
 * A still frame from a video file.
 *
 * A paused `<video>` seeked a little way in, rather than a frame copied onto
 * a canvas: local files are served over Tauri's asset protocol, which is a
 * different origin from the app, so drawing one to a canvas taints it and
 * `toDataURL` throws. Showing the element itself sidesteps that entirely.
 *
 * `preload="metadata"` keeps this to a header read and one decoded frame per
 * tile. A library large enough for that to matter would want the grid
 * virtualised, which is a problem worth having.
 */
export default function Thumb({ path, seconds = 1 }: { path: string; seconds?: number }) {
  const [state, setState] = useState<'loading' | 'ready' | 'failed'>('loading')

  if (state === 'failed') {
    return (
      <div className="thumb thumb-missing" title="Dosya okunamadı">
        <span>?</span>
      </div>
    )
  }

  return (
    <>
      {state === 'loading' && <div className="thumb thumb-loading" />}
      <video
        className="thumb"
        style={state === 'loading' ? { display: 'none' } : undefined}
        src={convertFileSrc(path)}
        muted
        playsInline
        preload="metadata"
        // Seeking a little way in avoids the black or blank first frame most
        // encoders produce.
        onLoadedData={(e) => {
          const video = e.currentTarget
          video.currentTime = Math.min(seconds, Math.max(0, video.duration - 0.1))
        }}
        onSeeked={() => setState('ready')}
        onError={() => setState('failed')}
      />
    </>
  )
}
