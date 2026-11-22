// Owns the inline reply draft for a mobile
// conversation. Navigation and pending saves must
// preserve the text and identity belonging to that
// thread.

package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.size
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.ComposeDraft
import jp.nonbili.meron.shared.DiscardDraftParams
import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.ReplyRecipients
import jp.nonbili.meron.shared.SendIdentity
import jp.nonbili.meron.shared.SendStatus
import jp.nonbili.meron.shared.SignatureMark
import jp.nonbili.meron.shared.SignaturePlacement
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.accountSendIdentities
import jp.nonbili.meron.shared.bodyWithSignature
import jp.nonbili.meron.shared.bodyWithSwappedSignature
import jp.nonbili.meron.shared.buildReplyRecipients
import jp.nonbili.meron.shared.detectReplyFromIdentity
import jp.nonbili.meron.shared.folderIsDrafts
import jp.nonbili.meron.shared.folderIsInbox
import jp.nonbili.meron.shared.newDraftMessageId
import jp.nonbili.meron.shared.threadIdIsRss
import jp.nonbili.meron.shared.toReplyMailParams
import jp.nonbili.meron.shared.toSaveDraftParams
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

// The message a quick reply answers: the newest incoming message that isn't a
// draft, preferring one that accepts replies so a no-reply notification or a
// bounce doesn't take it (the core decides which, see `accepts_replies`). Keeps
// mail we sent out of recipient/threading derivation — see [sentByUs] for what
// counts as ours.
internal fun MeronMobileState.quickReplyParent(
    thread: ThreadSummary? = selectedCoreThread,
    messages: List<MessageBody> = this.messages,
): MessageBody? {
    val accountId = thread?.accountId?.ifBlank { defaultSendAccountId() }.orEmpty()
    val account = coreAccounts.firstOrNull { it.id == accountId }
    val ownAddresses =
        account
            ?.let(::accountSendIdentities)
            ?.map { it.email.trim().lowercase() }
            ?.filter { it.isNotBlank() }
            ?.toSet()
            .orEmpty()

    val received = messages.filter { !folderIsDrafts(it.folderId) && !it.sentByUs(ownAddresses) }
    return received.lastOrNull { it.reply?.acceptsReplies != false }
        ?: received.lastOrNull()
        ?: messages.lastOrNull { !folderIsDrafts(it.folderId) }
        ?: messages.lastOrNull()
}

// Whether a loaded message is one we sent, as opposed to one we received. The
// core settles this from the message's own delivery headers whenever it has the
// body cached. Until then `outgoing` — like the address check kept here for rows
// shaped before that flag existed — falls back to matching From against our
// identities, which also fires for a colleague's mail from a shared alias;
// sitting in the inbox vetoes that match. An optimistic send is ours whatever
// folder it claims.
private fun MessageBody.sentByUs(ownAddresses: Set<String>): Boolean {
    if (sendStatus != SendStatus.None) return true
    if (folderIsInbox(folderId)) return false
    return outgoing || ownAddresses.contains(fromAddr.trim().lowercase())
}

// Continue a thread with the configured identity used by its most recent
// outgoing message. Null means there is no such message; blank means primary.
private fun detectRecentThreadFrom(
    account: AccountSummary,
    messages: List<MessageBody>,
): String? {
    val identities = accountSendIdentities(account)
    val byEmail = identities.associateBy { it.email.trim().lowercase() }
    val ownAddresses = identities.map { it.email.trim().lowercase() }.filter { it.isNotBlank() }.toSet()
    for (message in messages.asReversed()) {
        if (folderIsDrafts(message.folderId)) continue
        if (!message.sentByUs(ownAddresses)) continue
        val identity = byEmail[message.fromAddr.trim().lowercase()] ?: continue
        return if (identity.email.equals(account.email, ignoreCase = true)) "" else identity.email
    }
    return null
}

