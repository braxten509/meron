// Clears the browser media session associated with message content when a desktop
// conversation stops using it.
// Callers include HtmlFrame.tsx and MessagePane.tsx.

export function clearMediaSession(win: Window | null | undefined = window) {
  const session = win?.navigator?.mediaSession
  if (!session) return
  try {
    session.metadata = null
    session.playbackState = 'none'
  } catch {
    // Some WebKitGTK builds expose MediaSession partially.
  }
}
