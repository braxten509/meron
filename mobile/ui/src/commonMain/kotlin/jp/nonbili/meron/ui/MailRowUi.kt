// Renders the conversation rows shown in the mobile mailbox.
// Each row combines correspondent information, message state and attachment previews.
// The mailbox owns the click and selection handlers passed to the row.

package jp.nonbili.meron.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.filled.StarBorder
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.compositeOver
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.MessageAttachment
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.folderIsDrafts

@OptIn(ExperimentalFoundationApi::class)
@Composable
internal fun MailRow(
    thread: ThreadSummary,
    account: AccountSummary?,
    showSenderImages: Boolean,
    selected: Boolean,
    selectionActive: Boolean,
    onOpen: () -> Unit,
    onLongPress: () -> Unit,
    onToggleStar: () -> Unit,
    meronFolderLabel: String? = null,
) {
    val unread = thread.unread
    val chat = LocalChatColors.current
    val senderLabel = thread.sender.ifBlank { thread.accountId }
    val hasDraft = thread.hasDraft || thread.folderRole == "drafts" || (thread.folderRole == "folder" && folderIsDrafts(thread.folder))
    val draftBadge = tr("chat.draft")
    val rowSurface = MaterialTheme.colorScheme.surface
    val rowBackground =
        when {
            selected -> {
                MaterialTheme.colorScheme.primaryContainer
                    .copy(alpha = 0.65f)
                    .compositeOver(rowSurface)
            }

            unread -> {
                MaterialTheme.colorScheme.primary
                    .copy(alpha = 0.07f)
                    .compositeOver(rowSurface)
            }

            else -> {
                rowSurface
            }
        }
    Row(
        Modifier
            .fillMaxWidth()
            .background(rowBackground)
            .combinedClickable(onClick = onOpen, onLongClick = onLongPress)
            .padding(horizontal = 12.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        if (selectionActive) {
            Box(Modifier.size(38.dp), contentAlignment = Alignment.Center) {
                Icon(
                    Icons.Filled.CheckCircle,
                    contentDescription = tr(if (selected) "common.selected" else "common.notSelected"),
                    tint = if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outlineVariant,
                    modifier = Modifier.size(24.dp),
                )
            }
        } else {
            ThreadAvatarWithAccountBadge(
                senderLabel = senderLabel,
                showSenderImages = showSenderImages,
                account = account,
            )
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(1.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                // The name gives up width before the count does (weight without
                // fill), so the count stays pinned to the end of the sender
                // rather than drifting out to the timestamp.
                Row(
                    Modifier.weight(1f),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    val me = tr("threads.me")
                    Text(
                        if (thread.senders.size > 1) {
                            val names = thread.senders.map { it.ifBlank { me } }
                            if (thread.sendersTruncated) {
                                names.first() + " .. " + names.drop(1).joinToString(", ")
                            } else {
                                names.joinToString(", ")
                            }
                        } else {
                            senderLabel.substringBefore('@')
                        },
                        fontSize = 13.sp,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f, fill = false),
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                    // Gmail-style thread size, shown only once a thread has a
                    // reply. It rides beside the sender so it never competes
                    // with the unread dot in the trailing slot.
                    if (thread.messageCount > 1) {
                        Text(
                            thread.messageCount.toString(),
                            fontSize = 12.sp,
                            fontWeight = FontWeight.Normal,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(start = 4.dp),
                        )
                    }
                }
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    val unreadContentDescription = tr("filters.unread")
                    Text(
                        formatInboxTimestamp(thread.dateEpochSeconds),
                        fontSize = 11.sp,
                        fontWeight = FontWeight.Normal,
                        color = if (unread) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    if (unread) {
                        Box(
                            Modifier
                                .size(8.dp)
                                .clip(CircleShape)
                                .background(MaterialTheme.colorScheme.primary)
                                .semantics { contentDescription = unreadContentDescription },
                        )
                    }
                }
            }
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                Text(
                    buildAnnotatedString {
                        if (hasDraft) {
                            withStyle(
                                SpanStyle(
                                    color = MaterialTheme.colorScheme.error,
                                    fontWeight = FontWeight.Normal,
                                ),
                            ) {
                                append(draftBadge)
                                append(" ")
                            }
                        }
                        withStyle(
                            SpanStyle(
                                color = MaterialTheme.colorScheme.onSurface,
                                fontWeight = if (unread) FontWeight.SemiBold else FontWeight.Normal,
                            ),
                        ) {
                            append(thread.subject.ifBlank { "(no subject)" })
                        }
                        if (thread.preview.isNotBlank()) {
                            withStyle(SpanStyle(color = MaterialTheme.colorScheme.onSurfaceVariant)) {
                                append(" — ${thread.preview}")
                            }
                        }
                    },
                    fontSize = 12.sp,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                // Sender rows star single emails inside the chat, not the row; the
                // spacer keeps the row the same size as a thread row.
                if (selectionActive || thread.isSenderGroup) {
                    Spacer(Modifier.size(30.dp))
                } else {
                    IconButton(onClick = onToggleStar, modifier = Modifier.size(30.dp)) {
                        Icon(
                            if (thread.starred) Icons.Filled.Star else Icons.Filled.StarBorder,
                            contentDescription = tr(if (thread.starred) "chat.unstar" else "chat.star"),
                            tint = if (thread.starred) chat.star else MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.size(22.dp),
                        )
                    }
                }
            }
            if (meronFolderLabel != null) {
                // One line with the tags, so the row grows no taller than a tagged one.
                Row(
                    Modifier.padding(top = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    MeronFolderRowChip(meronFolderLabel, Modifier.widthIn(max = 140.dp))
                    MailRowTagChips(thread.tags)
                }
            } else {
                MailRowTagChips(thread.tags, Modifier.padding(top = 4.dp))
            }
            if (thread.files.isNotEmpty()) {
                MailRowAttachmentChips(thread.files, Modifier.padding(top = 4.dp))
            }
        }
    }
}

/** How many chips a row names before collapsing the rest into "+N". */
private const val MAX_ATTACHMENT_CHIPS = 2

// Gmail-style attachment chips under a mail row: the first files by name, then
// a count of the rest. Display only — the row itself is the click target.
@Composable
internal fun MailRowAttachmentChips(
    files: List<MessageAttachment>,
    modifier: Modifier = Modifier,
) {
    val shown = files.take(MAX_ATTACHMENT_CHIPS)
    val rest = files.size - shown.size
    Row(
        modifier.padding(end = 30.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        shown.forEach { file ->
            Row(
                Modifier
                    .weight(1f, fill = false)
                    .border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(50))
                    .padding(horizontal = 8.dp, vertical = 3.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Icon(
                    fileIconFor(file.filename, file.mimeType),
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.size(14.dp),
                )
                Text(
                    file.filename,
                    fontSize = 11.sp,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    color = MaterialTheme.colorScheme.onSurface,
                )
            }
        }
        if (rest > 0) {
            Text(
                "+$rest",
                fontSize = 11.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
