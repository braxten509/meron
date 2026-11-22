// Coordinates desktop mailbox loading and conversation refresh. Cached pages, live
// syncs and optimistic sends must agree on which messages are visible.
// Callers include backup.ts and bulkProgress.ts.

import type { Message } from '../types'
import { invoke } from '../lib/bridge'
import { t } from '../lib/i18n'
import { ui$, showToast, type BulkSelectionItem } from './ui'
import { accounts$, unifiedAccounts } from './accounts'
import { kanban$, getKanbanColumns, kanbanColumnKey } from './kanban'
import { columnSearchActive, loadKanbanColumn } from '../lib/kanbanData'
import { isRssAccount } from '../lib/threadActions'
import { isMeronView, isUnifiedStarred, unifiedFolderRole } from '../lib/unifiedFolders'
import { setMeronFolderUnread } from './meronFolders'
import { CONVERSATION_PAGE_SIZE } from '../lib/pagination'
import { decrementFolderUnread, updateCachedFolderUnread } from './mailFolders'
import { mail$ } from './mailState'
import { compose$ } from './composeState'
import { mergeRefreshedThreadMessages } from './sentCopyMatching'
import { updateKanbanThread, snapshotRecord } from './mailBoardRows'
import {
  threadListViewKey,
  listFilterKey,
  listGroupBy,
  beginThreadListRead,
  type ThreadListRead,
  THREAD_LIST_PAGE_SIZE,
  mergeRefreshedThreadPage,
  nextSelectedAfterRefresh,
} from './threadPageState'
import { getFilteredThreads } from './threadSelection'
export { getFilteredThreads, selectAdjacentThread, getActiveThread, threadFromTab } from './threadSelection'

export {
  captureKeys,
  kanbanKeysWithThread,
  restoreAccountFolders,
  restoreKanbanColumns,
  updateKanbanThread,
  removeKanbanThread,
  findLocalThread,
} from './mailBoardRows'
export {
  threadListViewKey,
  listFilterKey,
  listGroupBy,
  beginThreadListRead,
  suppressRemovedThread,
  completeRemovedThread,
  releaseRemovedThread,
  THREAD_LIST_PAGE_SIZE,
  mergeRefreshedThreadPage,
  nextSelectedAfterRefresh,
} from './threadPageState'

export { openAttachment, downloadAttachment, saveMessageAsEml, copyAttachmentImage } from './mailAttachments'
export { mergeRefreshedThreadMessages, normalizeMessageId } from './sentCopyMatching'

export { mail$ }

// Re-read the list whose card stands for this thread after a change inside the
// conversation (a draft discarded, a message deleted) that the card's message
// count and Draft badge reflect. In the mail view that is the thread list. With
// a Kanban board up, `loadThreads` steps out — the list is off screen and
// reloads when the board closes — but the board's columns still show the card,
// and nothing else re-reads them: a reply sent from the board's pane left its
// card counting the draft the send had discarded until some later sync happened
// to touch that column.
//
// `columnKeys` are the columns that held the card, captured before any
// optimistic removal: a conversation whose only loaded message went is dropped
// from the columns ahead of the server round-trip, and looking the card up
// afterwards would find nothing to reload — yet the server may still hold older
// messages of the thread, which only a re-read of those columns brings back.
export async function reloadThreadCards(columnKeys: string[]) {
  const boardId = kanban$.activeBoardId.peek()
  if (!boardId) {
    await loadThreads(false)
    return
  }
  const keys = new Set(columnKeys)
  const query = kanban$.searchQuery.peek().trim()
  const scope = kanban$.searchScope.peek()
  await Promise.all(
    getKanbanColumns(boardId)
      .filter((column) => keys.has(kanbanColumnKey(column)))
      .map((column) => {
        const key = kanbanColumnKey(column)
        return loadKanbanColumn(column, false, columnSearchActive(key, query, scope) ? query : '')
      }),
  )
}

function reconcileThreadUnreadFromLoadedMessages(
  threadId: string,
  messages: Message[],
  nextCursor: string | undefined,
) {
  if (nextCursor) return
  const threadMessages = messages.filter((message) => message.thread_id === threadId)
  if (threadMessages.length === 0 || threadMessages.some((message) => message.unread)) return

  const thread = mail$.threads.get().find((item) => item.thread_id === threadId)
  if (!thread?.unread) return
  const unreadCount = Math.max(1, thread.unread_count ?? 0)
  mail$.threads.set(
    mail$.threads
      .get()
      .map((item) => (item.thread_id === threadId ? { ...item, unread: false, unread_count: 0 } : item)),
  )
  decrementFolderUnread(thread.account_id, thread.folder_id, unreadCount)
  updateKanbanThread(threadId, (item) => ({ ...item, unread: false, unread_count: 0 }))
}

