// Dialogs can replace or nest inside one another while focus
// moves between openers. The fixtures exercise focus
// restoration, keyboard trapping and rendering outside
// clipping parents.

import { afterEach, expect, test } from 'bun:test'
import { cleanup, fireEvent, render } from '@testing-library/react'
import { useRef, useState } from 'react'
import { Dialog } from './Dialog'
import { FloatingContextMenu } from '../menu/FloatingContextMenu'

afterEach(cleanup)

test('removing a child dialog opener keeps focus in its surviving parent', () => {
  function Example() {
    const [open, setOpen] = useState(false)
    const [confirm, setConfirm] = useState(false)
    const [deleted, setDeleted] = useState(false)
    return (
      <>
        <button onClick={() => setOpen(true)}>Open</button>
        {open && (
          <Dialog onClose={() => setOpen(false)} aria-label="Parent">
            <button>Surviving control</button>
            {!deleted && <button onClick={() => setConfirm(true)}>Delete</button>}
            {confirm && (
              <Dialog onClose={() => setConfirm(false)} aria-label="Confirm">
                <button
                  onClick={() => {
                    setDeleted(true)
                    setConfirm(false)
                  }}
                >
                  Confirm delete
                </button>
              </Dialog>
            )}
          </Dialog>
        )}
      </>
    )
  }
  const view = render(<Example />)
  view.getByText('Open').focus()
  fireEvent.click(view.getByText('Open'))
  view.getByText('Delete').focus()
  fireEvent.click(view.getByText('Delete'))
  fireEvent.click(view.getByText('Confirm delete'))
  expect(view.queryByRole('dialog', { name: 'Confirm' })).toBeNull()
  expect(document.activeElement).toBe(view.getByText('Surviving control'))
})

test('replacement dialogs restore the original opener after their predecessor is removed', () => {
  function Example() {
    const [step, setStep] = useState(0)
    return (
      <>
        <button onClick={() => setStep(1)}>Open</button>
        {step > 0 && (
          <Dialog key={step} onClose={() => setStep(0)} aria-label="Step">
            <button autoFocus onClick={() => setStep(2)}>
              Next
            </button>
          </Dialog>
        )}
      </>
    )
  }
  const view = render(<Example />)
  const opener = view.getByText('Open')
  opener.focus()
  fireEvent.click(opener)
  fireEvent.click(view.getByText('Next'))
  fireEvent.keyDown(window, { key: 'Escape' })
  expect(document.activeElement).toBe(opener)
})

test('portals out of clipping parents, honors autofocus, traps Tab and restores the opener', () => {
  function Example() {
    const [open, setOpen] = useState(false)
    return (
      <div style={{ overflow: 'hidden', transform: 'translateX(0)' }}>
        <button onClick={() => setOpen(true)}>Open</button>
        {open && (
          <Dialog onClose={() => setOpen(false)} aria-label="Editor">
            <button>First</button>
            <input autoFocus aria-label="Name" />
            <button>Last</button>
          </Dialog>
        )}
      </div>
    )
  }
  const view = render(<Example />)
  const opener = view.getByText('Open')
  opener.focus()
  fireEvent.click(opener)
  const dialog = view.getByRole('dialog', { name: 'Editor' })
  expect(view.container.contains(dialog)).toBe(false)
  expect(document.activeElement).toBe(view.getByLabelText('Name'))
  view.getByText('Last').focus()
  fireEvent.keyDown(document.activeElement!, { key: 'Tab' })
  expect(document.activeElement).toBe(view.getByText('First'))
  fireEvent.keyDown(document.activeElement!, { key: 'Tab', shiftKey: true })
  expect(document.activeElement).toBe(view.getByText('Last'))
  fireEvent.keyDown(window, { key: 'Escape' })
  expect(view.queryByRole('dialog')).toBeNull()
  expect(document.activeElement).toBe(opener)
})

