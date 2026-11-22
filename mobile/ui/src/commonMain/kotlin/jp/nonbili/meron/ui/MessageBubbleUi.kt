// Renders a mobile conversation bubble with message
// content, remote media controls and delivery status.
// The parent thread supplies the selected message and
// callbacks.

package jp.nonbili.meron.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.ExpandLess
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material.icons.filled.OpenInFull
import androidx.compose.material.icons.filled.UnfoldLess
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.layout.IntrinsicMeasurable
import androidx.compose.ui.layout.IntrinsicMeasureScope
import androidx.compose.ui.layout.LayoutModifier
import androidx.compose.ui.layout.Measurable
import androidx.compose.ui.layout.MeasureResult
import androidx.compose.ui.layout.MeasureScope
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import jp.nonbili.meron.shared.MessageAttachment
import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.folderIsDrafts
import jp.nonbili.meron.shared.formatRecipientSummary
import jp.nonbili.meron.shared.standaloneAttachments
import jp.nonbili.meron.shared.visibleImageAttachments
import kotlinx.coroutines.delay
import kotlin.math.roundToInt

/** Bubble inner padding; capped bodies offset their scrollbar back over it. */
internal val BubbleHorizontalPadding = 14.dp

/**
 * Bubble inner padding for an HTML body. Mail HTML usually centres itself in a wrapper with 20-40px of
 * padding of its own, and on a phone the two gutters together eat most of the bubble, so the bubble
 * keeps only enough of its own to hold the body off the rounded corners. The chrome around the body
 * pads back up to [BubbleHorizontalPadding] so it still lines up bubble to bubble.
 */
internal val HtmlBubbleHorizontalPadding = 6.dp

/** How long a bubble stays invisible waiting for its web view to report a width
 *  before it shows at full width anyway (a page whose script never runs). */
private const val NATURAL_WIDTH_WAIT_MS = 1000L

/** How long a reported width has to stand before the bubble shows at it: a few
 *  frames, enough for the report that follows the web view's own resize. */
private const val NATURAL_WIDTH_SETTLE_MS = 120L

/** What each HTML body last reported as the width it needs, so a bubble scrolled
 *  back into the list takes its width on the first frame instead of flashing at
 *  full width. [Dp.Unspecified] is not stored: it means not measured yet. */
private val htmlNaturalWidths = HashMap<String, Dp>()

/** Wide screens cap every bubble here so lines stay readable. */
private val MaxBubbleWidth = 560.dp

/** Leaves something out of its parent's intrinsic width, so a hugging bubble
 *  sizes to its text rather than to, say, a photo's pixel width. */
internal object NoIntrinsicWidth : LayoutModifier {
    override fun MeasureScope.measure(
        measurable: Measurable,
        constraints: Constraints,
    ): MeasureResult {
        val placeable = measurable.measure(constraints)
        return layout(placeable.width, placeable.height) { placeable.place(0, 0) }
    }

    override fun IntrinsicMeasureScope.minIntrinsicWidth(
        measurable: IntrinsicMeasurable,
        height: Int,
    ): Int = 0

    override fun IntrinsicMeasureScope.maxIntrinsicWidth(
        measurable: IntrinsicMeasurable,
        height: Int,
    ): Int = 0
}

/**
 * Sizes a bubble to [fraction] of the available width, capped at [MaxBubbleWidth].
 *
 * Narrower when it can hug its content: [hugIntrinsic] takes the content's own widest line (plain
 * text, which Compose can measure), widened to fit [imageColumns] grid tiles at the size a full-width
 * bubble gives them; a measured HTML [natural] width takes that plus [chrome], widened to whatever the
 * header and the rest of the bubble need (the web view counts for nothing there).
 *
 * A [natural] of zero or less means the HTML body fills the bubble.
 */
internal fun Modifier.bubbleWidth(
    fraction: Float,
    natural: Dp = 0.dp,
    chrome: Dp = 0.dp,
    hugIntrinsic: Boolean = false,
    imageColumns: Int = 0,
): Modifier =
    layout { measurable, constraints ->
        val cap = minOf((constraints.maxWidth * fraction).roundToInt(), MaxBubbleWidth.roundToPx())
        val width =
            when {
                hugIntrinsic -> {
                    val grid =
                        if (imageColumns > 0) {
                            val gap = AttachmentImageGridGap.roundToPx()
                            val tile = (cap - chrome.roundToPx() - 2 * gap) / 3
                            imageColumns * tile + (imageColumns - 1) * gap + chrome.roundToPx()
                        } else {
                            0
                        }
                    maxOf(measurable.maxIntrinsicWidth(constraints.maxHeight), grid).coerceAtMost(cap)
                }

                natural > 0.dp -> {
                    maxOf((natural + chrome).roundToPx(), measurable.maxIntrinsicWidth(constraints.maxHeight)).coerceAtMost(cap)
                }

                else -> {
                    cap
                }
            }
        val placeable = measurable.measure(constraints.copy(minWidth = width, maxWidth = width))
        layout(placeable.width, placeable.height) { placeable.place(0, 0) }
    }

