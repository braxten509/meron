import { useEffect, useState, type ImgHTMLAttributes } from 'react'

// WebKit shrinks a picture to its box in one bilinear step, which turns a
// 96-1024 px photo or logo shown at 16-40 px into a blocky mess. Here it is
// shrunk once, in halving steps, to exactly the box's device pixels, and the
// result is kept, so small pictures look smooth everywhere they are shown.

type Fit = 'cover' | 'contain'

const MAX_ENTRIES = 512
const done = new Map<string, string | null>() // key -> blob URL, or null: use the original
const running = new Map<string, Promise<string | null>>()

const keyOf = (src: string, px: number, fit: Fit) => `${fit}:${px}:${src}`

function remember(key: string, url: string | null) {
  done.set(key, url)
  while (done.size > MAX_ENTRIES) {
    const [oldKey, oldUrl] = done.entries().next().value!
    done.delete(oldKey)
    if (oldUrl) URL.revokeObjectURL(oldUrl)
  }
}

async function shrink(src: string, px: number, fit: Fit): Promise<string | null> {
  const img = new Image()
  // Remote pictures without CORS can't be read back; they fail here and the
  // original is used, so nothing is lost.
  if (/^https?:/i.test(src) && !src.startsWith(window.location.origin)) img.crossOrigin = 'anonymous'
  img.decoding = 'async'
  img.src = src
  await img.decode()
  const { naturalWidth: w, naturalHeight: h } = img
  if (!w || !h) return null
  const scale = fit === 'cover' ? px / Math.min(w, h) : px / Math.max(w, h)
  // Up to 2x is smooth enough already.
  if (scale >= 0.5) return null
  const tw = Math.max(1, Math.round(w * scale))
  const th = Math.max(1, Math.round(h * scale))
  let source: CanvasImageSource = img
  let cw = w
  let ch = h
  while (cw / 2 >= tw && ch / 2 >= th) {
    cw = Math.round(cw / 2)
    ch = Math.round(ch / 2)
    source = draw(source, cw, ch)
  }
  const out = draw(source, tw, th)
  const blob = await new Promise<Blob | null>((resolve) => out.toBlob(resolve, 'image/png'))
  return blob ? URL.createObjectURL(blob) : null
}

function draw(source: CanvasImageSource, width: number, height: number): HTMLCanvasElement {
  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  const ctx = canvas.getContext('2d')!
  ctx.imageSmoothingEnabled = true
  ctx.imageSmoothingQuality = 'high'
  ctx.drawImage(source, 0, 0, width, height)
  return canvas
}

/** `src` shrunk smoothly for a `box` css-px box (the original until ready, or
 *  when it is small enough, or can't be read). */
export function useSharpSrc(src: string | undefined, box: number, fit: Fit = 'cover'): string | undefined {
  // Twice the box's device pixels, as an @2x picture: full detail kept, and
  // the screen's own last step (at most 2x) is smooth.
  const px = Math.ceil(box * (window.devicePixelRatio || 1) * 2)
  const key = src ? keyOf(src, px, fit) : ''
  const [sharp, setSharp] = useState<{ key: string; url: string | null } | null>(null)
  useEffect(() => {
    if (!src || done.has(key)) return
    let cancelled = false
    let job = running.get(key)
    if (!job) {
      job = shrink(src, px, fit).catch(() => null)
      running.set(key, job)
      void job.then((url) => {
        running.delete(key)
        remember(key, url)
      })
    }
    void job.then((url) => {
      if (!cancelled) setSharp({ key, url })
    })
    return () => {
      cancelled = true
    }
  }, [src, key, px, fit])
  if (!src) return undefined
  const ready = done.has(key) ? done.get(key) : sharp?.key === key ? sharp.url : null
  return ready || src
}

/** An `<img>` for a small fixed box (`box` css px), drawn smoothly. */
export function SharpImg({
  src,
  box,
  fit = 'cover',
  ...props
}: ImgHTMLAttributes<HTMLImageElement> & { src?: string; box: number; fit?: Fit }) {
  return <img {...props} src={useSharpSrc(src, box, fit)} />
}
