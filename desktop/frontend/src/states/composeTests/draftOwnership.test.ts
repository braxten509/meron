import { describe, expect, it } from 'bun:test'
import { activateConversationTab, openReplyInFullEditor } from '../compose'
import { compose$ } from '../composeState'
import { quickReplyDraftBelongsToThread, saveQuickReplyDraft } from '../quickReply'
import { retrySend, sendReply } from '../quickReplySend'
import { settings$ } from '../settings'
import { ui$ } from '../ui'
import { mail$ } from '../mail'
import { message } from './message'
import { setupDraftFixture } from './draftFixture'

describe('quick reply draft sharing', () => {
  const calls = setupDraftFixture()

  it('discards the draft an autosave was still writing when the send overtook it', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Racing the autosave')
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    let saveStarted!: () => void
    const started = new Promise<void>((resolve) => (saveStarted = resolve))
    let releaseSave!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') {
        return { message_id: (payload as { draft?: boolean }).draft ? 'raced-draft@example.com' : 'sent@example.com' }
      }
      if (command === 'mail.saveDraft') {
        saveStarted()
        await new Promise<void>((resolve) => (releaseSave = resolve))
      }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    // The autosave is on the wire — its draft exists on the server, but the
    // "saved" flag hasn't flipped yet — when the user hits send.
    const save = saveQuickReplyDraft()
    await started
    expect(compose$.quickReplyDraftSaved.get()).toBe(false)
    const send = sendReply()
    await new Promise((resolve) => setTimeout(resolve, 0))
    // The discard waits for that save rather than deleting nothing ahead of it.
    expect(calls.some((call) => call.command === 'mail.discardDraft')).toBe(false)
    // The user starts the next reply straight away, so the save's own
    // "composer went blank under me" cleanup won't fire: the send is the only
    // thing left that still knows this draft id.
    compose$.composer.set('Next reply')

    releaseSave()
    await save
    await send

    const discard = calls.find((call) => call.command === 'mail.discardDraft')
    expect((discard?.payload as { draft_id?: string })?.draft_id).toBe('raced-draft@example.com')
  })

  it('keeps the saved draft when a send that overtook its autosave fails', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Reply text')
    compose$.quickReplyDraftId.set('safety-draft@example.com')
    compose$.quickReplyDraftSaved.set(true)
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    let saveStarted!: () => void
    const started = new Promise<void>((resolve) => (saveStarted = resolve))
    let releaseSave!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') return { message_id: 'sent@example.com' }
      if (command === 'mail.saveDraft') {
        saveStarted()
        await new Promise<void>((resolve) => (releaseSave = resolve))
      }
      if (command === 'mail.send') throw new Error('smtp unreachable')
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    // A replacement autosave of the already-saved draft is on the wire when the
    // send takes the box, and the send then fails.
    const save = saveQuickReplyDraft()
    await started
    const send = sendReply()
    await new Promise((resolve) => setTimeout(resolve, 0))
    releaseSave()
    await save
    await send

    // The draft is the only copy of a reply that never went out.
    expect(calls.some((call) => call.command === 'mail.discardDraft')).toBe(false)
  })

  it('does not hand a draft written after the send was clicked to that send', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('First reply')
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    let identityStarted!: () => void
    const started = new Promise<void>((resolve) => (identityStarted = resolve))
    let releaseIdentity!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') {
        if ((payload as { draft?: boolean }).draft) return { message_id: 'later-draft@example.com' }
        identityStarted()
        await new Promise<void>((resolve) => (releaseIdentity = resolve))
        return { message_id: 'sent@example.com' }
      }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    // The send is still allocating its identity — an await the user can type
    // through — when the next autosave writes a draft the send never took.
    const send = sendReply()
    await started
    compose$.composer.set('First reply, and more written after the click')
    await saveQuickReplyDraft()
    releaseIdentity()
    await send

    const discarded = calls
      .filter((call) => call.command === 'mail.discardDraft')
      .map((call) => (call.payload as { draft_id?: string }).draft_id)
    expect(discarded).not.toContain('later-draft@example.com')
  })

  it('discards the draft an autosave lands after the full editor took the reply over', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Escalate me')
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    let saveStarted!: () => void
    const started = new Promise<void>((resolve) => (saveStarted = resolve))
    let releaseSave!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') return { message_id: 'orphan-draft@example.com' }
      if (command === 'mail.saveDraft') {
        saveStarted()
        await new Promise<void>((resolve) => (releaseSave = resolve))
      }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    // The first autosave is still allocating its id — nothing the escalation can
    // hand over — so the full editor starts a draft of its own.
    const save = saveQuickReplyDraft()
    await started
    openReplyInFullEditor()
    // Back in the conversation, writing the next reply: the box being non-blank
    // is what used to leave the landed copy stranded in Drafts.
    activateConversationTab()
    compose$.composer.set('A different reply')

    releaseSave()
    await save

    const discard = calls.find((call) => call.command === 'mail.discardDraft')
    expect((discard?.payload as { draft_id?: string })?.draft_id).toBe('orphan-draft@example.com')
  })

  it('does not discard the reclaimed draft when a failed send is retried', async () => {
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
    mail$.threads.set([thread, other])
    mail$.messages.set([thread])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Reply text')
    compose$.quickReplyDraftId.set('safety-draft@example.com')
    compose$.quickReplyDraftSaved.set(true)
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    let failSend = true
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') return { message_id: 'sent@example.com' }
      if (command === 'mail.send' && failSend) throw new Error('smtp unreachable')
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    await sendReply()
    const failed = mail$.messages.get().find((item) => item.send_status === 'failed')
    expect(failed).toBeTruthy()

    // Leaving the thread releases the failed send's safety draft, and coming
    // back hydrates it into the box, where the user edits it further.
    ui$.selectedThread.set('t-2')
    ui$.selectedThread.set('t-1')
    const safetyDraft = message({
      id: 'draft-row',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      to: 'them@example.com',
      subject: 'Re: Notification thread',
      outgoing: true,
      thread_id: 't-1',
      message_id: 'safety-draft@example.com',
      body: 'Reply text',
      date: 2000,
    })
    mail$.messages.set([thread, safetyDraft])
    expect(compose$.quickReplyDraftId.get()).toBe('safety-draft@example.com')

    failSend = false
    calls.length = 0
    await retrySend(failed!.id)

    // The box is still editing that draft — the retry's cleanup isn't its owner.
    expect(calls.some((call) => call.command === 'mail.discardDraft')).toBe(false)
    expect(compose$.quickReplyDraftId.get()).toBe('safety-draft@example.com')
  })

  it('leaves a draft the full editor took over out of the quick reply', () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Escalate me')
    compose$.quickReplyDraftId.set('escalated-draft@example.com')
    compose$.quickReplyDraftSaved.set(true)
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    openReplyInFullEditor()
    activateConversationTab()

    // A refresh delivers the escalated draft as the thread's tail.
    const escalated = message({
      id: 'draft-row',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      thread_id: 't-1',
      message_id: 'escalated-draft@example.com',
      body: 'Escalate me',
      date: 2000,
    })
    mail$.messages.set([thread, escalated])

    // Two editors on one server draft is how it gets discarded under the tab.
    expect(compose$.quickReplyDraftId.get()).toBe('')
    expect(compose$.composer.get()).not.toBe('Escalate me')
  })

  it('discards a draft that lands between the send click and its guard', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Racing reply')
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    let saveStarted!: () => void
    const saveRunning = new Promise<void>((resolve) => (saveStarted = resolve))
    let releaseSave!: () => void
    let identityStarted!: () => void
    const allocating = new Promise<void>((resolve) => (identityStarted = resolve))
    let releaseIdentity!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') {
        if ((payload as { draft?: boolean }).draft) return { message_id: 'inflight-draft@example.com' }
        identityStarted()
        await new Promise<void>((resolve) => (releaseIdentity = resolve))
        return { message_id: 'sent@example.com' }
      }
      if (command === 'mail.saveDraft') {
        saveStarted()
        await new Promise<void>((resolve) => (releaseSave = resolve))
      }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    // The autosave is on the wire at the click, and lands while the send is
    // still allocating its identity — before the send has a guard to adopt it.
    const save = saveQuickReplyDraft()
    await saveRunning
    const send = sendReply()
    await allocating
    releaseSave()
    await save
    // The late save belongs to the send now, not to the box. A remount in this
    // allocation window must not preserve the id the send will discard.
    expect(compose$.quickReplyDraftId.get()).toBe('')
    expect(compose$.quickReplyDraftThreadId.get()).toBe('')
    expect(quickReplyDraftBelongsToThread('t-1')).toBe(false)
    releaseIdentity()
    await send

    const discard = calls.find((call) => call.command === 'mail.discardDraft')
    expect((discard?.payload as { draft_id?: string })?.draft_id).toBe('inflight-draft@example.com')
  })

  it('does not hydrate a draft into the box while a send is still allocating its identity', async () => {
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
    mail$.threads.set([thread, other])
    mail$.messages.set([thread])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Reply text')
    compose$.quickReplyDraftId.set('claimed-draft@example.com')
    compose$.quickReplyDraftSaved.set(true)
    compose$.quickReplyDraftThreadId.set('t-1')
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    let identityStarted!: () => void
    const allocating = new Promise<void>((resolve) => (identityStarted = resolve))
    let releaseIdentity!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') {
        identityStarted()
        await new Promise<void>((resolve) => (releaseIdentity = resolve))
        return { message_id: 'sent@example.com' }
      }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    const send = sendReply()
    await allocating
    // The user closes the conversation and reopens it while the allocation
    // stalls — which resets the box — then a refresh delivers the claimed draft
    // as the thread's tail.
    ui$.selectedThread.set('')
    ui$.selectedThread.set('t-1')
    const claimed = message({
      id: 'draft-row',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      thread_id: 't-1',
      message_id: 'claimed-draft@example.com',
      body: 'Reply text',
      date: 2000,
    })
    mail$.messages.set([thread, claimed])

    // The send owns it and will discard it; editing it here would lose that work.
    expect(compose$.quickReplyDraftId.get()).toBe('')

    releaseIdentity()
    await send
    const discarded = calls
      .filter((call) => call.command === 'mail.discardDraft')
      .map((call) => (call.payload as { draft_id?: string }).draft_id)
    expect(discarded).toContain('claimed-draft@example.com')
  })

  it('hands the draft back to the box when the send never gets off the ground', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Racing reply')
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    let saveStarted!: () => void
    const saveRunning = new Promise<void>((resolve) => (saveStarted = resolve))
    let releaseSave!: () => void
    let identityStarted!: () => void
    const allocating = new Promise<void>((resolve) => (identityStarted = resolve))
    let failIdentity!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') {
        if ((payload as { draft?: boolean }).draft) return { message_id: 'inflight-draft@example.com' }
        identityStarted()
        await new Promise<void>((_resolve, reject) => (failIdentity = () => reject(new Error('offline'))))
      }
      if (command === 'mail.saveDraft') {
        saveStarted()
        await new Promise<void>((resolve) => (releaseSave = resolve))
      }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    // The autosave lands mid-send and hands its id to the claim; then the send
    // dies before it ever has a guard.
    const save = saveQuickReplyDraft()
    await saveRunning
    const send = sendReply().catch(() => undefined)
    await allocating
    releaseSave()
    await save
    failIdentity()
    await send

    // The box is left holding a draft it knows is on the server.
    expect(compose$.quickReplyDraftId.get()).toBe('inflight-draft@example.com')
    expect(compose$.quickReplyDraftSaved.get()).toBe(true)
    expect(calls.some((call) => call.command === 'mail.discardDraft')).toBe(false)
  })

  it('keeps the unsent draft when a send dies after the user has left the thread', async () => {
    const thread = message({
      id: 'root',
      account_id: 'acc-1',
      thread_id: 't-1',
      folder_id: 'INBOX',
      from_addr: 'them@example.com',
      message_id: 'root@example.com',
      date: 1000,
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Never sent')
    compose$.quickReplyDraftId.set('unsent-draft@example.com')
    compose$.quickReplyDraftSaved.set(true)
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    let identityStarted!: () => void
    const allocating = new Promise<void>((resolve) => (identityStarted = resolve))
    let failIdentity!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') {
        identityStarted()
        await new Promise<void>((_resolve, reject) => (failIdentity = () => reject(new Error('offline'))))
      }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    const send = sendReply().catch(() => undefined)
    await allocating
    ui$.selectedThread.set('')
    failIdentity()
    await send

    // Nothing was ever sent, so the draft is still the only copy of that reply.
    expect(calls.some((call) => call.command === 'mail.discardDraft' || call.command === 'mail.delete')).toBe(false)
  })
})
