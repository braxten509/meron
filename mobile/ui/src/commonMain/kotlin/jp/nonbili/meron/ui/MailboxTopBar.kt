// Shows the mobile mailbox title, search field and folder menu.
// Selection, account state and the current query determine the available actions.
// The mailbox owns search focus and opens the drawer and filter sheet.

package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Archive
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.CreateNewFolder
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.DeleteForever
import androidx.compose.material.icons.filled.DriveFileRenameOutline
import androidx.compose.material.icons.filled.FolderDelete
import androidx.compose.material.icons.filled.MarkEmailUnread
import androidx.compose.material.icons.filled.Menu
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.NotificationsOff
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.SelectAll
import androidx.compose.material.icons.filled.Sell
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.filled.StarBorder
import androidx.compose.material.icons.outlined.FolderOpen
import androidx.compose.material.icons.outlined.NotificationsNone
import androidx.compose.material3.DrawerState
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.accountSummaryIsRss
import jp.nonbili.meron.shared.threadIdIsRss
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class, ExperimentalComposeUiApi::class)
@Composable
internal fun MailboxTopBar(
    state: MeronMobileState,
    drawerState: DrawerState,
    selectedAccount: AccountSummary?,
    selectedAccountIsRss: Boolean,
    mailSelectionActive: Boolean,
    selectedMailThreads: List<ThreadSummary>,
    importOpml: (PickedFile?) -> Unit,
    showMailSearch: Boolean,
    mailSearchFocus: FocusRequester,
    closeMailSearch: () -> Unit,
    openMailSearch: () -> Unit,
    openOptions: () -> Unit,
    appBarColors: androidx.compose.material3.TopAppBarColors,
) = with(state) {
    TopAppBar(
        colors = appBarColors,
        title = {
            if (mailSelectionActive) {
                MailSelectionTitle(selectedMailThreads.size, height = 44.dp)
            } else if (showMailSearch) {
                HeaderSearchField(
                    search = mailSearch,
                    placeholder = if (selectedAccountIsRss) tr("threads.searchFeeds") else tr("mobile.mail.searchMail"),
                    onSearchChange = { mailSearch = it },
                    onSearchSubmit = { syncCoreThreads(syncFirst = false) },
                    // Feed search is plain text; the operators are mail's.
                    onOpenOptions = if (selectedAccountIsRss) null else openOptions,
                    focusRequester = mailSearchFocus,
                )
            } else {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    // An RSS account has no folders to choose between.
                    // The unified view picks by role instead of by
                    // mailbox name — each entry means "every account's
                    // own Sent/Archive/…".
                    val unifiedMailbox = selectedCoreAccountId == UNIFIED_ACCOUNT_ID
                    if (inMeronFolderView) {
                        Text(
                            meronFolderName(openMeronFolder?.id).orEmpty(),
                            fontSize = 18.sp,
                            fontWeight = FontWeight.SemiBold,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    } else if (!selectedAccountIsRss) {
                        FolderSwitcher(
                            label =
                                folderTitle(
                                    selectedCoreAccountId,
                                    selectedCoreFolder,
                                    coreAccounts,
                                    foldersByAccount,
                                ),
                            folders =
                                if (unifiedMailbox) {
                                    unifiedFolders()
                                } else {
                                    foldersByAccount[selectedCoreAccountId].orEmpty()
                                },
                            currentFolderId = selectedCoreFolder,
                            onRequestFolders = {
                                if (!unifiedMailbox) ensureAccountFolders(selectedCoreAccountId)
                            },
                            onSelectFolder = { folder ->
                                selectCoreMailbox(selectedCoreAccountId, folder)
                                syncCoreThreads(
                                    accountOverride = selectedCoreAccountId,
                                    folderOverride = folder,
                                    syncFirst = false,
                                )
                            },
                            fontSize = 18.sp,
                            onSetFolderNotify =
                                if (unifiedMailbox) {
                                    null
                                } else {
                                    { folder, enabled ->
                                        setMailFolderNotify(
                                            accountId = folder.accountId,
                                            folderId = folder.name,
                                            folderName = folder.displayName,
                                            enabled = enabled,
                                        )
                                    }
                                },
                        )
                    }
                    if (selectedAccountIsRss) {
                        Text(
                            selectedAccount?.let { it.displayName.ifBlank { it.email } }.orEmpty(),
                            fontSize = 18.sp,
                            fontWeight = FontWeight.SemiBold,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
            }
        },
        navigationIcon = {
            if (mailSelectionActive) {
                IconButton(onClick = {
                    selectedMailThreadIds = emptySet()
                    mailSelectionMenuOpen = false
                }) {
                    Icon(Icons.Filled.Close, contentDescription = tr("mobile.actions.clearSelection"))
                }
            } else if (showMailSearch) {
                IconButton(onClick = closeMailSearch) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = tr("buttons.back"))
                }
            } else {
                IconButton(onClick = { scope.launch { drawerState.open() } }) {
                    Icon(Icons.Filled.Menu, contentDescription = tr("mobile.actions.openNavigation"))
                }
            }
        },
        actions = {
            if (mailSelectionActive) {
                // Conversations in the built-in Archive are archived already.
                if (!(inMeronFolderView && selectedCoreFolder == MERON_ARCHIVE_FOLDER_ID)) {
                    IconButton(onClick = {
                        archiveThreads(selectedMailThreads)
                        selectedMailThreadIds = emptySet()
                    }) {
                        Icon(Icons.Filled.Archive, contentDescription = tr("threads.actions.archiveThread"))
                    }
                }
                IconButton(onClick = {
                    deleteThreads(selectedMailThreads)
                    selectedMailThreadIds = emptySet()
                }) {
                    Icon(Icons.Filled.Delete, contentDescription = tr("buttons.delete"))
                }
                Box {
                    IconButton(onClick = { mailSelectionMenuOpen = true }) {
                        Icon(Icons.Filled.MoreVert, contentDescription = tr("chat.moreActions"))
                    }
                    DropdownMenu(
                        expanded = mailSelectionMenuOpen,
                        onDismissRequest = { mailSelectionMenuOpen = false },
                    ) {
                        // Sender rows carry no star or tags of their own: those
                        // stay on the single emails inside the chat.
                        val senderRowsSelected = selectedMailThreads.any { it.isSenderGroup }
                        val selectableThreadIds = coreThreads.map { it.id }.toSet()
                        if (!selectedMailThreadIds.containsAll(selectableThreadIds)) {
                            DropdownMenuItem(
                                text = { Text(tr("threads.actions.selectAll")) },
                                leadingIcon = { Icon(Icons.Filled.SelectAll, contentDescription = null) },
                                onClick = {
                                    mailSelectionMenuOpen = false
                                    selectedMailThreadIds = selectableThreadIds
                                },
                            )
                            HorizontalDivider(Modifier.padding(vertical = 4.dp))
                        }
                        val markRead = selectedMailThreads.any { it.unread }
                        DropdownMenuItem(
                            text = { Text(if (markRead) tr("threads.actions.markAsRead") else tr("threads.actions.markAsUnread")) },
                            leadingIcon = { Icon(Icons.Filled.MarkEmailUnread, contentDescription = null) },
                            onClick = {
                                mailSelectionMenuOpen = false
                                selectedMailThreads
                                    .filter { it.unread == markRead }
                                    .forEach(::toggleRead)
                                selectedMailThreadIds = emptySet()
                            },
                        )
                        if (!senderRowsSelected) {
                            val star = selectedMailThreads.any { !it.starred }
                            DropdownMenuItem(
                                text = { Text(if (star) tr("chat.star") else tr("chat.unstar")) },
                                leadingIcon = {
                                    Icon(
                                        if (star) Icons.Filled.StarBorder else Icons.Filled.Star,
                                        contentDescription = null,
                                    )
                                },
                                onClick = {
                                    mailSelectionMenuOpen = false
                                    selectedMailThreads
                                        .filter { it.starred != star }
                                        .forEach(::toggleStar)
                                    selectedMailThreadIds = emptySet()
                                },
                            )
                        }
                        val singleSelectedMailThread =
                            selectedMailThreads
                                .singleOrNull()
                                ?.takeUnless { threadIdIsRss(it.id) }
                        if (singleSelectedMailThread != null) {
                            HorizontalDivider(Modifier.padding(vertical = 4.dp))
                            if (!threadIdIsRss(singleSelectedMailThread.id) && !singleSelectedMailThread.isSenderGroup) {
                                DropdownMenuItem(
                                    text = { Text(tr("tags.title")) },
                                    leadingIcon = { Icon(Icons.Filled.Sell, contentDescription = null) },
                                    onClick = {
                                        mailSelectionMenuOpen = false
                                        selectedTagThread = singleSelectedMailThread
                                    },
                                )
                            }
                            DropdownMenuItem(
                                text = { Text(tr("threads.actions.moveTo")) },
                                leadingIcon = { Icon(Icons.Outlined.FolderOpen, contentDescription = null) },
                                onClick = {
                                    mailSelectionMenuOpen = false
                                    ensureThreadActionFolders(
                                        thread = singleSelectedMailThread,
                                        includeAllMailAccounts = true,
                                    ) {
                                        selectedMailMoveThread = singleSelectedMailThread
                                    }
                                },
                            )
                            DropdownMenuItem(
                                text = { Text(tr("threads.actions.copyTo")) },
                                leadingIcon = { Icon(Icons.Filled.ContentCopy, contentDescription = null) },
                                onClick = {
                                    mailSelectionMenuOpen = false
                                    ensureThreadActionFolders(
                                        thread = singleSelectedMailThread,
                                        includeAllMailAccounts = true,
                                    ) {
                                        selectedMailCopyThread = singleSelectedMailThread
                                    }
                                },
                            )
                        }
                        MeronFolderSelectionItems(
                            state = state,
                            threads = selectedMailThreads,
                            onDone = { mailSelectionMenuOpen = false },
                        )
                    }
                }
            } else if (!showMailSearch) {
                // An open search takes the whole bar, the folder's menu included.
                val loadedUnread = coreThreads.any { it.unread }
                val folderUnreadTotal =
                    if (inMeronFolderView) {
                        openMeronFolder?.unread ?: 0
                    } else if (selectedCoreAccountId == UNIFIED_ACCOUNT_ID) {
                        val includedAccountIds =
                            coreAccounts.filter { it.includedInUnified }.map { it.id }.toSet()
                        coreFolders
                            .filter { it.accountId in includedAccountIds }
                            .groupBy { it.accountId }
                            .values
                            .sumOf { accountFolders -> folderUnread(accountFolders, INBOX_FOLDER) }
                    } else {
                        folderUnread(
                            coreFolders.filter { it.accountId == selectedCoreAccountId },
                            selectedCoreFolder,
                        )
                    }
                val showMarkAllRead = folderUnreadTotal > 0 || loadedUnread
                val showAccountActions = selectedAccount != null
                // Emptying is offered only in a per-account Trash or Junk
                // folder — never the unified inbox, which spans accounts.
                val emptiableFolder =
                    if (selectedAccountIsRss || selectedCoreAccountId == UNIFIED_ACCOUNT_ID) {
                        null
                    } else {
                        coreFolders.firstOrNull { folder ->
                            folder.accountId == selectedCoreAccountId &&
                                folder.name == selectedCoreFolder &&
                                (folder.role == "trash" || folder.role == "junk")
                        }
                    }
                // Deleting the folder itself is offered only for an
                // ordinary folder; anything nested under it goes too.
                val deletableMailboxFolder =
                    if (selectedAccountIsRss) {
                        null
                    } else {
                        deletableFolder(coreFolders, selectedCoreAccountId, selectedCoreFolder)
                    }
                val notifiableMailboxFolder =
                    if (selectedAccountIsRss) {
                        null
                    } else {
                        notifiableFolder(coreFolders, selectedCoreAccountId, selectedCoreFolder)
                    }
                // Any single mail account can gain a folder; feeds and
                // the unified inbox cannot.
                val newFolderAccount =
                    selectedAccount?.takeIf {
                        !selectedAccountIsRss && selectedCoreAccountId != UNIFIED_ACCOUNT_ID
                    }
                // Renaming follows the delete rule, and moves the
                // subfolders along too.
                val showFolderActions = newFolderAccount != null || deletableMailboxFolder != null
                IconButton(onClick = {
                    openMailSearch()
                }) {
                    Icon(
                        Icons.Filled.Search,
                        contentDescription = if (selectedAccountIsRss) tr("threads.searchFeeds") else tr("mobile.mail.searchMail"),
                    )
                }
                Box {
                    IconButton(onClick = { mailboxMenuOpen = true }) {
                        Icon(
                            Icons.Filled.MoreVert,
                            contentDescription =
                                if (selectedAccountIsRss) {
                                    tr("feeds.manageSubscription")
                                } else {
                                    tr("threads.actions.title")
                                },
                        )
                    }
                    DropdownMenu(
                        expanded = mailboxMenuOpen,
                        onDismissRequest = { mailboxMenuOpen = false },
                        modifier = Modifier.width(260.dp),
                    ) {
                        FilterModeSegmentedControl(
                            filter = mailFilter,
                            onFilterChange = { mode ->
                                mailFilter = mode
                                syncCoreThreads(syncFirst = false)
                            },
                            modifier =
                                Modifier
                                    .fillMaxWidth()
                                    .padding(horizontal = 12.dp, vertical = 8.dp),
                        )
                        AttachmentsOnlyMenuItem(
                            checked = mailAttachmentsOnly,
                            onCheckedChange = { on ->
                                mailAttachmentsOnly = on
                                syncCoreThreads(syncFirst = false)
                            },
                        )
                        // Starred lists starred items, never people; a
                        // Meron folder lists conversations.
                        if (
                            !(selectedCoreAccountId == UNIFIED_ACCOUNT_ID && isUnifiedStarredFolder(selectedCoreFolder)) &&
                            !inMeronFolderView
                        ) {
                            GroupBySenderMenuItem(
                                checked = mailGroupBySender,
                                onCheckedChange = { on ->
                                    mailGroupBySender = on
                                    saveAppBoolean(prefs, GROUP_BY_SENDER_PREF, on)
                                    // A selection of thread rows means nothing
                                    // among sender rows, and the other way round.
                                    selectedMailThreadIds = emptySet()
                                    syncCoreThreads(syncFirst = false)
                                },
                            )
                        }
                        if (showMarkAllRead || showAccountActions) {
                            HorizontalDivider(Modifier.padding(vertical = 4.dp))
                        }
                        if (showMarkAllRead) {
                            DropdownMenuItem(
                                text = {
                                    Text(tr(if (inMeronFolderView) "meronFolders.markAllRead" else "threads.actions.markAllAsRead"))
                                },
                                leadingIcon = {
                                    Icon(Icons.Filled.MarkEmailUnread, contentDescription = null)
                                },
                                onClick = {
                                    mailboxMenuOpen = false
                                    if (inMeronFolderView) {
                                        markMeronFolderAllRead(selectedCoreFolder)
                                    } else {
                                        markVisibleMailboxAllRead()
                                    }
                                },
                            )
                        }
                        emptiableFolder?.let { folder ->
                            DropdownMenuItem(
                                text = {
                                    Text(
                                        if (folder.role == "junk") {
                                            tr("threads.actions.emptyJunk")
                                        } else {
                                            tr("threads.actions.emptyTrash")
                                        },
                                        color = MaterialTheme.colorScheme.error,
                                    )
                                },
                                leadingIcon = {
                                    Icon(
                                        Icons.Filled.DeleteForever,
                                        contentDescription = null,
                                        tint = MaterialTheme.colorScheme.error,
                                    )
                                },
                                onClick = {
                                    mailboxMenuOpen = false
                                    pendingEmptyFolder =
                                        EmptyFolderTarget(
                                            accountId = folder.accountId,
                                            folderId = folder.name,
                                            folderName = folder.displayName,
                                            role = folder.role,
                                        )
                                },
                            )
                        }
                        notifiableMailboxFolder?.let { folder ->
                            DropdownMenuItem(
                                text = {
                                    Text(
                                        tr(
                                            if (folder.notify) {
                                                "folders.notify.disable"
                                            } else {
                                                "folders.notify.enable"
                                            },
                                        ),
                                    )
                                },
                                leadingIcon = {
                                    Icon(
                                        if (folder.notify) {
                                            Icons.Filled.NotificationsOff
                                        } else {
                                            Icons.Outlined.NotificationsNone
                                        },
                                        contentDescription = null,
                                    )
                                },
                                onClick = {
                                    mailboxMenuOpen = false
                                    setMailFolderNotify(
                                        accountId = folder.accountId,
                                        folderId = folder.name,
                                        folderName = folder.displayName,
                                        enabled = !folder.notify,
                                    )
                                },
                            )
                        }
                        // Changes to the folder itself, the one destructive item last.
                        if (showFolderActions) {
                            HorizontalDivider(Modifier.padding(vertical = 4.dp))
                        }
                        newFolderAccount?.let { account ->
                            DropdownMenuItem(
                                text = { Text(tr("folders.newFolderOrLabel")) },
                                leadingIcon = {
                                    Icon(Icons.Filled.CreateNewFolder, contentDescription = null)
                                },
                                onClick = {
                                    mailboxMenuOpen = false
                                    promptNewFolder(
                                        account,
                                        NewFolderPlacement.OpenMailbox(selectedCoreFolder),
                                    )
                                },
                            )
                        }
                        deletableMailboxFolder?.let { folder ->
                            DropdownMenuItem(
                                text = { Text(tr("folders.rename.action")) },
                                leadingIcon = {
                                    Icon(Icons.Filled.DriveFileRenameOutline, contentDescription = null)
                                },
                                onClick = {
                                    mailboxMenuOpen = false
                                    promptRenameFolder(folder)
                                },
                            )
                        }
                        deletableMailboxFolder?.let { folder ->
                            DropdownMenuItem(
                                text = {
                                    Text(
                                        tr("folders.delete.action"),
                                        color = MaterialTheme.colorScheme.error,
                                    )
                                },
                                leadingIcon = {
                                    Icon(
                                        Icons.Filled.FolderDelete,
                                        contentDescription = null,
                                        tint = MaterialTheme.colorScheme.error,
                                    )
                                },
                                onClick = {
                                    mailboxMenuOpen = false
                                    pendingDeleteFolder =
                                        DeleteFolderTarget(
                                            accountId = folder.accountId,
                                            folderId = folder.name,
                                            folderName = folder.displayName,
                                            nested =
                                                nestedFolders(
                                                    coreFolders,
                                                    folder.accountId,
                                                    folder.name,
                                                ).size,
                                        )
                                },
                            )
                        }
                        if (showAccountActions) {
                            if (showMarkAllRead || showFolderActions) {
                                HorizontalDivider(Modifier.padding(vertical = 4.dp))
                            }
                            DropdownMenuItem(
                                text = { Text(tr("settings.account.accountSettings")) },
                                leadingIcon = {
                                    Icon(Icons.Filled.Settings, contentDescription = null)
                                },
                                onClick = {
                                    mailboxMenuOpen = false
                                    accountSettingsTargetId = selectedAccount.id
                                    previousTopScreen = Screen.Mail
                                    screen = Screen.Settings
                                },
                            )
                            if (selectedAccount.let(::accountSummaryIsRss)) {
                                HorizontalDivider(Modifier.padding(vertical = 4.dp))
                                DropdownMenuItem(
                                    text = { Text(tr("feeds.actions.addFeed")) },
                                    onClick = {
                                        mailboxMenuOpen = false
                                        addFeedUrl = ""
                                        addFeedError = ""
                                        showAddFeedDialog = true
                                    },
                                )
                                DropdownMenuItem(
                                    text = { Text(tr("common.import")) },
                                    onClick = {
                                        mailboxMenuOpen = false
                                        services.pickFile(
                                            listOf("text/xml", "application/xml", "text/*", "*/*"),
                                            importOpml,
                                        )
                                    },
                                )
                                DropdownMenuItem(
                                    text = { Text(tr("common.export")) },
                                    onClick = {
                                        mailboxMenuOpen = false
                                        exportOpmlForSelectedAccount()
                                    },
                                )
                            }
                        }
                    }
                }
            }
        },
    )
}
