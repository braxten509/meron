// Coordinates the mobile compose draft, its server
// identity and submission. The editor can change
// or close while a save or send is still in
// flight.

package jp.nonbili.meron.ui

import androidx.compose.material.icons.outlined.Drafts
import androidx.compose.ui.input.key.key
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.AllocateIdentityParams
import jp.nonbili.meron.shared.AttachmentReadParams
import jp.nonbili.meron.shared.ComposeDraft
import jp.nonbili.meron.shared.ContactSuggestParams
import jp.nonbili.meron.shared.ContactSuggestion
import jp.nonbili.meron.shared.FORWARD_QUOTE_MARKER
import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.SendIdentity
import jp.nonbili.meron.shared.SignatureMark
import jp.nonbili.meron.shared.SignaturePlacement
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.accountSendIdentities
import jp.nonbili.meron.shared.accountSummaryIsRss
import jp.nonbili.meron.shared.attachmentToDraftAttachment
import jp.nonbili.meron.shared.bodyWithSignature
import jp.nonbili.meron.shared.bodyWithSwappedSignature
import jp.nonbili.meron.shared.forwardHtmlForSend
import jp.nonbili.meron.shared.forwardableAttachments
import jp.nonbili.meron.shared.inlineImageToDraftAttachment
import jp.nonbili.meron.shared.messageEditAsNewDraft
import jp.nonbili.meron.shared.messageForwardDraft
import jp.nonbili.meron.shared.newDraftMessageId
import jp.nonbili.meron.shared.noSignatureMark
import jp.nonbili.meron.shared.parseAllocatedMessageId
import jp.nonbili.meron.shared.parseAttachmentDataResponse
import jp.nonbili.meron.shared.parseContactSuggestResponse
import jp.nonbili.meron.shared.plainTextToHtml
import jp.nonbili.meron.shared.recipientTail
import jp.nonbili.meron.shared.replaceRecipientTail
import jp.nonbili.meron.shared.resolveSignatureHtml
import jp.nonbili.meron.shared.signaturePlainText
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

// Only touched from the main thread (all send paths run in the UI scope).
internal var localSendSequence = 0L

internal fun MeronMobileState.defaultSendAccountId(): String =
    selectedCoreAccountId.takeIf { selected ->
        selected != UNIFIED_ACCOUNT_ID && coreAccounts.any { it.id == selected && !accountSummaryIsRss(it) }
    } ?: coreAccounts.firstOrNull { !accountSummaryIsRss(it) }?.id.orEmpty()

internal fun MeronMobileState.composeIdentityCandidates(): List<SendIdentity> =
    coreAccounts
        .filter { !accountSummaryIsRss(it) && !it.needsReconnect }
        .flatMap { accountSendIdentities(it) }

internal fun MeronMobileState.selectedComposeIdentity(): SendIdentity? {
    val candidates = composeIdentityCandidates()
    return candidates.firstOrNull { it.accountId == composeFromAccountId && it.email == composeFromEmail }
        ?: candidates.firstOrNull { it.accountId == composeFromAccountId }
        ?: candidates.firstOrNull { it.accountId == defaultSendAccountId() }
        ?: candidates.firstOrNull()
}

/**
 * The signature for a draft sent from [accountId], already converted to the
 * plain text the composer edits. Blank when nothing is configured.
 */
internal fun MeronMobileState.signatureTextFor(accountId: String): String {
    val account = coreAccounts.firstOrNull { it.id == accountId }
    return signaturePlainText(resolveSignatureHtml(account, appSignatureHtml))
}

/**
 * Seed a fresh draft body with the sending account's signature, remembering what
 * was inserted so a later change of identity can swap it out.
 */
internal fun MeronMobileState.seedBodyWithSignature(
    body: String,
    accountId: String,
    placement: SignaturePlacement = SignaturePlacement.BelowText,
): String {
    val signature = signatureTextFor(accountId)
    // The placement is recorded even when the account sends no signature, so a
    // forward that later moves to an account with one still puts it above the
    // quote rather than after it.
    composeSignature = if (signature.isBlank()) noSignatureMark(placement) else SignatureMark(signature, placement)
    return bodyWithSignature(body, signature, placement)
}

/**
 * Move the draft to another send identity, swapping the signature it carries for
 * the new account's. Sending account B's mail under account A's signature is
 * worse than no signature at all; an edited signature is left alone.
 */
