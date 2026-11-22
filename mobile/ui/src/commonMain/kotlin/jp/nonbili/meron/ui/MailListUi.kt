// Renders the mobile mailbox list and handles row
// selection and pagination. Visible thread summaries
// correspond to the current account, folder and search
// scope.

package jp.nonbili.meron.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Archive
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Tune
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.threadIdIsRss
import kotlinx.coroutines.launch

// Rows from the end at which scrolling starts fetching the next page.
private const val THREAD_LIST_PAGINATION_MARGIN = 3

/**
 * Whether the thread list is scrolled close enough to its end to pull the next page.
 * [lastVisibleIndex] is null before anything is laid out — that, and an empty list, are deliberately
 * *not* near the bottom: the plain comparison also holds for a list with no rows (`0 >= 0 - 3`), which
 * fired pagination against a list the user had never seen.
 */
internal fun threadListNearBottom(
    lastVisibleIndex: Int?,
    threadCount: Int,
): Boolean {
    if (lastVisibleIndex == null || threadCount == 0) return false
    return lastVisibleIndex >= threadCount - THREAD_LIST_PAGINATION_MARGIN
}

/**
 * A mailbox's retained thread-list position: the scroll state, and the id of the row that was at the top of the list the last time it was on screen.
 *
 * The id has to outlive composition alongside the scroll state. The mail route is disposed while a
 * conversation is open, so an id remembered inside the list would come back initialized to whatever
 * arrived meanwhile and read as "no new mail", while the retained scroll state re-anchors the arrivals
 * off screen.
 */
internal class MailListPosition(
    val listState: LazyListState = LazyListState(),
) {
    var lastFirstThreadId: String? = null
}

/**
 * Whether a list that just grew at the top should follow the arrivals up.
 *
 * New mail is prepended, and the keyed [LazyColumn] pins the viewport to the row that was showing — so
 * a list sitting at the top has the new rows pushed off screen above it, with nothing to say they
 * arrived. Only a list that was at the top follows: anyone who has scrolled keeps their place.
 *
 * [previousFirstIndex] is where the old first row sits in the new list, so `> 0` is the test for a
 * prepend. A row that vanished (-1) or stayed put (0) means the list was swapped wholesale (folder
 * change, search) or did not grow at the top, and neither is new mail.
 *
 * "Was at the top" is read from the last laid-out frame rather than the scroll position, because this
 * runs either side of the measure that re-anchors the keyed list and the two regimes report different
 * indices for the same viewport. The layout is unambiguous in both: the old first row sits flush with
 * the top only for a list that was at the top.
 *
 * [firstVisibleKey] is null before anything has been laid out, and then the untouched scroll position
 * is the same answer.
 */
