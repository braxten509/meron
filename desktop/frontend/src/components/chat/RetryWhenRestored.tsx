// Remounts media after a previously missing attachment file becomes available. The
// reader can recover its image without reopening the entire conversation.
// Imported by MessageContent.tsx.

import { Fragment, useEffect, useRef, useState } from 'react'
import type { ReactNode } from 'react'

/**
 * Remounts its media when attachment files that were missing come back.
 *
 * A message can be shown before its files are on disk, and their `/media` URLs
 * are the same before and after, so an element whose request failed has no
 * reason to ask again. Only one that did fail is remounted: a video that is
 * playing is left alone when some other attachment of its message is restored.
 */
export function RetryWhenRestored({ mediaMissing, children }: { mediaMissing: number; children: ReactNode }) {
  const hostRef = useRef<HTMLDivElement | null>(null)
  const failedRef = useRef(false)
  const pendingRef = useRef(new Set<EventTarget>())
  const seenRef = useRef(mediaMissing)
  const [attempt, setAttempt] = useState(0)

  // `error` on media does not bubble, so it is caught on the way down.
  useEffect(() => {
    const host = hostRef.current
    if (!host) return
    const onError = (event: Event) => {
      if (event.target && pendingRef.current.has(event.target)) {
        pendingRef.current.clear()
        failedRef.current = false
        setAttempt((count) => count + 1)
      } else {
        failedRef.current = true
      }
    }
    const onLoaded = (event: Event) => {
      if (event.target) pendingRef.current.delete(event.target)
    }
    host.addEventListener('error', onError, true)
    host.addEventListener('load', onLoaded, true)
    host.addEventListener('loadedmetadata', onLoaded, true)
    return () => {
      host.removeEventListener('error', onError, true)
      host.removeEventListener('load', onLoaded, true)
      host.removeEventListener('loadedmetadata', onLoaded, true)
      pendingRef.current.clear()
    }
  }, [])

  useEffect(() => {
    const restored = mediaMissing < seenRef.current
    seenRef.current = mediaMissing
    if (!restored) return
    if (!failedRef.current) {
      for (const media of hostRef.current?.querySelectorAll('img, video, audio') ?? []) {
        const pending =
          media instanceof HTMLImageElement
            ? !media.complete
            : (media as HTMLMediaElement).readyState === 0 && !(media as HTMLMediaElement).error
        if (pending) pendingRef.current.add(media)
      }
      return
    }
    pendingRef.current.clear()
    failedRef.current = false
    setAttempt((count) => count + 1)
  }, [mediaMissing])

  return (
    <div ref={hostRef} className="contents">
      <Fragment key={attempt}>{children}</Fragment>
    </div>
  )
}
