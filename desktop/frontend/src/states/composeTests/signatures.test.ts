import { beforeEach, describe, expect, it } from 'bun:test'
import type { Message } from '../../types'
import { openReplyInFullEditor, openComposeTab, updateComposeDraft } from '../compose'
import { compose$ } from '../composeState'
import { isQuickReplyBlank, quickReplyCaretOffset, saveQuickReplyDraft, seedQuickReplySignature } from '../quickReply'
import { accounts$ } from '../accounts'
import { settings$ } from '../settings'
import { ui$ } from '../ui'
import { mail$ } from '../mail'
import { message } from './message'

describe('signatures in a new compose tab', () => {
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

  beforeEach(() => {
    compose$.tabs.set([])
    compose$.activeTab.set('')
    ui$.selectedAccount.set('acc-1')
    accounts$.set([sendable])
    settings$.signature.set('')
  })

  const draftOf = (id: string | undefined) => compose$.tabs.get().find((tab) => tab.id === id)?.compose

  it('seeds a rich draft with the app-wide signature', () => {
    settings$.signature.set('<p>Ping</p>')

    expect(draftOf(openComposeTab())?.html).toBe('<p></p><p>Ping</p>')
  })

  it('lets an account override or opt out', () => {
    settings$.signature.set('<p>Ping</p>')
    accounts$.set([{ ...sendable, signature: { mode: 'custom', html: '<p>Mine</p>' } }])
    expect(draftOf(openComposeTab())?.html).toBe('<p></p><p>Mine</p>')

    accounts$.set([{ ...sendable, signature: { mode: 'none', html: '<p>Mine</p>' } }])
    expect(draftOf(openComposeTab())?.html).toBe('')
  })

  it('skips the signature when re-opening an existing message', () => {
    settings$.signature.set('<p>Ping</p>')

    expect(draftOf(openComposeTab({ noSignature: true }))?.html).toBe('')
  })
})

describe('signature on a change of From account', () => {
  const account = (id: string, signature?: { mode: 'global' | 'none' | 'custom'; html: string }) => ({
    id,
    email: `${id}@example.com`,
    display_name: id,
    provider: 'custom',
    auth_type: 'password' as const,
    imap_host: 'imap.example.com',
    imap_port: 993,
    smtp_host: 'smtp.example.com',
    smtp_port: 465,
    tls: true,
    signature,
  })

  beforeEach(() => {
    compose$.tabs.set([])
    compose$.activeTab.set('')
    ui$.selectedAccount.set('a')
    settings$.signature.set('')
    accounts$.set([
      account('a', { mode: 'custom', html: '<p>A</p>' }),
      account('b', { mode: 'custom', html: '<p>B</p>' }),
      account('c', { mode: 'none', html: '' }),
    ])
  })

  const draftOf = (id: string | undefined) => compose$.tabs.get().find((tab) => tab.id === id)?.compose

  it("replaces the old account's signature with the new one's", () => {
    const id = openComposeTab()!
    expect(draftOf(id)?.html).toBe('<p></p><p>A</p>')

    updateComposeDraft(id, { accountId: 'b', fromEmail: 'b@example.com' })

    expect(draftOf(id)?.html).toBe('<p></p><p>B</p>')
    expect(draftOf(id)?.signature?.html).toBe('<p>B</p>')
  })

  it('drops it when the new account sends none', () => {
    const id = openComposeTab()!
    updateComposeDraft(id, { accountId: 'c' })

    // The blank line the signature came with goes too, so switching accounts
    // repeatedly cannot pile up empty paragraphs.
    expect(draftOf(id)?.html).toBe('')
    // Not "unmanaged": the app knows this body has no signature and where one
    // would go, so moving to an account that has one puts it back there.
    expect(draftOf(id)?.signature).toEqual({ html: '', text: '', placement: 'belowText' })

    updateComposeDraft(id, { accountId: 'b' })
    expect(draftOf(id)?.html).toBe('<p></p><p>B</p>')
  })

  it('follows the draft through a switch to plaintext', () => {
    const id = openComposeTab()!
    // What the composer's rich/plain toggle does: the body becomes text, and
    // the tracked signature has to be findable in that form too.
    updateComposeDraft(id, { rich: false, html: '', text: '\n\nA' })
    updateComposeDraft(id, { accountId: 'b' })

    expect(draftOf(id)?.text).toBe('\n\nB')
  })

  it('leaves a reopened draft body alone, rather than giving it two signatures', () => {
    // "Edit as New Message" and saved drafts come with a body this app did not
    // compose; it may already end in a signature.
    const id = openComposeTab({ rich: true, html: '<p>hello</p><p>A</p>', noSignature: true })!
    expect(draftOf(id)?.signature).toBeUndefined()

    updateComposeDraft(id, { accountId: 'b' })

    expect(draftOf(id)?.html).toBe('<p>hello</p><p>A</p>')
    expect(draftOf(id)?.signature).toBeUndefined()
  })

  it('leaves the body alone once the signature has been edited', () => {
    const id = openComposeTab()!
    updateComposeDraft(id, { html: '<p></p><p>A, but mine now</p>' })
    updateComposeDraft(id, { accountId: 'b' })

    expect(draftOf(id)?.html).toBe('<p></p><p>A, but mine now</p>')
  })

  it('does not touch the body when the account is unchanged', () => {
    const id = openComposeTab()!
    updateComposeDraft(id, { accountId: 'a', fromEmail: 'alias@example.com' })

    expect(draftOf(id)?.html).toBe('<p></p><p>A</p>')
  })
})

