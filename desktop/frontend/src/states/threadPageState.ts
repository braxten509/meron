// Tracks pending removals and reconciles overlapping thread-page reads.
// A read retains the identities that disappeared while it was in progress.
// Refresh merges stable rows and selects the next visible conversation.

import { ui$ } from './ui'
import type { Message } from '../types'

// Suppress removals only while the backend mutation is pending. Each list read
// keeps its own snapshot, including removals that start while it is in flight.
// Completing a mutation releases future reads, while older reads still cannot
// paint its stale row back. The core's UID guards protect the committed cache.
const pendingRemovals = new Map<string, symbol>()

const activeThreadListReads = new Set<Map<string, symbol>>()

export function beginThreadListRead() {
  const removed = new Map(pendingRemovals)
  activeThreadListReads.add(removed)
  return {
    filter: (threads: Message[]) =>
      threads.filter((thread) => !removed.has(thread.thread_id) && !pendingRemovals.has(thread.thread_id)),
    dispose: () => {
      activeThreadListReads.delete(removed)
    },
  }
}

export type ThreadListRead = ReturnType<typeof beginThreadListRead>

export function suppressRemovedThread(threadId: string): () => void {
  if (!threadId) return () => {}
  const token = Symbol(threadId)
  pendingRemovals.set(threadId, token)
  for (const read of activeThreadListReads) read.set(threadId, token)
  return () => {
    if (pendingRemovals.get(threadId) === token) pendingRemovals.delete(threadId)
    for (const read of activeThreadListReads) {
      if (read.get(threadId) === token) read.delete(threadId)
    }
  }
}

// The backend has removed the source rows. Reads begun afterwards may show a
// new reply or a moved-back copy of this conversation; older reads stay guarded.
export function completeRemovedThread(threadId: string) {
  pendingRemovals.delete(threadId)
}

export function releaseRemovedThread(threadId: string) {
  pendingRemovals.delete(threadId)
  for (const read of activeThreadListReads) read.delete(threadId)
}

// `mail.threadList` uses this when the client doesn't pass a limit. The
// background merge needs it to tell a full first page from the end of the folder.
export const THREAD_LIST_PAGE_SIZE = 50

// A background refresh only re-reads the first page. Threads it no longer
// returns left that page (archived, moved, deleted). Threads older than the
// page are ones the reader scrolled to, and this refresh didn't look at them.
// A short page still has those when the server sent a cursor — the unified
// inbox stops at a date cutoff before the page is full. Only a short page with
// nowhere further to read has covered the folder, and then a missing row is
// gone. Threads that slide into the page, or that actually just arrived, join
// by date. Putting them on the front is how an older conversation from another
// account landed above newer mail after an archive.
export function mergeRefreshedThreadPage(
  previous: Message[],
  fetched: Message[],
  hasMore = false,
  pinned: Record<string, boolean> = {},
): Message[] {
  const fetchedById = new Map(fetched.map((thread) => [thread.thread_id, thread]))
  const previousIds = new Set(previous.map((thread) => thread.thread_id))
  const brandNew = fetched.filter((thread) => !previousIds.has(thread.thread_id))
  const oldest = Math.min(...fetched.map((thread) => thread.date))
  const pageCoversRest = fetched.length < THREAD_LIST_PAGE_SIZE && !hasMore
  const kept = previous.filter((thread) => {
    // A filtered page cannot distinguish a pinned row's flag change from an
    // external move. Keep triage pins until the user reloads this view.
    if (fetchedById.has(thread.thread_id) || pinned[thread.thread_id]) return true
    if (pageCoversRest) return false
    // Date alone cannot place equal-timestamp rows on either side of this
    // boundary. Conservatively keep them until a full user-initiated reload.
    return thread.date <= oldest
  })
  const updated = kept.map((thread) => fetchedById.get(thread.thread_id) ?? thread)
  const merged = brandNew.length > 0 ? [...brandNew, ...updated] : updated
  return merged.length < 2 ? merged : merged.sort((a, b) => b.date - a.date)
}

// The open conversation left the list this refresh just wrote. Follow it to the
// next row that is still on screen (or the previous one, if it was last) so a
// burst of archives doesn't land on a blank pane. A selection that was never in
// this list — a notification that hasn't landed — stays where it is.
export function nextSelectedAfterRefresh(previous: Message[], visible: Message[], selectedId: string): string {
  if (!selectedId || visible.some((thread) => thread.thread_id === selectedId)) return selectedId
  const index = previous.findIndex((thread) => thread.thread_id === selectedId)
  if (index === -1) return selectedId
  const visibleIds = new Set(visible.map((thread) => thread.thread_id))
  const after = previous.slice(index + 1).find((thread) => visibleIds.has(thread.thread_id))
  if (after) return after.thread_id
  const before = previous
    .slice(0, index)
    .reverse()
    .find((thread) => visibleIds.has(thread.thread_id))
  return before?.thread_id ?? visible[0]?.thread_id ?? ''
}

// Identity of a thread-list view: the four inputs `loadThreads` reads and
// `superseded` watches, newline-joined like `kanbanColumnKey` (neither a folder
// id nor a single-line search box carries one). Both the loader and the list
// build the key from the same fields, so the list can tell "these rows are for
// what I'm showing" from "these rows are the previous view's".
export function threadListViewKey(account: string, folder: string, query: string, filter: string) {
  return [account, folder, query, filter].join('\n')
}

/** The list's filter inputs as one comparable value — the read-state filter,
 * the attachments toggle and the sender grouping — for view keys and staleness
 * checks. Sender rows and thread rows are different lists, so the grouping is
 * part of the key: a view's rows never answer for the other mode. */
export function listFilterKey(
  mode = ui$.filterMode.get(),
  attachments = ui$.attachmentsOnly.get(),
  groupBySender = ui$.groupBySender.get(),
) {
  const key = attachments ? `${mode}+attachments` : mode
  return groupBySender ? `${key}+sender` : key
}

/** The `group_by` the list asks the core for. The core applies it to the plain
 * list only; search, Starred and Drafts stay threaded whatever is sent. */
export function listGroupBy(groupBySender = ui$.groupBySender.get()): 'sender' | 'thread' {
  return groupBySender ? 'sender' : 'thread'
}