internal fun MeronMobileState.changeComposeIdentity(
    accountId: String,
    email: String,
) {
    if (accountId != composeDraftAccountId && composeDraftSaved && composeDraftAccountId.isNotBlank() && composeDraftId.isNotBlank()) {
        val owner = ComposeDraftOwner(composeDraftAccountId, composeDraftId, composeDraftThreadId())
        if (owner !in composeDraftCleanupOwners) composeDraftCleanupOwners = composeDraftCleanupOwners + owner
        composeDraftId = newDraftMessageId(accountId)
        composeDraftSaved = false
        composeDraftAccountId = ""
    }
    composeFromAccountId = accountId
    composeFromEmail = email
    val sessionGeneration = composeSessionGeneration
    val identityGeneration = ++composeIdentityGeneration
    if (!appSignatureLoaded) {
        composeSignaturePending = true
        scope.launch {
            awaitAppSignatureLoaded()
            if (sessionGeneration == composeSessionGeneration && identityGeneration == composeIdentityGeneration) {
                applyComposeIdentitySignature(accountId)
                composeSignaturePending = false
            }
        }
        return
    }
    applyComposeIdentitySignature(accountId)
    composeSignaturePending = false
}

private fun MeronMobileState.applyComposeIdentitySignature(accountId: String) {
    val swapped = bodyWithSwappedSignature(body, composeSignature, signatureTextFor(accountId))
    body = swapped.body
    composeSignature = swapped.tracking
}

internal fun MeronMobileState.clearComposeDraftState() {
    attachments = emptyList()
    composeSignature = null
    to = ""
    cc = ""
    bcc = ""
    subject = ""
    body = ""
    composeFromAccountId = ""
    composeFromEmail = ""
    composeSignaturePending = false
    ++composeIdentityGeneration
    composeDraftId = ""
    composeDraftSaved = false
    composeDraftAccountId = ""
    composeInReplyTo = ""
    composeReferences = ""
    composeForwardHtml = ""
    composeForwardInlineAttachments = emptyList()
    composeQuoteMarker = FORWARD_QUOTE_MARKER
    composeReplyQuote = ""
    composeFoldedQuote = null
    composeSeed = ComposeSeed()
    recipientSuggestionField = ""
    recipientSuggestions = emptyList()
}

/**
 * The composer with the signature this app seeded taken back out, and a reply's seeded quote with it
 * while the quote is untouched. A signature the user has typed into can no longer be identified (see
 * [bodyWithSwappedSignature]), which is the same answer as "this text is theirs now". Mirrors the
 * reply bar's [quickReplyIsBlank].
 */
private fun MeronMobileState.composeBodyWithoutSignature(): String {
    val quoteAt = if (composeReplyQuote.isBlank()) -1 else body.lastIndexOf(composeReplyQuote)
    val written = if (quoteAt < 0) body else body.removeRange(quoteAt, quoteAt + composeReplyQuote.length)
    val mark = composeSignature ?: return written
    if (mark.text.isBlank()) return written
    val swapped = bodyWithSwappedSignature(written, mark, "")
    return if (swapped.tracking == null) written else swapped.body
}

/**
 * Whether the composer holds nothing the user put there. A seeded signature does
 * not count as content: it is not something they wrote, and saving on it leaves
 * an empty draft behind for every composer merely opened and closed again.
 */
internal fun MeronMobileState.composeIsBlank(): Boolean = composeBodyWithoutSignature().isBlank() && currentComposeSeed() == composeSeed

/** Records what a freshly opened composer holds, so [composeIsBlank] can tell it apart from the user's own writing. */
internal fun MeronMobileState.rememberComposeSeed() {
    composeSeed = currentComposeSeed()
}

private fun MeronMobileState.currentComposeSeed(): ComposeSeed =
    ComposeSeed(
        to = recipientEntries(to),
        cc = recipientEntries(cc),
        bcc = recipientEntries(bcc),
        subject = subject.trim(),
        attachments = attachments,
    )

private fun recipientEntries(value: String): List<String> {
    val (completed, active) = parseRecipients(value)
    return (completed + active).map { it.trim() }.filter { it.isNotEmpty() }
}

