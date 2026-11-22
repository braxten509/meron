// Maintains mobile boards, active columns and
// their mailbox data. A column refresh or mutation
// must remain scoped to the relevant board
// selection.

package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.size
import androidx.compose.ui.input.key.key
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.StarredItemSummary
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.accountSummaryIsRss
import kotlinx.coroutines.withContext

internal fun MeronMobileState.persistKanbanBoards(next: List<KanbanBoardSpec>) {
    kanbanBoards = next
    saveKanbanBoards(kanbanPrefs, next)
    if (activeKanbanBoardId.isBlank() || next.none { it.id == activeKanbanBoardId }) {
        activeKanbanBoardId = next.firstOrNull()?.id.orEmpty()
        saveActiveKanbanBoardId(kanbanPrefs, activeKanbanBoardId)
    }
}

internal fun MeronMobileState.persistKanbanFilter(next: FilterMode) {
    kanbanFilter = next
    saveKanbanFilter(kanbanPrefs, next)
}

internal fun MeronMobileState.persistKanbanAttachmentsOnly(next: Boolean) {
    kanbanAttachmentsOnly = next
    saveKanbanAttachmentsOnly(kanbanPrefs, next)
}

internal fun MeronMobileState.persistKanbanSearch(next: String) {
    kanbanSearch = next
    saveKanbanSearch(kanbanPrefs, next)
}

internal fun MeronMobileState.persistKanbanSearchScope(next: String) {
    kanbanSearchScope = next.ifBlank { "all" }
    saveKanbanSearchScope(kanbanPrefs, kanbanSearchScope)
}

// The board search as it applies to [column]; a saved search column is read with
// its own query narrowed by it.
internal fun MeronMobileState.kanbanColumnSearchQuery(column: KanbanColumnSpec): String {
    val query = kanbanSearch.trim()
    val scope = kanbanSearchScope.ifBlank { "all" }
    val boardSearch = if (query.isNotBlank() && (scope == "all" || scope == kanbanColumnKey(column))) query else ""
    return narrowKanbanQuery(column.query, boardSearch)
}

internal fun MeronMobileState.currentKanbanColumnView(column: KanbanColumnSpec): KanbanColumnView =
    KanbanColumnView(
        query = kanbanColumnSearchQuery(column),
        filter = kanbanFilter,
        attachmentsOnly = kanbanAttachmentsOnly,
    )

internal fun isUnifiedStarredColumn(column: KanbanColumnSpec): Boolean = column.accountId == UNIFIED_ACCOUNT_ID && column.folderId.equals(STARRED_FOLDER, ignoreCase = true)

internal fun shouldSyncUnfetchedKanbanColumn(
    column: KanbanColumnSpec,
    refresh: Boolean,
    query: String,
    result: MailboxLoadResult,
    accounts: List<AccountSummary>,
): Boolean {
    if (refresh || query.isNotBlank() || result.threads.isNotEmpty() || result.folderSynced != false) return false
    if (column.accountId == UNIFIED_ACCOUNT_ID) return false
    val account = accounts.firstOrNull { it.id == column.accountId } ?: return false
    return !account.paused && !account.needsReconnect && !accountSummaryIsRss(account)
}

internal fun StarredItemSummary.toThreadSummary(): ThreadSummary =
    ThreadSummary(
        id = id,
        threadId = threadId,
        accountId = accountId,
        folder = folder,
        folderRole = folderRole,
        subject = subject,
        sender = sender,
        preview = preview,
        unread = unread,
        starred = true,
        dateEpochSeconds = dateEpochSeconds,
    )

internal fun MeronMobileState.updateKanbanColumn(
    key: String,
    update: (KanbanColumnState) -> KanbanColumnState,
) {
    kanbanColumns = kanbanColumns + (key to update(kanbanColumns[key] ?: KanbanColumnState()))
}

internal fun MeronMobileState.updateThreadEverywhere(
    thread: ThreadSummary,
    update: (ThreadSummary) -> ThreadSummary,
) {
    val next = update(thread)
    val beforeUnread = if (thread.unread) thread.unreadCount.coerceAtLeast(1) else 0
    val afterUnread = if (next.unread) next.unreadCount.coerceAtLeast(1) else 0
    val unreadDelta = afterUnread - beforeUnread
    coreThreads = coreThreads.map { if (it.id == thread.id) next else it }.forStarredView(selectedCoreAccountId, selectedCoreFolder)
    selectedCoreThread = selectedCoreThread?.let { if (it.id == thread.id) next else it }
    kanbanColumns =
        kanbanColumns.mapValues { (key, state) ->
            if (state.threads.none { it.id == thread.id }) {
                state
            } else {
                state.copy(
                    threads =
                        state.threads.map { if (it.id == thread.id) next else it }.let { threads ->
                            val (accountId, folderId) = kanbanKeyAccountFolder(key)
                            threads.forStarredView(accountId, folderId)
                        },
                    unreadCount = state.unreadCount?.let { (it + unreadDelta).coerceAtLeast(0) },
                )
            }
        }
}

