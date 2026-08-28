/**
 * Browsing motionbgs.com without leaving the app.
 *
 * Muivly works entirely offline; this is a place to get wallpapers from, not
 * something the app needs. Nothing here runs until this view is opened, and
 * nothing is sent but the request for a public page.
 *
 * The HTML is parsed here rather than in Rust because the WebView already
 * has a parser and an HTML crate would be a large dependency to do the same
 * job worse. Rust only fetches and saves — see src-tauri/src/web.rs, which
 * also holds the host allowlist that keeps this view to the one site.
 */
import { useCallback, useEffect, useRef, useState } from 'react'

import { disk, fileSize, web } from '../api'

const SITE = 'https://motionbgs.com'

/** What the chips across the top offer. The site groups by tag; these are
 *  the groups worth putting one click away, and every one of them is a tag
 *  the site actually has — a made-up tag is an empty page, not an error. */
const SECTIONS: { label: string; path: string }[] = [
  { label: 'Tümü', path: '/4k' },
  { label: 'Anime', path: '/tag:anime' },
  { label: 'Oyun', path: '/tag:games' },
  { label: 'Doğa', path: '/tag:nature' },
  { label: 'Araba', path: '/tag:car' },
  { label: 'Uzay', path: '/tag:space' },
  { label: 'Şehir', path: '/tag:city' },
  { label: 'Soyut', path: '/tag:abstract' },
  { label: 'Süper kahraman', path: '/tag:superhero' },
  { label: 'Hayvan', path: '/tag:animal' },
  { label: 'Yağmur', path: '/tag:rain' },
  { label: 'Müzik', path: '/tag:music' },
]

/**
 * Which copy to download.
 *
 * The site keeps a 4K and a 1080p file for each wallpaper. On the machines
 * Muivly is for, the 1080p one is usually the better wallpaper: a quarter of
 * the pixels to decode every frame, a quarter of the disk, and on a 1080p
 * screen nothing visible to lose.
 */
const QUALITIES = [
  { id: '4k', label: '4K', help: 'tam çözünürlük' },
  { id: 'hd', label: '1080p', help: 'daha hafif' },
] as const

type Quality = (typeof QUALITIES)[number]['id']

/** Placeholder cards for the first page, so the grid has a shape while the
 *  page is on its way instead of jumping into existence. */
const SKELETONS = 12

type Wallpaper = {
  id: string
  slug: string
  title: string
  /** The resolution baked into the file names, e.g. `3840x2160`. */
  size: string
  thumb: string
}

/**
 * Pull the wallpapers out of a listing page.
 *
 * Everything needed is in the thumbnail path — `/i/c/364x205/media/<id>/
 * <slug>.<size>.jpg` — which is also the most stable thing on the page: the
 * class names are Tailwind and change with the design, the media path is
 * how the site addresses its own files.
 */
function parse(html: string): Wallpaper[] {
  const doc = new DOMParser().parseFromString(html, 'text/html')
  const found = new Map<string, Wallpaper>()

  for (const link of doc.querySelectorAll('a[href]')) {
    const image = link.querySelector('img')
    const source = image?.getAttribute('src') ?? ''

    const match = source.match(/\/media\/(\d+)\/(.+?)\.(\d+x\d+)\.jpg/)
    if (!match) continue

    const [, id, slug, size] = match
    // The same wallpaper can appear in more than one row of a listing.
    if (found.has(id)) continue

    const title =
      link.querySelector('.ttl')?.textContent?.trim() ||
      link.getAttribute('title')?.replace(/\s*live wallpaper$/i, '') ||
      slug.replace(/-/g, ' ')

    found.set(id, { id, slug, title, size, thumb: `${SITE}${source}` })
  }

  return [...found.values()]
}

/** `3840x2160` reads better as `4K`; anything else keeps its height. */
function resolution(size: string): string {
  const height = Number(size.split('x')[1] ?? 0)
  if (height >= 2160) return '4K'
  if (height >= 1440) return '1440p'
  if (height >= 1080) return '1080p'
  return size
}

/** What a downloaded file is called, per quality. */
function fileName(item: Wallpaper, quality: Quality): string {
  return quality === 'hd' ? `${item.slug}.1080p.mp4` : `${item.slug}.${item.size}.mp4`
}

type Props = {
  /** Library file paths, so an already-downloaded wallpaper says so. */
  have: string[]
  /** Called with the downloaded file path once it is on disk. */
  onDownloaded: (path: string, title: string) => void
}

