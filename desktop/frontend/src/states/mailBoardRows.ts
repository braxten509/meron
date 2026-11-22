// Updates the board rows and folder counts shared with mailbox views.
// Thread actions capture the affected columns before applying local changes.
// A failed action can restore these snapshots or reload the affected cards.

import type { Folder, Message } from '../types'
import { kanban$ } from './kanban'
import { mail$ } from './mailState'

// Optimistic rollback for the keyed caches, key by key. Two reasons not to keep a
// whole `.get()` and restore that: Legend-State mutates a record's raw object in
// place when a *child* node is set (`kanban$.threads[key]`,
// `mail$.foldersByAccount[accountId]`), so the "previous state" keeps changing
// under us; and even a true whole-record snapshot would undo writes that landed
// for *other* keys while the mutation was in flight — a folder LIST for another
// account, another column's page. So each flow captures only the keys it is about
// to touch and puts back exactly those. Plain arrays (`mail$.threads`,
// `mail$.messages`, `mail$.folders`) are always replaced whole, so a bare `.get()`
// is already a snapshot there.
type KeyEntries<T> = [key: string, value: T | undefined][]

export function captureKeys<T>(record: Record<string, T>, keys: string[]): KeyEntries<T> {
  return keys.map((key) => [key, record[key]])
}

/** Kanban columns holding a card for this thread: all `updateKanbanThread` and
 * `removeKanbanThread` can touch, so all a rollback has to put back. */
export function kanbanKeysWithThread(threadId: string): string[] {
  return Object.entries(kanban$.threads.get())
    .filter(([, threads]) => threads.some((thread) => thread.thread_id === threadId))
    .map(([key]) => key)
}

export function restoreAccountFolders(entries: KeyEntries<Folder[]>) {
  for (const [accountId, folders] of entries) {
    if (folders === undefined) mail$.foldersByAccount[accountId].delete()
    else mail$.foldersByAccount[accountId].set(folders)
  }
}

export function restoreKanbanColumns(threads: KeyEntries<Message[]>, unreadCounts: KeyEntries<number> = []) {
  for (const [key, columnThreads] of threads) {
    if (columnThreads === undefined) kanban$.threads[key].delete()
    else kanban$.threads[key].set(columnThreads)
  }
  for (const [key, count] of unreadCounts) {
    if (count === undefined) kanban$.unreadCounts[key].delete()
    else kanban$.unreadCounts[key].set(count)
  }
}

// The one record a flow does restore whole: `loadThreads` owns the entire cursor
// set for the query it is loading, and clears it outright, so there are no other
// keys to preserve.
export function snapshotRecord<T>(record: Record<string, T>): Record<string, T> {
  return { ...record }
}

export function updateKanbanThread(threadId: string, update: (thread: Message) => Message) {
  const columns = kanban$.threads.get()
  const unreadCounts = kanban$.unreadCounts.get()
  const nextUnreadCounts = { ...unreadCounts }
  let changed = false
  const nextColumns = Object.fromEntries(
    Object.entries(columns).map(([key, threads]) => {
      let columnChanged = false
      const nextThreads = threads.map((thread) => {
        if (thread.thread_id !== threadId) return thread
        columnChanged = true
        const next = update(thread)
        const beforeUnread = thread.unread ? (thread.unread_count ?? 1) : 0
        const afterUnread = next.unread ? (next.unread_count ?? 1) : 0
        if (nextUnreadCounts[key] !== undefined) {
          nextUnreadCounts[key] = Math.max(0, nextUnreadCounts[key] + afterUnread - beforeUnread)
        }
        return next
      })
      if (columnChanged) changed = true
      return [key, columnChanged ? nextThreads : threads]
    }),
  )
  if (changed) {
    kanban$.threads.set(nextColumns)
    kanban$.unreadCounts.set(nextUnreadCounts)
  }
}

export function removeKanbanThread(threadId: string) {
  const columns = kanban$.threads.get()
  let changed = false
  const nextColumns = Object.fromEntries(
    Object.entries(columns).map(([key, threads]) => {
      const nextThreads = threads.filter((thread) => thread.thread_id !== threadId)
      if (nextThreads.length !== threads.length) changed = true
      return [key, nextThreads]
    }),
  )
  if (changed) kanban$.threads.set(nextColumns)
}

export function findLocalThread(threadId: string): Message | undefined {
  const thread = mail$.threads.get().find((item) => item.thread_id === threadId)
  if (thread) return thread
  for (const threads of Object.values(kanban$.threads.get())) {
    const match = threads.find((item) => item.thread_id === threadId)
    if (match) return match
  }
  return mail$.messages.get().find((item) => item.thread_id === threadId)
}
