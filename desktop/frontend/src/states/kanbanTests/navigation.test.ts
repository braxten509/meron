// Conversation selection and presentation preferences on a desktop board.
// The fixtures open and close conversations while changing the active board.
// Cases also cover adjacent-thread navigation, wallpaper and unread badges.
import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { compose$ } from '../composeState'
import type { MessageTab } from '../../types'
import {
  kanban$,
  closeCurrentConversation,
  kanbanPaneThreadId,
  kanbanColumnKey,
  kanbanBoardColumnKey,
  selectAdjacentKanbanThread,
  selectKanbanBoard,
  setKanbanBoardUnreadBadge,
  setKanbanBoardWallpaper,
} from '../kanban'
import { settings$ } from '../settings'
import { ui$ } from '../ui'
import { message } from './fixtures'

describe('closeCurrentConversation', () => {
  const tab = (threadId: string): MessageTab => ({
    id: `thread-${threadId}`,
    kind: 'thread',
    messageId: '',
    threadId,
    subject: threadId,
    from: 'sender@example.com',
    body: '',
    viewMode: 'plain',
  })

  beforeEach(() => {
    kanban$.activeBoardId.set('b1')
    kanban$.paneThreadId.set('t-card')
    kanban$.paneColumnKey.set('b1:INBOX')
    ui$.selectedThread.set('t-card')
    compose$.tabs.set([])
    compose$.activeTab.set('')
  })

  afterEach(() => {
    kanban$.activeBoardId.set('')
  })

  it('closes the pane when the card conversation is all it holds', () => {
    closeCurrentConversation()

    expect(kanban$.paneThreadId.get()).toBe('')
    expect(ui$.selectedThread.get()).toBe('')
    expect(ui$.mobilePane.get()).toBe('threads')
  })

  it('keeps the pane on a still-open tab instead of hiding it', () => {
    compose$.tabs.set([tab('t-tab')])

    closeCurrentConversation()

    expect(kanban$.paneThreadId.get()).toBe('')
    expect(compose$.activeTab.get()).toBe('thread-t-tab')
    expect(ui$.selectedThread.get()).toBe('t-tab')
    expect(compose$.conversationThread.get()).toBe('')
  })

  it('leaves an active tab alone and only closes the card conversation', () => {
    compose$.tabs.set([tab('t-tab')])
    compose$.activeTab.set('thread-t-tab')
    ui$.selectedThread.set('t-tab')

    closeCurrentConversation()

    expect(kanban$.paneThreadId.get()).toBe('')
    expect(compose$.activeTab.get()).toBe('thread-t-tab')
    expect(ui$.selectedThread.get()).toBe('t-tab')
  })
})

describe('setKanbanBoardWallpaper', () => {
  it('clears the background when Plain is picked', () => {
    settings$.kanbanBoards.set([
      { id: 'b1', name: 'Board', columns: [], wallpaper: { kind: 'preset', presetId: 'dots' } },
    ])

    setKanbanBoardWallpaper('b1', { kind: 'preset', presetId: 'plain' })

    expect(settings$.kanbanBoards.get()[0]).not.toHaveProperty('wallpaper')
  })
})

describe('setKanbanBoardUnreadBadge', () => {
  it('turns the badge on for one board and clears the field when turned off', () => {
    settings$.kanbanBoards.set([
      { id: 'b1', name: 'Board', columns: [] },
      { id: 'b2', name: 'Other', columns: [] },
    ])

    setKanbanBoardUnreadBadge('b1', true)
    expect(settings$.kanbanBoards.get()[0].showUnreadBadge).toBe(true)
    expect(settings$.kanbanBoards.get()[1]).not.toHaveProperty('showUnreadBadge')

    setKanbanBoardUnreadBadge('b1', false)
    expect(settings$.kanbanBoards.get()[0]).not.toHaveProperty('showUnreadBadge')
  })
})

describe('selectKanbanBoard', () => {
  const tab = (threadId: string): MessageTab => ({
    id: `thread-${threadId}`,
    kind: 'thread',
    messageId: '',
    threadId,
    subject: threadId,
    from: 'sender@example.com',
    body: '',
    viewMode: 'plain',
  })

  beforeEach(() => {
    settings$.kanbanBoards.set([{ id: 'b1', name: 'Board', columns: [{ accountId: 'acc1', folderId: 'INBOX' }] }])
    kanban$.activeBoardId.set('')
    compose$.tabs.set([])
    compose$.activeTab.set('')
  })

  afterEach(() => {
    kanban$.activeBoardId.set('')
    compose$.tabs.set([])
    compose$.activeTab.set('')
  })

  it('clears the mail view selection when no tab is on screen', () => {
    ui$.selectedThread.set('t-mail')

    selectKanbanBoard('b1')

    expect(kanban$.activeBoardId.get()).toBe('b1')
    expect(ui$.selectedThread.get()).toBe('')
  })

  it('keeps the active thread tab rendering when switching from the mail view', () => {
    compose$.tabs.set([tab('t-tab')])
    compose$.activeTab.set('thread-t-tab')
    ui$.selectedThread.set('t-tab')

    selectKanbanBoard('b1')

    expect(kanban$.activeBoardId.get()).toBe('b1')
    expect(ui$.selectedThread.get()).toBe('t-tab')
  })
})