internal fun threadListShouldFollowNewThreads(
    previousFirstId: String,
    previousFirstIndex: Int,
    firstVisibleKey: Any?,
    firstVisibleOffset: Int,
    savedFirstVisibleIndex: Int,
    savedFirstVisibleScrollOffset: Int,
): Boolean {
    if (previousFirstIndex <= 0) return false
    if (firstVisibleKey != null) return firstVisibleKey == previousFirstId && firstVisibleOffset == 0
    return savedFirstVisibleIndex == 0 && savedFirstVisibleScrollOffset == 0
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun MailList(
    threads: List<ThreadSummary>,
    accounts: List<AccountSummary>,
    position: MailListPosition,
    scrollToTopRequest: Long,
    canLoadMore: Boolean,
    loadingMore: Boolean,
    onOpen: (ThreadSummary) -> Unit,
    onToggleStar: (ThreadSummary) -> Unit,
    onArchive: (ThreadSummary) -> Unit,
    onDelete: (ThreadSummary) -> Unit,
    selectedThreadIds: Set<String>,
    selectionActive: Boolean,
    onToggleSelected: (ThreadSummary) -> Unit,
    onLongPress: (ThreadSummary) -> Unit,
    // Invoked with true when the user tapped the load-more button, false when
    // pagination auto-fired from scrolling near the bottom.
    onLoadMore: (userInitiated: Boolean) -> Unit,
    showSenderImages: Boolean,
    showAccountBadge: Boolean,
    /** The Meron folder a row's conversation is in, for its "In ‹name›" chip; null for none. */
    meronFolderLabel: (ThreadSummary) -> String? = { null },
) {
    val listState = position.listState
    LaunchedEffect(scrollToTopRequest) {
        if (scrollToTopRequest > 0) listState.scrollToItem(0)
    }
    // Ride mail arriving at the top of a list the user is already sitting at the
    // top of. Every refresh other than pull-to-refresh leaves the scroll
    // position alone, so without this the new rows land above the viewport.
    val firstThreadId = threads.firstOrNull()?.id
    LaunchedEffect(position, firstThreadId) {
        val previousId = position.lastFirstThreadId
        position.lastFirstThreadId = firstThreadId
        if (previousId == null || previousId == firstThreadId) return@LaunchedEffect
        val firstVisible = listState.layoutInfo.visibleItemsInfo.firstOrNull()
        if (
            threadListShouldFollowNewThreads(
                previousFirstId = previousId,
                previousFirstIndex = threads.indexOfFirst { it.id == previousId },
                firstVisibleKey = firstVisible?.key,
                firstVisibleOffset = firstVisible?.offset ?: 0,
                savedFirstVisibleIndex = listState.firstVisibleItemIndex,
                savedFirstVisibleScrollOffset = listState.firstVisibleItemScrollOffset,
            )
        ) {
            listState.scrollToItem(0)
        }
    }
    var pendingFeedRemoval by remember { mutableStateOf<ThreadSummary?>(null) }
    val accountsById = remember(accounts) { accounts.associateBy { it.id } }
    // Read the list through rememberUpdatedState so the derivation tracks the
    // current page count; a keyless remember would freeze the first list's size
    // into the closure and near-bottom detection would drift after reloads.
    val currentThreads by rememberUpdatedState(threads)
    val nearBottom by remember(listState) {
        derivedStateOf {
            threadListNearBottom(
                lastVisibleIndex =
                    listState.layoutInfo.visibleItemsInfo
                        .lastOrNull()
                        ?.index,
                threadCount = currentThreads.size,
            )
        }
    }
    LaunchedEffect(nearBottom, canLoadMore, loadingMore) {
        if (nearBottom && canLoadMore && !loadingMore) onLoadMore(false)
    }
    pendingFeedRemoval?.let { thread ->
        AlertDialog(
            onDismissRequest = { pendingFeedRemoval = null },
            title = { Text(tr("feeds.actions.confirmDelete")) },
            text = { Text(tr("feeds.deleteHint")) },
            confirmButton = {
                TextButton(
                    onClick = {
                        pendingFeedRemoval = null
                        onArchive(thread)
                    },
                ) {
                    Text(tr("buttons.delete"), color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { pendingFeedRemoval = null }) { Text(tr("buttons.cancel")) }
            },
        )
    }
    LazyColumn(
        Modifier.fillMaxSize().appScrollbar(listState),
        state = listState,
        contentPadding = PaddingValues(bottom = 88.dp),
    ) {
        items(threads, key = { it.id }) { thread ->
            if (selectionActive) {
                MailRow(
                    thread = thread,
                    account = accountsById[thread.accountId].takeIf { showAccountBadge },
                    showSenderImages = showSenderImages,
                    selected = thread.id in selectedThreadIds,
                    selectionActive = true,
                    onOpen = { onToggleSelected(thread) },
                    onLongPress = { onLongPress(thread) },
                    onToggleStar = { onToggleStar(thread) },
                    meronFolderLabel = meronFolderLabel(thread),
                )
            } else {
                val isRss = threadIdIsRss(thread.id)
                val isRssItem = isRss && thread.threadId.isNotBlank() && thread.threadId != thread.id
                val dismissState = rememberSwipeToDismissBoxState()
                val swipeScope = rememberCoroutineScope()
                SwipeToDismissBox(
                    state = dismissState,
                    enableDismissFromStartToEnd = !isRss,
                    enableDismissFromEndToStart = !isRssItem,
                    // Called once the swipe has settled past the threshold, never
                    // mid-drag. Watching the state's value instead fired while the
                    // finger was still down: the reset lost to the drag, the action
                    // never ran, and the row stayed slid open on its colour.
                    onDismiss = { direction ->
                        swipeScope.launch { dismissState.snapTo(SwipeToDismissBoxValue.Settled) }
                        when (direction) {
                            SwipeToDismissBoxValue.StartToEnd -> onArchive(thread)
                            SwipeToDismissBoxValue.EndToStart -> if (isRss) pendingFeedRemoval = thread else onDelete(thread)
                            SwipeToDismissBoxValue.Settled -> Unit
                        }
                    },
                    backgroundContent = {
                        val direction = dismissState.dismissDirection
                        val deleting = direction == SwipeToDismissBoxValue.EndToStart
                        Box(
                            Modifier
                                .fillMaxSize()
                                .background(
                                    if (deleting) {
                                        MaterialTheme.colorScheme.errorContainer
                                    } else {
                                        MaterialTheme.colorScheme.primaryContainer
                                    },
                                ).padding(horizontal = 24.dp),
                            contentAlignment =
                                if (deleting) {
                                    Alignment.CenterEnd
                                } else {
                                    Alignment.CenterStart
                                },
                        ) {
                            Icon(
                                if (deleting || isRss) Icons.Filled.Delete else Icons.Filled.Archive,
                                contentDescription =
                                    when {
                                        deleting -> tr("buttons.delete")
                                        isRss -> tr("mobile.actions.removeFeed")
                                        else -> tr("notification.actionArchive")
                                    },
                                tint =
                                    if (deleting) {
                                        MaterialTheme.colorScheme.onErrorContainer
                                    } else {
                                        MaterialTheme.colorScheme.onPrimaryContainer
                                    },
                            )
                        }
                    },
                ) {
                    MailRow(
                        thread = thread,
                        account = accountsById[thread.accountId].takeIf { showAccountBadge },
                        showSenderImages = showSenderImages,
                        selected = false,
                        selectionActive = false,
                        onOpen = { onOpen(thread) },
                        onLongPress = { onLongPress(thread) },
                        onToggleStar = { onToggleStar(thread) },
                        meronFolderLabel = meronFolderLabel(thread),
                    )
                }
            }
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.4f))
        }
        if (canLoadMore || loadingMore) {
            item {
                Box(Modifier.fillMaxWidth().padding(16.dp), contentAlignment = Alignment.Center) {
                    if (loadingMore) {
                        CircularProgressIndicator(Modifier.size(24.dp))
                    } else {
                        OutlinedButton(onClick = { onLoadMore(true) }) {
                            Text(tr("threads.actions.loadMore"))
                        }
                    }
                }
            }
        }
    }
}

