// Mobile mailbox state changes for archive mail.
// Called by the Compose routes and their thread actions.
// Server responses are reconciled with the local mailbox state here.
package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.filled.Archive
import androidx.compose.material.icons.filled.Delete
import androidx.compose.ui.input.key.key
import jp.nonbili.meron.shared.RemoveRssFeedParams
import jp.nonbili.meron.shared.ThreadActionParams
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.threadIdIsRss

internal fun MeronMobileState.archiveOrRemove(thread: ThreadSummary) {
    if (threadIdIsRss(thread.id)) {
        if (thread.rssItemKeys().isNotEmpty()) {
            status = trs("mobile.status.rssItemsCannotBeRemoved")
            return
        }
        runCoreThreadAction(
            thread = thread,
            label = "Remove feed",
            doneStatus = trs("feeds.toast.feedRemoved"),
            failedStatus = trs("feeds.toast.failedToRemove"),
            removesThread = true,
            action = { removeRssFeed(RemoveRssFeedParams(threadId = thread.id)) },
            update = { threads -> threads.filterNot { it.id == thread.id } },
            afterSuccess = {
                syncCoreThreads(
                    accountOverride = thread.accountId,
                    folderOverride = INBOX_FOLDER,
                    syncFirst = false,
                    successStatus = trs("feeds.toast.feedRemoved"),
                )
            },
        )
    } else {
        val threadsSnapshot = coreThreads
        val kanbanSnapshot = kanbanColumns
        val mailboxKey = visibleMailboxKey
        val shownInInbox = isShownInInbox(thread)
        runCoreThreadAction(
            thread = thread,
            label = "Archive",
            doneStatus = null,
            failedStatus = trs("mail.toast.archiveFailed"),
            removesThread = true,
            action = { archive(ThreadActionParams(threadId = thread.id)) },
            update = { threads -> threads.filterNot { it.id == thread.id } },
            undoMessage = trs("mail.toast.archivedCount", mapOf("count" to 1)),
            onUndo = { response -> restoreThread(thread, threadsSnapshot, kanbanSnapshot, response, shownInInbox) },
            onUndoTap = { showRestoredThread(setOf(thread.id), mailboxKey, threadsSnapshot, kanbanSnapshot) },
        )
    }
}

/** The selection's Archive: feed rows each as [archiveOrRemove] treats them,
 *  the mail rows as one action with one counted, undoable snackbar. */
internal fun MeronMobileState.archiveThreads(threads: List<ThreadSummary>) {
    val (feeds, mail) = threads.partition { threadIdIsRss(it.id) }
    feeds.forEach(::archiveOrRemove)
    if (mail.size <= 1) {
        mail.forEach(::archiveOrRemove)
        return
    }
    runBulkThreadRemoval(
        threads = mail,
        label = "Archive",
        failedStatus = trs("mail.toast.archiveFailed"),
        undoMessage = { moved -> trs("mail.toast.archivedCount", mapOf("count" to moved.size)) },
        action = { thread -> archive(ThreadActionParams(threadId = thread.id)) },
    )
}

/** The selection's Delete, as [archiveThreads] is its Archive. */
internal fun MeronMobileState.deleteThreads(threads: List<ThreadSummary>) {
    val (feeds, mail) = threads.partition { threadIdIsRss(it.backendThreadId()) }
    feeds.forEach(::deleteThread)
    if (mail.size <= 1) {
        mail.forEach(::deleteThread)
        return
    }
    runBulkThreadRemoval(
        threads = mail,
        label = "Delete",
        failedStatus = trs("mail.toast.deleteFailed"),
        undoMessage = { moved -> trs(threadsDeletedStatusKey(moved), mapOf("count" to moved.size)) },
        action = { thread -> delete(ThreadActionParams(threadId = thread.id, folderId = thread.folder)) },
        groupAction = { group ->
            deleteMany(group.map { thread -> ThreadActionParams(threadId = thread.id, folderId = thread.folder) })
        },
    )
}

/** What deleting several rows says: the rows' own key when they agree on one
 *  that counts. "Thread moved to Trash" has no plural form in the catalog, so
 *  a move of several to Trash, or a mix, says how many were deleted. */
private fun threadsDeletedStatusKey(threads: List<ThreadSummary>): String {
    // One left after the others failed: what deleting that one row says.
    threads.singleOrNull()?.let { return threadDeletedStatusKey(it.folder, it.folderRole) }
    val keys = threads.map { threadDeletedStatusKey(it.folder, it.folderRole) }.toSet()
    return keys.singleOrNull()?.takeIf { it != "mail.toast.threadMovedToTrash" } ?: "mail.toast.deletedCount"
}

internal fun MeronMobileState.deleteThread(thread: ThreadSummary) {
    if (threadIdIsRss(thread.backendThreadId())) {
        status = trs(if (thread.rssItemKeys().isNotEmpty()) "mobile.status.rssItemsCannotBeDeleted" else "mobile.status.useRemoveFeed")
        return
    }
    val threadsSnapshot = coreThreads
    val kanbanSnapshot = kanbanColumns
    val mailboxKey = visibleMailboxKey
    val shownInInbox = isShownInInbox(thread)
    runCoreThreadAction(
        thread = thread,
        label = "Delete",
        doneStatus = null,
        failedStatus = trs("mail.toast.deleteFailed"),
        removesThread = true,
        action = { delete(ThreadActionParams(threadId = thread.id, folderId = thread.folder)) },
        update = { threads -> threads.filterNot { it.id == thread.id } },
        undoMessage = trs(threadDeletedStatusKey(thread.folder, thread.folderRole), mapOf("count" to 1)),
        onUndo = { response -> restoreThread(thread, threadsSnapshot, kanbanSnapshot, response, shownInInbox) },
        onUndoTap = { showRestoredThread(setOf(thread.id), mailboxKey, threadsSnapshot, kanbanSnapshot) },
    )
}
