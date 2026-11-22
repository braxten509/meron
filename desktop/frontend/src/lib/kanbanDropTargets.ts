// Resolves drag targets and mail actions for desktop board columns.
// Folder roles, search criteria and account identity determine the permitted move.
// The board displays the corresponding target style and reason.

import { type KanbanColumn } from '../states/kanban'
import type { Message } from '../types'

// A column a drag can't land in is drawn as refused rather than inviting, so the
// user sees the "no" before letting go; the reason itself is shown on hover and
// repeated as a toast if they drop anyway.
export function columnDropTargetClass(isOver: boolean, blocked = false): string {
  if (blocked) {
    return isOver
      ? 'border-red-400 bg-red-500/10 ring-2 ring-red-400/40 dark:border-red-900/70'
      : 'border-dashed opacity-60'
  }
  return isOver ? 'border-accent bg-accent/10 ring-2 ring-accent/35 dark:bg-accent/15' : ''
}

// A column only ever changes slots sideways. Left free, a column dragged past
// the board's bottom edge grows the scroll area and auto-scroll chases it. Cards
// share the DndContext and still move freely.
export function restrictColumnDragToHorizontal<T extends { x: number; y: number }>(args: {
  transform: T
  active: { data: { current?: Record<string, unknown> } } | null
}): T {
  if (args.active?.data.current?.type !== 'column') return args.transform
  return { ...args.transform, y: 0 }
}

/**
 * Why a drop was refused, as locale keys — the reason is shown on the column
 * while dragging and toasted if the card is dropped anyway, so it is translated
 * where it is rendered rather than here (this stays pure and testable).
 */
export const KANBAN_DROP_REASONS = {
  unifiedTarget: 'kanban.drop.unifiedTarget',
  savedSearchTarget: 'kanban.drop.savedSearchTarget',
  unknownOrigin: 'kanban.drop.unknownOrigin',
  feedToMail: 'kanban.drop.feedToMail',
  mailToFeed: 'kanban.drop.mailToFeed',
} as const

/**
 * The locale keys the kanban move path toasts once a drop is under way. Kept
 * beside the refusal keys so both sets are covered by the same catalogue test.
 */
export const KANBAN_MOVE_MESSAGES = {
  moved: 'kanban.move.moved',
  failed: 'kanban.move.failed',
  copyFailed: 'kanban.move.copyFailed',
  noMatchingMessages: 'kanban.move.noMatchingMessages',
  copiedNotRemoved: 'kanban.move.copiedNotRemoved',
  copiedNotRemovedReason: 'kanban.move.copiedNotRemovedReason',
} as const