/**
 * The search box an app bar opens into, shared by the mail list and the board: a filled pill with the query, search options and a clear button.
 *
 * The bar's back arrow stands beside it, so it carries no search icon of its own and the text gets the
 * width. [leading] goes before the text, such as the column a board search is narrowed to, and
 * [onBackspaceEmpty] is a hardware Backspace in the empty box.
 */
@Composable
internal fun HeaderSearchField(
    search: String,
    placeholder: String,
    onSearchChange: (String) -> Unit,
    onSearchSubmit: () -> Unit,
    onOpenOptions: (() -> Unit)? = null,
    focusRequester: FocusRequester? = null,
    onBackspaceEmpty: (() -> Unit)? = null,
    leading: (@Composable () -> Unit)? = null,
) {
    Surface(
        modifier = Modifier.fillMaxWidth().height(48.dp),
        shape = CircleShape,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
    ) {
        Row(
            Modifier.fillMaxWidth().padding(start = if (leading != null) 8.dp else 16.dp, end = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            leading?.invoke()
            BasicTextField(
                value = search,
                onValueChange = onSearchChange,
                singleLine = true,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                keyboardActions =
                    KeyboardActions(
                        onSearch = { onSearchSubmit() },
                        onDone = { onSearchSubmit() },
                    ),
                textStyle = MaterialTheme.typography.bodyMedium.copy(color = MaterialTheme.colorScheme.onSurface),
                modifier =
                    Modifier
                        .weight(1f)
                        .padding(start = if (leading != null) 8.dp else 0.dp)
                        .then(if (focusRequester != null) Modifier.focusRequester(focusRequester) else Modifier)
                        .onPreviewKeyEvent { event ->
                            if (event.type == KeyEventType.KeyUp && event.key == Key.Enter) {
                                onSearchSubmit()
                                true
                            } else if (
                                onBackspaceEmpty != null &&
                                event.type == KeyEventType.KeyDown &&
                                event.key == Key.Backspace &&
                                search.isEmpty()
                            ) {
                                onBackspaceEmpty()
                                true
                            } else {
                                false
                            }
                        },
                decorationBox = { innerTextField ->
                    Box(contentAlignment = Alignment.CenterStart) {
                        if (search.isBlank()) {
                            Text(
                                placeholder,
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                        innerTextField()
                    }
                },
            )
            if (onOpenOptions != null) {
                IconButton(onClick = onOpenOptions, modifier = Modifier.size(36.dp)) {
                    Icon(Icons.Filled.Tune, contentDescription = tr("search.options"), modifier = Modifier.size(18.dp))
                }
            }
            if (search.isNotBlank()) {
                IconButton(
                    onClick = {
                        onSearchChange("")
                        onSearchSubmit()
                    },
                    modifier = Modifier.size(36.dp),
                ) {
                    Icon(Icons.Filled.Close, contentDescription = tr("common.clearSearch"), modifier = Modifier.size(18.dp))
                }
            }
        }
    }
}

/** "In ‹name›" under a search result whose conversation sits in a Meron folder.
 *  Same size as a tag chip; neutral colours so it never reads as a tag. */
@Composable
internal fun MeronFolderRowChip(
    name: String,
    modifier: Modifier = Modifier,
) {
    Text(
        tr("meronFolders.inFolder", mapOf("name" to name)),
        fontSize = 11.sp,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier =
            modifier
                .clip(RoundedCornerShape(6.dp))
                .background(MaterialTheme.colorScheme.surfaceVariant)
                .padding(horizontal = 6.dp, vertical = 2.dp),
    )
}
