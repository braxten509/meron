// Board search and filters applied to desktop columns.
// Fixtures exercise scopes, saved searches and global filter changes.
// The resulting query must match the one used to load the column.
import { beforeEach, describe, expect, it } from 'bun:test'
import {
  editKanbanSearchColumn,
  getKanbanColumns,
  kanban$,
  narrowKanbanQuery,
  saveKanbanSearchColumn,
  setGlobalKanbanAttachmentsOnly,
  setGlobalKanbanFilter,
} from '../states/kanban'
import { settings$ } from '../states/settings'
import {
  activeKanbanColumnAttachmentsOnly,
  activeKanbanColumnFilter,
  activeKanbanColumnQuery,
  columnEmptyText,
  columnSearchActive,
  searchScopeColumn,
  searchTargets,
} from './kanbanData'
import { account, resetBoardData } from './kanbanTests/fixtures'

beforeEach(resetBoardData)

describe('searchScopeColumn', () => {
  it("returns null for empty and 'all' scopes", () => {
    expect(searchScopeColumn('')).toBeNull()
    expect(searchScopeColumn('all')).toBeNull()
  })

  it('splits an account/folder scope on newline', () => {
    expect(searchScopeColumn('acc1\nINBOX')).toEqual({ accountId: 'acc1', folderId: 'INBOX' })
  })

  it('returns null for a malformed scope', () => {
    expect(searchScopeColumn('acc1')).toBeNull()
  })
})

describe('searchTargets', () => {
  const columns = [
    { accountId: 'acc1', folderId: 'INBOX' },
    { accountId: 'acc2', folderId: 'INBOX' },
  ]

  it("returns all columns for scope 'all'", () => {
    expect(searchTargets(columns, 'all')).toEqual(columns)
  })

  it('returns only the scoped column', () => {
    expect(searchTargets(columns, 'acc2\nINBOX')).toEqual([columns[1]])
  })

  it('reaches saved search columns, and scopes to one apart from its folder', () => {
    const saved = { accountId: 'acc1', folderId: 'INBOX', query: 'invoice' }
    expect(searchTargets([...columns, saved], 'all')).toEqual([...columns, saved])
    expect(searchTargets([...columns, saved], 'acc1\nINBOX\ninvoice')).toEqual([saved])
    expect(searchTargets([...columns, saved], 'acc1\nINBOX')).toEqual([columns[0]])
  })
})

describe('narrowKanbanQuery', () => {
  it('joins the board search to a saved query with AND', () => {
    expect(narrowKanbanQuery('from:boss', ' budget ')).toBe('from:boss\nAND budget')
    expect(narrowKanbanQuery('from:boss', '  ')).toBe('from:boss')
    expect(narrowKanbanQuery(undefined, ' budget ')).toBe('budget')
  })

  it('leaves the saved query as it is, quotes and groups left open too', () => {
    expect(narrowKanbanQuery('subject:"Q3 AND budget', 'paid')).toBe('subject:"Q3 AND budget\nAND paid')
    expect(narrowKanbanQuery('27" monitor', '24" monitor')).toBe('27" monitor\nAND 24" monitor')
    expect(narrowKanbanQuery('subject:"Q3', '  ')).toBe('subject:"Q3')
  })

  it('drops a leading OR left stray after the AND', () => {
    expect(narrowKanbanQuery('from:a', 'OR from:b')).toBe('from:a\nAND from:b')
    expect(narrowKanbanQuery('from:a', 'OR OR')).toBe('from:a')
    expect(narrowKanbanQuery('from:a', 'ORACLE')).toBe('from:a\nAND ORACLE')
    expect(narrowKanbanQuery(undefined, 'OR x')).toBe('OR x')
  })
})

describe('editKanbanSearchColumn', () => {
  const inbox = { accountId: 'acc1', folderId: 'INBOX' }
  const saved = { ...inbox, query: 'invoice' }
  const sent = { accountId: 'acc1', folderId: 'Sent' }

  beforeEach(() => {
    settings$.kanbanBoards.set([{ id: 'kb-1', name: 'Work', columns: [inbox, saved, sent] }])
    settings$.kanbanMinimizedColumns.set({})
  })

  it('changes the query in place and carries the column state over', () => {
    kanban$.filters['acc1\nINBOX\ninvoice'].set('unread')
    settings$.kanbanMinimizedColumns['kb-1\nacc1\nINBOX\ninvoice'].set(true)
    expect(editKanbanSearchColumn('kb-1', saved, ' receipt ')).toEqual({ ...inbox, query: 'receipt' })
    expect(getKanbanColumns('kb-1')).toEqual([inbox, { ...inbox, query: 'receipt' }, sent])
    expect(kanban$.filters['acc1\nINBOX\nreceipt'].get()).toBe('unread')
    expect(kanban$.filters['acc1\nINBOX\ninvoice'].get()).toBeUndefined()
    expect(settings$.kanbanMinimizedColumns['kb-1\nacc1\nINBOX\nreceipt'].get()).toBe(true)
  })

  it('refuses an empty, unchanged or duplicate query, and a plain folder column', () => {
    settings$.kanbanBoards.set([{ id: 'kb-1', name: 'Work', columns: [inbox, saved, { ...inbox, query: 'receipt' }] }])
    expect(editKanbanSearchColumn('kb-1', saved, '   ')).toBeNull()
    expect(editKanbanSearchColumn('kb-1', saved, 'invoice')).toBeNull()
    expect(editKanbanSearchColumn('kb-1', saved, 'receipt')).toBeNull()
    expect(editKanbanSearchColumn('kb-1', inbox, 'invoice')).toBeNull()
    expect(getKanbanColumns('kb-1')).toEqual([inbox, saved, { ...inbox, query: 'receipt' }])
  })
})

