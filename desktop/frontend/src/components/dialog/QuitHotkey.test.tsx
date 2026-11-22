// Mounts the quit hotkey without the rest of the app shortcuts.
// Keyboard events exercise the platform modifier, user
// overrides and the command delivered to the Wails host.

import { afterEach, beforeEach, expect, test } from 'bun:test'
import { cleanup, render } from '@testing-library/react'
import { isMac, setShortcutOverrides } from '../../lib/shortcuts'
import { ui$ } from '../../states/ui'
import { QuitHotkey } from './QuitHotkey'

let calls: string[]
beforeEach(() => {
  calls = []
  ;(window as any).go = {
    main: {
      App: {
        Invoke: async (command: string) => {
          calls.push(command)
          return { ok: true }
        },
      },
    },
  }
})

afterEach(() => {
  cleanup()
  setShortcutOverrides({})
  ui$.settingsOpen.set(false)
  delete (window as any).go
})

const press = (key: string, mods: KeyboardEventInit = {}) => {
  const event = new KeyboardEvent('keydown', { key, cancelable: true, ...mods })
  window.dispatchEvent(event)
  return event
}

// Mounted on its own, as on the first-run setup screen, where the rest of the
// app's shortcuts (AppHotkeys) are not.
test.skipIf(isMac)('Ctrl+Q asks the Go side to quit and takes the keystroke', () => {
  render(<QuitHotkey />)
  const event = press('q', { ctrlKey: true })
  expect(calls).toEqual(['app.quit'])
  expect(event.defaultPrevented).toBe(true)
})

test.skipIf(isMac)('other keys are left alone', () => {
  render(<QuitHotkey />)
  const plain = press('q')
  const other = press('w', { ctrlKey: true })
  expect(calls).toEqual([])
  expect(plain.defaultPrevented).toBe(false)
  expect(other.defaultPrevented).toBe(false)
})

test.if(isMac)('leaves ⌘Q to macOS', () => {
  render(<QuitHotkey />)
  const event = press('q', { metaKey: true })
  expect(calls).toEqual([])
  expect(event.defaultPrevented).toBe(false)
})

test.skipIf(isMac)('quit rebound to a single key stands down while typing or in a modal', () => {
  setShortcutOverrides({ 'app.quit': { key: 'Enter' } })
  render(<QuitHotkey />)

  const input = document.createElement('input')
  document.body.appendChild(input)
  const typed = new KeyboardEvent('keydown', { key: 'Enter', cancelable: true, bubbles: true })
  input.dispatchEvent(typed)
  input.remove()

  ui$.settingsOpen.set(true)
  const inModal = press('Enter')
  ui$.settingsOpen.set(false)

  expect(calls).toEqual([])
  expect(typed.defaultPrevented).toBe(false)
  expect(inModal.defaultPrevented).toBe(false)

  press('Enter')
  expect(calls).toEqual(['app.quit'])
})
