// Mobile mailbox state changes for mail.
// Called by the Compose routes and their thread actions.
// Server responses are reconciled with the local mailbox state here.
package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarResult
import androidx.compose.ui.input.key.key
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.isOAuthLoginFailure
import jp.nonbili.meron.shared.parseBulkActionResults
import jp.nonbili.meron.shared.requireCoreOk
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

internal fun MeronMobileState.runCoreThreadAction(
    thread: ThreadSummary,
    /** For the log only. */
    label: String,
    /** Shown once the action is done, unless the undo snackbar is. */
    doneStatus: String?,
    /** Shown, with the reason, if the action fails. */
    failedStatus: String,
    action: suspend MobileMailCommandClient.() -> String,
    update: (List<ThreadSummary>) -> List<ThreadSummary>,
    undoMessage: String? = null,
    onUndo: ((String) -> Unit)? = null,
    /** Runs the moment Undo is tapped, before the server answers. */
    onUndoTap: (() -> Unit)? = null,
    afterSuccess: (() -> Unit)? = null,
    removesThread: Boolean = false,
) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    // Apply optimistically so the UI reacts instantly, then revert if the core
    // call fails. Snapshots taken here back the failure rollback.
    val threadsBefore = coreThreads
    val kanbanBefore = kanbanColumns
    val removal = if (removesThread) suppressRemovedThread(thread.id) else null
    applyOptimisticThreadUpdate(update)
    // The action commits immediately; Undo issues a compensating action (onUndo).
    // Track the commit so an Undo tap waits for it to finish, and is skipped if
    // the commit itself failed (the UI has already rolled back in that case).
    val committed = CompletableDeferred<String?>()
    scope.launch {
        runCatching {
            requireCoreOk(
                withContext(ioDispatcher) {
                    val client = MobileMailCommandClient(core)
                    withManagedGoogleAuth(client, thread.accountId) { client.action() }
                },
            )
        }.onSuccess { response ->
            removal?.complete()
            if (removesThread) releaseMovedThread(response)
            applyCoreFolderUnreadChanges(response)
            if ((undoMessage == null || onUndo == null) && doneStatus != null) status = doneStatus
            committed.complete(response)
            afterSuccess?.invoke()
        }.onFailure {
            removal?.rollback()
            Log.w("Mail", "$label failed", it)
            coreThreads = removal?.filter?.invoke(threadsBefore) ?: threadsBefore
            kanbanColumns = kanbanBefore.mapValues { (_, state) -> state.copy(threads = removal?.filter?.invoke(state.threads) ?: state.threads) }
            status = withReason(failedStatus, it.message) { key, args -> trs(key, args) }
            snackbarHost.currentSnackbarData?.dismiss()
            committed.complete(null)
        }
    }
    // Show the undo snackbar immediately rather than gating it on the round-trip,
    // so the undo window starts the moment the user sees the optimistic change.
    if (undoMessage != null && onUndo != null) {
        scope.launch {
            val result =
                snackbarHost.showSnackbar(
                    message = undoMessage,
                    actionLabel = trs("buttons.undo"),
                    duration = SnackbarDuration.Long,
                )
            if (result == SnackbarResult.ActionPerformed) {
                snackbarHost.currentSnackbarData?.dismiss()
                onUndoTap?.invoke()
                committed.await()?.let { response -> onUndo(response) }
            }
        }
    }
}

// The list and every Kanban column take [update] at once, the columns' unread
// counts following the rows they lose or gain.
private fun MeronMobileState.applyOptimisticThreadUpdate(update: (List<ThreadSummary>) -> List<ThreadSummary>) {
    coreThreads = update(coreThreads).forStarredView(selectedCoreAccountId, selectedCoreFolder)
    kanbanColumns =
        kanbanColumns.mapValues { (key, state) ->
            val (accountId, folderId) = kanbanKeyAccountFolder(key)
            val nextThreads = update(state.threads).forStarredView(accountId, folderId)
            val unreadDelta = loadedUnreadCount(nextThreads) - loadedUnreadCount(state.threads)
            state.copy(
                threads = nextThreads,
                unreadCount = state.unreadCount?.let { (it + unreadDelta).coerceAtLeast(0) },
            )
        }
}

