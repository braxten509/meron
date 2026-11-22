// Load thread pages and restore selections.
// Uses the same observable state and command interface as the thread list.
// Fixtures supply synthetic messages, independent of a real mailbox.
import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { compose$ } from '../composeState'
import { kanban$ } from '../kanban'
import { loadMoreThreads, loadThreads, mail$, requestThreadReselect } from '../mail'
import { ui$ } from '../ui'
import { thread, nextTick } from './fixture'

describe('thread list paging', () => {
  beforeEach(() => {
    kanban$.activeBoardId.set('')
    mail$.threads.set([thread({ subject: 'Old first page' })])
    mail$.threadsCursor.set('old-cursor')
    mail$.threadsLoadingMore.set(false)
    mail$.threadAccountCursors.set({})
    ui$.selectedAccount.set('acc')
    ui$.selectedFolder.set('inbox')
    ui$.selectedThread.set('')
    ui$.query.set('old')
    ui$.filterMode.set('all')
  })

  afterEach(() => {
    ui$.query.set('')
  })

  it('drops a load-more response after a newer search replaces the list', async () => {
    let resolveOldPage!: (value: unknown) => void
    const oldPage = new Promise((resolve) => {
      resolveOldPage = resolve
    })
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (_command: string, payload: { before_cursor?: string; query?: string }) => {
            if (payload.before_cursor === 'old-cursor') return oldPage
            if (payload.query === 'new') {
              return {
                threads: [thread({ thread_id: 'acc:inbox:thread:new', subject: 'New result' })],
                next_cursor: 'new-cursor',
              }
            }
            return {}
          },
        },
      },
    }

    const loadingMore = loadMoreThreads()
    ui$.query.set('new')
    await loadThreads()
    resolveOldPage({
      threads: [thread({ thread_id: 'acc:inbox:thread:old-page', subject: 'Stale result' })],
      next_cursor: 'stale-cursor',
    })
    await loadingMore

    expect(mail$.threads.get().map((item) => item.subject)).toEqual(['New result'])
    expect(mail$.threadsCursor.get()).toBe('new-cursor')
  })

  it('paints cached search results before replacing them with live results', async () => {
    let resolveLive!: (value: unknown) => void
    const livePage = new Promise((resolve) => {
      resolveLive = resolve
    })
    const refreshes: boolean[] = []
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async (_command: string, payload: { refresh?: boolean }) => {
            refreshes.push(payload.refresh ?? true)
            if (payload.refresh) return livePage
            return {
              threads: [thread({ thread_id: 'acc:inbox:thread:cached', subject: 'Cached result' })],
              next_cursor: 'cached-cursor',
            }
          },
        },
      },
    }

    ui$.query.set('deploy')
    const loading = loadThreads()
    await nextTick()

    expect(refreshes).toEqual([false, true])
    expect(mail$.threads.get().map((item) => item.subject)).toEqual(['Cached result'])
    expect(mail$.threadsCursor.get()).toBe('cached-cursor')
    // Neither stage opens a result on its own — that would mark it read.
    expect(ui$.selectedThread.get()).toBe('')

    resolveLive({
      threads: [thread({ thread_id: 'acc:inbox:thread:live', subject: 'Live result' })],
      next_cursor: 'live-cursor',
    })
    await loading

    expect(mail$.threads.get().map((item) => item.subject)).toEqual(['Live result'])
    expect(mail$.threadsCursor.get()).toBe('live-cursor')
    expect(ui$.selectedThread.get()).toBe('')
  })
})

describe('thread selection on load', () => {
  beforeEach(() => {
    mail$.threads.set([])
    mail$.messages.set([])
    mail$.threadsCursor.set('')
    mail$.threadAccountCursors.set({})
    mail$.threadLoading.set(false)
    ui$.selectedAccount.set('unified')
    ui$.selectedFolder.set('inbox')
    ui$.selectedThread.set('')
    ui$.query.set('')
    ui$.filterMode.set('all')
    kanban$.activeBoardId.set('')
    ;(window as any).go = {
      main: {
        App: {
          Invoke: async () => ({
            threads: [thread({ thread_id: 'acc:inbox:thread:top', unread: true })],
          }),
        },
      },
    }
  })

  afterEach(() => {
    kanban$.activeBoardId.set('')
  })

  it('leaves the conversation pane closed when a view is opened', async () => {
    await loadThreads()

    expect(mail$.threads.get()).toHaveLength(1)
    // Opening a thread marks its visible messages read, so switching to a view
    // must never do it on the user's behalf.
    expect(ui$.selectedThread.get()).toBe('')
  })

  it('closes a selection the new view does not contain instead of opening another thread', async () => {
    // The conversation is open and fully loaded, as it is after any ordinary
    // click — its messages must not keep it selected in a view it is absent from.
    ui$.selectedThread.set('acc:other:thread:9')
    mail$.messages.set([thread({ id: 'acc:other:thread:9#1', thread_id: 'acc:other:thread:9' })])

    await loadThreads()

    expect(ui$.selectedThread.get()).toBe('')
  })

  it("keeps an active thread tab's selection the new view does not contain", async () => {
    // Leaving a kanban board restores the mail view's folder; a tab opened from
    // a card in another folder still owns the pane and must keep rendering.
    compose$.tabs.set([
      {
        id: 'thread-tab',
        kind: 'thread',
        messageId: '',
        threadId: 'acc:other:thread:9',
        subject: 'Tab',
        from: 'sender@example.com',
        body: '',
        viewMode: 'plain',
      },
    ])
    compose$.activeTab.set('thread-tab')
    ui$.selectedThread.set('acc:other:thread:9')

    try {
      await loadThreads()
      expect(ui$.selectedThread.get()).toBe('acc:other:thread:9')
    } finally {
      compose$.tabs.set([])
      compose$.activeTab.set('')
    }
  })

  it('keeps a selection whose conversation is still loading', async () => {
    ui$.selectedThread.set('acc:other:thread:9')
    mail$.threadLoading.set(true)

    await loadThreads()

    expect(ui$.selectedThread.get()).toBe('acc:other:thread:9')
  })

  it('keeps the open thread when it is still in the loaded view', async () => {
    ui$.selectedThread.set('acc:inbox:thread:top')

    await loadThreads()

    expect(ui$.selectedThread.get()).toBe('acc:inbox:thread:top')
  })

  it('does not replace the waiting mail list while a kanban card owns the folder selection', async () => {
    const sent = thread({ folder_id: 'sent', subject: 'Unified sent' })
    let calls = 0
    mail$.threads.set([sent])
    ui$.selectedFolder.set('[Gmail]/Sent Mail')
    kanban$.activeBoardId.set('board-1')
    ;(window as any).go.main.App.Invoke = async () => {
      calls += 1
      return { threads: [thread({ subject: 'Unified inbox' })] }
    }

    await loadThreads()

    expect(calls).toBe(0)
    expect(mail$.threads.get()).toEqual([sent])
  })

  it('drops a reselect request the skipped kanban load was meant to answer', async () => {
    kanban$.activeBoardId.set('board-1')
    // Deleting the last card of a column clears the pane and asks the load that
    // follows to pick the replacement — a load that never runs behind a board.
    requestThreadReselect()

    await loadThreads()

    kanban$.activeBoardId.set('')
    await loadThreads()

    expect(ui$.selectedThread.get()).toBe('')
  })
})
