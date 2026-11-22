// Quoted message text and headers included in replies.
// Default replies leave the original message unquoted.
// Explicit quoting carries the original content through the HTML reply path.
import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import type { Message } from '../../types'
import { openDraftCompose, openReplyInFullEditor, replyToMessage, composedContent } from '../compose'
import { formatFullTimestamp } from '../../components/chat/messageHelpers'
import { compose$ } from '../composeState'
import { cancelQuickReplyDraftSave } from '../quickReply'
import { sendReply } from '../quickReplySend'
import { accounts$ } from '../accounts'
import { settings$ } from '../settings'
import { ui$ } from '../ui'
import { mail$ } from '../mail'
import { message } from './message'

describe('quoting the original in replies', () => {
  const calls: { command: string; payload: unknown }[] = []

  const sendable = {
    id: 'acc-1',
    email: 'me@example.com',
    display_name: 'Me',
    provider: 'custom',
    auth_type: 'password' as const,
    imap_host: 'imap.example.com',
    imap_port: 993,
    smtp_host: 'smtp.example.com',
    smtp_port: 465,
    tls: true,
  }

  const original = (overrides: Partial<Message> = {}) =>
    message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_name: 'Them',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      subject: 'Lunch',
      body: 'Lunch on Friday?\n\nJane',
      ...overrides,
    })
  const attribution = (message: Message) => `On ${formatFullTimestamp(message.date)}, Them <them@example.com> wrote:`
  const htmlAttribution = (message: Message) => attribution(message).replace('<', '&lt;').replace('>', '&gt;')
  const draft = () => compose$.tabs.get()[0]?.compose

  beforeEach(() => {
    cancelQuickReplyDraftSave()
    calls.length = 0
    compose$.tabs.set([])
    compose$.activeTab.set('')
    compose$.composer.set('')
    compose$.composerAttachments.set([])
    compose$.quickReplyDraftId.set('')
    compose$.quickReplyDraftSaved.set(false)
    compose$.quickReplyFrom.set('')
    compose$.quickReplySignature.set(null)
    mail$.messages.set([])
    mail$.threads.set([])
    mail$.folders.set([])
    mail$.foldersByAccount.set({})
    ui$.selectedThread.set('')
    ui$.selectedAccount.set('acc-1')
    accounts$.set([sendable])
    settings$.signature.set('')
    settings$.quoteReplies.set(true)
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: unknown) => {
            calls.push({ command, payload })
            if (command === 'mail.allocateIdentity') return { message_id: 'sent@example.com' }
            if (command === 'mail.folderList') return { folders: [] }
            if (command === 'mail.threadList') return { threads: [] }
            return {}
          },
        },
      },
    }
  })

  afterEach(() => {
    settings$.quoteReplies.set(false)
  })

  it('leaves replies unquoted by default', () => {
    settings$.quoteReplies.set(false)
    accounts$.set([{ ...sendable, conversation_html: false }])

    replyToMessage(original())

    expect(draft()?.text).toBe('')
  })

  it('folds a plain original away under the signature', () => {
    accounts$.set([{ ...sendable, conversation_html: false }])
    settings$.signature.set('<p>Ping</p>')
    const message = original()

    replyToMessage(message)

    expect(draft()?.rich).toBe(false)
    expect(draft()?.text).toBe('\n\nPing')
    expect(draft()?.quote?.text).toBe(`${attribution(message)}\n> Lunch on Friday?\n>\n> Jane`)
    expect(composedContent(draft()!)).toBe(`\n\nPing\n\n${attribution(message)}\n> Lunch on Friday?\n>\n> Jane`)
  })

  it('folds an HTML original away in a cite blockquote', () => {
    accounts$.set([{ ...sendable, conversation_html: false }])
    settings$.signature.set('<p>Ping</p>')
    const message = original({ body_html: '<p>Lunch on <b>Friday</b>?</p>' })

    replyToMessage(message)

    expect(draft()?.rich).toBe(true)
    expect(draft()?.html).toBe('<p></p><p>Ping</p>')
    expect(composedContent(draft()!)).toBe(
      `<p></p><p>Ping</p><p>${htmlAttribution(message)}</p><blockquote type="cite"><p>Lunch on <b>Friday</b>?</p></blockquote>`,
    )
  })

  it('folds the quote under the text carried from the quick reply into the full editor', () => {
    const message = original()
    mail$.threads.set([message])
    mail$.messages.set([message])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Sounds good')

    openReplyInFullEditor()

    expect(draft()?.text).toBe('Sounds good')
    expect(composedContent(draft()!)).toBe(`Sounds good\n\n${attribution(message)}\n> Lunch on Friday?\n>\n> Jane`)
  })

  it('does not quote a saved quoted reply carried back into a reply twice', async () => {
    const message = original()
    mail$.threads.set([message])
    mail$.messages.set([message])
    ui$.selectedThread.set('t-1')
    const saved = `Sounds good\n\n${attribution(message)}\n> Lunch on`
    compose$.composer.set(saved)

    openReplyInFullEditor()
    expect(draft()?.text).toBe(saved)
    expect(draft()?.quote).toBeUndefined()

    compose$.tabs.set([])
    compose$.composer.set(saved)
    await sendReply()
    const sent = calls.find((call) => call.command === 'mail.send')?.payload as { body: string }
    expect(sent.body).toBe(saved)
  })

  it('folds the quote of a reopened draft away again', async () => {
    const plain = message({
      id: 'acc-1#Drafts#1#1',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      thread_id: 'acc-1#Drafts#1',
      message_id: 'draft-plain@example.com',
      body: 'Sounds good\n\nOn Mon, Them wrote:\n> Lunch?',
      body_quote_start: 13,
    })
    const rich = message({
      ...plain,
      id: 'acc-1#Drafts#2#1',
      thread_id: 'acc-1#Drafts#2',
      message_id: 'draft-rich@example.com',
      body_html:
        '<p>Sounds good</p><p data-meron-quote="">On Mon, Them wrote:</p><blockquote type="cite" data-meron-quote=""><p>Lunch?</p></blockquote>',
    })
    ;(window as any).go.main.App.Invoke = async (command: string, payload: any) => {
      calls.push({ command, payload })
      if (command === 'mail.threadRead') return { messages: [payload.thread_id === plain.thread_id ? plain : rich] }
      return {}
    }

    await openDraftCompose(plain)
    expect(draft()?.text).toBe('Sounds good')
    expect(draft()?.quote?.text).toBe('On Mon, Them wrote:\n> Lunch?')

    compose$.tabs.set([])
    await openDraftCompose(rich)
    expect(draft()?.html).toBe('<p>Sounds good</p>')
    expect(draft()?.quote?.html).toBe('<p>On Mon, Them wrote:</p><blockquote type="cite"><p>Lunch?</p></blockquote>')

    // A quote that shares a wrapper with the reply.
    compose$.tabs.set([])
    rich.body_html = `<div>${rich.body_html}</div>`
    rich.message_id = 'draft-wrapped@example.com'
    await openDraftCompose(rich)
    expect(draft()?.html).toBe('<div><p>Sounds good</p></div>')
    expect(draft()?.quote?.html).toBe(
      '<div><p>On Mon, Them wrote:</p><blockquote type="cite"><p>Lunch?</p></blockquote></div>',
    )

    // An Outlook quote whose header sits in a wrapper and whose body runs past it.
    compose$.tabs.set([])
    rich.body_html =
      '<div><p>Sounds good</p><div id="divRplyFwdMsg" data-meron-quote="">From: Them</div></div><p data-meron-quote="">Lunch?</p>'
    rich.message_id = 'draft-outlook@example.com'
    await openDraftCompose(rich)
    expect(draft()?.html).toBe('<div><p>Sounds good</p></div>')
    expect(draft()?.quote?.html).toBe('<div><div id="divRplyFwdMsg">From: Them</div></div><p>Lunch?</p>')
  })

  it('sends the quote with a quick reply but keeps it out of the bubble', async () => {
    const m = original()
    mail$.threads.set([m])
    mail$.messages.set([m])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Sounds good')

    await sendReply()

    const sent = calls.find((call) => call.command === 'mail.send')?.payload as { body: string; html: string }
    expect(sent.body).toBe(`Sounds good\n\n${attribution(m)}\n> Lunch on Friday?\n>\n> Jane`)
    expect(sent.html).toBe('')
    expect(mail$.messages.get().find((message) => message.send_status)?.body).toBe('Sounds good')
  })

  it('sends an HTML quote with a quick reply to an HTML original', async () => {
    const message = original({ body_html: '<p>Lunch on <b>Friday</b>?</p>' })
    mail$.threads.set([message])
    mail$.messages.set([message])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Sounds good')

    await sendReply()

    const sent = calls.find((call) => call.command === 'mail.send')?.payload as { body: string; html: string }
    expect(sent.body).toBe(`Sounds good\n\n${attribution(message)}\n> Lunch on Friday?\n>\n> Jane`)
    expect(sent.html).toStartWith(`<p>Sounds good</p><p>${htmlAttribution(message)}</p><blockquote `)
    expect(sent.html).toContain('type="cite"')
    expect(sent.html).toContain('<p>Lunch on <b>Friday</b>?</p></blockquote>')
  })
})
