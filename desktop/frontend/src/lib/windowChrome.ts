// Reads the native decoration layout and window capabilities. Desktop titlebar
// controls use that state to place the available buttons.
// Callers include SettingsDialog.tsx and TitleBar.tsx.

import { observable } from '@legendapp/state'
import { invoke } from './bridge'

// The integrated title bar (Linux): GTK draws the window frame but no title
// bar, and Meron's own (TitleBar.tsx) takes its place, drawing the window
// controls itself. Button set, side and double-click action follow the desktop
// (GNOME's button-layout and action-double-click-titlebar, as GTK reports
// them) instead of settings of our own.

export type WindowButton = 'minimize' | 'maximize' | 'close'

export type DecorationLayout = { start: WindowButton[]; end: WindowButton[] }

type WindowChromeReply = {
  supported?: boolean
  integrated?: boolean
  wanted?: boolean
  platform?: string
  layout?: string
  doubleClick?: string
}

export const windowChrome$ = observable({
  /** Whether this window can drop the system title bar (Linux, Windows). */
  supported: false,
  /** Whether Meron's title bar is in effect now. */
  integrated: false,
  /** The saved choice; differs from `integrated` until a restart where the
   *  switch can't apply to the live window (Windows, KDE). */
  wanted: false,
  /** Whose window controls to draw: 'windows' or the Linux desktop's ('linux'). */
  platform: '',
  layout: { start: [], end: ['close'] } as DecorationLayout,
  doubleClick: 'toggle-maximize',
  maximised: false,
})

const BUTTONS: readonly string[] = ['minimize', 'maximize', 'close']

/**
 * Parse GTK's decoration layout ("appmenu:minimize,maximize,close"): buttons
 * before the colon go on the left, after it on the right. Entries that aren't
 * window buttons (appmenu, menu, icon) are dropped, and so are repeats.
 */
export function parseDecorationLayout(layout: string): DecorationLayout {
  const seen = new Set<string>()
  const side = (part: string | undefined) =>
    (part ?? '')
      .split(',')
      .map((name) => name.trim())
      .filter((name) => BUTTONS.includes(name) && !seen.has(name) && seen.add(name)) as WindowButton[]
  const colon = layout.indexOf(':')
  if (colon === -1) return { start: [], end: side(layout) }
  const start = side(layout.slice(0, colon))
  return { start, end: side(layout.slice(colon + 1)) }
}

function applyReply(reply: WindowChromeReply) {
  windowChrome$.supported.set(reply.supported === true)
  windowChrome$.integrated.set(reply.integrated === true)
  windowChrome$.wanted.set(reply.wanted === true)
  windowChrome$.platform.set(typeof reply.platform === 'string' ? reply.platform : '')
  if (typeof reply.layout === 'string' && reply.layout !== '') {
    windowChrome$.layout.set(parseDecorationLayout(reply.layout))
  }
  if (typeof reply.doubleClick === 'string' && reply.doubleClick !== '') {
    windowChrome$.doubleClick.set(reply.doubleClick)
  }
  document.documentElement.classList.toggle('titlebar-integrated', reply.integrated === true)
}

export async function loadWindowChrome() {
  applyReply(await invoke<WindowChromeReply>('window.chrome'))
}

/**
 * Samples the window (the backend also saves its size from this) and applies
 * what the frame depends on: rounded corners while a Linux window floats, and
 * restore instead of maximize on the window controls.
 */
export async function syncWindowFrame() {
  const result = await invoke<{ rounded?: boolean; maximised?: boolean }>('window.resized')
  document.documentElement.classList.toggle('window-rounded', result?.rounded === true)
  windowChrome$.maximised.set(result?.maximised === true)
}

/**
 * Settles the window chrome before the first render, so the first frame
 * already has the right title bar and corners instead of the defaults being
 * replaced a moment later. Capped, so a slow or missing backend never holds
 * up the app.
 */
export function windowChromeReady(timeoutMs = 300): Promise<void> {
  const loaded = Promise.allSettled([loadWindowChrome(), syncWindowFrame()]).then(() => {})
  return Promise.race([loaded, new Promise<void>((resolve) => setTimeout(resolve, timeoutMs))])
}

export async function setIntegratedTitlebar(integrated: boolean) {
  await invoke('window.setTitlebar', { integrated })
  await loadWindowChrome()
}

export function windowCommand(command: 'minimise' | 'toggleMaximise' | 'close') {
  void invoke(`window.${command}`).catch(() => {})
}

// Whether a press on this element moves the window: the --wails-draggable that
// index.css sets on title bar rows and clears on the controls inside them,
// read the way Wails reads it, so a double-click acts exactly where a drag would.
// Custom properties inherit, so in the webview the target's own value is the
// answer; walking up also covers a DOM that doesn't cascade them (happy-dom).
function isDragRegion(target: Element) {
  for (let element: Element | null = target; element; element = element.parentElement) {
    const value = getComputedStyle(element).getPropertyValue('--wails-draggable').trim()
    if (value) return value === 'drag'
  }
  return false
}

/** The action GNOME runs for a double-click on a title bar, if we can do it. */
export function doubleClickCommand(action: string): 'minimise' | 'toggleMaximise' | null {
  if (action === 'toggle-maximize') return 'toggleMaximise'
  if (action === 'minimize') return 'minimise'
  // toggle-maximize-horizontally/-vertically, lower, menu and none have no
  // Wails equivalent; doing nothing is closest for all of them.
  return null
}

/**
 * Double-clicking empty space in a title bar region runs the desktop's
 * double-click action; the button layout is re-read when the window regains
 * focus, since it can change in GNOME Settings meanwhile.
 */
export function installWindowChrome(): () => void {
  const onDoubleClick = (event: MouseEvent) => {
    if (!windowChrome$.integrated.peek() || event.button !== 0) return
    const target = event.target
    if (!(target instanceof Element) || !isDragRegion(target)) return
    const command = doubleClickCommand(windowChrome$.doubleClick.peek())
    if (command) windowCommand(command)
  }
  const onFocus = () => {
    void loadWindowChrome().catch(() => {})
  }
  window.addEventListener('dblclick', onDoubleClick)
  window.addEventListener('focus', onFocus)
  onFocus()
  return () => {
    window.removeEventListener('dblclick', onDoubleClick)
    window.removeEventListener('focus', onFocus)
  }
}
