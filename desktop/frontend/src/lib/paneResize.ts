// Tracks pointer movement while resizing the thread list or board conversation
// pane. Bounds and final cleanup belong to the resize interaction.
// Imported by App.tsx.

import type { PointerEvent as ReactPointerEvent } from 'react'
import { settings$ } from '../states/settings'

const MIN_THREAD_LIST_WIDTH = 280
const MAX_THREAD_LIST_WIDTH = 560
const MIN_KANBAN_PANE_WIDTH = 25
const MAX_KANBAN_PANE_WIDTH = 60

// A pointer over a message frame moves within the frame's own document, out of
// this window's sight, so the frames stop taking the pointer for the drag.
function beginDrag(onMove: (event: PointerEvent) => void) {
  const onUp = () => {
    window.removeEventListener('pointermove', onMove)
    window.removeEventListener('pointerup', onUp)
    document.body.style.cursor = ''
    document.body.style.userSelect = ''
    document.body.classList.remove('pane-resizing')
  }
  document.body.style.cursor = 'col-resize'
  document.body.style.userSelect = 'none'
  document.body.classList.add('pane-resizing')
  window.addEventListener('pointermove', onMove)
  window.addEventListener('pointerup', onUp)
}

// Drag the kanban conversation pane's left edge; width persists as a percentage
// of the viewport, matching KanbanConversationPane's vw sizing. Its right edge
// excludes the content frame's border/margin and any open Tasks panel.
export function startKanbanResize(event: ReactPointerEvent<HTMLDivElement>) {
  event.preventDefault()
  const pane = event.currentTarget.parentElement
  if (!pane) return
  beginDrag((moveEvent) => {
    const rect = pane.getBoundingClientRect()
    const raw = ((rect.right - moveEvent.clientX) / window.innerWidth) * 100
    const next = Math.min(MAX_KANBAN_PANE_WIDTH, Math.max(MIN_KANBAN_PANE_WIDTH, raw))
    settings$.kanbanPaneWidth.set(Math.round(next))
  })
}

// Drag the thread list's right edge; width persists in pixels.
export function startThreadListResize(event: ReactPointerEvent<HTMLDivElement>) {
  event.preventDefault()
  const pane = event.currentTarget.parentElement
  if (!pane) return
  const paneLeft = pane.getBoundingClientRect().left
  beginDrag((moveEvent) => {
    const raw = moveEvent.clientX - paneLeft
    const next = Math.min(MAX_THREAD_LIST_WIDTH, Math.max(MIN_THREAD_LIST_WIDTH, raw))
    settings$.threadListWidth.set(Math.round(next))
  })
}