// Guards against a slow load repainting the list after a newer one already did.
// A search is a live IMAP round trip — seconds, not milliseconds — so typing
// another character, clearing the box, or switching account/folder while one is
// in flight routinely finishes out of order. Every write below belongs to the
// newest call only; the losers drop their results (same idea as the kanban
// column loader's `columnLoadVersions` and mobile's `activeMailboxLoadToken`).
let threadLoadVersion = 0

// Opening a conversation is never passive: the message pane marks every visible
// unread message read as soon as it renders. So the thread list must not open a
// conversation on its own — switching account/folder (the unified inbox above
// all, since it lands on the newest mail across every account) would silently
// clear the unread flag on a thread the user never looked at. Selection follows
// a click, a j/k move, or the flows below that delete what was open and hand the
// slot to the replacement thread; every other load leaves it alone.
let reselectAfterThreadLoad = false

// View key of the refresh:true load currently out to the server, '' when none,
// tagged with the `threadLoadVersion` of the load holding it so only that load
// releases it. Read by the background-refresh guard below.
let pendingRefreshKey = ''
let pendingRefreshVersion = 0
// View key of a background refresh that stepped aside for the load above, ''
// when none. The load it yielded to read the cache before whatever prompted
// the refresh changed it — a post-send draft discard, say — so its rows are
// not the answer the refresh was after; it runs again once that load lands.
let deferredRefreshKey = ''

// Called by flows that clear `selectedThread` because the open conversation is
// leaving the list (delete, move, discard draft) and want the next load to open
// whatever takes its place. One-shot: consumed by the next `loadThreads`.
export function requestThreadReselect() {
  reselectAfterThreadLoad = true
}

type ThreadSearchStage = 'auto' | 'cache' | 'live'

// Encoded t. keys include the subject branch and are stable across folders.
// Numeric IMAP UIDs and RSS item ids remain tied to their original location.
function starredConversationIdentity(thread: Message): string {
  if (thread.thread_id.includes('#rss#')) return thread.thread_id
  const key = thread.thread_id.slice(thread.thread_id.lastIndexOf('#') + 1)
  return key.startsWith('t.') ? `${thread.account_id}#${key}` : thread.thread_id
}

export async function loadThreads(refresh = true, searchStage: ThreadSearchStage = 'auto') {
  const read = beginThreadListRead()
  try {
    await loadThreadsWithRead(refresh, searchStage, read)
  } finally {
    read.dispose()
  }
}