// The draft as the core should receive it: the plain body the composer edits,
// plus — for a forward or a quoted reply — the rebuilt HTML alternative and the
// inline images it references. A reply whose quote the user has edited goes out
// plain: the cached HTML quote would no longer say what the plain one does.
internal fun MeronMobileState.currentComposeDraft(): ComposeDraft {
    composeFoldedQuote?.let { quote ->
        // A folded quote goes out under whatever the body holds.
        val own = body.trim()
        val html = if (quote.html.isBlank()) "" else plainTextToHtml(own) + quote.html
        return ComposeDraft(
            to = to.trim(),
            cc = cc.trim(),
            bcc = bcc.trim(),
            subject = subject.trim(),
            body = if (own.isBlank()) quote.text.trim() else own + quote.text,
            attachments = if (html.isBlank()) attachments else attachments + quote.inlineAttachments,
            html = html,
        )
    }
    val quoteIntact = composeReplyQuote.isBlank() || body.trim().endsWith(composeReplyQuote)
    val html = if (quoteIntact) forwardHtmlForSend(body.trim(), composeForwardHtml, composeQuoteMarker) else ""
    return ComposeDraft(
        to = to.trim(),
        cc = cc.trim(),
        bcc = bcc.trim(),
        subject = subject.trim(),
        body = body.trim(),
        // The inline images exist only to back the quote's cid: refs. If the
        // quote is gone the HTML is empty, and attaching them would ship files
        // nothing references.
        attachments = if (html.isBlank()) attachments else attachments + composeForwardInlineAttachments,
        html = html,
    )
}

internal fun MeronMobileState.loadRecipientSuggestions(
    field: String,
    value: String,
) {
    val accountId = defaultSendAccountId()
    if (accountId.isBlank() || !coreLoaded) {
        recipientSuggestions = emptyList()
        recipientSuggestionField = field
        return
    }
    recipientSuggestionField = field
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                client.suggestContacts(
                    ContactSuggestParams(
                        accountId = accountId,
                        query = recipientTail(value),
                        limit = 6,
                    ),
                )
            }
        }.onSuccess {
            if (recipientSuggestionField == field) {
                recipientSuggestions = parseContactSuggestResponse(it)
            }
        }.onFailure {
            if (recipientSuggestionField == field) {
                recipientSuggestions = emptyList()
            }
        }
    }
}

internal fun MeronMobileState.acceptRecipientSuggestion(
    field: String,
    contact: ContactSuggestion,
) {
    when (field) {
        "to" -> to = replaceRecipientTail(to, contact)
        "cc" -> cc = replaceRecipientTail(cc, contact)
        "bcc" -> bcc = replaceRecipientTail(bcc, contact)
    }
    recipientSuggestions = emptyList()
}

internal fun String.normalizedComposeDraftId(): String = trim().trim('<', '>').lowercase()

internal suspend fun allocateCoreMessageId(
    client: MobileMailCommandClient,
    accountId: String,
    draft: Boolean,
): String {
    val id = parseAllocatedMessageId(client.allocateIdentity(AllocateIdentityParams(accountId, draft)))
    require(id.isNotBlank()) { "Core did not allocate a message identity" }
    return id
}

internal suspend fun MeronMobileState.readAttachmentData(
    client: MobileMailCommandClient,
    accountId: String,
    key: String,
): String {
    val response =
        withManagedGoogleAuth(client, accountId) {
            client.readAttachment(AttachmentReadParams(key))
        }
    return parseAttachmentDataResponse(response)
}

