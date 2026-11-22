// Sends and retries a reply composed in the thread view.
// The pending-send record retains the account, draft owner and displayed message.
// Completion reconciles that state with the conversation currently on screen.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.DiscardDraftParams
import jp.nonbili.meron.shared.MessageAttachment
import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.SendStatus
import jp.nonbili.meron.shared.threadIdIsRss
import jp.nonbili.meron.shared.toReplyMailParams
import jp.nonbili.meron.shared.untrustedCertificateProtocol
import jp.nonbili.meron.shared.withReplyQuote
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

internal fun MeronMobileState.sendQuickReply() {
    if (quickReplySendInFlight) return
    val thread = selectedCoreThread
    val threadId = thread?.backendThreadId().orEmpty()
    val accountId = thread?.accountId?.ifBlank { defaultSendAccountId() }.orEmpty()
    val parent = quickReplyParent()
    val sentBody = quickReplyBody.trim()
    val sentAttachments = quickReplyAttachments
    if (accountId.isBlank() || thread == null || parent == null) {
        status = trs("mobile.status.openThreadBeforeReplying")
        return
    }
    if (threadIdIsRss(thread.id)) {
        status = trs("mobile.status.rssNoReplies")
        return
    }
    // A box holding only the seeded signature is not a reply worth sending.
    if (quickReplyIsBlank()) {
        status = trs("mobile.status.writeReplyBeforeSending")
        return
    }
    quickReplyFailure = ""
    quickReplyAutosaveJob?.cancel()
    quickReplySendInFlight = true
    // Which conversation the send belongs to, so a read landing mid-send knows
    // the draft at its tail is this send's and not a reply left unfinished.
    // The bar is empty from the click, so its own emptiness no longer says so.
    quickReplySendThreadId = threadId
    pendingCertificateRetry = null
    pendingQuickReplySend = null
    val generation = quickReplyGeneration
    // The bar's draft as it stood at the click. Read here rather than in the
    // coroutine below, which starts a turn later: a tap that leaves the
    // conversation in between would clear the bar, and this send would have no
    // owner to discard.
    val claimedDraftOwner =
        quickReplyDraftId
            .takeIf { quickReplyDraftSaved && it.isNotBlank() }
            ?.let { ComposeDraftOwner(accountId, it, threadId) }
    // Hold it from the click too. The allocation below is long enough to leave
    // the conversation and come back, and the draft rehydrated in between holds
    // the text being sent — indistinguishable, by the time the send settles,
    // from a newer reply the user has started.
    claimedDraftOwner?.let { quickReplyConsumedDraftIds += it.draftId.normalizedComposeDraftId() }
    // Whatever draft this send turns out to consume (the bar's, or one an
    // autosave hands over below), its server row can come back in a read
    // before its body does.
    quickReplyDraftThreadIds += threadId
    val account = coreAccounts.firstOrNull { it.id == accountId }
    val replyFrom = resolveQuickReplyFrom(parent, account)
    val baseParams =
        parent.toReplyMailParams(
            accountId = accountId,
            body = sentBody,
            from = replyFrom,
            attachments = sentAttachments,
        )
    // Empty the visible bar on the click, not when the send settles. The work
    // below — an autosave still holding the lock, then an identity round trip,
    // then the send itself — runs for seconds against a slow mailbox, and
    // leaving the text sitting there reads as a tap that did nothing.
    //
    // Only what the user can see is cleared. The draft id stays on the bar: a
    // failed send keeps that draft as its safety net, and edits made after the
    // failure have to write back into the same copy rather than open a second
    // one beside it. The success path below clears the id once there is nothing
    // left to fall back to.
    val barBodyAtClick = quickReplyBody
    val barSignatureAtClick = quickReplySignature
    quickReplyAttachments = emptyList()
    seedQuickReplySignature()
    // Render the sent bubble optimistically — on the click, before the send
    // round-trip — so replying feels instant. The bubble shows a "Sending…"
    // status until the canonical stored message replaces it on re-fetch; on
    // failure it flips to "Failed" and stays visible so the reply isn't lost.
    // A counter suffix keeps ids unique even for two sends in the same
    // millisecond — message ids key the conversation list, duplicates crash.
    val tempId = "local-send-${currentTimeMillis()}-${localSendSequence++}"
    messages =
        messages +
        MessageBody(
            id = tempId,
            folderId = parent.folderId,
            from = trs("chat.you"),
            fromAddr = replyFrom.ifBlank { account?.email.orEmpty() },
            to = baseParams.to,
            cc = baseParams.cc,
            subject = baseParams.subject,
            body = sentBody,
            references = baseParams.references,
            dateEpochSeconds = currentTimeMillis() / 1000,
            hasAttachments = sentAttachments.isNotEmpty(),
            attachments =
                sentAttachments.map {
                    MessageAttachment(filename = it.displayName, mimeType = it.mimeType, sizeBytes = it.sizeBytes)
                },
            sendStatus = SendStatus.Sending,
        )
    scope.launch {
        val pending =
            quickReplySaveMutex.withLock {
                // Settle who owns the draft this reply consumed *before* the
                // identity allocation below: that is a round trip, and until the
                // owner is resolved and held, reopening the conversation can
                // hydrate the very draft being sent, and a failed allocation can
                // leave a handover behind for some later send to act on.
                //
                // Any autosave from before the click has finished by now — it
                // held this same lock — so whatever it handed over is here.
                val resolvedOwner =
                    claimedDraftOwner
                        ?: quickReplyDraftId
                            .takeIf { quickReplyDraftSaved && it.isNotBlank() && quickReplyThreadId == threadId }
                            ?.let { ComposeDraftOwner(accountId, it, threadId) }
                        ?: quickReplySendDraftHandover
                            ?.takeIf { it.first == threadId }
                            ?.let { ComposeDraftOwner(accountId, it.second, threadId) }
                quickReplySendDraftHandover = null
                resolvedOwner?.let { quickReplyConsumedDraftIds += it.draftId.normalizedComposeDraftId() }
                val outboundMessageId =
                    runCatching {
                        withContext(ioDispatcher) { allocateCoreMessageId(MobileMailCommandClient(core), accountId, draft = false) }
                    }.getOrElse {
                        quickReplySendInFlight = false
                        quickReplySendThreadId = ""
                        resolvedOwner?.let { owner ->
                            quickReplyConsumedDraftIds -= owner.draftId.normalizedComposeDraftId()
                        }
                        // Nothing was sent, so take the optimistic bubble back
                        // down rather than leave a reply that does not exist.
                        messages = messages.filterNot { message -> message.id == tempId }
                        // No send was ever dispatched, so there is no pending
                        // reply for Retry to resend. Put the reply back where the
                        // user left it instead: the box they typed it into, ready
                        // to send again. Only if they have not started another.
                        if (quickReplyGeneration == generation) {
                            quickReplyBody = barBodyAtClick
                            quickReplySignature = barSignatureAtClick
                            quickReplyAttachments = sentAttachments
                        }
                        quickReplyFailure = it.message.orEmpty()
                        status = trsError("compose.toast.sendFailed", it)
                        return@launch
                    }
                // The quote goes out with the mail but stays out of the bar and
                // the bubble: the conversation already shows what it answers.
                val quote = replyQuoteFor(parent, accountId)
                val params =
                    baseParams
                        .copy(messageId = outboundMessageId)
                        .let { if (quote == null) it else it.withReplyQuote(quote.text, quote.html, quote.marker, quote.inlineAttachments) }
                // The bubble went up on the click; only its id was still unknown.
                messages =
                    messages.map { message ->
                        if (message.id == tempId) message.copy(messageId = outboundMessageId) else message
                    }
                PendingQuickReplySend(
                    accountId = accountId,
                    params = params,
                    tempMessageId = tempId,
                    threadId = threadId,
                    draftOwner = resolvedOwner,
                    quickReplyGeneration = generation,
                )
            }
        pendingQuickReplySend = pending
        status = trs("mobile.status.sendingReply")
        dispatchQuickReplySend(pending)
    }
}

