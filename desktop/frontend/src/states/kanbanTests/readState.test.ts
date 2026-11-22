// Marking a column or board read while folder totals are updating.
// The fixtures exercise overlapping rows, in-flight writes and failures.
// The displayed unread state must settle to the result accepted by the core.
import { beforeEach, describe, expect, it } from 'bun:test'
import { accounts$ } from '../accounts'
import { kanban$, markColumnAllRead, markBoardAllRead } from '../kanban'
import { mail$ } from '../mail'
import {
  applyMutationFolderUnreads,
  holdFolderUnread,
  refreshAccountFoldersCache,
  updateCachedFolderUnread,
} from '../mailFolders'
import { settings$ } from '../settings'
import { ui$ } from '../ui'
import { message } from './fixtures'

describe('markColumnAllRead', () => {
  const calls: { command: string; payload: any }[] = []

  beforeEach(() => {
    calls.length = 0
    accounts$.set([
      {
        id: 'acc1',
        email: 'one@example.com',
        display_name: 'One',
        provider: 'custom',
        auth_type: 'password',
        imap_host: '',
        imap_port: 993,
        smtp_host: '',
        smtp_port: 465,
        tls: true,
      },
      {
        id: 'acc2',
        email: 'two@example.com',
        display_name: 'Two',
        provider: 'custom',
        auth_type: 'password',
        imap_host: '',
        imap_port: 993,
        smtp_host: '',
        smtp_port: 465,
        tls: true,
      },
    ])
    kanban$.threads.set({})
    kanban$.unreadCounts.set({})
    mail$.foldersByAccount.set({})
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: any) => {
            calls.push({ command, payload })
            if (command === 'mail.folderList') {
              return {
                folders: [{ id: 'INBOX', account_id: payload.account_id, name: 'Inbox', role: 'inbox', unread: 0 }],
              }
            }
            return { ok: true }
          },
        },
      },
    }
  })

  it('holds synthesized lowercase inbox counts against uppercase folder refreshes', async () => {
    updateCachedFolderUnread('acc1', 'inbox', 6)
    const settle = holdFolderUnread('acc1', 'inbox')
    updateCachedFolderUnread('acc1', 'inbox', 0, 'local')
    ;(window as any).go.main.App.Invoke = async () => ({
      folders: [{ id: 'INBOX', account_id: 'acc1', name: 'Inbox', role: 'inbox', unread: 6 }],
    })
    await refreshAccountFoldersCache('acc1')
    expect(mail$.foldersByAccount.acc1.get()[0].unread).toBe(0)
    settle(0, true)
  })

  it('publishes a failed column rollback while a sibling still holds the folder', () => {
    updateCachedFolderUnread('acc1', 'inbox', 0)
    const failed = holdFolderUnread('acc1', 'inbox')
    const sibling = holdFolderUnread('acc1', 'INBOX')
    failed(6, false)
    expect(mail$.foldersByAccount.acc1.get()[0].unread).toBe(6)
    sibling(6, false)
    expect(mail$.foldersByAccount.acc1.get()[0].unread).toBe(6)
  })

  it('preserves confirmed counts when a later sibling cannot refresh after failure', () => {
    updateCachedFolderUnread('acc1', 'inbox', 0)
    const succeeded = holdFolderUnread('acc1', 'inbox')
    const failed = holdFolderUnread('acc1', 'INBOX')
    succeeded(2, true)
    failed(6, false)
    expect(mail$.foldersByAccount.acc1.get()[0].unread).toBe(2)
  })

  it('lets a post-write refresh supersede a mutation observed before that refresh started', async () => {
    updateCachedFolderUnread('acc1', 'inbox', 6)
    let release = () => {}
    const gate = new Promise<void>((resolve) => (release = resolve))
    ;(window as any).go.main.App.Invoke = async (command: string) => {
      if (command === 'mail.markAllRead') await gate
      if (command === 'mail.folderList') {
        return { folders: [{ id: 'INBOX', account_id: 'acc1', name: 'Inbox', role: 'inbox', unread: 0 }] }
      }
      return { ok: true }
    }
    const pending = markColumnAllRead({ accountId: 'acc1', folderId: 'inbox' })
    applyMutationFolderUnreads({ folder_unreads: { acc1: { INBOX: 5 } } })
    expect(mail$.foldersByAccount.acc1.get()[0].unread).toBe(5)
    release()
    await pending
    expect(mail$.foldersByAccount.acc1.get()[0].unread).toBe(0)
  })

  it('keeps a concurrent mutation result when a column settles with an older refresh', async () => {
    updateCachedFolderUnread('acc1', 'inbox', 6)
    let release = () => {}
    let started = () => {}
    const gate = new Promise<void>((resolve) => (release = resolve))
    const refreshing = new Promise<void>((resolve) => (started = resolve))
    ;(window as any).go.main.App.Invoke = async (command: string) => {
      if (command === 'mail.folderList') {
        started()
        await gate
        return { folders: [{ id: 'INBOX', account_id: 'acc1', name: 'Inbox', role: 'inbox', unread: 3 }] }
      }
      return { ok: true }
    }
    const pending = markColumnAllRead({ accountId: 'acc1', folderId: 'inbox' })
    await refreshing
    applyMutationFolderUnreads({ folder_unreads: { acc1: { INBOX: 1 } } })
    expect(mail$.foldersByAccount.acc1.get()[0].unread).toBe(1)
    release()
    await pending
    expect(mail$.foldersByAccount.acc1.get()[0].unread).toBe(1)
  })

  it('keeps navigation counts cleared when stale folder and thread refreshes race a write', async () => {
    const folder = { id: 'INBOX', account_id: 'acc1', name: 'Inbox', role: 'inbox', unread: 6 }
    mail$.foldersByAccount.set({ acc1: [folder] })
    settings$.kanbanBoards.set([{ id: 'board', name: 'Board', columns: [{ accountId: 'acc1', folderId: 'INBOX' }] }])
    let releaseWrite = () => {}
    let releaseStale = () => {}
    const writeGate = new Promise<void>((resolve) => (releaseWrite = resolve))
    const staleGate = new Promise<void>((resolve) => (releaseStale = resolve))
    let written = false
    let delayRefresh = false
    ;(window as any).go.main.App.Invoke = async (command: string) => {
      if (command === 'mail.markAllRead') {
        await writeGate
        written = true
      }
      if (command === 'mail.folderList') {
        const unread = written ? 0 : 6
        if (delayRefresh) await staleGate
        return { folders: [{ ...folder, unread }] }
      }
      return { ok: true }
    }

    const pending = markBoardAllRead('board')
    expect(mail$.foldersByAccount.acc1.get()[0].unread).toBe(0)
    await refreshAccountFoldersCache('acc1')
    updateCachedFolderUnread('acc1', 'inbox', 6)
    expect(mail$.foldersByAccount.acc1.get()[0].unread).toBe(0)

    delayRefresh = true
    const stale = refreshAccountFoldersCache('acc1')
    delayRefresh = false
    releaseWrite()
    await pending
    releaseStale()
    await stale
    expect(mail$.foldersByAccount.acc1.get()[0].unread).toBe(0)

    // Once the operation is over, fresh incoming mail can raise the badge.
    updateCachedFolderUnread('acc1', 'inbox', 1)
    expect(mail$.foldersByAccount.acc1.get()[0].unread).toBe(1)
  })

  it('marks all board columns including unloaded and minimized folders, deduplicating unified overlap', async () => {
    settings$.kanbanBoards.set([
      {
        id: 'board',
        name: 'Board',
        columns: [
          { accountId: 'acc1', folderId: 'INBOX' },
          { accountId: 'unified', folderId: 'inbox' },
          { accountId: 'acc2', folderId: 'Archive' },
          { accountId: 'acc2', folderId: 'Archive' },
        ],
      },
      { id: 'other', name: 'Other', columns: [{ accountId: 'acc1', folderId: 'Trash' }] },
    ])
    settings$.kanbanMinimizedColumns.set({ 'board\nacc2\nArchive': true })
    await markBoardAllRead('board')
    expect(calls.filter((call) => call.command === 'mail.markAllRead').map((call) => call.payload)).toEqual([
      { account_id: 'unified', folder_id: 'inbox' },
      { account_id: 'acc2', folder_id: 'Archive' },
    ])
  })

  it('marks only the matches of a saved search, never widening an account folder write to unified', async () => {
    settings$.kanbanBoards.set([
      {
        id: 'board',
        name: 'Board',
        columns: [
          { accountId: 'acc1', folderId: 'INBOX' },
          { accountId: 'unified', folderId: 'inbox', query: 'invoice' },
        ],
      },
    ])
    kanban$.threads['unified\ninbox\ninvoice'].set([message({ thread_id: 'match' })])
    await markBoardAllRead('board')
    expect(calls.filter((call) => call.command === 'mail.markAllRead').map((call) => call.payload)).toEqual([
      { account_id: 'acc1', folder_id: 'INBOX' },
    ])
    expect(calls.filter((call) => call.command === 'mail.markRead').map((call) => call.payload)).toEqual([
      { thread_id: 'match' },
    ])
  })

  it('clears every column before the backend answers', async () => {
    settings$.kanbanBoards.set([
      {
        id: 'board',
        name: 'Board',
        columns: [
          { accountId: 'acc1', folderId: 'INBOX' },
          { accountId: 'acc2', folderId: 'Archive' },
        ],
      },
    ])
    kanban$.threads['acc1\nINBOX'].set([message()])
    kanban$.unreadCounts['acc1\nINBOX'].set(3)
    kanban$.threads['acc2\nArchive'].set([message({ account_id: 'acc2', folder_id: 'Archive' })])
    let release = () => {}
    const gate = new Promise<void>((resolve) => (release = resolve))
    const original = (window as any).go.main.App.Invoke
    ;(window as any).go.main.App.Invoke = async (command: string, payload: any) => {
      if (command === 'mail.markAllRead') await gate
      return original(command, payload)
    }

    const pending = markBoardAllRead('board')

    expect(kanban$.threads['acc1\nINBOX'].get()[0].unread).toBe(false)
    expect(kanban$.unreadCounts['acc1\nINBOX'].get()).toBe(0)
    expect(kanban$.threads['acc2\nArchive'].get()[0].unread).toBe(false)
    // A column reload racing the write brings back pre-write rows and badge.
    kanban$.threads['acc1\nINBOX'].set([message()])
    kanban$.unreadCounts['acc1\nINBOX'].set(3)
    kanban$.threads['acc2\nArchive'].set([message({ account_id: 'acc2', folder_id: 'Archive' })])
    release()
    await pending
    expect(kanban$.unreadCounts['acc1\nINBOX'].get()).toBe(0)
    expect(kanban$.threads['acc1\nINBOX'].get()[0].unread).toBe(false)
    expect(kanban$.threads['acc2\nArchive'].get()[0].unread).toBe(false)
  })

  it('keeps failed columns unread while completing other columns and reporting a partial failure', async () => {
    settings$.kanbanBoards.set([
      {
        id: 'board',
        name: 'Board',
        columns: [
          { accountId: 'unified', folderId: 'inbox' },
          { accountId: 'acc2', folderId: 'Archive' },
        ],
      },
    ])
    kanban$.threads['unified\ninbox'].set([message()])
    kanban$.unreadCounts['unified\ninbox'].set(4)
    kanban$.threads['acc2\nArchive'].set([message({ account_id: 'acc2', folder_id: 'Archive' })])
    const original = (window as any).go.main.App.Invoke
    ;(window as any).go.main.App.Invoke = async (command: string, payload: any) => {
      if (command === 'mail.markAllRead' && payload.account_id === 'unified') {
        return { ok: false, failures: [{ account_id: 'acc1', message: 'Offline' }] }
      }
      return original(command, payload)
    }
    await markBoardAllRead('board')
    expect(kanban$.threads['unified\ninbox'].get()[0].unread).toBe(true)
    expect(kanban$.unreadCounts['unified\ninbox'].get()).toBe(4)
    expect(kanban$.threads['acc2\nArchive'].get()[0].unread).toBe(false)
    expect(ui$.toastTone.get()).toBe('error')
  })

  it('refreshes affected account folder caches after marking a kanban column read', async () => {
    kanban$.threads['unified\ninbox'].set([
      message({ account_id: 'acc1', thread_id: 'acc1#INBOX#t1' }),
      message({ id: 'acc2:INBOX:t2#1', account_id: 'acc2', thread_id: 'acc2#INBOX#t2' }),
    ])
    kanban$.unreadCounts['unified\ninbox'].set(2)

    await markColumnAllRead({ accountId: 'unified', folderId: 'inbox' })

    expect(calls.filter((call) => call.command === 'mail.markAllRead').map((call) => call.payload)).toEqual([
      { account_id: 'unified', folder_id: 'inbox' },
    ])
    expect(calls.filter((call) => call.command === 'mail.folderList').map((call) => call.payload)).toEqual([
      { account_id: 'acc1', refresh: false },
      { account_id: 'acc2', refresh: false },
    ])
    expect(mail$.foldersByAccount.acc1.get()?.[0]?.unread).toBe(0)
    expect(mail$.foldersByAccount.acc2.get()?.[0]?.unread).toBe(0)
    expect(kanban$.unreadCounts['unified\ninbox'].get()).toBe(0)
  })

  it('clears the side navigation folder badges before the backend answers', async () => {
    mail$.foldersByAccount.set({
      acc1: [{ id: 'INBOX', account_id: 'acc1', name: 'Inbox', role: 'inbox', unread: 5 }],
      acc2: [{ id: 'INBOX', account_id: 'acc2', name: 'Inbox', role: 'inbox', unread: 2 }],
    })
    kanban$.threads['unified\ninbox'].set([message()])
    kanban$.unreadCounts['unified\ninbox'].set(7)
    let release = () => {}
    const gate = new Promise<void>((resolve) => (release = resolve))
    const original = (window as any).go.main.App.Invoke
    ;(window as any).go.main.App.Invoke = async (command: string, payload: any) => {
      if (command === 'mail.markAllRead') await gate
      return original(command, payload)
    }

    const pending = markColumnAllRead({ accountId: 'unified', folderId: 'inbox' })

    expect(mail$.foldersByAccount.acc1.get()?.[0]?.unread).toBe(0)
    expect(mail$.foldersByAccount.acc2.get()?.[0]?.unread).toBe(0)
    release()
    await pending
  })

  it('puts the side navigation folder badges back when the write fails', async () => {
    mail$.foldersByAccount.set({
      acc1: [{ id: 'INBOX', account_id: 'acc1', name: 'Inbox', role: 'inbox', unread: 5 }],
    })
    kanban$.threads['acc1\nINBOX'].set([message()])
    kanban$.unreadCounts['acc1\nINBOX'].set(5)
    ;(window as any).go.main.App.Invoke = async (command: string, payload: any) => {
      calls.push({ command, payload })
      if (command === 'mail.markAllRead') return { ok: false, failures: [{ message: 'Offline' }] }
      // The failed write left the folder unread on the server too.
      return { folders: [{ id: 'INBOX', account_id: payload.account_id, name: 'Inbox', role: 'inbox', unread: 5 }] }
    }

    await markColumnAllRead({ accountId: 'acc1', folderId: 'INBOX' })

    expect(mail$.foldersByAccount.acc1.get()?.[0]?.unread).toBe(5)
    expect(kanban$.unreadCounts['acc1\nINBOX'].get()).toBe(5)
  })

  // A board-wide mark shares one write between overlapping columns, so a single
  // failure rolls back both — and the second column would otherwise have
  // snapshotted the zero the first one had already written.
  it('rolls overlapping board columns back to the count the folder had before the board cleared', async () => {
    settings$.kanbanBoards.set([
      {
        id: 'board',
        name: 'Board',
        columns: [
          { accountId: 'unified', folderId: 'inbox' },
          { accountId: 'acc1', folderId: 'INBOX' },
        ],
      },
    ])
    mail$.foldersByAccount.set({
      acc1: [{ id: 'INBOX', account_id: 'acc1', name: 'Inbox', role: 'inbox', unread: 5 }],
    })
    kanban$.threads['unified\ninbox'].set([message()])
    kanban$.threads['acc1\nINBOX'].set([message()])
    ;(window as any).go.main.App.Invoke = async (command: string, payload: any) => {
      calls.push({ command, payload })
      if (command === 'mail.markAllRead') return { ok: false, failures: [{ message: 'Offline' }] }
      return { folders: [{ id: 'INBOX', account_id: payload.account_id, name: 'Inbox', role: 'inbox', unread: 5 }] }
    }

    await markBoardAllRead('board')

    expect(mail$.foldersByAccount.acc1.get()?.[0]?.unread).toBe(5)
  })

  // The refresh that follows the writes answers with the server's own counts, so
  // a folder a sibling write did mark read keeps that count instead of being
  // rolled back with the column that failed.
  it('keeps the refreshed count for a folder the failed column did not leave unread', async () => {
    mail$.foldersByAccount.set({
      acc1: [{ id: 'INBOX', account_id: 'acc1', name: 'Inbox', role: 'inbox', unread: 5 }],
      acc2: [{ id: 'INBOX', account_id: 'acc2', name: 'Inbox', role: 'inbox', unread: 3 }],
    })
    kanban$.threads['unified\ninbox'].set([message()])
    ;(window as any).go.main.App.Invoke = async (command: string, payload: any) => {
      calls.push({ command, payload })
      if (command === 'mail.markAllRead') return { ok: false, failures: [{ account_id: 'acc1', message: 'Offline' }] }
      const unread = payload.account_id === 'acc1' ? 5 : 0
      return { folders: [{ id: 'INBOX', account_id: payload.account_id, name: 'Inbox', role: 'inbox', unread }] }
    }

    await markColumnAllRead({ accountId: 'unified', folderId: 'inbox' })

    expect(mail$.foldersByAccount.acc1.get()?.[0]?.unread).toBe(5)
    expect(mail$.foldersByAccount.acc2.get()?.[0]?.unread).toBe(0)
  })

  it('sends a unified non-inbox role through the role-resolving backend path', async () => {
    kanban$.threads['unified\nsent'].set([message({ folder_id: '[Gmail]/Sent Mail' })])

    await markColumnAllRead({ accountId: 'unified', folderId: 'sent' })

    expect(calls.filter((call) => call.command === 'mail.markAllRead').map((call) => call.payload)).toEqual([
      { account_id: 'unified', folder_id: 'sent' },
    ])
  })

  it('marks a mail column read even when no unread thread is loaded', async () => {
    kanban$.threads['acc1\ninbox'].set([])

    await markColumnAllRead({ accountId: 'acc1', folderId: 'inbox' })

    expect(calls.filter((call) => call.command === 'mail.markAllRead').map((call) => call.payload)).toEqual([
      { account_id: 'acc1', folder_id: 'inbox' },
    ])
    expect(calls.filter((call) => call.command === 'mail.folderList').map((call) => call.payload)).toEqual([
      { account_id: 'acc1', refresh: false },
    ])
  })
})