async function loadThreadsWithRead(refresh: boolean, searchStage: ThreadSearchStage, read: ThreadListRead) {
  // A Kanban card temporarily points selectedFolder at the card's real mailbox
  // so thread actions have the right context. The normal mail list is hidden,
  // and treating that account-specific id as a unified role falls back to Inbox,
  // replacing the rows that should still be waiting for the mail view.
  if (kanban$.activeBoardId.peek()) {
    // A reselect request belongs to the load that was asked for, which is this
    // one; dropping it here keeps it from arming a later load in the mail view,
    // where it would open (and mark read) an unrelated conversation.
    reselectAfterThreadLoad = false
    return
  }

  const initialAccount = ui$.selectedAccount.get()
  const initialFolder = ui$.selectedFolder.get()
  const initialQuery = ui$.query.get()
  const initialFilter = listFilterKey()
  const activeAccount = accounts$.get().find((account) => account.id === initialAccount)
  // Starred is answered from the local cache, so there is no live stage to run.
  // So is a Meron folder: it exists only on this device.
  const canSearchLive =
    !isUnifiedStarred(initialAccount, initialFolder) &&
    !isMeronView(initialAccount) &&
    (initialAccount === 'unified' || !isRssAccount(activeAccount, initialAccount))

  // Paint results from the local FTS index before starting the live IMAP
  // request. RSS search is already local, and background refreshes deliberately
  // remain cache-only.
  if (refresh && searchStage === 'auto' && initialQuery.trim() && canSearchLive) {
    await loadThreads(false, 'cache')
    if (
      ui$.selectedAccount.get() !== initialAccount ||
      ui$.selectedFolder.get() !== initialFolder ||
      ui$.query.get() !== initialQuery ||
      listFilterKey() !== initialFilter
    ) {
      return
    }
    await loadThreads(true, 'live')
    return
  }

  const selectedAcc = ui$.selectedAccount.get()
  const selectedFol = ui$.selectedFolder.get()
  const searchText = ui$.query.get()
  const filter = ui$.filterMode.get()
  const attachments = ui$.attachmentsOnly.get()
  const groupBySender = ui$.groupBySender.get()
  const groupBy = listGroupBy(groupBySender)
  const filterKey = listFilterKey(filter, attachments, groupBySender)
  const viewKey = threadListViewKey(selectedAcc, selectedFol, searchText, filterKey)

  // A background refresh steps aside for a server-bound load already running for
  // the same view. Taking the version from it would throw away the fresher rows
  // it is about to return, and — since a background load never settles a view —
  // would strand the list on the spinner: the foreground load loses `superseded`
  // when it lands, and nothing else is scheduled to try again. Nothing is lost by
  // skipping; the load in flight is asking the server the same question.
  // Only while the claiming load is still the newest one. A load another has
  // overtaken will drop its results, and it releases the claim on its own only
  // when its request finally lands — a slow or wedged search would keep every
  // background refresh of the view out until then.
  if (
    !refresh &&
    searchStage !== 'cache' &&
    pendingRefreshKey === viewKey &&
    pendingRefreshVersion === threadLoadVersion
  ) {
    deferredRefreshKey = viewKey
    return
  }

  const version = (threadLoadVersion += 1)
  if (refresh) {
    pendingRefreshKey = viewKey
    pendingRefreshVersion = version
    // This load reads the cache after anything a skipped refresh was reacting to.
    deferredRefreshKey = ''
  }
  // Hand the claim above back the moment this load stops being the one that will
  // write. Every background refresh of this view steps aside while it stands, so
  // a claim left behind by a load that gave up silences them all for as long as
  // the view is on screen — that is how a reply sent from a search left the
  // thread's card showing a Draft badge and a message count that still counted
  // the draft: the refresh the post-send discard runs was skipped for a search
  // load the next keystroke had already superseded.
  const releasePendingRefresh = () => {
    if (pendingRefreshVersion !== version) return
    pendingRefreshKey = ''
    pendingRefreshVersion = 0
  }
  // Asked at every point this load would drop its results, so it is also where
  // the claim is released.
  const superseded = () => {
    const stale =
      threadLoadVersion !== version ||
      ui$.selectedAccount.get() !== selectedAcc ||
      ui$.selectedFolder.get() !== selectedFol ||
      ui$.query.get() !== searchText ||
      listFilterKey() !== filterKey
    if (stale) releasePendingRefresh()
    return stale
  }
  const previousThreads = mail$.threads.get()
  const previousThreadsCursor = mail$.threadsCursor.get()
  const previousAccountCursors = snapshotRecord(mail$.threadAccountCursors.get())
  const userInitiated = refresh || searchStage === 'cache'

  // A background refresh (a sync, not a user-initiated account/folder/query/filter
  // change) only re-fetches the first page of the thread list. If the user has
  // scrolled the list and loaded extra pages, replacing the whole array with just
  // the first page collapses it and resets the scroll position. In that case we
  // merge the fresh page into the list we already have instead.
  const backgroundRefresh = !refresh && searchStage !== 'cache' && previousThreads.length > 0
  const unifiedStarred = isUnifiedStarred(selectedAcc, selectedFol)
  const meronView = isMeronView(selectedAcc)
  const mergeBackground = backgroundRefresh && !unifiedStarred && !meronView

  let allThreads: Message[] = []
  let readFailed = false
  const failedAccountIds = new Set<string>()

  // Starred spans every account and every folder, so it is answered by a
  // cross-account cache query rather than the per-account folder fan-out. Its
  // rows are ordinary thread cards, so only the fetch is special-cased.
  if (meronView) {
    // A Meron folder spans every account and is answered from the local cache.
    // Like Starred, a background refresh re-reads every loaded row: a
    // conversation can leave (moved out, archived) from anywhere in the list.
    try {
      const response = await invoke<{ threads: Message[]; next_cursor?: string; folder_unread?: number }>(
        'mail.meronFolderThreads',
        {
          folder_id: selectedFol,
          query: searchText,
          filter,
          attachments,
          limit: backgroundRefresh ? Math.max(THREAD_LIST_PAGE_SIZE, previousThreads.length) : THREAD_LIST_PAGE_SIZE,
        },
      )
      if (superseded()) return
      allThreads = response.threads ?? []
      if (typeof response.folder_unread === 'number') setMeronFolderUnread(selectedFol, response.folder_unread)
      mail$.threadsCursor.set(response.next_cursor ?? '')
      mail$.threadAccountCursors.set({})
    } catch (requestFailure) {
      if (superseded()) return
      console.error('Failed to load Meron folder:', requestFailure)
      readFailed = true
      if (userInitiated) {
        mail$.threadsCursor.set('')
        mail$.threadAccountCursors.set({})
      }
    }
  } else if (isUnifiedStarred(selectedAcc, selectedFol)) {
    try {
      const response = await invoke<{ items: Message[]; next_cursor?: string }>('mail.starredItems', {
        query: searchText,
        filter,
        // Hidden in the starred view (no menu offers it), so a toggle left on
        // elsewhere must not narrow this list.
        attachments: false,
        // Re-fetch every loaded row: stars can disappear and the core can pick
        // a different folder copy of the same conversation after a flag change.
        // Keeping absent rows or merging by folder-specific thread_id duplicates it.
        // Optimistically removed rows no longer count toward this loaded window.
        limit: backgroundRefresh ? Math.max(50, previousThreads.length) : 50,
      })
      if (superseded()) return
      allThreads = response.items ?? []
      mail$.threadsCursor.set(response.next_cursor ?? '')
      mail$.threadAccountCursors.set({})
    } catch (requestFailure) {
      if (superseded()) return
      console.error('Failed to load starred items:', requestFailure)
      readFailed = true
      if (userInitiated) {
        mail$.threadsCursor.set('')
        mail$.threadAccountCursors.set({})
      }
    }
  } else if (selectedAcc === 'unified') {
    const role = unifiedFolderRole(selectedFol)
    try {
      const result = await invoke<{
        threads: Message[]
        next_cursor?: string
        folder_unreads?: Record<string, number>
        failures?: Array<{ account_id: string; message: string }>
        search_incomplete?: boolean
      }>('mail.threadList', {
        account_id: 'unified',
        folder_id: role,
        folder_role: role,
        query: searchText,
        filter,
        attachments,
        group_by: groupBy,
        refresh,
      })
      if (superseded()) return
      allThreads = result.threads || []
      // Only the Inbox totals feed the side-nav badges; the other unified
      // folders have no badge to keep in sync.
      if (role === 'inbox') {
        for (const [accountId, unread] of Object.entries(result.folder_unreads ?? {})) {
          updateCachedFolderUnread(accountId, 'inbox', unread)
        }
      }
      for (const failure of result.failures ?? []) {
        failedAccountIds.add(failure.account_id)
        console.error(`Failed to load threads for ${failure.account_id}: ${failure.message}`)
      }
      // Failed accounts did not check their cached rows. If every participating
      // account failed, there is no authoritative page or cursor at all.
      const participatingIds = unifiedAccounts().map((account) => account.id)
      const expectedIds = participatingIds.length
        ? participatingIds
        : [...new Set([...previousThreads, ...allThreads].map((thread) => thread.account_id))]
      readFailed = expectedIds.length > 0 && expectedIds.every((id) => failedAccountIds.has(id))
      if (!readFailed || userInitiated) {
        mail$.threadAccountCursors.set({})
        mail$.threadsCursor.set(result.next_cursor ?? '')
      }
      // Some account's server search failed; its part of the list is cached only.
      if (refresh && result.search_incomplete) showToast(t('threads.searchServerUnavailable'))
    } catch (requestFailure) {
      if (superseded()) return
      console.error('Failed to load unified threads:', requestFailure)
      readFailed = true
      if (userInitiated) {
        mail$.threadAccountCursors.set({})
        mail$.threadsCursor.set('')
      }
    }
  } else {
    try {
      const result = await invoke<{
        threads: Message[]
        next_cursor?: string
        folder_unread?: number
        search_incomplete?: boolean
      }>('mail.threadList', {
        account_id: selectedAcc,
        folder_id: selectedFol,
        query: searchText,
        filter,
        attachments,
        group_by: groupBy,
        refresh,
      })
      if (superseded()) return
      if (typeof result.folder_unread === 'number') {
        updateCachedFolderUnread(selectedAcc, selectedFol, result.folder_unread)
      }
      allThreads = result.threads || []
      mail$.threadsCursor.set(result.next_cursor ?? '')
      mail$.threadAccountCursors.set({})
      // The server half of the search failed; say the list is only what this
      // device has cached rather than letting older matches silently vanish.
      if (refresh && result.search_incomplete) showToast(t('threads.searchServerUnavailable'))
    } catch (requestFailure) {
      if (superseded()) return
      console.error('Failed to load threads:', requestFailure)
      readFailed = true
      if (userInitiated) {
        mail$.threadsCursor.set('')
        mail$.threadAccountCursors.set({})
      }
    }
  }

  // A failed read has no authoritative page. Preserve the current list and
  // pagination; a successful empty page below is authoritative.
  if (readFailed && !userInitiated) return

  const fetchedThreads = allThreads
  if (mergeBackground) {
    // Update the threads we already show with their fresh copies (new unread
    // counts, latest message, etc.), keep the extra pages the user loaded by
    // scrolling, and fold in threads that are new since the last load.
    // A thread the fresh page no longer has left the folder — keeping it is how
    // an archived thread came back and then refused to archive again. The cursor
    // is read first: restoring the scrolled cursor below would hide that this
    // page still has mail under it.
    const fetchedCursor = mail$.threadsCursor.get()
    const fetchedHasMore = fetchedCursor !== ''
    const pinned = { ...(filter !== 'all' || attachments ? mail$.readThreads.get() : {}) }
    // A background refresh of a search is answered from the local index, which
    // lacks the hits only the server found. A row it does not return is "not
    // found yet", not gone; keep the results and their pagination.
    const searching = searchText.trim() !== ''
    for (const thread of previousThreads) {
      if (searching || failedAccountIds.has(thread.account_id)) pinned[thread.thread_id] = true
    }
    allThreads = mergeRefreshedThreadPage(previousThreads, fetchedThreads, fetchedHasMore, pinned)
    // Preserve loaded pagination depth when it has a cursor; an exhausted old
    // list must adopt the new page's cursor when more mail becomes available.
    // A partial failure cannot establish that the whole unified list is exhausted.
    const exhausted =
      !searching && fetchedThreads.length < THREAD_LIST_PAGE_SIZE && !fetchedHasMore && failedAccountIds.size === 0
    mail$.threadsCursor.set(exhausted ? '' : previousThreadsCursor || fetchedCursor)
    mail$.threadAccountCursors.set(exhausted || !previousThreadsCursor ? {} : previousAccountCursors)
  }

  // Read selection here, not at the start of the load. Archiving the open
  // thread moves it to the neighbour while this load is in flight; deciding
  // from the old id would put that thread back or close the one that replaced it.
  // Done after the merge so pinning the open row can't make the fresh page look
  // longer than it is and keep threads that actually left.
  const selectedNow = ui$.selectedThread.get()
  if (
    (filter !== 'all' || attachments) &&
    selectedNow &&
    !allThreads.some((thread) => thread.thread_id === selectedNow)
  ) {
    const selectedThread = previousThreads.find((thread) => thread.thread_id === selectedNow)
    const replacement =
      selectedThread &&
      allThreads.some((thread) => starredConversationIdentity(thread) === starredConversationIdentity(selectedThread))
    // Keep the conversation while reading clears its unread flag, but never
    // retain an unstarred row or duplicate a freshly chosen folder copy.
    if (selectedThread && (!unifiedStarred || (filter === 'unread' && selectedThread.starred && !replacement))) {
      allThreads = [...allThreads, selectedThread]
      allThreads.sort((a, b) => b.date - a.date)
    }
  }

  allThreads = read.filter(allThreads)
  mail$.threads.set(allThreads)
  // A background refresh can drop the row the selection just moved to (a short
  // page, or a neighbour archived while this load was in flight). Leaving the
  // id in place shows a blank conversation. Follow the row that is still here.
  // A load the user started still closes the pane below, on purpose.
  if (
    !userInitiated &&
    selectedNow &&
    !allThreads.some((thread) => thread.thread_id === selectedNow) &&
    !mail$.threadLoading.get() &&
    !activeThreadTabOwns(selectedNow)
  ) {
    const nextSelected = nextSelectedAfterRefresh(previousThreads, allThreads, selectedNow)
    if (nextSelected !== selectedNow) ui$.selectedThread.set(nextSelected)
  }
  // These rows now stand for this view — but only a load that went to the server
  // for them may say so, which is what `refresh` marks. The two cache-only kinds
  // both answer from the local index, so an empty result from either means "not
  // found *yet*", and settling the view on one puts "No matching mail" on screen
  // while the real answer is still coming:
  //   - the cache stage of a search, whose live IMAP half is the slow one, and
  //     the one that finds mail the index has not got;
  //   - a background refresh (sync event, feed edit), which fires on its own
  //     schedule and so can land in the gap between a keystroke and the
  //     debounced search, or between a folder switch and its own load.
  // Every view reaches the screen through an effect that loads it with
  // refresh:true, including the ones answered locally (Starred, RSS), so none
  // depends on a cache-only load to settle.
  // Only a load that got past `superseded` reaches here, so the captured fields
  // are still the ones on screen.
  if (refresh) {
    mail$.threadsLoadedKey.set(viewKey)
    releasePendingRefresh()
    // A background refresh that stepped aside for this load was asking about a
    // cache this load had already read — the draft a post-send discard removed
    // is still in the rows just written, so its card keeps the Draft badge and
    // counts the draft. Run the refresh now that nothing is in its way.
    if (deferredRefreshKey === viewKey) {
      deferredRefreshKey = ''
      void loadThreads(false).catch(console.error)
    }
  }

  const filtered = getFilteredThreads()
  // Consume the reselect request here rather than at the top of the load: a
  // superseded or replaced load returns above without ever reaching the
  // selection, and the request belongs to whichever load actually lands.
  const reselect = reselectAfterThreadLoad
  reselectAfterThreadLoad = false
  // In kanban view the open conversation is owned by kanban$.paneThreadId, not by
  // mail$.threads/filtered. Clicking a card sets selectedFolder (firing this load)
  // and selectedThread together; auto-selecting or snapping here would yank
  // selectedThread to an unrelated normal-view thread while the pane stays open on
  // the card — rendering the wrong conversation. So leave the selection alone.
  if (kanban$.activeBoardId.get()) {
    return
  }
  if (!selectedNow) {
    // Only a flow that just cleared the selection may fill it (see
    // `requestThreadReselect`); an empty pane otherwise stays empty.
    if (reselect) ui$.selectedThread.set(filtered[0]?.thread_id ?? '')
  } else if (
    // Only drop the selection on a user-initiated load (account/folder/query/
    // filter change, including the cache stage of a search). A background
    // refresh only re-fetches the first page, so an open thread the user
    // scrolled down to and opened from a later page would look "missing" and
    // get closed a second or two later.
    userInitiated &&
    !allThreads.some((thread) => thread.thread_id === selectedNow) &&
    // A selection whose conversation is still being fetched — a notification or
    // starred jump that set account, folder and thread together — isn't missing,
    // it just hasn't landed. Only close one that has settled.
    !mail$.threadLoading.get() &&
    // A thread tab on screen owns the selection (it renders only while the two
    // match) and needn't be in this folder's list: it was opened from elsewhere,
    // e.g. a kanban card in another folder, and closing it is the tab's call.
    !activeThreadTabOwns(selectedNow)
  ) {
    // The thread the user was reading is not in this view: close the pane rather
    // than opening an unrelated one for them.
    ui$.selectedThread.set('')
  }
}

