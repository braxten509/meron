// Restoring an unsent reply when delivery or draft recovery fails.
// A failed send can race with deletion, draft recovery or opening the editor.
// The fixtures exercise which draft each action owns when its request completes.
import { describe, expect, it } from 'bun:test'
import { openDraftCompose, openComposeTab } from '../compose'
import { compose$ } from '../composeState'
import { saveQuickReplyDraft, withoutHydratedQuickReplyDraft } from '../quickReply'
import { sendReply } from '../quickReplySend'
import { settings$ } from '../settings'
import { ui$ } from '../ui'
import { mail$ } from '../mail'
import { deleteMessage } from '../mailMoves'
import { message } from './message'
import { setupDraftFixture } from './draftFixture'

describe('quick reply draft sharing', () => {
  const calls = setupDraftFixture()

  it('undoes the rescue when the failed bubble is deleted while it is saving', async () => {
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
    compose$.composer.set('Never left')
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    let identityStarted!: () => void
    const allocating = new Promise<void>((resolve) => (identityStarted = resolve))
    let failIdentity!: () => void
    let saveStarted!: () => void
    const saving = new Promise<void>((resolve) => (saveStarted = resolve))
    let releaseSave!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') {
        if ((payload as { draft?: boolean }).draft) return { message_id: 'rescued-draft@example.com' }
        identityStarted()
        await new Promise<void>((resolve) => (failIdentity = resolve))
        throw new Error('offline')
      }
      if (command === 'mail.saveDraft') {
        saveStarted()
        await new Promise<void>((resolve) => (releaseSave = resolve))
      }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    const send = sendReply()
    await allocating
    compose$.composer.set('Next reply')
    failIdentity()
    await expect(send).rejects.toThrow('offline')

    // The rescue is on the wire when the user deletes the bubble it belongs to.
    await saving
    const bubble = mail$.messages.get().find((m) => m.send_status === 'failed')
    await deleteMessage(bubble!)
    releaseSave()
    await new Promise((resolve) => setTimeout(resolve, 0))

    // The copy it wrote goes with the bubble rather than turning up in Drafts.
    const discard = calls.find((call) => call.command === 'mail.discardDraft')
    expect((discard?.payload as { draft_id?: string })?.draft_id).toBe('rescued-draft@example.com')

    // And the deleted bubble stays deleted across a reload of the thread.
    mail$.messages.set([thread])
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(mail$.messages.get().some((m) => m.send_status)).toBe(false)
  })

  it('lets the autosave it overtook rescue the reply instead of saving a second draft', async () => {
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
    compose$.composer.set('Never left')
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    let saveGateUsed = false
    let saveStarted!: () => void
    const started = new Promise<void>((resolve) => (saveStarted = resolve))
    let releaseSave!: () => void
    let identityStarted!: () => void
    const allocating = new Promise<void>((resolve) => (identityStarted = resolve))
    let failIdentity!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') {
        if ((payload as { draft?: boolean }).draft) return { message_id: 'raced-draft@example.com' }
        identityStarted()
        await new Promise<void>((resolve) => (failIdentity = resolve))
        throw new Error('offline')
      }
      // Only the first save is held open; a rescue save racing it must not
      // hang the test that is meant to prove it never happens.
      if (command === 'mail.saveDraft' && !saveGateUsed) {
        saveGateUsed = true
        saveStarted()
        await new Promise<void>((resolve) => (releaseSave = resolve))
      }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    // The autosave of this reply is on the wire — its id isn't back yet — when
    // the send takes the box and dies, with the user already typing the next.
    const save = saveQuickReplyDraft()
    await started
    const send = sendReply()
    await allocating
    compose$.composer.set('Next reply')
    failIdentity()
    await expect(send).rejects.toThrow('offline')
    releaseSave()
    await save
    await new Promise((resolve) => setTimeout(resolve, 0))

    // That save's copy is the reply: no rescue draft beside it, and nothing
    // discarded it.
    expect(calls.filter((call) => call.command === 'mail.saveDraft')).toHaveLength(1)
    expect(calls.some((call) => call.command === 'mail.discardDraft')).toBe(false)
    expect(mail$.messages.get().some((m) => m.send_status)).toBe(false)
    expect(compose$.composer.get()).toBe('Next reply')
  })

  it('keeps a draft written after a From change during the send', async () => {
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
    const allocating = new Promise<void>((resolve) => (identityStarted = resolve))
    let releaseIdentity!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') {
        if ((payload as { draft?: boolean }).draft) return { message_id: 'from-change-draft@example.com' }
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
    // The click emptied the box; the next reply goes in, and picking another
    // identity re-saves it against that (QuickReplyFrom).
    compose$.composer.set('Second reply')
    compose$.quickReplyFrom.set('me@example.com')
    await saveQuickReplyDraft()
    releaseIdentity()
    await send

    // That draft is the box's, not the send's: clearing the pointer would strand
    // it in Drafts.
    expect(compose$.composer.get()).toBe('Second reply')
    expect(compose$.quickReplyDraftId.get()).toBe('from-change-draft@example.com')
    expect(compose$.quickReplyDraftSaved.get()).toBe(true)
  })

  it('hides the claimed draft from the click, not from the guard', async () => {
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
      message_id: 'claimed-draft@example.com',
      body: 'First reply',
      date: 2000,
    })
    mail$.threads.set([thread])
    mail$.messages.set([thread, draft])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('First reply')
    compose$.quickReplyDraftId.set('claimed-draft@example.com')
    compose$.quickReplyDraftSaved.set(true)
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

    // The box has let go of the draft; nothing else may put it back on screen
    // beside the reply going out.
    expect(compose$.sendingDraftIds.get()).toContain('claimed-draft@example.com')
    expect(
      withoutHydratedQuickReplyDraft(
        [thread, draft],
        compose$.quickReplyDraftId.get(),
        compose$.quickReplyDraftSaved.get(),
        compose$.sendingDraftIds.get(),
      ),
    ).toEqual([thread])

    releaseIdentity()
    await send
    // The post-send discard runs on after the send resolves.
    for (let attempt = 0; attempt < 20 && compose$.sendingDraftIds.get().length > 0; attempt += 1) {
      await new Promise((resolve) => setTimeout(resolve, 0))
    }
    expect(compose$.sendingDraftIds.get()).toEqual([])
  })

  it('leaves a draft open in the full editor alone when the send completes', async () => {
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
    compose$.quickReplyDraftId.set('claimed-draft@example.com')
    compose$.quickReplyDraftSaved.set(true)
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    let sendStarted!: () => void
    const sending = new Promise<void>((resolve) => (sendStarted = resolve))
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
    await sending
    // The user opens that same draft from the Drafts list while the send is out.
    openComposeTab({
      accountId: 'acc-1',
      to: 'them@example.com',
      subject: 'Re: Notification thread',
      text: 'First reply',
      draftMessageId: 'claimed-draft@example.com',
    })

    releaseSend()
    await send

    // Discarding it would empty the editor the user is typing in.
    expect(calls.some((call) => call.command === 'mail.discardDraft')).toBe(false)
  })

  it('leaves a draft alone while the user is opening it, before its tab exists', async () => {
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
    compose$.quickReplyDraftId.set('claimed-draft@example.com')
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
    const draftRow = message({
      id: 'draft-row',
      account_id: 'acc-1',
      folder_id: 'Drafts',
      thread_id: 't-9',
      message_id: 'claimed-draft@example.com',
      body: 'First reply',
      date: 2000,
    })

    const send = sendReply()
    await sending
    // Clicking the draft in the Drafts list reads the thread first, so its tab
    // appears an RPC after the click — the window the cleanup used to fall into.
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
    const tab = compose$.tabs.get().find((item) => item.kind === 'compose')
    expect(tab?.compose?.draftMessageId).toBe('claimed-draft@example.com')
  })

  it('gives the editor its own draft when the one it opens is already being expunged', async () => {
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

    let discardStarted!: () => void
    const discarding = new Promise<void>((resolve) => (discardStarted = resolve))
    let releaseDiscard!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') return { message_id: 'sent@example.com' }
      if (command === 'mail.discardDraft') {
        discardStarted()
        await new Promise<void>((resolve) => (releaseDiscard = resolve))
      }
      if (command === 'mail.threadRead') return { messages: [draftRow] }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    // The send is done and its cleanup is on the wire when the user clicks the
    // draft: the copy they are opening is on its way out.
    await sendReply()
    await discarding
    const opening = openDraftCompose(draftRow)
    releaseDiscard()
    await opening

    // Their text opens as a draft of its own rather than one being deleted.
    const tab = compose$.tabs.get().find((item) => item.kind === 'compose')
    expect(tab?.compose?.draftMessageId).not.toBe('claimed-draft@example.com')
    expect(tab?.compose?.draftMessageId).toMatch(/^local-draft-/)
    expect(tab?.compose?.text).toBe('First reply')
  })
})