// The address the quick reply sends from: the identity picked in the reply bar's
// From row, the identity used by the newest outgoing message, or the alias the
// inbound parent was delivered to. Blank means the account primary, which the
// send and draft paths read as "use the default".
internal fun MeronMobileState.resolveQuickReplyFrom(
    parent: MessageBody,
    account: AccountSummary?,
    pickedFrom: String = quickReplyFrom,
    messages: List<MessageBody> = this.messages,
): String {
    if (account == null) return ""
    if (pickedFrom.isNotBlank()) {
        return if (pickedFrom.equals(account.email, ignoreCase = true)) "" else pickedFrom
    }
    detectRecentThreadFrom(account, messages)?.let { return it }
    return detectReplyFromIdentity(parent, account)
}

// Identities the open thread's quick reply can send as. Empty when there is
// nothing to choose between (no thread, unknown account, or a single address) —
// the reply bar hides its From row rather than stating the obvious.
internal fun MeronMobileState.quickReplyIdentities(thread: ThreadSummary? = selectedCoreThread): List<SendIdentity> {
    if (thread == null) return emptyList()
    val accountId = thread.accountId.ifBlank { defaultSendAccountId() }
    val account = coreAccounts.firstOrNull { it.id == accountId } ?: return emptyList()
    val identities = accountSendIdentities(account)
    return if (identities.size < 2) emptyList() else identities
}

/** Backs the reply bar's recipient row: the To/Cc the bar would send to as it
 * stands. Both blank when the open thread cannot be replied to, which hides the
 * row. */
internal fun MeronMobileState.quickReplyRecipients(
    thread: ThreadSummary? = selectedCoreThread,
    messages: List<MessageBody> = this.messages,
): ReplyRecipients {
    val none = ReplyRecipients("", "")
    if (thread == null || threadIdIsRss(thread.id)) return none
    val parent = quickReplyParent(thread, messages) ?: return none
    return buildReplyRecipients(parent)
}

// The identity the reply bar's From row shows as current — the resolved send-as
// address matched back to the pickable list.
// A conversation shown beside the open one has no pick of its own yet, so it
// shows the identity its reply would start from.
internal fun MeronMobileState.selectedQuickReplyIdentity(
    thread: ThreadSummary? = selectedCoreThread,
    messages: List<MessageBody> = this.messages,
): SendIdentity? {
    val identities = quickReplyIdentities(thread)
    if (identities.isEmpty()) return null
    val account = coreAccounts.firstOrNull { it.id == identities.first().accountId } ?: return null
    val pickedFrom = if (thread?.id == selectedCoreThread?.id) quickReplyFrom else ""
    val parent = quickReplyParent(thread, messages)
    val email = (parent?.let { resolveQuickReplyFrom(it, account, pickedFrom, messages) } ?: pickedFrom).ifBlank { account.email }
    return identities.firstOrNull { it.email.equals(email, ignoreCase = true) } ?: identities.first()
}

/**
 * The account the open thread's quick reply sends from. Blank when there is no
 * thread, or none of the accounts can send.
 */
private fun MeronMobileState.quickReplyAccountId(thread: ThreadSummary? = selectedCoreThread): String {
    if (thread == null) return ""
    if (threadIdIsRss(thread.id)) return ""
    return thread.accountId.ifBlank { defaultSendAccountId() }
}

/** What [thread]'s reply bar would be seeded with on opening it: the
 *  replying account's signature, as [seedQuickReplySignature] puts there. */
internal fun MeronMobileState.seededQuickReplyBody(thread: ThreadSummary): String = bodyWithSignature("", quickReplyAccountId(thread).takeIf { it.isNotBlank() }?.let { signatureTextFor(it) }.orEmpty())

/**
 * Seed the quick reply body with the selected account's signature.
 *
 * Seed the reply bar with the replying account's signature, as the box the user starts typing into rather
 * than something stapled on at send time — the rule the full composer follows, and what every other mail
 * client shows.
 *
 * Replaces whatever the box holds, so it is only ever called on a fresh quick reply: a thread switch, or
 * the clear after a send or an escalation. The app signature is read back from the core asynchronously, so
 * a box seeded before it lands is seeded again once it does — unless the user has since typed.
 */

