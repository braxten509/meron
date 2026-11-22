// The quit shortcut, kept apart from AppHotkeys so it can be mounted on the first-run
// setup screen too, where the rest of the app's shortcuts have nothing to act on.
// Keystrokes forwarded out of a message iframe are replayed on window by AppHotkeys,
// so they reach this listener as well.

import { useEffect } from 'react'
import { invoke } from '../../lib/bridge'
import { isBareKeystroke, isMac, isTyping, matchShortcut } from '../../lib/shortcuts'
import { modalOpen } from '../../states/ui'

export function QuitHotkey() {
  useEffect(() => {
    // macOS quits natively on ⌘Q.
    if (isMac) return

    const onKeyDown = (event: KeyboardEvent) => {
      if (matchShortcut(event) !== 'app.quit') return
      // Quit rebound to a single key follows the other single-key shortcuts:
      // never while typing or with a modal open (Enter on a dialog button).
      if (isBareKeystroke(event) && (isTyping(event.target) || modalOpen())) return
      // The Go side decides: hide to the tray with close-to-tray on, quit with
      // it off.
      event.preventDefault()
      void invoke('app.quit').catch(() => {})
    }

    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])

  return null
}
