// Mobile mailbox state changes for read state.
// Called by the Compose routes and their thread actions.
// Server responses are reconciled with the local mailbox state here.
package jp.nonbili.meron.ui

import androidx.compose.foundation.lazy.items
import androidx.compose.ui.input.key.key
import jp.nonbili.meron.shared.MarkReadParams
import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.RssMarkReadParams
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.mailThreadIdFolder
import jp.nonbili.meron.shared.requireCoreOk
import jp.nonbili.meron.shared.threadIdIsRss
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

internal fun MeronMobileState.toggleRead(thread: ThreadSummary) {
    val backendThreadId = thread.backendThreadId()
    val isRssThread = threadIdIsRss(backendThreadId)
    val itemIds = listOf(thread.id).takeIf { backendThreadId != thread.id }.orEmpty()
    runCoreThreadAction(
        thread = thread,
        label = if (thread.unread) "Mark read" else "Mark unread",
        doneStatus = trs(if (thread.unread) "mail.toast.markedReadCount" else "mail.toast.markedUnreadCount", mapOf("count" to 1)),
        failedStatus = trs(if (thread.unread) "notification.markReadFailed" else "mobile.status.markUnreadFailed"),
        action = {
            readMarkMutex.withLock {
                if (isRssThread) {
                    markRssRead(RssMarkReadParams(threadId = backendThreadId, seen = thread.unread, itemKeys = thread.rssItemKeys()))
                } else {
                    markRead(MarkReadParams(threadId = backendThreadId, seen = thread.unread, messageIds = itemIds))
                }
            }
        },
        // Marking unread flags the newest message only (see the core), so the
        // card comes back as a single unread message rather than claiming every
        // message in the thread is unread.
        update = { threads ->
            threads.map {
                if (it.id == thread.id) {
                    it.copy(unread = !thread.unread, unreadCount = if (thread.unread) 0 else 1)
                } else {
                    it
                }
            }
        },
    )
}

internal fun MeronMobileState.updateMessageEverywhere(
    messageId: String,
    update: (MessageBody) -> MessageBody,
) {
    messages = messages.map { if (it.id == messageId) update(it) else it }
}

internal fun MeronMobileState.toggleMessageRead(message: MessageBody) {
    val thread = selectedCoreThread ?: return
    val backendThreadId = thread.backendThreadId()
    val seen = message.unread
    val (op, changes) = readLedger.begin(listOf(Triple(message.id, message.folderId, message.unread)), unread = !seen)
    applyReadChanges(thread, changes)
    status = trs(if (seen) "mobile.status.markingRead" else "mobile.status.markingUnread")
    scope.launch {
        readMarkMutex.withLock {
            runCatching {
                requireCoreOk(
                    withContext(ioDispatcher) {
                        val client = MobileMailCommandClient(core)
                        // Feed items carry "<thread>#<item key>" ids the core splits
                        // apart, so one item reads back the same way a message does.
                        if (threadIdIsRss(backendThreadId)) {
                            client.markRssRead(
                                RssMarkReadParams(threadId = backendThreadId, seen = seen, itemKeys = listOf(message.id)),
                            )
                        } else {
                            withManagedGoogleAuth(client, thread.accountId) {
                                client.markRead(
                                    MarkReadParams(
                                        threadId = backendThreadId,
                                        seen = seen,
                                        messageIds = listOf(message.id),
                                        folderId = message.folderId,
                                    ),
                                )
                            }
                        }
                    },
                )
            }.onSuccess {
                applyReadChanges(thread, readLedger.finish(op, succeeded = true))
                status = trs(if (seen) "mail.toast.markedReadCount" else "mail.toast.markedUnreadCount", mapOf("count" to 1))
            }.onFailure {
                applyReadChanges(thread, readLedger.finish(op, succeeded = false))
                Log.w("Mail", "toggle message read failed", it)
                status = trsError("mobile.status.messageUpdateFailed", it)
            }
        }
    }
}

// Scroll-driven read marking, like desktop: local state flips optimistically
// and settles through readLedger, so a failed folder turns its messages unread
// again unless a later write still pending covers them. ThreadUi then holds a
// turned-back message like a hand-unread one until it leaves the viewport, so
// reading it again retries instead of looping on a dead connection. For RSS the
// message ids ("<thread>#<item key>") pass through as item keys; the core
// strips the thread prefix.
internal fun MeronMobileState.markMessagesReadOnScroll(messageIds: List<String>) {
    val thread = selectedCoreThread ?: return
    val backendThreadId = thread.backendThreadId()
    val ids = messageIds.distinct().filter { id -> messages.any { it.id == id && it.unread } }
    if (ids.isEmpty()) return
    // Group by folder here, not inside the coroutine: a thread can span folders
    // and `messages` may be replaced (thread switch, page reload, a rollback from
    // a concurrent action) before the IO block runs, which would silently drop
    // the mark instead of sending it.
    val idsByFolder = messages.filter { it.id in ids }.groupBy { it.folderId }
    // One op per folder: each folder's request succeeds or fails on its own.
    val ops =
        idsByFolder.mapValues { (_, folderMessages) ->
            val (op, changes) = readLedger.begin(folderMessages.map { Triple(it.id, it.folderId, it.unread) }, unread = false)
            applyReadChanges(thread, changes)
            op
        }
    scope.launch {
        readMarkMutex.withLock {
            // One response per folder, each carrying only that folder's unread counts.
            val responses = mutableListOf<String>()
            val failedFolders = mutableSetOf<String>()
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                if (threadIdIsRss(backendThreadId)) {
                    runCatching {
                        requireCoreOk(client.markRssRead(RssMarkReadParams(threadId = backendThreadId, seen = true, itemKeys = ids)))
                    }.onSuccess { responses += it }
                        .onFailure {
                            Log.w("Mail", "scroll mark read failed", it)
                            failedFolders += idsByFolder.keys
                        }
                } else {
                    idsByFolder.forEach { (folder, folderMessages) ->
                        runCatching {
                            requireCoreOk(
                                withManagedGoogleAuth(client, thread.accountId) {
                                    client.markRead(
                                        MarkReadParams(
                                            threadId = backendThreadId,
                                            seen = true,
                                            messageIds = folderMessages.map { it.id },
                                            folderId = folder,
                                        ),
                                    )
                                },
                            )
                        }.onSuccess { responses += it }
                            .onFailure {
                                Log.w("Mail", "scroll mark read failed", it)
                                failedFolders += folder
                            }
                    }
                }
            }
            // Apply whatever came back, so a later folder failing does not discard the
            // unread counts of the folders that succeeded.
            responses.forEach(::applyCoreFolderUnreadChanges)
            ops.forEach { (folder, op) ->
                applyReadChanges(thread, readLedger.finish(op, succeeded = folder !in failedFolders))
            }
        }
    }
}

