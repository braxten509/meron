// Observes whether the desktop viewport requires the narrow layout. Components
// share this decision when choosing how navigation is presented.
// Callers include App.tsx and ContactsView.tsx.

import { useSyncExternalStore } from 'react'

// Below Tailwind's `max-[900px]`, where the right-hand panels are hidden.
const NARROW_QUERY = '(max-width: 899.98px)'

/** Whether the window is too narrow for the right-hand panels. */
export function isNarrowWindow(): boolean {
  return typeof matchMedia === 'function' && matchMedia(NARROW_QUERY).matches
}

function subscribe(onChange: () => void): () => void {
  if (typeof matchMedia !== 'function') return () => {}
  const query = matchMedia(NARROW_QUERY)
  query.addEventListener?.('change', onChange)
  return () => query.removeEventListener?.('change', onChange)
}

/** `isNarrowWindow`, re-rendering as the window crosses the breakpoint. */
export function useNarrowWindow(): boolean {
  return useSyncExternalStore(subscribe, isNarrowWindow)
}
