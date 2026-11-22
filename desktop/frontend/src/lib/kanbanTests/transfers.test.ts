// Destination rules used when dragging cards on the board.
// The fixtures cover account folders, unified views and tag columns.
// Drag constraints and rejection messages explain which moves are available.
import { beforeEach, describe, expect, it } from 'bun:test'
import { t } from '../i18n'
import {
  KANBAN_DROP_REASONS,
  columnReadsTags,
  columnTag,
  KANBAN_MOVE_MESSAGES,
  columnDropTargetClass,
  restrictColumnDragToHorizontal,
  resolveKanbanMove,
} from '../kanbanData'
import { account, resetBoardData, message } from './fixtures'

beforeEach(resetBoardData)

describe('resolveKanbanMove with tag columns', () => {
  const mailAccounts = [
    { id: 'acc1', provider: 'imap', auth_type: 'password' },
    { id: 'acc2', provider: 'imap', auth_type: 'password' },
    { id: 'feed1', provider: 'rss', auth_type: 'rss' },
    { id: 'feed2', provider: 'rss', auth_type: 'rss' },
  ]
  const todo = { accountId: 'unified', folderId: 'inbox', query: 'tag:todo' }
  const doing = { accountId: 'unified', folderId: 'inbox', query: 'tag:doing' }
  const inbox = { accountId: 'acc1', folderId: 'INBOX' }
  const archive = { accountId: 'acc1', folderId: 'Archive' }
  const thread = message({ account_id: 'acc1', folder_id: 'INBOX' })

  it('only a lone tag term makes a tag column', () => {
    expect(columnTag(todo)).toBe('todo')
    expect(columnTag({ ...todo, query: 'tag:Todo' })).toBe('todo')
    expect(columnTag({ ...inbox, query: 'tag:todo is:unread' })).toBeUndefined()
    expect(columnTag(inbox)).toBeUndefined()
    expect(columnTag({ accountId: 'unified', folderId: 'starred', query: 'tag:todo' })).toBeUndefined()
    expect(columnReadsTags({ ...inbox, query: 'tag:todo is:unread' })).toBe(true)
    expect(columnReadsTags({ ...inbox, query: 'stag:x' })).toBe(false)
  })

  it('dropping on a tag column tags and leaves the mail in place', () => {
    expect(resolveKanbanMove(inbox, todo, thread, mailAccounts)).toEqual({ kind: 'tag', add: 'todo' })
  })

  it('dropping between tag columns swaps the tag', () => {
    expect(resolveKanbanMove(todo, doing, thread, mailAccounts)).toEqual({
      kind: 'tag',
      add: 'doing',
      remove: 'todo',
    })
    expect(resolveKanbanMove(todo, todo, thread, mailAccounts)).toEqual({ kind: 'noop' })
  })

  it('dropping out of a tag column removes the tag, moving only if the folder differs', () => {
    expect(resolveKanbanMove(todo, inbox, thread, mailAccounts)).toEqual({ kind: 'tag', remove: 'todo' })
    expect(resolveKanbanMove(todo, archive, thread, mailAccounts)).toEqual({
      kind: 'move',
      origin: inbox,
      removeTag: 'todo',
    })
  })

  it('a unified tag column refuses an account it leaves out', () => {
    const excluded = [{ id: 'acc1', provider: 'imap', auth_type: 'password', included_in_unified: false }]
    expect(resolveKanbanMove(inbox, todo, thread, excluded)).toEqual({
      kind: 'blocked',
      reasonKey: KANBAN_DROP_REASONS.savedSearchTarget,
    })
  })

  it('mail in the trash cannot join a tag column, which would not list it', () => {
    expect(resolveKanbanMove(inbox, todo, { ...thread, folder_role: 'trash' }, mailAccounts)).toEqual({
      kind: 'blocked',
      reasonKey: KANBAN_DROP_REASONS.savedSearchTarget,
    })
  })

  it("another account's tag column is refused", () => {
    const other = { accountId: 'acc2', folderId: 'INBOX', query: 'tag:todo' }
    expect(resolveKanbanMove(inbox, other, thread, mailAccounts)).toEqual({
      kind: 'blocked',
      reasonKey: KANBAN_DROP_REASONS.savedSearchTarget,
    })
  })
})

