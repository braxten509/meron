package jp.nonbili.meron.ui

import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.ExpandLess
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material.icons.filled.History
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.FolderSummary
import jp.nonbili.meron.shared.ThreadSummary

@Composable
internal fun MoveThreadDialog(
    thread: ThreadSummary,
    folders: List<FolderSummary>,
    accounts: List<AccountSummary> = emptyList(),
    recentTransfers: List<RecentThreadTransferItem> = emptyList(),
    onMove: (FolderSummary) -> Unit,
    onCreateAndMove: (String) -> Unit,
    onDismiss: () -> Unit,
    // A single email's move says "Move to..." rather than "Move conversation".
    titleKey: String = "threads.moveConversation",
) {
    ThreadDestinationDialog(
        thread,
        folders,
        accounts,
        recentTransfers.forAction("move"),
        titleKey,
        "folders.noOtherAvailable",
        onMove,
        onDismiss,
    )
}

@Composable
internal fun CopyThreadDialog(
    thread: ThreadSummary,
    folders: List<FolderSummary>,
    accounts: List<AccountSummary> = emptyList(),
    recentTransfers: List<RecentThreadTransferItem> = emptyList(),
    onCopy: (FolderSummary) -> Unit,
    onDismiss: () -> Unit,
) {
    ThreadDestinationDialog(
        thread,
        folders,
        accounts,
        recentTransfers.forAction("copy"),
        "threads.copyConversation",
        "folders.noCopyTargets",
        onCopy,
        onDismiss,
    )
}

private data class DestinationAccount(
    val id: String,
    val label: String,
    val avatarUrl: String = "",
)

/**
 * Recent targets of this action first, then one row per account (the thread's
 * own first) that expands to that account's folders. A lone account's folders
 * are listed directly.
 */
