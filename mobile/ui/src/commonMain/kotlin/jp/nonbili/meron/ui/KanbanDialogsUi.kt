// Shows the account, folder and thread actions available from a board.
// Column editing uses the same folder and saved-search choices as the board.
// Dialog actions return the selected operation to the board state.

package jp.nonbili.meron.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Inbox
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.RssFeed
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.key.key
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.FolderSummary
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.accountSummaryIsRss
import jp.nonbili.meron.shared.threadIdIsRss

@Composable
internal fun KanbanColumnDialog(
    accounts: List<AccountSummary>,
    board: KanbanBoardSpec?,
    foldersByAccount: Map<String, List<FolderSummary>>,
    onApply: (List<KanbanColumnSpec>) -> Unit,
    onCreateFolder: (AccountSummary) -> Unit,
    onDismiss: () -> Unit,
) {
    // Local multi-select state: tapping a row toggles it; nothing is applied to
    // the board until "Done". Keyed by kanbanColumnKey so order is preserved
    // for re-application.
    val selected =
        remember(board) {
            mutableStateMapOf<String, KanbanColumnSpec>().apply {
                board?.columns.orEmpty().forEach { put(kanbanColumnKey(it), it) }
            }
        }
    val collapsedAccounts = remember(accounts) { mutableStateMapOf<String, Boolean>() }

    fun toggle(column: KanbanColumnSpec) {
        val key = kanbanColumnKey(column)
        if (selected.remove(key) == null) selected[key] = column
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(tr("kanban.actions.addColumn")) },
        text = {
            val listState = rememberLazyListState()
            LazyColumn(
                Modifier.heightIn(max = 460.dp).appScrollbar(listState),
                state = listState,
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                item {
                    Text(
                        tr("kanban.addColumnsHint"),
                        fontSize = 12.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(bottom = 4.dp),
                    )
                }
                item {
                    val unified = KanbanColumnSpec(UNIFIED_ACCOUNT_ID, INBOX_FOLDER)
                    SidebarLikeDialogRow(
                        selected = selected.contains(kanbanColumnKey(unified)),
                        title = tr("kanban.columns.unifiedInbox"),
                        subtitle = null,
                        onClick = { toggle(unified) },
                        leadingIcon = Icons.Filled.Inbox,
                    )
                }
                accounts.forEach { account ->
                    val folders = foldersByAccount[account.id].orEmpty()
                    // Always surface folders that are already board columns, even if
                    // they haven't been fetched into foldersByAccount yet — otherwise
                    // existing columns can't be shown as selected or removed.
                    val boardFolders =
                        board
                            ?.columns
                            .orEmpty()
                            .filter { it.accountId == account.id }
                            .map { it.folderId }
                    // Board folders first so existing columns keep their stored folderId
                    // (folder name casing may differ from the fetched list, and the
                    // selection key is case-sensitive).
                    val visibleFolders =
                        buildList {
                            val seen = mutableSetOf<String>()
                            val known = folders.associateBy { it.name.lowercase() }
                            (boardFolders + folders.map { it.name }).forEach { name ->
                                if (seen.add(name.lowercase())) {
                                    add(known[name.lowercase()] ?: FolderSummary(account.id, name, 0))
                                }
                            }
                            if (isEmpty()) add(FolderSummary(account.id, INBOX_FOLDER, 0))
                        }
                    // Nested folders are indented under their parent, the same
                    // hierarchy the desktop column picker shows.
                    val folderRows = flattenFolderTree(buildFolderTree(visibleFolders))
                    item {
                        val collapsed = collapsedAccounts[account.id] == true
                        AccountHeaderDialogRow(
                            account = account,
                            collapsed = collapsed,
                            onClick = { collapsedAccounts[account.id] = !collapsed },
                        )
                    }
                    if (!accountSummaryIsRss(account) && collapsedAccounts[account.id] != true) {
                        item {
                            TextButton(onClick = { onCreateFolder(account) }) {
                                Icon(Icons.Filled.Add, contentDescription = null, modifier = Modifier.size(16.dp))
                                Spacer(Modifier.width(6.dp))
                                Text(tr("folders.create"))
                            }
                        }
                    }
                    if (collapsedAccounts[account.id] != true) {
                        itemsIndexed(
                            folderRows,
                            key = { index, row -> "${account.id}\n$index\n${row.node.folder?.name ?: row.node.name}" },
                        ) { _, row ->
                            val folder = row.node.folder
                            val column = folder?.let { KanbanColumnSpec(account.id, it.name) }
                            val isRss = accountSummaryIsRss(account)
                            SidebarLikeDialogRow(
                                selected = column != null && selected.contains(kanbanColumnKey(column)),
                                title = row.node.name.replaceFirstChar { it.uppercase() },
                                subtitle = null,
                                onClick = { column?.let(::toggle) },
                                indent = 24.dp + (row.depth * 16).dp,
                                // Structural nodes (no folder of their own) are labels only.
                                enabled = column != null,
                                leadingIcon =
                                    when {
                                        isRss -> Icons.Filled.RssFeed
                                        folder != null -> folderIcon(folder)
                                        else -> folderIcon(row.node.name)
                                    },
                            )
                        }
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { onApply(selected.values.toList()) }) { Text(tr("buttons.done")) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(tr("buttons.cancel")) }
        },
    )
}

@Composable
internal fun AccountHeaderDialogRow(
    account: AccountSummary,
    collapsed: Boolean,
    onClick: () -> Unit,
) {
    val label = account.displayName.ifBlank { account.email.ifBlank { account.id } }
    Row(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(8.dp))
            .clickable(onClick = onClick)
            .padding(horizontal = 4.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            if (collapsed) Icons.Filled.KeyboardArrowRight else Icons.Filled.KeyboardArrowDown,
            contentDescription = null,
            modifier = Modifier.size(18.dp),
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.width(6.dp))
        AccountBadgeAvatar(
            label = label,
            avatarUrl = account.avatarUrl,
            size = 22.dp,
            fallbackIcon = accountAvatarFallbackIcon(account),
        )
        Spacer(Modifier.width(9.dp))
        Text(
            label,
            fontWeight = FontWeight.SemiBold,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f),
        )
    }
}

