// Renders the mobile board, column headers, cards
// and search controls. Each column keeps its own
// mailbox or search scope while the board is
// displayed.

package jp.nonbili.meron.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.FilterList
import androidx.compose.material.icons.filled.PersonAdd
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.ViewKanban
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.input.key.key
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.FolderSummary
import jp.nonbili.meron.shared.ThreadSummary

@Composable
internal fun MailSearchFilterBar(
    search: String,
    filter: FilterMode,
    onSearchChange: (String) -> Unit,
    onSearchSubmit: () -> Unit,
    onFilterChange: (FilterMode) -> Unit,
) {
    Column(
        Modifier.fillMaxWidth().background(MaterialTheme.colorScheme.surface).padding(horizontal = 12.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        OutlinedTextField(
            value = search,
            onValueChange = onSearchChange,
            modifier = Modifier.fillMaxWidth().height(44.dp),
            textStyle = MaterialTheme.typography.bodyMedium,
            placeholder = { Text(tr("mobile.mail.searchMail"), style = MaterialTheme.typography.bodyMedium) },
            leadingIcon = { Icon(Icons.Filled.Search, contentDescription = null, modifier = Modifier.size(19.dp)) },
            trailingIcon = {
                if (search.isNotBlank()) {
                    IconButton(
                        onClick = {
                            onSearchChange("")
                            onSearchSubmit()
                        },
                        modifier = Modifier.size(34.dp),
                    ) {
                        Icon(Icons.Filled.Close, contentDescription = tr("common.clearSearch"), modifier = Modifier.size(18.dp))
                    }
                }
            },
            singleLine = true,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            FilterMode.values().forEach { mode ->
                FilterChip(
                    selected = filter == mode,
                    onClick = { onFilterChange(mode) },
                    label = { Text(mode.label()) },
                )
            }
            Spacer(Modifier.weight(1f))
            TextButton(onClick = onSearchSubmit) {
                Text(tr("buttons.done"))
            }
        }
    }
}

@Composable
internal fun KanbanHeaderSearchField(
    search: String,
    searchScope: String,
    columns: List<KanbanColumnSpec>,
    accounts: List<AccountSummary>,
    foldersByAccount: Map<String, List<FolderSummary>>,
    onSearchChange: (String) -> Unit,
    onSearchScopeChange: (String) -> Unit,
    onSearchSubmit: () -> Unit,
    onOpenOptions: (() -> Unit)? = null,
    focusRequester: FocusRequester? = null,
) {
    val normalizedScope = searchScope.ifBlank { "all" }
    val scopedColumn = columns.firstOrNull { kanbanColumnKey(it) == normalizedScope }
    val clearScope = {
        onSearchScopeChange("all")
        onSearchSubmit()
    }
    HeaderSearchField(
        search = search,
        placeholder = tr("kanban.searchBoard"),
        onSearchChange = onSearchChange,
        onSearchSubmit = onSearchSubmit,
        onOpenOptions = onOpenOptions,
        focusRequester = focusRequester,
        // Backspace in an empty box takes the scope off, as with any token before a caret.
        onBackspaceEmpty = if (scopedColumn != null) clearScope else null,
        // A search narrowed to one column says which, as on desktop; clearing it searches them all.
        // It gives up its name for the query's room once typing starts.
        leading =
            scopedColumn?.let {
                { KanbanSearchScopeChip(it, accounts, foldersByAccount, showName = search.isEmpty(), onClear = clearScope) }
            },
    )
}

@Composable
private fun KanbanSearchScopeChip(
    column: KanbanColumnSpec,
    accounts: List<AccountSummary>,
    foldersByAccount: Map<String, List<FolderSummary>>,
    showName: Boolean,
    onClear: () -> Unit,
) {
    val accent = MaterialTheme.colorScheme.primary
    // The whole chip is its own remove button, so it needs no × beside the text's.
    Row(
        Modifier
            .height(32.dp)
            .widthIn(max = 160.dp)
            .clip(RoundedCornerShape(8.dp))
            .background(accent.copy(alpha = 0.1f))
            .clickable(onClickLabel = tr("kanban.searchScope.allColumns"), role = Role.Button, onClick = onClear)
            .padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        // Whose folder it is, since a board can hold the same folder of several accounts.
        Avatar(columnAvatarLabel(column, accounts), 16.dp, columnAvatarFallbackIcon(column, accounts))
        if (showName) {
            Text(
                searchScopeTitle(column, accounts, foldersByAccount),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                fontSize = 12.sp,
                fontWeight = FontWeight.SemiBold,
                color = accent,
                modifier = Modifier.weight(1f, fill = false),
            )
        }
    }
}

@Composable
internal fun KanbanScreen(
    modifier: Modifier,
    accounts: List<AccountSummary>,
    accountsLoading: Boolean,
    board: KanbanBoardSpec?,
    columns: Map<String, KanbanColumnState>,
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
    onRefreshColumn: (KanbanColumnSpec) -> Unit,
    onLoadMoreColumn: (KanbanColumnSpec) -> Unit,
    onMarkColumnAllRead: (KanbanColumnSpec) -> Unit,
    markingRead: Boolean,
    onEmptyColumnFolder: (KanbanColumnSpec, FolderSummary) -> Unit,
    onDeleteColumnFolder: (KanbanColumnSpec, FolderSummary) -> Unit,
    onRenameColumnFolder: (FolderSummary) -> Unit,
    onNewColumnFolder: (KanbanColumnSpec, AccountSummary) -> Unit,
    onRemoveColumn: (KanbanColumnSpec) -> Unit,
    onMoveColumn: (KanbanColumnSpec, Int) -> Unit,
    onSearchColumn: (KanbanColumnSpec) -> Unit,
    onSaveSearchColumn: (KanbanColumnSpec, String) -> Unit,
    /** Opens a saved search column's query for editing. */
    onEditSearchColumn: (KanbanColumnSpec) -> Unit,
    onRequestColumnFolders: (KanbanColumnSpec) -> Unit,
    onSwitchColumnFolder: (KanbanColumnSpec, String) -> Unit,
    onAddColumn: () -> Unit,
    showSenderImages: Boolean,
    kanbanColumnWidth: Dp,
) {
    if (accountsLoading) {
        LoadingState(tr("mobile.status.loadingAccounts"))
        return
    }
    if (accounts.isEmpty()) {
        EmptyState(
            icon = Icons.Filled.PersonAdd,
            title = tr("empty.welcomeTitle"),
            text = tr("empty.kanbanSetupText"),
            actionLabel = tr("accounts.actions.addAccount"),
            onAction = {},
        )
        return
    }
    val boardColumns = board?.columns.orEmpty()
    var minimizedColumns by remember(board?.id) { mutableStateOf(emptySet<String>()) }
    Box(modifier.background(MaterialTheme.colorScheme.background)) {
        // The picked board background, drawn as the thread view draws an
        // account's chat wallpaper; the columns are opaque cards on top of it.
        if (board != null && (board.wallpaperPresetId.isNotBlank() || board.wallpaperUrl.isNotBlank())) {
            ChatWallpaperBackground(
                presetId = board.wallpaperPresetId,
                customUrl = board.wallpaperUrl,
                modifier = Modifier.matchParentSize(),
            )
        }
        Column(Modifier.fillMaxSize()) {
            if (boardColumns.isEmpty()) {
                EmptyState(
                    icon = Icons.Filled.ViewKanban,
                    title = tr("empty.noColumns"),
                    text = tr("empty.noColumnsText"),
                    actionLabel = tr("kanban.actions.addColumn"),
                    onAction = onAddColumn,
                )
            } else {
                val boardRowState = rememberLazyListState()
                LazyRow(
                    Modifier.fillMaxSize().appScrollbar(boardRowState, Orientation.Horizontal),
                    state = boardRowState,
                    contentPadding = PaddingValues(start = 12.dp, top = 8.dp, end = 12.dp, bottom = 16.dp),
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    items(boardColumns, key = { kanbanColumnKey(it) }) { column ->
                        val key = kanbanColumnKey(column)
                        if (key in minimizedColumns) {
                            val state = columns[key] ?: KanbanColumnState()
                            KanbanMinimizedColumn(
                                column = column,
                                accounts = accounts,
                                foldersByAccount = foldersByAccount,
                                unread = kanbanColumnUnreadCount(column, state.unreadCount, state.threads),
                                onRestore = { minimizedColumns = minimizedColumns - key },
                            )
                        } else {
                            KanbanColumn(
                                column = column,
                                state = columns[key] ?: KanbanColumnState(),
                                accounts = accounts,
                                foldersByAccount = foldersByAccount,
                                filter = filter,
                                attachmentsOnly = attachmentsOnly,
                                search = search,
                                searchScope = searchScope,
                                onOpen = onOpen,
                                selectedThreadIds = selectedThreadIds,
                                selectionActive = selectionActive,
                                onToggleSelected = onToggleSelected,
                                onLongPress = onLongPress,
                                onToggleStar = onToggleStar,
                                onRefresh = { onRefreshColumn(column) },
                                onLoadMore = { onLoadMoreColumn(column) },
                                onMarkAllRead = { onMarkColumnAllRead(column) },
                                markingRead = markingRead,
                                onEmptyFolder = { folder -> onEmptyColumnFolder(column, folder) },
                                onDeleteFolder = { folder -> onDeleteColumnFolder(column, folder) },
                                onRenameFolder = onRenameColumnFolder,
                                onNewFolder = { account -> onNewColumnFolder(column, account) },
                                onRemove = { onRemoveColumn(column) },
                                onMoveLeft = { onMoveColumn(column, -1) },
                                onMoveRight = { onMoveColumn(column, 1) },
                                onMinimize = { minimizedColumns = minimizedColumns + key },
                                onSearch = { onSearchColumn(column) },
                                onSaveSearch = { query -> onSaveSearchColumn(column, query) },
                                onEditSearch = { onEditSearchColumn(column) },
                                // A saved search only collides with the same search on another folder.
                                otherColumnFolderIds =
                                    boardColumns
                                        .filter { it.accountId == column.accountId && it.folderId != column.folderId && it.query == column.query }
                                        .map { it.folderId }
                                        .toSet(),
                                onRequestFolders = { onRequestColumnFolders(column) },
                                onSwitchFolder = { folderId -> onSwitchColumnFolder(column, folderId) },
                                showSenderImages = showSenderImages,
                                kanbanColumnWidth = kanbanColumnWidth,
                            )
                        }
                    }
                    item(key = "kanban-add-column") {
                        KanbanAddColumnRail(onClick = onAddColumn)
                    }
                }
            }
        }
    }
}

@Composable
private fun KanbanAddColumnRail(onClick: () -> Unit) {
    Surface(
        onClick = onClick,
        modifier = Modifier.fillMaxHeight().width(48.dp),
        shape = RoundedCornerShape(12.dp),
        color = MaterialTheme.colorScheme.surface.copy(alpha = 0.72f),
        border =
            androidx.compose.foundation.BorderStroke(
                1.dp,
                MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.85f),
            ),
    ) {
        Box(contentAlignment = Alignment.Center) {
            Surface(
                shape = CircleShape,
                color = MaterialTheme.colorScheme.surface.copy(alpha = 0.9f),
                tonalElevation = 2.dp,
            ) {
                Box(Modifier.size(32.dp), contentAlignment = Alignment.Center) {
                    Icon(
                        Icons.Filled.Add,
                        contentDescription = tr("kanban.actions.addColumn"),
                        modifier = Modifier.size(18.dp),
                        tint = MaterialTheme.colorScheme.primary,
                    )
                }
            }
        }
    }
}

@Composable
internal fun FilterModeButton(
    value: FilterMode,
    onChange: (FilterMode) -> Unit,
) {
    var expanded by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { expanded = true }) {
            Icon(Icons.Filled.FilterList, contentDescription = tr("filters.label"))
        }
        DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
            FilterMode.values().forEach { mode ->
                DropdownMenuItem(
                    text = { Text(mode.label()) },
                    onClick = {
                        expanded = false
                        onChange(mode)
                    },
                )
            }
        }
    }
}
