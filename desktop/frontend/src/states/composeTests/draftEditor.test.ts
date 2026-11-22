// Ownership of a saved draft while opening the full editor.
// Concurrent opens can still be reading the same draft.
// Escalating a quick reply passes ownership and its saved ID to the editor.
import { describe, expect, it } from 'bun:test'
import type { Message } from '../../types'
import { openDraftCompose, openDraftConversationOrCompose, openReplyInFullEditor } from '../compose'
import { compose$ } from '../composeState'
import { saveQuickReplyDraft, withoutHydratedQuickReplyDraft } from '../quickReply'
import { sendReply } from '../quickReplySend'
import { settings$ } from '../settings'
import { ui$ } from '../ui'
import { mail$ } from '../mail'
import { message } from './message'
import { setupDraftFixture } from './draftFixture'

describe('quick reply draft sharing', () => {
  const calls = setupDraftFixture()

  it('keeps the draft held while a second open is still reading', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    const draftRow = message({
      id: 'draft-row',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      thread_id: 't-9',
      message_id: 'claimed-draft@example.com',
      body: 'First reply',
      date: 2000,
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('First reply')
    compose$.quickReplyDraftId.set('claimed-draft@example.com')
    compose$.quickReplyDraftSaved.set(true)
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    let sendStarted!: () => void
    const sending = new Promise<void>((resolve) => (sendStarted = resolve))
    let releaseSend!: () => void
    let reads = 0
    let secondReadStarted!: () => void
    const secondReading = new Promise<void>((resolve) => (secondReadStarted = resolve))
    let releaseSecondRead!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') return { message_id: 'sent@example.com' }
      if (command === 'mail.send') {
        sendStarted()
        await new Promise<void>((resolve) => (releaseSend = resolve))
      }
      if (command === 'mail.threadRead') {
        reads += 1
        // The first click fails; the second is still reading.
        if (reads === 1) throw new Error('read failed')
        secondReadStarted()
        await new Promise<void>((resolve) => (releaseSecondRead = resolve))
        return { messages: [draftRow] }
      }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    const send = sendReply()
    await sending
    const firstOpen = openDraftCompose(draftRow)
    const secondOpen = openDraftCompose(draftRow)
    await secondReading
    await firstOpen

    releaseSend()
    await send
    for (let attempt = 0; attempt < 20 && compose$.sendingDraftIds.get().length > 0; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 0))
    }
    // The failed click must not have dropped the surviving one's hold.
    expect(calls.some((call) => call.command === 'mail.discardDraft')).toBe(false)

    releaseSecondRead()
    await secondOpen
  })

  it('holds a draft row whose Message-ID has not been loaded yet', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    // Synced from its envelope: no Message-ID until the body is fetched, so the
    // quick reply identified it — and the send claimed it — by row id.
    const draftRow = message({
      id: 'draft-row-uid-42',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      thread_id: 't-9',
      message_id: '',
      body: 'First reply',
      date: 2000,
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('First reply')
    compose$.quickReplyDraftId.set('draft-row-uid-42')
    compose$.quickReplyDraftSaved.set(true)
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    let sendStarted!: () => void
    const sending = new Promise<void>((resolve) => (sendStarted = resolve))
    let releaseSend!: () => void
    let readStarted!: () => void
    const reading = new Promise<void>((resolve) => (readStarted = resolve))
    let releaseRead!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') return { message_id: 'sent@example.com' }
      if (command === 'mail.send') {
        sendStarted()
        await new Promise<void>((resolve) => (releaseSend = resolve))
      }
      if (command === 'mail.threadRead') {
        readStarted()
        await new Promise<void>((resolve) => (releaseRead = resolve))
        return { messages: [draftRow] }
      }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    const send = sendReply()
    await sending
    const opening = openDraftCompose(draftRow)
    await reading

    releaseSend()
    await send
    for (let attempt = 0; attempt < 20 && compose$.sendingDraftIds.get().length > 0; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 0))
    }
    expect(calls.some((call) => call.command === 'mail.discardDraft')).toBe(false)

    releaseRead()
    await opening
  })

  it('hands a conversation-only open back to the quick reply', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-5',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    const draftRow = message({
      id: 'draft-row',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      to: 'them@example.com',
      subject: 'Re: Notification thread',
      outgoing: true,
      thread_id: 't-5',
      message_id: 'reopened-draft@example.com',
      body: 'Half-written answer',
      date: 2000,
    })
    mail$.threads.set([thread, draftRow])
    mail$.messages.set([])
    ui$.selectedThread.set('')
    compose$.composer.set('')
    compose$.quickReplyDraftId.set('')
    compose$.quickReplyDraftSaved.set(false)
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.threadRead') return { messages: [thread, draftRow] }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    // A reply-draft opens the conversation, not the full editor: the hold that
    // protected it during the read must not outlive the hydration that takes it.
    await openDraftConversationOrCompose(draftRow)

    expect(compose$.quickReplyDraftId.get()).toBe('reopened-draft@example.com')
    expect(compose$.composer.get()).toBe('Half-written answer')
  })

  it('does not take a draft row whose Message-ID has not been synced yet', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    // Synced from its envelope: no Message-ID, and no body fetched either.
    const envelopeOnly = message({
      id: 'draft-row-uid-42',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      to: 'them@example.com',
      subject: 'Re: Notification thread',
      outgoing: true,
      thread_id: 't-1',
      message_id: '',
      body: '',
      date: 2000,
    })
    mail$.threads.set([thread])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('')
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    mail$.messages.set([thread, envelopeOnly])

    // Taking it would put an empty box in front of the user calling itself their
    // draft, and give a send an id the server cannot address: a save would append
    // a second copy under it and cleanup could only ever delete one of the two.
    expect(compose$.quickReplyDraftId.get()).toBe('')
    expect(compose$.quickReplyDraftSaved.get()).toBe(false)
    expect(compose$.quickReplyDraftThreadId.get()).toBe('')

    // The thread read back-fills the header; then it hydrates as normal.
    mail$.messages.set([thread, { ...envelopeOnly, message_id: 'now-known@example.com', body: 'Half-written answer' }])
    expect(compose$.quickReplyDraftId.get()).toBe('now-known@example.com')
    expect(compose$.composer.get()).toBe('Half-written answer')
  })

  it('saves and discards a hydrated draft under the same Message-ID it was hydrated by', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    const draftRow = message({
      id: 'draft-row',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      to: 'them@example.com',
      subject: 'Re: Notification thread',
      outgoing: true,
      thread_id: 't-1',
      message_id: 'hydrated-draft@example.com',
      body: 'Half-written answer',
      date: 2000,
    })
    mail$.threads.set([thread])
    ui$.selectedThread.set('t-1')
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') return { message_id: 'sent@example.com' }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    mail$.messages.set([thread, draftRow])
    expect(compose$.quickReplyDraftId.get()).toBe('hydrated-draft@example.com')

    // Edit it, then send: the autosave replaces that same server draft and the
    // send discards the one id, leaving nothing behind.
    compose$.composer.set('Half-written answer, now finished')
    await saveQuickReplyDraft()
    await sendReply()
    for (let attempt = 0; attempt < 20 && compose$.sendingDraftIds.get().length > 0; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 0))
    }

    const saved = calls
      .filter((call) => call.command === 'mail.saveDraft')
      .map((call) => (call.payload as { draft_id?: string }).draft_id)
    const discarded = calls
      .filter((call) => call.command === 'mail.discardDraft')
      .map((call) => (call.payload as { draft_id?: string }).draft_id)
    expect(saved).toEqual(['hydrated-draft@example.com'])
    expect(discarded).toEqual(['hydrated-draft@example.com'])
  })

  it('does not overwrite a reply typed while the draft header was missing', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    const envelopeOnly = message({
      id: 'draft-row-uid-42',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      thread_id: 't-1',
      message_id: '',
      body: '',
      date: 2000,
    })
    mail$.threads.set([thread])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('')
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    mail$.messages.set([thread, envelopeOnly])
    expect(compose$.quickReplyDraftId.get()).toBe('')

    // The box was left free, so the user starts a reply in it.
    compose$.composer.set('A new reply, typed just now')

    // The background reread fills in that row's header and body.
    mail$.messages.set([thread, { ...envelopeOnly, message_id: 'now-known@example.com', body: 'The old saved draft' }])

    expect(compose$.composer.get()).toBe('A new reply, typed just now')
    expect(compose$.quickReplyDraftId.get()).toBe('')
  })

  it('hides the consumed draft from the conversation while the send is in flight', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    const draft = message({
      id: 'draft-row',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      thread_id: 't-1',
      message_id: 'reply-draft@example.com',
      body: 'Consumed reply',
      date: 2000,
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread, draft])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Consumed reply')
    compose$.quickReplyDraftId.set('reply-draft@example.com')
    compose$.quickReplyDraftSaved.set(true)
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    let sendStarted!: () => void
    const started = new Promise<void>((resolve) => (sendStarted = resolve))
    let releaseSend!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') return { message_id: 'sent@example.com' }
      if (command === 'mail.send') {
        sendStarted()
        await new Promise<void>((resolve) => (releaseSend = resolve))
      }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    const send = sendReply()
    await started
    // A background refresh re-delivers the still-cached draft after the quick
    // reply has let go of it.
    mail$.messages.set([thread, draft])
    expect(compose$.quickReplyDraftSaved.get()).toBe(false)
    expect(
      withoutHydratedQuickReplyDraft(
        [thread, draft],
        compose$.quickReplyDraftId.get(),
        compose$.quickReplyDraftSaved.get(),
        compose$.sendingDraftIds.get(),
      ),
    ).toEqual([thread])

    releaseSend()
    await send
  })

  it('keeps unrelated older drafts visible', () => {
    const olderDraft = message({ id: 'd0', folder_id: 'Drafts', message_id: 'draft-0@example.com' })
    const activeDraft = message({ id: 'd1', folder_id: 'Drafts', message_id: 'draft-1@example.com' })

    expect(withoutHydratedQuickReplyDraft([olderDraft, activeDraft], 'draft-1@example.com', true)).toEqual([olderDraft])
  })

  it('does not hydrate when the tail message is not a draft', () => {
    ui$.selectedThread.set('t-3')
    compose$.composer.set('unrelated text')
    const onlyMessage = message({ id: 'm1', thread_id: 't-3', folder_id: 'INBOX', date: 1000 })

    mail$.messages.set([onlyMessage])

    expect(compose$.composer.get()).toBe('unrelated text')
    expect(compose$.quickReplyDraftSaved.get()).toBe(false)
  })

  it('clears quick reply state when the active conversation is closed', () => {
    ui$.selectedThread.set('t-4')
    compose$.composer.set('abc')
    compose$.quickReplyDraftId.set('draft-1@example.com')
    compose$.quickReplyDraftSaved.set(true)

    ui$.selectedThread.set('')

    expect(compose$.composer.get()).toBe('')
    expect(compose$.quickReplyDraftId.get()).toBe('')
    expect(compose$.quickReplyDraftSaved.get()).toBe(false)
  })

  it('hands off the saved quick reply draft id when escalating to the full editor', async () => {
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
    compose$.composer.set('Hello there')
    await saveQuickReplyDraft()
    const draftId = compose$.quickReplyDraftId.get()

    openReplyInFullEditor()

    expect(compose$.tabs.get()).toHaveLength(1)
    expect(compose$.tabs.get()[0].compose?.draftMessageId).toBe(draftId)
    expect(compose$.quickReplyDraftId.get()).toBe('')
    expect(compose$.quickReplyDraftSaved.get()).toBe(false)
    expect(compose$.quickReplyDraftThreadId.get()).toBe('')
  })
})