// The conversation was viewed to the bottom: mark the whole thread read, which
// also covers unread messages on older pages that were never loaded. The core
// marks every message it still has unread, so the op covers every loaded
// message — including one whose own per-message mark is pending and may fail,
// which this request then reads anyway. Unread on unloaded pages exists only in
// the card's count; that share comes off optimistically and goes back on
// failure.
internal fun MeronMobileState.markThreadReadOnScroll() {
    val thread = selectedCoreThread ?: return
    val backendThreadId = thread.backendThreadId()
    if (!thread.unread && messages.none { it.unread }) return
    val (op, changes) = readLedger.begin(messages.map { Triple(it.id, it.folderId, it.unread) }, unread = false)
    val loadedOnCard = changes.count { it.moved && cardCountsFolder(thread, it.folder) }
    val cardUnread = if (thread.unread) thread.unreadCount.coerceAtLeast(1) else 0
    val unloadedOnCard = (cardUnread - loadedOnCard).coerceAtLeast(0)
    applyReadChanges(thread, changes, extraCardDelta = -unloadedOnCard)
    scope.launch {
        readMarkMutex.withLock {
            runCatching {
                requireCoreOk(
                    withContext(ioDispatcher) {
                        val client = MobileMailCommandClient(core)
                        if (threadIdIsRss(backendThreadId)) {
                            client.markRssRead(RssMarkReadParams(threadId = backendThreadId, seen = true))
                        } else {
                            withManagedGoogleAuth(client, thread.accountId) {
                                client.markRead(MarkReadParams(threadId = backendThreadId, seen = true))
                            }
                        }
                    },
                )
            }.onSuccess { response ->
                applyCoreFolderUnreadChanges(response)
                applyReadChanges(thread, readLedger.finish(op, succeeded = true))
            }.onFailure {
                Log.w("Mail", "thread mark read failed", it)
                applyReadChanges(thread, readLedger.finish(op, succeeded = false), extraCardDelta = unloadedOnCard)
            }
        }
    }
}

// Show read-state changes from readLedger. The loaded messages take the
// reported state outright — they may have been replaced by a reload since the
// ledger last spoke — while the card's count moves only by what the ledger saw
// change, in the card's own mailbox (plus `extraCardDelta`, for unread the card
// counts on pages that are not loaded). Relative, not a snapshot, so unread
// that arrived meanwhile stays counted.
private fun MeronMobileState.applyReadChanges(
    thread: ThreadSummary,
    changes: List<MessageReadLedger.Change>,
    extraCardDelta: Int = 0,
) {
    val shown = changes.associate { it.id to it.unread }
    if (messages.any { message -> shown[message.id]?.let { it != message.unread } == true }) {
        messages = messages.map { message -> shown[message.id]?.let { message.copy(unread = it) } ?: message }
    }
    val delta =
        changes.filter { it.moved && cardCountsFolder(thread, it.folder) }.sumOf { if (it.unread) 1 else -1 } + extraCardDelta
    if (delta == 0) return
    currentThreadSummary(thread.id)?.let { current -> updateThreadEverywhere(current) { threadWithUnreadDelta(it, delta) } }
}

// Whether a message in `folder` counts toward the thread card's unread count.
// That count is mailbox-scoped, so reading a Sent reply must not clear the
// INBOX card's remaining unread state. The card's folder comes from the thread
// id, since `thread.folder` holds the Kanban column id when the thread was
// opened from a column: a role for the unified columns, no mailbox at all for
// starred.
private fun MeronMobileState.cardCountsFolder(
    thread: ThreadSummary,
    folder: String,
): Boolean {
    val cardFolder = mailThreadIdFolder(thread.backendThreadId()).ifBlank { thread.folder }
    return threadCardCoversFolder(cardFolder, foldersByAccount[thread.accountId].orEmpty(), folder)
}

// The live copy of a thread card, which may have moved on since an action
// captured it: the open thread, the list, or a Kanban column.
private fun MeronMobileState.currentThreadSummary(threadId: String): ThreadSummary? =
    selectedCoreThread?.takeIf { it.id == threadId }
        ?: coreThreads.firstOrNull { it.id == threadId }
        ?: kanbanColumns.values.firstNotNullOfOrNull { state -> state.threads.firstOrNull { it.id == threadId } }