internal fun MeronMobileState.removeThreadEverywhere(threadId: String) {
    coreThreads = coreThreads.filterNot { it.id == threadId }
    kanbanColumns =
        kanbanColumns.mapValues { (_, state) ->
            state.copy(threads = state.threads.filterNot { it.id == threadId })
        }
    if (selectedCoreThread?.id == threadId) {
        selectedCoreThread = null
        messages = emptyList()
    }
}

internal suspend fun MeronMobileState.fetchKanbanColumn(
    client: MobileMailCommandClient,
    column: KanbanColumnSpec,
    refresh: Boolean,
    view: KanbanColumnView,
    beforeCursor: String? = null,
    accountCursors: Map<String, String> = emptyMap(),
    listLimit: Int = MAILBOX_PAGE_SIZE,
): MailboxLoadResult {
    val columnQuery = view.query
    if (isUnifiedStarredColumn(column)) {
        // The column applies its own filter to the loaded cards, so the load
        // itself asks for all of them.
        return loadUnifiedStarred(
            client = client,
            query = columnQuery,
            filter = FilterMode.All,
            attachmentsOnly = false,
            beforeCursor = beforeCursor,
            limit = listLimit,
        )
    }
    return if (column.accountId == UNIFIED_ACCOUNT_ID) {
        val unifiedAccounts = coreAccounts.filter { it.includedInUnified }
        loadUnifiedInbox(
            client = client,
            accounts = unifiedAccounts,
            query = columnQuery,
            filter = view.filter,
            attachmentsOnly = view.attachmentsOnly,
            syncFirst = refresh,
            beforeCursor = beforeCursor,
            listLimit = listLimit,
            folderRole = column.folderId,
        )
    } else {
        val account =
            coreAccounts.firstOrNull { it.id == column.accountId }
                ?: return MailboxLoadResult(emptyList(), column.folderId, emptyList())
        loadAccountInbox(
            client,
            account,
            column.folderId,
            query = columnQuery,
            filter = view.filter,
            attachmentsOnly = view.attachmentsOnly,
            syncFirst = refresh,
            beforeCursor = beforeCursor,
            listLimit = listLimit,
        )
    }
}