describe('resolveKanbanMove', () => {
  const mailAccounts = [
    { id: 'acc1', provider: 'imap', auth_type: 'password' },
    { id: 'acc2', provider: 'imap', auth_type: 'password' },
    { id: 'feed1', provider: 'rss', auth_type: 'rss' },
    { id: 'feed2', provider: 'rss', auth_type: 'rss' },
  ]
  const unified = { accountId: 'unified', folderId: 'inbox' }

  it('takes the origin from the card when dragging out of a unified column', () => {
    const thread = message({ account_id: 'acc2', folder_id: 'INBOX' })
    expect(resolveKanbanMove(unified, { accountId: 'acc1', folderId: 'Archive' }, thread, mailAccounts)).toEqual({
      kind: 'move',
      origin: { accountId: 'acc2', folderId: 'INBOX' },
    })
  })

  it('refuses a drop onto a saved search column', () => {
    const source = { accountId: 'acc1', folderId: 'Archive' }
    const target = { accountId: 'acc1', folderId: 'INBOX', query: 'invoice' }
    expect(resolveKanbanMove(source, target, message({}), mailAccounts)).toEqual({
      kind: 'blocked',
      reasonKey: KANBAN_DROP_REASONS.savedSearchTarget,
    })
  })

  it('uses the column itself as the origin for a single-account column', () => {
    const source = { accountId: 'acc1', folderId: 'INBOX' }
    expect(resolveKanbanMove(source, { accountId: 'acc1', folderId: 'Archive' }, message({}), mailAccounts)).toEqual({
      kind: 'move',
      origin: source,
    })
  })

  it('resolves the origin even when the card is missing from a single-account column', () => {
    const source = { accountId: 'acc1', folderId: 'INBOX' }
    expect(resolveKanbanMove(source, { accountId: 'acc2', folderId: 'INBOX' }, undefined, mailAccounts)).toEqual({
      kind: 'move',
      origin: source,
    })
  })

  it('does nothing when the card lands back on its own mailbox', () => {
    const thread = message({ account_id: 'acc1', folder_id: 'INBOX' })
    // Case-insensitively: a unified card reports "INBOX", the column stores "inbox".
    expect(resolveKanbanMove(unified, { accountId: 'acc1', folderId: 'inbox' }, thread, mailAccounts).kind).toBe('noop')
    const source = { accountId: 'acc1', folderId: 'INBOX' }
    expect(resolveKanbanMove(source, { accountId: 'acc1', folderId: 'INBOX' }, thread, mailAccounts).kind).toBe('noop')
  })

  it('refuses a unified column as the drop target', () => {
    expect(resolveKanbanMove({ accountId: 'acc1', folderId: 'INBOX' }, unified, message({}), mailAccounts)).toEqual({
      kind: 'blocked',
      reasonKey: KANBAN_DROP_REASONS.unifiedTarget,
    })
  })

  it('refuses a unified drag whose card it cannot find', () => {
    expect(resolveKanbanMove(unified, { accountId: 'acc1', folderId: 'Archive' }, undefined, mailAccounts)).toEqual({
      kind: 'blocked',
      reasonKey: KANBAN_DROP_REASONS.unknownOrigin,
    })
  })

  it('refuses feed/mail mixes in either direction, resolving the feed through a unified column', () => {
    const feedItem = message({ account_id: 'feed1', folder_id: 'inbox' })
    expect(resolveKanbanMove(unified, { accountId: 'acc1', folderId: 'Archive' }, feedItem, mailAccounts)).toEqual({
      kind: 'blocked',
      reasonKey: KANBAN_DROP_REASONS.feedToMail,
    })
    expect(
      resolveKanbanMove(
        { accountId: 'acc1', folderId: 'INBOX' },
        { accountId: 'feed1', folderId: 'inbox' },
        message({}),
        mailAccounts,
      ),
    ).toEqual({ kind: 'blocked', reasonKey: KANBAN_DROP_REASONS.mailToFeed })
  })

  it('allows a feed to move between RSS accounts', () => {
    const feedItem = message({ account_id: 'feed1', folder_id: 'inbox' })
    expect(resolveKanbanMove(unified, { accountId: 'feed2', folderId: 'inbox' }, feedItem, mailAccounts)).toEqual({
      kind: 'move',
      origin: { accountId: 'feed1', folderId: 'inbox' },
    })
  })
})

describe('restrictColumnDragToHorizontal', () => {
  const transform = { x: 40, y: 25, scaleX: 1, scaleY: 1 }
  const dragging = (type: string) => ({ data: { current: { type } } })

  it('pins a column to its row and leaves a card free', () => {
    expect(restrictColumnDragToHorizontal({ transform, active: dragging('column') })).toEqual({ ...transform, y: 0 })
    expect(restrictColumnDragToHorizontal({ transform, active: dragging('thread') })).toEqual(transform)
  })
})

describe('columnDropTargetClass', () => {
  it('marks a refused column, hovered or not', () => {
    expect(columnDropTargetClass(true, true)).toContain('red')
    expect(columnDropTargetClass(false, true)).toContain('opacity-60')
    expect(columnDropTargetClass(true)).toContain('accent')
    expect(columnDropTargetClass(false)).toBe('')
  })
})

describe('kanban move strings', () => {
  it('name locale keys that every catalog carries', () => {
    const keys = [...Object.values(KANBAN_DROP_REASONS), ...Object.values(KANBAN_MOVE_MESSAGES)]
    for (const key of keys) {
      expect(key.startsWith('kanban.')).toBe(true)
      // t() falls back to the key itself when nothing is catalogued. The value
      // covers the one message that interpolates an error.
      expect(t(key, { error: 'x' })).not.toBe(key)
    }
  })
})
