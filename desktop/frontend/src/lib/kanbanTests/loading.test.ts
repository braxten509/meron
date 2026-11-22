// Loading board columns and their continuation pages.
// The fixtures vary folder scope, search filters and account cursors.
// Cases control request order to check when stale data can be reused.
import { beforeEach, describe, expect, it } from 'bun:test'
import { accounts$ } from '../../states/accounts'
import { kanban$ } from '../../states/kanban'
import { mail$, releaseRemovedThread, suppressRemovedThread } from '../../states/mail'
import { ui$ } from '../../states/ui'
import {
  activeKanbanColumnFilter,
  loadKanbanColumn,
  loadMoreKanbanColumn,
  syncFolderAndWait,
  syncKanbanColumn,
} from '../kanbanData'
import { account, resetBoardData, message } from './fixtures'

beforeEach(resetBoardData)

describe('kanban column loading filters', () => {
  it('keeps a manual column sync pending until its completion event', async () => {
    const calls: string[] = []
    const handlers = new Map<string, (detail: unknown) => void>()
    ;(window as any).runtime = {
      EventsOn: (name: string, handler: (detail: unknown) => void) => {
        handlers.set(name, handler)
        return () => handlers.delete(name)
      },
    }
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string) => {
            calls.push(command)
            return command === 'mail.threadList' ? { threads: [], next_cursor: '' } : { online: true, queued: true }
          },
        },
      },
    }

    const sync = syncKanbanColumn({ accountId: 'acc1', folderId: 'INBOX' })
    let finished = false
    void sync.then(() => {
      finished = true
    })
    await Promise.resolve()

    expect(finished).toBe(false)
    expect(calls).toEqual(['mail.sync'])

    handlers.get('mail.synced')?.({ account: 'acc1', folder: 'INBOX' })
    await sync

    expect(calls).toEqual(['mail.sync', 'mail.threadList'])
    expect(finished).toBe(true)
    expect(handlers.size).toBe(0)
  })

  it('holds a background folder sync until that folder has landed', async () => {
    const handlers = new Map<string, (detail: unknown) => void>()
    ;(window as any).runtime = {
      EventsOn: (name: string, handler: (detail: unknown) => void) => {
        handlers.set(name, handler)
        return () => handlers.delete(name)
      },
    }
    ;(window as any).go = { main: { App: { Invoke: async () => ({ online: true, queued: true }) } } }

    let finished = false
    const sync = syncFolderAndWait('acc1', 'Later').then(() => {
      finished = true
    })
    await new Promise((resolve) => setTimeout(resolve))
    expect(finished).toBe(false)

    handlers.get('mail.synced')?.({ account: 'acc1', folder: 'Other' })
    await new Promise((resolve) => setTimeout(resolve))
    expect(finished).toBe(false)

    handlers.get('mail.synced')?.({ account: 'acc1', folder: 'Later' })
    await sync
    expect(handlers.size).toBe(0)
  })

  it('reloads a synced saved search column still narrowed by the board search', async () => {
    const handlers = new Map<string, (detail: unknown) => void>()
    ;(window as any).runtime = {
      EventsOn: (name: string, handler: (detail: unknown) => void) => {
        handlers.set(name, handler)
        return () => handlers.delete(name)
      },
    }
    const queries: string[] = []
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: any) => {
            if (command !== 'mail.threadList') return { online: true, queued: true }
            queries.push(payload.query)
            return { threads: [], next_cursor: '' }
          },
        },
      },
    }
    kanban$.searchQuery.set('receipt')
    kanban$.searchScope.set('all')
    try {
      const sync = syncKanbanColumn({ accountId: 'acc1', folderId: 'INBOX', query: 'invoice' })
      await Promise.resolve()
      handlers.get('mail.synced')?.({ account: 'acc1', folder: 'INBOX' })
      await sync
      expect(queries).toEqual(['invoice\nAND receipt'])
    } finally {
      kanban$.searchQuery.set('')
    }
  })

  it('waits for every account in a unified column before finishing sync', async () => {
    const handlers = new Map<string, (detail: unknown) => void>()
    ;(window as any).runtime = {
      EventsOn: (name: string, handler: (detail: unknown) => void) => {
        handlers.set(name, handler)
        return () => handlers.delete(name)
      },
    }
    accounts$.set([account('acc1'), account('acc2')])
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string) =>
            command === 'mail.threadList'
              ? { threads: [], next_cursor: '', folder_unreads: {} }
              : { online: true, queued: true },
        },
      },
    }

    const sync = syncKanbanColumn({ accountId: 'unified', folderId: 'inbox' })
    let finished = false
    void sync.then(() => {
      finished = true
    })
    await Promise.resolve()

    handlers.get('mail.newMessages')?.({ account: 'acc1', folder: 'inbox' })
    await Promise.resolve()
    expect(finished).toBe(false)

    handlers.get('mail.synced')?.({ account: 'acc2', folder: 'INBOX' })
    await sync

    expect(finished).toBe(true)
    expect(handlers.size).toBe(0)
  })

  it('sends the board attachments toggle alongside the read-state filter', async () => {
    const calls: { command: string; payload: any }[] = []
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: unknown) => {
            calls.push({ command, payload })
            return { threads: [], next_cursor: '', folder_unread: 0 }
          },
        },
      },
    }
    kanban$.globalFilter.set('unread')
    kanban$.globalAttachmentsOnly.set(true)
    try {
      await loadKanbanColumn({ accountId: 'acc1', folderId: 'INBOX' }, true)
      expect(calls.find((call) => call.command === 'mail.threadList')?.payload).toMatchObject({
        filter: 'unread',
        attachments: true,
      })
    } finally {
      kanban$.globalAttachmentsOnly.set(false)
      kanban$.globalFilter.set('all')
    }
  })

  it('reads a saved search column with its own query, narrowed by the board search', async () => {
    const calls: { command: string; payload: any }[] = []
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: unknown) => {
            calls.push({ command, payload })
            return { threads: [], next_cursor: '', folder_unread: 0 }
          },
        },
      },
    }
    await loadKanbanColumn({ accountId: 'acc1', folderId: 'INBOX', query: 'invoice' }, false, 'receipt')
    expect(calls.find((call) => call.command === 'mail.threadList')?.payload).toMatchObject({
      folder_id: 'INBOX',
      query: 'invoice\nAND receipt',
    })
  })

  it('sends the active global filter when loading a single-account column', async () => {
    const calls: { command: string; payload: unknown }[] = []
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: unknown) => {
            calls.push({ command, payload })
            return { threads: [], next_cursor: 'uid:10', folder_unread: 3 }
          },
        },
      },
    }
    kanban$.globalFilter.set('unread')

    await loadKanbanColumn({ accountId: 'acc1', folderId: 'INBOX' }, true)

    const threadListCalls = calls.filter((call) => call.command === 'mail.threadList').map((call) => call.payload)
    expect(threadListCalls).toHaveLength(1)
    expect(threadListCalls[0]).toMatchObject({
      account_id: 'acc1',
      folder_id: 'INBOX',
      filter: 'unread',
      refresh: true,
      before_cursor: undefined,
    })
    expect((threadListCalls[0] as { attachments?: boolean }).attachments).toBe(false)
    expect(activeKanbanColumnFilter({ accountId: 'acc1', folderId: 'INBOX' })).toBe('unread')
    expect(kanban$.unreadCounts['acc1\nINBOX'].get()).toBe(3)
    expect(mail$.foldersByAccount.acc1.get()?.[0]?.unread).toBe(3)
  })

  it('waits for a persisted never-synced folder before treating it as empty', async () => {
    const handlers = new Map<string, (detail: unknown) => void>()
    let reads = 0
    accounts$.set([account('acc1')])
    ;(window as any).runtime = {
      EventsOn: (name: string, handler: (detail: unknown) => void) => {
        handlers.set(name, handler)
        return () => handlers.delete(name)
      },
    }
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async () => {
            reads += 1
            return reads === 1
              ? { threads: [], folder_synced: false }
              : { threads: [message({ thread_id: 'synced' })], folder_synced: true }
          },
        },
      },
    }

    const load = loadKanbanColumn({ accountId: 'acc1', folderId: 'Archive' }, true)
    for (let attempt = 0; attempt < 10 && !handlers.has('mail.synced'); attempt += 1) {
      await Promise.resolve()
    }

    expect(kanban$.loading['acc1\nArchive'].get()).toBe(true)
    expect(reads).toBe(1)
    expect(handlers.has('mail.synced')).toBe(true)

    handlers.get('mail.synced')?.({ account: 'acc1', folder: 'Archive' })
    await load

    expect(reads).toBe(2)
    expect(kanban$.threads['acc1\nArchive'].get()[0]?.thread_id).toBe('synced')
    expect(kanban$.loading['acc1\nArchive'].get()).toBe(false)
  })

  it('reads on after a reload until the oldest card shown is back', async () => {
    // Each request answers one card whatever its limit — as an attachments
    // filter or a long thread can — so the read depth alone falls short.
    const pages: Record<string, { threads: import('../../types').Message[]; next_cursor: string }> = {
      '': { threads: [message({ id: 't3', thread_id: 't3', date: 300 })], next_cursor: 'c1' },
      c1: { threads: [message({ id: 't2', thread_id: 't2', date: 200 })], next_cursor: 'c2' },
      c2: { threads: [message({ id: 't1', thread_id: 't1', date: 100 })], next_cursor: '' },
    }
    const calls: { before_cursor?: string; limit?: number }[] = []
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: { before_cursor?: string; limit?: number }) => {
            if (command !== 'mail.threadList') return {}
            calls.push(payload)
            return pages[payload.before_cursor ?? '']
          },
        },
      },
    }
    const column = { accountId: 'deep', folderId: 'INBOX' }
    const key = 'deep\nINBOX'

    await loadKanbanColumn(column)
    await loadMoreKanbanColumn(column)
    expect(kanban$.threads[key].get()?.map((thread) => thread.id)).toEqual(['t3', 't2'])

    calls.length = 0
    await loadKanbanColumn(column)

    expect(calls.map((call) => [call.before_cursor, call.limit])).toEqual([
      [undefined, 100],
      ['c1', 50],
    ])
    expect(kanban$.threads[key].get()?.map((thread) => thread.id)).toEqual(['t3', 't2'])
    expect(kanban$.cursors[key].get()).toBe('c2')
  })

  it('keeps using the active filter when loading more of a column', async () => {
    const calls: { command: string; payload: unknown }[] = []
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: unknown) => {
            calls.push({ command, payload })
            return { threads: [], next_cursor: '', folder_unread: command === 'mail.threadList' ? 2 : 0 }
          },
        },
      },
    }
    kanban$.globalFilter.set('unread')
    kanban$.cursors['acc1\nINBOX'].set('uid:10')
    kanban$.unreadCounts['acc1\nINBOX'].set(5)

    await loadMoreKanbanColumn({ accountId: 'acc1', folderId: 'INBOX' })

    const threadListCalls = calls.filter((call) => call.command === 'mail.threadList').map((call) => call.payload)
    expect(threadListCalls).toHaveLength(1)
    expect(threadListCalls[0]).toMatchObject({
      account_id: 'acc1',
      folder_id: 'INBOX',
      filter: 'unread',
      refresh: false,
      before_cursor: 'uid:10',
    })
    expect(kanban$.unreadCounts['acc1\nINBOX'].get()).toBe(5)
  })

  it('pages with the search and filters the cursor was issued for', async () => {
    const column = { accountId: 'acc1', folderId: 'Projects' }
    const key = 'acc1\nProjects'
    const calls: Record<string, unknown>[] = []
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: Record<string, unknown>) => {
            if (command === 'mail.threadList') calls.push(payload)
            return { threads: [], next_cursor: 'cursor-a' }
          },
        },
      },
    }
    kanban$.searchQuery.set('alpha')
    kanban$.searchScope.set('all')
    kanban$.globalAttachmentsOnly.set(true)
    await loadKanbanColumn(column, true, 'alpha')

    // The board has moved on to another search and filters, but the column
    // still holds search A's cursor until B's first page replaces it.
    kanban$.searchQuery.set('beta')
    kanban$.globalAttachmentsOnly.set(false)
    kanban$.filters[key].set('unread')
    await loadMoreKanbanColumn(column)

    expect(calls[1]).toMatchObject({
      query: 'alpha',
      filter: 'all',
      attachments: true,
      before_cursor: 'cursor-a',
    })
    kanban$.searchQuery.set('')
  })

  it('does not page a column while a reload is replacing it', async () => {
    const calls: string[] = []
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string) => {
            calls.push(command)
            return { threads: [], next_cursor: '' }
          },
        },
      },
    }
    kanban$.cursors['acc1\nINBOX'].set('uid:10')
    kanban$.loading['acc1\nINBOX'].set(true)

    await loadMoreKanbanColumn({ accountId: 'acc1', folderId: 'INBOX' })

    expect(calls).toEqual([])
  })

  it('sends one core-owned request for a unified column', async () => {
    const calls: { command: string; payload: unknown }[] = []
    accounts$.set([account('acc1'), { ...account('acc2'), included_in_unified: false }, account('acc3')])
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: unknown) => {
            calls.push({ command, payload })
            return { threads: [], next_cursor: '', folder_unread: 4, folder_unreads: { acc1: 2, acc3: 2 } }
          },
        },
      },
    }
    kanban$.globalFilter.set('unread')

    await loadKanbanColumn({ accountId: 'unified', folderId: 'inbox' }, true)

    const threadListCalls = calls.filter((call) => call.command === 'mail.threadList').map((call) => call.payload)
    expect(threadListCalls).toHaveLength(1)
    expect(threadListCalls[0]).toMatchObject({ account_id: 'unified', folder_id: 'inbox', filter: 'unread' })
    expect(kanban$.unreadCounts['unified\ninbox'].get()).toBe(4)
    expect(mail$.foldersByAccount.acc1.get()?.[0]?.unread).toBe(2)
    expect(mail$.foldersByAccount.acc3.get()?.[0]?.unread).toBe(2)
  })

  it('asks for a unified column by role and keeps its unreads out of the inbox badge', async () => {
    const calls: { command: string; payload: unknown }[] = []
    accounts$.set([account('acc1')])
    mail$.foldersByAccount.acc1.set([{ account_id: 'acc1', id: 'INBOX', name: 'Inbox', role: 'inbox', unread: 7 }])
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: unknown) => {
            calls.push({ command, payload })
            return { threads: [], next_cursor: '', folder_unreads: { acc1: 1 } }
          },
        },
      },
    }
    kanban$.globalFilter.set('all')

    await loadKanbanColumn({ accountId: 'unified', folderId: 'trash' }, true)

    const threadListCalls = calls.filter((call) => call.command === 'mail.threadList').map((call) => call.payload)
    expect(threadListCalls).toHaveLength(1)
    // Both fields: folder_id keeps the request readable, folder_role is what the
    // core resolves per account.
    expect(threadListCalls[0]).toMatchObject({
      account_id: 'unified',
      folder_id: 'trash',
      folder_role: 'trash',
    })
    // Trash's unread count must not land on the side-nav's inbox badge.
    expect(mail$.foldersByAccount.acc1.get()?.[0]?.unread).toBe(7)
  })

  it('syncs each account own folder behind a unified non-inbox column', async () => {
    const calls: { command: string; payload: any }[] = []
    accounts$.set([account('acc1'), account('acc2')])
    // acc1 has a Sent folder; acc2's server does not, so it sits the sync out.
    mail$.foldersByAccount.acc1.set([
      { account_id: 'acc1', id: 'INBOX', name: 'Inbox', role: 'inbox', unread: 0 },
      { account_id: 'acc1', id: '[Gmail]/Sent Mail', name: 'Sent', role: 'sent', unread: 0 },
    ])
    mail$.foldersByAccount.acc2.set([{ account_id: 'acc2', id: 'INBOX', name: 'Inbox', role: 'inbox', unread: 0 }])
    const handlers = new Map<string, (detail: unknown) => void>()
    ;(window as any).runtime = {
      EventsOn: (name: string, handler: (detail: unknown) => void) => {
        handlers.set(name, handler)
        return () => handlers.delete(name)
      },
    }
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: unknown) => {
            calls.push({ command, payload })
            return command === 'mail.threadList' ? { threads: [], next_cursor: '' } : { online: true, queued: true }
          },
        },
      },
    }

    const sync = syncKanbanColumn({ accountId: 'unified', folderId: 'sent' })
    await Promise.resolve()
    handlers.get('mail.synced')?.({ account: 'acc1', folder: '[Gmail]/Sent Mail' })
    await sync

    const syncCalls = calls.filter((call) => call.command === 'mail.sync').map((call) => call.payload)
    expect(syncCalls).toEqual([{ account_id: 'acc1', folder: '[Gmail]/Sent Mail' }])
  })

  it('keeps a just-read thread in an unread column when the reload drops it', async () => {
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async () => ({ threads: [message({ thread_id: 't2', date: 200, unread: true })], next_cursor: '' }),
        },
      },
    }
    kanban$.globalFilter.set('unread')
    // t1 was showing and is the thread we just read; the unread refetch omits it.
    kanban$.threads['acc1\nINBOX'].set([message({ thread_id: 't1', date: 100, unread: false })])
    mail$.readThreads.set({ t1: true })

    await loadKanbanColumn({ accountId: 'acc1', folderId: 'INBOX' }, false)

    expect(kanban$.threads['acc1\nINBOX'].get().map((thread) => thread.thread_id)).toEqual(['t2', 't1'])
  })

  it('keeps an archived card out of a column reload and follows the open card', async () => {
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async () => ({
            threads: [
              message({ id: 't1', thread_id: 't1', date: 200 }),
              message({ id: 't2', thread_id: 't2', date: 100 }),
            ],
            next_cursor: '',
          }),
        },
      },
    }
    const column = { accountId: 'acc1', folderId: 'INBOX' }
    kanban$.activeBoardId.set('board')
    kanban$.paneColumnKey.set('board\nacc1\nINBOX')
    kanban$.paneThreadId.set('t1')
    ui$.selectedThread.set('t1')
    kanban$.threads['acc1\nINBOX'].set([
      message({ id: 't1', thread_id: 't1', date: 200 }),
      message({ id: 't2', thread_id: 't2', date: 100 }),
    ])
    await loadKanbanColumn(column, false)
    const undo = suppressRemovedThread('t1')
    try {
      await loadKanbanColumn(column, false)
      expect(kanban$.threads['acc1\nINBOX'].get().map((thread) => thread.thread_id)).toEqual(['t2'])
      expect(kanban$.paneThreadId.get()).toBe('t2')
      expect(ui$.selectedThread.get()).toBe('t2')
    } finally {
      undo()
      releaseRemovedThread('t1')
      kanban$.activeBoardId.set('')
      kanban$.paneThreadId.set('')
      kanban$.paneColumnKey.set('')
      ui$.selectedThread.set('')
    }
  })

  it('does not resurrect read threads when the column filter is All', async () => {
    ;(window as any).go = {
      main: { App: { Invoke: async () => ({ threads: [], next_cursor: '' }) } },
    }
    kanban$.globalFilter.set('all')
    kanban$.threads['acc1\nINBOX'].set([message({ thread_id: 't1', date: 100, unread: false })])
    mail$.readThreads.set({ t1: true })

    await loadKanbanColumn({ accountId: 'acc1', folderId: 'INBOX' }, false)

    expect(kanban$.threads['acc1\nINBOX'].get()).toEqual([])
  })

  it('loads unified starred items through the starred-items bridge', async () => {
    const calls: { command: string; payload: unknown }[] = []
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (command: string, payload: unknown) => {
            calls.push({ command, payload })
            return {
              items: [
                {
                  id: 'old',
                  account_id: 'acc1',
                  folder_id: 'inbox',
                  thread_id: 't1',
                  from_name: 'Alice',
                  from_addr: 'a@example.com',
                  to: '',
                  subject: 'Old',
                  preview: 'quarterly report',
                  body: '',
                  date: 1704067200, // 2024-01-01T00:00:00Z
                  unread: false,
                  starred: true,
                  has_attachments: false,
                },
                {
                  id: 'new',
                  account_id: 'acc2',
                  folder_id: 'inbox',
                  thread_id: 't2',
                  from_name: 'Bob',
                  from_addr: 'b@example.com',
                  to: '',
                  subject: 'New quarterly',
                  preview: '',
                  body: '',
                  date: 1704153600, // 2024-01-02T00:00:00Z
                  unread: false,
                  starred: true,
                  has_attachments: false,
                },
              ],
            }
          },
        },
      },
    }

    await loadKanbanColumn({ accountId: 'unified', folderId: 'starred' }, true, 'quarterly')

    expect(calls).toEqual([
      {
        command: 'mail.starredItems',
        payload: { query: 'quarterly', filter: 'all', attachments: false, limit: 50, before_cursor: undefined },
      },
    ])
    expect(kanban$.threads['unified\nstarred'].get().map((item) => item.id)).toEqual(['old', 'new'])
    expect(kanban$.cursors['unified\nstarred'].get()).toBe('')
    expect(kanban$.accountCursors['unified\nstarred'].get()).toEqual({})
  })
})
