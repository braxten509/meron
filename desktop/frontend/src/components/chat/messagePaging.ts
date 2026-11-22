// Keyboard scrolling of the open message: Space / Shift+Space page it, and the arrow keys step it while focus is in an HTML message's frame.
//
// The conversation and the reader tab mark their scroll containers with data-message-scroller (and
// make them focusable, so a click into the message hands the arrow keys to the browser's own
// scrolling). The conversation stays mounted, hidden, under a reader tab, so the one on screen is the
// target.

import { COMPOSER_OVERLAP_VAR } from './conversationScroll'

/** Dispatched on the scroller just before it is scrolled, so the conversation can
 *  drop a pinned message the way it does for the reader's own scrolling. */
export const MESSAGE_PAGE_EVENT = 'meron.messagePage'

/** Share of the visible height one page moves, leaving a few lines of overlap
 *  to keep the reader's place, as a browser's Space does. */
const PAGE_FRACTION = 0.85

/** One arrow-key step, about what a browser scrolls. */
const LINE_PX = 40

function visibleMessageScroller(): HTMLElement | null {
  for (const element of document.querySelectorAll<HTMLElement>('[data-message-scroller]')) {
    if (element.closest('[aria-hidden="true"]') || element.getClientRects().length === 0) continue
    return element
  }
  return null
}

/** Whether focus is in a message, where the arrow keys scroll it rather than
 *  step through the thread list. */
export function inMessageScroller(target: EventTarget | null): boolean {
  return !!(target as Element | null)?.closest?.('[data-message-scroller]')
}

/** Scroll the open message by a page. False when no message is on screen. */
export function pageOpenMessage(direction: 1 | -1): boolean {
  const element = visibleMessageScroller()
  if (!element) return false
  element.dispatchEvent(new Event(MESSAGE_PAGE_EVENT))
  // The quick-reply composer floats over the bottom of the conversation.
  const overlap = parseFloat(getComputedStyle(element).getPropertyValue(COMPOSER_OVERLAP_VAR)) || 0
  const page = Math.max(element.clientHeight - overlap, 0) * PAGE_FRACTION
  element.scrollBy({ top: direction * page, behavior: 'smooth' })
  return true
}

/** Scroll the open message by an arrow-key step. False when no message is on
 *  screen. Not smoothed, so a held key scrolls evenly. */
export function stepOpenMessage(direction: 1 | -1): boolean {
  const element = visibleMessageScroller()
  if (!element) return false
  element.dispatchEvent(new Event(MESSAGE_PAGE_EVENT))
  element.scrollBy({ top: direction * LINE_PX })
  return true
}