test('nested busy dialogs consume Escape and keep focus until dismissible', () => {
  function Example() {
    const [nested, setNested] = useState(false)
    const [busy, setBusy] = useState(true)
    const [open, setOpen] = useState(true)
    return (
      open && (
        <Dialog onClose={() => setOpen(false)} aria-label="Parent">
          <button onClick={() => setNested(true)}>Nested</button>
          {nested && (
            <Dialog onClose={() => setNested(false)} dismissible={!busy} aria-label="Child">
              <button onClick={() => setBusy(false)}>Finish</button>
            </Dialog>
          )}
        </Dialog>
      )
    )
  }
  const view = render(<Example />)
  fireEvent.click(view.getByText('Nested'))
  expect(document.activeElement).toBe(view.getByText('Finish'))
  fireEvent.keyDown(window, { key: 'Escape' })
  fireEvent.mouseDown(view.getByRole('dialog', { name: 'Child' }).parentElement!)
  expect(view.getAllByRole('dialog')).toHaveLength(2)
  fireEvent.keyDown(view.getByText('Finish'), { key: 'Tab' })
  expect(document.activeElement).toBe(view.getByText('Finish'))
  fireEvent.click(view.getByText('Finish'))
  fireEvent.keyDown(window, { key: 'Escape' })
  expect(view.queryByRole('dialog', { name: 'Child' })).toBeNull()
  expect(document.activeElement).toBe(view.getByText('Nested'))
  fireEvent.keyDown(window, { key: 'Escape' })
  expect(view.queryByRole('dialog')).toBeNull()
})

test('explicit initial focus and a portalled menu preserve the Escape stack', () => {
  function Example() {
    const focus = useRef<HTMLButtonElement>(null)
    const [menu, setMenu] = useState(false)
    return (
      <Dialog onClose={() => {}} aria-label="Parent" initialFocusRef={focus}>
        <button>First</button>
        <button ref={focus} onClick={() => setMenu(true)}>
          Menu
        </button>
        {menu && (
          <FloatingContextMenu x={10} y={10} className="fixed" onClose={() => setMenu(false)}>
            <button autoFocus>Action</button>
          </FloatingContextMenu>
        )}
      </Dialog>
    )
  }
  const view = render(<Example />)
  expect(document.activeElement).toBe(view.getByText('Menu'))
  fireEvent.click(view.getByText('Menu'))
  expect(document.activeElement).toBe(view.getByText('Action'))
  expect(view.getByText('Action').parentElement?.className).toContain('menu-surface')
  fireEvent.keyDown(window, { key: 'Escape' })
  expect(view.queryByText('Action')).toBeNull()
  expect(view.getByRole('dialog')).toBeTruthy()
})

test('backdrop clicks dismiss only when allowed, and empty dialogs keep keyboard focus', () => {
  let closed = 0
  const view = render(
    <Dialog onClose={() => closed++} aria-label="Empty" closeOnBackdrop={false}>
      <p>Content</p>
    </Dialog>,
  )
  const panel = view.getByRole('dialog')
  expect(document.activeElement).toBe(panel)
  fireEvent.keyDown(panel, { key: 'Tab' })
  expect(document.activeElement).toBe(panel)
  fireEvent.mouseDown(panel.parentElement!)
  expect(closed).toBe(0)
  view.rerender(
    <Dialog onClose={() => closed++} aria-label="Empty">
      <p>Content</p>
    </Dialog>,
  )
  fireEvent.mouseDown(panel)
  expect(closed).toBe(0)
  fireEvent.mouseDown(panel.parentElement!)
  expect(closed).toBe(1)
})

test('Tab skips disabled and hidden controls, and custom Escape does not change backdrop dismissal', () => {
  let closed = 0
  let escaped = 0
  const view = render(
    <Dialog onClose={() => closed++} onEscape={() => escaped++} aria-label="Editor">
      <div style={{ display: 'none' }}>
        <button>Hidden</button>
      </div>
      <button disabled>Disabled</button>
      <button>Visible</button>
    </Dialog>,
  )
  expect(document.activeElement).toBe(view.getByText('Visible'))
  fireEvent.keyDown(document.activeElement!, { key: 'Tab' })
  expect(document.activeElement).toBe(view.getByText('Visible'))
  fireEvent.keyDown(window, { key: 'Escape' })
  expect(escaped).toBe(1)
  expect(closed).toBe(0)
  fireEvent.mouseDown(view.getByRole('dialog').parentElement!)
  expect(closed).toBe(1)
})