internal fun MeronMobileState.retryQuickReplySend() {
    val pending = pendingQuickReplySend ?: return
    // The reply is resent from the params it was captured with, so it only has
    // to belong to the conversation on screen. Requiring the bar to be untouched
    // made a failed send unretryable the moment the user typed anything.
    if (selectedCoreThread?.backendThreadId() != pending.threadId) return
    retryQuickReplySend(pending)
}

internal fun MeronMobileState.retryQuickReplySend(pending: PendingQuickReplySend) {
    if (quickReplySendInFlight) return
    pendingQuickReplySend = pending
    quickReplySendInFlight = true
    // A retry is a send: it owns this conversation's draft for as long as it is
    // out, exactly as the first attempt did. Without this the guards that read
    // the pair see a blank thread and stand down for the whole retry.
    quickReplySendThreadId = pending.threadId
    quickReplyFailure = ""
    messages = messages.map { if (it.id == pending.tempMessageId) it.copy(sendStatus = SendStatus.Sending) else it }
    status = trs("mobile.status.sendingReply")
    scope.launch { dispatchQuickReplySend(pending) }
}

private suspend fun MeronMobileState.dispatchQuickReplySend(pending: PendingQuickReplySend) {
    // Claim the draft this reply consumed for as long as the send is settling,
    // so reopening the conversation before the discard returns cannot hydrate
    // the sent text back into the reply bar.
    val consumedDraftId =
        pending.draftOwner
            ?.draftId
            ?.normalizedComposeDraftId()
            .orEmpty()
    if (consumedDraftId.isNotBlank()) quickReplyConsumedDraftIds += consumedDraftId
    runCatching {
        withContext(ioDispatcher) {
            val client = MobileMailCommandClient(core)
            withManagedGoogleAuth(client, pending.accountId) { client.send(pending.params) }
        }
    }.onSuccess {
        val sameEditorGeneration = quickReplyGeneration == pending.quickReplyGeneration
        // The bar has carried on writing into this same draft — it moved on but
        // kept the id — so it holds the user's next reply now, not the text that
        // just went out, and the autosave following this send will write theirs
        // over it. That is the one case where the draft stays.
        val barKeptTheDraft =
            !sameEditorGeneration &&
                // An empty box is not "the user's next reply". Typing one and
                // clearing it again moves the generation on while leaving the id
                // claimed, and treating that as kept is what left the draft in
                // Drafts still holding the reply that already went out.
                !quickReplyIsBlank() &&
                quickReplyThreadId == pending.draftOwner?.threadId &&
                quickReplyDraftId.normalizedComposeDraftId() == pending.draftOwner?.draftId?.normalizedComposeDraftId()
        // Everything else — the bar untouched, or moved to another thread or
        // another draft — leaves nobody pointing at the consumed copy, and not
        // discarding it is what strands it in Drafts beside the sent reply.
        var consumedDraftDiscarded = false
        pending.draftOwner?.takeIf { !barKeptTheDraft }?.let { owner ->
            consumedDraftDiscarded =
                runCatching {
                    withContext(ioDispatcher) {
                        MobileMailCommandClient(core).discardDraft(
                            DiscardDraftParams(accountId = owner.accountId, draftId = owner.draftId),
                        )
                    }
                }.isSuccess
            if (consumedDraftDiscarded) {
                quickReplyDiscardedDraftIds += owner.draftId.normalizedComposeDraftId()
                removeDiscardedDraftFromOpenThread(owner.draftId, owner.threadId)
                // The bar can still be claiming the copy just deleted — an empty
                // box that was typed in and cleared keeps its id. Leaving the
                // claim would point the next save at a draft that is gone.
                if (quickReplyDraftId.normalizedComposeDraftId() == owner.draftId.normalizedComposeDraftId()) {
                    quickReplyDraftId = ""
                    quickReplyDraftSaved = false
                }
            }
        }
        if (pendingQuickReplySend == pending) pendingQuickReplySend = null
        if (pendingCertificateRetry == PendingCertificateRetry.QuickReply(pending)) pendingCertificateRetry = null
        quickReplySendInFlight = false
        quickReplySendThreadId = ""
        if (sameEditorGeneration) {
            quickReplyFailure = ""
            quickReplyAttachments = emptyList()
            quickReplyDraftId = ""
            quickReplyDraftSaved = false
            quickReplyInReplyTo = ""
            quickReplyReferences = ""
            ++quickReplyGeneration
            seedQuickReplySignature()
        }
        status = trs("mobile.status.replySent")
        val threadStillOpen = selectedCoreThread?.backendThreadId() == pending.threadId
        if (threadStillOpen) {
            messages = messages.map { if (it.id == pending.tempMessageId) it.copy(sendStatus = SendStatus.None) else it }
        }
        errorBanner = null
        // Shared with the reloads the send's own announcements ask for at the
        // same moment, instead of one each.
        scheduleVisibleMailboxReload()
        refreshKanbanColumnsHoldingThread(pending.threadId)
        if (threadStillOpen) {
            // Through the coalescer: the send's own announcements (Sent copy
            // cached, sync) ask for the same re-read at the same moment, and
            // one of those set off before the discard. This returns once a
            // re-read started after the discard has finished.
            runCatching { refreshOpenThreadCoalesced() }.onSuccess {
                // The read can race the server-side discard and return its stale
                // pre-discard row. The discard already succeeded, so reconcile
                // that row once more after applying the refreshed conversation.
                pending.draftOwner?.takeIf { !barKeptTheDraft && consumedDraftDiscarded }?.let { owner ->
                    removeDiscardedDraftFromOpenThread(owner.draftId, owner.threadId)
                }
            }
        }
        // Only now stop hiding it. Released any earlier, the refreshes above can
        // paint the server's pre-discard draft row beside the reply it was sent
        // as, leaving both cards on screen until the next read drops one.
        val undiscardedDraft = pending.draftOwner?.takeIf { !barKeptTheDraft && !consumedDraftDiscarded }
        if (undiscardedDraft != null) {
            // The reply went out, so the copy left behind is stale rather than
            // a safety net: released now, it could hydrate the sent text back
            // into the bar. It stays hidden until a retry removes it.
            retrySentDraftDiscard(undiscardedDraft)
        } else if (consumedDraftId.isNotBlank()) {
            quickReplyConsumedDraftIds -= consumedDraftId
        }
        runDeferredQuickReplyAutosave()
    }.onFailure {
        // A failed send leaves the draft as the safety net it was written to be.
        if (consumedDraftId.isNotBlank()) quickReplyConsumedDraftIds -= consumedDraftId
        quickReplySendInFlight = false
        quickReplySendThreadId = ""
        val message = it.message ?: trs("compose.toast.sendFailed")
        status = trsError("mobile.status.replyFailed", it)
        // The bubble belongs to this send, not to whoever holds the bar now.
        // Gating it on the editor left it reading "Sending…" for good once the
        // user started their next reply — the one state it must never show is a
        // send that is no longer happening.
        messages = messages.map { if (it.id == pending.tempMessageId) it.copy(sendStatus = SendStatus.Failed) else it }
        // Retry resends the params this send captured, so the conversation being
        // open is enough to offer it; the bar having moved on is not a reason to
        // hide the only way back to a reply that never went out.
        if (selectedCoreThread?.backendThreadId() == pending.threadId) {
            quickReplyFailure = message
        }
        if (untrustedCertificateProtocol(message) != null) {
            errorBanner = message
            pendingCertificateRetry = PendingCertificateRetry.QuickReply(pending)
        } else if (pendingCertificateRetry == PendingCertificateRetry.QuickReply(pending)) {
            pendingCertificateRetry = null
        }
        runDeferredQuickReplyAutosave()
    }
}

