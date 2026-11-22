// Edits and attachment changes made while a previous reply is being sent.
// A pending send can finish after the user has begun a new draft.
// Cases cover preserving the new text, attachments and autosave state.
import { describe, expect, it } from 'bun:test'
import { openReplyInFullEditor } from '../compose'
import { compose$ } from '../composeState'
import { saveQuickReplyDraft, scheduleQuickReplyDraftSave } from '../quickReply'
import { sendReply } from '../quickReplySend'
import { settings$ } from '../settings'
import { ui$ } from '../ui'
import { mail$ } from '../mail'
import { message } from './message'
import { setupDraftFixture } from './draftFixture'

describe('quick reply draft sharing', () => {
  const calls = setupDraftFixture()

  it('leaves a reply written during the send alone instead of clearing the box', async () => {
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
        if ((payload as { draft?: boolean }).draft) return { message_id: 'later-draft@example.com' }
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
    compose$.composer.set('A second reply, written while the first was going out')
    await saveQuickReplyDraft()
    releaseIdentity()
    await send

    // The send took the first reply; the second is the user's, and so is the
    // draft holding it.
    expect(compose$.composer.get()).toBe('A second reply, written while the first was going out')
    expect(compose$.quickReplyDraftId.get()).toBe('later-draft@example.com')
    expect(compose$.quickReplyDraftSaved.get()).toBe(true)
  })

  it("does not clear another thread's quick reply when a send completes", async () => {
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
    mail$.messages.set([thread, other])
    ui$.selectedThread.set('t-1')
    compose$.composer.set('Reply in the first thread')
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
    ui$.selectedThread.set('t-2')
    compose$.composer.set('Half-written reply in the second thread')
    releaseIdentity()
    await send

    expect(compose$.composer.get()).toBe('Half-written reply in the second thread')
  })

  it('gives a reply written during the send a draft of its own', async () => {
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

    let identityStarted!: () => void
    const allocating = new Promise<void>((resolve) => (identityStarted = resolve))
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

    // The send took the box's saved draft with it; the reply written next must
    // not be saved over that same server copy.
    const send = sendReply()
    await allocating
    compose$.composer.set('A second reply')
    await saveQuickReplyDraft()
    releaseIdentity()
    await send

    const saved = calls
      .filter((call) => call.command === 'mail.saveDraft')
      .map((call) => (call.payload as { draft_id?: string }).draft_id)
    expect(saved).toEqual(['later-draft@example.com'])
    const discarded = calls
      .filter((call) => call.command === 'mail.discardDraft')
      .map((call) => (call.payload as { draft_id?: string }).draft_id)
    expect(discarded).toEqual(['claimed-draft@example.com'])
    expect(compose$.quickReplyDraftId.get()).toBe('later-draft@example.com')
    expect(compose$.quickReplyDraftSaved.get()).toBe(true)
  })

  it('keeps an attachment swapped in during the send', async () => {
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
    compose$.composer.set('With a file')
    compose$.composerAttachments.set([
      { id: 'a1', filename: 'first.pdf', mime: 'application/pdf', size: 10, data: 'AA==' },
    ])
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

    // Same text, same attachment count, different file: by value the box looks
    // untouched, but the user picked this one after clicking send.
    const send = sendReply()
    await allocating
    compose$.composerAttachments.set([
      { id: 'a2', filename: 'second.pdf', mime: 'application/pdf', size: 12, data: 'BB==' },
    ])
    releaseIdentity()
    await send

    expect(compose$.composerAttachments.get().map((item) => item.filename)).toEqual(['second.pdf'])
  })

  it('cancels the pending autosave the moment the send takes the box', async () => {
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
    compose$.composer.set('Typed then sent')
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

    // The autosave is armed from typing when the user clicks send, and the
    // identity allocation outlasts the debounce.
    scheduleQuickReplyDraftSave()
    const send = sendReply()
    await allocating
    await new Promise((resolve) => setTimeout(resolve, 1400))

    // A draft landing here would have nothing pointing at it once the send
    // resets the box — the stranded copy this whole thing is about.
    expect(calls.some((call) => call.command === 'mail.saveDraft')).toBe(false)

    releaseIdentity()
    await send
  })

  it('takes the draft out of the box the moment the send claims it', async () => {
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

    // The send owns that draft and will discard it; the box must not still be
    // pointing at it while the user carries on typing.
    expect(compose$.quickReplyDraftId.get()).toBe('')
    expect(compose$.quickReplyDraftSaved.get()).toBe(false)
    expect(compose$.quickReplyDraftThreadId.get()).toBe('')

    releaseIdentity()
    await send
    const discarded = calls
      .filter((call) => call.command === 'mail.discardDraft')
      .map((call) => (call.payload as { draft_id?: string }).draft_id)
    expect(discarded).toEqual(['claimed-draft@example.com'])
  })

  it('does not hand the claimed draft to the full editor mid-send', async () => {
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
    compose$.composer.set('Something else entirely')
    openReplyInFullEditor()

    // Handing the tab the claimed id would have the send delete the draft out
    // from under the full editor.
    const tab = compose$.tabs.get().find((item) => item.kind === 'compose')
    expect(tab?.compose?.draftMessageId).not.toBe('claimed-draft@example.com')

    releaseIdentity()
    await send
  })

  it('re-arms the autosave when the send dies before it gets a guard', async () => {
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
    compose$.composer.set('Never saved anywhere')
    compose$.quickReplySignature.set(null)
    settings$.signature.set('')

    let identityStarted!: () => void
    const allocating = new Promise<void>((resolve) => (identityStarted = resolve))
    let failIdentity!: () => void
    ;(window as any).go.main.App.Invoke = async (command: string, payload: unknown) => {
      calls.push({ command, payload })
      if (command === 'mail.allocateIdentity') {
        if ((payload as { draft?: boolean }).draft) return { message_id: 'recovered-draft@example.com' }
        identityStarted()
        await new Promise<void>((_resolve, reject) => (failIdentity = () => reject(new Error('offline'))))
      }
      if (command === 'mail.folderList') return { folders: [] }
      if (command === 'mail.threadList') return { threads: [] }
      return {}
    }

    // The click cancelled the debounce; nothing else re-arms it if the user
    // doesn't type again, and their reply is not saved anywhere.
    scheduleQuickReplyDraftSave()
    const send = sendReply().catch(() => undefined)
    await allocating
    failIdentity()
    await send
    await new Promise((resolve) => setTimeout(resolve, 1400))

    expect(calls.some((call) => call.command === 'mail.saveDraft')).toBe(true)
  })
})
