// Displays a board column title, avatar and saved search.
// The expanded column supplies its account and current search parameters.
// Header actions let the board rename or change the column and its filter.

package jp.nonbili.meron.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Archive
import androidx.compose.material.icons.filled.BookmarkAdd
import androidx.compose.material.icons.filled.CreateNewFolder
import androidx.compose.material.icons.filled.DeleteForever
import androidx.compose.material.icons.filled.DoneAll
import androidx.compose.material.icons.filled.DriveFileRenameOutline
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.FolderDelete
import androidx.compose.material.icons.filled.KeyboardArrowLeft
import androidx.compose.material.icons.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Remove
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.FolderSummary
import jp.nonbili.meron.shared.accountSummaryIsRss

@Composable
internal fun KanbanColumnHeader(
    column: KanbanColumnSpec,
    accounts: List<AccountSummary>,
    foldersByAccount: Map<String, List<FolderSummary>>,
    unread: Int,
    /** Folders of this account already open as their own column on the board. */
    otherColumnFolderIds: Set<String>,
    onRefresh: () -> Unit,
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
    shownSearch: String,
    onSaveSearch: (String) -> Unit,
    onEditSearch: () -> Unit,
    onRequestFolders: () -> Unit,
    onSwitchFolder: (String) -> Unit,
) {
    var menuOpen by remember { mutableStateOf(false) }
    val savedSearch = column.query.isNotBlank()
    val account = accounts.firstOrNull { it.id == column.accountId }
    // A unified column is repointed by role — each account answers from its own
    // Sent/Archive/… — so it offers the fixed role list rather than any one
    // account's folders. An RSS account has no folders to choose between.
    val switchableFolders =
        when {
            column.accountId == UNIFIED_ACCOUNT_ID -> unifiedFolders()
            account == null || accountSummaryIsRss(account) -> emptyList()
            else -> foldersByAccount[column.accountId].orEmpty()
        }
    // Emptying is offered only for a per-account Trash or Junk column; a unified
    // column spans accounts and has no single folder to empty.
    // Nor a saved search, which shows only some of its folder.
    val emptiableFolder =
        if (column.accountId == UNIFIED_ACCOUNT_ID || savedSearch) {
            null
        } else {
            foldersByAccount[column.accountId]?.firstOrNull { folder ->
                folder.name == column.folderId && (folder.role == "trash" || folder.role == "junk")
            }
        }
    // Deleting the folder itself is offered only for an ordinary per-account
    // folder; anything nested under it goes too — see `deletableFolder`.
    val deletableColumnFolder =
        if (savedSearch) null else deletableFolder(foldersByAccount[column.accountId].orEmpty(), column.accountId, column.folderId)
    // Any single mail account can gain a folder, which the column then shows.
    val newFolderAccount = account?.takeIf { column.accountId != UNIFIED_ACCOUNT_ID && !accountSummaryIsRss(it) }
    Row(
        Modifier
            .fillMaxWidth()
            .height(48.dp)
            .padding(start = 12.dp, end = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        KanbanColumnAvatar(column, accounts)
        Row(
            Modifier.weight(1f),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            val canSwitchFolder =
                column.accountId == UNIFIED_ACCOUNT_ID || (account != null && !accountSummaryIsRss(account))
            FolderSwitcher(
                label = columnTitle(column, accounts, foldersByAccount),
                folders = switchableFolders,
                currentFolderId = column.folderId,
                onRequestFolders = onRequestFolders,
                onSelectFolder = onSwitchFolder,
                modifier = Modifier.weight(1f, fill = false),
                enabled = canSwitchFolder,
                takenFolderIds = otherColumnFolderIds,
            )
            if (unread > 0) {
                KanbanUnreadBadge(unread)
            }
        }
        IconButton(onClick = onMinimize, modifier = Modifier.size(34.dp)) {
            Icon(Icons.Filled.Remove, contentDescription = tr("kanban.actions.minimizeColumn"), modifier = Modifier.size(18.dp))
        }
        Box {
            IconButton(onClick = { menuOpen = true }, modifier = Modifier.size(34.dp)) {
                Icon(Icons.Filled.MoreVert, contentDescription = tr("kanban.actions.columnActions"), modifier = Modifier.size(18.dp))
            }
            DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                DropdownMenuItem(text = { Text(tr("threads.actions.markAllAsRead")) }, leadingIcon = {
                    Icon(Icons.Filled.DoneAll, contentDescription = null)
                }, onClick = {
                    menuOpen = false
                    onMarkAllRead()
                }, enabled = unread > 0 && !markingRead)
                emptiableFolder?.let { folder ->
                    DropdownMenuItem(text = {
                        Text(
                            if (folder.role == "junk") tr("threads.actions.emptyJunk") else tr("threads.actions.emptyTrash"),
                            color = MaterialTheme.colorScheme.error,
                        )
                    }, leadingIcon = {
                        Icon(
                            Icons.Filled.DeleteForever,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.error,
                        )
                    }, onClick = {
                        menuOpen = false
                        onEmptyFolder(folder)
                    })
                }
                if (savedSearch) {
                    DropdownMenuItem(text = { Text(tr("kanban.actions.editSearch")) }, leadingIcon = {
                        Icon(Icons.Filled.Edit, contentDescription = null)
                    }, onClick = {
                        menuOpen = false
                        onEditSearch()
                    })
                }
                DropdownMenuItem(text = { Text(tr("kanban.actions.searchColumn")) }, leadingIcon = {
                    Icon(Icons.Filled.Search, contentDescription = null)
                }, onClick = {
                    menuOpen = false
                    onSearch()
                })
                // A saved search narrowed by the board search isn't saved again.
                if (shownSearch.isNotBlank() && !savedSearch) {
                    DropdownMenuItem(text = { Text(tr("kanban.actions.saveSearchAsColumn")) }, leadingIcon = {
                        Icon(Icons.Filled.BookmarkAdd, contentDescription = null)
                    }, onClick = {
                        menuOpen = false
                        onSaveSearch(shownSearch)
                    })
                }
                HorizontalDivider(Modifier.padding(vertical = 4.dp))
                DropdownMenuItem(text = { Text(tr("kanban.actions.moveColumnLeft")) }, leadingIcon = {
                    Icon(Icons.Filled.KeyboardArrowLeft, contentDescription = null)
                }, onClick = {
                    menuOpen = false
                    onMoveLeft()
                })
                DropdownMenuItem(text = { Text(tr("kanban.actions.moveColumnRight")) }, leadingIcon = {
                    Icon(Icons.Filled.KeyboardArrowRight, contentDescription = null)
                }, onClick = {
                    menuOpen = false
                    onMoveRight()
                })
                HorizontalDivider(Modifier.padding(vertical = 4.dp))
                DropdownMenuItem(text = { Text(tr("mobile.actions.refresh")) }, leadingIcon = {
                    Icon(Icons.Filled.Refresh, contentDescription = null)
                }, onClick = {
                    menuOpen = false
                    onRefresh()
                })
                DropdownMenuItem(text = { Text(tr("kanban.actions.hideColumn")) }, leadingIcon = {
                    Icon(Icons.Filled.VisibilityOff, contentDescription = null)
                }, onClick = {
                    menuOpen = false
                    onRemove()
                })
                // Changes to the folder itself, the one destructive item last.
                if (newFolderAccount != null || deletableColumnFolder != null) {
                    HorizontalDivider(Modifier.padding(vertical = 4.dp))
                }
                newFolderAccount?.let { account ->
                    DropdownMenuItem(text = { Text(tr("folders.newFolderOrLabel")) }, leadingIcon = {
                        Icon(Icons.Filled.CreateNewFolder, contentDescription = null)
                    }, onClick = {
                        menuOpen = false
                        onNewFolder(account)
                    })
                }
                // Renaming follows the delete rule.
                deletableColumnFolder?.let { folder ->
                    DropdownMenuItem(text = { Text(tr("folders.rename.action")) }, leadingIcon = {
                        Icon(Icons.Filled.DriveFileRenameOutline, contentDescription = null)
                    }, onClick = {
                        menuOpen = false
                        onRenameFolder(folder)
                    })
                }
                deletableColumnFolder?.let { folder ->
                    DropdownMenuItem(text = {
                        Text(tr("folders.delete.action"), color = MaterialTheme.colorScheme.error)
                    }, leadingIcon = {
                        Icon(
                            Icons.Filled.FolderDelete,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.error,
                        )
                    }, onClick = {
                        menuOpen = false
                        onDeleteFolder(folder)
                    })
                }
            }
        }
    }
    if (savedSearch) {
        // Its own row under the header: the query is what tells saved searches of
        // one folder apart, so it gets the column's width instead of squeezing
        // the folder name.
        SavedSearchChip(column.query, onClick = onEditSearch, modifier = Modifier.padding(start = 12.dp, end = 12.dp, bottom = 8.dp))
    }
    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
}

