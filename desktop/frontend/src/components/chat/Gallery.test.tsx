// Inline pictures can have a remote source as well as a
// local attachment key. These fixtures exercise the
// gallery actions available for the image the reader
// selected.

import { afterEach, expect, it } from 'bun:test'
import { cleanup, fireEvent, render } from '@testing-library/react'
import { Gallery } from './Gallery'

afterEach(cleanup)

it("offers save for an inline picture's absolute src and keeps its right-click from the menus behind", () => {
  let parentMenus = 0
  const view = render(
    <div onContextMenu={() => parentMenus++}>
      <Gallery
        items={[{ src: `${window.location.origin}/media/msg/0.png`, filename: 'Logo' }]}
        index={0}
        onIndexChange={() => {}}
        onClose={() => {}}
      />
    </div>,
  )
  expect(view.getByLabelText(/Logo/)).toBeTruthy()
  fireEvent.contextMenu(view.getByAltText('Logo'))
  expect(parentMenus).toBe(0)
  expect(view.getByText('Copy image')).toBeTruthy()
})