internal fun MeronMobileState.loadKanbanColumn(
    column: KanbanColumnSpec,
    refresh: Boolean = false,
) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val key = kanbanColumnKey(column)
    val view = currentKanbanColumnView(column)
    val query = view.query
    val token = (kanbanColumnLoadTokens[key] ?: 0L) + 1
    kanbanColumnLoadTokens[key] = token
    // As in the mailbox list: re-reading the view already on screen (after a
    // sync or mail event) keeps every page scrolled in so far, or the column
    // shrinks under the reader and the scroll position jumps. A new query or
    // filter starts over at one page.
    val shown = kanbanColumns[key]?.takeIf { it.cursorView == view }
    val listLimit = shown?.readDepth?.coerceIn(MAILBOX_PAGE_SIZE, MAILBOX_MAX_RELOAD_DEPTH) ?: MAILBOX_PAGE_SIZE
    val oldestShown = shown?.threads?.minOfOrNull { it.dateEpochSeconds }
    updateKanbanColumn(key) { it.copy(loading = true, error = null) }
    val folderGeneration = mailboxFolderGeneration
    scope.launchThreadListRead(threadRemovalGuard) { read ->
        val folderReadVersion = folderReadGuard.version
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                val cached = fetchKanbanColumn(client, column, refresh, view, listLimit = listLimit)
                val first =
                    if (shouldSyncUnfetchedKanbanColumn(column, refresh, query, cached, coreAccounts)) {
                        fetchKanbanColumn(client, column, refresh = true, view = view, listLimit = listLimit)
                    } else {
                        cached
                    }
                readMailboxToOldestShown(first, listLimit, oldestShown) { beforeCursor, accountCursors ->
                    fetchKanbanColumn(
                        client = client,
                        column = column,
                        refresh = false,
                        view = view,
                        beforeCursor = beforeCursor,
                        accountCursors = accountCursors,
                    )
                }
            }
        }.onSuccess { (result, readDepth) ->
            // A newer load of this column is out or has landed; its rows are the
            // answer, and it clears the loading flag itself.
            if (kanbanColumnLoadTokens[key] != token) return@onSuccess
            // A read begun before a folder rename holds the old names, in its
            // folder list and its cards' ids alike: read again instead.
            if (folderGeneration != mailboxFolderGeneration) {
                loadKanbanColumn(column, refresh)
                return@onSuccess
            }
            if (result.folders.isNotEmpty()) {
                foldersByAccount = foldersByAccount + reconcileFolderUnread(result.folders, folderReadVersion).groupBy { it.accountId }
            }
            updateKanbanColumn(key) {
                it.copy(
                    threads = withLocalDraftFlags(withoutLocallyDiscardedThreads(read.filter(result.threads))),
                    unreadCount = result.unreadCount,
                    loading = false,
                    loadingMore = false,
                    error = null,
                    // A searched column keeps its cursor too: the attachments
                    // toggle can leave a search page empty with matches further
                    // back, and paging on with the same query reaches them.
                    nextCursor = result.nextCursor,
                    accountCursors = result.accountCursors,
                    cursorView = view,
                    readDepth = readDepth,
                    heldThreads = withLocalDraftFlags(withoutLocallyDiscardedThreads(read.filter(result.heldThreads))),
                )
            }
        }.onFailure {
            if (kanbanColumnLoadTokens[key] != token) return@onFailure
            updateKanbanColumn(key) { state -> state.copy(loading = false, error = it.message ?: trs("mobile.status.loadFailed")) }
            status = trsError("mobile.status.kanbanLoadFailed", it)
        }
    }
}

internal fun MeronMobileState.loadMoreKanbanColumn(column: KanbanColumnSpec) {
    if (!coreLoaded) return
    val key = kanbanColumnKey(column)
    val state = kanbanColumns[key] ?: return
    if (state.loading || state.loadingMore) return
    // Cards the last reload held back: show them, read nothing. The cursor is
    // already past them.
    if (state.heldThreads.isNotEmpty()) {
        updateKanbanColumn(key) { current ->
            val existingIds = current.threads.map { it.id }.toSet()
            val appended = threadRemovalGuard.filter(current.heldThreads).filterNot { it.id in existingIds }
            current.copy(
                threads = threadRemovalGuard.filter(current.threads + appended).sortedByDescending { it.dateEpochSeconds },
                heldThreads = emptyList(),
                readDepth = (current.readDepth + appended.size).coerceAtMost(MAILBOX_MAX_RELOAD_DEPTH),
            )
        }
        return
    }
    val hasCursor = state.nextCursor.isNotBlank()
    // A reload in flight is about to replace the list and the cursor with it.
    if (!hasCursor) return
    // A reload started since (new query, filter or sync) owns the list instead.
    val token = kanbanColumnLoadTokens[key] ?: 0L
    updateKanbanColumn(key) { it.copy(loadingMore = true, error = null) }
    val folderGeneration = mailboxFolderGeneration
    scope.launchThreadListRead(threadRemovalGuard) { read ->
        val folderReadVersion = folderReadGuard.version
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                fetchKanbanColumn(
                    client = client,
                    column = column,
                    refresh = false,
                    view = state.cursorView,
                    beforeCursor = state.nextCursor,
                    accountCursors = state.accountCursors,
                )
            }
        }.onSuccess { result ->
            // A page read before a folder rename holds the old names; drop it.
            if (folderGeneration != mailboxFolderGeneration) {
                updateKanbanColumn(key) { it.copy(loadingMore = false) }
                return@onSuccess
            }
            if (result.folders.isNotEmpty()) {
                foldersByAccount = foldersByAccount + reconcileFolderUnread(result.folders, folderReadVersion).groupBy { it.accountId }
            }
            if ((kanbanColumnLoadTokens[key] ?: 0L) != token) {
                updateKanbanColumn(key) { it.copy(loadingMore = false) }
                return@onSuccess
            }
            updateKanbanColumn(key) { current ->
                val existingIds = current.threads.map { it.id }.toSet()
                val appended = withLocalDraftFlags(read.filter(result.threads)).filterNot { it.id in existingIds }
                current.copy(
                    threads = read.filter(current.threads + appended).sortedByDescending { it.dateEpochSeconds },
                    loadingMore = false,
                    error = null,
                    nextCursor = result.nextCursor,
                    accountCursors = result.accountCursors,
                    readDepth = current.readDepth + MAILBOX_PAGE_SIZE,
                )
            }
        }.onFailure {
            updateKanbanColumn(key) { state -> state.copy(loadingMore = false, error = it.message ?: trs("mobile.status.loadFailed")) }
            status = trsError("mobile.status.loadMoreFailed", it)
        }
    }
}