/** The tag a column lists, when its saved search is exactly one `tag:` term. */
export function columnTag(column: KanbanColumn): string | undefined {
  // The unified Starred view filters its own list: a tag alone can't join it.
  if (isUnifiedStarredColumn(column)) return undefined
  return column.query?.match(/^tag:([^\s"]+)$/)?.[1]?.toLowerCase()
}

/** Whether a column's saved search uses tags at all, alone or among other terms. */
export function columnReadsTags(column: KanbanColumn): boolean {
  return /(^|\s)tag:\S/i.test(column.query ?? '')
}

/**
 * What dropping `thread` from `source` onto `target` would do: the move and the mailbox it leaves
 * from, nothing, or a refusal to explain. A unified column names a role across accounts rather than
 * one mailbox, so a thread dragged out of one takes its origin from the card itself; every other
 * column is its own origin. Pure, so the drop handler and the drag-time column styling agree.
 */
export type KanbanMoveResolution =
  | { kind: 'move'; origin: KanbanColumn; removeTag?: string }
  | { kind: 'tag'; add?: string; remove?: string }
  | { kind: 'noop' }
  | { kind: 'blocked'; reasonKey: string }

export function resolveKanbanMove(
  source: KanbanColumn,
  target: KanbanColumn,
  thread: (Pick<Message, 'account_id' | 'folder_id'> & Partial<Pick<Message, 'folder_role'>>) | undefined,
  accounts: { id: string; provider: string; auth_type: string; included_in_unified?: boolean }[],
): KanbanMoveResolution {
  const sameColumn = (a: KanbanColumn, b: KanbanColumn) =>
    a.accountId === b.accountId && folderMatches(a.folderId, b.folderId)
  const sourceTag = columnTag(source)
  const targetTag = columnTag(target)
  // A tag column is a state rather than a place: dropping on it tags the thread
  // and leaves the mail where it is, taking the tag of the column it came from.
  if (targetTag) {
    if (targetTag === sourceTag) return { kind: 'noop' }
    if (!thread) return { kind: 'blocked', reasonKey: KANBAN_DROP_REASONS.unknownOrigin }
    if (isRSSAccount(thread.account_id, accounts)) {
      return { kind: 'blocked', reasonKey: KANBAN_DROP_REASONS.feedToMail }
    }
    // A column that would never list it: another account's, a unified one this
    // account stays out of, or any of them for mail thrown away, which tag
    // searches skip.
    const discarded = thread.folder_role === 'trash' || thread.folder_role === 'junk'
    const listed =
      target.accountId === 'unified'
        ? accounts.find((account) => account.id === thread.account_id)?.included_in_unified !== false
        : target.accountId === thread.account_id
    if (!listed || discarded) return { kind: 'blocked', reasonKey: KANBAN_DROP_REASONS.savedSearchTarget }
    return { kind: 'tag', add: targetTag, remove: sourceTag }
  }
  if (target.accountId === 'unified') return { kind: 'blocked', reasonKey: KANBAN_DROP_REASONS.unifiedTarget }
  // A saved search only lists matches; a card dropped on it might not be one.
  if (target.query) return { kind: 'blocked', reasonKey: KANBAN_DROP_REASONS.savedSearchTarget }
  // A tag column spans folders, so like a unified one it leaves the origin to
  // the card. Dropped on its own folder, leaving the column is all that's left.
  if (sourceTag) {
    if (!thread) return { kind: 'blocked', reasonKey: KANBAN_DROP_REASONS.unknownOrigin }
    const origin = { accountId: thread.account_id, folderId: thread.folder_id }
    if (sameColumn(origin, target)) return { kind: 'tag', remove: sourceTag }
    if (isRSSAccount(target.accountId, accounts)) {
      return { kind: 'blocked', reasonKey: KANBAN_DROP_REASONS.mailToFeed }
    }
    return { kind: 'move', origin, removeTag: sourceTag }
  }
  if (sameColumn(source, target)) return { kind: 'noop' }
  if (source.accountId === 'unified' && !thread) {
    return { kind: 'blocked', reasonKey: KANBAN_DROP_REASONS.unknownOrigin }
  }
  const origin: KanbanColumn =
    source.accountId === 'unified' && thread ? { accountId: thread.account_id, folderId: thread.folder_id } : source
  // The card's own mailbox can be the column it was dropped on — dragging out of
  // Unified Inbox onto that same account's Inbox column.
  if (sameColumn(origin, target)) return { kind: 'noop' }
  const originRSS = isRSSAccount(origin.accountId, accounts)
  const targetRSS = isRSSAccount(target.accountId, accounts)
  if (originRSS && !targetRSS) return { kind: 'blocked', reasonKey: KANBAN_DROP_REASONS.feedToMail }
  if (!originRSS && targetRSS) return { kind: 'blocked', reasonKey: KANBAN_DROP_REASONS.mailToFeed }
  return { kind: 'move', origin }
}

// The sidecar reports the canonical IMAP folder ("INBOX", actual name); match it
// against a stored column folderId, treating the inbox role case-insensitively.
export function folderMatches(folderId: string, synced: string | undefined): boolean {
  if (!synced) return false
  const norm = (value: string) => (value.toLowerCase() === 'inbox' ? 'inbox' : value)
  return norm(folderId) === norm(synced)
}

export function isRSSAccount(accountId: string, accounts: { id: string; provider: string; auth_type: string }[]) {
  const account = accounts.find((item) => item.id === accountId)
  return accountId.startsWith('rss-') || account?.provider === 'rss' || account?.auth_type === 'rss'
}

export function isUnifiedStarredColumn(column: KanbanColumn): boolean {
  return column.accountId === 'unified' && column.folderId.toLowerCase() === 'starred'
}