function activeThreadTabOwns(threadId: string): boolean {
  const activeTab = compose$.activeTab.peek()
  return compose$.tabs.peek().some((tab) => tab.id === activeTab && tab.kind === 'thread' && tab.threadId === threadId)
}

export async function loadMoreThreads() {
  if (mail$.threadsLoadingMore.get()) return
  const selectedAcc = ui$.selectedAccount.get()
  const selectedFol = ui$.selectedFolder.get()
  const searchText = ui$.query.get()
  const filter = ui$.filterMode.get()
  const attachments = ui$.attachmentsOnly.get()
  const groupBySender = ui$.groupBySender.get()
  const groupBy = listGroupBy(groupBySender)
  const filterKey = listFilterKey(filter, attachments, groupBySender)
  const version = threadLoadVersion
  const stillCurrent = (cursor: string) =>
    threadLoadVersion === version &&
    ui$.selectedAccount.get() === selectedAcc &&
    ui$.selectedFolder.get() === selectedFol &&
    ui$.query.get() === searchText &&
    listFilterKey() === filterKey &&
    mail$.threadsCursor.get() === cursor
  // The unified starred filter is one unpaginated page; a search over it is
  // paged like any other, and every other view stops on an empty cursor below.
  // A single folder's starred filter pages on the core's cursor, and a Meron
  // folder pages every filter.
  if (!searchText.trim() && filter === 'starred' && selectedAcc === 'unified') return

  const read = beginThreadListRead()
  mail$.threadsLoadingMore.set(true)
  try {
    let moreThreads: Message[] = []
    if (isMeronView(selectedAcc)) {
      const cursor = mail$.threadsCursor.get()
      if (!cursor) return
      const response = await invoke<{ threads: Message[]; next_cursor?: string; folder_unread?: number }>(
        'mail.meronFolderThreads',
        {
          folder_id: selectedFol,
          query: searchText,
          filter,
          attachments,
          limit: THREAD_LIST_PAGE_SIZE,
          before_cursor: cursor,
        },
      )
      if (!stillCurrent(cursor)) return
      if (typeof response.folder_unread === 'number') setMeronFolderUnread(selectedFol, response.folder_unread)
      moreThreads = response.threads || []
      mail$.threadsCursor.set(response.next_cursor ?? '')
    } else if (isUnifiedStarred(selectedAcc, selectedFol)) {
      const cursor = mail$.threadsCursor.get()
      if (!cursor) return
      const response = await invoke<{ items: Message[]; next_cursor?: string }>('mail.starredItems', {
        // The cursor walks the *filtered* set, so later pages must repeat the
        // query or they'd page through items the first page never showed.
        query: searchText,
        filter,
        // Hidden in the starred view (no menu offers it), so a toggle left on
        // elsewhere must not narrow this list.
        attachments: false,
        limit: 50,
        before_cursor: cursor,
      })
      if (!stillCurrent(cursor)) return
      moreThreads = response.items || []
      mail$.threadsCursor.set(response.next_cursor ?? '')
    } else if (selectedAcc === 'unified') {
      const cursor = mail$.threadsCursor.get()
      if (!cursor) return
      const role = unifiedFolderRole(selectedFol)
      const response = await invoke<{
        threads: Message[]
        next_cursor?: string
        folder_unreads?: Record<string, number>
        search_incomplete?: boolean
      }>('mail.threadList', {
        account_id: 'unified',
        folder_id: role,
        folder_role: role,
        query: searchText,
        filter,
        attachments,
        group_by: groupBy,
        before_cursor: cursor,
        refresh: false,
      })
      if (!stillCurrent(cursor)) return
      if (role === 'inbox') {
        for (const [accountId, unread] of Object.entries(response.folder_unreads ?? {})) {
          updateCachedFolderUnread(accountId, 'inbox', unread)
        }
      }
      moreThreads = response.threads || []
      mail$.threadAccountCursors.set({})
      mail$.threadsCursor.set(response.next_cursor ?? '')
      if (response.search_incomplete) showToast(t('threads.searchServerUnavailable'))
    } else {
      const cursor = mail$.threadsCursor.get()
      if (!cursor) return
      const response = await invoke<{
        threads: Message[]
        next_cursor?: string
        folder_unread?: number
        search_incomplete?: boolean
      }>('mail.threadList', {
        account_id: selectedAcc,
        folder_id: selectedFol,
        query: searchText,
        filter,
        attachments,
        group_by: groupBy,
        before_cursor: cursor,
        refresh: false,
      })
      if (!stillCurrent(cursor)) return
      if (typeof response.folder_unread === 'number')
        updateCachedFolderUnread(selectedAcc, selectedFol, response.folder_unread)
      moreThreads = response.threads || []
      mail$.threadsCursor.set(response.next_cursor ?? '')
      // Older server matches could not be fetched; the list ends at what is cached.
      if (response.search_incomplete) showToast(t('threads.searchServerUnavailable'))
    }

    if (moreThreads.length > 0) {
      const existing = mail$.threads.get()
      const seen = new Set(existing.map((thread) => thread.thread_id))
      const merged = read.filter([...existing, ...moreThreads.filter((thread) => !seen.has(thread.thread_id))])
      if (selectedAcc === 'unified') {
        merged.sort((a, b) => b.date - a.date)
      }
      mail$.threads.set(merged)
    }
  } finally {
    read.dispose()
    mail$.threadsLoadingMore.set(false)
  }
}