internal fun MeronMobileState.seedQuickReplySignature() {
    val threadId = quickReplyThreadId
    val signature = quickReplyAccountId().takeIf { it.isNotBlank() }?.let { signatureTextFor(it) }.orEmpty()
    quickReplyBody = bodyWithSignature("", signature)
    quickReplySignature = if (signature.isBlank()) null else SignatureMark(signature, SignaturePlacement.BelowText)
    if (appSignatureLoaded) return
    scope.launch {
        awaitAppSignatureLoaded()
        if (quickReplyThreadId == threadId && quickReplyDraftId.isBlank() && quickReplyIsBlank()) {
            seedQuickReplySignature()
        }
    }
}

/**
 * Re-seed a reply bar the user has not written in yet, so it isn't left holding a signature that is no longer the one it would send.
 *
 * Called whenever the app-wide or an account's signature is rewritten — the settings screen can be
 * opened and left with the thread still behind it, and the bar is what gets sent. A bar with anything
 * of theirs in it — text, an attachment, a hydrated draft — is never rewritten.
 */
internal fun MeronMobileState.reseedUntouchedQuickReply() {
    if (quickReplyThreadId.isBlank()) return
    if (quickReplyDraftSaved || quickReplyDraftId.isNotBlank()) return
    if (!quickReplyIsBlank()) return
    seedQuickReplySignature()
}

/**
 * The reply bar with the signature this app seeded taken back out, and whether
 * it was still there to take. A signature the user has typed into can no longer
 * be identified (see [bodyWithSwappedSignature]), which is the same answer as
 * "this text is theirs now".
 */
private fun MeronMobileState.quickReplyWithoutSignature(): Pair<String, Boolean> {
    val mark = quickReplySignature ?: return quickReplyBody to false
    if (mark.text.isBlank()) return quickReplyBody to false
    val swapped = bodyWithSwappedSignature(quickReplyBody, mark, "")
    return if (swapped.tracking == null) quickReplyBody to false else swapped.body to true
}

/**
 * Whether the reply bar holds nothing the user put there. A seeded signature
 * does not count as content: it is not something they wrote, and treating it as
 * such would save a draft for every thread they merely open, and let an
 * untouched box be "sent".
 */
internal fun MeronMobileState.quickReplyIsBlank(): Boolean = quickReplyWithoutSignature().first.isBlank() && quickReplyAttachments.isEmpty()

private suspend fun MeronMobileState.saveQuickReplyDraft(showStatus: Boolean): Boolean =
    quickReplySaveMutex.withLock {
        // Sending cancels this job, but the write it is already making lands on
        // the server regardless. Cancelling between the write and the fold below
        // is what strands the draft: runCatching swallows the CancellationException
        // as a failed save, so the id just written is never published to the bar
        // nor handed over, and the send that cancelled us finds no draft to
        // discard. Once the write is under way, see it through and record it.
        withContext(NonCancellable) { saveQuickReplyDraftLocked(showStatus) }
    }

