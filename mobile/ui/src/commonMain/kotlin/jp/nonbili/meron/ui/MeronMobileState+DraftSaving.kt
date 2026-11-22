// Coordinates ownership of draft saves and discards in the full mobile composer.
// The editor records the account and server identity associated with each save.
// Completion is matched to its owner before replacing or discarding a draft.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.DiscardDraftParams
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.newDraftMessageId
import jp.nonbili.meron.shared.toSaveDraftParams
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

internal fun MeronMobileState.composeDraftOwnerSnapshot(): List<ComposeDraftOwner> =
    buildList {
        if (composeDraftSaved && composeDraftAccountId.isNotBlank() && composeDraftId.isNotBlank()) {
            add(ComposeDraftOwner(composeDraftAccountId, composeDraftId, composeDraftThreadId()))
        }
        addAll(composeDraftCleanupOwners)
    }.distinctBy { it.accountId to it.draftId }

/** The thread a compose draft belongs to, blank unless composing from a thread. */
internal fun MeronMobileState.composeDraftThreadId(): String = selectedCoreThread?.takeIf { composeReturnScreen == Screen.Thread }?.id.orEmpty()

internal data class ComposeDraftDiscardOutcome(
    val clearedThreadIds: Set<String> = emptySet(),
    val failedOwners: List<ComposeDraftOwner> = emptyList(),
)

internal suspend fun MeronMobileState.discardComposeDraftOwners(owners: List<ComposeDraftOwner>): ComposeDraftDiscardOutcome {
    val clearedThreadIds = mutableSetOf<String>()
    val failedOwners = mutableListOf<ComposeDraftOwner>()
    owners.forEach { owner ->
        val discarded =
            runCatching {
                withContext(ioDispatcher) {
                    MobileMailCommandClient(core).discardDraft(
                        DiscardDraftParams(accountId = owner.accountId, draftId = owner.draftId),
                    )
                }
            }.isSuccess
        if (discarded) {
            composeDraftCleanupOwners = composeDraftCleanupOwners - owner
            // A discard can also happen on a later retry from saveComposeDraft, so
            // the local copy of the draft is dropped here rather than at the send.
            removeDiscardedDraftFromOpenThread(owner.draftId, owner.threadId)?.let { clearedThreadIds.add(it) }
        } else {
            failedOwners.add(owner)
        }
    }
    return ComposeDraftDiscardOutcome(clearedThreadIds, failedOwners)
}

/**
 * Delete the server-side draft of a composer the user has emptied out, mirroring
 * the reply bar's [discardQuickReplyDraftIfEmpty]. Reports whether anything was
 * deleted.
 */
private suspend fun MeronMobileState.discardEmptiedComposeDraft(generation: Int): Boolean {
    val openDraft =
        ComposeDraftOwner(composeDraftAccountId, composeDraftId, composeDraftThreadId())
            .takeIf { composeDraftSaved && it.accountId.isNotBlank() && it.draftId.isNotBlank() }
    // The composer's own draft is deliberately not queued in
    // composeDraftCleanupOwners: it keeps its id, so a failed discard followed by
    // the user typing again would save under that id and then have the cleanup
    // delete the replacement. A retry comes from the next blank save instead.
    val owners = (listOfNotNull(openDraft) + composeDraftCleanupOwners).distinctBy { it.accountId to it.draftId }
    if (owners.isEmpty()) return false
    val outcome = discardComposeDraftOwners(owners)
    if (outcome.failedOwners.isNotEmpty()) return false
    // A newer composer may have opened while the discard was in flight; its draft
    // id and account are not this session's to clear.
    if (generation != composeSessionGeneration) return false
    composeDraftId = ""
    composeDraftSaved = false
    composeDraftAccountId = ""
    syncCoreThreads(syncFirst = false)
    return true
}

internal fun MeronMobileState.saveComposeDraft() {
    val generation = composeSessionGeneration
    scope.launch(start = CoroutineStart.UNDISPATCHED) {
        saveComposeDraft(showStatus = true, generation = generation)
    }
}

internal fun MeronMobileState.autoSaveComposeDraft() {
    val generation = composeSessionGeneration
    scope.launch(start = CoroutineStart.UNDISPATCHED) {
        saveComposeDraft(showStatus = false, generation = generation)
    }
}