describe('saveKanbanSearchColumn', () => {
  const inbox = { accountId: 'acc1', folderId: 'INBOX' }
  const sent = { accountId: 'acc1', folderId: 'Sent' }

  beforeEach(() => {
    settings$.kanbanBoards.set([{ id: 'kb-1', name: 'Work', columns: [inbox, sent] }])
  })

  it('places the saved search right after the column it came from', () => {
    expect(saveKanbanSearchColumn('kb-1', inbox, '  invoice \n due ')).toEqual({ ...inbox, query: 'invoice due' })
    expect(getKanbanColumns('kb-1')).toEqual([inbox, { ...inbox, query: 'invoice due' }, sent])
  })

  it('refuses an empty query, a duplicate, and saving a saved search again', () => {
    expect(saveKanbanSearchColumn('kb-1', inbox, '   ')).toBeNull()
    const saved = saveKanbanSearchColumn('kb-1', inbox, 'invoice')!
    expect(saveKanbanSearchColumn('kb-1', inbox, 'invoice')).toBeNull()
    expect(saveKanbanSearchColumn('kb-1', saved, 'other')).toBeNull()
    expect(getKanbanColumns('kb-1')).toHaveLength(3)
  })
})

describe('columnEmptyText', () => {
  it('describes empty columns per filter mode', () => {
    expect(columnEmptyText('all', false, false)).toBe('No threads')
    expect(columnEmptyText('unread', false, false)).toBe('No unread threads')
    expect(columnEmptyText('starred', false, false)).toBe('No starred threads')
  })

  it('uses feed wording for RSS columns', () => {
    expect(columnEmptyText('all', false, false, true)).toBe('No feeds')
    expect(columnEmptyText('unread', false, false, true)).toBe('No unread feeds')
    expect(columnEmptyText('starred', false, false, true)).toBe('No starred feeds')
  })

  it('distinguishes no matches from matches hidden by a filter', () => {
    expect(columnEmptyText('all', true, false)).toBe('No matches')
    expect(columnEmptyText('unread', true, false)).toBe('No unread matches')
    expect(columnEmptyText('unread', true, true)).toBe('Matches hidden by Unread filter')
    expect(columnEmptyText('starred', true, true)).toBe('Matches hidden by Starred filter')
  })

  it('adds the attachments toggle to whatever the filter says', () => {
    expect(columnEmptyText('all', false, false, false, true)).toBe('No threads with attachments')
    expect(columnEmptyText('unread', false, false, false, true)).toBe('No unread threads with attachments')
    expect(columnEmptyText('all', true, true, false, true)).toBe('Matches hidden by Attachments filter')
  })
})

describe('columnSearchActive', () => {
  it('requires a non-blank query and a matching scope', () => {
    expect(columnSearchActive('k1', '', 'all')).toBe(false)
    expect(columnSearchActive('k1', '   ', 'all')).toBe(false)
    expect(columnSearchActive('k1', 'q', 'all')).toBe(true)
    expect(columnSearchActive('k1', 'q', 'k1')).toBe(true)
    expect(columnSearchActive('k1', 'q', 'k2')).toBe(false)
  })
})

describe('activeKanbanColumnQuery', () => {
  const column = { accountId: 'acc1', folderId: 'INBOX' }

  beforeEach(() => {
    kanban$.searchQuery.set('')
    kanban$.searchScope.set('all')
  })

  it('returns the trimmed board query when the search covers every column', () => {
    kanban$.searchQuery.set('  invoice ')
    expect(activeKanbanColumnQuery(column)).toBe('invoice')
  })

  it('returns the query only for the column the search is scoped to', () => {
    kanban$.searchQuery.set('invoice')
    kanban$.searchScope.set('acc1\nINBOX')
    expect(activeKanbanColumnQuery(column)).toBe('invoice')
    expect(activeKanbanColumnQuery({ accountId: 'acc1', folderId: 'Sent' })).toBe('')
  })

  it('is empty when there is no search', () => {
    kanban$.searchQuery.set('   ')
    expect(activeKanbanColumnQuery(column)).toBe('')
  })

  it('gives a saved search column only the board search, not its own query', () => {
    const saved = { ...column, query: 'invoice' }
    kanban$.searchQuery.set('')
    expect(activeKanbanColumnQuery(saved)).toBe('')
    kanban$.searchQuery.set('receipt')
    expect(activeKanbanColumnQuery(saved)).toBe('receipt')
  })
})

describe('setGlobalKanbanFilter', () => {
  it('clears per-column overrides so the new global filter applies everywhere', () => {
    const column = { accountId: 'acc1', folderId: 'INBOX' }
    kanban$.filters['acc1\nINBOX'].set('all')
    expect(activeKanbanColumnFilter(column)).toBe('all')

    setGlobalKanbanFilter('unread')

    expect(kanban$.filters.get()).toEqual({})
    expect(activeKanbanColumnFilter(column)).toBe('unread')
  })
})

describe('setGlobalKanbanAttachmentsOnly', () => {
  it('clears per-column overrides so the new global toggle applies everywhere', () => {
    const column = { accountId: 'acc1', folderId: 'INBOX' }
    kanban$.attachmentsOnly['acc1\nINBOX'].set(true)
    expect(activeKanbanColumnAttachmentsOnly(column)).toBe(true)

    setGlobalKanbanAttachmentsOnly(false)

    expect(kanban$.attachmentsOnly.get()).toEqual({})
    expect(activeKanbanColumnAttachmentsOnly(column)).toBe(false)
  })
})