@Composable
internal fun KanbanThreadActionDialog(
    thread: ThreadSummary,
    board: KanbanBoardSpec?,
    accounts: List<AccountSummary>,
    onDismiss: () -> Unit,
    onOpen: () -> Unit,
    onToggleStar: () -> Unit,
    onToggleRead: () -> Unit,
    onArchive: () -> Unit,
    onDelete: () -> Unit,
    onTags: () -> Unit,
    onAddTag: (String) -> Unit,
    onCopyFeedUrl: () -> Unit,
    onMove: (KanbanColumnSpec) -> Unit,
) {
    val deleteLabel = tr(threadDeleteActionKey(thread.folder, thread.folderRole))
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(thread.subject.ifBlank { tr("threads.noSubject") }, maxLines = 2, overflow = TextOverflow.Ellipsis) },
        text = {
            val listState = rememberLazyListState()
            LazyColumn(
                Modifier.heightIn(max = 430.dp).appScrollbar(listState),
                state = listState,
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                item { DialogAction(tr("kanban.actions.openThread"), onOpen) }
                item { DialogAction(if (thread.starred) tr("chat.unstar") else tr("chat.star"), onToggleStar) }
                item { DialogAction(if (thread.unread) tr("threads.actions.markAsRead") else tr("threads.actions.markAsUnread"), onToggleRead) }
                if (!threadIdIsRss(thread.id)) {
                    item { DialogAction(tr("tags.title"), onTags) }
                }
                if (threadIdIsRss(thread.id) && thread.feedUrl.isNotBlank()) {
                    item { DialogAction(tr("feeds.copyUrl"), onCopyFeedUrl) }
                }
                item { DialogAction(if (threadIdIsRss(thread.id)) tr("feeds.actions.deleteFeed") else tr("threads.actions.archiveThread"), onArchive) }
                if (!threadIdIsRss(thread.id)) {
                    item { DialogAction(deleteLabel, onDelete) }
                }
                val moveTargets =
                    board
                        ?.columns
                        .orEmpty()
                        // A saved search only lists matches; the thread might not be one.
                        .filter { it.accountId != UNIFIED_ACCOUNT_ID && it.query.isBlank() }
                        .filter { target ->
                            val targetAccount = accounts.firstOrNull { it.id == target.accountId }
                            targetAccount != null && (threadIdIsRss(thread.id) == accountSummaryIsRss(targetAccount))
                        }.filterNot { it.accountId == thread.accountId && it.folderId.equals(thread.folder, ignoreCase = true) }
                // A tag column is a state: joining it tags the thread where it lies.
                val tagTargets =
                    // Tag searches skip the trash, so that mail could not join one.
                    if (threadIdIsRss(thread.id) || thread.folderRole == "trash" || thread.folderRole == "junk") {
                        emptyList()
                    } else {
                        board
                            ?.columns
                            .orEmpty()
                            .filter { column ->
                                column.accountId == thread.accountId ||
                                    (column.accountId == UNIFIED_ACCOUNT_ID && accounts.firstOrNull { it.id == thread.accountId }?.includedInUnified == true)
                            }.mapNotNull(::kanbanColumnTag)
                            .distinct()
                            .filterNot { it in thread.tags }
                    }
                items(tagTargets, key = { "tag:$it" }) { tag -> DialogAction(tag) { onAddTag(tag) } }
                if (moveTargets.isNotEmpty()) {
                    item {
                        Text(
                            tr("threads.actions.moveTo"),
                            fontSize = 12.sp,
                            fontWeight = FontWeight.Bold,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(top = 10.dp),
                        )
                    }
                    items(moveTargets, key = { kanbanColumnKey(it) }) { target ->
                        val account = accounts.firstOrNull { it.id == target.accountId }
                        val targetLabel = listOf(account?.displayName?.ifBlank { account.email } ?: target.accountId, target.folderId).joinToString(" / ")
                        DialogAction(targetLabel) {
                            onMove(target)
                        }
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text(tr("buttons.close")) }
        },
    )
}

