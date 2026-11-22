// Restoring a saved quick reply and suppressing drafts already being sent.
// Refreshing the conversation can bring an old draft back from the server.
// The send guard must survive navigation until the pending reply settles.
import { describe, expect, it, jest } from 'bun:test'
import { compose$ } from '../composeState'
import { saveQuickReplyDraft } from '../quickReply'
import { sendReply } from '../quickReplySend'
import { settings$ } from '../settings'
import { ui$ } from '../ui'
import { mail$ } from '../mail'
import { message } from './message'
import { setupDraftFixture } from './draftFixture'

describe('quick reply draft sharing', () => {
  const calls = setupDraftFixture()

  it('does not rehydrate the consumed draft when the thread refreshes during send', async () => {
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
      from_addr: 'me@example.com',
      message_id: 'reply-draft@example.com',
      body: 'Hello there',
      date: 2000,
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread, draft])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Hello there')
    compose$.quickReplyDraftId.set('reply-draft@example.com')
    compose$.quickReplyDraftSaved.set(true)
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') return { message_id: 'sent@example.com' }
      if (command === 'mail.send') {
        // A mail.synced refresh can land before sendReply gets to discard the
        // server draft, temporarily making that draft the conversation tail.
        mail$.messages.set([thread, draft])
      }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    await sendReply()

    expect(compose$.composer.get()).toBe('')
    expect(compose$.quickReplyDraftId.get()).toBe('')
    expect(compose$.quickReplyDraftSaved.get()).toBe(false)
  })

  it('discards a matching loaded draft when its quick-reply ownership was lost', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
      has_draft: true,
    })
    const draft = message({
      id: 'draft-row',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      thread_id: 't-1',
      from_addr: 'me@example.com',
      message_id: 'reply-draft@example.com',
      body: 'Reply that was sent',
      date: 2000,
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread, draft])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Reply that was sent')
    compose$.quickReplyDraftId.set('')
    compose$.quickReplyDraftSaved.set(false)
    compose$.quickReplyDraftThreadId.set('')
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    await sendReply()
    await new Promise((resolve) => setTimeout(resolve, 0))

    expect(calls.find((call) => call.command === 'mail.discardDraft')?.payload).toMatchObject({
      account_id: 'acc-1',
      draft_id: 'reply-draft@example.com',
      thread_id: 't-1',
    })
    expect(mail$.messages.get().some((item) => item.id === 'draft-row')).toBe(false)
    expect(mail$.threads.get().find((item) => item.thread_id === 't-1')?.has_draft).toBe(false)
  })

  it('keeps the consumed draft guarded while the post-send discard is still in flight', async () => {
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

    let releaseDiscard!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') return { message_id: 'sent@example.com' }
      if (command === 'mail.discardDraft') {
        await new Promise<void>((resolve) => (releaseDiscard = resolve))
      }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    await sendReply()

    // SMTP has returned, so the pending payload is gone and a refresh can
    // already have swapped the optimistic bubble for the canonical Sent copy —
    // while the server draft this reply consumed is still there, and still the
    // conversation tail. The guard has to outlive its bubble until the discard
    // resolves, or the just-sent text lands back in the cleared composer.
    const sentCopy = message({
      id: 'sent-row',
      account_id: 'acc-1',
      folder_id: 'Sent',
      thread_id: 't-1',
      message_id: 'sent@example.com',
      date: 1500,
    })
    mail$.messages.set([thread, sentCopy, draft])

    expect(compose$.composer.get()).toBe('')
    expect(compose$.quickReplyDraftId.get()).toBe('')

    releaseDiscard()
    await new Promise((resolve) => setTimeout(resolve, 0))

    expect(mail$.messages.get().some((message) => message.id === 'draft-row')).toBe(false)
  })

  it("keeps a sent reply's draft suppressed and retries when its discard fails", async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    const other = message({ id: 'other', account_id: 'acc-1', thread_id: 't-2', message_id: 'other@example.com' })
    const draft = message({
      id: 'draft-row',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      thread_id: 't-1',
      message_id: 'reply-draft@example.com',
      body: 'Consumed reply',
      date: 2000,
    })
    mail$.threads.set([thread, other])
    mail$.messages.set([thread, draft])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Consumed reply')
    compose$.quickReplyDraftId.set('reply-draft@example.com')
    compose$.quickReplyDraftSaved.set(true)
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    let discardFails = true
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') return { message_id: 'sent@example.com' }
      if (command === 'mail.discardDraft' && discardFails) throw new Error('offline')
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }
    const flush = async () => {
      for (let i = 0; i < 50; i++) await Promise.resolve()
    }

    jest.useFakeTimers()
    try {
      await sendReply()
      await flush()

      // The message went out, so the copy left behind is stale rather than a
      // safety net. Coming back to the thread before the Sent copy is cached
      // leaves the draft as its tail — it must not hydrate the box with text
      // that was already sent.
      ui$.selectedThread.set('t-2')
      ui$.selectedThread.set('t-1')
      mail$.messages.set([thread, draft])
      expect(compose$.composer.get()).toBe('')
      expect(compose$.sendingDraftIds.get()).toContain('reply-draft@example.com')

      discardFails = false
      jest.advanceTimersByTime(5_000)
      await flush()

      expect(calls.filter((call) => call.command === 'mail.discardDraft')).toHaveLength(2)
      expect(compose$.sendingDraftIds.get()).not.toContain('reply-draft@example.com')
      expect(mail$.messages.get().some((message) => message.id === 'draft-row')).toBe(false)
    } finally {
      jest.useRealTimers()
    }
  })

  it('keeps the id a failed autosave was writing under', async () => {
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

    let saveFails = true
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') return { message_id: 'draft-core@example.com' }
      // The host gave up waiting; the core may still have written the draft.
      if (command === 'mail.saveDraft' && saveFails) throw new Error('sidecar save_draft timeout')
      return {}
    }

    await saveQuickReplyDraft()
    expect(compose$.quickReplyDraftId.get()).toBe('draft-core@example.com')
    // Kept for reuse, but not known to be on the server.
    expect(compose$.quickReplyDraftSaved.get()).toBe(false)

    saveFails = false
    compose$.composer.set('Hello there, again')
    await saveQuickReplyDraft()

    const saves = calls.filter((call) => call.command === 'mail.saveDraft')
    expect(calls.filter((call) => call.command === 'mail.allocateIdentity')).toHaveLength(1)
    expect((saves[1].payload as { draft_id: string }).draft_id).toBe('draft-core@example.com')
    expect(compose$.quickReplyDraftSaved.get()).toBe(true)
  })

  it('allows a different draft to hydrate after the sent draft cleanup settles', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    const sentDraft = message({
      id: 'sent-draft-row',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      to: 'them@example.com',
      subject: 'Re: Notification thread',
      outgoing: true,
      thread_id: 't-1',
      message_id: 'sent-draft@example.com',
      body: 'First reply',
      date: 2000,
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread, sentDraft])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('First reply')
    compose$.quickReplyDraftId.set('sent-draft@example.com')
    compose$.quickReplyDraftSaved.set(true)
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    await sendReply()
    await new Promise((resolve) => setTimeout(resolve, 0))

    const newerDraft = message({
      id: 'new-draft-row',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      to: 'them@example.com',
      subject: 'Re: Notification thread',
      outgoing: true,
      thread_id: 't-1',
      message_id: 'different-draft@example.com',
      body: 'Written elsewhere',
      date: 3000,
    })
    mail$.messages.set([thread, newerDraft])

    expect(compose$.composer.get()).toBe('Written elsewhere')
    expect(compose$.quickReplyDraftId.get()).toBe('different-draft@example.com')
  })

  it('keeps the consumed draft guarded across a navigation round trip during send', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    const other = message({ id: 'other', account_id: 'acc-1', thread_id: 't-2', message_id: 'other@example.com' })
    const draft = message({
      id: 'draft-row',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      thread_id: 't-1',
      message_id: 'reply-draft@example.com',
      body: 'Consumed reply',
      date: 2000,
    })
    mail$.threads.set([thread, other])
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
    ui$.selectedThread.set('t-2')
    ui$.selectedThread.set('t-1')
    mail$.messages.set([thread, draft])

    expect(compose$.composer.get()).toBe('')
    expect(compose$.quickReplyDraftSaved.get()).toBe(false)

    releaseSend()
    await send
  })
})
