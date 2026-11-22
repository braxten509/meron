// Filters mailbox conversations and applies read actions to the resulting
// selection. RSS entries and ordinary mail can require different targets.
// Callers include ColumnLabel.tsx and ConversationHeader.tsx.

import { invoke } from './bridge'
import type { Account, Message } from '../types'
import type { FilterMode } from '../states/ui'

export function isRssAccount(account: Account | undefined, accountId: string): boolean {
  return account?.provider === 'rss' || account?.auth_type === 'rss' || accountId.startsWith('rss-')
}

// Pure thread filter shared by the chat thread list and kanban columns. `keepId`
// is an open thread to keep visible even when it no longer matches the filter
// (e.g. selecting an unread thread marks it read but it shouldn't vanish).
// `attachmentsOnly` is the independent attachments toggle, applied on top of
// the read-state `mode`.
export function filterThreads(
  threads: Message[],
  mode: FilterMode,
  keepId?: string,
  keepIds?: Record<string, boolean>,
  attachmentsOnly = false,
): Message[] {
  const matchesMode = (thread: Message) =>
    mode === 'unread' ? thread.unread : mode === 'starred' ? thread.starred || !!thread.has_starred_items : true
  if (mode === 'all' && !attachmentsOnly) return threads
  return threads.filter(
    (thread) =>
      thread.thread_id === keepId ||
      !!keepIds?.[thread.thread_id] ||
      (matchesMode(thread) && (!attachmentsOnly || thread.has_attachments)),
  )
}

// Mark a set of threads read on the backend. Mail accounts are marked folder-wide
// in one call each; RSS feeds (no folder-wide flag) are marked per-thread. The
// caller owns the optimistic local-state update.
export async function markThreadsReadRemote(threads: Message[], accounts: Account[], folderId: string): Promise<void> {
  const mailAccounts = new Set<string>()
  const rssThreadIds: string[] = []
  for (const thread of threads) {
    const account = accounts.find((acc) => acc.id === thread.account_id)
    if (isRssAccount(account, thread.account_id)) {
      rssThreadIds.push(thread.thread_id)
    } else {
      mailAccounts.add(thread.account_id)
    }
  }

  await Promise.all([
    ...Array.from(mailAccounts).map((accountId) =>
      invoke('mail.markAllRead', { account_id: accountId, folder_id: folderId }).catch((requestFailure) =>
        console.error('markAllRead failed:', requestFailure),
      ),
    ),
    ...rssThreadIds.map((threadId) =>
      invoke('mail.markRead', { thread_id: threadId }).catch((requestFailure) => console.error('markAllRead (rss) failed:', requestFailure)),
    ),
  ])
}