private suspend fun MeronMobileState.saveQuickReplyDraftLocked(showStatus: Boolean): Boolean {
    // A send is about to discard the draft; saving now could resurrect it. The
    // reply the user has since started still has to reach the server though, so
    // record that a save was turned away and run it once the send settles.
    if (quickReplySendInFlight) {
        quickReplyAutosaveDeferred = true
        return false
    }
    val thread = selectedCoreThread
    val generation = quickReplyGeneration
    val accountId = thread?.accountId?.ifBlank { defaultSendAccountId() }.orEmpty()
    val parent = quickReplyParent()
    if (accountId.isBlank() || thread == null || parent == null) {
        if (showStatus) status = trs("mobile.status.openThreadBeforeSavingReply")
        return false
    }
    if (quickReplyIsBlank()) {
        if (showStatus) status = trs("mobile.status.nothingToSave")
        return false
    }
    val account = coreAccounts.firstOrNull { it.id == accountId }
    val replyFrom = resolveQuickReplyFrom(parent, account)
    val replyParams =
        parent.toReplyMailParams(
            accountId = accountId,
            body = quickReplyBody.trim(),
            from = replyFrom,
            attachments = quickReplyAttachments,
        )
    val draftId = quickReplyDraftId.ifBlank { newDraftMessageId(accountId) }
    quickReplyInReplyTo = replyParams.inReplyTo
    quickReplyReferences = replyParams.references
    val draft = ComposeDraft(replyParams.to, replyParams.cc, "", replyParams.subject, quickReplyBody.trim(), quickReplyAttachments)
    if (showStatus) status = trs("composer.status.savingDraft")
    return runCatching {
        withContext(ioDispatcher) {
            val client = MobileMailCommandClient(core)
            val resolvedDraftId =
                if (draftId.startsWith("local-draft-")) allocateCoreMessageId(client, accountId, draft = true) else draftId
            val params =
                draft
                    .toSaveDraftParams(
                        accountId = accountId,
                        draftId = resolvedDraftId,
                        from = replyFrom,
                    ).copy(
                        inReplyTo = replyParams.inReplyTo,
                        references = replyParams.references,
                    )
            withManagedGoogleAuth(client, accountId) { client.saveDraft(params) }
            resolvedDraftId
        }
    }.fold(
        onSuccess = { savedDraftId ->
            val threadId = thread.backendThreadId()
            // Its copy can now come back in a read before its body does.
            quickReplyDraftThreadIds += threadId
            val sameEditor = selectedCoreThread?.backendThreadId() == threadId && quickReplyThreadId == threadId
            val newlyAllocated = draftId.startsWith("local-draft-")
            if (sameEditor && newlyAllocated && quickReplyDraftId.isBlank()) {
                // Publish the allocated id even if the text changed or a send is
                // waiting on the save lock. The next save reuses it, and the
                // waiting send can capture it as the draft it must discard.
                quickReplyDraftId = savedDraftId
                quickReplyDraftSaved = true
                markThreadDraftEverywhere(threadId)
                true
            } else if (quickReplyGeneration != generation || !sameEditor || quickReplySendInFlight) {
                // The save still belongs to the editor that started it. Keep its
                // remote draft, but do not let its completion mutate another
                // thread or a newer version of this reply.
                //
                // A send is waiting on this save's lock and cannot read the id
                // off the bar — the bar has moved on, or never held it — so hand
                // it over directly. Without this the copy just written is the one
                // left in Drafts beside the reply that send is about to deliver.
                if (quickReplySendInFlight) quickReplySendDraftHandover = threadId to savedDraftId
                true
            } else {
                quickReplyDraftId = savedDraftId
                quickReplyDraftSaved = true
                markThreadDraftEverywhere(threadId)
                if (showStatus) status = trs("mobile.status.draftSaved")
                true
            }
        },
        onFailure = {
            if (showStatus) status = trsError("mobile.status.draftSaveFailed", it)
            false
        },
    )
}

// Hides the draft hydrated into quick reply wherever it sits in the loaded
// conversation. An optimistic sent bubble is appended after it, so tail-only
// matching would reveal the draft again while sending.
internal fun MeronMobileState.visibleThreadMessages(): List<MessageBody> {
    // Drafts a send has taken over are hidden too, until their discard comes
    // back: navigating away clears the bar's id, so reopening the conversation
    // mid-send would otherwise show the draft beside the reply it was sent as.
    val hidden = (quickReplyConsumedDraftIds + quickReplyDiscardedDraftIds).toMutableSet()
    if (quickReplyDraftId.isNotBlank()) hidden += quickReplyDraftId.normalizedComposeDraftId()
    // A read can find the bar's draft (saved by autosave, or being sent and
    // discarded) synced from its envelope alone: no body yet, so no Message-ID
    // to match it by, and it would paint as a "Draft … Loading…" bubble beside
    // the reply until the discard landed. In a conversation the bar has saved
    // into, such a row is taken for that draft; once its body arrives it is
    // recognised by id like any other, and shown if it is a different one.
    val openThreadId = selectedCoreThread?.backendThreadId().orEmpty()
    val hideUnidentifiedDrafts = openThreadId.isNotBlank() && openThreadId in quickReplyDraftThreadIds
    if (hidden.isEmpty() && !hideUnidentifiedDrafts) return messages
    return messages.filterNot {
        folderIsDrafts(it.folderId) &&
            (
                it.messageId.normalizedComposeDraftId() in hidden ||
                    (hideUnidentifiedDrafts && it.bodyMissing && it.messageId.isBlank())
            )
    }
}

