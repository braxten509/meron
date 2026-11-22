// Mobile mailbox state changes for message actions.
// Called by the Compose routes and their thread actions.
// Server responses are reconciled with the local mailbox state here.
package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.FolderSummary
import jp.nonbili.meron.shared.MarkStarredParams
import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.MoveThreadParams
import jp.nonbili.meron.shared.RssMarkStarredParams
import jp.nonbili.meron.shared.ThreadActionParams
import jp.nonbili.meron.shared.folderIsDrafts
import jp.nonbili.meron.shared.requireCoreOk
import jp.nonbili.meron.shared.threadIdIsRss
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

internal fun MeronMobileState.toggleMessageStarred(message: MessageBody) {
    val thread = selectedCoreThread ?: return
    val backendThreadId = thread.backendThreadId()
    val starred = !message.starred
    val messagesBefore = messages
    val threadsBefore = coreThreads
    val kanbanBefore = kanbanColumns
    val selectedBefore = selectedCoreThread
    updateMessageEverywhere(message.id) { it.copy(starred = starred) }
    updateThreadEverywhere(thread) { it.copy(starred = messages.any { message -> message.starred }) }
    status = if (starred) "Starring..." else "Unstarring..."
    scope.launch {
        runCatching {
            requireCoreOk(
                withContext(ioDispatcher) {
                    val client = MobileMailCommandClient(core)
                    if (threadIdIsRss(backendThreadId)) {
                        client.markRssStarred(
                            RssMarkStarredParams(threadId = backendThreadId, starred = starred, itemKeys = listOf(message.id)),
                        )
                    } else {
                        withManagedGoogleAuth(client, thread.accountId) {
                            client.markStarred(
                                MarkStarredParams(
                                    threadId = backendThreadId,
                                    starred = starred,
                                    messageIds = listOf(message.id),
                                    folderId = message.folderId,
                                ),
                            )
                        }
                    }
                },
            )
        }.onSuccess {
            refreshUnifiedStarredAfterFlagChange()
            status = trs(if (starred) "mobile.status.starAdded" else "mobile.status.starRemoved")
        }.onFailure {
            messages = messagesBefore
            coreThreads = threadsBefore
            kanbanColumns = kanbanBefore
            selectedCoreThread = selectedBefore
            status = trsError(if (starred) "mobile.status.starFailed" else "mobile.status.unstarFailed", it)
        }
    }
}

internal fun MeronMobileState.deleteMessage(message: MessageBody) {
    val thread = selectedCoreThread ?: return
    // A thread can span folders (e.g. an INBOX message and its replies in Sent),
    // so delete from the message's own folder, not the thread's nominal folder.
    val messageFolder = message.folderId.ifBlank { thread.folder }
    val messagesBefore = messages
    messages = messages.filterNot { it.id == message.id }
    // Deleting the thread's last draft has to take its "Draft" marker down too.
    // The marker is UI-side state the core's next thread list would rewrite, but
    // returning to the list reloads nothing, so leaving it set keeps the badge
    // on a row whose draft is gone. Another draft still here keeps it on.
    // The reply bar's own saved draft lives on the server without a row in
    // messages until a later read brings one back, so the visible rows alone do
    // not say whether this was the thread's last draft.
    val barHoldsADraftHere =
        quickReplyDraftSaved && quickReplyDraftId.isNotBlank() && quickReplyThreadId == thread.backendThreadId()
    val clearedDraftThreadId =
        thread
            .backendThreadId()
            .takeIf {
                folderIsDrafts(messageFolder) &&
                    !barHoldsADraftHere &&
                    messages.none { other -> folderIsDrafts(other.folderId) }
            }
    clearedDraftThreadId?.let { clearThreadDraftEverywhere(it) }
    status = trs("mobile.status.deletingMessage")
    scope.launch {
        runCatching {
            val response =
                withContext(ioDispatcher) {
                    val client = MobileMailCommandClient(core)
                    withManagedGoogleAuth(client, thread.accountId) {
                        client.delete(
                            ThreadActionParams(
                                threadId = thread.id,
                                folderId = messageFolder,
                                messageIds = listOf(message.id),
                            ),
                        )
                    }
                }
            requireCoreOk(response)
        }.onSuccess {
            status = trs("mobile.status.deleteComplete")
        }.onFailure {
            Log.w("Mail", "delete message failed", it)
            messages = messagesBefore
            clearedDraftThreadId?.let { threadId -> markThreadDraftEverywhere(threadId) }
            status = trsError("mail.toast.deleteFailed", it)
        }
    }
}

// Moves one email (not the whole chat) to [folder], or archives it when
// [folder] is null. The message id names its own folder and uid, so a sender
// chat's Sent reply moves out of Sent and an inbox email out of the inbox.
internal fun MeronMobileState.moveMessage(
    message: MessageBody,
    folder: FolderSummary?,
) {
    val thread = selectedCoreThread ?: return
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val messageFolder = message.folderId.ifBlank { thread.folder }
    val targetAccountId = folder?.accountId?.ifBlank { thread.accountId } ?: thread.accountId
    if (folder != null && targetAccountId == thread.accountId && folder.name.equals(messageFolder, ignoreCase = true)) return
    val messagesBefore = messages
    messages = messages.filterNot { it.id == message.id }
    status = trs("mobile.status.moving")
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                withManagedGoogleAuth(client, thread.accountId) {
                    if (folder == null) {
                        client.archive(ThreadActionParams(threadId = message.id))
                    } else {
                        client.move(
                            MoveThreadParams(
                                threadId = message.id,
                                targetFolderId = folder.name,
                                targetAccountId = targetAccountId,
                            ),
                        )
                    }
                }
            }.let { response -> requireTransferAffected(requireCoreOk(response), "moved") }
        }.onSuccess {
            status = trs(if (folder == null) "mail.toast.messageArchived" else "mail.toast.messageMoved")
            if (folder != null) {
                rememberThreadTransfer(RecentThreadTransfer("move", thread.accountId, targetAccountId, folder.name))
            }
            syncCoreThreads(syncFirst = false)
        }.onFailure {
            Log.w("Mail", "move message failed", it)
            messages = messagesBefore
            status =
                when {
                    // One email on an account with no archive mailbox: the plain
                    // message, not the core's raw error text.
                    folder == null && it.message.orEmpty().contains("archive folder not found", ignoreCase = true) ->
                        trs("mail.toast.archiveFailed")
                    else -> trsError(if (folder == null) "mail.toast.archiveFailed" else "mail.toast.moveFailed", it)
                }
        }
    }
}
