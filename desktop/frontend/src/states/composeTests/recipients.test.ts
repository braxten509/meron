// Reply-all recipients and the recipient line shown to the sender.
// Reply-all cases cover fallback to the sender and which recipients appear in the editor.
// The quick-reply address line follows the currently open conversation.
import { beforeEach, describe, expect, it } from 'bun:test'
import type { Message } from '../../types'
import { openMessageTab, revealMessageRemote, replyAllToMessage } from '../compose'
import { buildReplyRecipients, quickReplyRecipients, replyAllAddsRecipients } from '../composeReply'
import { compose$ } from '../composeState'
import { accounts$ } from '../accounts'
import { ui$ } from '../ui'
import { mail$ } from '../mail'
import { resetThreadView, thread$ } from '../thread'
import { message } from './message'

describe('revealMessageRemote', () => {
  beforeEach(() => {
    compose$.tabs.set([])
    compose$.activeTab.set('')
    resetThreadView()
  })

  it('refreshes a pending reader body and enables printing when it arrives', () => {
    openMessageTab(message({ id: 'pending-reader', body: '', body_missing: true }))
    mail$.messages.set([
      message({ id: 'pending-reader', body: 'Downloaded body', body_html: '<p>Downloaded body</p>' }),
    ])
    const tab = compose$.tabs.get().find((tab) => tab.messageId === 'pending-reader')
    expect(tab?.bodyMissing).toBe(false)
    expect(tab?.body).toBe('Downloaded body')
    expect(tab?.bodyHtml).toBe('<p>Downloaded body</p>')
  })

  it("keeps a reader tab's read and star flags in step with the conversation", () => {
    openMessageTab(message({ id: 'flagged-reader', unread: false, starred: false }))
    mail$.messages.set([message({ id: 'flagged-reader', unread: true, starred: true })])
    // The conversation is left: the tab keeps what it last saw there.
    mail$.messages.set([])
    const tab = compose$.tabs.get().find((tab) => tab.messageId === 'flagged-reader')
    expect(tab?.message?.unread).toBe(true)
    expect(tab?.message?.starred).toBe(true)
  })

  it('marks an open reader tab so a thread switch cannot re-block it', () => {
    openMessageTab(message({ id: 'm1', body_html: '<p>hi</p>' }))
    openMessageTab(message({ id: 'm2', body_html: '<p>other</p>' }))
    // Reading the conversation, not the tab: the inactive tab is unmounted, so
    // it cannot copy the reveal itself.
    compose$.activeTab.set('')

    revealMessageRemote('m1')
    expect(thread$.revealedRemote.get().m1).toBe(true)

    // The thread switch clears the conversation's reveal map; the tab keeps its
    // own copy, and the message that was never revealed keeps none.
    resetThreadView()
    const tabs = compose$.tabs.get()
    expect(tabs.find((tab) => tab.messageId === 'm1')?.revealRemote).toBe(true)
    expect(tabs.find((tab) => tab.messageId === 'm2')?.revealRemote).toBe(false)
  })
})