/** True when the bubble shows the sender's HTML rather than plain text: the
 *  search highlighter works on the plain body, so an open search turns it off. */
internal fun usesHtmlBody(
    message: MessageBody,
    preferHtml: Boolean,
    searchQuery: String,
): Boolean = preferHtml && message.bodyHtml.isNotBlank() && searchQuery.isBlank()

/** Whether a chat bubble may start folded: one that needs a web page or shows
 *  pictures. Those are what make a long thread scroll slowly; a short plain
 *  email is cheap and always stays open. An open search never folds. */
internal fun bubbleFoldable(
    message: MessageBody,
    preferHtml: Boolean,
    searchQuery: String,
): Boolean =
    searchQuery.isBlank() &&
        (usesHtmlBody(message, preferHtml, searchQuery) || standaloneAttachments(message).any { it.mimeType.startsWith("image/") })

/** The start of what a folded bubble says, from the plain text the core keeps
 *  for every mail (HTML mail included), quoted reply left out. */
internal fun foldedPreviewText(message: MessageBody): String {
    val body = message.bodyQuoteStart?.let { message.body.take(it) } ?: message.body
    return body.take(600).replace(Regex("\\s+"), " ").trim()
}

@Composable
internal fun MessageBubble(
    message: MessageBody,
    outgoing: Boolean,
    chat: ChatColors,
    preferHtml: Boolean,
    searchQuery: String,
    activeSearchMatch: Boolean,
    actionsEnabled: Boolean,
    // Read and star work per item on feed threads too, unlike the mail-only
    // actions (forward, edit as new, delete) [actionsEnabled] gates.
    itemActionsEnabled: Boolean,
    showSubject: Boolean,
    isRss: Boolean,
    remoteContent: MessageRemoteContent,
    onForward: (MessageBody) -> Unit,
    // A small subject line at the top of the bubble; blank for none. A sender
    // chat sets it where the topic changes from the bubble before.
    subjectLine: String = "",
    onReplyAllToMessage: (MessageBody) -> Unit,
    canReplyAllToMessage: (MessageBody) -> Boolean,
    onEditAsNew: (MessageBody) -> Unit,
    onOpenDraft: (MessageBody) -> Unit,
    onToggleRead: (MessageBody) -> Unit,
    onToggleStarred: (MessageBody) -> Unit,
    onDelete: (MessageBody) -> Unit,
    // One email moved or archived on its own; null where that is not offered.
    onMove: ((MessageBody) -> Unit)? = null,
    onArchive: ((MessageBody) -> Unit)? = null,
    onOpenAttachment: (MessageAttachment) -> Unit,
    onSaveAttachment: (MessageAttachment) -> Unit,
    loadImageAttachment: suspend (MessageAttachment) -> ImageBitmap?,
    onOpenImageAttachment: (MessageAttachment) -> Unit,
    onOpenHtmlImage: (String) -> Unit,
    onCopyMessageText: (String, String) -> Unit,
    onComposeTo: (String) -> Unit,
    onOpenMessage: (MessageBody) -> Unit,
    onOpenUrl: (String) -> Unit,
    onRetryLoad: () -> Unit,
    onRespondToInvite: (MessageBody, String) -> Unit = { _, _ -> },
    onRemoveInviteEvent: (MessageBody) -> Unit = {},
    onAddInviteToCalendar: (MessageBody) -> Unit = {},
    // Folded: header plus two lines of text and no web page, until tapped.
    folded: Boolean = false,
    // Folds or opens this bubble; null where it can't fold.
    onToggleFold: (() -> Unit)? = null,
) {
    var menuOpen by remember { mutableStateOf(false) }
    var addressesOpen by remember(message.id) { mutableStateOf(false) }
    val bubbleShape =
        if (outgoing) {
            RoundedCornerShape(topStart = 16.dp, topEnd = 4.dp, bottomEnd = 16.dp, bottomStart = 16.dp)
        } else {
            RoundedCornerShape(topStart = 4.dp, topEnd = 16.dp, bottomEnd = 16.dp, bottomStart = 16.dp)
        }
    val themedBubbleColor = if (outgoing) chat.bubbleOut else chat.bubbleIn
    val textColor = if (outgoing) chat.bubbleOutText else chat.bubbleInText
    // Capped, a long body scrolls inside its bubble; uncapped (the full
    // messages setting), it grows to fit and only the conversation scrolls.
    val bodyMaxHeight = if (LocalChatFullMessages.current) Dp.Unspecified else 360.dp
    val htmlBody = usesHtmlBody(message, preferHtml, searchQuery) && !folded
    // The web view paints the mail on white, so a tinted light bubble (Material
    // You) would frame it as a square white box inside a rounded card. Let the
    // whole bubble be white instead, so the header and body read as one card.
    val bubbleColor =
        if (htmlBody && !LocalDarkMailBodies.current && themedBubbleColor.luminance() > 0.5f) {
            Color.White
        } else {
            themedBubbleColor
        }
    val bubblePadding = if (htmlBody) HtmlBubbleHorizontalPadding else BubbleHorizontalPadding
    // Attached pictures lay out against the bubble's width outside the web view,
    // so a bubble carrying them keeps its full width whatever its text needs.
    val hasImages = standaloneAttachments(message).any { it.mimeType.startsWith("image/") }
    val hugsText = htmlBody && !hasImages
    // A plain-text bubble shrinks to its images as well: one or two take only the
    // columns they fill. Counted the way the body decides what it shows.
    val visibleImageCount =
        if (htmlBody || folded) {
            0
        } else {
            visibleImageAttachments(
                standaloneAttachments(message).filter { it.mimeType.startsWith("image/") },
                remoteContent.allowRemote,
            ).size
        }
    val naturalWidthKey = "${message.id}:${message.bodyHtml.hashCode()}:${LocalDensity.current.fontScale}"
    var naturalWidth by remember(naturalWidthKey) { mutableStateOf(htmlNaturalWidths[naturalWidthKey] ?: Dp.Unspecified) }
    // Settled once the web view's reports stop changing. Each width the bubble
    // takes resizes the web view, which measures and reports again; shown on
    // the first report, the bubble visibly slid as it followed the next few
    // (an outgoing one, pinned to the right, moving its left edge). A width
    // remembered from an earlier showing has already settled.
    var widthSettled by remember(naturalWidthKey) { mutableStateOf(htmlNaturalWidths[naturalWidthKey] != null) }
    // An unmeasured body is laid out at full width but not painted: showing it
    // would flash it wide before it shrinks to its text.
    val widthPending = hugsText && !widthSettled
    LaunchedEffect(widthPending) {
        if (widthPending) {
            delay(NATURAL_WIDTH_WAIT_MS)
            if (naturalWidth == Dp.Unspecified) naturalWidth = 0.dp
            widthSettled = true
        }
    }
    LaunchedEffect(widthPending, naturalWidth) {
        if (widthPending && naturalWidth != Dp.Unspecified) {
            delay(NATURAL_WIDTH_SETTLE_MS)
            widthSettled = true
        }
    }
    // What the chrome around an HTML body adds back to sit where it always does.
    val chromeInset = BubbleHorizontalPadding - bubblePadding
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = if (outgoing) Arrangement.End else Arrangement.Start,
    ) {
        Column(
            Modifier
                // Bubble width tracks the screen: ~85% of available width so it
                // grows on tablets, capped so it stays readable on wide screens.
                // HTML mail is laid out for a wider page than a phone bubble, so
                // it gets the extra tenth (desktop widens its HTML bubbles too).
                .bubbleWidth(
                    fraction = if (htmlBody) 0.95f else 0.85f,
                    natural = if (hugsText && naturalWidth != Dp.Unspecified) naturalWidth else 0.dp,
                    // A dp of slack so a line measured at max-content never wraps on rounding.
                    chrome = bubblePadding * 2 + 1.dp,
                    hugIntrinsic = !htmlBody,
                    imageColumns = minOf(visibleImageCount, 3),
                ).alpha(if (widthPending) 0f else 1f)
                .shadow(3.dp, bubbleShape, clip = false)
                .clip(bubbleShape)
                .then(
                    if (activeSearchMatch) {
                        Modifier.border(2.dp, Color(0xFFFFC107), bubbleShape)
                    } else {
                        Modifier
                    },
                ).background(bubbleColor)
                .then(if (folded && onToggleFold != null) Modifier.clickable(onClick = onToggleFold) else Modifier)
                .padding(start = bubblePadding, end = bubblePadding, top = 8.dp, bottom = 6.dp),
            verticalArrangement = Arrangement.spacedBy(3.dp),
        ) {
            // Sender, timestamp and the actions menu share one row to keep the
            // bubble compact, matching the desktop reader's header layout.
            Row(
                Modifier.fillMaxWidth().padding(horizontal = chromeInset),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                // Tapping the sender (or, on outgoing bubbles, the recipient
                // summary) expands the full addresses, the way clicking the
                // sender does on desktop.
                // An incoming message names its sender, and the recipients only
                // when a reply-all would reach someone else: without them the
                // reader answers the sender alone without ever noticing the
                // others. A message addressed to us alone stays quiet — the
                // line would only repeat our own name.
                val recipients =
                    if (outgoing || canReplyAllToMessage(message)) {
                        remember(message.to, message.cc) { formatRecipientSummary(message.to, message.cc) }
                    } else {
                        ""
                    }
                // The toggle is on every bubble, even a draft with no recipients
                // yet: the details always lead with From, and a chevron that
                // came and went between messages read as an arbitrary
                // difference between them. A feed item is the exception — it
                // has no recipients at all, so the details could only repeat
                // the feed name the header already shows.
                Row(
                    Modifier
                        .weight(1f)
                        .clip(RoundedCornerShape(4.dp))
                        .then(if (isRss) Modifier else Modifier.clickable { addressesOpen = !addressesOpen }),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    if (!outgoing) {
                        Text(
                            message.from.ifBlank { message.fromAddr },
                            modifier = Modifier.weight(1f, fill = false),
                            fontSize = 12.5.sp,
                            fontWeight = FontWeight.Bold,
                            color = MaterialTheme.colorScheme.primary,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        if (recipients.isNotBlank()) {
                            Text(
                                tr("chat.toRecipients", mapOf("recipients" to recipients)),
                                modifier = Modifier.weight(1f, fill = false),
                                fontSize = 11.sp,
                                color = textColor.copy(alpha = 0.6f),
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    } else if (recipients.isNotBlank()) {
                        // An outgoing bubble has no sender to name, and a reply and a
                        // forward of the same text look identical without recipients —
                        // so the slot shows who received it instead.
                        Text(
                            tr("chat.toRecipients", mapOf("recipients" to recipients)),
                            modifier = Modifier.weight(1f, fill = false),
                            fontSize = 11.sp,
                            color = textColor.copy(alpha = 0.6f),
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    if (!isRss) {
                        Icon(
                            if (addressesOpen) Icons.Filled.ExpandLess else Icons.Filled.ExpandMore,
                            contentDescription = if (addressesOpen) tr("chat.hideDetails") else tr("chat.showDetails"),
                            modifier = Modifier.size(14.dp),
                            tint = textColor.copy(alpha = 0.55f),
                        )
                    }
                }
                if (folderIsDrafts(message.folderId)) {
                    Surface(
                        shape = RoundedCornerShape(4.dp),
                        color = MaterialTheme.colorScheme.errorContainer,
                        modifier = Modifier.padding(end = 4.dp),
                    ) {
                        Text(
                            text = tr("chat.draft"),
                            color = MaterialTheme.colorScheme.onErrorContainer,
                            fontSize = 10.sp,
                            fontWeight = FontWeight.Bold,
                            modifier = Modifier.padding(horizontal = 4.dp, vertical = 1.dp),
                        )
                    }
                }
                BlockedRemoteButton(
                    message = message,
                    remoteContent = remoteContent,
                    preferHtml = preferHtml,
                    searchQuery = searchQuery,
                )
                Text(
                    formatInboxTimestamp(message.dateEpochSeconds),
                    fontSize = 10.5.sp,
                    color = textColor.copy(alpha = 0.55f),
                )
                if (onToggleFold != null && !folded) {
                    IconButton(onClick = onToggleFold, modifier = Modifier.size(24.dp)) {
                        Icon(
                            imageVector = Icons.Filled.UnfoldLess,
                            contentDescription = tr("chat.fold"),
                            modifier = Modifier.size(16.dp),
                            tint = textColor.copy(alpha = 0.55f),
                        )
                    }
                }
                val isDraft = folderIsDrafts(message.folderId)
                IconButton(
                    onClick = {
                        if (isDraft) {
                            onOpenDraft(message)
                        } else {
                            onOpenMessage(message)
                        }
                    },
                    modifier = Modifier.size(24.dp),
                ) {
                    Icon(
                        imageVector = if (isDraft) Icons.Filled.Edit else Icons.Filled.OpenInFull,
                        contentDescription = if (isDraft) tr("chat.draft") else tr("threads.actions.openInNewTab"),
                        modifier = Modifier.size(15.dp),
                        tint = textColor.copy(alpha = 0.55f),
                    )
                }
                MessageActionsButton(
                    preferHtml = preferHtml,
                    allowRemote = remoteContent.allowRemote,
                    message = message,
                    isRss = isRss,
                    tint = textColor.copy(alpha = 0.55f),
                    actionsEnabled = actionsEnabled,
                    itemActionsEnabled = itemActionsEnabled,
                    onForward = onForward,
                    onReplyAllToMessage = onReplyAllToMessage,
                    canReplyAllToMessage = canReplyAllToMessage,
                    onEditAsNew = onEditAsNew,
                    onToggleRead = onToggleRead,
                    onToggleStarred = onToggleStarred,
                    onDelete = onDelete,
                    onMove = onMove,
                    onArchive = onArchive,
                    onCopyMessageText = onCopyMessageText,
                    onOpenUrl = onOpenUrl,
                )
            }
            if (addressesOpen) {
                MessageAddressDetails(
                    message = message,
                    onCopy = onCopyMessageText,
                    onComposeTo = onComposeTo,
                    textColor = textColor,
                    modifier = Modifier.padding(bottom = 2.dp, start = chromeInset, end = chromeInset),
                )
            }
            if (folded) {
                FoldedBubbleBody(
                    message = message,
                    textColor = textColor,
                    subjectLine = if (showSubject) message.subject else subjectLine,
                    modifier = Modifier.padding(horizontal = chromeInset),
                )
            } else {
                MessageBodyContent(
                    message = message,
                    textColor = textColor,
                    preferHtml = preferHtml,
                    searchQuery = searchQuery,
                    activeSearchMatch = activeSearchMatch,
                    showSubject = showSubject,
                    subjectLine = subjectLine,
                    bodyMaxHeight = bodyMaxHeight,
                    chromeInset = chromeInset,
                    surface = bubbleColor,
                    remoteContent = remoteContent,
                    onOpenAttachment = onOpenAttachment,
                    onSaveAttachment = onSaveAttachment,
                    loadImageAttachment = loadImageAttachment,
                    onOpenImageAttachment = onOpenImageAttachment,
                    onOpenHtmlImage = onOpenHtmlImage,
                    onOpenUrl = onOpenUrl,
                    onRetryLoad = onRetryLoad,
                    onRespondToInvite = { response -> onRespondToInvite(message, response) },
                    onRemoveInviteEvent = { onRemoveInviteEvent(message) },
                    onAddInviteToCalendar = { onAddInviteToCalendar(message) },
                    imageColumns = if (htmlBody) 3 else visibleImageCount.coerceIn(1, 3),
                    onHtmlNaturalWidth =
                        if (hugsText) {
                            { width ->
                                naturalWidth = width.coerceAtLeast(0.dp)
                                htmlNaturalWidths[naturalWidthKey] = naturalWidth
                            }
                        } else {
                            null
                        },
                )
            }
        }
    }
}

/** A folded bubble's body: the subject line, two lines of the mail's text and
 *  the cue that a tap opens it. The bubble itself takes the tap. */
@Composable
private fun FoldedBubbleBody(
    message: MessageBody,
    textColor: Color,
    subjectLine: String,
    modifier: Modifier = Modifier,
) {
    Column(modifier, verticalArrangement = Arrangement.spacedBy(2.dp)) {
        if (subjectLine.isNotBlank()) {
            Text(
                subjectLine,
                color = textColor.copy(alpha = 0.7f),
                fontSize = 12.sp,
                lineHeight = 16.sp,
                fontWeight = FontWeight.Medium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        val preview = remember(message.id, message.body) { foldedPreviewText(message) }
        if (preview.isNotBlank()) {
            Text(
                preview,
                color = textColor.copy(alpha = 0.8f),
                fontSize = 14.sp,
                lineHeight = 19.sp,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
        Text(
            tr("chat.tapToOpen"),
            color = MaterialTheme.colorScheme.primary,
            fontSize = 12.5.sp,
            fontWeight = FontWeight.Medium,
            modifier = Modifier.padding(top = 2.dp, bottom = 2.dp),
        )
    }
}
