// Renders the expanded and minimized forms of a board column.
// Both layouts use the same column identity, unread count and search criteria.
// The board owns selection and supplies callbacks for actions on each conversation.

package jp.nonbili.meron.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.key.key
import androidx.compose.ui.layout.layout
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.FolderSummary
import jp.nonbili.meron.shared.ThreadSummary

@Composable
internal fun KanbanColumn(
    column: KanbanColumnSpec,
    state: KanbanColumnState,
    accounts: List<AccountSummary>,
    foldersByAccount: Map<String, List<FolderSummary>>,
    filter: FilterMode,
    attachmentsOnly: Boolean,
    search: String,
    searchScope: String,
    onOpen: (ThreadSummary, KanbanColumnSpec) -> Unit,
    selectedThreadIds: Set<String>,
    selectionActive: Boolean,
    onToggleSelected: (ThreadSummary) -> Unit,
    onLongPress: (ThreadSummary) -> Unit,
    onToggleStar: (ThreadSummary) -> Unit,
    onRefresh: () -> Unit,
    onLoadMore: () -> Unit,
    onMarkAllRead: () -> Unit,
    markingRead: Boolean,
    onEmptyFolder: (FolderSummary) -> Unit,
    onDeleteFolder: (FolderSummary) -> Unit,
    onRenameFolder: (FolderSummary) -> Unit,
    onNewFolder: (AccountSummary) -> Unit,
    onRemove: () -> Unit,
    onMoveLeft: () -> Unit,
    onMoveRight: () -> Unit,
    onMinimize: () -> Unit,
    onSearch: () -> Unit,
    onSaveSearch: (String) -> Unit,
    onEditSearch: () -> Unit,
    otherColumnFolderIds: Set<String>,
    onRequestFolders: () -> Unit,
    onSwitchFolder: (String) -> Unit,
    showSenderImages: Boolean,
    kanbanColumnWidth: Dp,
) {
    val visibleThreads = kanbanColumnVisibleThreads(column, state.threads, filter, search, searchScope, attachmentsOnly)
    val canLoadMore = state.nextCursor.isNotBlank() || state.accountCursors.isNotEmpty() || state.heldThreads.isNotEmpty()
    val accountsById = remember(accounts) { accounts.associateBy { it.id } }
    val showAccountBadge = column.accountId == UNIFIED_ACCOUNT_ID
    Card(
        modifier =
            Modifier
                .width(kanbanColumnWidth)
                .fillMaxSize()
                .border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(8.dp)),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
        shape = RoundedCornerShape(8.dp),
    ) {
        Column(Modifier.fillMaxSize()) {
            KanbanColumnHeader(
                column = column,
                accounts = accounts,
                foldersByAccount = foldersByAccount,
                unread = kanbanColumnUnreadCount(column, state.unreadCount, state.threads),
                otherColumnFolderIds = otherColumnFolderIds,
                onRefresh = onRefresh,
                onMarkAllRead = onMarkAllRead,
                markingRead = markingRead,
                onEmptyFolder = onEmptyFolder,
                onDeleteFolder = onDeleteFolder,
                onRenameFolder = onRenameFolder,
                onNewFolder = onNewFolder,
                onRemove = onRemove,
                onMoveLeft = onMoveLeft,
                onMoveRight = onMoveRight,
                onMinimize = onMinimize,
                onSearch = onSearch,
                // Offered once the column is showing a search's results, with
                // the query that produced them.
                shownSearch = state.cursorView.query,
                onSaveSearch = onSaveSearch,
                onEditSearch = onEditSearch,
                onRequestFolders = onRequestFolders,
                onSwitchFolder = onSwitchFolder,
            )
            if (state.loading) {
                Box(Modifier.fillMaxWidth().padding(16.dp), contentAlignment = Alignment.Center) {
                    CircularProgressIndicator(Modifier.size(24.dp))
                }
            }
            if (state.error != null) {
                Text(
                    state.error,
                    color = MaterialTheme.colorScheme.error,
                    fontSize = 12.sp,
                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            if (!state.loading && visibleThreads.isEmpty()) {
                Column(
                    Modifier.fillMaxSize().padding(18.dp),
                    verticalArrangement = Arrangement.Center,
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text(
                        if (search.isBlank() && column.query.isBlank()) {
                            tr(
                                when (filter) {
                                    FilterMode.All -> "kanban.emptyAllColumn"
                                    FilterMode.Unread -> "kanban.emptyUnreadColumn"
                                    FilterMode.Starred -> "kanban.emptyStarredColumn"
                                },
                            )
                        } else {
                            tr("mobile.mail.noSearchMatches")
                        },
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        fontSize = 13.sp,
                    )
                    // A narrowed page can come back empty with older mail still
                    // to read, and an empty column has nothing to scroll.
                    if (canLoadMore) {
                        TextButton(onClick = onLoadMore, enabled = !state.loadingMore) {
                            Text(if (state.loadingMore) tr("common.loading") else tr("threads.actions.loadMore"))
                        }
                    }
                }
            } else {
                val columnListState = rememberLazyListState()
                // Read the list through rememberUpdatedState so the derivation
                // tracks the current page count; a keyless remember would freeze
                // the first list's size into the closure and near-bottom
                // detection would drift after reloads.
                val currentThreads by rememberUpdatedState(visibleThreads)
                val nearBottom by remember {
                    derivedStateOf {
                        val lastVisible =
                            columnListState.layoutInfo.visibleItemsInfo
                                .lastOrNull()
                                ?.index ?: 0
                        lastVisible >= currentThreads.size - 3
                    }
                }
                LaunchedEffect(nearBottom, canLoadMore, state.loadingMore) {
                    if (nearBottom && canLoadMore && !state.loadingMore) onLoadMore()
                }
                LazyColumn(
                    Modifier.fillMaxSize().appScrollbar(columnListState),
                    state = columnListState,
                    contentPadding = PaddingValues(bottom = 8.dp),
                ) {
                    items(visibleThreads, key = { it.id }) { thread ->
                        MailRow(
                            thread = thread,
                            account = accountsById[thread.accountId].takeIf { showAccountBadge },
                            showSenderImages = showSenderImages,
                            selected = thread.id in selectedThreadIds,
                            selectionActive = selectionActive,
                            onOpen = {
                                if (selectionActive) {
                                    onToggleSelected(thread)
                                } else {
                                    onOpen(thread, column)
                                }
                            },
                            onLongPress = { onLongPress(thread) },
                            onToggleStar = { onToggleStar(thread) },
                        )
                        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.4f))
                    }
                    if (canLoadMore || state.loadingMore) {
                        item {
                            Box(Modifier.fillMaxWidth().padding(8.dp), contentAlignment = Alignment.Center) {
                                if (state.loadingMore) {
                                    CircularProgressIndicator(Modifier.size(22.dp))
                                } else {
                                    OutlinedButton(onClick = onLoadMore) {
                                        Text(tr("threads.actions.loadMore"))
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
internal fun KanbanMinimizedColumn(
    column: KanbanColumnSpec,
    accounts: List<AccountSummary>,
    foldersByAccount: Map<String, List<FolderSummary>>,
    unread: Int,
    onRestore: () -> Unit,
) {
    Card(
        modifier =
            Modifier
                .width(58.dp)
                .fillMaxSize()
                .clickable(onClick = onRestore)
                .border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(8.dp)),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
        shape = RoundedCornerShape(8.dp),
    ) {
        Column(
            Modifier.fillMaxSize().padding(horizontal = 6.dp, vertical = 8.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            KanbanColumnAvatar(column, accounts)
            val title = columnTitle(column, accounts, foldersByAccount)
            CollapsedColumnTitle(if (column.query.isBlank()) title else "$title · ${column.query}")
            if (unread > 0) {
                KanbanUnreadBadge(unread)
            }
        }
    }
}

private fun Modifier.rotate90(): Modifier =
    this.then(
        layout { measurable, constraints ->
            val childConstraints =
                constraints.copy(
                    minWidth = constraints.minHeight,
                    maxWidth = constraints.maxHeight,
                    minHeight = constraints.minWidth,
                    maxHeight = constraints.maxWidth,
                )
            val placeable = measurable.measure(childConstraints)
            layout(placeable.height, placeable.width) {
                placeable.placeWithLayer(
                    x = (placeable.height - placeable.width) / 2,
                    y = (placeable.width - placeable.height) / 2,
                ) {
                    rotationZ = 90f
                }
            }
        },
    )

@Composable
internal fun CollapsedColumnTitle(title: String) {
    Text(
        text = title,
        modifier =
            Modifier
                .rotate90()
                .heightIn(max = 180.dp),
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
        fontSize = 11.sp,
        lineHeight = 12.sp,
        fontWeight = FontWeight.SemiBold,
    )
}

@Composable
internal fun KanbanUnreadBadge(
    unread: Int,
    modifier: Modifier = Modifier,
) {
    val chat = LocalChatColors.current
    Box(
        modifier
            .heightIn(min = 18.dp)
            .widthIn(min = 18.dp)
            .clip(CircleShape)
            .background(chat.unreadBackground)
            .padding(horizontal = 5.dp, vertical = 2.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            unread.toString(),
            color = chat.unreadText,
            fontSize = 10.sp,
            lineHeight = 10.sp,
            fontWeight = FontWeight.Bold,
            maxLines = 1,
        )
    }
}

internal fun columnAvatarFallbackIcon(
    column: KanbanColumnSpec,
    accounts: List<AccountSummary>,
): ImageVector? {
    if (column.accountId == UNIFIED_ACCOUNT_ID) return null
    return accounts.firstOrNull { it.id == column.accountId }?.let(::accountAvatarFallbackIcon)
}

@Composable
internal fun columnAvatarLabel(
    column: KanbanColumnSpec,
    accounts: List<AccountSummary>,
): String {
    if (column.accountId == UNIFIED_ACCOUNT_ID) return unifiedColumnLabel(column.folderId)
    val account = accounts.firstOrNull { it.id == column.accountId }
    return account?.displayName?.ifBlank { account.email } ?: column.accountId
}
