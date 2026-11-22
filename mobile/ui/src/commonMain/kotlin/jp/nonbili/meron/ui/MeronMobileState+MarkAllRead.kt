// Mobile mailbox state changes for mark all read.
// Called by the Compose routes and their thread actions.
// Server responses are reconciled with the local mailbox state here.
package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.filled.Archive
import androidx.compose.ui.input.key.key
import jp.nonbili.meron.shared.MarkAllReadParams
import jp.nonbili.meron.shared.MarkReadParams
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.RssMarkReadParams
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.accountSummaryIsRss
import jp.nonbili.meron.shared.parseFolderUnreadChanges
import jp.nonbili.meron.shared.requireCoreAllOk
import jp.nonbili.meron.shared.requireCoreOk
import jp.nonbili.meron.shared.threadIdIsRss
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

internal fun MeronMobileState.markVisibleMailboxAllRead() {
    val unread = coreThreads.filter { it.unread }
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val accountsById = coreAccounts.associateBy { it.id }
    val unifiedStarred = selectedCoreAccountId == UNIFIED_ACCOUNT_ID && isUnifiedStarredFolder(selectedCoreFolder)
    // Starred rows are single items spread across folders, so there is no
    // mailbox to mark: read them item by item, the way their column does.
    val starredTargets =
        if (unifiedStarred) {
            unread
                .filterNot { threadIdIsRss(it.id) }
                .groupBy { it.backendThreadId() }
                .map { (threadId, rows) -> threadId to rows.map { it.id } }
        } else {
            emptyList()
        }
    val unifiedRole = unifiedFolderRole(selectedCoreFolder)
    val unifiedMailAccounts =
        if (selectedCoreAccountId == UNIFIED_ACCOUNT_ID && !unifiedStarred) {
            coreAccounts.filter { it.includedInUnified && !accountSummaryIsRss(it) }
        } else {
            emptyList()
        }
    // The unified view marks the selected role (Sent, Archive, ...) in each
    // account; the core resolves the mailbox itself, and the resolved names
    // here only steer the optimistic badges and rows.
    val mailTargets =
        if (unifiedStarred) {
            emptyList()
        } else if (selectedCoreAccountId == UNIFIED_ACCOUNT_ID) {
            unifiedMailAccounts.mapNotNull { account ->
                val folders = foldersByAccount[account.id] ?: coreFolders.filter { it.accountId == account.id }
                unifiedAccountFolder(folders, unifiedRole)?.let { account.id to it }
            }
        } else {
            val account = accountsById[selectedCoreAccountId]
            if (account != null && !accountSummaryIsRss(account)) listOf(selectedCoreAccountId to selectedCoreFolder) else emptyList()
        }
    val rssTargets = unread.filter { threadIdIsRss(it.backendThreadId()) }
    if (mailTargets.isEmpty() && unifiedMailAccounts.isEmpty() && starredTargets.isEmpty() && rssTargets.isEmpty()) {
        status = trs("mobile.status.noUnreadMessages")
        return
    }
    val threadsBefore = coreThreads
    val kanbanBefore = kanbanColumns
    val foldersBefore = foldersByAccount
    val coreFoldersBefore = coreFolders
    // Only cards this mark covers: the loaded rows, and anything else in a
    // mailbox marked folder-wide. Other columns keep their unread state.
    val clearedIds = unread.map { it.id }.toSet()
    val inMarkedFolder = { card: ThreadSummary ->
        mailTargets.any { (accountId, folder) ->
            card.accountId == accountId && card.folder.equals(folder, ignoreCase = folder.equals(INBOX_FOLDER, ignoreCase = true))
        }
    }
    coreThreads = coreThreads.map { if (it.unread) it.copy(unread = false) else it }
    kanbanColumns =
        kanbanColumns.mapValues { (_, state) ->
            state.copy(
                threads =
                    state.threads.map {
                        if (it.unread && (it.id in clearedIds || inMarkedFolder(it))) it.copy(unread = false) else it
                    },
            )
        }
    applyLocalFolderUnread(plannedFolderUnread(mailTargets, rssTargets).first)
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                val responses = mutableListOf<String>()
                starredTargets.forEach { (threadId, messageIds) ->
                    responses += requireCoreOk(client.markRead(MarkReadParams(threadId = threadId, messageIds = messageIds)))
                }
                if (unifiedMailAccounts.isNotEmpty()) {
                    unifiedMailAccounts.forEach { withManagedGoogleAuth(client, it.id) { "" } }
                    responses +=
                        requireCoreAllOk(
                            client.markAllRead(MarkAllReadParams(accountId = UNIFIED_ACCOUNT_ID, folderId = unifiedRole)),
                        )
                } else {
                    mailTargets.forEach { (accountId, folderId) ->
                        responses +=
                            requireCoreOk(
                                withManagedGoogleAuth(client, accountId) {
                                    client.markAllRead(MarkAllReadParams(accountId = accountId, folderId = folderId))
                                },
                            )
                    }
                }
                rssTargets.groupBy { it.backendThreadId() }.forEach { (threadId, rows) ->
                    requireCoreOk(
                        client.markRssRead(
                            RssMarkReadParams(
                                threadId = threadId,
                                seen = true,
                                itemKeys = rows.flatMap { it.rssItemKeys() }.distinct(),
                            ),
                        ),
                    )
                }
                responses
            }
        }.onSuccess { responses ->
            responses.forEach(::applyCoreFolderUnreadChanges)
            status = trs("mobile.ios.markedUnreadItemCountRead", mapOf("count" to unread.size))
            syncCoreThreads(syncFirst = false)
        }.onFailure {
            Log.w("Mail", "mark all read failed", it)
            coreThreads = threadsBefore
            kanbanColumns = kanbanBefore
            foldersByAccount = foldersBefore
            coreFolders = coreFoldersBefore
            status = trsError("mobile.status.markAllReadFailed", it)
        }
    }
}