export async function loadThread(threadId: string) {
  mail$.threadLoading.set(true)
  if (mail$.threadErrorId.get() === threadId) mail$.threadErrorId.set('')
  try {
    const result = await invoke<{ messages: Message[]; next_cursor?: string }>('mail.threadRead', {
      thread_id: threadId,
      limit: CONVERSATION_PAGE_SIZE,
    })
    // Guard against a stale response: the user may have switched threads while
    // this was in flight (e.g. during the ancestor fetch). Don't overwrite the
    // newer thread's messages — and let that newer load own the loading flag.
    if (ui$.selectedThread.get() !== threadId) return
    // Bodies still filling in the background arrive via the `mail.synced`
    // re-read; until then hide their placeholders rather than render empty
    // bubbles.
    const refreshed = result.messages.filter((message) => !message.body_missing)
    const messages = mergeRefreshedThreadMessages(mail$.messages.get(), refreshed, threadId)
    mail$.messages.set(messages)
    mail$.messagesCursor.set(result.next_cursor ?? '')
    mail$.messagesCursorThread.set(threadId)
    mail$.messagesLoadingMore.set(false)
    // Reconcile against the unfiltered page: a hidden placeholder can still be
    // unread, and dropping it must not mark the thread read early.
    reconcileThreadUnreadFromLoadedMessages(threadId, result.messages, result.next_cursor)
  } catch (requestFailure) {
    // Without this the failure is silent: the finally below clears the
    // spinner and the reader sits blank. Flag the thread so the pane can
    // offer a retry; keep it only if the user is still looking at it.
    console.error('threadRead failed', requestFailure)
    if (ui$.selectedThread.get() === threadId) mail$.threadErrorId.set(threadId)
  } finally {
    if (ui$.selectedThread.get() === threadId) {
      mail$.threadLoading.set(false)
    }
  }
}

