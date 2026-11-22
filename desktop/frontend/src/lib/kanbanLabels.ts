// Labels board columns and search scopes for display.
// The board uses the selected accounts and folders to describe each column.
// Unread counts and empty-state labels use the same visible row data.

import { useRef } from 'react'
import { useValue } from '@legendapp/state/react'
import { folderLeafName } from './folderTree'
import { isLocalAccountId } from '../states/accounts'
import { kanbanColumnKey, type KanbanColumn } from '../states/kanban'
import { mail$ } from '../states/mail'
import { unifiedFolderLabel } from './unifiedFolders'
import type { Account, Folder, Message } from '../types'
import { isRSSAccount, isUnifiedStarredColumn } from './kanbanDropTargets'

export function searchScopeColumn(scope: string): KanbanColumn | null {
  if (!scope || scope === 'all') return null
  const [accountId, folderId] = scope.split('\n')
  if (!accountId || !folderId) return null
  return { accountId, folderId }
}

// The columns the board search reads. A saved search column is one of them: the
// search narrows its own query (see narrowKanbanQuery). The scope is matched on
// the whole key, which only a saved search's query sets apart from its folder.
export function searchTargets(columns: KanbanColumn[], scope: string): KanbanColumn[] {
  if (!searchScopeColumn(scope)) return columns
  return columns.filter((column) => kanbanColumnKey(column) === scope)
}

// The full folder list used for column/scope labels: an account's own folders
// plus every account's folders (so a column can name a folder for an account
// other than the currently selected one).
export function mergeLabelFolders(folders: Folder[], foldersByAccount: Record<string, Folder[]>): Folder[] {
  return [...folders, ...Object.values(foldersByAccount).flat()]
}

// Read the per-account folder cache in a way memoization can trust. Legend-State
// mutates the raw object in place when a child node is set, so
// `useValue(mail$.foldersByAccount)` keeps handing back the *same* object
// reference after one account's list is replaced. The change does bubble up, so
// the component re-renders — but anything memoized on that reference (the column
// picker's folder tree, the label lookups) keeps its stale value, which is why a
// folder created in the app only showed up after a restart. Every write replaces
// an account's array wholesale, so comparing array identities is enough to know
// when to mint a fresh object.
export function useFoldersByAccount(): Record<string, Folder[]> {
  const byAccount = useValue(mail$.foldersByAccount)
  const snapshot = useRef<Record<string, Folder[]>>({})
  snapshot.current = nextFoldersSnapshot(snapshot.current, byAccount)
  return snapshot.current
}

// The comparison behind `useFoldersByAccount`, split out so it can be tested
// without a DOM renderer: a fresh object when any account's list was replaced (or
// an account came or went), the previous one otherwise so memos still hold.
export function nextFoldersSnapshot(
  previous: Record<string, Folder[]>,
  current: Record<string, Folder[]>,
): Record<string, Folder[]> {
  const keys = Object.keys(current)
  const changed = keys.length !== Object.keys(previous).length || keys.some((key) => previous[key] !== current[key])
  return changed ? { ...current } : previous
}

export function accountLabel(accountId: string, accounts: { id: string; email: string; display_name: string }[]) {
  if (accountId === 'unified') return 'Unified'
  const account = accounts.find((item) => item.id === accountId)
  return account?.display_name || account?.email || accountId
}

export function folderLabel(column: KanbanColumn, folders: Folder[], accounts: Account[], t: (key: string) => string) {
  const isInbox = column.folderId.toLowerCase() === 'inbox'
  // The unified view's synthetic folders are named after their role alone
  // ("Sent", "Archive") for the side nav, where the selected account already
  // says whose mail it is. Anywhere a column or header stands beside a single
  // account's, the qualified name is the one that reads unambiguously.
  if (column.accountId === 'unified') return unifiedFolderLabel(column.folderId, t)
  const isRSS = isRSSAccount(column.accountId, accounts)
  const found = folders.find((folder) => folder.account_id === column.accountId && folder.id === column.folderId)
  // Local folders have no Inbox: a view opened before any folder exists (or
  // after the last one went) is named after the account, not a missing Inbox.
  if (isLocalAccountId(column.accountId)) {
    return found?.name ? folderLeafName(found) : accountLabel(column.accountId, accounts)
  }
  if (isInbox) return isRSS ? 'Feed' : t('folders.roles.inbox')
  if (!found?.name) return column.folderId
  // A column names its folder without the path above it ("Now", not
  // "[Gmail]/Now"); the folder switcher's tree still shows where it sits. Feed
  // titles are not paths, so a "/" in one stays.
  return isRSS ? found.name : folderLeafName(found)
}

export function searchColumnLabel(
  column: KanbanColumn,
  folders: Folder[],
  accounts: Account[],
  t: (key: string) => string = (key) => key,
) {
  if (column.accountId === 'unified') return unifiedFolderLabel(column.folderId, t)
  return `${accountLabel(column.accountId, accounts)} / ${folderLabel(column, folders, accounts, t)}`
}

// Whether a column is in the active search's scope ("all" or this column's key).
export function columnSearchActive(key: string, searchQuery: string, searchScope: string): boolean {
  return !!searchQuery.trim() && (searchScope === 'all' || searchScope === key)
}

// Over a board wallpaper the usual translucent dark surface lets the image
// bleed through the column, so it swaps to a more opaque, blurred one.
export function columnSearchHighlightClass(active: boolean, overWallpaper = false): string {
  if (active) return 'border-accent/70 ring-2 ring-accent/20 bg-chats dark:bg-black/35'
  return overWallpaper
    ? 'border-border bg-raised/95 backdrop-blur-sm dark:bg-black/45'
    : 'border-border bg-raised dark:bg-black/20'
}

// Placeholder text for an empty column, branching on the active filter and
// whether a search hid otherwise-present threads.
export function columnEmptyText(
  filterMode: string,
  searchActive: boolean,
  hasRawThreads: boolean,
  isRss = false,
  attachmentsOnly = false,
): string {
  // RSS columns list feed subscriptions rather than mail threads, so the noun
  // shifts to match the user's mental model of the column.
  const noun = isRss ? 'feeds' : 'threads'
  // The attachments toggle narrows whatever the read-state filter shows.
  const withFiles = attachmentsOnly ? ' with attachments' : ''
  if (searchActive) {
    if (hasRawThreads && (filterMode !== 'all' || attachmentsOnly)) {
      if (filterMode === 'unread') return 'Matches hidden by Unread filter'
      if (filterMode === 'starred') return 'Matches hidden by Starred filter'
      return 'Matches hidden by Attachments filter'
    }
    if (filterMode === 'unread') return `No unread matches${withFiles}`
    if (filterMode === 'starred') return `No starred matches${withFiles}`
    return `No matches${withFiles}`
  }
  if (filterMode === 'unread') return `No unread ${noun}${withFiles}`
  if (filterMode === 'starred') return `No starred ${noun}${withFiles}`
  return `No ${noun}${withFiles}`
}

export function loadedUnreadCount(threads: Message[]): number {
  return threads.reduce((count, thread) => count + (thread.unread ? (thread.unread_count ?? 1) : 0), 0)
}

export function kanbanColumnUnreadCount(
  column: KanbanColumn,
  folderUnread: number | undefined,
  loadedThreads: Message[] = [],
): number {
  // The folder's total says nothing about how many of its matches are unread.
  if (isUnifiedStarredColumn(column) || column.query) return loadedUnreadCount(loadedThreads)
  return folderUnread ?? loadedUnreadCount(loadedThreads)
}
