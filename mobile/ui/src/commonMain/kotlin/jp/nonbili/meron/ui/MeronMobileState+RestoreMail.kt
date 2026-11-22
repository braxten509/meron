// Mobile mailbox state changes for restore mail.
// Called by the Compose routes and their thread actions.
// Server responses are reconciled with the local mailbox state here.
package jp.nonbili.meron.ui

import androidx.compose.material.icons.filled.Archive
import androidx.compose.ui.input.key.key
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.MoveThreadParams
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.parseThreadActionLocationResponse
import jp.nonbili.meron.shared.requireCoreOk
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

// Puts the undone rows [ids] back the moment Undo is tapped, where the
// snapshots taken at the action had them; restoreThread then moves the mail
// back on the server, which can take seconds (the archive/delete itself has to
// finish first). Only those rows: putting the whole snapshot back would also
// bring back rows archived or moved since, and drop rows that arrived. A row
// still on screen (a Meron-folder move only relabels it) takes its snapshot
// version back.
internal fun MeronMobileState.showRestoredThread(
    ids: Set<String>,
    mailboxKey: MailboxCacheKey?,
    threadsSnapshot: List<ThreadSummary>,
    kanbanSnapshot: Map<String, KanbanColumnState>,
) {
    val revert = { rows: List<ThreadSummary>, snapshot: List<ThreadSummary> ->
        val before = snapshot.filter { it.id in ids }.associateBy { it.id }
        if (before.isEmpty() || rows.none { it.id in before }) rows else rows.map { before[it.id] ?: it }
    }
    if (visibleMailboxKey == mailboxKey) coreThreads = revert(coreThreads, threadsSnapshot)
    kanbanColumns =
        kanbanColumns.mapValues { (key, state) ->
            val next = revert(state.threads, kanbanSnapshot[key]?.threads.orEmpty())
            if (next === state.threads) state else state.copy(threads = next)
        }
    reinsertBulkRows(ids, mailboxKey, threadsSnapshot, kanbanSnapshot)
    status = trs("mobile.status.restored")
}

/** A row the Inbox (an account's or the unified one) lists from another
 *  folder: mail AI sorting left there and shows in the Inbox. Its Undo shows
 *  it in the Inbox again. */
internal fun MeronMobileState.isShownInInbox(thread: ThreadSummary): Boolean =
    selectedCoreFolder.equals(INBOX_FOLDER, ignoreCase = true) &&
        selectedCoreAccountId != MERON_FOLDERS_ACCOUNT_ID &&
        thread.folder.isNotBlank() &&
        !thread.folder.equals(INBOX_FOLDER, ignoreCase = true)

// Takes the row out again when the server move back is not possible.
private fun MeronMobileState.hideRestoredThread(thread: ThreadSummary) {
    coreThreads = coreThreads.filterNot { it.id == thread.id }
    kanbanColumns = kanbanColumns.mapValues { (_, state) -> state.copy(threads = state.threads.filterNot { it.id == thread.id }) }
}

// Moves a thread back to the folder it was in before an archive/delete, backing
// the "Undo" snackbar action. The row is already back (showRestoredThread).
internal fun MeronMobileState.restoreThread(
    thread: ThreadSummary,
    threadsSnapshot: List<ThreadSummary>,
    kanbanSnapshot: Map<String, KanbanColumnState>,
    actionResponse: String,
    /** The row was shown in the Inbox from its own folder ([isShownInInbox]). */
    shownInInbox: Boolean = false,
) {
    if (!coreLoaded) return
    val location = parseThreadActionLocationResponse(actionResponse)
    // Archived Meron-only (no archive mailbox): nothing moved on the server,
    // so Undo only takes it out of Archive again, back into its earlier
    // Meron folder when it had one.
    if (location.meronOnly) {
        val ids = listOf(location.threadId.ifBlank { thread.id })
        scope.launch {
            runCatching {
                withContext(ioDispatcher) {
                    val client = MobileMailCommandClient(core)
                    requireCoreOk(
                        location.meronFolderId.ifBlank { null }?.let { client.moveToMeronFolder(it, ids, undo = true) }
                            ?: client.moveOutOfMeronFolder(ids, undo = true),
                    )
                }
            }.onSuccess {
                threadRemovalGuard.release(thread.id)
                mailboxCache = mailboxCache.filterKeys { it.accountId != MERON_FOLDERS_ACCOUNT_ID }
                refreshMeronFolders()
                syncCoreThreads(syncFirst = false)
            }.onFailure {
                hideRestoredThread(thread)
                status = trsError("mobile.status.undoFailed", it)
            }
        }
        return
    }
    // Move back only the messages this action moved, so other messages of the
    // same conversation already in Archive/Trash stay there. When the core
    // could not tell which copies those are, there is no safe way back: any
    // other would move mail the action never touched.
    val undoThreadId =
        undoSourceThreadId(thread, actionResponse)?.takeIf { location.targetUids.isNotEmpty() }
            ?: run {
                hideRestoredThread(thread)
                status = trs("mobile.status.undoUnavailable")
                return
            }
    val undoMessageIds = location.targetUids.map { "$undoThreadId#$it" }
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                requireCoreOk(
                    MobileMailCommandClient(core).move(
                        MoveThreadParams(
                            threadId = undoThreadId,
                            targetFolderId = thread.folder,
                            messageIds = undoMessageIds,
                            undo = true,
                            showInInbox = shownInInbox,
                            meronFolderId = location.meronFolderId.ifBlank { null },
                            meronSource = location.meronSource.ifBlank { null },
                        ),
                    ),
                )
            }
        }.onSuccess {
            threadRemovalGuard.release(thread.id)
            // A list refresh that landed before the move back finished would
            // have dropped the row again; read the list once more if so.
            if (coreThreads.none { it.id == thread.id } && threadsSnapshot.any { it.id == thread.id }) {
                syncCoreThreads(syncFirst = false)
            }
        }.onFailure {
            hideRestoredThread(thread)
            status = trsError("mobile.status.undoFailed", it)
        }
    }
}

private fun undoSourceThreadId(
    thread: ThreadSummary,
    actionResponse: String,
): String? {
    val location = parseThreadActionLocationResponse(actionResponse)
    if (location.permanent) return null
    if (location.threadId.isNotBlank()) return location.threadId
    if (location.folder.isBlank()) return thread.id
    val threadKey = thread.id.substringAfterLast("#", missingDelimiterValue = "")
    if (thread.accountId.isBlank() || threadKey.isBlank()) return thread.id
    return "${thread.accountId}#${location.folder}#$threadKey"
}
