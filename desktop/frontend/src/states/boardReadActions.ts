// Applies read-state changes to one column or a complete desktop board.
// The board resolves account and folder scopes before issuing the core commands.
// Pending updates retain the information needed to restore failed reads.

import type { Folder } from '../types'
import { showToast } from './ui'
import { mail$ } from './mail'
import {
  captureFolderUnreadVersion,
  holdFolderUnread,
  folderMatches,
  refreshAccountFoldersCache,
  updateCachedFolderUnread,
} from './mailFolders'
import { accounts$ } from './accounts'
import { isRssAccount } from '../lib/threadActions'
import { isUnifiedStarredColumn } from '../lib/kanbanData'
import { accountFolderForRole, unifiedFolderRole } from '../lib/unifiedFolders'
import { invoke } from '../lib/bridge'
import { t } from '../lib/i18n'
import { type KanbanColumn, kanbanColumnKey, kanban$, getKanbanColumns } from './kanbanState'

// Mark a column as read. Mail columns are marked folder-wide, so unread messages
// outside the loaded page are cleared too; RSS/starred aggregates fall back to
// per loaded item/thread operations because they have no folder-wide unread flag.
type ReadRequest = (command: string, payload: Record<string, string>) => Promise<void>

const requestRead: ReadRequest = async (command, payload) => {
  const result = await invoke<{ ok?: boolean; failures?: Array<{ message: string }> }>(command, payload)
  if (result?.ok === false || result?.failures?.length) {
    throw new Error(result.failures?.map((failure) => failure.message).join('; ') || t('notification.markReadFailed'))
  }
}

async function markColumnRead(column: KanbanColumn, request: ReadRequest, baseline?: Record<string, Folder[]>) {
  const key = kanbanColumnKey(column)
  const threads = kanban$.threads[key].get() ?? []
  const unread = threads.filter((thread) => thread.unread)
  const accounts = accounts$.get()
  const starred = isUnifiedStarredColumn(column)
  // A saved search marks only its matches read, never the whole folder, so it
  // goes thread by thread like the starred aggregate.
  const perItem = starred || !!column.query
  const columnAccount = accounts.find((account) => account.id === column.accountId)
  const mailAccountIds = perItem
    ? []
    : column.accountId === 'unified'
      ? accounts
          .filter((account) => account.included_in_unified !== false && !isRssAccount(account, account.id))
          .map((account) => account.id)
      : !isRssAccount(columnAccount, column.accountId)
        ? [column.accountId]
        : []
  const itemTargets = perItem
    ? unread
    : unread.filter((thread) =>
        isRssAccount(
          accounts.find((account) => account.id === thread.account_id),
          thread.account_id,
        ),
      )
  const writes = [
    ...(mailAccountIds.length
      ? [() => request('mail.markAllRead', { account_id: column.accountId, folder_id: column.folderId })]
      : []),
    ...itemTargets.map((thread) => () => request('mail.markRead', { thread_id: thread.thread_id })),
  ]
  // The side navigation reads its badges from the folder cache, not from the
  // column, so clearing only `unreadCounts` left the nav counts stale until the
  // folder refresh below came back from the server. Work out the same folders
  // the writes cover and their pre-write totals, to clear now and put back if a
  // write fails: a mail account's folder goes to zero (the write is
  // folder-wide), an RSS folder loses only the items marked here.
  const folderTargets = new Map<string, { accountId: string; folderId: string; previous: number; next: number }>()
  const role = column.accountId === 'unified' ? unifiedFolderRole(column.folderId) : ''
  const targetFolder = (accountId: string, folderId: string | undefined) => {
    if (!accountId || accountId === 'unified' || !folderId) return undefined
    const folders = mail$.foldersByAccount[accountId].get() ?? []
    const resolved = folders.find((folder) => folderMatches(folder, accountId, folderId))
    if (!resolved) return undefined
    const key = `${accountId}\n${resolved.id}`
    let target = folderTargets.get(key)
    if (!target) {
      // The columns of one board clear in turn without awaiting, so by the time
      // a later column reads the cache an earlier one sharing this folder has
      // already zeroed it. `baseline` is the board's pre-clear snapshot, so both
      // columns roll back to the count the folder really had; the optimistic
      // number still counts down from what is on screen now.
      const before = baseline?.[accountId]?.find((folder) => folderMatches(folder, accountId, folderId))
      target = { accountId, folderId: resolved.id, previous: before?.unread ?? resolved.unread, next: resolved.unread }
      folderTargets.set(key, target)
    }
    return target
  }
  for (const accountId of mailAccountIds) {
    const folderId = role ? accountFolderForRole(mail$.foldersByAccount[accountId].get(), role) : column.folderId
    const target = targetFolder(accountId, folderId)
    if (target) target.next = 0
  }
  for (const thread of itemTargets) {
    const target = targetFolder(thread.account_id, thread.folder_id)
    if (target) target.next = Math.max(0, target.next - 1)
  }
  // Optimistic: clear the cards before any await, so a board-wide mark flips
  // every column at once. A failure puts back only the rows cleared here (by id,
  // so a page load that landed meanwhile survives) and this column's badge.
  const previousById = new Map(unread.map((thread) => [thread.thread_id, thread]))
  const previousUnreadCount = kanban$.unreadCounts[key].get()
  const clearsBadge = !starred && writes.length > 0
  kanban$.threads[key].set(
    threads.map((thread) =>
      previousById.has(thread.thread_id) ? { ...thread, unread: false, unread_count: 0 } : thread,
    ),
  )
  if (clearsBadge) kanban$.unreadCounts[key].set(0)
  for (const target of folderTargets.values())
    updateCachedFolderUnread(target.accountId, target.folderId, target.next, 'local')
  const settleCounts = new Map(
    Array.from(folderTargets.values(), (target) => [target, holdFolderUnread(target.accountId, target.folderId)]),
  )
  const results = await Promise.allSettled(writes.map((write) => write()))
  const refreshed = new Map<string, Folder[]>()
  const refreshVersions = new Map<string, number>()
  await Promise.all(
    Array.from(new Set([...mailAccountIds, ...itemTargets.map((thread) => thread.account_id)]))
      .filter(Boolean)
      .map(async (accountId) => {
        refreshVersions.set(accountId, captureFolderUnreadVersion())
        refreshed.set(accountId, await refreshAccountFoldersCache(accountId, false))
      }),
  )
  const failure = results.find((result) => result.status === 'rejected')
  if (failure?.status === 'rejected') {
    kanban$.threads[key].set(
      (kanban$.threads[key].get() ?? []).map((thread) => {
        const previous = previousById.get(thread.thread_id)
        return previous ? { ...thread, unread: previous.unread, unread_count: previous.unread_count } : thread
      }),
    )
    if (clearsBadge) kanban$.unreadCounts[key].set(previousUnreadCount)
    for (const target of folderTargets.values()) {
      // One failed write does not mean every folder is still unread: the
      // refresh above answered with the server's own counts for the accounts it
      // reached — including those a sibling write did mark read — so put back
      // the pre-write count only where it did not answer.
      const server = refreshed
        .get(target.accountId)
        ?.find((folder) => folderMatches(folder, target.accountId, target.folderId))
      settleCounts.get(target)!(
        server?.unread ?? target.previous,
        !!server,
        server ? refreshVersions.get(target.accountId) : undefined,
      )
    }
    throw failure.reason
  }
  for (const target of folderTargets.values()) {
    const server = refreshed
      .get(target.accountId)
      ?.find((folder) => folderMatches(folder, target.accountId, target.folderId))
    settleCounts.get(target)!(
      server?.unread ?? target.next,
      true,
      server ? refreshVersions.get(target.accountId) : undefined,
    )
  }
  // Reapply on success: a column reload that raced the write may have put the
  // pre-write rows and badge back over the optimistic clear.
  kanban$.threads[key].set(
    (kanban$.threads[key].get() ?? []).map((thread) =>
      previousById.has(thread.thread_id) ? { ...thread, unread: false, unread_count: 0 } : thread,
    ),
  )
  if (clearsBadge) kanban$.unreadCounts[key].set(0)
}