/**
 * Puts the rows [ids] back where [snapshot] had them, around whatever [current]
 * holds now: the rows of a bulk action that failed, while the rows it did move
 * stay gone and the rest of the list keeps any change since.
 */
private fun reinsertThreads(
    current: List<ThreadSummary>,
    snapshot: List<ThreadSummary>,
    ids: Set<String>,
): List<ThreadSummary> {
    val back = snapshot.filter { it.id in ids && current.none { row -> row.id == it.id } }
    if (back.isEmpty()) return current
    val position = snapshot.withIndex().associate { (index, row) -> row.id to index }
    val result = current.toMutableList()
    back.forEach { row ->
        val at = position.getValue(row.id)
        val before = result.indexOfFirst { other -> (position[other.id] ?: -1) > at }
        if (before < 0) result.add(row) else result.add(before, row)
    }
    return result
}

/**
 * Puts the rows [ids] of a bulk action back into what is on screen now, where the snapshots taken at
 * the action had them: the list only while it still shows the mailbox it was taken from
 * ([mailboxKey]), each Kanban column from its own snapshot. Nothing else in the current list or
 * columns changes, so a folder switched to or a row removed since stays as it is.
 */
internal fun MeronMobileState.reinsertBulkRows(
    ids: Set<String>,
    mailboxKey: MailboxCacheKey?,
    threadsBefore: List<ThreadSummary>,
    kanbanBefore: Map<String, KanbanColumnState>,
) {
    if (ids.isEmpty()) return
    if (visibleMailboxKey == mailboxKey) coreThreads = reinsertThreads(coreThreads, threadsBefore, ids)
    kanbanColumns =
        kanbanColumns.mapValues { (key, state) ->
            val nextThreads = reinsertThreads(state.threads, kanbanBefore[key]?.threads.orEmpty(), ids)
            if (nextThreads === state.threads) return@mapValues state
            val unreadDelta = loadedUnreadCount(nextThreads) - loadedUnreadCount(state.threads)
            state.copy(
                threads = nextThreads,
                unreadCount = state.unreadCount?.let { (it + unreadDelta).coerceAtLeast(0) },
            )
        }
}

/** How many rows a bulk Delete sends to the mail server in one go. The whole
 *  group costs about what one row does alone. */
private const val BULK_GROUP_SIZE = 25

/**
 * Archive or delete the selected conversations as a single UI action.
 *
 * Archives or deletes several mail rows as one action: all leave the list at once and, once the server has
 * answered, one snackbar counts the rows that actually moved, its Undo bringing every one of those back.
 * Each row is still answered on its own; one that fails comes back on its own, and is neither counted nor
 * undone.
 *
 * With [groupAction] the rows go to the core a group at a time (one account's rows, [BULK_GROUP_SIZE] at
 * most), and it answers every row of the group (see [parseBulkActionResults]). Without it each row is its
 * own [action].
 */