describe('kanbanPaneThreadId', () => {
  const tab = (id: string, kind: MessageTab['kind'], threadId: string): MessageTab => ({
    id,
    kind,
    messageId: '',
    threadId,
    subject: threadId,
    from: 'sender@example.com',
    body: '',
    viewMode: 'plain',
  })

  beforeEach(() => {
    kanban$.paneThreadId.set('t-card')
    compose$.tabs.set([
      tab('thread-t-tab', 'thread', 't-tab'),
      tab('item-1', 'reader', 't-feed'),
      tab('compose-1', 'compose', ''),
    ])
    compose$.activeTab.set('')
  })

  afterEach(() => {
    kanban$.paneThreadId.set('')
    compose$.tabs.set([])
    compose$.activeTab.set('')
  })

  it('is the card conversation while no tab is active', () => {
    expect(kanbanPaneThreadId()).toBe('t-card')
  })

  it('is the active thread or reader tab rather than the card behind it', () => {
    compose$.activeTab.set('thread-t-tab')
    expect(kanbanPaneThreadId()).toBe('t-tab')
    compose$.activeTab.set('item-1')
    expect(kanbanPaneThreadId()).toBe('t-feed')
  })

  it('finds a tab opened while the pane was closed', () => {
    kanban$.paneThreadId.set('')
    compose$.activeTab.set('thread-t-tab')
    expect(kanbanPaneThreadId()).toBe('t-tab')
  })

  it('falls back to the card conversation behind a compose tab', () => {
    compose$.activeTab.set('compose-1')
    expect(kanbanPaneThreadId()).toBe('t-card')
  })
})

describe('selectAdjacentKanbanThread', () => {
  const inbox = { accountId: 'acc1', folderId: 'INBOX' }
  const starred = { accountId: 'unified', folderId: 'starred' }
  const card = (n: number) => message({ id: `m${n}`, thread_id: `t${n}`, subject: `S${n}`, unread: false })
  // Items of one feed share a thread_id.
  const item = (n: number) =>
    message({ id: `rss-1:item${n}`, account_id: 'rss-1', folder_id: 'feed', thread_id: 'feed-1', unread: false })
  const threadTab = (threadId: string): MessageTab => ({
    id: `thread-${threadId}`,
    kind: 'thread',
    messageId: '',
    threadId,
    subject: threadId,
    from: 'sender@example.com',
    body: '',
    viewMode: 'plain',
  })

  beforeEach(() => {
    settings$.kanbanBoards.set([{ id: 'b1', name: 'Board', columns: [inbox, starred] }])
    kanban$.activeBoardId.set('b1')
    kanban$.threads.set({
      [kanbanColumnKey(inbox)]: [card(1), card(2), card(3)],
      [kanbanColumnKey(starred)]: [item(1), item(2), item(3)],
    })
    kanban$.paneThreadId.set('')
    kanban$.paneColumnKey.set('')
    compose$.tabs.set([])
    compose$.activeTab.set('')
    ui$.selectedThread.set('')
  })

  afterEach(() => {
    kanban$.activeBoardId.set('')
    kanban$.threads.set({})
    compose$.tabs.set([])
    compose$.activeTab.set('')
  })

  it('steps from the thread tab on screen, not the card conversation behind it', () => {
    kanban$.paneThreadId.set('t1')
    compose$.tabs.set([threadTab('t2')])
    compose$.activeTab.set('thread-t2')
    ui$.selectedThread.set('t2')

    selectAdjacentKanbanThread(1)

    expect(kanban$.paneThreadId.get()).toBe('t3')
    expect(compose$.activeTab.get()).toBe('')
  })

  it('steps from a tab opened over a closed pane', () => {
    compose$.tabs.set([threadTab('t2')])
    compose$.activeTab.set('thread-t2')
    ui$.selectedThread.set('t2')

    selectAdjacentKanbanThread(-1)

    expect(kanban$.paneThreadId.get()).toBe('t1')
  })

  it('walks feed items by their own id and keeps them out of the card conversation', () => {
    kanban$.paneColumnKey.set(kanbanBoardColumnKey('b1', starred))
    compose$.tabs.set([{ ...threadTab('feed-1'), id: 'rss-1:item2', kind: 'reader', messageId: 'rss-1:item2' }])
    compose$.activeTab.set('rss-1:item2')

    selectAdjacentKanbanThread(1)

    expect(compose$.activeTab.get()).toBe('rss-1:item3')
    expect(kanban$.paneThreadId.get()).toBe('')
  })
})
