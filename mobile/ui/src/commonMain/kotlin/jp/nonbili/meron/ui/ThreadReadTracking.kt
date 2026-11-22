// Marks visible messages as read as the conversation scrolls.
// Manual unread choices are held until their messages leave the viewport.
// The reply overlay and partially loaded bodies affect which rows qualify as read.

package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.size
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.ThreadSummary

@OptIn(ExperimentalMaterial3Api::class, ExperimentalComposeUiApi::class)
@Composable
internal fun ThreadReadTracking(
    thread: ThreadSummary?,
    active: Boolean,
    messages: List<MessageBody>,
    listState: androidx.compose.foundation.lazy.LazyListState,
    headerCount: Int,
    overlayHeight: Int,
    onMessagesRead: (List<String>) -> Unit,
    onViewedToBottom: () -> Unit,
) {
    val currentMessages by rememberUpdatedState(messages)
    val currentHeaderItemCount by rememberUpdatedState(headerCount)
    val replyOverlayPx by rememberUpdatedState(overlayHeight)
    val currentOnMessagesRead by rememberUpdatedState(onMessagesRead)
    val currentOnViewedToBottom by rememberUpdatedState(onViewedToBottom)
    if (thread != null && active) {
        val density = LocalDensity.current
        val markedReadIds = remember(thread.id) { mutableSetOf<String>() }
        var viewedToBottomSent by remember(thread.id) { mutableStateOf(false) }
        // Messages the reader turned unread by hand, held out of scroll marking
        // until they leave the viewport — otherwise the next layout change
        // (rotation, a body finishing its measurement, older messages loading)
        // marks them read again and the action looks like it did nothing.
        val heldUnreadIds = remember(thread.id) { mutableSetOf<String>() }
        val previousUnread = remember(thread.id) { mutableMapOf<String, Boolean>() }
        for (id in manualUnreadIds(messages, previousUnread)) {
            heldUnreadIds += id
            // Forget that it was ever marked read, so revisiting the thread and
            // reading it again marks it read as usual.
            markedReadIds -= id
        }
        // A message that is no longer unread has nothing left to protect —
        // the reader marked it read again, or the request rolled back. Holding
        // its id would keep the whole-thread marking below switched off for
        // every other message too.
        heldUnreadIds.retainAll { id -> messages.any { it.id == id && it.unread } }
        messages.forEach { previousUnread[it.id] = it.unread }
        LaunchedEffect(thread.id) {
            val topSlackPx = with(density) { 24.dp.roundToPx() }
            val bottomSlackPx = with(density) { 160.dp.roundToPx() }
            snapshotFlow {
                val info = listState.layoutInfo
                ThreadScrollSnapshot(
                    firstVisibleIndex = listState.firstVisibleItemIndex,
                    visible = info.visibleItemsInfo.map { ListItemGeometry(it.index, it.offset, it.size) },
                    totalItemCount = info.totalItemsCount,
                    viewportEndOffset = info.viewportEndOffset - replyOverlayPx,
                )
            }.collect { snapshot ->
                val visibleMessages = currentMessages
                if (visibleMessages.isEmpty()) return@collect
                // A held message is released once it is out of view; reading it
                // again then marks it read like any other.
                val visibleIndices = snapshot.visible.mapTo(mutableSetOf()) { it.index }
                heldUnreadIds.removeAll { id ->
                    val messageIndex = visibleMessages.indexOfFirst { it.id == id }
                    messageIndex < 0 || (messageIndex + currentHeaderItemCount) !in visibleIndices
                }
                val readIds =
                    readMessageIndices(
                        visible = snapshot.visible,
                        firstVisibleIndex = snapshot.firstVisibleIndex,
                        headerItemCount = currentHeaderItemCount,
                        messageCount = visibleMessages.size,
                        topSlackPx = topSlackPx,
                        viewportEndOffset = snapshot.viewportEndOffset,
                    ).mapNotNull { visibleMessages.getOrNull(it) }
                        // A body still loading (or failed) shows only a short
                        // placeholder, which fits on screen without having been
                        // read. It is marked once the body lands and relayouts.
                        .filter { it.unread && !it.bodyMissing }
                        .map { it.id }
                        .filter { it !in heldUnreadIds }
                        .filter { markedReadIds.add(it) }
                if (readIds.isNotEmpty()) currentOnMessagesRead(readIds)
                val atBottom =
                    listViewedToBottom(
                        visible = snapshot.visible,
                        totalItemCount = snapshot.totalItemCount,
                        viewportEndOffset = snapshot.viewportEndOffset,
                        bottomSlackPx = bottomSlackPx,
                    )
                // Reaching the bottom marks the whole thread read — including
                // messages on older pages that are not loaded — so it has to
                // stand down while a hand-unread message is on screen, or it
                // undoes the action the held set just protected. It resumes as
                // soon as that message scrolls out of view.
                // It also waits for placeholder bodies, for the same reason.
                if (
                    atBottom &&
                    heldUnreadIds.isEmpty() &&
                    visibleMessages.none { it.unread && it.bodyMissing } &&
                    (!viewedToBottomSent || visibleMessages.any { it.unread })
                ) {
                    viewedToBottomSent = true
                    currentOnViewedToBottom()
                }
            }
        }
    }
}