internal fun MeronMobileState.autoSaveQuickReplyDraft() {
    scope.launch {
        saveQuickReplyDraft(showStatus = false)
    }
}

/**
 * Run a save that a send in flight turned away. The bar empties on the click, so the user can start
 * their next reply while the previous one is still going out — and that reply's debounce lands inside
 * the send, where saving is refused. Nothing else reschedules it: until it runs, the copy in Drafts
 * still holds the text that was already sent, and the reply on screen is not saved anywhere.
 */
internal fun MeronMobileState.runDeferredQuickReplyAutosave() {
    if (!quickReplyAutosaveDeferred) return
    quickReplyAutosaveDeferred = false
    if (quickReplyIsBlank()) return
    autoSaveQuickReplyDraft()
}

// Flushes any pending debounced autosave immediately — used when navigating
// away from the thread screen, mirroring closeCompose()'s autosave-on-close
// for the full composer, so the last few keystrokes aren't lost to the
// debounce window.
internal fun MeronMobileState.flushQuickReplyAutosave() {
    quickReplyAutosaveJob?.cancel()
    quickReplyAutosaveJob = null
    if (quickReplyIsBlank()) {
        discardQuickReplyDraftIfEmpty()
    } else {
        autoSaveQuickReplyDraft()
    }
}

internal fun MeronMobileState.discardQuickReplyDraftIfEmpty() {
    if (!quickReplyIsBlank()) return
    // A send out for this conversation is still holding this draft as the copy
    // it falls back on if it fails. Emptying a *newer* reply out of the bar is
    // not permission to delete the safety net of the one already on its way.
    if (quickReplySendInFlight && quickReplySendThreadId == quickReplyThreadId) return
    val draftId = quickReplyDraftId.takeIf { quickReplyDraftSaved } ?: return
    val thread = selectedCoreThread ?: return
    val accountId = thread.accountId.ifBlank { defaultSendAccountId() }
    quickReplyDraftId = ""
    quickReplyDraftSaved = false
    quickReplyInReplyTo = ""
    quickReplyReferences = ""
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                withManagedGoogleAuth(client, accountId) {
                    client.discardDraft(
                        DiscardDraftParams(accountId = accountId, draftId = draftId),
                    )
                }
            }
        }.onSuccess {
            val normalizedDraftId = draftId.normalizedComposeDraftId()
            val anotherDraftRemains =
                messages.any {
                    it.messageId.normalizedComposeDraftId() != normalizedDraftId && folderIsDrafts(it.folderId)
                }
            // Clearing only the open thread's copy leaves the thread in
            // locallyDraftedThreadIds, and the refresh below stamps the flag
            // straight back onto the row it just fetched — so the badge outlives
            // the draft everywhere the thread is listed.
            if (!anotherDraftRemains) clearThreadDraftEverywhere(thread.backendThreadId())
            syncCoreThreads(syncFirst = false)
            selectedCoreThread = selectedCoreThread?.copy(hasDraft = anotherDraftRemains)
        }
    }
}

internal fun MeronMobileState.onQuickReplyBodyChange(value: String) {
    quickReplyBody = value
    ++quickReplyGeneration
    quickReplyFailure = ""
    quickReplyAutosaveJob?.cancel()
    quickReplyAutosaveJob =
        scope.launch {
            delay(1200)
            if (quickReplyIsBlank()) {
                discardQuickReplyDraftIfEmpty()
            } else {
                saveQuickReplyDraft(showStatus = false)
            }
        }
}