// A column's account avatar. A saved search marks it too, so it stands apart
// from the folder's plain column.
@Composable
internal fun KanbanColumnAvatar(
    column: KanbanColumnSpec,
    accounts: List<AccountSummary>,
) {
    Box {
        Avatar(columnAvatarLabel(column, accounts), 26.dp, columnAvatarFallbackIcon(column, accounts))
        if (column.query.isNotBlank()) {
            Box(
                Modifier
                    .align(Alignment.BottomEnd)
                    .offset(x = 4.dp, y = 4.dp)
                    .size(16.dp)
                    .clip(CircleShape)
                    .background(MaterialTheme.colorScheme.surface)
                    .padding(2.dp)
                    .clip(CircleShape)
                    .background(MaterialTheme.colorScheme.primary),
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Filled.Search, contentDescription = null, tint = MaterialTheme.colorScheme.onPrimary, modifier = Modifier.size(9.dp))
            }
        }
    }
}

// A saved search column's query under its folder name; tapping it edits it.
@Composable
private fun SavedSearchChip(
    query: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        onClick = onClick,
        modifier = modifier,
        shape = RoundedCornerShape(6.dp),
        color = MaterialTheme.colorScheme.primary.copy(alpha = 0.12f),
        contentColor = MaterialTheme.colorScheme.primary,
    ) {
        Row(
            Modifier.padding(horizontal = 6.dp, vertical = 3.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(3.dp),
        ) {
            Icon(Icons.Filled.Search, contentDescription = tr("kanban.actions.editSearch"), modifier = Modifier.size(12.dp))
            Text(query, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}