internal suspend fun MeronMobileState.saveComposeDraft(
    showStatus: Boolean,
    generation: Int,
    keepObsoleteDraft: Boolean = false,
): Boolean {
    return composeSaveMutex.withLock {
        if (generation != composeSessionGeneration) return@withLock false
        // A send is about to discard the draft; saving now could resurrect it.
        if (composeSendInFlight) return@withLock false
        if (composeSignaturePending || !appSignatureLoaded) {
            if (showStatus) status = trs("mobile.status.waitingForSignatureSave")
            return@withLock false
        }
        val identityGeneration = composeIdentityGeneration
        val identity = selectedComposeIdentity()
        val accountId = identity?.accountId ?: defaultSendAccountId()
        if (accountId.isBlank()) {
            if (showStatus) status = trs("mobile.status.selectAccountBeforeSaving")
            return@withLock false
        }
        val draft = currentComposeDraft()
        if (composeIsBlank()) {
            // Everything the user wrote is gone. A copy already on the server has
            // to go with it, or closing and reopening would bring back the text
            // they deliberately erased.
            val discarded = discardEmptiedComposeDraft(generation)
            // Same reason the save path rechecks below: a status line belongs to
            // whichever composer is open now.
            if (generation != composeSessionGeneration) return@withLock false
            if (showStatus) status = trs(if (discarded) "mail.toast.draftDiscarded" else "mobile.status.nothingToSave")
            return@withLock false
        }
        val draftId = composeDraftId.ifBlank { newDraftMessageId(accountId) }
        val cleanupOwners = composeDraftCleanupOwners
        val draftThreadId = composeDraftThreadId().ifBlank { null }
        val inReplyTo = composeInReplyTo
        val references = composeReferences
        if (showStatus) status = trs("composer.status.savingDraft")
        var resolvedDraftId = draftId
        var allocatedRemoteDraft = false
        val result =
            runCatching {
                withContext(ioDispatcher) {
                    val client = MobileMailCommandClient(core)
                    resolvedDraftId =
                        if (draftId.startsWith("local-draft-")) {
                            allocateCoreMessageId(client, accountId, draft = true).also { allocatedRemoteDraft = true }
                        } else {
                            draftId
                        }
                    val params =
                        draft
                            .toSaveDraftParams(
                                accountId = accountId,
                                draftId = resolvedDraftId,
                                from = identity?.email.orEmpty(),
                            ).copy(
                                inReplyTo = inReplyTo,
                                references = references,
                            )
                    withManagedGoogleAuth(client, accountId) { client.saveDraft(params) }
                    resolvedDraftId
                }
            }
        val sessionObsolete = generation != composeSessionGeneration
        val identityObsolete = identityGeneration != composeIdentityGeneration
        val obsolete = sessionObsolete || identityObsolete || composeSendInFlight
        if (obsolete) {
            val keepClosingDraft = keepObsoleteDraft && sessionObsolete && result.isSuccess
            if (keepClosingDraft) {
                discardComposeDraftOwners(cleanupOwners)
                draftThreadId?.let { markThreadDraftEverywhere(it) }
                syncCoreThreads(syncFirst = false)
                return@withLock true
            }
            if (allocatedRemoteDraft && resolvedDraftId.isNotBlank()) {
                discardObsoleteComposeDraft(accountId, resolvedDraftId)
            }
            return@withLock false
        }
        result.fold(
            onSuccess = { savedDraftId ->
                composeDraftId = savedDraftId
                composeDraftSaved = true
                composeDraftAccountId = accountId
                discardComposeDraftOwners(cleanupOwners)
                // Only the conversation this draft answers, and only when it was
                // written from one. `selectedCoreThread` outlives leaving a
                // conversation: a new message composed after backing out of one
                // flagged that conversation "Draft", and since the flag is
                // re-stamped onto every list load (locallyDraftedThreadIds) and
                // the sent draft's discard has no conversation to clear, the
                // badge stayed for the session.
                draftThreadId?.let { markThreadDraftEverywhere(it) }
                if (showStatus) status = trs("mobile.status.draftSaved")
                syncCoreThreads(syncFirst = false)
                // Nothing on an open conversation changed for a draft of its own.
                if (draftThreadId != null) runCatching { reloadCurrentThreadMessages() }
                true
            },
            onFailure = {
                status =
                    if (showStatus) {
                        trsError("mobile.status.draftSaveFailed", it)
                    } else {
                        trsError("composer.status.draftAutosaveFailed", it)
                    }
                false
            },
        )
    }
}

private suspend fun MeronMobileState.discardObsoleteComposeDraft(
    accountId: String,
    draftId: String,
) {
    runCatching {
        withContext(ioDispatcher) {
            MobileMailCommandClient(core).discardDraft(DiscardDraftParams(accountId = accountId, draftId = draftId))
        }
    }
}