// Every column an account-wide cache change can touch. A change that names no
// folder (a late Sent copy) has nothing for folder matching to work with — the
// conversation it belongs to sits in whichever mailbox holds it — so each of
// the account's columns re-reads. Unified columns only count when the account
// is part of Unified, as elsewhere in this file.
internal fun MeronMobileState.refreshKanbanColumnsForAccount(accountId: String) {
    if (accountId.isBlank()) return
    val board = kanbanBoards.firstOrNull { it.id == activeKanbanBoardId } ?: return
    val accountIncludedInUnified =
        coreAccounts.firstOrNull { it.id == accountId }?.includedInUnified == true
    board.columns
        .filter { column ->
            column.accountId == accountId ||
                (column.accountId == UNIFIED_ACCOUNT_ID && accountIncludedInUnified)
        }.distinctBy(::kanbanColumnKey)
        .forEach { column -> loadKanbanColumn(column, refresh = false) }
}

// Re-read the active board's columns whose card stands for [threadId] after a
// change inside the conversation the card's message count and Draft badge
// reflect — a quick reply's post-send draft discard. The mailbox reload that
// runs alongside feeds the list behind the board, not the board; and the Sent
// copy event that does re-read the account's columns fires from the send,
// before the discard, so the card kept counting the draft until some later
// sync happened to touch its column. The discard already dropped the cached
// draft rows, so a cache read is enough.
internal fun MeronMobileState.refreshKanbanColumnsHoldingThread(threadId: String) {
    if (threadId.isBlank()) return
    val board = kanbanBoards.firstOrNull { it.id == activeKanbanBoardId } ?: return
    val keys =
        kanbanColumns
            .filterValues { state -> state.threads.any { it.id == threadId || it.backendThreadId() == threadId } }
            .keys
    if (keys.isEmpty()) return
    board.columns
        .filter { kanbanColumnKey(it) in keys }
        .distinctBy(::kanbanColumnKey)
        .forEach { column -> loadKanbanColumn(column, refresh = false) }
}

internal fun MeronMobileState.refreshKanbanColumnsForMailEvent(
    accountId: String,
    folderId: String,
    refresh: Boolean = false,
) {
    val board = kanbanBoards.firstOrNull { it.id == activeKanbanBoardId } ?: return
    val folder = folderId.ifBlank { INBOX_FOLDER }
    val accountIncludedInUnified =
        coreAccounts.firstOrNull { it.id == accountId }?.includedInUnified == true
    board.columns
        .filter { column ->
            val directFolderMatch =
                column.accountId == accountId &&
                    column.folderId.equals(folder, ignoreCase = true)
            val unifiedFolderMatch =
                column.accountId == UNIFIED_ACCOUNT_ID &&
                    accountIncludedInUnified &&
                    unifiedColumnMatchesFolder(column.folderId, foldersByAccount[accountId].orEmpty(), folderId)
            // A tag search reads the whole account, whatever folder anchors it.
            val tagMatch =
                kanbanColumnReadsTags(column) &&
                    (column.accountId == accountId || (column.accountId == UNIFIED_ACCOUNT_ID && accountIncludedInUnified))
            directFolderMatch || unifiedFolderMatch || tagMatch
        }.distinctBy(::kanbanColumnKey)
        .forEach { column ->
            // The IDLE/event path has already synced the core DB, so callers can
            // re-read the affected active Kanban columns from cache without
            // another IMAP pass. Callers whose own action (e.g. discarding a
            // draft) didn't go through that sync must pass refresh = true.
            loadKanbanColumn(column, refresh = refresh)
        }
}

internal fun MeronMobileState.loadKanbanBoard(refresh: Boolean = false) {
    val board = kanbanBoards.firstOrNull { it.id == activeKanbanBoardId } ?: return
    if (kanbanSearchScope != "all" && board.columns.none { kanbanColumnKey(it) == kanbanSearchScope }) {
        persistKanbanSearchScope("all")
    }
    board.columns.forEach { column -> loadKanbanColumn(column, refresh) }
}
