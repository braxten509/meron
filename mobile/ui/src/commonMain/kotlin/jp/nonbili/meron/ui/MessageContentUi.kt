// Displays inline message content and remote-image choices.
// The selected display mode determines which body renderer is used.
// Remote content stays under the sender and account preferences.

package jp.nonbili.meron.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.HideImage
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.layout
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import jp.nonbili.meron.shared.MessageAttachment
import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.SendStatus
import jp.nonbili.meron.shared.htmlHasRemoteMedia
import jp.nonbili.meron.shared.standaloneAttachments
import jp.nonbili.meron.shared.visibleImageAttachments

/**
 * Everything below a message's header: the optional subject, a calendar invitation's card, the body
 * (HTML or plain), standalone attachments, and the send-status line. MessageBubble (chat) and
 * MessageRow (traditional) each wrap it in their own chrome, so the two layouts differ only in the
 * frame around this.
 */
@Composable
internal fun ColumnScope.MessageBodyContent(
    message: MessageBody,
    textColor: Color,
    preferHtml: Boolean,
    searchQuery: String,
    activeSearchMatch: Boolean,
    showSubject: Boolean,
    bodyMaxHeight: Dp,
    // See MessageBubble's subjectLine: small, one line, above the body.
    subjectLine: String = "",
    // Horizontal inset for everything but the HTML body: the chat bubble trims
    // its own padding for HTML mail (see [HtmlBubbleHorizontalPadding]) and pads
    // the rest of the message back to where it sits in every other bubble.
    chromeInset: Dp = 0.dp,
    // The bubble or row this sits in: what a darkened HTML body blends onto.
    surface: Color,
    // Whether this message's remote content may load, and the two ways the
    // reader can change that (see [MessageRemoteContent]).
    remoteContent: MessageRemoteContent,
    onOpenAttachment: (MessageAttachment) -> Unit,
    onSaveAttachment: (MessageAttachment) -> Unit,
    loadImageAttachment: suspend (MessageAttachment) -> ImageBitmap?,
    onOpenImageAttachment: (MessageAttachment) -> Unit,
    onOpenHtmlImage: (String) -> Unit,
    onOpenUrl: (String) -> Unit,
    onRetryLoad: () -> Unit,
    // Accept / Maybe / Decline on an invitation's card, with the response.
    onRespondToInvite: (String) -> Unit = {},
    // A cancelled invitation's "Remove from calendar".
    onRemoveInviteEvent: () -> Unit = {},
    // An answered invitation's "Add to calendar", once its event is gone.
    onAddInviteToCalendar: () -> Unit = {},
    // HTML body only: the width a plain-text-like document needs (zero when it
    // should fill the bubble). Null where the caller doesn't size to the body.
    onHtmlNaturalWidth: ((Dp) -> Unit)? = null,
    // How many columns the image grid lays out; the bubble passes fewer for one
    // or two images, and sizes itself to them (see bubbleWidth).
    imageColumns: Int = 3,
) {
    if (showSubject && message.subject.isNotBlank()) {
        Text(
            text = highlightedMessageText(message.subject, searchQuery, activeSearchMatch),
            modifier = Modifier.padding(horizontal = chromeInset),
            color = textColor,
            fontSize = 16.sp,
            lineHeight = 21.sp,
            fontWeight = FontWeight.SemiBold,
        )
    } else if (subjectLine.isNotBlank()) {
        Text(
            text = highlightedMessageText(subjectLine, searchQuery, activeSearchMatch),
            modifier = Modifier.padding(horizontal = chromeInset).padding(bottom = 2.dp),
            color = textColor.copy(alpha = 0.7f),
            fontSize = 12.sp,
            lineHeight = 16.sp,
            fontWeight = FontWeight.Medium,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
    message.invite?.let { invite ->
        InviteCard(
            invite = invite,
            outgoing = message.outgoing,
            textColor = textColor,
            onRespond = onRespondToInvite,
            onRemoveFromCalendar = onRemoveInviteEvent,
            onAddToCalendar = onAddInviteToCalendar,
            modifier = Modifier.padding(horizontal = chromeInset).padding(bottom = 6.dp),
        )
    }
    val htmlBody = usesHtmlBody(message, preferHtml, searchQuery)
    val standaloneAttachmentsForMessage = standaloneAttachments(message)
    val (imageAttachments, otherAttachments) =
        standaloneAttachmentsForMessage.partition { it.mimeType.startsWith("image/") }
    val visibleImages = visibleImageAttachments(imageAttachments, remoteContent.allowRemote)
    if (htmlBody) {
        // The document reports the width it needs instead (see bubbleWidth); the
        // bubble's intrinsic width is left to the header and what else it holds.
        Box(Modifier.then(NoIntrinsicWidth)) {
            HtmlMessageBody(
                html = message.bodyHtml,
                mediaMissing = message.mediaMissing,
                quoteKey = message.id,
                allowRemote = remoteContent.allowRemote,
                maxHeight = bodyMaxHeight,
                onOpenUrl = onOpenUrl,
                onOpenImage = onOpenHtmlImage,
                // Opt-in (the auto-fit setting): a bubble fitting a 640px mail
                // renders it at about half size.
                fitWideContent = LocalAutoFitMessages.current,
                onNaturalWidth = onHtmlNaturalWidth ?: {},
                darkSurface = surface,
            )
        }
    } else if (message.bodyMissing) {
        // The core has no cached body — a different state from a genuinely
        // empty message, so not "(no content)". Still downloading in the
        // background, it arrives by itself; only a failed fetch offers a retry.
        Column(Modifier.padding(horizontal = chromeInset)) {
            Text(
                tr(if (message.bodyLoading) "common.loading" else "chat.messageLoadFailed"),
                color = textColor.copy(alpha = 0.6f),
                fontSize = 15.5.sp,
                lineHeight = 21.sp,
            )
            if (!message.bodyLoading) {
                TextButton(onClick = onRetryLoad, modifier = Modifier.align(Alignment.End)) {
                    Text(tr("chat.retry"))
                }
            }
        }
    } else {
        // Subject is the conversation title (top bar); the body shows the
        // message text, matching the desktop chat reader.
        // The quoted tail folds behind a toggle (see QuoteFold), and opens on its
        // own while the in-thread search matches inside it.
        val quoted = remember(message.body, message.bodyQuoteStart) { splitQuotedBody(message.body, message.bodyQuoteStart) }
        var quoteOpen by remember(message.id) { mutableStateOf(QuoteFoldMemory.isOpen(message.id)) }
        val showQuote = quoteOpen || quoteMatchesSearch(quoted.quote, searchQuery)
        val bodyStyle =
            messageBodyTextStyle(
                MaterialTheme.typography.bodyLarge.copy(
                    fontSize = 15.5.sp,
                    lineHeight = 21.sp,
                ),
            )
        val bodyText: @Composable () -> Unit = {
            Column {
                SelectableMessageText(
                    text = quoted.reply.ifBlank { "(no content)" },
                    onOpenUrl = onOpenUrl,
                    searchQuery = searchQuery,
                    activeSearchMatch = activeSearchMatch,
                    color = if (message.body.isBlank()) textColor.copy(alpha = 0.6f) else textColor,
                    style = bodyStyle,
                )
                if (quoted.quote.isNotEmpty()) {
                    QuoteToggle(
                        open = showQuote,
                        color = textColor,
                        onToggle = {
                            quoteOpen = !showQuote
                            QuoteFoldMemory.setOpen(message.id, quoteOpen)
                        },
                    )
                    if (showQuote) {
                        SelectableMessageText(
                            text = quoted.quote,
                            onOpenUrl = onOpenUrl,
                            searchQuery = searchQuery,
                            activeSearchMatch = activeSearchMatch,
                            color = textColor,
                            style = bodyStyle,
                        )
                    }
                }
            }
        }
        if (bodyMaxHeight == Dp.Unspecified) {
            // Uncapped (the traditional layout, or full chat messages): the
            // message is as tall as it needs to be and the conversation list
            // scrolls it. A nested scroller here would be measured with an
            // infinite height by the lazy list and throw.
            Box(Modifier.padding(horizontal = chromeInset)) { bodyText() }
        } else {
            val bodyScrollState = rememberScrollState()
            Box(
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = chromeInset)
                    .heightIn(max = bodyMaxHeight)
                    .appScrollbar(
                        bodyScrollState,
                        color = textColor.copy(alpha = 0.4f),
                        endOffset = BubbleHorizontalPadding,
                    ).verticalScroll(bodyScrollState),
            ) {
                bodyText()
            }
        }
    }
    if (visibleImages.isNotEmpty() || otherAttachments.isNotEmpty()) {
        Column(
            Modifier.padding(horizontal = chromeInset),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            if (visibleImages.isNotEmpty()) {
                AttachmentImageGrid(
                    images = visibleImages,
                    mediaMissing = message.mediaMissing,
                    loadImageAttachment = loadImageAttachment,
                    onOpen = onOpenImageAttachment,
                    columns = imageColumns,
                    // The tiles take the width they are given, and a picture's
                    // own size would otherwise count as the bubble's content.
                    modifier = Modifier.then(NoIntrinsicWidth),
                )
            }
            otherAttachments.forEach { attachment ->
                AttachmentRow(
                    attachment = attachment,
                    textColor = textColor,
                    onOpen = { onOpenAttachment(attachment) },
                    onSave = { onSaveAttachment(attachment) },
                )
            }
        }
    }
    // Send lifecycle for an optimistically inserted reply: shown until the
    // canonical sent message replaces it on re-fetch (which clears the
    // status). On failure the bubble stays visible so the reply isn't lost.
    when (message.sendStatus) {
        SendStatus.Sending -> {
            Text(
                tr("composer.status.sending"),
                modifier = Modifier.align(Alignment.End).padding(horizontal = chromeInset),
                fontSize = 10.5.sp,
                color = textColor.copy(alpha = 0.55f),
            )
        }

        SendStatus.Failed -> {
            Text(
                tr("mobile.status.failedToSend"),
                modifier = Modifier.align(Alignment.End).padding(horizontal = chromeInset),
                fontSize = 10.5.sp,
                color = MaterialTheme.colorScheme.error,
            )
        }

        SendStatus.None -> {
            Unit
        }
    }
}

/**
 * Whether a message is holding remote content back, and so has something for
 * [BlockedRemoteButton] to offer: blocked attachment images, or a body that
 * references remote media (a newsletter keeps its images there, not in the
 * attachment list).
 */
internal fun blockedRemoteImageCount(
    message: MessageBody,
    remoteContent: MessageRemoteContent,
    preferHtml: Boolean,
    searchQuery: String,
): Int? {
    if (remoteContent.allowRemote) return null
    val images = standaloneAttachments(message).filter { it.mimeType.startsWith("image/") }
    val hidden = images.size - visibleImageAttachments(images, false).size
    if (hidden > 0) return hidden
    val htmlBody = usesHtmlBody(message, preferHtml, searchQuery)
    return if (htmlBody && htmlHasRemoteMedia(message.bodyHtml)) 0 else null
}

/**
 * Offer image-loading choices for one message with blocked remote content.
 *
 * The whole blocked-remote-content affordance for one message: a tinted icon in the header that opens the
 * two reveal actions — show this message's remote content once, or trust its sender for good. It replaces
 * the strip that used to sit above every body: on a newsletter-heavy mailbox that strip showed on nearly
 * every message.
 *
 * Renders nothing when the content is already allowed, or when the message has no remote content to hold
 * back.
 */
@Composable
internal fun BlockedRemoteButton(
    message: MessageBody,
    remoteContent: MessageRemoteContent,
    preferHtml: Boolean,
    searchQuery: String,
    modifier: Modifier = Modifier,
    iconSize: Dp = 15.dp,
) {
    val hiddenImageCount = blockedRemoteImageCount(message, remoteContent, preferHtml, searchQuery) ?: return
    var menuOpen by remember { mutableStateOf(false) }
    Box(modifier) {
        IconButton(onClick = { menuOpen = true }, modifier = Modifier.size(24.dp)) {
            Icon(
                Icons.Filled.HideImage,
                contentDescription = tr("chat.remoteBlocked"),
                modifier = Modifier.size(iconSize),
                // The theme's accent, softened: muted grey among the other
                // header icons reads as decoration, and this is the only sign
                // that part of the message is missing.
                tint = MaterialTheme.colorScheme.primary.copy(alpha = 0.65f),
            )
        }
        DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
            DropdownMenuItem(
                text = {
                    Text(
                        if (hiddenImageCount > 0) {
                            tr("chat.showImages", mapOf("count" to hiddenImageCount))
                        } else {
                            tr("chat.showRemoteContent")
                        },
                    )
                },
                onClick = {
                    menuOpen = false
                    remoteContent.onReveal()
                },
            )
            // Trusting the sender is app-wide and outlives the thread, so it
            // trails the one-off reveal.
            if (remoteContent.senderAddress.isNotEmpty()) {
                DropdownMenuItem(
                    text = {
                        Text(
                            tr("chat.allowRemoteFrom", mapOf("sender" to remoteContent.senderAddress)),
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    },
                    onClick = {
                        menuOpen = false
                        remoteContent.onAllowSender()
                    },
                )
            }
        }
    }
}