internal fun MeronMobileState.runBulkThreadRemoval(
    threads: List<ThreadSummary>,
    /** For the log only. */
    label: String,
    failedStatus: String,
    /** The snackbar's text for the rows that moved. */
    undoMessage: (List<ThreadSummary>) -> String,
    action: suspend MobileMailCommandClient.(ThreadSummary) -> String,
    groupAction: (suspend MobileMailCommandClient.(List<ThreadSummary>) -> String)? = null,
) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val ids = threads.map { it.id }.toSet()
    val threadsBefore = coreThreads
    val kanbanBefore = kanbanColumns
    val mailboxKey = visibleMailboxKey
    val shownInInbox = threads.filter(::isShownInInbox).map { it.id }.toSet()
    val removals = threads.associate { it.id to suppressRemovedThread(it.id) }
    applyOptimisticThreadUpdate { rows -> rows.filterNot { it.id in ids } }
    scope.launch {
        val moved = mutableListOf<Pair<ThreadSummary, String>>()
        val failed = mutableListOf<Pair<ThreadSummary, Throwable>>()
        val settle = { thread: ThreadSummary, answer: Result<String> ->
            val removal = removals.getValue(thread.id)
            answer
                .onSuccess { response ->
                    removal.complete()
                    releaseMovedThread(response)
                    applyCoreFolderUnreadChanges(response)
                    moved += thread to response
                }.onFailure {
                    removal.rollback()
                    Log.w("Mail", "$label failed", it)
                    failed += thread to it
                }
        }
        if (groupAction == null) {
            // One after another: the moves share the account's write session.
            for (thread in threads) {
                val answer =
                    runCatching {
                        requireCoreOk(
                            withContext(ioDispatcher) {
                                val client = MobileMailCommandClient(core)
                                withManagedGoogleAuth(client, thread.accountId) { client.action(thread) }
                            },
                        )
                    }
                settle(thread, answer)
            }
        } else {
            // One account's rows at a time: a sign-in is renewed per account.
            val groups = threads.groupBy { it.accountId }.values.flatMap { it.chunked(BULK_GROUP_SIZE) }

            suspend fun ask(rows: List<ThreadSummary>): List<Result<String>> {
                val response =
                    requireCoreOk(
                        withContext(ioDispatcher) {
                            val client = MobileMailCommandClient(core)
                            withManagedGoogleAuth(client, rows.first().accountId) { client.groupAction(rows) }
                        },
                    )
                return parseBulkActionResults(response)?.takeIf { it.size == rows.size }
                    ?: throw RuntimeException("The mail engine gave no answer")
            }
            for (group in groups) {
                val answers =
                    runCatching {
                        val first = ask(group)
                        // A sign-in that ran out fails each row on its own
                        // answer: renew it and try just those rows once more,
                        // as a single delete does.
                        val signedOut = group.indices.filter { isOAuthLoginFailure(first[it].exceptionOrNull()?.message) }
                        val renewed =
                            signedOut.isNotEmpty() &&
                                withContext(ioDispatcher) {
                                    ensureManagedGoogleToken(MobileMailCommandClient(core), group.first().accountId, force = true)
                                }
                        if (!renewed) return@runCatching first
                        val again = ask(signedOut.map { group[it] })
                        first.toMutableList().also { all -> signedOut.forEachIndexed { n, index -> all[index] = again[n] } }
                    }
                group.forEachIndexed { index, thread ->
                    settle(thread, answers.fold({ it[index] }, { Result.failure(it) }))
                }
            }
        }
        if (failed.isNotEmpty()) {
            // Only the rows that failed come back, minus any another action
            // still has out of the list.
            val failedIds = threadRemovalGuard.filter(failed.map { it.first }).map { it.id }.toSet()
            reinsertBulkRows(failedIds, mailboxKey, threadsBefore, kanbanBefore)
            status = withReason(failedStatus, failed.first().second.message) { key, args -> trs(key, args) }
        }
        // Nothing moved: nothing to count or undo.
        if (moved.isEmpty()) return@launch
        val result =
            snackbarHost.showSnackbar(
                message = undoMessage(moved.map { it.first }),
                actionLabel = trs("buttons.undo"),
                duration = SnackbarDuration.Long,
            )
        if (result == SnackbarResult.ActionPerformed) {
            snackbarHost.currentSnackbarData?.dismiss()
            // Back on screen the moment Undo is tapped; the moves back follow.
            reinsertBulkRows(moved.map { it.first.id }.toSet(), mailboxKey, threadsBefore, kanbanBefore)
            status = trs("mobile.status.restored")
            // The list snapshot only backs restoreThread's check for a row a
            // refresh dropped again, which is moot once another mailbox is shown.
            val listSnapshot = if (visibleMailboxKey == mailboxKey) threadsBefore else emptyList()
            moved.forEach { (thread, response) ->
                restoreThread(thread, listSnapshot, kanbanBefore, response, thread.id in shownInInbox)
            }
        }
    }
}