export default function Browse({ have, onDownloaded }: Props) {
  const [section, setSection] = useState<string>(SECTIONS[0].path)
  /** Set while showing search results; the chips are inactive then. */
  const [search, setSearch] = useState<string | null>(null)
  const [term, setTerm] = useState('')
  const [quality, setQuality] = useState<Quality>('4k')

  const [items, setItems] = useState<Wallpaper[]>([])
  const [loading, setLoading] = useState(false)
  const [end, setEnd] = useState(false)
  const [error, setError] = useState<string | null>(null)
  /** Ids currently downloading, so a second click cannot start a second one. */
  const [busy, setBusy] = useState<string[]>([])
  /** Downloaded this session: id to the size on disk, for the card to show. */
  const [got, setGot] = useState<Record<string, number>>({})
  /** Hide what is already downloaded. Off by default — the badge is enough
   *  until the library has grown enough for it not to be. */
  const [hideOwned, setHideOwned] = useState(false)

  // Kept in refs so the scroll observer below can read them without being
  // rebuilt — and torn down — on every page that arrives.
  const page = useRef(1)
  const state = useRef({ loading, end, section, search })
  state.current = { loading, end, section, search }

  /**
   * One page of a listing, or the single page a search answers with.
   *
   * Search has no pagination on the site: `?q=` returns one set of matches
   * and `&page=2` returns the same one, so asking for more would loop.
   */
  const load = useCallback(
    async (path: string, query: string | null, wanted: number) => {
      setLoading(true)
      setError(null)
      try {
        // Page one has no number in the path; the rest are `/4k/2/`.
        const url = query
          ? `${SITE}/search?q=${encodeURIComponent(query)}`
          : wanted === 1
            ? `${SITE}${path}/`
            : `${SITE}${path}/${wanted}/`

        const found = parse(await web.fetch(url))
        if (query) setEnd(true)

        if (found.length === 0) {
          setEnd(true)
          return
        }

        setItems((previous) => {
          if (wanted === 1) return found
          // Listings overlap at the edges often enough to matter.
          const seen = new Set(previous.map((item) => item.id))
          return [...previous, ...found.filter((item) => !seen.has(item.id))]
        })
      } catch (e) {
        setError(String(e))
        setEnd(true)
      } finally {
        setLoading(false)
      }
    },
    [],
  )

  // Switching sections replaces the grid under a reader who may be a
  // thousand pixels down it. Without this the new page arrives already
  // scrolled past its own end, and the sentinel immediately asks for more.
  const top = useRef<HTMLDivElement>(null)
  const firstLoad = useRef(true)

  useEffect(() => {
    setItems([])
    setEnd(false)
    page.current = 1
    void load(section, search, 1)

    if (firstLoad.current) firstLoad.current = false
    else top.current?.scrollIntoView({ block: 'start' })
  }, [section, search, load])

  /** Ask for the page that failed again, without losing what did arrive. */
  function retry() {
    setEnd(false)
    void load(section, search, page.current)
  }

  // Endless scroll: a sentinel below the grid asks for the next page as it
  // comes into view, so the common case — keep looking — costs no clicks.
  const sentinel = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const target = sentinel.current
    if (!target) return

    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries[0].isIntersecting) return
        if (state.current.loading || state.current.end) return
        page.current += 1
        void load(state.current.section, state.current.search, page.current)
      },
      { rootMargin: '400px' },
    )

    observer.observe(target)
    return () => observer.disconnect()
  }, [load])

  /** Already on disk, under either quality's name. */
  function inLibrary(item: Wallpaper): string | null {
    const names = QUALITIES.map((entry) => fileName(item, entry.id))
    return have.find((path) => names.some((name) => path.endsWith(name))) ?? null
  }

  async function download(item: Wallpaper) {
    setBusy((current) => [...current, item.id])
    setError(null)
    try {
      const path = await web.download(
        `${SITE}/dl/${quality}/${item.id}`,
        fileName(item, quality),
      )
      onDownloaded(path, item.title)

      // What it actually cost on disk, which is the one number worth
      // knowing before downloading a dozen more.
      const info = await disk.infos([path]).catch(() => ({}) as Record<string, never>)
      setGot((current) => ({ ...current, [item.id]: info[path]?.size ?? 0 }))
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy((current) => current.filter((id) => id !== item.id))
    }
  }

  function runSearch() {
    const wanted = term.trim()
    setSearch(wanted === '' ? null : wanted)
  }

  const downloaded = Object.keys(got).length

  /** Already on disk, either from an earlier session or from this one. */
  function owned(item: Wallpaper): boolean {
    return inLibrary(item) !== null || item.id in got
  }

  const visible = hideOwned ? items.filter((item) => !owned(item)) : items
  const hidden = items.length - visible.length

  return (
    <>
      <div ref={top} />

      <header className="view-head">
        <div>
          <h1 className="view-title">Keşfet</h1>
          <p className="view-sub">
            motionbgs.com — ücretsiz canlı duvar kağıtları. İndirdiklerin
            doğrudan kitaplığına eklenir; başka hiçbir yere bağlanılmaz.
            {downloaded > 0 && ` Bu oturumda ${downloaded} indirme.`}
          </p>
        </div>
      </header>

      <div className="toolbar">
        <input
          className="search"
          type="search"
          placeholder="Sitede ara"
          value={term}
          onChange={(e) => setTerm(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') runSearch()
            if (e.key === 'Escape') {
              setTerm('')
              setSearch(null)
            }
          }}
        />
        <button onClick={runSearch}>Ara</button>

        <div className="spacer" />

        <button
          className="chip"
          data-active={hideOwned}
          title="Zaten kitaplığında olanları listeden çıkar"
          onClick={() => setHideOwned(!hideOwned)}
        >
          Kitaplıktakileri gizle
          {hideOwned && hidden > 0 ? ` (${hidden})` : ''}
        </button>

        {/* Which copy the download button fetches. */}
        <div className="chips">
          {QUALITIES.map((entry) => (
            <button
              key={entry.id}
              className="chip"
              data-active={quality === entry.id}
              title={entry.help}
              onClick={() => setQuality(entry.id)}
            >
              {entry.label}
            </button>
          ))}
        </div>
      </div>

      <div className="chips section-chips">
        {SECTIONS.map((entry) => (
          <button
            key={entry.path}
            className="chip"
            data-active={search === null && section === entry.path}
            onClick={() => {
              setSearch(null)
              setTerm('')
              setSection(entry.path)
            }}
          >
            {entry.label}
          </button>
        ))}
      </div>

      {search !== null && (
        <p className="muted search-note">
          “{search}” için sonuçlar — site aramayı tek sayfada veriyor.{' '}
          <button className="link" onClick={() => setSearch(null)}>
            temizle
          </button>
        </p>
      )}

      {error && (
        <p className="error-text">
          {error}{' '}
          <button className="link" onClick={retry}>
            tekrar dene
          </button>
        </p>
      )}

      <div className="wall">
        {visible.map((item) => {
          const downloading = busy.includes(item.id)
          const have = owned(item)

          return (
            <article className="wp" key={item.id} data-live={have}>
              <div className="wp-media">
                <img className="thumb" src={item.thumb} alt="" loading="lazy" />

                <span className="wp-kind">{resolution(item.size)}</span>

                {have && (
                  <span className="wp-flag">
                    <span className="dot" />
                    Kitaplıkta
                  </span>
                )}

                <div className="wp-overlay" data-busy={downloading}>
                  <button
                    className="primary"
                    disabled={downloading || have}
                    onClick={() => void download(item)}
                  >
                    {downloading
                      ? 'İndiriliyor…'
                      : have
                        ? 'İndirildi'
                        : `İndir · ${quality === 'hd' ? '1080p' : '4K'}`}
                  </button>
                  <span className="wp-hint">
                    {downloading ? 'dosya kitaplığa eklenecek' : item.title}
                  </span>
                </div>
              </div>

              <div className="wp-meta">
                <div className="wp-name" title={item.title}>
                  {item.title}
                </div>
                <div className="wp-facts">
                  <span>{item.size}</span>
                  <span>{quality === 'hd' ? '1080p indir' : '4K indir'}</span>
                  {got[item.id] > 0 && <span>{fileSize(got[item.id])}</span>}
                </div>
              </div>
            </article>
          )
        })}

        {loading &&
          Array.from({ length: items.length === 0 ? SKELETONS : 4 }, (_, i) => (
            <div className="wp wp-skeleton" key={`skeleton-${i}`}>
              <div className="thumb thumb-loading" />
            </div>
          ))}
      </div>

      <div ref={sentinel} className="scroll-sentinel" />

      {end && items.length > 0 && !loading && (
        <p className="muted center">Bu kadar — listenin sonu.</p>
      )}

      {end && items.length === 0 && !error && !loading && (
        <div className="card empty">
          <p>
            {search !== null
              ? `“${search}” için bir şey bulunamadı.`
              : 'Bu bölümde şu an bir şey yok.'}
          </p>
        </div>
      )}
    </>
  )
}
