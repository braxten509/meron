// Folding a message's quoted tail — the conversation a reply pastes under its new text — behind a "•••" toggle, the way Gmail does.
//
// The core finds the quote: HTML bodies arrive with it marked by `data-meron-quote`, plain bodies with
// the UTF-16 offset it starts at (`body_quote_start`). Which bodies the user unfolded is kept for the
// session, so a bubble that scrolls out of view and back comes back the way it was left.

export const HTML_QUOTE_SELECTOR = '[data-meron-quote]'
export const QUOTE_FOLDED_CLASS = 'meron-quote-folded'
export const QUOTE_TOGGLE_CLASS = 'meron-quote-toggle'

const unfolded = new Set<string>()

export function isQuoteUnfolded(key: string): boolean {
  return unfolded.has(key)
}

export function setQuoteUnfolded(key: string, open: boolean) {
  if (open) unfolded.add(key)
  else unfolded.delete(key)
}

/** Split a plain body at the core's quote offset; `quote` is empty when there
 *  is nothing to fold. */
export function splitQuotedBody(body: string, quoteStart: number | null | undefined): { reply: string; quote: string } {
  if (!quoteStart || quoteStart <= 0 || quoteStart >= body.length) return { reply: body, quote: '' }
  return { reply: body.slice(0, quoteStart), quote: body.slice(quoteStart) }
}

export type QuoteToggleLabels = { show: string; hide: string }

/**
 * Fold the marked quote of a frame document behind a toggle placed where the
 * quote starts. Returns the function that folds or unfolds it, or null when the
 * document has no quote. The toggle's dots are drawn by the frame stylesheet
 * rather than written as text, so the in-thread search never matches them.
 */
export function installFrameQuoteFold(
  doc: Document,
  key: string,
  labels: QuoteToggleLabels,
): ((open: boolean) => void) | null {
  const first = doc.querySelector(HTML_QUOTE_SELECTOR)
  if (!first?.parentNode) return null

  // A document wired again gets a fresh toggle rather than a second listener.
  doc.querySelector(`button.${QUOTE_TOGGLE_CLASS}`)?.remove()
  const toggle = doc.createElement('button')
  toggle.type = 'button'
  toggle.className = QUOTE_TOGGLE_CLASS
  first.parentNode.insertBefore(toggle, first)

  const apply = (open: boolean) => {
    setQuoteUnfolded(key, open)
    doc.documentElement.classList.toggle(QUOTE_FOLDED_CLASS, !open)
    const label = open ? labels.hide : labels.show
    toggle.title = label
    toggle.setAttribute('aria-label', label)
    toggle.setAttribute('aria-expanded', String(open))
  }
  toggle.addEventListener('click', (event) => {
    event.preventDefault()
    event.stopPropagation()
    apply(!isQuoteUnfolded(key))
  })
  apply(isQuoteUnfolded(key))
  return apply
}

/** Whether `element` is hidden inside its frame's folded quote. */
export function isInFoldedQuote(element: Element): boolean {
  return (
    element.ownerDocument.documentElement.classList.contains(QUOTE_FOLDED_CLASS) &&
    !!element.closest(HTML_QUOTE_SELECTOR)
  )
}
