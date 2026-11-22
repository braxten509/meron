// Details opened from a mail conversation or an RSS feed.
// The screen derives participants, attachments and media from the loaded messages.
// Caller callbacks handle composing to a participant and opening, copying or saving content.
package jp.nonbili.meron.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.AttachFile
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Email
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.backhandler.BackHandler
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import jp.nonbili.meron.shared.MessageAttachment
import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.ThreadMediaItem

@OptIn(ExperimentalMaterial3Api::class, ExperimentalComposeUiApi::class)
@Composable
internal fun ConversationDetailsScreen(
    subject: String,
    messages: List<MessageBody>,
    mediaItems: List<ThreadMediaItem>,
    loadImageAttachment: suspend (MessageAttachment) -> ImageBitmap?,
    isRss: Boolean,
    feedUrl: String,
    ownEmail: String,
    onBack: () -> Unit,
    onComposeTo: (String) -> Unit,
    onCopy: (String, String) -> Unit,
    onOpenAttachment: (MessageAttachment) -> Unit,
    onSaveAttachment: (MessageAttachment) -> Unit,
    onOpenGalleryIndex: (Int) -> Unit,
    onOpenUrl: (String) -> Unit,
) {
    val participants = remember(messages, isRss, ownEmail) { conversationParticipants(messages, ownEmail, isRss) }
    val attachments = remember(messages) { messages.flatMap { it.attachments }.asReversed() }
    val fileAttachments = remember(attachments) { attachments.filter { !it.mimeType.startsWith("image/") && !it.mimeType.startsWith("video/") } }
    val mediaRows = remember(mediaItems) { mediaItems.chunked(3) }
    val subjectLabel = tr("composer.fields.subject")

    BackHandler(onBack = onBack)

    Scaffold(
        modifier = Modifier.fillMaxSize(),
        topBar = {
            TopAppBar(
                title = { Text(if (isRss) tr("chat.feedDetails") else tr("chat.conversationDetails")) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = tr("buttons.back"))
                    }
                },
            )
        },
    ) { innerPadding ->
        val detailsListState = rememberLazyListState()
        LazyColumn(
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(innerPadding)
                    .appScrollbar(detailsListState)
                    .padding(horizontal = 16.dp),
            state = detailsListState,
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            // Section 0: Subject
            item {
                Text(
                    text = subjectLabel.uppercase(),
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.primary,
                    fontWeight = FontWeight.Bold,
                    modifier = Modifier.padding(top = 8.dp, bottom = 4.dp),
                )
                Row(
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .padding(vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Text(
                        text = subject,
                        modifier = Modifier.weight(1f),
                    )
                    IconButton(
                        onClick = { onCopy(subjectLabel, subject) },
                        modifier = Modifier.size(36.dp),
                    ) {
                        Icon(
                            imageVector = Icons.Filled.ContentCopy,
                            contentDescription = tr("chat.copySubject"),
                            modifier = Modifier.size(18.dp),
                        )
                    }
                }
                HorizontalDivider(
                    modifier = Modifier.padding(top = 12.dp),
                    color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f),
                )
            }

            // Section 1: Feed URL
            if (isRss && feedUrl.isNotBlank()) {
                item {
                    Text(
                        text = tr("chat.feedUrl").uppercase(),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.primary,
                        fontWeight = FontWeight.Bold,
                        modifier = Modifier.padding(top = 8.dp, bottom = 4.dp),
                    )
                    Row(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .clip(RoundedCornerShape(8.dp))
                                .clickable { onOpenUrl(feedUrl) }
                                .padding(vertical = 6.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        Text(
                            text = feedUrl,
                            color = MaterialTheme.colorScheme.primary,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f),
                        )
                        val feedUrlLabel = tr("chat.feedUrl")
                        IconButton(
                            onClick = { onCopy(feedUrlLabel, feedUrl) },
                            modifier = Modifier.size(36.dp),
                        ) {
                            Icon(
                                imageVector = Icons.Filled.ContentCopy,
                                contentDescription = tr("common.copy"),
                                modifier = Modifier.size(18.dp),
                            )
                        }
                    }
                    HorizontalDivider(
                        modifier = Modifier.padding(top = 12.dp),
                        color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f),
                    )
                }
            }

            // Section 2: People
            if (participants.isNotEmpty()) {
                item {
                    Text(
                        text = tr("chat.people", mapOf("count" to participants.size)).uppercase(),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.primary,
                        fontWeight = FontWeight.Bold,
                        modifier = Modifier.padding(top = 8.dp, bottom = 4.dp),
                    )
                }
                items(participants, key = { it.email }) { person ->
                    Row(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .clip(RoundedCornerShape(8.dp))
                                .padding(vertical = 6.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        Avatar(person.name.ifBlank { person.email }, 36.dp)
                        Column(Modifier.weight(1f)) {
                            Text(
                                text = person.name.ifBlank { person.email } + if (person.isSelf) " (${tr("chat.you")})" else "",
                                fontWeight = FontWeight.Medium,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            if (person.name.isNotBlank() && person.name != person.email) {
                                Text(
                                    text = person.email,
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            }
                        }
                        Row(
                            horizontalArrangement = Arrangement.spacedBy(4.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            val emailLabel = tr("accounts.fields.emailAddress")
                            IconButton(
                                onClick = { onCopy(emailLabel, person.email) },
                                modifier = Modifier.size(36.dp),
                            ) {
                                Icon(
                                    imageVector = Icons.Filled.ContentCopy,
                                    contentDescription = tr("chat.copyEmailAddress"),
                                    modifier = Modifier.size(18.dp),
                                )
                            }
                            if (!person.isSelf) {
                                IconButton(
                                    onClick = { onComposeTo(person.email) },
                                    modifier = Modifier.size(36.dp),
                                ) {
                                    Icon(
                                        imageVector = Icons.Filled.Email,
                                        contentDescription = tr("mobile.tabs.mail"),
                                        modifier = Modifier.size(18.dp),
                                    )
                                }
                            }
                        }
                    }
                }
            }

            // Section 3: Media
            if (mediaItems.isNotEmpty()) {
                item {
                    HorizontalDivider(
                        modifier = Modifier.padding(vertical = 12.dp),
                        color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f),
                    )
                    Text(
                        text = tr("chat.media").uppercase(),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.primary,
                        fontWeight = FontWeight.Bold,
                        modifier = Modifier.padding(bottom = 4.dp),
                    )
                }
                itemsIndexed(
                    mediaRows,
                    key = { index, row -> "$index:${row.joinToString("|") { "${it.type}-${it.filename}" }}" },
                ) { _, row ->
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(bottom = 6.dp),
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        row.forEach { item ->
                            Box(Modifier.weight(1f)) {
                                ConversationMediaTile(
                                    item = item,
                                    loadImageAttachment = loadImageAttachment,
                                    onOpen = {
                                        val imageIndex = item.galleryIndex
                                        if (item.type == "image" && imageIndex != null) {
                                            onOpenGalleryIndex(imageIndex)
                                        } else {
                                            onOpenAttachment(item.attachment)
                                        }
                                    },
                                )
                            }
                        }
                        repeat(3 - row.size) { Spacer(Modifier.weight(1f)) }
                    }
                }
            }

            // Section 3: Files
            if (fileAttachments.isNotEmpty()) {
                item {
                    HorizontalDivider(
                        modifier = Modifier.padding(vertical = 12.dp),
                        color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f),
                    )
                    Text(
                        text = tr("chat.files").uppercase(),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.primary,
                        fontWeight = FontWeight.Bold,
                        modifier = Modifier.padding(bottom = 4.dp),
                    )
                }
                items(fileAttachments.withIndex().toList(), key = { "file-${it.index}" }) { indexed ->
                    val attachment = indexed.value
                    Row(
                        modifier =
                            Modifier
                                .fillMaxWidth()
                                .clip(RoundedCornerShape(8.dp))
                                .padding(vertical = 6.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        Icon(
                            imageVector = Icons.Filled.AttachFile,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.primary,
                            modifier = Modifier.size(24.dp),
                        )
                        Column(Modifier.weight(1f)) {
                            Text(
                                text = attachment.filename.ifBlank { tr("chat.attachment") },
                                fontWeight = FontWeight.Medium,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            Text(
                                text =
                                    listOf(
                                        attachment.mimeType,
                                        formatBytes(attachment.sizeBytes),
                                    ).filter { it.isNotBlank() }.joinToString(" · "),
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                        Row(
                            horizontalArrangement = Arrangement.spacedBy(4.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            TextButton(
                                onClick = { onOpenAttachment(attachment) },
                                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
                                modifier = Modifier.height(32.dp),
                            ) {
                                Text(tr("kanban.actions.openThread"), fontSize = 12.sp)
                            }
                            TextButton(
                                onClick = { onSaveAttachment(attachment) },
                                contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
                                modifier = Modifier.height(32.dp),
                            ) {
                                Text(tr("buttons.save"), fontSize = 12.sp)
                            }
                        }
                    }
                }
            }
        }
    }
}