private fun MeronMobileState.markKanbanCardsReadElsewhere(
    key: String,
    readIds: Set<String>,
) {
    kanbanColumns =
        kanbanColumns.mapValues { (otherKey, state) ->
            val cleared = state.threads.filter { it.unread && it.id in readIds }
            if (otherKey == key || cleared.isEmpty()) {
                state
            } else {
                state.copy(
                    threads = state.threads.map { if (it.id in readIds) it.copy(unread = false) else it },
                    unreadCount = state.unreadCount?.let { count -> (count - cleared.sumOf { it.unreadCount.coerceAtLeast(1) }).coerceAtLeast(0) },
                )
            }
        }
}

internal fun MeronMobileState.markKanbanColumnAllRead(column: KanbanColumnSpec) {
    markKanbanColumnsAllRead(listOf(column))
}

internal fun MeronMobileState.markKanbanBoardAllRead() {
    val board = kanbanBoards.firstOrNull { it.id == activeKanbanBoardId } ?: return
    markKanbanColumnsAllRead(board.columns.distinctBy(::kanbanColumnKey))
}

private fun MeronMobileState.markKanbanColumnsAllRead(columns: List<KanbanColumnSpec>) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    if (kanbanMarkingRead || columns.isEmpty()) return
    kanbanMarkingRead = true
    val accounts = coreAccounts
    // Capture every column's targets up front, then clear them all optimistically
    // so the board flips at once. A failed column puts back only its own rows and
    // badge; other columns are never rolled back.
    val plans =
        columns.map { column ->
            val key = kanbanColumnKey(column)
            val state = kanbanColumns[key]
            val unread = state?.threads.orEmpty().filter { it.unread }
            // A saved search marks only its matches read, never the whole folder,
            // so it goes thread by thread like the starred aggregate.
            val perItem = isUnifiedStarredColumn(column) || column.query.isNotBlank()
            val mailAccounts =
                if (perItem) {
                    emptyList()
                } else if (column.accountId == UNIFIED_ACCOUNT_ID) {
                    accounts.filter { it.includedInUnified && !accountSummaryIsRss(it) }
                } else {
                    accounts.filter { it.id == column.accountId && !accountSummaryIsRss(it) }
                }
            val writes =
                mailAccounts.isNotEmpty() ||
                    (perItem && unread.any { !threadIdIsRss(it.id) }) ||
                    unread.any { threadIdIsRss(it.backendThreadId()) }
            val mailFolders =
                mailAccounts.mapNotNull { account ->
                    val folderId =
                        if (column.accountId == UNIFIED_ACCOUNT_ID) {
                            unifiedAccountFolder(foldersByAccount[account.id].orEmpty(), column.folderId)
                        } else {
                            column.folderId
                        }
                    folderId?.let { account.id to it }
                }
            val (folderUnreadNext, folderUnreadBefore) =
                plannedFolderUnread(mailFolders, unread.filter { threadIdIsRss(it.backendThreadId()) })
            KanbanMarkReadPlan(
                column,
                key,
                unread,
                perItem,
                mailAccounts,
                writes,
                state?.unreadCount,
                folderUnreadNext,
                folderUnreadBefore,
            )
        }
    val folderTargets = plans.flatMap { it.folderUnread.keys }.toSet()
    for (plan in plans) {
        val readIds = plan.unread.map { it.id }.toSet()
        updateKanbanColumn(plan.key) { state ->
            state.copy(
                threads = state.threads.map { if (it.id in readIds) it.copy(unread = false) else it },
                unreadCount = if (plan.writes) 0 else state.unreadCount,
            )
        }
        applyLocalFolderUnread(plan.folderUnread)
    }
    val optimisticIds = plans.flatMap { plan -> plan.unread.map { it.id } }.toSet()
    val coreUnreadBefore = coreThreads.filter { it.id in optimisticIds }.associate { it.id to it.unread }
    coreThreads = coreThreads.map { if (it.id in optimisticIds) it.copy(unread = false) else it }
    scope.launch(start = kotlinx.coroutines.CoroutineStart.UNDISPATCHED) {
        folderReadGuard.begin(folderTargets)
        val boardReadVersion = folderReadGuard.version
        val confirmedFolders = mutableMapOf<Pair<String, String>, Int>()
        val succeededIds = mutableSetOf<String>()
        val failedIds = mutableSetOf<String>()
        try {
            val requests = KanbanReadRequests()
            val writeVersions = mutableMapOf<String, Long>()
            var failures = 0
            val marked = mutableSetOf<Pair<String, String>>()
            for (plan in plans) {
                val column = plan.column
                val key = plan.key
                val unread = plan.unread
                val perItem = plan.perItem
                val mailAccounts = plan.mailAccounts
                val unifiedTarget =
                    if (column.accountId != UNIFIED_ACCOUNT_ID && mailAccounts.any { it.includedInUnified }) {
                        // A saved unified search marks only its matches, so it never stands in.
                        columns.firstOrNull {
                            it.accountId == UNIFIED_ACCOUNT_ID && !isUnifiedStarredColumn(it) && it.query.isBlank() &&
                                unifiedColumnMatchesFolder(it.folderId, foldersByAccount[column.accountId].orEmpty(), column.folderId)
                        }
                    } else {
                        null
                    }
                val writeColumn = unifiedTarget ?: column
                val writeVersion = writeVersions.getOrPut(kanbanColumnKey(writeColumn)) { folderReadGuard.version }
                val result =
                    runCatching {
                        withContext(ioDispatcher) {
                            val client = MobileMailCommandClient(core)
                            val responses = mutableListOf<String>()
                            if (mailAccounts.isNotEmpty()) {
                                responses +=
                                    requests.run("folder:${kanbanColumnKey(writeColumn)}") {
                                        val params = MarkAllReadParams(accountId = writeColumn.accountId, folderId = writeColumn.folderId)
                                        val response =
                                            if (writeColumn.accountId == UNIFIED_ACCOUNT_ID) {
                                                accounts
                                                    .filter { it.includedInUnified && !accountSummaryIsRss(it) }
                                                    .forEach { withManagedGoogleAuth(client, it.id) { "" } }
                                                client.markAllRead(params)
                                            } else {
                                                withManagedGoogleAuth(client, writeColumn.accountId) {
                                                    client.markAllRead(params)
                                                }
                                            }
                                        requireCoreAllOk(response)
                                    }
                            }
                            if (perItem) {
                                unread.filterNot { threadIdIsRss(it.id) }.groupBy { it.backendThreadId() }.forEach { (threadId, rows) ->
                                    // A starred card is one message; a saved search's card is the
                                    // whole thread (its id is the thread's), which the core marks
                                    // only when no message ids narrow it, as in toggleRead.
                                    val messageIds =
                                        if (rows.any { it.id == threadId }) emptyList() else rows.map { it.id }.distinct().sorted()
                                    responses +=
                                        requests.run("thread:$threadId:$messageIds") {
                                            withManagedGoogleAuth(client, rows.first().accountId) {
                                                requireCoreOk(client.markRead(MarkReadParams(threadId = threadId, messageIds = messageIds)))
                                            }
                                        }
                                }
                            }
                            unread.filter { threadIdIsRss(it.backendThreadId()) }.groupBy { it.backendThreadId() }.forEach { (threadId, rows) ->
                                val itemKeys = rows.flatMap { it.rssItemKeys() }.distinct().sorted()
                                responses +=
                                    requests.run("rss:$threadId:$itemKeys") {
                                        requireCoreOk(client.markRssRead(RssMarkReadParams(threadId = threadId, seen = true, itemKeys = itemKeys)))
                                    }
                            }
                            responses
                        }
                    }
                result
                    .onSuccess { responses ->
                        // Reapply: a column reload that raced the write may have put
                        // the pre-write rows and badge back over the optimistic clear.
                        val readIds = unread.map { it.id }.toSet()
                        updateKanbanColumn(key) { state ->
                            state.copy(
                                threads = state.threads.map { if (it.id in readIds) it.copy(unread = false) else it },
                                unreadCount = if (plan.writes) 0 else state.unreadCount,
                            )
                        }
                        coreThreads = coreThreads.map { if (it.id in readIds) it.copy(unread = false) else it }
                        // A thread-by-thread write raises no folder change for the
                        // board to reload on, so other columns holding the same
                        // cards (the folder's own, another saved search) follow here.
                        if (plan.perItem) markKanbanCardsReadElsewhere(key, readIds)
                        responses.forEach { applyCoreFolderUnreadChangesAt(it, writeVersion) }
                        val changes = responses.flatMap(::parseFolderUnreadChanges)
                        for ((target, count) in plan.folderUnread) {
                            confirmedFolders[folderReadTarget(target.first, target.second)] = changes
                                .lastOrNull {
                                    it.accountId == target.first && it.folderId.equals(target.second, ignoreCase = target.second.equals(INBOX_FOLDER, true))
                                }?.unread ?: count
                        }
                        applyLocalFolderUnread(
                            plan.folderUnread.mapValues { (target, _) ->
                                folderReadGuard.resolveMutation(target.first, target.second, confirmedFolders.getValue(folderReadTarget(target.first, target.second)), writeVersion)
                            },
                        )
                        succeededIds += readIds
                        marked += unread.map { it.accountId to it.backendThreadId() }
                    }.onFailure {
                        if (it is kotlinx.coroutines.CancellationException) throw it
                        Log.w("Mail", "kanban mark all read failed", it)
                        failures++
                        val readIds = unread.map { it.id }.toSet()
                        failedIds += readIds
                        updateKanbanColumn(key) { state ->
                            state.copy(
                                threads = state.threads.map { if (it.id in readIds) it.copy(unread = true) else it },
                                unreadCount = if (plan.writes) plan.unreadCountBefore else state.unreadCount,
                            )
                        }
                        applyLocalFolderUnread(
                            plan.folderUnreadBefore.mapValues { (target, previous) ->
                                folderReadGuard.resolveMutation(target.first, target.second, confirmedFolders[folderReadTarget(target.first, target.second)] ?: previous, boardReadVersion)
                            },
                        )
                    }
            }
            // A row shared with a column that did succeed is read on the server.
            val revertIds = failedIds - succeededIds
            if (revertIds.isNotEmpty()) {
                coreThreads = coreThreads.map { thread -> coreUnreadBefore[thread.id]?.takeIf { thread.id in revertIds }?.let { thread.copy(unread = it) } ?: thread }
            }
            val language = loadAppLanguageTag(prefs).ifBlank { "en" }
            status =
                if (failures > 0) {
                    localizedString(language, "notification.markReadFailed")
                } else {
                    localizedString(language, "mail.toast.markedReadCount", mapOf("count" to marked.size))
                }
        } finally {
            folderReadGuard.end(folderTargets)
            kanbanMarkingRead = false
        }
    }
}