describe('signature in the quick reply', () => {
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

  const setUpThread = () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread])
    ui$.selectedThread.set('t-1')
    return thread
  }

  beforeEach(() => {
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
    settings$.signature.set('<p>from u2</p>')
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: unknown) => {
            calls.push({ command, payload })
            if (command === 'mail.allocateIdentity') return { message_id: 'draft-core@example.com' }
            return {}
          },
        },
      },
    }
  })

  it('seeds the box with the signature, under a blank line to type in', () => {
    setUpThread()
    seedQuickReplySignature()

    expect(compose$.composer.get()).toBe('\n\nfrom u2')
    expect(compose$.quickReplySignature.get()?.text).toBe('from u2')
  })

  it('honours a per-account override', () => {
    accounts$.set([{ ...sendable, signature: { mode: 'none', html: '' } }])
    setUpThread()
    seedQuickReplySignature()

    expect(compose$.composer.get()).toBe('')
    expect(compose$.quickReplySignature.get()).toBeNull()
  })

  it('counts a box holding only the signature as blank, and does not save a draft for it', async () => {
    setUpThread()
    seedQuickReplySignature()

    expect(isQuickReplyBlank()).toBe(true)
    await saveQuickReplyDraft()
    expect(calls.filter((c) => c.command === 'mail.saveDraft')).toHaveLength(0)
  })

  it('counts typing above the signature as content', async () => {
    setUpThread()
    seedQuickReplySignature()
    compose$.composer.set('t2\n\nfrom u2')

    expect(isQuickReplyBlank()).toBe(false)
    await saveQuickReplyDraft()

    const saved = calls.find((c) => c.command === 'mail.saveDraft')?.payload as { body?: string }
    expect(saved.body).toBe('t2\n\nfrom u2')
  })

  it("sends the user's own whitespace exactly as written", async () => {
    setUpThread()
    seedQuickReplySignature()
    compose$.composer.set('    indented start\n\nand a trailing blank line\n\n\nfrom u2')

    await saveQuickReplyDraft()

    const saved = calls.find((c) => c.command === 'mail.saveDraft')?.payload as { body?: string }
    expect(saved.body).toBe('    indented start\n\nand a trailing blank line\n\n\nfrom u2')
  })

  it('drops the seeded blank line when nothing was typed above it', async () => {
    setUpThread()
    seedQuickReplySignature()
    // Attachment-only: the body is the untouched signature, which must not go
    // out with the leading newlines it was seeded under.
    compose$.composerAttachments.set([{ id: 'a1', filename: 'x.png', mime: 'image/png', size: 1, data: 'AA==' }])

    await saveQuickReplyDraft()

    const saved = calls.find((c) => c.command === 'mail.saveDraft')?.payload as { body?: string }
    expect(saved.body).toBe('from u2')
  })

  it('puts the caret at the end of the user text, above the signature', () => {
    setUpThread()
    seedQuickReplySignature()
    expect(quickReplyCaretOffset()).toBe(0)

    compose$.composer.set('t2\n\nfrom u2')
    expect(quickReplyCaretOffset()).toBe(2)
  })

  it('leaves a hydrated draft body alone', () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
    })
    mail$.folders.set([{ id: 'Drafts', account_id: 'acc-1', name: 'Drafts', kind: 'drafts' } as any])
    mail$.foldersByAccount.set({ 'acc-1': [{ id: 'Drafts', name: 'Drafts', kind: 'drafts' } as any] })
    mail$.threads.set([thread])
    ui$.selectedThread.set('t-1')
    mail$.messages.set([
      thread,
      message({
        id: 'd1',
        account_id: 'acc-1',
        thread_id: 't-1',
        folder_id: 'Drafts',
        to: 'them@example.com',
        subject: 'Re: Notification thread',
        outgoing: true,
        message_id: 'draft@example.com',
        body: 'half typed\n\nfrom u2',
        date: thread.date + 60,
      }),
    ])

    // The saved body already carries its own signature: nothing is appended,
    // and none of it is discounted as "not content".
    expect(compose$.composer.get()).toBe('half typed\n\nfrom u2')
    expect(compose$.quickReplySignature.get()).toBeNull()
    expect(isQuickReplyBlank()).toBe(false)
  })

  it('hands the full editor a stripped body so it inserts and tracks its own', () => {
    setUpThread()
    seedQuickReplySignature()
    compose$.composer.set('t2\n\nfrom u2')

    openReplyInFullEditor()

    const draft = compose$.tabs.get()[0].compose
    expect(draft?.text).toBe('t2\n\nfrom u2')
    expect(draft?.signature).toEqual({ html: '<p>from u2</p>', text: 'from u2', placement: 'belowText' })
    // And the box behind the new tab is a fresh quick reply, not an empty one.
    expect(compose$.composer.get()).toBe('\n\nfrom u2')
  })

  it('does not give the full editor a second copy of an edited signature', () => {
    setUpThread()
    seedQuickReplySignature()
    compose$.composer.set('t2\n\nfrom u2, but edited')

    openReplyInFullEditor()

    const draft = compose$.tabs.get()[0].compose
    expect(draft?.text).toBe('t2\n\nfrom u2, but edited')
    expect(draft?.signature).toBeUndefined()
  })

  it('re-seeds an untouched box when the app signature arrives late', () => {
    settings$.signature.set('')
    setUpThread()
    seedQuickReplySignature()
    expect(compose$.composer.get()).toBe('')

    settings$.signature.set('<p>from u2</p>')
    expect(compose$.composer.get()).toBe('\n\nfrom u2')
  })

  it('does not re-seed over something the user typed', () => {
    setUpThread()
    seedQuickReplySignature()
    compose$.composer.set('t2\n\nfrom u2')

    settings$.signature.set('<p>different</p>')

    expect(compose$.composer.get()).toBe('t2\n\nfrom u2')
  })
})
