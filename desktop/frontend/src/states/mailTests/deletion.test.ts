// Delete messages and clean up their drafts.
// Uses the same observable state and command interface as the thread list.
// Fixtures supply synthetic messages, independent of a real mailbox.
import { beforeEach, describe, expect, it } from 'bun:test'
import { kanban$, kanbanColumnKey } from '../kanban'
import { mail$ } from '../mail'
import { deleteThread, discardSavedDraftCopy } from '../mailMoves'
import { settings$ } from '../settings'
import { settleConfirm, ui$ } from '../ui'
import { thread } from './fixture'

describe('deleteThread', () => {
  const calls: { command: string; payload: unknown }[] = []
  // What the mocked backend returns per command; tests override per scenario.
  let responses: Record<string, unknown> = {}

  beforeEach(() => {
    calls.length = 0
    responses = { 'mail.delete': { ok: true, deleted: 0 } }
    mail$.threads.set([thread()])
    mail$.messages.set([])
    mail$.folders.set([
      { id: 'inbox', account_id: 'acc', name: 'Inbox', role: 'inbox', unread: 0 },
      { id: 'Trash', account_id: 'acc', name: 'Trash', role: 'trash', unread: 0 },
    ])
    mail$.foldersByAccount.set({
      acc: [
        { id: 'inbox', account_id: 'acc', name: 'Inbox', role: 'inbox', unread: 0 },
        { id: 'Trash', account_id: 'acc', name: 'Trash', role: 'trash', unread: 0 },
      ],
    })
    kanban$.threads.set({})
    ui$.selectedThread.set('acc:inbox:thread:1')
    ui$.selectedAccount.set('acc')
    ui$.selectedFolder.set('inbox')
    ui$.toast.set('')
    ui$.toastTone.set('success')
    ui$.toastUndo.set(null)
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: unknown) => {
            calls.push({ command, payload })
            return responses[command] ?? {}
          },
        },
      },
    }
  })

  it('rolls back and shows an error when delete affects no messages', async () => {
    await deleteThread('acc:inbox:thread:1')

    expect(calls.filter((call) => call.command === 'mail.delete')).toHaveLength(1)
    expect(calls.some((call) => call.command === 'mail.threadList')).toBe(false)
    expect(mail$.threads.get()).toHaveLength(1)
    expect(ui$.selectedThread.get()).toBe('acc:inbox:thread:1')
    expect(ui$.toastTone.get()).toBe('error')
    expect(ui$.toast.get()).toBe('Delete failed: no matching messages found')
  })

  it('shows an error when the backend reports success but the thread survives the refresh', async () => {
    responses['mail.delete'] = { ok: true, deleted: 2 }
    responses['mail.threadList'] = { threads: [thread()] }

    await deleteThread('acc:inbox:thread:1')

    const threadListCall = calls.find((call) => call.command === 'mail.threadList')
    expect(threadListCall?.payload).toMatchObject({ refresh: false })
    expect(mail$.threads.get()).toHaveLength(1)
    expect(ui$.toastTone.get()).toBe('error')
    expect(ui$.toast.get()).toBe('Delete failed: thread is still in this folder')
  })

  it('shows the success toast when the thread is gone after the refresh', async () => {
    responses['mail.delete'] = { ok: true, deleted: 2 }
    responses['mail.threadList'] = { threads: [] }

    await deleteThread('acc:inbox:thread:1')

    const threadListCall = calls.find((call) => call.command === 'mail.threadList')
    expect(threadListCall?.payload).toMatchObject({ refresh: false })
    expect(mail$.threads.get()).toHaveLength(0)
    expect(ui$.toastTone.get()).toBe('success')
    expect(ui$.toast.get()).toBe('Thread moved to Trash')
  })

  it('advances the selection to the next thread after deleting the selected one', async () => {
    const second = thread({
      id: 'acc:inbox:thread:2#201',
      thread_id: 'acc:inbox:thread:2',
      date: Math.floor(Date.parse('2026-06-10T12:00:00Z') / 1000),
    })
    mail$.threads.set([thread(), second])
    responses['mail.delete'] = { ok: true, deleted: 2 }
    responses['mail.threadList'] = { threads: [second] }

    await deleteThread('acc:inbox:thread:1')

    expect(mail$.threads.get()).toHaveLength(1)
    expect(ui$.selectedThread.get()).toBe('acc:inbox:thread:2')
  })

  it('opens the replacement thread when the deleted one had no visible neighbour', async () => {
    const replacement = thread({
      id: 'acc:inbox:thread:2#201',
      thread_id: 'acc:inbox:thread:2',
      date: Math.floor(Date.parse('2026-06-10T12:00:00Z') / 1000),
    })
    responses['mail.delete'] = { ok: true, deleted: 2 }
    // The deleted thread was the only row we had, so the neighbour comes from
    // the refreshed list rather than from the local one.
    responses['mail.threadList'] = { threads: [replacement] }

    await deleteThread('acc:inbox:thread:1')

    expect(ui$.selectedThread.get()).toBe('acc:inbox:thread:2')
  })

  it('advances the selection to the column neighbour when deleting in kanban view', async () => {
    const second = thread({
      id: 'acc:inbox:thread:2#201',
      thread_id: 'acc:inbox:thread:2',
      date: Math.floor(Date.parse('2026-06-10T12:00:00Z') / 1000),
    })
    kanban$.activeBoardId.set('board')
    kanban$.threads.set({ 'acc:inbox': [thread(), second] })
    kanban$.paneThreadId.set('acc:inbox:thread:1')
    responses['mail.delete'] = { ok: true, deleted: 2 }
    responses['mail.threadList'] = { threads: [second] }

    await deleteThread('acc:inbox:thread:1')

    expect(ui$.selectedThread.get()).toBe('acc:inbox:thread:2')
    expect(kanban$.paneThreadId.get()).toBe('acc:inbox:thread:2')
    kanban$.activeBoardId.set('')
    kanban$.paneThreadId.set('')
  })

  it('permanently deletes threads already in Trash', async () => {
    const trashThread = thread({
      id: 'acc:Trash:thread:1#101',
      folder_id: 'Trash',
      thread_id: 'acc:Trash:thread:1',
    })
    mail$.threads.set([trashThread])
    ui$.selectedThread.set('acc:Trash:thread:1')
    ui$.selectedFolder.set('Trash')
    responses['mail.delete'] = { ok: true, deleted: 1, permanent: true }
    responses['mail.threadList'] = { threads: [] }

    const pending = deleteThread('acc:Trash:thread:1')
    settleConfirm(true)

    await pending

    expect(calls.find((call) => call.command === 'mail.delete')?.payload).toMatchObject({
      thread_id: 'acc:Trash:thread:1',
      folder: 'Trash',
    })
    expect(mail$.threads.get()).toHaveLength(0)
    expect(ui$.toastTone.get()).toBe('success')
    expect(ui$.toast.get()).toBe('Thread deleted')
  })

  it('discards drafts permanently with draft wording (engine expunges in place)', async () => {
    const draftThread = thread({
      id: 'acc:Drafts:thread:1#101',
      folder_id: 'Drafts',
      thread_id: 'acc:Drafts:thread:1',
    })
    mail$.threads.set([draftThread])
    ui$.selectedThread.set('acc:Drafts:thread:1')
    ui$.selectedFolder.set('Drafts')
    responses['mail.delete'] = { ok: true, deleted: 1, permanent: true }
    responses['mail.threadList'] = { threads: [] }

    const pending = deleteThread('acc:Drafts:thread:1')
    settleConfirm(true)

    await pending

    expect(calls.find((call) => call.command === 'mail.delete')?.payload).toMatchObject({
      thread_id: 'acc:Drafts:thread:1',
      folder: 'Drafts',
    })
    expect(mail$.threads.get()).toHaveLength(0)
    expect(ui$.toastTone.get()).toBe('success')
    expect(ui$.toast.get()).toBe('Draft discarded')
  })

  it('sends the displayed Trash folder even when the thread id still encodes another folder', async () => {
    const movedThread = thread({
      folder_id: 'Trash',
    })
    mail$.threads.set([movedThread])
    ui$.selectedFolder.set('Trash')
    responses['mail.delete'] = { ok: true, deleted: 1, permanent: true }
    responses['mail.threadList'] = { threads: [] }

    const pending = deleteThread('acc:inbox:thread:1')
    settleConfirm(true)

    await pending

    expect(calls.find((call) => call.command === 'mail.delete')?.payload).toMatchObject({
      thread_id: 'acc:inbox:thread:1',
      folder: 'Trash',
    })
    expect(ui$.toast.get()).toBe('Thread deleted')
  })

  it('silently discards the saved draft copy after sending', async () => {
    const draftThread = thread({
      id: 'acc:Drafts:thread:1#101',
      folder_id: 'Drafts',
      thread_id: 'acc:Drafts:thread:1',
    })
    const draftMessage = thread({
      id: 'acc:Drafts:thread:1#101',
      folder_id: 'Drafts',
      thread_id: 'acc:Drafts:thread:1',
    })
    mail$.threads.set([draftThread])
    mail$.messages.set([draftMessage])
    ui$.selectedThread.set('acc:Drafts:thread:1')
    ui$.selectedFolder.set('Drafts')
    responses['mail.delete'] = { ok: true, deleted: 1, permanent: true }
    responses['mail.threadList'] = { threads: [] }

    await discardSavedDraftCopy({
      threadId: 'acc:Drafts:thread:1',
      messageId: 'acc:Drafts:thread:1#101',
      folderId: 'Drafts',
      accountId: 'acc',
    })

    expect(calls.find((call) => call.command === 'mail.delete')?.payload).toMatchObject({
      thread_id: 'acc:Drafts:thread:1',
      message_ids: ['acc:Drafts:thread:1#101'],
      folder: 'Drafts',
    })
    expect(mail$.threads.get()).toHaveLength(0)
    expect(mail$.messages.get()).toHaveLength(0)
    expect(ui$.toast.get()).toBe('')
  })

  it('leaves a conversation that loaded during a failed discard on screen', async () => {
    // A restored compose tab has no conversation loaded; closing it opens one.
    mail$.messages.set([])
    let refuse = (_reason: string) => {}
    const invoke = (window as any).go.main.App.Invoke
    ;(window as any).go.main.App.Invoke = (command: string, payload: unknown) =>
      command === 'mail.discardDraft'
        ? new Promise((_resolve, reject) => {
            refuse = reject
          })
        : invoke(command, payload)

    const discard = discardSavedDraftCopy({
      threadId: '',
      messageId: '',
      folderId: '',
      accountId: 'acc',
      draftMessageId: 'draft-id@example.com',
    })
    const loaded = thread()
    ui$.selectedThread.set(loaded.thread_id)
    mail$.messages.set([loaded])
    refuse('sidecar discard_draft timeout after 15s')

    expect(await discard).toBe(false)
    expect(mail$.messages.get()).toEqual([loaded])
    // Wails rejects with a plain string, which is still the reason.
    expect(ui$.toast.get()).toBe('Could not discard draft: sidecar discard_draft timeout after 15s')
  })

  it('puts a draft back when its discard fails', async () => {
    const reply = thread({ id: 'reply', has_draft: true })
    const saved = thread({ id: 'saved', folder_id: 'Drafts', message_id: 'draft-id@example.com' })
    mail$.threads.set([reply])
    mail$.messages.set([reply, saved])
    ui$.selectedThread.set(reply.thread_id)
    const invoke = (window as any).go.main.App.Invoke
    ;(window as any).go.main.App.Invoke = (command: string, payload: unknown) =>
      command === 'mail.discardDraft' ? Promise.reject('refused') : invoke(command, payload)

    await discardSavedDraftCopy({
      threadId: '',
      messageId: '',
      folderId: '',
      accountId: 'acc',
      draftMessageId: 'draft-id@example.com',
      replyThreadId: reply.thread_id,
    })

    expect(mail$.messages.get().map((message) => message.id)).toEqual(['reply', 'saved'])
    expect(mail$.threads.get()[0].has_draft).toBe(true)
  })

  it('keeps one copy of a draft that a refresh reloaded during its failed discard', async () => {
    const reply = thread({ id: 'reply' })
    const saved = thread({ id: 'saved', folder_id: 'Drafts', message_id: 'draft-id@example.com' })
    mail$.messages.set([reply, saved])
    ui$.selectedThread.set(reply.thread_id)
    let refuse = (_reason: string) => {}
    const invoke = (window as any).go.main.App.Invoke
    ;(window as any).go.main.App.Invoke = (command: string, payload: unknown) =>
      command === 'mail.discardDraft'
        ? new Promise((_resolve, reject) => {
            refuse = reject
          })
        : invoke(command, payload)

    const discard = discardSavedDraftCopy({
      threadId: '',
      messageId: '',
      folderId: '',
      accountId: 'acc',
      draftMessageId: 'draft-id@example.com',
    })
    mail$.messages.set([{ ...reply }, { ...saved }])
    refuse('refused')
    await discard

    expect(mail$.messages.get().map((message) => message.id)).toEqual(['reply', 'saved'])
  })

  it('puts a draft back in date order when its discard fails', async () => {
    const first = thread({ id: 'first', date: 100 })
    const saved = thread({ id: 'saved', folder_id: 'Drafts', message_id: 'draft-id@example.com', date: 200 })
    const last = thread({ id: 'last', date: 300 })
    mail$.messages.set([first, saved, last])
    ui$.selectedThread.set(first.thread_id)
    const invoke = (window as any).go.main.App.Invoke
    ;(window as any).go.main.App.Invoke = (command: string, payload: unknown) =>
      command === 'mail.discardDraft' ? Promise.reject('refused') : invoke(command, payload)

    await discardSavedDraftCopy({
      threadId: '',
      messageId: '',
      folderId: '',
      accountId: 'acc',
      draftMessageId: 'draft-id@example.com',
    })

    expect(mail$.messages.get().map((message) => message.id)).toEqual(['first', 'saved', 'last'])
  })

  it('does not report a discard as failed when only the card reload fails', async () => {
    const reply = thread({ id: 'reply' })
    const column = { accountId: 'acc', folderId: 'inbox' }
    const boards = settings$.kanbanBoards.get()
    settings$.kanbanBoards.set([{ ...boards[0], id: 'board', columns: [column] }])
    kanban$.activeBoardId.set('board')
    kanban$.threads[kanbanColumnKey(column)].set([reply])
    mail$.messages.set([reply])
    ui$.selectedThread.set(reply.thread_id)
    responses['mail.discardDraft'] = { ok: true, deleted: 1 }
    const invoke = (window as any).go.main.App.Invoke
    ;(window as any).go.main.App.Invoke = (command: string, payload: unknown) =>
      command === 'mail.threadList' ? Promise.reject('list timeout') : invoke(command, payload)

    try {
      const discarded = await discardSavedDraftCopy({
        threadId: '',
        messageId: '',
        folderId: '',
        accountId: 'acc',
        draftMessageId: 'draft-id@example.com',
        replyThreadId: reply.thread_id,
      })
      expect(discarded).toBe(true)
      expect(ui$.toast.get()).toBe('')
    } finally {
      settings$.kanbanBoards.set(boards)
      kanban$.activeBoardId.set('')
      kanban$.threads.set({})
    }
  })

  it('discards an autosaved compose draft by stable draft message id', async () => {
    responses['mail.discardDraft'] = { ok: true, deleted: 0, permanent: true }
    responses['mail.threadList'] = { threads: [] }

    await discardSavedDraftCopy({
      threadId: '',
      messageId: '',
      folderId: '',
      accountId: 'acc',
      draftMessageId: 'draft-id@example.com',
    })

    expect(calls.find((call) => call.command === 'mail.discardDraft')?.payload).toMatchObject({
      account_id: 'acc',
      draft_id: 'draft-id@example.com',
    })
    expect(calls.some((call) => call.command === 'mail.delete')).toBe(false)
    expect(ui$.toast.get()).toBe('')
  })

  it('clears the thread draft badge when the quick reply draft is discarded', async () => {
    const inboxThread = thread({
      has_draft: true,
      preview: 'test',
    })
    const inboxMessage = thread({
      id: 'acc:inbox:thread:1#101',
      folder_id: 'inbox',
      message_id: 'root@example.com',
    })
    const draftMessage = thread({
      id: 'acc:Drafts:thread:1#201',
      folder_id: 'Drafts',
      // IMAP commonly returns the header form while the identity allocated for
      // save/discard is bare. They identify the same draft.
      message_id: '<DRAFT-ID@example.com>',
      body: 'test',
    })
    mail$.threads.set([inboxThread])
    mail$.messages.set([inboxMessage, draftMessage])
    responses['mail.discardDraft'] = { ok: true, deleted: 1, permanent: true }
    responses['mail.threadList'] = { threads: [{ ...inboxThread, has_draft: true }] }

    await discardSavedDraftCopy({
      threadId: 'acc:inbox:thread:1',
      messageId: '',
      folderId: '',
      accountId: 'acc',
      draftMessageId: 'draft-id@example.com',
    })

    expect(mail$.messages.get()).toEqual([inboxMessage])
    expect(mail$.threads.get()[0]).toMatchObject({ has_draft: false, preview: 'test' })
  })

  // With a board up the mail list is off screen and its reload steps out, but
  // the board's column still shows the card. A reply sent from the board's pane
  // left that card counting the discarded draft until some later sync happened
  // to touch the column.
  it('re-reads the kanban column holding the thread when its draft is discarded', async () => {
    const inboxThread = thread({ has_draft: true, message_count: 3 })
    settings$.kanbanBoards.set([{ id: 'b1', name: 'Board', columns: [{ accountId: 'acc', folderId: 'inbox' }] }])
    kanban$.activeBoardId.set('b1')
    kanban$.threads.set({ 'acc\ninbox': [inboxThread] })
    mail$.threads.set([])
    mail$.messages.set([thread({ id: 'acc:inbox:thread:1#101', message_id: 'root@example.com' })])
    responses['mail.discardDraft'] = { ok: true, deleted: 1, permanent: true }
    responses['mail.threadList'] = { threads: [{ ...inboxThread, has_draft: false, message_count: 2 }] }

    try {
      await discardSavedDraftCopy({
        threadId: 'acc:inbox:thread:1',
        messageId: '',
        folderId: '',
        accountId: 'acc',
        draftMessageId: 'draft-id@example.com',
      })

      expect(calls.find((call) => call.command === 'mail.threadList')?.payload).toMatchObject({
        account_id: 'acc',
        folder_id: 'inbox',
        refresh: false,
      })
      expect(kanban$.threads['acc\ninbox'].get()[0]).toMatchObject({ has_draft: false, message_count: 2 })
    } finally {
      kanban$.activeBoardId.set('')
      settings$.kanbanBoards.set([])
    }
  })

  // The draft was the only loaded message, so its card leaves the column ahead
  // of the round-trip. The column still has to be re-read: the server may hold
  // older messages of the thread the pane never loaded, and the card comes back.
  it('still re-reads the column when the discard emptied the card out of it', async () => {
    const inboxThread = thread({ has_draft: true, message_count: 2 })
    settings$.kanbanBoards.set([{ id: 'b1', name: 'Board', columns: [{ accountId: 'acc', folderId: 'inbox' }] }])
    kanban$.activeBoardId.set('b1')
    kanban$.threads.set({ 'acc\ninbox': [inboxThread] })
    mail$.threads.set([])
    mail$.messages.set([
      thread({ id: 'acc:Drafts:thread:1#201', folder_id: 'Drafts', message_id: '<draft-id@example.com>' }),
    ])
    responses['mail.discardDraft'] = { ok: true, deleted: 1, permanent: true }
    responses['mail.threadList'] = { threads: [{ ...inboxThread, has_draft: false, message_count: 1 }] }

    try {
      await discardSavedDraftCopy({
        threadId: 'acc:inbox:thread:1',
        messageId: '',
        folderId: '',
        accountId: 'acc',
        draftMessageId: 'draft-id@example.com',
      })

      expect(calls.some((call) => call.command === 'mail.threadList')).toBe(true)
      expect(kanban$.threads['acc\ninbox'].get()).toHaveLength(1)
    } finally {
      kanban$.activeBoardId.set('')
      settings$.kanbanBoards.set([])
    }
  })
})
