// Holds desktop board state while a window is open.
// Saved board definitions supply the columns and their identities.
// Folder queries share the same keys as the readers that update each column.

import { observable } from '@legendapp/state'
import type { Message } from '../types'
import { type FilterMode } from './ui'
import { settings$ } from './settings'

export type KanbanColumn = {
  accountId: string
  folderId: string
  /** Set on a saved search column: it always lists this folder's matches. */
  query?: string
}

export const DEFAULT_BOARD_NAME = 'Kanban board'

// Ephemeral kanban runtime state. Board definitions and pane/column sizes live
// in `settings$`; thread caches and open conversation state are per session.
export const kanban$ = observable({
  activeBoardId: '',
  threads: {} as Record<string, Message[]>,
  // Authoritative folder unread totals returned with each column's thread page.
  unreadCounts: {} as Record<string, number>,
  loading: {} as Record<string, boolean>,
  // Pagination cursors per column key. `cursors` holds a single-account folder's
  // opaque next-page cursor; `accountCursors` holds the per-account cursors a
  // unified column needs (one inbox per account). "" / empty = no more pages.
  cursors: {} as Record<string, string>,
  accountCursors: {} as Record<string, Record<string, string>>,
  // Cards a reload read below the oldest card that was on screen, per column
  // key: the next "load more" shows them before reading on from the cursor.
  heldThreads: {} as Record<string, Message[]>,
  loadingMore: {} as Record<string, boolean>,
  // Per-column thread filter (keyed by kanbanColumnKey). When set, overrides the
  // board-wide globalFilter for that column; unset columns follow globalFilter.
  filters: {} as Record<string, FilterMode>,
  // Board-wide filter from the header switch; the default for columns that don't
  // have their own filter set.
  globalFilter: 'all' as FilterMode,
  // Per-column attachments toggle (keyed by kanbanColumnKey), overriding
  // globalAttachmentsOnly for that column the same way `filters` does.
  attachmentsOnly: {} as Record<string, boolean>,
  // Board-wide attachments toggle, independent of the read-state filters above
  // and the default for columns without their own toggle set.
  globalAttachmentsOnly: false,
  searchQuery: '',
  searchScope: 'all',
  movingThread: '',
  // The card conversation in the pane, behind any tabs (see kanbanPaneThreadId).
  paneThreadId: '',
  // The column of the card last opened, as a conversation or in a tab: j/k
  // steps through it first.
  paneColumnKey: '',
})

// A saved search is its own column beside the folder's plain one, so its query is
// part of the key. Splitting a key on "\n" still yields account and folder first.
export function kanbanColumnKey(column: KanbanColumn): string {
  const base = `${column.accountId}\n${column.folderId}`
  return column.query ? `${base}\n${column.query}` : base
}

// The board search inside a saved search column narrows it. The core reads each
// side of an `AND` as its own query that must also match, so free text, dates
// and read state narrow the saved query rather than merging into or replacing
// its own. The search starts a line of its own, which no quote or group left
// open in the saved query reaches into. A leading `OR` would be a stray word
// there, so it is dropped.
export function narrowKanbanQuery(saved: string | undefined, query: string): string {
  if (!saved) return query.trim()
  const extra = query.trim().replace(/^(?:OR(?:\s+|$))+/, '')
  return extra ? `${saved}\nAND ${extra}` : saved
}

export function kanbanBoardColumnKey(boardId: string, column: KanbanColumn): string {
  return `${boardId}\n${kanbanColumnKey(column)}`
}

export function getKanbanColumns(boardId: string): KanbanColumn[] {
  return settings$.kanbanBoards.get().find((board) => board.id === boardId)?.columns ?? []
}

export function getAllKanbanColumns(): KanbanColumn[] {
  return settings$.kanbanBoards.get().flatMap((board) => board.columns)
}
