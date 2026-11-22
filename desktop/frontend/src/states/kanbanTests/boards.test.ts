// Changes to the board and its column definitions.
// The cases switch folders, reorder columns and remove deleted folders or boards.
// State assertions check the resulting board selection and column identity.
import { beforeEach, describe, expect, it } from 'bun:test'
import {
  kanban$,
  removeKanbanBoard,
  removeKanbanColumnsForFolder,
  reorderKanbanColumn,
  switchKanbanColumnFolder,
} from '../kanban'
import { settings$ } from '../settings'
import { message } from './fixtures'

describe('switchKanbanColumnFolder', () => {
  beforeEach(() => {
    settings$.kanbanBoards.set([
      {
        id: 'b1',
        name: 'Board',
        columns: [
          { accountId: 'acc1', folderId: 'INBOX' },
          { accountId: 'acc1', folderId: 'Archive' },
        ],
      },
    ])
    settings$.kanbanMinimizedColumns.set({})
    kanban$.threads.set({})
    kanban$.cursors.set({})
    kanban$.accountCursors.set({})
    kanban$.filters.set({})
    kanban$.paneColumnKey.set('')
    kanban$.searchScope.set('all')
  })

  it('repoints the column in place and carries its state to the new key', () => {
    kanban$.threads['acc1\nINBOX'].set([message()])
    kanban$.cursors['acc1\nINBOX'].set('cursor')
    kanban$.filters['acc1\nINBOX'].set('unread')
    settings$.kanbanMinimizedColumns['b1\nacc1\nINBOX'].set(true)
    kanban$.paneColumnKey.set('b1\nacc1\nINBOX')
    kanban$.searchScope.set('acc1\nINBOX')

    const switched = switchKanbanColumnFolder('b1', { accountId: 'acc1', folderId: 'INBOX' }, 'Sent')

    expect(switched).toBe(true)
    expect(settings$.kanbanBoards.get()[0].columns).toEqual([
      { accountId: 'acc1', folderId: 'Sent' },
      { accountId: 'acc1', folderId: 'Archive' },
    ])
    expect(kanban$.filters['acc1\nSent'].get()).toBe('unread')
    expect(settings$.kanbanMinimizedColumns['b1\nacc1\nSent'].get()).toBe(true)
    expect(settings$.kanbanMinimizedColumns['b1\nacc1\nINBOX'].get()).toBeUndefined()
    expect(kanban$.paneColumnKey.get()).toBe('b1\nacc1\nSent')
    expect(kanban$.searchScope.get()).toBe('acc1\nSent')
    expect(kanban$.threads['acc1\nINBOX'].get()).toBeUndefined()
    expect(kanban$.cursors['acc1\nINBOX'].get()).toBeUndefined()
  })

  it('refuses a folder that already has its own column', () => {
    const switched = switchKanbanColumnFolder('b1', { accountId: 'acc1', folderId: 'INBOX' }, 'Archive')

    expect(switched).toBe(false)
    expect(settings$.kanbanBoards.get()[0].columns).toEqual([
      { accountId: 'acc1', folderId: 'INBOX' },
      { accountId: 'acc1', folderId: 'Archive' },
    ])
  })

  it('keeps the old folder cache when another board still shows it', () => {
    settings$.kanbanBoards.set([
      { id: 'b1', name: 'Board', columns: [{ accountId: 'acc1', folderId: 'INBOX' }] },
      { id: 'b2', name: 'Other', columns: [{ accountId: 'acc1', folderId: 'INBOX' }] },
    ])
    kanban$.threads['acc1\nINBOX'].set([message()])

    expect(switchKanbanColumnFolder('b1', { accountId: 'acc1', folderId: 'INBOX' }, 'Sent')).toBe(true)
    expect(kanban$.threads['acc1\nINBOX'].get()).toHaveLength(1)
  })
})

describe('reorderKanbanColumn', () => {
  const inbox = { accountId: 'acc1', folderId: 'INBOX' }
  const archive = { accountId: 'acc1', folderId: 'Archive' }
  const sent = { accountId: 'acc1', folderId: 'Sent' }
  const order = () => settings$.kanbanBoards.get()[0].columns.map((column) => column.folderId)

  beforeEach(() => {
    settings$.kanbanBoards.set([{ id: 'b1', name: 'Board', columns: [inbox, archive, sent] }])
  })

  it('moves a column into the slot of the one it was dropped on, in either direction', () => {
    reorderKanbanColumn('b1', inbox, sent)
    expect(order()).toEqual(['Archive', 'Sent', 'INBOX'])
    reorderKanbanColumn('b1', inbox, archive)
    expect(order()).toEqual(['INBOX', 'Archive', 'Sent'])
  })

  it('leaves the board alone for a column it does not have', () => {
    reorderKanbanColumn('b1', inbox, { accountId: 'acc2', folderId: 'INBOX' })
    expect(order()).toEqual(['INBOX', 'Archive', 'Sent'])
  })
})

describe('removeKanbanColumnsForFolder', () => {
  it("drops the folder's columns on every board and forgets its cache", () => {
    settings$.kanbanBoards.set([
      {
        id: 'b1',
        name: 'Board',
        columns: [
          { accountId: 'acc1', folderId: 'INBOX' },
          { accountId: 'acc1', folderId: 'Work' },
        ],
      },
      { id: 'b2', name: 'Other', columns: [{ accountId: 'acc1', folderId: 'Work' }] },
    ])
    kanban$.threads.set({})
    kanban$.threads['acc1\nWork'].set([message({ folder_id: 'Work' })])

    removeKanbanColumnsForFolder('acc1', 'Work')

    expect(settings$.kanbanBoards.get()[0].columns).toEqual([{ accountId: 'acc1', folderId: 'INBOX' }])
    expect(settings$.kanbanBoards.get()[1].columns).toEqual([])
    expect(kanban$.threads['acc1\nWork'].get()).toBeUndefined()
  })

  it('leaves a same-named folder of another account alone', () => {
    settings$.kanbanBoards.set([{ id: 'b1', name: 'Board', columns: [{ accountId: 'acc2', folderId: 'Work' }] }])

    removeKanbanColumnsForFolder('acc1', 'Work')

    expect(settings$.kanbanBoards.get()[0].columns).toEqual([{ accountId: 'acc2', folderId: 'Work' }])
  })
})

describe('removeKanbanBoard', () => {
  beforeEach(() => {
    settings$.kanbanBoards.set([
      { id: 'b1', name: 'Board', columns: [] },
      { id: 'b2', name: 'Other', columns: [] },
    ])
  })

  it('falls back to the neighbouring board when the open one is deleted', () => {
    kanban$.activeBoardId.set('b2')

    removeKanbanBoard('b2')

    expect(settings$.kanbanBoards.get().map((board) => board.id)).toEqual(['b1'])
    expect(kanban$.activeBoardId.get()).toBe('b1')
  })

  it('deletes the last board and leaves the board view', () => {
    settings$.kanbanBoards.set([{ id: 'b1', name: 'Board', columns: [] }])
    kanban$.activeBoardId.set('b1')

    removeKanbanBoard('b1')

    expect(settings$.kanbanBoards.get()).toEqual([])
    expect(kanban$.activeBoardId.get()).toBe('')
  })
})