/** Escalate the quick reply bar into the full composer, seeded as a reply and
 * carrying over whatever has been typed. `replyAll` seeds the wider recipient
 * list; the bar has no recipient fields to show it in, so reply-all always
 * lands here rather than in the bar. */
internal fun MeronMobileState.openQuickReplyInFullEditor(replyAll: Boolean = false) {
    val thread = selectedCoreThread
    val accountId = thread?.accountId?.ifBlank { defaultSendAccountId() }.orEmpty()
    val parent = quickReplyParent()
    if (accountId.isBlank() || thread == null || parent == null) {
        status = trs("mobile.status.openThreadBeforeReplying")
        return
    }
    if (threadIdIsRss(thread.id)) {
        status = trs("mobile.status.rssNoReplies")
        return
    }
    ++quickReplyGeneration
    val replyFrom = resolveQuickReplyFrom(parent, coreAccounts.firstOrNull { it.id == accountId })
    val generation = ++composeSessionGeneration
    val open: MeronMobileState.(ReplyQuote?) -> Unit = open@{ quote ->
        if (generation != composeSessionGeneration || selectedCoreThread?.id != thread.id) return@open
        // The bar as it stands now, not at the tap: loading the quote's images
        // can take a moment the user keeps typing through.
        //
        // The seeded signature is handed over stripped, so the full composer
        // inserts and tracks its own copy — the account is the same, so this is
        // the identical text, now swappable if the draft later changes identity.
        // When it can't be found the user has written into it: it stays in the
        // body as theirs, and the composer must not add a second.
        val (carriedBody, signatureStripped) = quickReplyWithoutSignature()
        val carriesEditedSignature = !signatureStripped && quickReplySignature?.text?.isNotBlank() == true
        val params =
            parent.toReplyMailParams(
                accountId = accountId,
                body = carriedBody.trim(),
                from = replyFrom,
                attachments = quickReplyAttachments,
                replyAll = replyAll,
            )
        composeSignaturePending = false
        ++composeIdentityGeneration
        to = params.to
        cc = params.cc
        bcc = params.bcc
        subject = params.subject
        // A quote goes under what was typed, with the signature between the two.
        // A bar holding a saved quoted reply already has both.
        val alreadyQuoted = quote?.inBody(params.body) == true
        body =
            if (alreadyQuoted || carriesEditedSignature) {
                composeSignature = null
                params.body
            } else {
                seedBodyWithSignature(params.body, accountId)
            }
        attachments = quickReplyAttachments
        // A quick reply is plain text; nothing carries over from an earlier
        // forward, only the quote of the message it answers: folded away, or
        // left where it is when the bar already holds it.
        applyReplyQuote(quote?.takeIf { alreadyQuoted })
        composeFoldedQuote = quote?.takeUnless { alreadyQuoted }
        composeFromAccountId = accountId
        composeFromEmail = replyFrom
        // Hand off any draft already saved for this quick reply so continuing in the
        // full editor keeps editing the same server-side draft instead of creating a
        // duplicate one.
        composeDraftId = quickReplyDraftId
        composeDraftSaved = quickReplyDraftSaved
        composeDraftAccountId = if (quickReplyDraftSaved) accountId else ""
        composeInReplyTo = params.inReplyTo
        composeReferences = params.references
        quickReplyAutosaveJob?.cancel()
        quickReplyAttachments = emptyList()
        quickReplyFailure = ""
        quickReplyDraftId = ""
        quickReplyDraftSaved = false
        quickReplyInReplyTo = ""
        quickReplyReferences = ""
        quickReplyFrom = ""
        // The thread is still behind the composer, so the bar the user comes
        // back to is a fresh quick reply — signature and all.
        seedQuickReplySignature()
        composeReturnScreen = Screen.Thread
        rememberComposeSeed()
        screen = Screen.Compose
        status = ""
    }
    if (appSignatureLoaded && !quoteReplies) {
        open(null)
    } else {
        scope.launch {
            awaitAppSignatureLoaded()
            open(replyQuoteFor(parent, accountId))
        }
    }
}
