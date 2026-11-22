// The core reports a boundary between a message and quoted
// history. The fixtures check how the reader splits at that
// boundary and handles offsets with no content on one side.

import { afterEach, describe, expect, it } from 'bun:test'
import { cleanup, fireEvent, render } from '@testing-library/react'
// Initialize this side of the mail/compose cycle before the body pulls both
// modules in through its native/link helpers (see ConversationMessageList.test).
import '../../states/compose'
import type { Message } from '../../types'
import { MessageBubbleBody } from './MessageBubbleBody'
import { installFrameQuoteFold, isInFoldedQuote, splitQuotedBody } from './quoteFold'
import { messageMatchCount } from './threadSearchMatches'

const REPLY = 'See you Friday.\n\n'
const QUOTE = 'On Mon, Jane wrote:\n> Lunch on Friday?'
const BODY = REPLY + QUOTE

function message(body: string, quoteStart: number | null): Message {
  return {
    id: 'message-1',
    account_id: 'account-1',
    folder_id: 'inbox',
    thread_id: 'thread-1',
    from_name: 'Sender',
    from_addr: 'sender@example.com',
    to: 'me@example.com',
    subject: 'Subject',
    preview: body,
    body,
    body_quote_start: quoteStart,
    date: 0,
    unread: false,
    starred: false,
    has_attachments: false,
  }
}

function renderBody(body: string, quoteStart: number | null, query = '', activeSearchOffset = -1) {
  return render(
    <MessageBubbleBody
      message={message(body, quoteStart)}
      useHtmlBody={false}
      normalizedSearchQuery={query}
      activeSearchOffset={activeSearchOffset}
    />,
  )
}

afterEach(cleanup)

describe('splitQuotedBody', () => {
  it('splits at the offset and ignores offsets with nothing on one side', () => {
    expect(splitQuotedBody(BODY, REPLY.length)).toEqual({ reply: REPLY, quote: QUOTE })
    for (const start of [null, undefined, 0, BODY.length, BODY.length + 5]) {
      expect(splitQuotedBody(BODY, start)).toEqual({ reply: BODY, quote: '' })
    }
  })
})

describe('MessageBubbleBody quote folding', () => {
  it('folds the quoted tail behind a toggle', () => {
    // A body of its own, so the session's unfolded set starts clean for it.
    const body = `${REPLY}${QUOTE} (fold)`
    const { container, getByRole } = renderBody(body, REPLY.length)
    expect(container.textContent).toContain('See you Friday.')
    expect(container.textContent).not.toContain('Lunch on Friday?')

    const toggle = getByRole('button', { expanded: false })
    fireEvent.click(toggle)
    expect(container.textContent).toContain('Lunch on Friday?')
    expect(toggle.getAttribute('aria-expanded')).toBe('true')
  })

  it('renders a body without a quote as before', () => {
    const { container, queryByRole } = renderBody(BODY, null)
    expect(container.textContent).toContain('Lunch on Friday?')
    expect(queryByRole('button', { expanded: false })).toBeNull()
  })

  it('opens the quote when the search is parked on a match inside it', () => {
    const body = `Friday works.\n\n${QUOTE} (search)`
    const start = 'Friday works.\n\n'.length
    // Occurrence 0 is in the reply; occurrence 1 is in the quote.
    expect(renderBody(body, start, 'friday', 0).container.textContent).not.toContain('Lunch on')
    cleanup()
    const { container } = renderBody(body, start, 'friday', 1)
    expect(container.textContent).toContain('Lunch on')
    expect(container.querySelector('[data-search-active="true"]')?.textContent?.toLowerCase()).toBe('friday')
  })

  it('counts the same matches the unfolded body marks', () => {
    const body = `a **kitchen** reply\n\nOn Mon, Jane wrote:\n> - kitchen sink (count)`
    const start = body.indexOf('On Mon')
    const count = messageMatchCount(message(body, start), 'kitchen', false)
    expect(count).toBe(2)
    const { container } = renderBody(body, start, 'kitchen', count - 1)
    expect(container.querySelectorAll('mark').length).toBe(count)
  })
})

describe('installFrameQuoteFold', () => {
  function frameDoc(html: string) {
    return new DOMParser().parseFromString(html, 'text/html')
  }

  it('does nothing for a document without a marked quote', () => {
    expect(installFrameQuoteFold(frameDoc('<p>hi</p>'), 'no-quote', { show: 'Show', hide: 'Hide' })).toBeNull()
  })

  it('places one toggle before the quote and flips the folded class', () => {
    const doc = frameDoc('<p>Yes</p><div data-meron-quote=""><blockquote>Lunch?</blockquote></div>')
    const labels = { show: 'Show', hide: 'Hide' }
    installFrameQuoteFold(doc, 'frame-quote', labels)
    const fold = installFrameQuoteFold(doc, 'frame-quote', labels)
    const toggles = doc.querySelectorAll('button')
    expect(toggles.length).toBe(1)
    expect(toggles[0].nextElementSibling?.hasAttribute('data-meron-quote')).toBe(true)

    const quoted = doc.querySelector('blockquote')!
    expect(isInFoldedQuote(quoted)).toBe(true)
    expect(toggles[0].getAttribute('aria-label')).toBe('Show')

    toggles[0].click()
    expect(isInFoldedQuote(quoted)).toBe(false)
    expect(toggles[0].getAttribute('aria-label')).toBe('Hide')

    fold?.(false)
    expect(isInFoldedQuote(quoted)).toBe(true)
  })
})