@Composable
private fun ThreadDestinationDialog(
    thread: ThreadSummary,
    folders: List<FolderSummary>,
    accounts: List<AccountSummary>,
    recents: List<RecentThreadTransferItem>,
    titleKey: String,
    emptyKey: String,
    onPick: (FolderSummary) -> Unit,
    onDismiss: () -> Unit,
) {
    val foldersByAccount = folders.groupBy { it.accountId.ifBlank { thread.accountId } }
    val accountGroups =
        if (accounts.isNotEmpty()) {
            accounts.map { account ->
                DestinationAccount(account.id, account.displayName.ifBlank { account.email.ifBlank { account.id } }, account.avatarUrl)
            }
        } else {
            foldersByAccount.keys.map { accountId -> DestinationAccount(accountId, accountId) }
        }.sortedBy { it.id != thread.accountId }

    fun targetFolders(accountId: String) =
        foldersByAccount[accountId]
            .orEmpty()
            .filterNot { folder ->
                accountId == thread.accountId &&
                    folder.name.equals(thread.folder, ignoreCase = true)
            }
    val inlineAccount = accountGroups.singleOrNull()
    val targetCount = accountGroups.sumOf { targetFolders(it.id).size }
    var expandedAccountId by remember { mutableStateOf<String?>(null) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(tr(titleKey)) },
        text = {
            val listState = rememberLazyListState()
            LazyColumn(
                Modifier.heightIn(max = 420.dp).appScrollbar(listState),
                state = listState,
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                item {
                    Text(
                        thread.subject.ifBlank { tr("threads.noSubject") },
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        style = MaterialTheme.typography.bodySmall,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                if (targetCount == 0 && recents.isEmpty()) {
                    item {
                        Text(
                            tr(emptyKey),
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            style = MaterialTheme.typography.bodyMedium,
                            modifier = Modifier.padding(vertical = 12.dp),
                        )
                    }
                    return@LazyColumn
                }
                items(recents, key = { "recent:${it.transfer.targetAccountId}\n${it.folder.name}" }) { recent ->
                    DestinationAction({ DestinationIcon(Icons.Filled.History) }, recent.destination) {
                        onPick(recent.folder.copy(accountId = recent.transfer.targetAccountId))
                    }
                }
                if (recents.isNotEmpty() && accountGroups.isNotEmpty()) {
                    item(key = "recent-divider") { HorizontalDivider(Modifier.padding(vertical = 4.dp)) }
                }
                if (inlineAccount != null) {
                    val accountFolders = targetFolders(inlineAccount.id)
                    if (accountFolders.isEmpty()) {
                        item(key = "empty:${inlineAccount.id}") { NoFoldersText() }
                    } else {
                        items(accountFolders, key = { "${inlineAccount.id}\n${it.name}" }) { folder ->
                            FolderDestinationAction(folder) { onPick(folder) }
                        }
                    }
                    return@LazyColumn
                }
                accountGroups.forEach { account ->
                    val expanded = expandedAccountId == account.id
                    item(key = "account:${account.id}") {
                        DestinationAction(
                            { AccountBadgeAvatar(account.label, account.avatarUrl, 20.dp) },
                            account.label,
                            trailing = if (expanded) Icons.Filled.ExpandLess else Icons.Filled.ExpandMore,
                        ) {
                            expandedAccountId = if (expanded) null else account.id
                        }
                    }
                    if (expanded) {
                        val accountFolders = targetFolders(account.id)
                        if (accountFolders.isEmpty()) {
                            item(key = "empty:${account.id}") { NoFoldersText(Modifier.padding(start = 16.dp)) }
                        } else {
                            items(accountFolders, key = { "${account.id}\n${it.name}" }) { folder ->
                                FolderDestinationAction(folder, Modifier.padding(start = 16.dp)) { onPick(folder) }
                            }
                        }
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text(tr("buttons.cancel")) }
        },
    )
}

@Composable
private fun NoFoldersText(modifier: Modifier = Modifier) {
    Text(
        tr("folders.noneAvailable"),
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        style = MaterialTheme.typography.bodySmall,
        modifier = modifier.padding(horizontal = 8.dp, vertical = 6.dp),
    )
}

@Composable
internal fun FolderDestinationAction(
    folder: FolderSummary,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    DestinationAction({ DestinationIcon(folderIcon(folder)) }, folder.displayName.replaceFirstChar { it.uppercase() }, modifier, onClick = onClick)
}

@Composable
private fun DestinationIcon(icon: ImageVector) {
    Icon(
        icon,
        contentDescription = null,
        modifier = Modifier.size(18.dp),
        tint = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

@Composable
private fun DestinationAction(
    leading: @Composable () -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    trailing: ImageVector? = null,
    onClick: () -> Unit,
) {
    TextButton(onClick = onClick, modifier = modifier.fillMaxWidth()) {
        leading()
        Spacer(Modifier.width(10.dp))
        Text(
            label,
            modifier = Modifier.weight(1f),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        if (trailing != null) {
            Icon(
                trailing,
                contentDescription = null,
                modifier = Modifier.size(18.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/** "Move to Meron folder": the folders A to Z (the conversations' own left
 *  out when they all share it), then "New Meron folder". */
@Composable
internal fun MeronFolderPickerDialog(
    threads: List<ThreadSummary>,
    folders: List<jp.nonbili.meron.shared.MeronFolderSummary>,
    onPick: (jp.nonbili.meron.shared.MeronFolderSummary) -> Unit,
    onNewFolder: () -> Unit,
    onDismiss: () -> Unit,
) {
    val shared = threads.map { it.meronFolderId }.distinct().singleOrNull()
    val targets = folders.filterNot { it.id == shared }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(tr("meronFolders.moveTo")) },
        text = {
            val listState = rememberLazyListState()
            LazyColumn(
                Modifier.heightIn(max = 420.dp).appScrollbar(listState),
                state = listState,
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                threads.singleOrNull()?.let { thread ->
                    item(key = "subject") {
                        Text(
                            // A sender row moves the person's mail, not its newest subject.
                            if (thread.isSenderGroup) {
                                thread.sender.ifBlank { thread.senderAddress }
                            } else {
                                thread.subject.ifBlank { tr("threads.noSubject") }
                            },
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            style = MaterialTheme.typography.bodySmall,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
                if (targets.isEmpty()) {
                    item(key = "none") {
                        Text(
                            tr("meronFolders.none"),
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            style = MaterialTheme.typography.bodyMedium,
                            modifier = Modifier.padding(vertical = 8.dp),
                        )
                    }
                }
                items(targets, key = { "meron:${it.id}" }) { folder ->
                    DestinationAction({ DestinationIcon(Icons.Outlined.Folder) }, folder.name) { onPick(folder) }
                }
                item(key = "new-divider") { HorizontalDivider(Modifier.padding(vertical = 4.dp)) }
                item(key = "new") {
                    DestinationAction({ DestinationIcon(Icons.Filled.Add) }, tr("meronFolders.new"), onClick = onNewFolder)
                }
            }
        },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text(tr("buttons.cancel")) }
        },
    )
}

/** Name a new Meron folder, or rename one. [error] shows under the field. */
@Composable
internal fun MeronFolderNameDialog(
    title: String,
    confirmLabel: String,
    name: String,
    error: String,
    busy: Boolean,
    onNameChange: (String) -> Unit,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    val canConfirm = name.isNotBlank() && !busy
    // Opens ready to type, with a rename's old name selected so typing replaces
    // it instead of landing in front of it.
    var field by remember { mutableStateOf(TextFieldValue(name, TextRange(0, name.length))) }
    if (field.text != name) field = TextFieldValue(name, TextRange(name.length))
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            OutlinedTextField(
                value = field,
                onValueChange = {
                    field = it
                    if (it.text != name) onNameChange(it.text)
                },
                label = { Text(tr("meronFolders.nameLabel")) },
                // The same line holds the hint or the refusal, so the dialog keeps its height.
                supportingText = { Text(error.ifBlank { tr("meronFolders.thisDeviceOnly") }) },
                isError = error.isNotBlank(),
                singleLine = true,
                keyboardOptions = nativeTextKeyboardOptions.copy(imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { if (canConfirm) onConfirm() }),
                modifier = Modifier.fillMaxWidth().focusRequester(focus),
            )
        },
        confirmButton = {
            TextButton(onClick = onConfirm, enabled = canConfirm) { Text(confirmLabel) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(tr("buttons.cancel")) }
        },
    )
}

/** Asked before a Meron folder goes; its conversations go back. */
@Composable
internal fun DeleteMeronFolderDialog(
    folderName: String,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(tr("meronFolders.delete")) },
        text = { Text(tr("meronFolders.deleteConfirm", mapOf("name" to folderName))) },
        confirmButton = {
            TextButton(onClick = onConfirm) {
                Text(tr("buttons.delete"), color = MaterialTheme.colorScheme.error)
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(tr("buttons.cancel")) }
        },
    )
}
