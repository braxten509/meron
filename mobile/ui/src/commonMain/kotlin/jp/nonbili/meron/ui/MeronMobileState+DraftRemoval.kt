// Discards saved drafts and updates the conversation containing them.
// The compose screen removes the selected draft from visible state.
// If discarding fails, the previous messages are restored.

package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.size
import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.folderIsDrafts
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.withLock

internal fun MeronMobileState.discardComposeDraft() {
    ++composeSessionGeneration
    val identity = selectedComposeIdentity()
    val accountId = composeDraftAccountId.ifBlank { identity?.accountId ?: defaultSendAccountId() }
    val draftId = composeDraftId.takeIf { composeDraftSaved }
    val draftOwners =
        buildList {
            if (!draftId.isNullOrBlank() && accountId.isNotBlank()) add(ComposeDraftOwner(accountId, draftId, composeDraftThreadId()))
            addAll(composeDraftCleanupOwners)
        }.distinctBy { it.accountId to it.draftId }
    composeDraftCleanupOwners = draftOwners
    val returnScreen = composeReturnScreen
    val thread = selectedCoreThread
    val draftThread = thread?.takeIf { folderIsDrafts(it.folder) }
    if (!draftId.isNullOrBlank() && accountId.isBlank()) {
        status = trs("mobile.status.selectAccountBeforeDiscarding")
        return
    }
    val previousMessages = messages
    val previousThread = selectedCoreThread
    val previousThreads = coreThreads
    val previousKanbanColumns = kanbanColumns
    val optimisticallyClearedThreadId = removeDiscardedDraftFromOpenThread(draftId, composeDraftThreadId())
    if (draftThread != null) {
        removeThreadEverywhere(draftThread.id)
        locallyDiscardedThreadIds = locallyDiscardedThreadIds + draftThread.id
    }
    clearComposeDraftState()
    screen = returnScreen
    status = trs("mobile.status.discardingDraft")
    scope.launch(start = CoroutineStart.UNDISPATCHED) {
        composeSaveMutex.withLock {
            val outcome = discardComposeDraftOwners(draftOwners)
            if (outcome.failedOwners.isEmpty()) {
                status = trs("mail.toast.draftDiscarded")
                syncCoreThreads(syncFirst = true)
                if (thread != null) {
                    refreshKanbanColumnsForMailEvent(accountId, thread.folder, refresh = true)
                }
                if (draftThread == null) {
                    runCatching { reloadCurrentThreadMessages() }
                }
            } else {
                val openDraftFailed = draftOwners.firstOrNull { it.draftId == draftId } in outcome.failedOwners
                // Roll back only the drafts that survived: one owner failing must not
                // bring back a draft another owner really did delete. The open
                // draft's own thread is restored only when its draft is the one that
                // stayed behind.
                if (openDraftFailed) {
                    restoreUndiscardedDraftMessages(previousMessages, outcome.failedOwners)
                    selectedCoreThread = previousThread
                    coreThreads = previousThreads
                    kanbanColumns = previousKanbanColumns
                    if (draftThread != null) {
                        locallyDiscardedThreadIds = locallyDiscardedThreadIds - draftThread.id
                    }
                }
                // The snapshots above predate the per-owner clearing, and whole-map
                // restores would also drop a marker set meanwhile by an autosave for
                // another thread, so the flags are re-applied one thread at a time.
                outcome.clearedThreadIds.forEach { clearThreadDraftEverywhere(it) }
                outcome.failedOwners.forEach { markThreadDraftEverywhere(it.threadId) }
                if (openDraftFailed) {
                    optimisticallyClearedThreadId?.let { markThreadDraftEverywhere(it) }
                }
                status = trs("mobile.status.draftDiscardFailed")
            }
        }
    }
}

/**
 * Drop a discarded draft from the open thread, returning the id of the thread
 * whose draft marker was cleared, or null when nothing was cleared.
 * [draftThreadId] is the thread the caller knows the draft belongs to.
 */
internal fun MeronMobileState.removeDiscardedDraftFromOpenThread(
    draftId: String?,
    draftThreadId: String = "",
): String? {
    val normalizedDraftId = draftId?.normalizedComposeDraftId().orEmpty()
    if (normalizedDraftId.isBlank()) return null
    val remaining =
        messages.filterNot { message ->
            message.id == "local-draft-$normalizedDraftId" ||
                message.messageId.normalizedComposeDraftId() == normalizedDraftId
        }
    val foundInOpenThread = remaining.size != messages.size
    // Finding the draft in the open thread proves it lived there; otherwise only
    // the caller knows. Absence proves nothing on its own: a quick reply saves a
    // draft without adding a message locally, the list is empty while a thread
    // loads, and a cleanup owner can belong to a thread that is no longer open.
    val threadId = if (foundInOpenThread) selectedCoreThread?.id.orEmpty() else draftThreadId
    if (threadId.isBlank()) return null
    if (foundInOpenThread) messages = remaining
    // Another draft still sitting in the thread keeps the marker on.
    if (threadId == selectedCoreThread?.id && remaining.any { folderIsDrafts(it.folderId) }) {
        selectedCoreThread = selectedCoreThread?.copy(hasDraft = true)
        return null
    }
    clearThreadDraftEverywhere(threadId)
    return threadId
}

/**
 * Puts back the messages of drafts a failed discard left behind, at roughly
 * their old place, keeping whatever arrived while the discard was in flight.
 */
private fun MeronMobileState.restoreUndiscardedDraftMessages(
    previousMessages: List<MessageBody>,
    owners: List<ComposeDraftOwner>,
) {
    val draftIds = owners.map { it.draftId.normalizedComposeDraftId() }.toSet()
    val presentIds = messages.map { it.id }.toSet()
    val restored = messages.toMutableList()
    previousMessages.forEachIndexed { index, message ->
        if (message.id in presentIds) return@forEachIndexed
        val belongsToOwner =
            draftIds.any { draftId ->
                message.id == "local-draft-$draftId" || message.messageId.normalizedComposeDraftId() == draftId
            }
        if (belongsToOwner) restored.add(index.coerceAtMost(restored.size), message)
    }
    messages = restored
}
