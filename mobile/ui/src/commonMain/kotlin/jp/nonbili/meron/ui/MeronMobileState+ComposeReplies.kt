// Builds a reply from the selected conversation or one of its messages.
// Sender identity and recipient lists determine Reply and Reply All behavior.
// The composer restores quoted text and inline images from the source message.

package jp.nonbili.meron.ui

import androidx.compose.ui.input.key.key
import jp.nonbili.meron.shared.DraftAttachment
import jp.nonbili.meron.shared.FORWARD_QUOTE_MARKER
import jp.nonbili.meron.shared.ForwardInlineImage
import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.detectReplyFromIdentity
import jp.nonbili.meron.shared.forwardInlineImages
import jp.nonbili.meron.shared.hasReplyQuote
import jp.nonbili.meron.shared.inlineImageToDraftAttachment
import jp.nonbili.meron.shared.replyAllAddsRecipients
import jp.nonbili.meron.shared.replyQuoteAttribution
import jp.nonbili.meron.shared.replyQuotedHtmlBody
import jp.nonbili.meron.shared.replyQuotedPlainBody
import jp.nonbili.meron.shared.threadIdIsRss
import jp.nonbili.meron.shared.toReplyMailParams
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Whether the open conversation's reply target has other recipients, so
 * reply-all would reach someone the reply bar does not. */
internal fun MeronMobileState.canReplyAllToThread(
    thread: ThreadSummary? = selectedCoreThread,
    messages: List<MessageBody> = this.messages,
): Boolean {
    if (thread == null || threadIdIsRss(thread.id)) return false
    val parent = quickReplyParent(thread, messages) ?: return false
    return replyAllAddsRecipients(parent)
}

/** The same question for one message, for its own menu. */
internal fun MeronMobileState.canReplyAllToMessage(message: MessageBody): Boolean = replyAllAddsRecipients(message)

internal fun MeronMobileState.replyAllToMessage(message: MessageBody) = replyToMessage(message, replyAll = true)

/**
 * Reply (or reply-all) to one message, rather than to the conversation's reply target: the message
 * menu and the reader act on the message they belong to. Opens the full composer — for reply-all the
 * recipients are the point of the action, and only the composer shows them. The reply bar and any
 * draft it holds are left untouched.
 */
internal fun MeronMobileState.replyToMessage(
    message: MessageBody,
    replyAll: Boolean,
) {
    val thread = selectedCoreThread
    val accountId = thread?.accountId?.ifBlank { defaultSendAccountId() }.orEmpty()
    if (accountId.isBlank() || thread == null) {
        status = trs("mobile.status.openThreadBeforeReplying")
        return
    }
    if (threadIdIsRss(thread.id)) {
        status = trs("mobile.status.rssNoReplies")
        return
    }
    val account = coreAccounts.firstOrNull { it.id == accountId }
    // The bar's From override belongs to the bar's own reply; this one sends
    // from whichever identity this message was addressed to.
    val replyFrom = account?.let { detectReplyFromIdentity(message, it) }.orEmpty()
    val params =
        message.toReplyMailParams(
            accountId = accountId,
            body = "",
            from = replyFrom,
            replyAll = replyAll,
        )
    val generation = ++composeSessionGeneration
    val open: MeronMobileState.(ReplyQuote?) -> Unit = open@{ quote ->
        if (generation != composeSessionGeneration) return@open
        // A reply of its own, not a continuation of whatever the composer last
        // held: threading headers and leftover forward state go first.
        clearComposeDraftState()
        to = params.to
        cc = params.cc
        bcc = params.bcc
        subject = params.subject
        body = seedBodyWithSignature(params.body, accountId)
        composeFoldedQuote = quote
        composeFromAccountId = accountId
        composeFromEmail = replyFrom
        composeInReplyTo = params.inReplyTo
        composeReferences = params.references
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
            open(replyQuoteFor(message, accountId))
        }
    }
}

