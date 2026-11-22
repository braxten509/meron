// The desktop titlebar reads the operating system's
// decoration layout. These cases cover left and right
// button groups, repeated entries and unsupported window
// controls.

import { afterEach, describe, expect, it } from 'bun:test'
import {
  doubleClickCommand,
  installWindowChrome,
  loadWindowChrome,
  parseDecorationLayout,
  windowChrome$,
  windowChromeReady,
} from './windowChrome'

const original = (window as any).go

afterEach(() => {
  ;(window as any).go = original
  document.documentElement.classList.remove('titlebar-integrated')
})

describe('parseDecorationLayout', () => {
  it('splits the buttons at the colon', () => {
    expect(parseDecorationLayout('appmenu:minimize,maximize,close')).toEqual({
      start: [],
      end: ['minimize', 'maximize', 'close'],
    })
    expect(parseDecorationLayout('close,minimize:')).toEqual({ start: ['close', 'minimize'], end: [] })
  })

  it('drops entries that are not window buttons, and repeats', () => {
    expect(parseDecorationLayout('menu:close')).toEqual({ start: [], end: ['close'] })
    expect(parseDecorationLayout('icon,close:close, maximize ,spacer')).toEqual({
      start: ['close'],
      end: ['maximize'],
    })
  })

  it('reads a layout without a colon as the right side, and an empty one as no buttons', () => {
    expect(parseDecorationLayout('close')).toEqual({ start: [], end: ['close'] })
    expect(parseDecorationLayout('')).toEqual({ start: [], end: [] })
  })
})

describe('doubleClickCommand', () => {
  it('maps GNOME double-click actions Wails can perform', () => {
    expect(doubleClickCommand('toggle-maximize')).toBe('toggleMaximise')
    expect(doubleClickCommand('minimize')).toBe('minimise')
    expect(doubleClickCommand('none')).toBeNull()
    expect(doubleClickCommand('menu')).toBeNull()
  })
})

describe('loadWindowChrome', () => {
  it('applies the reply and marks the document', async () => {
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async () => ({
            supported: true,
            integrated: true,
            wanted: false,
            platform: 'windows',
            layout: 'close:minimize',
            doubleClick: 'minimize',
          }),
        },
      },
    }
    await loadWindowChrome()
    expect(windowChrome$.integrated.peek()).toBe(true)
    expect(windowChrome$.wanted.peek()).toBe(false)
    expect(windowChrome$.platform.peek()).toBe('windows')
    expect(windowChrome$.layout.peek()).toEqual({ start: ['close'], end: ['minimize'] })
    expect(windowChrome$.doubleClick.peek()).toBe('minimize')
    expect(document.documentElement.classList.contains('titlebar-integrated')).toBe(true)
  })
})

describe('installWindowChrome', () => {
  it('runs the double-click action on the drag region only, not on controls or tabs in it', () => {
    const calls: string[] = []
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string) => {
            calls.push(command)
            return command === 'window.chrome' ? { supported: true, integrated: true } : {}
          },
        },
      },
    }
    const style = document.createElement('style')
    style.textContent = `
      [data-titlebar] { --wails-draggable: drag; }
      [data-titlebar] :is(button, [data-tab-id]) { --wails-draggable: no-drag; }`
    document.head.append(style)
    document.body.innerHTML = `<div data-titlebar><span id="empty">x</span><div data-tab-id="t"><span id="tab">Tab</span></div><button id="button">B</button></div>`
    windowChrome$.integrated.set(true)
    windowChrome$.doubleClick.set('toggle-maximize')
    const uninstall = installWindowChrome()
    calls.length = 0
    try {
      for (const id of ['tab', 'button', 'empty']) {
        document.getElementById(id)!.dispatchEvent(new MouseEvent('dblclick', { bubbles: true, button: 0 }))
      }
      expect(calls).toEqual(['window.toggleMaximise'])
    } finally {
      uninstall()
      style.remove()
      document.body.innerHTML = ''
    }
  })
})

describe('windowChromeReady', () => {
  it('applies the chrome and the frame before resolving', async () => {
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string) =>
            command === 'window.chrome' ? { supported: true, integrated: true } : { rounded: true, maximised: true },
        },
      },
    }
    await windowChromeReady()
    expect(windowChrome$.integrated.peek()).toBe(true)
    expect(windowChrome$.maximised.peek()).toBe(true)
    expect(document.documentElement.classList.contains('window-rounded')).toBe(true)
    document.documentElement.classList.remove('window-rounded')
  })

  it('never holds up the first render for long', async () => {
    ;(window as any).go = { main: { App: { Invoke: () => new Promise(() => {}) } } }
    const started = Date.now()
    await windowChromeReady(20)
    expect(Date.now() - started).toBeLessThan(200)
  })

  it('renders anyway without a backend', async () => {
    ;(window as any).go = undefined
    await windowChromeReady()
  })
})