internal fun MeronMobileState.openMessageCompose(
    message: MessageBody,
    forward: Boolean,
) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val generation = ++composeSessionGeneration
    scope.launch {
        awaitAppSignatureLoaded()
        if (generation != composeSessionGeneration) return@launch
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                val accountId = selectedCoreThread?.accountId.orEmpty()
                val copiedAttachments =
                    forwardableAttachments(message).mapNotNull { attachment ->
                        readAttachmentData(client, accountId, attachment.key).takeIf { it.isNotBlank() }?.let {
                            attachmentToDraftAttachment(attachment, it)
                        }
                    }
                // Only a forward quotes the original's HTML, so only a forward
                // needs its inline images re-attached.
                val inlineImages = if (forward) readQuoteInlineImages(client, accountId, message) else emptyList()
                val draft =
                    if (forward) {
                        messageForwardDraft(
                            message = message,
                            attachments = copiedAttachments,
                            dateLabel = formatMessageFullTimestamp(message.dateEpochSeconds),
                            inlineImages = inlineImages.map { it.first },
                        )
                    } else {
                        messageEditAsNewDraft(message, copiedAttachments)
                    }
                draft to inlineImages.map { (image, data) -> inlineImageToDraftAttachment(image, data) }
            }
        }.onSuccess { (draft, inlineAttachments) ->
            if (generation != composeSessionGeneration) return@onSuccess
            // A forward and a copy are both new conversations: everything the
            // previous draft left behind goes, threading headers included, or
            // they would thread themselves under the last reply's parent.
            clearComposeDraftState()
            to = draft.to
            cc = draft.cc
            bcc = draft.bcc
            subject = draft.subject
            // A forward's body is the quote, so the signature goes above it. A
            // copied message ("edit as new") already carries the signature it
            // was written with, and must not collect a second one.
            body =
                if (forward) {
                    seedBodyWithSignature(
                        draft.body,
                        selectedCoreThread?.accountId.orEmpty().ifBlank { defaultSendAccountId() },
                        SignaturePlacement.AboveQuote,
                    )
                } else {
                    composeSignature = null
                    draft.body
                }
            attachments = draft.attachments
            composeForwardHtml = draft.html
            composeForwardInlineAttachments = inlineAttachments
            composeFromAccountId = selectedCoreThread?.accountId ?: selectedCoreAccountId.takeIf { it != UNIFIED_ACCOUNT_ID }.orEmpty()
            composeFromEmail = ""
            composeDraftId = ""
            composeDraftSaved = false
            composeDraftAccountId = ""
            composeReturnScreen = Screen.Thread
            rememberComposeSeed()
            screen = Screen.Compose
            status = trs(if (forward) "mobile.status.forwardDraftReady" else "mobile.status.copiedIntoCompose")
        }.onFailure {
            if (generation != composeSessionGeneration) return@onFailure
            status = trsError(if (forward) "mobile.status.forwardFailed" else "mobile.status.editAsNewFailed", it)
        }
    }
}

/**
 * Open a compose screen addressed to one person (the "message this participant"
 * action in a thread), signature included like any other new message.
 */
internal fun MeronMobileState.openComposeTo(
    email: String,
    accountId: String,
    returnScreen: Screen = Screen.Thread,
) {
    openSignatureCompose {
        clearComposeDraftState()
        composeFromAccountId = accountId
        composeFromEmail = ""
        to = email
        body = seedBodyWithSignature("", accountId.ifBlank { defaultSendAccountId() })
        composeReturnScreen = returnScreen
        rememberComposeSeed()
        screen = Screen.Compose
    }
}

/**
 * Open a compose screen from a `mailto:` link. The link's own body counts as
 * text the user asked for, so the signature goes below it.
 */
internal fun MeronMobileState.openMailtoCompose(draft: ComposeDraft) {
    openMailtoCompose(draft, onOpened = {})
}

internal fun MeronMobileState.openMailtoCompose(
    draft: ComposeDraft,
    onOpened: () -> Unit,
) {
    openSignatureCompose {
        clearComposeDraftState()
        to = draft.to
        cc = draft.cc
        bcc = draft.bcc
        subject = draft.subject
        attachments = draft.attachments
        body = seedBodyWithSignature(draft.body, defaultSendAccountId())
        composeReturnScreen = if (screen == Screen.Kanban) screen else Screen.Mail
        rememberComposeSeed()
        screen = Screen.Compose
        onOpened()
    }
}

internal fun MeronMobileState.openCompose() {
    // Everything the previous draft left behind goes, the sender and threading
    // headers included: a fresh message that kept `composeInReplyTo` would
    // thread itself into the conversation the last reply belonged to, and one
    // that kept `composeFromAccountId` would send from an account other than
    // the one whose signature it is about to be seeded with.
    openSignatureCompose {
        clearComposeDraftState()
        body = seedBodyWithSignature("", defaultSendAccountId())
        composeReturnScreen = if (screen == Screen.Kanban) screen else Screen.Mail
        rememberComposeSeed()
        screen = Screen.Compose
    }
}

private fun MeronMobileState.openSignatureCompose(open: MeronMobileState.() -> Unit) {
    val generation = ++composeSessionGeneration
    if (appSignatureLoaded) {
        if (generation == composeSessionGeneration) open()
        return
    }
    scope.launch {
        awaitAppSignatureLoaded()
        if (generation == composeSessionGeneration) open()
    }
}

internal fun MeronMobileState.closeCompose() {
    val generation = composeSessionGeneration
    val returnScreen = composeReturnScreen
    showLocalDraftInOpenThread()
    screen = returnScreen
    // Start immediately so the closing session is snapshotted before another
    // compose can open. A later session suppresses state updates but keeps the
    // remote copy written for the composer that was just closed.
    scope.launch(start = CoroutineStart.UNDISPATCHED) {
        saveComposeDraft(showStatus = false, generation = generation, keepObsoleteDraft = true)
    }
}