export async function markColumnAllRead(column: KanbanColumn) {
  try {
    await markColumnRead(column, requestRead)
  } catch {
    showToast(t('notification.markReadFailed'), 'error')
  }
}

export async function markBoardAllRead(boardId: string) {
  const columns = Array.from(
    new Map(getKanbanColumns(boardId).map((column) => [kanbanColumnKey(column), column])).values(),
  )
  const requests = new Map<string, Promise<void>>()
  const request: ReadRequest = (command, payload) => {
    // Reuse a unified folder write for a concrete column covering the same folder.
    // A saved unified search marks only its matches, so it never stands in.
    if (command === 'mail.markAllRead' && payload.account_id !== 'unified') {
      const account = accounts$.get().find((account) => account.id === payload.account_id)
      const folder = mail$.foldersByAccount[payload.account_id].get()?.find((folder) => folder.id === payload.folder_id)
      const role = folder?.role || (payload.folder_id.toLowerCase() === 'inbox' ? 'inbox' : '')
      const unified = columns.find(
        (column) =>
          column.accountId === 'unified' &&
          !column.query &&
          column.folderId.toLowerCase() === role &&
          !isUnifiedStarredColumn(column),
      )
      if (account && account.included_in_unified !== false && unified) {
        payload = { account_id: 'unified', folder_id: unified.folderId }
      }
    }
    const key = JSON.stringify([command, payload])
    let pending = requests.get(key)
    if (!pending) {
      pending = requestRead(command, payload)
      requests.set(key, pending)
    }
    return pending
  }
  const baseline = mail$.foldersByAccount.get()
  const results = await Promise.allSettled(columns.map((column) => markColumnRead(column, request, baseline)))
  if (results.some((result) => result.status === 'rejected')) {
    showToast(t('notification.markReadFailed'), 'error')
  }
}
