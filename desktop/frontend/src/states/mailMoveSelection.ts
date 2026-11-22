// Updates mailbox and board selection while a conversation moves.
// The current pane chooses a neighboring thread and retains the removed card for undo.
// Background refresh requests are coalesced after the optimistic state update.

import type { Message } from '../types'
import { ui$ } from './ui'
import { kanban$ } from './kanban'
import {
  captureKeys,
  getFilteredThreads,
  kanbanKeysWithThread,
  loadThreads,
  mail$,
  removeKanbanThread,
  completeRemovedThread,
  suppressRemovedThread,
  requestThreadReselect,
} from './mail'
import { loadFolders } from './mailFolders'

// Neighbour of a thread inside the kanban column that holds it (next, or
// previous if it was last). The chat-view getFilteredThreads list doesn't apply
// in kanban, where cards live in per-column lists.
function threadsInPaneColumn(): Message[] {
  const paneKey = kanban$.paneColumnKey.get()
  const split = paneKey.indexOf('\n')
  if (split === -1) return []
  return kanban$.threads[paneKey.slice(split + 1)].get() ?? []
}

function kanbanNeighbourThreadId(threadId: string): string {
  for (const threads of Object.values(kanban$.threads.get())) {
    const index = threads.findIndex((thread) => thread.thread_id === threadId)
    if (index === -1) continue
    const neighbour = threads[index + 1] ?? threads[index - 1]
    return neighbour?.thread_id ?? ''
  }
  // The open card is already off the board (a column reload dropped it). Stay
  // on a card still in that column instead of closing the conversation.
  return threadsInPaneColumn().find((thread) => thread.thread_id !== threadId)?.thread_id ?? ''
}

export function restoreRemovedThread(current: Message[], previous: Message[], threadId: string): Message[] {
  const thread = previous.find((item) => item.thread_id === threadId)
  if (!thread || current.some((item) => item.thread_id === threadId)) return current
  const positions = new Map(previous.map((item, index) => [item.thread_id, index]))
  return [...current, thread].sort(
    (a, b) =>
      b.date - a.date ||
      (positions.get(a.thread_id) ?? previous.length) - (positions.get(b.thread_id) ?? previous.length),
  )
}

export function removeThreadLocally(threadId: string) {
  const previousThreads = mail$.threads.get()
  const previousMessages = mail$.messages.get()
  const previousKanbanThreads = captureKeys(kanban$.threads.get(), kanbanKeysWithThread(threadId))
  const previousSelected = ui$.selectedThread.get()
  const previousPaneThreadId = kanban$.paneThreadId.get()
  // When the selected thread is the one leaving, advance to its neighbour in the
  // visible list (next, or previous if it was last) instead of snapping back to
  // the top — this keeps keyboard triage (e/# on the j/k selection) in place.
  // Computed before mutating mail$.threads, since getFilteredThreads reads it.
  let nextSelected = previousSelected
  if (previousSelected === threadId) {
    if (kanban$.activeBoardId.get()) {
      nextSelected = kanbanNeighbourThreadId(threadId)
    } else {
      const visible = getFilteredThreads()
      const index = visible.findIndex((thread) => thread.thread_id === threadId)
      // Skip over rows of the deleted thread itself — the unified starred folder
      // can list the same thread once per folder it is starred in. When the
      // open row is already gone (a refresh dropped it mid-burst), stay on the
      // first row still on screen instead of clearing the conversation.
      const neighbour =
        index === -1
          ? visible.find((item) => item.thread_id !== threadId)
          : (visible.slice(index + 1).find((item) => item.thread_id !== threadId) ??
            visible
              .slice(0, index)
              .reverse()
              .find((item) => item.thread_id !== threadId))
      nextSelected = neighbour?.thread_id ?? ''
    }
  }
  const nextThreads = previousThreads.filter((thread) => thread.thread_id !== threadId)
  const unsuppress = suppressRemovedThread(threadId)

  mail$.threads.set(nextThreads)
  mail$.messages.set(previousMessages.filter((message) => message.thread_id !== threadId))
  removeKanbanThread(threadId)
  if (previousSelected === threadId) {
    ui$.selectedThread.set(nextSelected)
    // No neighbour in the list we have: let the fresh list pick the replacement.
    if (!nextSelected) requestThreadReselect()
  }
  // If the kanban conversation pane was open on the deleted card, follow the
  // selection so it doesn't keep rendering the removed thread.
  if (previousPaneThreadId === threadId) {
    kanban$.paneThreadId.set(nextSelected)
  }

  return {
    complete: () => completeRemovedThread(threadId),
    rollback: () => {
      unsuppress()
      // Put this thread back only. Restoring the whole list would resurrect
      // threads archived after this one started, which is what a fast burst
      // looks like when an earlier move fails.
      mail$.threads.set(restoreRemovedThread(mail$.threads.get(), previousThreads, threadId))
      const currentMessages = mail$.messages.get()
      const seenMessages = new Set(currentMessages.map((message) => message.id))
      const missingMessages = previousMessages.filter(
        (message) => message.thread_id === threadId && !seenMessages.has(message.id),
      )
      if (missingMessages.length > 0) mail$.messages.set([...currentMessages, ...missingMessages])
      for (const [key, columnThreads] of previousKanbanThreads) {
        if (!columnThreads) continue
        kanban$.threads[key].set(restoreRemovedThread(kanban$.threads[key].get() ?? [], columnThreads, threadId))
      }
      // The reader may already have archived the neighbour. Only come back
      // here when the selection is still the one this removal chose.
      if (ui$.selectedThread.get() === nextSelected) ui$.selectedThread.set(previousSelected)
      if (previousPaneThreadId === threadId && kanban$.paneThreadId.get() === nextSelected) {
        kanban$.paneThreadId.set(previousPaneThreadId)
      }
    },
  }
}

// One list rewrite for a burst of archives. Each move waits on IMAP for a few
// seconds and then reloads; letting every one of those land repaints the list
// out from under the selection the reader has already moved past.
let backgroundListRefresh: Promise<void> | null = null

let backgroundListRefreshAgain = false

function coalescedBackgroundListRefresh(): Promise<void> {
  if (!backgroundListRefresh) backgroundListRefresh = drainBackgroundListRefresh()
  else backgroundListRefreshAgain = true
  return backgroundListRefresh
}

async function drainBackgroundListRefresh() {
  try {
    do {
      backgroundListRefreshAgain = false
      await loadThreads(false)
    } while (backgroundListRefreshAgain)
  } finally {
    backgroundListRefresh = null
    if (backgroundListRefreshAgain) {
      backgroundListRefreshAgain = false
      void coalescedBackgroundListRefresh()
    }
  }
}

export async function refreshThreadLocation(accountId?: string, refresh = false) {
  if (refresh) await loadThreads(true)
  else await coalescedBackgroundListRefresh()
  const selectedAcc = ui$.selectedAccount.get()
  if (selectedAcc) {
    void loadFolders(selectedAcc, false)
  }
  if (accountId && accountId !== selectedAcc) {
    void loadFolders(accountId, false)
  }
}
