// Resolves the current thread and neighboring rows for keyboard navigation.
// Mailboxes, board columns and reader tabs provide candidate rows.
// The active mailbox filter determines which rows the keyboard can select.

import type { Message, MessageTab } from '../types'
import { ui$ } from './ui'
import { kanban$ } from './kanban'
import { filterThreads } from '../lib/threadActions'
import { isUnifiedStarred } from '../lib/unifiedFolders'
import { mail$ } from './mailState'
import { compose$ } from './composeState'

// The visible thread list after applying the active filter (all / unread / starred).
export function getFilteredThreads() {
  const threads = mail$.threads.get()
  // The starred folder already lists starred threads only; a leftover filter
  // mode from the previous mailbox must not hide rows here.
  if (isUnifiedStarred(ui$.selectedAccount.get(), ui$.selectedFolder.get())) return threads
  const filterMode = ui$.filterMode.get()
  const selected = ui$.selectedThread.get()
  return filterThreads(threads, filterMode, selected, mail$.readThreads.get(), ui$.attachmentsOnly.get())
}

// Move the selection up (delta -1) or down (delta +1) through the visible
// thread list, clamping at the ends (no wrap, so a held key doesn't loop back).
// Backs the j/k keyboard navigation.
export function selectAdjacentThread(delta: number) {
  const list = getFilteredThreads()
  if (list.length === 0) return
  const selected = ui$.selectedThread.get()
  const current = list.findIndex((thread) => thread.thread_id === selected)
  const next = current === -1 ? 0 : Math.min(list.length - 1, Math.max(0, current + delta))
  const target = list[next]
  if (target) ui$.selectedThread.set(target.thread_id)
}

export function getActiveThread() {
  const filtered = getFilteredThreads()
  const threads = mail$.threads.get()
  const selected = ui$.selectedThread.get()
  // Nothing selected means an empty conversation pane. Falling back to the top
  // of the list here would re-open — and so mark read — a thread the user never
  // picked, which is exactly what `loadThreads` refuses to do.
  if (!selected) return null
  const fromList = filtered.find((thread) => thread.thread_id === selected)
  if (fromList) return fromList
  const fromAllThreads = threads.find((thread) => thread.thread_id === selected)
  if (fromAllThreads) return fromAllThreads
  const kanbanColumns = kanban$.threads.get()
  for (const threads of Object.values(kanbanColumns)) {
    const match = threads.find((thread) => thread.thread_id === selected)
    if (match) return match
  }
  // A conversation open in its own tab renders outside the list on the left
  // (a Meron folder, another account): the tab itself says what it is.
  // Without this the reply box found no conversation, and Send did nothing.
  const tab = compose$.tabs.get().find((candidate) => candidate.id === compose$.activeTab.get())
  if (tab?.kind === 'thread' && tab.threadId === selected) return threadFromTab(tab, null)
  // The selected thread's row hasn't landed yet (a notification or kanban jump
  // that outran the list load). Wait for it rather than showing an unrelated
  // conversation from the list we happen to have.
  return null
}

/** A thread tab as a thread row, for a conversation not in the list on the left. */
export function threadFromTab(tab: MessageTab, fallback: Message | null): Message {
  if (fallback?.thread_id === tab.threadId) return fallback
  return {
    id: tab.messageId || tab.threadId,
    account_id: tab.accountId ?? '',
    folder_id: tab.folderId ?? 'inbox',
    thread_id: tab.threadId,
    from_name: '',
    from_addr: tab.from,
    to: '',
    subject: tab.subject || '(no subject)',
    preview: '',
    body: '',
    date: tab.date ?? 0,
    unread: false,
    starred: false,
    has_attachments: false,
  }
}
