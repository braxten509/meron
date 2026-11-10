import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import type { Message } from '../types'
import './accounts'
import { kanban$ } from './kanban'
import { mail$, releaseRemovedThread } from './mail'
import { markUnreadWithUndo } from './mailFlags'
import { BULK_GROUP_SIZE, bulkProgress$, requestBulkStop } from './bulkProgress'
import {
  archiveThread,
  bulkDeleteSelected,
  bulkMoveSelectedToFolder,
  copyThreadToFolder,
  deleteThread,
  moveThreadToFolder,
} from './mailMoves'
import { ui$ } from './ui'
import { recentThreadTransfers$ } from './recentThreadTransfers'

const THREAD = 'acc#INBOX#t.dG9waWM'
// One full group of a bulk action, and two conversations of the next group.
const FIRST_GROUP = Array.from({ length: BULK_GROUP_SIZE }, (_, n) => `g${n}`)
const NEXT_GROUP = ['late1', 'late2']

const thread = (overrides: Partial<Message> = {}): Message => ({
  id: `${THREAD}#7`,
  account_id: 'acc',
  folder_id: 'INBOX',
  thread_id: THREAD,
  from_name: 'Sender',
  from_addr: 'sender@example.com',
  to: 'me@example.com',
  subject: 'Subject',
  preview: '',
  body: '',
  date: 1,
  unread: false,
  starred: false,
  has_attachments: false,
  ...overrides,
})

