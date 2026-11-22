// Sends a composed message and tracks its pending delivery.
// The editor hands off its draft and attachments to the core.
// Retries retain the identity and content captured for the original send.

package jp.nonbili.meron.ui

import androidx.compose.material.icons.automirrored.filled.Send
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.toSendMailParams
import jp.nonbili.meron.shared.untrustedCertificateProtocol
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

internal fun MeronMobileState.sendMail() {
    if (composeSendInFlight) return
    if (composeSignaturePending || !appSignatureLoaded) {
        status = trs("mobile.status.waitingForSignatureSend")
        return
    }
    val identity = selectedComposeIdentity()
    val accountId = identity?.accountId ?: defaultSendAccountId()
    if (accountId.isBlank()) {
        status = trs("mobile.status.selectAccountBeforeSending")
        return
    }
    val draft = currentComposeDraft()
    if (!draft.canSend) {
        status = trs("mobile.status.completeBeforeSending")
        return
    }
    composeSendInFlight = true
    pendingComposeSend = null
    pendingCertificateRetry = null
    val generation = composeSessionGeneration
    val initialDraftOwners = composeDraftOwnerSnapshot()
    status = trs("composer.status.sending")
    scope.launch {
        composeSaveMutex.withLock {
            val draftOwners =
                if (composeSessionGeneration == generation) {
                    composeDraftOwnerSnapshot()
                } else {
                    initialDraftOwners
                }
            val params =
                runCatching {
                    withContext(ioDispatcher) {
                        val client = MobileMailCommandClient(core)
                        draft.toSendMailParams(accountId = accountId, from = identity?.email.orEmpty()).copy(
                            inReplyTo = composeInReplyTo,
                            references = composeReferences,
                            messageId = allocateCoreMessageId(client = client, accountId = accountId, draft = false),
                        )
                    }
                }.getOrElse {
                    failComposeSend(error = it)
                    return@withLock
                }
            // Remembered so a send refused by a certificate we cannot validate
            // can be sent again as the same message — including its Message-ID —
            // once the user trusts the server.
            val pending = PendingComposeSend(accountId, params, generation, draftOwners)
            pendingComposeSend = pending
            dispatchComposeSend(pending)
        }
    }
}

/**
 * Send a prepared message and reconcile the composer around the result. Split
 * out so a retry sends exactly the message that failed rather than whatever the
 * composer holds by the time the user gets back to it.
 */
private suspend fun MeronMobileState.dispatchComposeSend(
    pending: PendingComposeSend,
) {
    runCatching {
        withContext(ioDispatcher) {
            val client = MobileMailCommandClient(core)
            withManagedGoogleAuth(client, pending.accountId) { client.send(pending.params) }
        }
    }.onSuccess {
        finishComposeSend(pending)
    }.onFailure {
        failComposeSend(pending, it)
    }
}

/** Re-send the message a certificate rejection stopped, after it was trusted. */
internal fun MeronMobileState.retryComposeSend() {
    retryComposeSend(pendingComposeSend ?: return)
}

internal fun MeronMobileState.retryComposeSend(pending: PendingComposeSend) {
    if (composeSendInFlight) return
    pendingComposeSend = pending
    composeSendInFlight = true
    status = trs("composer.status.sending")
    scope.launch {
        composeSaveMutex.withLock {
            dispatchComposeSend(pending)
        }
    }
}

private suspend fun MeronMobileState.finishComposeSend(pending: PendingComposeSend) {
    val discardOutcome = discardComposeDraftOwners(pending.draftOwners)
    // The message went out, so a draft that survived its discard is stale. Left
    // to the next save's cleanup it lingered for as long as no other draft was
    // saved — and, in a thread, could hydrate the reply bar with the sent text.
    composeDraftCleanupOwners = composeDraftCleanupOwners - discardOutcome.failedOwners.toSet()
    discardOutcome.failedOwners.forEach { retrySentDraftDiscard(it) }
    if (pendingComposeSend == pending) pendingComposeSend = null
    if (pendingCertificateRetry == PendingCertificateRetry.Compose(pending)) pendingCertificateRetry = null
    composeSendInFlight = false
    if (composeSessionGeneration == pending.composeSessionGeneration) {
        clearComposeDraftState()
        // A retry can land after the user has left the composer to read the banner;
        // only the captured composer still on screen gets closed.
        if (screen == Screen.Compose) closeCompose()
    }
    errorBanner = null
    status = trs("mobile.status.messageSent")
    syncCoreThreads()
}

private fun MeronMobileState.failComposeSend(
    pending: PendingComposeSend? = pendingComposeSend,
    error: Throwable,
) {
    composeSendInFlight = false
    val message = error.message ?: trs("compose.toast.sendFailed")
    errorBanner = message
    status = trsError("compose.toast.sendFailed", error)
    // Trusting the certificate has to resume the send, not fall back to a
    // sync: the message is still unsent and the composer may be closed by then.
    if (pending != null && untrustedCertificateProtocol(message) != null) {
        pendingCertificateRetry = PendingCertificateRetry.Compose(pending)
    } else if (pending != null && pendingCertificateRetry == PendingCertificateRetry.Compose(pending)) {
        pendingCertificateRetry = null
    }
}
