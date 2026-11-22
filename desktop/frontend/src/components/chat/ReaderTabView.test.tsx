// Checks that opening or changing a reader tab preserves each
// tab's scroll position. The test mounts conversation readers
// and observes which scrolling container receives the update.

import { afterEach, describe, expect, it } from 'bun:test'
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react'
import '../../states/compose'
import type { MessageTab } from '../../types'
import { ReaderTabView } from './ReaderTabView'

afterEach(cleanup)

describe('reader tab scroll isolation', () => {
  for (const viewMode of ['html', 'plain'] as const) {
    it(`starts new ${viewMode} tabs at the top and restores each tab's saved position`, async () => {
      const tab = (id: string): MessageTab => ({
        id,
        kind: 'reader',
        messageId: id,
        threadId: id,
        subject: id,
        from: 'Sender',
        body: 'Caption',
        bodyHtml: '<p>Caption</p>',
        attachments: [{ key: 'clip.mp4', filename: 'clip.mp4', mime: 'video/mp4', size: 0, url: null }],
        viewMode,
      })
      const a = tab(`scroll-a-${viewMode}`)
      const b = tab(`scroll-b-${viewMode}`)
      const view = render(<ReaderTabView tab={a} />)
      const scroller = () => view.container.querySelector<HTMLElement>('.overflow-y-auto')!
      const loadFrame = () => {
        const frame = view.container.querySelector('iframe')
        if (!frame) return
        const doc = document.implementation.createHTMLDocument('Feed')
        Object.defineProperty(doc, 'defaultView', { value: window })
        doc.body.getBoundingClientRect = () => new DOMRect(0, 0, 600, 1200)
        doc.documentElement.getBoundingClientRect = doc.body.getBoundingClientRect
        Object.defineProperty(frame, 'contentDocument', { value: doc })
        Object.defineProperty(frame, 'contentWindow', { value: window })
        fireEvent.load(frame)
      }

      loadFrame()
      const firstScroller = scroller()
      firstScroller.scrollTop = 400
      fireEvent.scroll(firstScroller)

      view.rerender(<ReaderTabView tab={b} />)
      expect(scroller()).not.toBe(firstScroller)
      expect(scroller().scrollTop).toBe(0)
      loadFrame()
      scroller().scrollTop = 150
      fireEvent.scroll(scroller())

      view.rerender(<ReaderTabView tab={a} />)
      loadFrame()
      await waitFor(() => expect(scroller().scrollTop).toBe(400))

      view.rerender(<ReaderTabView tab={b} />)
      loadFrame()
      await waitFor(() => expect(scroller().scrollTop).toBe(150))
    })
  }
})
