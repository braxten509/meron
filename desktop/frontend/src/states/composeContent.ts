// Formats message content for replies, forwarded messages and restored drafts.
// The composer uses the same escaping and quotation rules in its text and HTML views.
// A folded draft can recover its quoted section without changing the source message.

import type { ComposeDraft, Message } from '../types'
import { htmlToText } from '../lib/html'
import { type ComposeBody } from '../lib/signature'
import { settings$ } from './settings'
import { formatFullTimestamp } from '../components/chat/messageHelpers'

// Reader and compose tabs: opening threads and drafts, the full editor, forwarding,
// and saving or sending composed messages.

export function escapeHtml(value: string): string {
  return value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

export function textToHtml(text: string): string {
  if (!text.trim()) return ''
  return text
    .split(/\n{2,}/)
    .map((para) => `<p>${para.split('\n').map(escapeHtml).join('<br>')}</p>`)
    .join('')
}

function headerLine(label: string, value?: string): string {
  const trimmed = value?.trim()
  return trimmed ? `${label}: ${trimmed}` : ''
}

function formatMessageFrom(message: Message): string {
  return message.from_name ? `${message.from_name} <${message.from_addr}>` : message.from_addr
}

export function forwardedSubject(subject: string): string {
  const trimmed = subject.trim()
  if (!trimmed) return 'Fwd: (no subject)'
  return /^fwd?:/i.test(trimmed) ? trimmed : `Fwd: ${trimmed}`
}

export function forwardedPlainBody(message: Message): string {
  const header = [
    '---------- Forwarded message ---------',
    headerLine('From', formatMessageFrom(message)),
    headerLine('Date', formatFullTimestamp(message.date)),
    headerLine('Subject', message.subject || '(no subject)'),
    headerLine('To', message.to),
    headerLine('Cc', message.cc),
  ].filter(Boolean)
  return `\n\n${header.join('\n')}\n\n${message.body ?? ''}`
}

export function forwardedHtmlBody(message: Message): string {
  const rows: Array<[string, string | undefined]> = [
    ['From', formatMessageFrom(message)],
    ['Date', formatFullTimestamp(message.date)],
    ['Subject', message.subject || '(no subject)'],
    ['To', message.to],
    ['Cc', message.cc],
  ]
  const presentRows = rows.filter((row): row is [string, string] => !!row[1]?.trim())
  const header = presentRows
    .map(([label, value]) => `<div><strong>${escapeHtml(label)}:</strong> ${escapeHtml(value)}</div>`)
    .join('')
  const body = message.body_html || textToHtml(message.body ?? '')
  return `<p><br></p><div class="meron-forwarded-message"><p>---------- Forwarded message ---------</p>${header}<br>${body}</div>`
}

/**
 * Build the plain-text and HTML quotation for a reply.
 *
 * The attribution precedes the original text. HTML uses a cite blockquote that the reader can fold.
 * The rich flag records whether the original contained HTML. Return undefined if quoting is
 * disabled or the original body has not been loaded.
 */
export function replyQuote(message: Message): ComposeBody | undefined {
  if (!settings$.quoteReplies.peek()) return undefined
  const body = message.body ?? ''
  if (!message.body_html && !body.trim()) return undefined
  const attribution = `On ${formatFullTimestamp(message.date)}, ${formatMessageFrom(message)} wrote:`
  const quoted = body
    .replace(/\s+$/, '')
    .split('\n')
    .map((line) => (line ? `> ${line}` : '>'))
    .join('\n')
  return {
    rich: !!message.body_html,
    html: `<p>${escapeHtml(attribution)}</p><blockquote type="cite">${message.body_html || textToHtml(body)}</blockquote>`,
    text: `${attribution}\n${quoted}`,
  }
}

/** Whether a body already quotes the message `quote` answers — a saved quoted
 * reply carried back into a reply — by the attribution line it opens with. */
export function bodyHasReplyQuote(body: string, quote: ComposeBody): boolean {
  const attribution = quote.text.split('\n', 1)[0]
  return !!attribution && body.includes(attribution)
}

/** What a draft sends and saves: its body, with a folded quote under it. */
export function composedContent(draft: Pick<ComposeDraft, 'rich' | 'html' | 'text' | 'quote'>): string {
  const own = draft.rich ? draft.html : draft.text
  if (!draft.quote) return own
  if (draft.rich) return `${own}${draft.quote.html}`
  return own.trim() ? `${own.replace(/\s+$/, '')}\n\n${draft.quote.text}` : draft.quote.text
}

/**
 * A reopened draft's body with the quote at its tail folded away again, as it
 * was while the reply was being written. The core marks that tail when it reads
 * the draft back: `data-meron-quote` on the HTML elements, `body_quote_start`
 * in the plain body. A draft without one comes back whole.
 */
export function foldDraftQuote(message: Message): ComposeBody & { quote?: ComposeDraft['quote'] } {
  if (message.body_html) {
    const doc = new DOMParser().parseFromString(message.body_html, 'text/html')
    // The core marks only a quote that ends the message, so everything from the
    // first marked element to the end of the document is the quote — at the top
    // level, inside a wrapper the reply shares with it, or (Outlook) running on
    // past that wrapper. The range splits whatever wrappers it crosses.
    const first = doc.body.querySelector('[data-meron-quote]')
    if (!first) return { rich: true, html: message.body_html, text: '' }
    const from = (node: Node) => {
      const siblings: Node[] = []
      for (let at: Node | null = node; at; at = at.nextSibling) siblings.push(at)
      return siblings
    }
    let tail = from(first)
    for (let wrapper = first.parentNode; wrapper && wrapper !== doc.body; wrapper = wrapper.parentNode) {
      const following = wrapper.nextSibling ? from(wrapper.nextSibling) : []
      const part = wrapper.cloneNode(false)
      for (const node of tail) part.appendChild(node)
      tail = [part, ...following]
    }
    const folded = doc.createElement('div')
    for (const node of tail) folded.appendChild(node)
    for (const element of Array.from(folded.querySelectorAll('[data-meron-quote]'))) {
      element.removeAttribute('data-meron-quote')
    }
    const html = folded.innerHTML.trim()
    return { rich: true, html: doc.body.innerHTML, text: '', quote: { html, text: htmlToText(html) } }
  }
  const body = message.body ?? ''
  const start = message.body_quote_start
  if (start == null || start <= 0 || start >= body.length) return { rich: false, html: '', text: body }
  const text = body.slice(start)
  return {
    rich: false,
    html: '',
    text: body.slice(0, start).replace(/\s+$/, ''),
    quote: { html: `<blockquote type="cite">${textToHtml(text)}</blockquote>`, text },
  }
}