export async function loadMoreMessages(threadId: string) {
  const cursor = mail$.messagesCursor.get()
  if (!cursor || mail$.messagesLoadingMore.get()) return
  // Guard against a stale click after the thread switched out from under us.
  if (ui$.selectedThread.get() !== threadId) return
  // Paging the new thread with the previous thread's cursor would prepend to the
  // wrong messages.
  if (mail$.messagesCursorThread.get() !== threadId) return
  mail$.messagesLoadingMore.set(true)
  try {
    const result = await invoke<{ messages: Message[]; next_cursor?: string }>('mail.threadRead', {
      thread_id: threadId,
      limit: CONVERSATION_PAGE_SIZE,
      before_cursor: cursor,
    })
    if (ui$.selectedThread.get() !== threadId) return
    // Prepend the older page; engine returns ascending order within the page.
    const existing = mail$.messages.get()
    const seen = new Set(existing.map((message) => message.id))
    const merged = [...result.messages.filter((message) => !seen.has(message.id) && !message.body_missing), ...existing]
    mail$.messages.set(merged)
    mail$.messagesCursor.set(result.next_cursor ?? '')
  } finally {
    mail$.messagesLoadingMore.set(false)
  }
}

export function uniqueThreadItems(items: BulkSelectionItem[]) {
  const byThread = new Map<string, BulkSelectionItem>()
  for (const item of items) {
    if (!item.threadId || item.kind !== 'mail') continue
    if (!byThread.has(item.threadId)) byThread.set(item.threadId, item)
  }
  return [...byThread.values()]
}

export async function syncMail() {
  mail$.readThreads.set({})
  const selectedAcc = ui$.selectedAccount.get()
  if (!selectedAcc) return

  ui$.busy.set(true)
  try {
    if (selectedAcc === 'unified') {
      const accounts = unifiedAccounts()
      await Promise.all(
        accounts.map((acc) =>
          invoke('mail.sync', { account_id: acc.id }).catch((requestFailure) =>
            console.error(`Sync failed for ${acc.email}:`, requestFailure),
          ),
        ),
      )
    } else {
      await invoke('mail.sync', { account_id: selectedAcc })
    }
    await loadThreads()
    showToast(t('mail.toast.synced'))
  } catch (error) {
    showToast(error instanceof Error ? error.message : t('mail.toast.syncFailed'), 'error')
  } finally {
    ui$.busy.set(false)
  }
}