/**
 * The original quoted under a reply (see `settings.composer.quoteReplies`): the
 * plain text the composer edits, the HTML alternative with the inline images it
 * references (both empty for a text-only original), and the attribution line
 * that marks where the quote starts.
 */
internal data class ReplyQuote(
    val text: String,
    val html: String,
    val marker: String,
    val inlineAttachments: List<DraftAttachment>,
)

/** Whether [body] already ends in this quote, untouched, as a saved quoted
 *  reply does until its quote is edited. */
internal fun ReplyQuote.endsBody(body: String): Boolean = body.trim().endsWith(text.trim())

/** Whether [body] already quotes the message, edited or not: its attribution
 *  line is still there. Such a body is the user's as it stands. */
internal fun ReplyQuote.inBody(body: String): Boolean = body.contains(marker)

/** The quote for a reply to [message], or null when the user does not quote
 *  replies or the message has no body to quote. Reads the original's inline
 *  images, so it is a round trip for an HTML mail that has any. */
internal suspend fun MeronMobileState.replyQuoteFor(
    message: MessageBody,
    accountId: String,
): ReplyQuote? = if (quoteReplies) buildReplyQuote(message, accountId) else null

/** [replyQuoteFor] whatever the setting says: a saved reply that already holds
 *  its quote is rebuilt from it. */
internal suspend fun MeronMobileState.buildReplyQuote(
    message: MessageBody,
    accountId: String,
): ReplyQuote? {
    if (!hasReplyQuote(message)) return null
    val dateLabel = formatMessageFullTimestamp(message.dateEpochSeconds)
    val images =
        if (message.bodyHtml.isBlank()) {
            emptyList()
        } else {
            runCatching {
                withContext(ioDispatcher) { readQuoteInlineImages(MobileMailCommandClient(core), accountId, message) }
            }.getOrDefault(emptyList())
        }
    return ReplyQuote(
        text = replyQuotedPlainBody(message, dateLabel),
        html = replyQuotedHtmlBody(message, dateLabel, images.map { it.first }),
        marker = replyQuoteAttribution(message, dateLabel),
        inlineAttachments = images.map { (image, data) -> inlineImageToDraftAttachment(image, data) },
    )
}

/** Unfold the folded quote into the end of the body, where the user can trim it
 *  like any other text; from then on it is part of the body. */
internal fun MeronMobileState.expandComposeQuote() {
    val quote = composeFoldedQuote ?: return
    body = body.trimEnd('\n') + quote.text
    applyReplyQuote(quote)
    composeFoldedQuote = null
}

/** Drop the folded quote: the reply goes out without it. */
internal fun MeronMobileState.removeComposeQuote() {
    composeFoldedQuote = null
}

/** Carry a reply's quote, already in the body, into the composer: the HTML that
 *  stands for it at send time while the plain quote stays untouched. */
internal fun MeronMobileState.applyReplyQuote(quote: ReplyQuote?) {
    composeForwardHtml = quote?.html.orEmpty()
    composeForwardInlineAttachments = quote?.inlineAttachments.orEmpty()
    composeQuoteMarker = quote?.marker ?: FORWARD_QUOTE_MARKER
    composeReplyQuote = quote?.text?.trim().orEmpty()
}

// The inline images a quoted original's HTML shows, with their bytes. Images
// whose bytes can't be read are left out of the quote's cid: rewrite rather than
// being referenced with no part behind them.
internal suspend fun MeronMobileState.readQuoteInlineImages(
    client: MobileMailCommandClient,
    accountId: String,
    message: MessageBody,
): List<Pair<ForwardInlineImage, String>> =
    forwardInlineImages(message).mapNotNull { image ->
        runCatching {
            readAttachmentData(client, accountId, image.attachment.key)
        }.getOrNull()
            ?.takeIf { it.isNotBlank() }
            ?.let { image to it }
    }