describe('reply-all recipients', () => {
  // The rule itself lives in the core (meron-core/src/reply.rs) and is tested
  // there; what matters here is that the composer uses the recipients the core
  // sent rather than deriving its own.
  const replied = (overrides: Partial<Message['reply']> = {}): Message['reply'] => ({
    to: 'Them <them@example.com>',
    cc: 'bob@example.com',
    all_to: 'Them <them@example.com>',
    all_cc: 'Alice <alice@example.com>, bob@example.com',
    all_adds_recipients: true,
    ...overrides,
  })

  const incoming = message({
    from_name: 'Them',
    from_addr: 'them@example.com',
    to: 'me@example.com, Alice <alice@example.com>',
    cc: 'bob@example.com',
    reply: replied(),
  })

  it('addresses the sender and copies the rest for reply-all', () => {
    expect(buildReplyRecipients(incoming, true)).toEqual({
      to: 'Them <them@example.com>',
      cc: 'Alice <alice@example.com>, bob@example.com',
    })
  })

  it('takes the plain reply form by default', () => {
    expect(buildReplyRecipients(incoming)).toEqual({ to: 'Them <them@example.com>', cc: 'bob@example.com' })
  })

  it('falls back to the sender for a target with no recipients of its own', () => {
    // A thread card standing in for a thread whose messages have not loaded.
    const card = message({ from_name: 'Them', from_addr: 'them@example.com', to: '', reply: undefined })

    expect(buildReplyRecipients(card, true)).toEqual({ to: 'Them <them@example.com>', cc: '' })
  })

  it('offers reply-all only when the core says it reaches someone new', () => {
    expect(replyAllAddsRecipients(incoming)).toBe(true)
    expect(replyAllAddsRecipients(message({ reply: replied({ all_adds_recipients: false }) }))).toBe(false)
    expect(replyAllAddsRecipients(message({ reply: undefined }))).toBe(false)
  })

  it('seeds a composer tab addressed to everyone when replying all to one message', () => {
    accounts$.set([
      {
        id: 'acc-1',
        email: 'me@example.com',
        display_name: 'Me',
        sender_name: 'Me',
        provider: 'custom',
        auth_type: 'password',
        imap_host: 'imap.example.com',
        imap_port: 993,
        smtp_host: 'smtp.example.com',
        smtp_port: 465,
        tls: true,
      },
    ])
    compose$.tabs.set([])
    compose$.activeTab.set('')

    replyAllToMessage(
      message({
        account_id: 'acc-1',
        thread_id: 'reply-thread',
        from_name: 'Them',
        from_addr: 'them@example.com',
        to: 'me@example.com, Alice <alice@example.com>',
        cc: 'bob@example.com',
        subject: 'Design',
        message_id: 'root@example.com',
        reply: replied(),
      }),
    )

    expect(compose$.tabs.peek()[0].threadId).toBe('reply-thread')
    const draft = compose$.tabs.get()[0]?.compose
    expect(draft?.to).toBe('Them <them@example.com>')
    expect(draft?.cc).toBe('Alice <alice@example.com>, bob@example.com')
    expect(draft?.showCcBcc).toBe(true)
    expect(draft?.subject).toBe('Re: Design')
    expect(draft?.inReplyTo).toBe('root@example.com')
  })
})

describe('quick reply recipient line', () => {
  beforeEach(() => {
    mail$.threads.set([])
    mail$.messages.set([])
    ui$.selectedThread.set('')
    ui$.selectedAccount.set('acc-1')
    accounts$.set([
      {
        id: 'acc-1',
        email: 'me@example.com',
        display_name: 'Me',
        sender_name: 'Me',
        provider: 'custom',
        auth_type: 'password',
        imap_host: 'imap.example.com',
        imap_port: 993,
        smtp_host: 'smtp.example.com',
        smtp_port: 465,
        tls: true,
      },
    ])
  })

  it('reports the addresses the box would send to', () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_name: 'Them',
      from_addr: 'them@example.com',
      to: 'me@example.com, alice@example.com',
      cc: 'bob@example.com, me@example.com',
      message_id: 'root@example.com',
      reply: {
        to: 'Them <them@example.com>',
        cc: 'bob@example.com',
        all_to: 'Them <them@example.com>',
        all_cc: 'alice@example.com, bob@example.com',
        all_adds_recipients: true,
      },
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread])
    ui$.selectedThread.set('t-1')

    // A plain reply: alice is dropped, our own address stays out of the Cc.
    expect(quickReplyRecipients()).toEqual({ to: 'Them <them@example.com>', cc: 'bob@example.com' })
  })

  it('reports nothing without an open conversation, which hides the line', () => {
    expect(quickReplyRecipients()).toEqual({ to: '', cc: '' })
  })
})