// Undo moves back exactly the copies the action reported (`target_uids`), so
// the undo's mail.move must carry them as message_ids — resolving the thread
// in the target instead would also take older mail of the conversation there,
// and finds nothing for a uid: key.
describe('undo after moving a thread', () => {
  let previousGo: unknown
  let calls: Array<{ command: string; payload: any }> = []
  let responses: Record<string, unknown> = {}

  beforeEach(() => {
    previousGo = (window as any).go
    calls = []
    responses = {}
    recentThreadTransfers$.set([])
    ui$.selectedAccount.set('acc')
    ui$.selectedFolder.set('INBOX')
    ui$.selectedThread.set('')
    ui$.toast.set('')
    ui$.toastUndo.set(null)
    mail$.threads.set([thread()])
    mail$.messages.set([])
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: any) => {
            calls.push({ command, payload })
            if (command in responses) {
              const response = responses[command]
              if (response instanceof Error) throw response
              return response
            }
            return { ok: true }
          },
        },
      },
    }
  })

  afterEach(() => {
    recentThreadTransfers$.set([])
    for (const id of [THREAD, 'a', 'b', 'missing', ...FIRST_GROUP, ...NEXT_GROUP]) releaseRemovedThread(id)
    ;(window as any).go = previousGo
    kanban$.activeBoardId.set('')
    kanban$.paneThreadId.set('')
    kanban$.paneColumnKey.set('')
    kanban$.threads.set({})
  })

  async function runUndo() {
    const undo = ui$.toastUndo.get() as (() => void) | null
    expect(typeof undo).toBe('function')
    calls = []
    undo?.()
    await new Promise((resolve) => setTimeout(resolve, 0))
    return calls.find((call) => call.command === 'mail.move')?.payload
  }

  it('remembers transfers for threads outside the loaded list using their encoded account', async () => {
    mail$.threads.set([])
    responses['mail.move'] = { moved: 1 }
    await moveThreadToFolder(THREAD, 'Work')
    expect(recentThreadTransfers$.get()[0]).toEqual({
      action: 'move',
      sourceAccountId: 'acc',
      targetAccountId: 'acc',
      targetFolderId: 'Work',
    })
    responses['mail.copy'] = { copied: 1 }
    await copyThreadToFolder(THREAD, 'other', 'Receipts')
    expect(recentThreadTransfers$.get()[0]).toEqual({
      action: 'copy',
      sourceAccountId: 'acc',
      targetAccountId: 'other',
      targetFolderId: 'Receipts',
    })
  })

  it('records successful transfers but not failed transfers or undo destinations', async () => {
    responses['mail.move'] = { moved: 1, target_uids: [5] }
    await moveThreadToFolder(THREAD, 'Work')
    expect(recentThreadTransfers$.get()).toEqual([
      { action: 'move', sourceAccountId: 'acc', targetAccountId: 'acc', targetFolderId: 'Work' },
    ])
    await runUndo()
    expect(recentThreadTransfers$.get()).toHaveLength(1)

    mail$.threads.set([thread()])
    responses['mail.copy'] = { copied: 1 }
    await copyThreadToFolder(THREAD, 'other', 'Receipts')
    expect(recentThreadTransfers$.get()[0]).toEqual({
      action: 'copy',
      sourceAccountId: 'acc',
      targetAccountId: 'other',
      targetFolderId: 'Receipts',
    })
    responses['mail.copy'] = new Error('Rejected')
    await copyThreadToFolder(THREAD, 'other', 'Failed')
    responses['mail.move'] = { moved: 0 }
    await moveThreadToFolder(THREAD, 'Failed')
    expect(recentThreadTransfers$.get()).toHaveLength(2)
  })

  it('moves to another account even when the destination folder has the same name', async () => {
    responses['mail.move'] = { moved: 1 }
    expect(await moveThreadToFolder(THREAD, 'INBOX', { targetAccountId: 'other' })).toBe(true)
    expect(calls.find((call) => call.command === 'mail.move')?.payload).toEqual({
      thread_id: THREAD,
      target_folder_id: 'INBOX',
      target_account_id: 'other',
    })
    expect(recentThreadTransfers$.get()[0]).toEqual({
      action: 'move',
      sourceAccountId: 'acc',
      targetAccountId: 'other',
      targetFolderId: 'INBOX',
    })
    expect(ui$.toastUndo.get()).toBe(null)
  })

  it('restores the source after a cross-account removal failure and records no successful move', async () => {
    responses['mail.move'] = new Error('Move copied messages but could not confirm removal from the source folder')
    expect(await moveThreadToFolder(THREAD, 'INBOX', { targetAccountId: 'other' })).toBe(false)
    expect(mail$.threads.get().some((item) => item.thread_id === THREAD)).toBe(true)
    expect(recentThreadTransfers$.get()).toEqual([])
    expect(ui$.toast.get()).toContain('copied')
  })

  it('bulk moves same-named folders across accounts and skips copies already at the destination', async () => {
    responses['mail.moveMany'] = { results: [{ moved: 1 }] }
    await bulkMoveSelectedToFolder(
      [
        {
          kind: 'mail',
          key: THREAD,
          groupKey: '',
          surface: 'thread-list',
          unread: false,
          starred: false,
          draft: false,
          trash: false,
          threadId: THREAD,
          accountId: 'acc',
          folderId: 'INBOX',
        },
        {
          kind: 'mail',
          key: 'other',
          groupKey: '',
          surface: 'thread-list',
          unread: false,
          starred: false,
          draft: false,
          trash: false,
          threadId: 'other#INBOX#t.dG9waWM',
          accountId: 'other',
          folderId: 'INBOX',
        },
      ],
      'INBOX',
      'other',
    )
    // One call for the whole group, with one item per conversation.
    expect(calls.filter((call) => call.command === 'mail.moveMany').map((call) => call.payload)).toEqual([
      { items: [{ thread_id: THREAD, target_folder_id: 'INBOX', target_account_id: 'other' }] },
    ])
  })

  it('moves the reported Trash copies back out of Trash', async () => {
    responses['mail.delete'] = { deleted: 1, trash: 'Trash', thread_id: 'acc#Trash#t.dG9waWM', target_uids: [42, 43] }
    await deleteThread(THREAD)

    expect(await runUndo()).toEqual({
      thread_id: 'acc#Trash#t.dG9waWM',
      target_folder_id: 'INBOX',
      message_ids: ['42', '43'],
      // An Undo says so: the core then moves back exactly these copies.
      undo: true,
    })
  })

  // Without the exact copies there is no safe way back: the thread key in
  // Trash would also take older mail of the conversation.
  it('offers no undo when the copies are unknown', async () => {
    responses['mail.delete'] = { deleted: 1, trash: 'Trash', thread_id: 'acc#Trash#t.dG9waWM', target_uids: [] }
    await deleteThread(THREAD)
    expect(ui$.toastUndo.get()).toBe(null)
    expect(ui$.toast.get()).not.toBe('')

    mail$.threads.set([thread()])
    responses['mail.archive'] = { moved: 1, folder: 'Archive', thread_id: 'acc#Archive#t.dG9waWM' }
    await archiveThread(THREAD)
    expect(ui$.toastUndo.get()).toBe(null)

    mail$.threads.set([thread()])
    responses['mail.move'] = { moved: 1, target_uids: [] }
    await moveThreadToFolder(THREAD, 'Work')
    expect(ui$.toastUndo.get()).toBe(null)
  })

  it('moves the reported archive copies back', async () => {
    responses['mail.archive'] = { moved: 1, folder: 'Archive', thread_id: 'acc#Archive#t.dG9waWM', target_uids: [9] }
    await archiveThread(THREAD)

    expect(await runUndo()).toEqual({
      thread_id: 'acc#Archive#t.dG9waWM',
      target_folder_id: 'INBOX',
      message_ids: ['9'],
      // An Undo says so: the core then moves back exactly these copies.
      undo: true,
    })
  })

  it('moves the reported copies back from a folder', async () => {
    responses['mail.move'] = { moved: 1, target_uids: [5] }
    await moveThreadToFolder(THREAD, 'Work')

    expect(await runUndo()).toEqual({
      thread_id: 'acc#Work#t.dG9waWM',
      target_folder_id: 'INBOX',
      message_ids: ['5'],
      // An Undo says so: the core then moves back exactly these copies.
      undo: true,
    })
  })

  it('shows the server reason when Wails rejects a copy with a string', async () => {
    ;(window as any).go.main.App.Invoke = async () => {
      throw 'sidecar messages.copy error: [LIMIT] Mail size is larger than the maximum size allowed by server configuration'
    }
    await copyThreadToFolder(THREAD, 'other', 'INBOX')
    expect(ui$.toast.get()).toContain('destination server rejected a message')
    expect(ui$.toast.get()).not.toContain('sidecar')
  })

  it('keeps completed bulk moves removed and reports partial success', async () => {
    mail$.threads.set([thread({ thread_id: 'a' }), thread({ thread_id: 'b' })])
    // The group is one call; each conversation gets its own answer.
    ;(window as any).go.main.App.Invoke = (command: string, payload: { items?: { thread_id?: string }[] }) => {
      if (command === 'mail.moveMany') {
        return Promise.resolve({
          results: (payload.items ?? []).map((item) =>
            item.thread_id === 'a' ? { moved: 1 } : { error: '[NOPERM] permission denied' },
          ),
        })
      }
      return Promise.resolve({ threads: mail$.threads.get() })
    }
    await bulkMoveSelectedToFolder(
      ['a', 'b'].map((threadId) => ({
        key: threadId,
        groupKey: '',
        threadId,
        accountId: 'acc',
        folderId: 'INBOX',
        surface: 'thread-list',
        kind: 'mail',
        unread: false,
        starred: false,
        draft: false,
        trash: false,
      })),
      'Work',
    )
    expect(mail$.threads.get().map((item) => item.thread_id)).toEqual(['b'])
    expect(ui$.toast.get()).toContain('denied access')
    expect(ui$.toast.get()).toContain('Completed 1 of 2 selected threads')
  })

  // The strip's Stop and a failure part-way share one rule: conversations the
  // job did not get done stay exactly where they were, and the reader is told
  // how far it got. A bulk delete goes to the core a group at a time, so Stop
  // acts between groups; inside a group each conversation has its own answer.
  it('bulk delete leaves the unreached threads alone after Stop and after a failure', async () => {
    const items = (ids: string[]) =>
      ids.map((threadId) => ({
        key: threadId,
        groupKey: '',
        threadId,
        accountId: 'acc',
        folderId: 'INBOX',
        surface: 'thread-list' as const,
        kind: 'mail' as const,
        unread: false,
        starred: false,
        draft: false,
        trash: false,
      }))
    const groups: string[][] = []
    const deleted: string[] = []
    let failOn = ''
    let stopAfter = ''
    const seen: { whileRunning?: { done: number; pending: string[] } } = {}
    ;(window as any).go.main.App.Invoke = (command: string, payload: { items?: { thread_id?: string }[] }) => {
      if (command !== 'mail.deleteMany') return Promise.resolve({ threads: mail$.threads.get() })
      const ids = (payload.items ?? []).map((item) => item.thread_id ?? '')
      groups.push(ids)
      const job = bulkProgress$.job.get()
      if (ids.includes('b') && job) seen.whileRunning = { done: job.done, pending: Object.keys(job.pending) }
      if (ids.includes(stopAfter)) requestBulkStop()
      return Promise.resolve({
        results: ids.map((id) => {
          if (id === failOn) return { error: 'server said no' }
          deleted.push(id)
          return { deleted: 1 }
        }),
      })
    }

    const many = [...FIRST_GROUP, ...NEXT_GROUP]
    mail$.threads.set(many.map((id) => thread({ id, thread_id: id })))
    stopAfter = FIRST_GROUP[0]
    await bulkDeleteSelected(items(many))
    expect(groups).toEqual([FIRST_GROUP])
    expect(deleted).toEqual(FIRST_GROUP)
    expect(mail$.threads.get().map((item) => item.thread_id)).toEqual(NEXT_GROUP)
    expect(ui$.toast.get()).toBe(`Deleted ${FIRST_GROUP.length} of ${many.length}. Stopped.`)
    expect(bulkProgress$.job.get()).toBeNull()

    groups.length = 0
    deleted.length = 0
    stopAfter = ''
    failOn = 'missing'
    mail$.threads.set(['a', 'b', 'missing'].map((id) => thread({ id, thread_id: id })))
    await bulkDeleteSelected(items(['a', 'b', 'missing']))
    expect(groups).toEqual([['a', 'b', 'missing']])
    expect(deleted).toEqual(['a', 'b'])
    expect(seen.whileRunning).toEqual({ done: 0, pending: ['a', 'b', 'missing'] })
    expect(mail$.threads.get().map((item) => item.thread_id)).toEqual(['missing'])
    expect(ui$.toast.get()).toContain('server said no')
    expect(ui$.toast.get()).toContain('Completed 2 of 3 selected threads')
    expect(bulkProgress$.job.get()).toBeNull()
  })

  it('keeps later archives when an earlier one fails', async () => {
    const older = thread({ id: 'c', thread_id: 'c', date: 10 })
    const middle = thread({ id: 'b', thread_id: 'b', date: 20 })
    const newest = thread({ id: 'a', thread_id: 'a', date: 30 })
    mail$.threads.set([newest, middle, older])
    ui$.selectedThread.set('a')
    let rejectFirst: (error: Error) => void = () => {}
    ;(window as any).go = {
      main: {
        App: {
          Invoke: (command: string, payload: { thread_id?: string }) => {
            if (command === 'mail.archive' && payload.thread_id === 'a') {
              return new Promise((_resolve, reject) => {
                rejectFirst = reject
              })
            }
            return new Promise(() => {})
          },
        },
      },
    }

    const first = archiveThread('a')
    void archiveThread('b')
    await Promise.resolve()
    expect(mail$.threads.get().map((item) => item.thread_id)).toEqual(['c'])
    expect(ui$.selectedThread.get()).toBe('c')

    rejectFirst(new Error('archive failed'))
    await first
    expect(mail$.threads.get().map((item) => item.thread_id)).toEqual(['a', 'c'])
    expect(ui$.selectedThread.get()).toBe('c')
  })

  it('stays on a card when the archived thread is already off the board', async () => {
    const next = thread({ id: 'b', thread_id: 'b', date: 20, account_id: 'acc', folder_id: 'INBOX' })
    kanban$.activeBoardId.set('board')
    kanban$.paneColumnKey.set('board\nacc\nINBOX')
    kanban$.paneThreadId.set('missing')
    kanban$.threads['acc\nINBOX'].set([next, thread({ id: 'c', thread_id: 'c', date: 10 })])
    ui$.selectedThread.set('missing')
    mail$.threads.set([])
    ;(window as any).go = {
      main: {
        App: {
          Invoke: () => new Promise(() => {}),
        },
      },
    }

    void archiveThread('missing')
    await Promise.resolve()
    expect(ui$.selectedThread.get()).toBe('b')
    expect(kanban$.paneThreadId.get()).toBe('b')
  })

  it('stays on a visible row when the archived thread is already off the list', async () => {
    const next = thread({ id: 'b', thread_id: 'b', date: 20 })
    mail$.threads.set([next, thread({ id: 'c', thread_id: 'c', date: 10 })])
    ui$.selectedThread.set('missing')
    ;(window as any).go = {
      main: {
        App: {
          Invoke: () => new Promise(() => {}),
        },
      },
    }

    void archiveThread('missing')
    await Promise.resolve()
    expect(ui$.selectedThread.get()).toBe('b')
  })

  it('offers undo for mark unread only once it has landed', async () => {
    markUnreadWithUndo(THREAD)
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(typeof ui$.toastUndo.get()).toBe('function')

    ui$.toastUndo.set(null)
    mail$.threads.set([thread()])
    responses['mail.markRead'] = new Error('server refused')
    markUnreadWithUndo(THREAD)
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(ui$.toastUndo.get()).toBe(null)
    expect(ui$.toast.get()).toBe('server refused')
    expect(ui$.toastTone.get()).toBe('error')
  })
})