private fun MeronMobileState.showLocalDraftInOpenThread() {
    val thread = selectedCoreThread ?: return
    if (composeReturnScreen != Screen.Thread) return
    // As the draft was saved, folded quote and all, and answering the same
    // parent: reopening it folds the quote away again (see openDraftCompose).
    val draft = currentComposeDraft()
    if (composeIsBlank()) return
    val accountId = selectedComposeIdentity()?.accountId ?: thread.accountId.ifBlank { defaultSendAccountId() }
    if (accountId.isBlank()) return
    val draftId = composeDraftId.ifBlank { newDraftMessageId(accountId) }
    composeDraftId = draftId
    val normalizedDraftId = draftId.trim().trim('<', '>').lowercase()
    val localDraft =
        MessageBody(
            id = "local-draft-$normalizedDraftId",
            folderId = "Drafts",
            from = selectedComposeIdentity()?.email.orEmpty(),
            to = draft.to,
            cc = draft.cc,
            bcc = draft.bcc,
            subject = draft.subject,
            body = draft.body,
            dateEpochSeconds = currentTimeMillis() / 1000,
            fromAddr = selectedComposeIdentity()?.email.orEmpty(),
            messageId = draftId,
            inReplyTo = composeInReplyTo,
            references = composeReferences,
            outgoing = true,
            hasAttachments = attachments.isNotEmpty(),
        )
    messages =
        (
            messages.filterNot {
                it.id == localDraft.id || it.messageId
                    .trim()
                    .trim('<', '>')
                    .lowercase() == normalizedDraftId
            } + localDraft
        ).sortedBy { it.dateEpochSeconds }
    selectedCoreThread = thread.copy(hasDraft = true)
    markThreadDraftEverywhere(thread.id)
}

internal fun MeronMobileState.markThreadDraftEverywhere(threadId: String) {
    if (threadId.isBlank()) return
    val canonicalId =
        (listOfNotNull(selectedCoreThread).asSequence() + coreThreads.asSequence())
            .firstOrNull { it.id == threadId || it.threadId == threadId }
            ?.backendThreadId()
            ?: threadId
    locallyDraftedThreadIds = locallyDraftedThreadIds + canonicalId
    coreThreads = threadsWithDraftFlag(coreThreads, setOf(canonicalId))
    selectedCoreThread =
        selectedCoreThread?.let { thread ->
            if (thread.id == threadId || thread.threadId == threadId) thread.copy(hasDraft = true) else thread
        }
    kanbanColumns =
        kanbanColumns.mapValues { (_, state) ->
            state.copy(threads = threadsWithDraftFlag(state.threads, setOf(threadId)))
        }
    mailboxCache =
        mailboxCache.mapValues { (_, cached) ->
            cached.copy(threads = threadsWithDraftFlag(cached.threads, setOf(threadId)))
        }
}

internal fun MeronMobileState.clearThreadDraftEverywhere(threadId: String) {
    if (threadId.isBlank()) return
    val matchingIds =
        buildSet {
            add(threadId)
            (
                listOfNotNull(selectedCoreThread).asSequence() +
                    coreThreads.asSequence() +
                    kanbanColumns.values.asSequence().flatMap { it.threads.asSequence() } +
                    mailboxCache.values.asSequence().flatMap { it.threads.asSequence() }
            ).filter { it.id == threadId || it.threadId == threadId }
                .forEach {
                    add(it.id)
                    if (it.threadId.isNotBlank()) add(it.threadId)
                }
        }
    locallyDraftedThreadIds = locallyDraftedThreadIds - matchingIds
    coreThreads = threadsWithoutDraftFlag(coreThreads, threadId)
    selectedCoreThread =
        selectedCoreThread?.let { thread ->
            if (thread.id == threadId || thread.threadId == threadId) thread.copy(hasDraft = false) else thread
        }
    kanbanColumns =
        kanbanColumns.mapValues { (_, state) ->
            state.copy(threads = threadsWithoutDraftFlag(state.threads, threadId))
        }
    mailboxCache =
        mailboxCache.mapValues { (_, cached) ->
            cached.copy(threads = threadsWithoutDraftFlag(cached.threads, threadId))
        }
}

private fun threadsWithoutDraftFlag(
    threads: List<ThreadSummary>,
    threadId: String,
): List<ThreadSummary> =
    threads.map { thread ->
        if (thread.id == threadId || thread.threadId == threadId) thread.copy(hasDraft = false) else thread
    }