/**
 * Keep trying to discard the draft of a message that already went out, after the discard that followed
 * the send failed. Until one succeeds the draft stays among
 * [MeronMobileState.quickReplyConsumedDraftIds]: hidden from the conversation and never hydrated into
 * the reply bar, where it would put text that was already sent one tap from going out twice.
 */
internal fun MeronMobileState.retrySentDraftDiscard(owner: ComposeDraftOwner) {
    val id = owner.draftId.normalizedComposeDraftId()
    if (id.isBlank()) return
    quickReplyConsumedDraftIds += id
    scope.launch {
        for (delayMs in sentDraftDiscardRetryDelaysMs) {
            delay(delayMs)
            // Reopened in the full composer meanwhile: it is the user's again.
            if (composeDraftId.normalizedComposeDraftId() == id) {
                quickReplyConsumedDraftIds -= id
                return@launch
            }
            val discarded =
                runCatching {
                    withContext(ioDispatcher) {
                        MobileMailCommandClient(core).discardDraft(
                            DiscardDraftParams(accountId = owner.accountId, draftId = owner.draftId),
                        )
                    }
                }.isSuccess
            if (discarded) {
                quickReplyDiscardedDraftIds += id
                quickReplyConsumedDraftIds -= id
                removeDiscardedDraftFromOpenThread(owner.draftId, owner.threadId)
                return@launch
            }
        }
        // Still hidden, so it cannot come back into the reply bar this session;
        // the Drafts folder is where the user can remove it.
        status = trs("composer.status.couldNotDiscardDraft")
    }
}

private fun MeronMobileState.quickReplyEditorOwns(pending: PendingQuickReplySend): Boolean =
    selectedCoreThread?.backendThreadId() == pending.threadId &&
        quickReplyThreadId == pending.threadId &&
        quickReplyGeneration == pending.quickReplyGeneration
