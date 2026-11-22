import { describe, expect, it } from 'bun:test'
import { openComposeTab } from '../compose'
import { compose$ } from '../composeState'
import {
  discardQuickReplyDraftIfEmpty,
  quickReplyDraftBelongsToThread,
  saveQuickReplyDraft,
  withoutHydratedQuickReplyDraft,
} from '../quickReply'
import { settings$ } from '../settings'
import { ui$ } from '../ui'
import { mail$ } from '../mail'
import { message } from './message'
import { setupDraftFixture } from './draftFixture'

describe('quick reply draft sharing', () => {
  const calls = setupDraftFixture()

  it('saves the quick reply as a server draft and reuses the same id across autosaves', async () => {
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

    expect(calls.filter((c) => c.command === 'mail.saveDraft')).toHaveLength(1)
    expect(calls.filter((c) => c.command === 'mail.allocateIdentity')).toHaveLength(1)
    expect(compose$.quickReplyDraftSaved.get()).toBe(true)
    const firstDraftId = compose$.quickReplyDraftId.get()
    expect(firstDraftId).toBe('draft-core@example.com')

    compose$.composer.set('Hello there, updated')
    await saveQuickReplyDraft()

    const saveCalls = calls.filter((c) => c.command === 'mail.saveDraft')
    expect(saveCalls).toHaveLength(2)
    expect(calls.filter((c) => c.command === 'mail.allocateIdentity')).toHaveLength(1)
    expect(compose$.quickReplyDraftId.get()).toBe(firstDraftId)
    expect((saveCalls[1].payload as { draft_id: string }).draft_id).toBe(firstDraftId)
  })

  it('marks a newly minted draft unsaved when the hydrated draft is claimed by an editor', async () => {
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
    compose$.composer.set('Updated reply')
    compose$.quickReplyDraftId.set('hydrated-draft@example.com')
    compose$.quickReplyDraftSaved.set(true)
    compose$.quickReplyDraftThreadId.set('t-1')
    openComposeTab({
      accountId: 'acc-1',
      draftMessageId: 'hydrated-draft@example.com',
      title: 'Claimed draft',
    })

    let saveStarted!: () => void
    const started = new Promise<void>((resolve) => (saveStarted = resolve))
    let releaseSave!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') return { message_id: 'replacement-draft@example.com' }
      if (command === 'mail.saveDraft') {
        saveStarted()
        await new Promise<void>((resolve) => (releaseSave = resolve))
      }
      return {}
    }

    const saving = saveQuickReplyDraft()
    await started

    expect(compose$.quickReplyDraftId.get()).not.toBe('hydrated-draft@example.com')
    expect(compose$.quickReplyDraftSaved.get()).toBe(false)

    releaseSave()
    await saving
    expect(compose$.quickReplyDraftId.get()).toBe('replacement-draft@example.com')
    expect(compose$.quickReplyDraftSaved.get()).toBe(true)
  })

  it('does nothing when there is no active thread or the composer is empty', async () => {
    await saveQuickReplyDraft()
    expect(calls.some((c) => c.command === 'mail.saveDraft')).toBe(false)
    expect(compose$.quickReplyDraftSaved.get()).toBe(false)
  })

  it('discards the quick reply draft once cleared back to blank', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
    })
    const draft = message({
      id: 'draft-row',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      thread_id: 't-1',
      from_addr: 'me@example.com',
      message_id: 'draft-1@example.com',
      body: 'Hello there',
      date: thread.date + 1,
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread, draft])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Hello there')
    compose$.quickReplyDraftId.set('draft-1@example.com')
    compose$.quickReplyDraftSaved.set(true)

    compose$.composer.set('')
    await discardQuickReplyDraftIfEmpty()

    expect(calls.find((c) => c.command === 'mail.discardDraft')?.payload).toMatchObject({
      account_id: 'acc-1',
      draft_id: 'draft-1@example.com',
      thread_id: 't-1',
    })
    expect(mail$.messages.get()).toEqual([thread])
    expect(compose$.quickReplyDraftId.get()).toBe('')
    expect(compose$.quickReplyDraftSaved.get()).toBe(false)
  })

  it('discards the copy a failed autosave may have left once cleared back to blank', async () => {
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
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') return { message_id: 'uncertain@example.com' }
      // The host gave up waiting; the core may still have written the draft.
      if (command === 'mail.saveDraft') throw new Error('sidecar save_draft timeout')
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    await saveQuickReplyDraft()
    expect(compose$.quickReplyDraftSaved.get()).toBe(false)

    compose$.composer.set('')
    await discardQuickReplyDraftIfEmpty()

    expect(calls.find((c) => c.command === 'mail.discardDraft')?.payload).toMatchObject({
      account_id: 'acc-1',
      draft_id: 'uncertain@example.com',
    })
    expect(compose$.quickReplyDraftId.get()).toBe('')
  })

  it('discards the draft when the composer is cleared while the autosave is still in flight', async () => {
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
    compose$.composer.set('abc')

    let saveStarted!: () => void
    const started = new Promise<void>((resolve) => (saveStarted = resolve))
    let releaseSave!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') return { message_id: 'draft-core@example.com' }
      if (command === 'mail.saveDraft') {
        saveStarted()
        await new Promise<void>((resolve) => (releaseSave = resolve))
      }
      return {}
    }

    const save = saveQuickReplyDraft()
    await started

    // Cleared mid-save: quickReplyDraftSaved is still false here, so this
    // discard alone can't see the draft the RPC is about to create.
    compose$.composer.set('')
    const discard = discardQuickReplyDraftIfEmpty()
    releaseSave()
    await save
    await discard

    expect(calls.filter((c) => c.command === 'mail.saveDraft')).toHaveLength(1)
    const discardCalls = calls.filter((c) => c.command === 'mail.discardDraft')
    expect(discardCalls).toHaveLength(1)
    expect(discardCalls[0].payload).toMatchObject({ account_id: 'acc-1', thread_id: 't-1' })
    expect(compose$.quickReplyDraftId.get()).toBe('')
    expect(compose$.quickReplyDraftSaved.get()).toBe(false)
  })

  it('does nothing when there is no saved draft to discard', async () => {
    await discardQuickReplyDraftIfEmpty()
    expect(calls.some((c) => c.command === 'mail.discardDraft')).toBe(false)
  })

  it('hydrates the quick reply from a saved draft at the tail of the conversation', () => {
    ui$.selectedThread.set('t-2')
    const ancestor = message({
      id: 'm1',
      thread_id: 't-2',
      folder_id: 'INBOX',
      date: 1000,
    })
    const draft = message({
      id: 'd1',
      thread_id: 't-2',
      folder_id: 'Drafts',
      from_addr: 'me@example.com',
      message_id: 'draft-1@example.com',
      body: 'saved draft body',
      date: 2000,
    })

    mail$.messages.set([ancestor, draft])

    expect(compose$.composer.get()).toBe('saved draft body')
    expect(compose$.quickReplyDraftId.get()).toBe('draft-1@example.com')
    expect(compose$.quickReplyDraftSaved.get()).toBe(true)
    expect(compose$.quickReplyFrom.get()).toBe('me@example.com')
  })

  it.each([
    ['a rich body', { body_html: '<p>rich</p>' }],
    ['a Bcc', { bcc: 'hidden@example.com' }],
    ['a Reply-To', { reply_to: 'elsewhere@example.com' }],
    ['other recipients', { to: 'someone-else@example.com' }],
    ['added recipients', { cc: 'added@example.com' }],
    ['another subject', { subject: 'Re: Reworded' }],
  ])('leaves a draft with %s to the full composer', (_, draft) => {
    const thread = message({ id: 'm1', thread_id: 't-2', folder_id: 'INBOX', date: 1000 })
    mail$.threads.set([thread])
    ui$.selectedThread.set('t-2')

    mail$.messages.set([
      thread,
      message({
        id: 'd1',
        thread_id: 't-2',
        folder_id: 'Drafts',
        to: 'Sender <sender@example.com>',
        subject: 'Re: Notification thread',
        message_id: 'draft-1@example.com',
        body: 'saved draft body',
        date: 2000,
        ...draft,
      }),
    ])

    expect(compose$.composer.get()).toBe('')
    expect(compose$.quickReplyDraftId.get()).toBe('')
  })

  it('takes a plain draft the quick reply could have written itself', () => {
    const thread = message({ id: 'm1', thread_id: 't-2', folder_id: 'INBOX', date: 1000 })
    mail$.threads.set([thread])
    ui$.selectedThread.set('t-2')

    mail$.messages.set([
      thread,
      message({
        id: 'd1',
        thread_id: 't-2',
        folder_id: 'Drafts',
        to: 'sender@example.com',
        subject: 'Re: Notification thread',
        message_id: 'draft-1@example.com',
        body: 'saved draft body',
        date: 2000,
      }),
    ])

    expect(compose$.composer.get()).toBe('saved draft body')
  })

  it('recognizes a draft hydrated before the quick-reply component mounts', () => {
    compose$.composer.set('Already hydrated')
    compose$.quickReplyDraftId.set('draft-1@example.com')
    compose$.quickReplyDraftSaved.set(true)
    compose$.quickReplyDraftThreadId.set('t-1')

    expect(quickReplyDraftBelongsToThread('t-1')).toBe(true)
    expect(quickReplyDraftBelongsToThread('t-2')).toBe(false)
    compose$.quickReplyDraftSaved.set(false)
    expect(quickReplyDraftBelongsToThread('t-1')).toBe(true)
  })

  it('keeps the hydrated draft hidden after an optimistic sent bubble is appended', () => {
    const ancestor = message({ id: 'm1', folder_id: 'INBOX', message_id: 'root@example.com' })
    const draft = message({
      id: 'd1',
      folder_id: 'Drafts',
      message_id: 'draft-1@example.com',
    })
    const sending = message({ id: 'local-send-1', folder_id: 'INBOX', send_status: 'sending' })

    expect(withoutHydratedQuickReplyDraft([ancestor, draft, sending], 'draft-1@example.com', true)).toEqual([
      ancestor,
      sending,
    ])
  })
})