internal fun MeronMobileState.withLocalDraftFlags(threads: List<ThreadSummary>): List<ThreadSummary> = threadsWithDraftFlag(threads, locallyDraftedThreadIds)

// A just-discarded draft can briefly reappear in a server refetch: some IMAP
// providers (Gmail included) don't guarantee an expunge on one connection is
// visible to a concurrent read session immediately. Keep hiding a discarded
// thread until a fetch actually confirms it's gone, then stop tracking it.
internal fun MeronMobileState.withoutLocallyDiscardedThreads(threads: List<ThreadSummary>): List<ThreadSummary> {
    if (locallyDiscardedThreadIds.isEmpty()) return threads
    locallyDiscardedThreadIds =
        locallyDiscardedThreadIds.filter { id -> threads.any { it.id == id } }.toSet()
    if (locallyDiscardedThreadIds.isEmpty()) return threads
    return threads.filterNot { it.id in locallyDiscardedThreadIds }
}

internal fun threadsWithDraftFlag(
    threads: List<ThreadSummary>,
    threadId: String,
): List<ThreadSummary> = threadsWithDraftFlag(threads, setOf(threadId))

internal fun threadsWithDraftFlag(
    threads: List<ThreadSummary>,
    threadIds: Set<String>,
): List<ThreadSummary> {
    if (threadIds.isEmpty()) return threads
    var changed = false
    val updated =
        threads.map { thread ->
            if (thread.id !in threadIds && thread.threadId !in threadIds) {
                thread
            } else if (thread.hasDraft) {
                thread
            } else {
                changed = true
                thread.copy(hasDraft = true)
            }
        }
    return if (changed) updated else threads
}

// Let the user fix an account's credentials. OAuth accounts go straight to
// the provider's sign-in for that address — the setup screen would only offer
// the same button with nothing saying which account it is for. Password
// accounts re-open account setup pre-filled, to re-enter the password (the
// IMAP/SMTP host fields keep their last values).
internal fun MeronMobileState.reconnectAccount(account: AccountSummary) {
    val isOAuth = account.authType == "oauth" || account.provider == "gmail" || account.provider == "outlook"
    when {
        accountSummaryIsRss(account) -> {
            addSection = 2
            passwordServerSettingsOpen = false
        }

        isOAuth -> {
            oauthEmail = account.email
            if (account.provider == "gmail" || account.provider == "outlook") oauthProvider = account.provider
            oauthAuthorizationCode = ""
            addSection = 0
            passwordServerSettingsOpen = false
            errorBanner = null
            if (oauthProvider == "gmail") connectGoogleDeviceAccount() else launchOAuthFlow()
            return
        }

        else -> {
            resetPasswordAccountForm()
            displayName = account.displayName
            senderName = account.senderName
            email = account.email
            username = account.email
            password = ""
            // The prefilled address is already the account's own; don't let the
            // on-blur lookup re-run and rewrite the form around it.
            lastAutodiscoverEmail = account.email
            if (account.imapHost.isNotBlank()) host = account.imapHost
            hostTouched = host.isNotBlank()
            if (account.imapPort > 0) imapPort = account.imapPort.toString()
            imapPortTouched = account.imapPort > 0
            imapSecurity =
                when {
                    account.starttls -> MailSecurity.STARTTLS
                    account.tls -> MailSecurity.TLS
                    else -> MailSecurity.NONE
                }
            imapSecurityTouched =
                account.imapPort > 0 && imapSecurity != mailSecurityForPort(account.imapPort)
            if (account.smtpHost.isNotBlank()) smtpHost = account.smtpHost
            smtpHostTouched = smtpHost.isNotBlank()
            if (account.smtpPort > 0) smtpPort = account.smtpPort.toString()
            smtpPortTouched = account.smtpPort > 0
            smtpSecurity =
                when {
                    account.smtpStarttls -> MailSecurity.STARTTLS
                    account.smtpTls -> MailSecurity.TLS
                    else -> MailSecurity.NONE
                }
            smtpSecurityTouched =
                account.smtpPort > 0 && smtpSecurity != mailSecurityForPort(account.smtpPort)
            addSection = 1
            passwordServerSettingsOpen = true
        }
    }
    errorBanner = null
    previousTopScreen = if (screen == Screen.Kanban) screen else Screen.Mail
    screen = Screen.AddAccount
}