@Composable
internal fun DialogAction(
    label: String,
    onClick: () -> Unit,
) {
    TextButton(onClick = onClick, modifier = Modifier.fillMaxWidth()) {
        Text(label, modifier = Modifier.weight(1f))
    }
}

@Composable
internal fun SidebarLikeDialogRow(
    selected: Boolean,
    title: String,
    subtitle: String?,
    onClick: () -> Unit,
    leadingIcon: ImageVector? = null,
    indent: Dp = 0.dp,
    enabled: Boolean = true,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(8.dp))
            .background(if (selected) MaterialTheme.colorScheme.primary.copy(alpha = 0.12f) else Color.Transparent)
            .clickable(enabled = enabled, onClick = onClick)
            .padding(start = 10.dp + indent, end = 10.dp, top = 9.dp, bottom = 9.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (leadingIcon != null) {
            Icon(
                leadingIcon,
                contentDescription = null,
                modifier = Modifier.size(16.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.width(10.dp))
        }
        Column(Modifier.weight(1f)) {
            Text(
                title,
                fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Medium,
                color =
                    if (enabled) {
                        MaterialTheme.colorScheme.onSurface
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    },
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            if (subtitle != null) {
                Text(
                    subtitle,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    fontSize = 11.sp,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        if (selected) {
            Spacer(Modifier.width(10.dp))
            Icon(
                Icons.Filled.Check,
                contentDescription = null,
                modifier = Modifier.size(18.dp),
                tint = MaterialTheme.colorScheme.primary,
            )
        }
    }
}
