// Connects mobile navigation destinations to their
// screens and actions. Each route receives the state and
// callbacks needed for the currently selected mailbox or
// feature.

package jp.nonbili.meron.ui

import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Archive
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.CreateNewFolder
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.DeleteForever
import androidx.compose.material.icons.filled.DriveFileRenameOutline
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.FolderDelete
import androidx.compose.material.icons.filled.MarkEmailUnread
import androidx.compose.material.icons.filled.Menu
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.NotificationsOff
import androidx.compose.material.icons.filled.PersonAdd
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.SelectAll
import androidx.compose.material.icons.filled.Sell
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.filled.StarBorder
import androidx.compose.material.icons.outlined.Drafts
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.FolderOff
import androidx.compose.material.icons.outlined.FolderOpen
import androidx.compose.material.icons.outlined.NotificationsNone
import androidx.compose.material3.DrawerState
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.backhandler.BackHandler
import androidx.compose.ui.focus.FocusRequester
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
import jp.nonbili.meron.shared.untrustedCertificateProtocol
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

// Thread-list scroll positions kept across mailbox switches. Comfortably covers
// the unified inbox plus a few accounts' folders; beyond that the oldest are
// dropped rather than held for the lifetime of the process.
private const val MAX_REMEMBERED_MAIL_LIST_STATES = 12

// Contacts can be restored as the root. Its history return target is Contacts,
// not the destination to leave the directory for when no earlier route exists.
internal fun rootBackDestination(
    screen: Screen,
    threadReturnScreen: Screen,
): Screen = if (screen == Screen.Contacts) Screen.Mail else threadReturnScreen

internal fun rememberedMailListKeysToEvict(
    existingKeys: List<MailboxCacheKey>,
    requestedKey: MailboxCacheKey,
    maxSize: Int,
): List<MailboxCacheKey> {
    if (requestedKey in existingKeys || existingKeys.size < maxSize) return emptyList()
    return existingKeys.take(existingKeys.size - maxSize + 1)
}

@Composable
internal fun MailSelectionTitle(
    selectedCount: Int,
    height: Dp,
) {
    Box(
        modifier = Modifier.fillMaxWidth().height(height),
        contentAlignment = Alignment.CenterStart,
    ) {
        Text(
            "$selectedCount selected",
            fontWeight = FontWeight.SemiBold,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** "Move to Meron folder" and "Move out" in a selection menu; hidden when any
 *  selected row is a feed or local mail. A sender row moves every conversation
 *  of that person in the mailbox; it offers no "Move out", since what it
 *  shows is never in a Meron folder. */
@Composable
internal fun MeronFolderSelectionItems(
    state: MeronMobileState,
    threads: List<ThreadSummary>,
    onDone: () -> Unit,
) {
    if (threads.isEmpty() || !threads.all(state::canUseMeronFolders)) return
    HorizontalDivider(Modifier.padding(vertical = 4.dp))
    DropdownMenuItem(
        text = { Text(tr("meronFolders.moveTo")) },
        leadingIcon = { Icon(Icons.Outlined.Folder, contentDescription = null) },
        onClick = {
            onDone()
            state.meronMoveThreads = threads
        },
    )
    if (threads.none { it.isSenderGroup } && (state.inMeronFolderView || threads.any { it.meronFolderId != null })) {
        // Out of the built-in Archive is back to the Inbox (un-archived on the server).
        val outOfArchive =
            threads.all { (it.meronFolderId ?: state.selectedCoreFolder.takeIf { state.inMeronFolderView }) == MERON_ARCHIVE_FOLDER_ID }
        DropdownMenuItem(
            text = { Text(tr(if (outOfArchive) "meronFolders.moveToInbox" else "meronFolders.moveOut")) },
            leadingIcon = { Icon(Icons.Outlined.FolderOff, contentDescription = null) },
            onClick = {
                onDone()
                state.moveOutOfMeronFolder(threads)
                state.selectedMailThreadIds = emptySet()
            },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalComposeUiApi::class)
@Composable
internal fun MailRouteContent(
    state: MeronMobileState,
    drawerState: DrawerState,
    drawerFolders: List<FolderSummary>,
    selectedAccount: AccountSummary?,
    selectedAccountIsRss: Boolean,
    mailSelectionActive: Boolean,
    selectedMailThreads: List<ThreadSummary>,
    importOpml: (PickedFile?) -> Unit,
    mailListStates: MutableMap<MailboxCacheKey, MailListPosition>,
) {
    with(state) {
        var searchOptionsOpen by remember { mutableStateOf(false) }
        // Search folds to an icon beside the folder title and opens across the
        // app bar when wanted. A running search keeps it open.
        // Latched open by any search, as on the board.
        var mailSearchOpen by remember { mutableStateOf(mailSearch.isNotBlank()) }
        LaunchedEffect(mailSearch.isNotBlank()) { if (mailSearch.isNotBlank()) mailSearchOpen = true }
        val showMailSearch = mailSearchOpen || mailSearch.isNotBlank()
        val mailSearchFocus = remember { FocusRequester() }
        var mailSearchFocusRequests by remember { mutableStateOf(0) }
        LaunchedEffect(mailSearchFocusRequests) {
            if (mailSearchFocusRequests > 0) runCatching { mailSearchFocus.requestFocus() }
        }
        val closeMailSearch = {
            mailSearchOpen = false
            // A search emptied by hand but never resubmitted still has its results
            // on screen, or on their way, so check what the list was read with
            // and what is being read, not just the box.
            if (
                mailSearch.isNotBlank() ||
                visibleMailboxKey?.query?.isNotBlank() == true ||
                activeMailboxLoadKey?.query?.isNotBlank() == true
            ) {
                mailSearch = ""
                syncCoreThreads(syncFirst = false)
            }
        }
        BackHandler(enabled = showMailSearch && !mailSelectionActive && drawerState.isClosed, onBack = closeMailSearch)
        if (searchOptionsOpen) {
            SearchOptionsSheet(
                state = state,
                query = mailSearch,
                onDismiss = { searchOptionsOpen = false },
                onSearch = { query ->
                    mailSearch = query
                    syncCoreThreads(syncFirst = false)
                },
                contactsAccountId = selectedCoreAccountId.takeUnless { it == UNIFIED_ACCOUNT_ID }.orEmpty(),
            )
        }
        val mailListKey =
            visibleMailboxKey
                ?: mailboxCacheKey(selectedCoreAccountId, selectedCoreFolder, mailSearch, mailFilter, mailAttachmentsOnly, mailGroupBySender)
        val mailListPosition =
            remember(mailListKey) {
                // Keyed by search text and filter as well as the mailbox, so every
                // query the user submits leaves a state behind. Evict the
                // oldest-inserted ones past the cap — the mailboxes worth
                // returning to are the handful most recently opened.
                rememberedMailListKeysToEvict(
                    existingKeys = mailListStates.keys.toList(),
                    requestedKey = mailListKey,
                    maxSize = MAX_REMEMBERED_MAIL_LIST_STATES,
                ).forEach(mailListStates::remove)
                mailListStates.getOrPut(mailListKey) { MailListPosition() }
            }
        // Use the retained position so mailbox switches and returning from a
        // conversation restore the tint without waiting for another swipe.
        val listScrolled by remember(mailListPosition) { derivedStateOf { mailListPosition.listState.canScrollBackward } }
        val appBarColors = TopAppBarDefaults.topAppBarColors()
        val appBarColor by animateColorAsState(
            targetValue = if (listScrolled) appBarColors.scrolledContainerColor else appBarColors.containerColor,
        )
        DrawerBackHandler(drawerState)
        ModalNavigationDrawer(
            drawerState = drawerState,
            drawerContent = {
                MailDrawer(
                    accounts = coreAccounts.filterNot { it.id in hiddenNavigationAccountIds },
                    selectedAccountId = selectedCoreAccountId,
                    folders = drawerFolders,
                    currentScreen = screen,
                    showUnreadBadges = showUnreadBadges,
                    showUnifiedInboxNav = showUnifiedInboxNav,
                    kanbanBoards = kanbanBoards,
                    activeKanbanBoardId = activeKanbanBoardId,
                    tasksEnabled = tasksEnabled,
                    calendarEnabled = calendarEnabled,
                    meronFolders = meronFolders,
                    selectedFolderId = selectedCoreFolder,
                    onSelectMeronFolder = { folder ->
                        openMeronFolderView(folder.id)
                        screen = Screen.Mail
                        scope.launch { drawerState.close() }
                    },
                    onNewMeronFolder = {
                        promptNewMeronFolder()
                        scope.launch { drawerState.close() }
                    },
                    onRenameMeronFolder = { folder ->
                        promptRenameMeronFolder(folder)
                        scope.launch { drawerState.close() }
                    },
                    onSetMeronFolderNotify = { folder, enabled ->
                        setMeronFolderNotify(folder, enabled)
                    },
                    onDeleteMeronFolder = { folder ->
                        promptDeleteMeronFolder(folder)
                        scope.launch { drawerState.close() }
                    },
                    onSelectUnified = {
                        if (selectedCoreAccountId != UNIFIED_ACCOUNT_ID) {
                            selectCoreMailbox(UNIFIED_ACCOUNT_ID)
                            syncCoreThreads(accountOverride = UNIFIED_ACCOUNT_ID, folderOverride = selectedCoreFolder, syncFirst = false)
                        }
                        screen = Screen.Mail
                        scope.launch { drawerState.close() }
                    },
                    onSelectAccount = { account ->
                        if (selectedCoreAccountId != account.id) {
                            selectCoreMailbox(account.id)
                            syncCoreThreads(accountOverride = account.id, folderOverride = selectedCoreFolder, syncFirst = false)
                        }
                        screen = Screen.Mail
                        scope.launch { drawerState.close() }
                    },
                    onSelectKanbanBoard = { board ->
                        activeKanbanBoardId = board.id
                        saveActiveKanbanBoardId(kanbanPrefs, board.id)
                        screen = Screen.Kanban
                        previousTopScreen = Screen.Kanban
                        loadKanbanBoard(refresh = false)
                        scope.launch { drawerState.close() }
                    },
                    onSelectTasks = {
                        scope.launch {
                            openTasks()
                            drawerState.close()
                        }
                    },
                    onSelectCalendar = {
                        scope.launch {
                            openCalendar()
                            drawerState.close()
                        }
                    },
                    onAddAccount = {
                        resetPasswordAccountForm()
                        addSection = 0
                        previousTopScreen = screen
                        screen = Screen.AddAccount
                        scope.launch { drawerState.close() }
                    },
                    onOpenSettings = {
                        previousTopScreen = screen
                        screen = Screen.Settings
                        scope.launch { drawerState.close() }
                    },
                    onOpenContacts = {
                        previousTopScreen = screen
                        screen = Screen.Contacts
                        scope.launch { drawerState.close() }
                    },
                    onShowAbout = {
                        showAboutDialog = true
                        scope.launch { drawerState.close() }
                    },
                    googleReauthAccountId = googleReauthAccountId,
                    onReconnectGoogle = {
                        connectGoogleDeviceAccount()
                        scope.launch { drawerState.close() }
                    },
                )
            },
        ) {
            Scaffold(
                topBar = {
                    MailboxTopBar(
                        state,
                        drawerState,
                        selectedAccount,
                        selectedAccountIsRss,
                        mailSelectionActive,
                        selectedMailThreads,
                        importOpml,
                        showMailSearch,
                        mailSearchFocus,
                        closeMailSearch,
                        {
                            mailSearchOpen = true
                            mailSearchFocusRequests++
                        },
                        { searchOptionsOpen = true },
                        appBarColors.copy(containerColor = appBarColor),
                    )
                },
                floatingActionButton = {
                    when {
                        selectedAccountIsRss -> {
                            ExtendedFloatingActionButton(
                                onClick = {
                                    addFeedUrl = ""
                                    addFeedError = ""
                                    showAddFeedDialog = true
                                },
                                icon = { Icon(Icons.Filled.Add, contentDescription = tr("feeds.actions.addFeed")) },
                                text = { Text(tr("feeds.actions.addFeed")) },
                            )
                        }

                        coreAccounts.any { !accountSummaryIsRss(it) } -> {
                            ExtendedFloatingActionButton(
                                onClick = ::openCompose,
                                icon = { Icon(Icons.Filled.Edit, contentDescription = tr("mobile.tabs.compose")) },
                                text = { Text(tr("mobile.tabs.compose")) },
                            )
                        }
                    }
                },
                snackbarHost = { SnackbarHost(snackbarHost) },
            ) { innerPadding ->
                Column(Modifier.fillMaxSize().padding(innerPadding)) {
                    MailboxStatus(selectedAccount)
                    val showingBlockingInboxLoad =
                        !initialAccountsLoaded ||
                            accountsLoading ||
                            (coreThreads.isEmpty() && (syncing || !initialThreadsLoaded))
                    LaunchedEffect(
                        screen,
                        initialAccountsLoaded,
                        accountsLoading,
                        coreAccounts.size,
                        coreThreads.size,
                        syncing,
                        initialThreadsLoaded,
                        selectedCoreAccountId,
                        selectedCoreFolder,
                    ) {
                        if (screen == Screen.Mail) {
                            Log.i(
                                "MailLoad",
                                "render mail blocking=$showingBlockingInboxLoad initialAccountsLoaded=$initialAccountsLoaded accountsLoading=$accountsLoading accounts=${coreAccounts.size} threads=${coreThreads.size} syncing=$syncing initialThreadsLoaded=$initialThreadsLoaded selectedAccount=$selectedCoreAccountId folder=$selectedCoreFolder",
                            )
                        }
                    }
                    LaunchedEffect(
                        screen,
                        showingBlockingInboxLoad,
                        activeMailboxLoadKey,
                        activeMailboxLoadStartedAtMillis,
                    ) {
                        if (screen != Screen.Mail || !showingBlockingInboxLoad) return@LaunchedEffect
                        delay(MAILBOX_BLOCKING_WARN_AFTER_MS)
                        val stillBlocking =
                            screen == Screen.Mail &&
                                (
                                    !initialAccountsLoaded ||
                                        accountsLoading ||
                                        (coreThreads.isEmpty() && (syncing || !initialThreadsLoaded))
                                )
                        if (stillBlocking && !blockingMailboxLoadWarned) {
                            blockingMailboxLoadWarned = true
                            Log.w(
                                "MailLoad",
                                "mail UI still blocking after ${MAILBOX_BLOCKING_WARN_AFTER_MS}ms accountsLoaded=$initialAccountsLoaded accountsLoading=$accountsLoading accounts=${coreAccounts.size} threads=${coreThreads.size} syncing=$syncing initialThreadsLoaded=$initialThreadsLoaded activeLoad=$activeMailboxLoadKey selectedAccount=$selectedCoreAccountId folder=$selectedCoreFolder",
                            )
                        }
                        delay(MAILBOX_BLOCKING_TIMEOUT_MS - MAILBOX_BLOCKING_WARN_AFTER_MS)
                        val stillBlocked = {
                            screen == Screen.Mail &&
                                coreAccounts.isNotEmpty() &&
                                coreThreads.isEmpty() &&
                                (syncing || !initialThreadsLoaded)
                        }
                        if (stillBlocked() && syncing) {
                            // The sync request is still in flight — a slow
                            // first sync, not a failure. Switch the loader
                            // copy and give it until the hard cap before
                            // treating it as an error.
                            Log.w(
                                "MailLoad",
                                "mail sync still running after ${MAILBOX_BLOCKING_TIMEOUT_MS}ms accounts=${coreAccounts.size} activeLoad=$activeMailboxLoadKey selectedAccount=$selectedCoreAccountId folder=$selectedCoreFolder",
                            )
                            blockingMailboxLoadSlow = true
                            delay(MAILBOX_BLOCKING_HARD_TIMEOUT_MS - MAILBOX_BLOCKING_TIMEOUT_MS)
                        }
                        if (stillBlocked()) expireBlockingMailboxLoad()
                    }
                    PullToRefreshBox(
                        isRefreshing = pullRefreshing && syncing && !showingBlockingInboxLoad,
                        onRefresh = {
                            pullRefreshing = true
                            syncCoreThreads(scrollToTopOnSuccess = true)
                        },
                        modifier = Modifier.fillMaxSize(),
                    ) {
                        when {
                            !initialAccountsLoaded || accountsLoading -> {
                                LoadingState(
                                    tr(if (coreMigrating) "connectivity.upgradingStore" else "mobile.status.loadingInbox"),
                                )
                            }

                            coreAccounts.isEmpty() -> {
                                EmptyState(
                                    icon = Icons.Filled.PersonAdd,
                                    title = tr("empty.welcomeTitle"),
                                    text = tr("empty.mailOrRssSetupText"),
                                    actionLabel = tr("accounts.actions.addAccount"),
                                    onAction = {
                                        resetPasswordAccountForm()
                                        addSection = 0
                                        screen = Screen.AddAccount
                                    },
                                    footer = { state.CopySetupFromComputerLink() },
                                )
                            }

                            coreThreads.isEmpty() && (syncing || !initialThreadsLoaded) -> {
                                LoadingState(
                                    if (selectedAccountIsRss) {
                                        if (blockingMailboxLoadSlow) {
                                            tr("mobile.status.stillSyncingFeeds")
                                        } else {
                                            tr("mobile.status.loadingFeeds")
                                        }
                                    } else {
                                        if (blockingMailboxLoadSlow) {
                                            tr("mobile.status.stillSyncingInbox")
                                        } else {
                                            tr("mobile.status.loadingInbox")
                                        }
                                    },
                                )
                            }

                            coreThreads.isEmpty() &&
                                inMeronFolderView &&
                                mailSearch.isBlank() &&
                                mailFilter == FilterMode.All &&
                                !mailAttachmentsOnly -> {
                                EmptyState(
                                    icon = Icons.Outlined.Folder,
                                    title = tr("meronFolders.empty"),
                                    text =
                                        tr(
                                            if (selectedCoreFolder == MERON_ARCHIVE_FOLDER_ID) {
                                                "meronFolders.archiveEmptyHint"
                                            } else {
                                                "meronFolders.emptyHint"
                                            },
                                        ),
                                )
                            }

                            coreThreads.isEmpty() -> {
                                val narrowed = mailSearch.isNotBlank() || mailFilter != FilterMode.All || mailAttachmentsOnly
                                // A narrowed page can come back empty with older
                                // mail still to read (the core stops reading on
                                // after a few emptied pages): offer the next page
                                // rather than a dead end.
                                val canPageOn = narrowed && pageableCoreAccounts().isNotEmpty()
                                EmptyState(
                                    icon = Icons.Outlined.Drafts,
                                    title =
                                        if (!narrowed) {
                                            if (selectedAccountIsRss) tr("empty.noFeeds") else tr("empty.nothingHereYet")
                                        } else {
                                            tr("empty.noMatchingMail")
                                        },
                                    text =
                                        if (!narrowed) {
                                            if (selectedAccountIsRss) tr("empty.addFeedToStart") else tr("empty.pullLatestMessages")
                                        } else {
                                            tr("empty.adjustSearchFilter")
                                        },
                                    actionLabel =
                                        when {
                                            canPageOn -> tr("threads.actions.loadMore")
                                            selectedAccountIsRss -> null
                                            else -> tr("mobile.mail.syncMailbox")
                                        },
                                    onAction =
                                        when {
                                            canPageOn -> ({ loadMoreCoreThreads(quiet = false) })
                                            selectedAccountIsRss -> null
                                            else -> ({ syncCoreThreads() })
                                        },
                                )
                            }

                            else -> {
                                MailList(
                                    threads = coreThreads,
                                    accounts = coreAccounts,
                                    position = mailListPosition,
                                    scrollToTopRequest = mailListScrollToTopRequest,
                                    canLoadMore = pageableCoreAccounts().isNotEmpty(),
                                    loadingMore = loadingMoreThreads,
                                    onOpen = { readCoreThread(it, browseSource = ThreadBrowseSource.MailList) },
                                    onToggleStar = ::toggleStar,
                                    onArchive = ::archiveOrRemove,
                                    onDelete = ::deleteThread,
                                    selectedThreadIds = selectedMailThreadIds,
                                    selectionActive = mailSelectionActive,
                                    onToggleSelected = { thread ->
                                        selectedMailThreadIds =
                                            if (thread.id in selectedMailThreadIds) {
                                                selectedMailThreadIds - thread.id
                                            } else {
                                                selectedMailThreadIds + thread.id
                                            }
                                        if (selectedMailThreadIds.isEmpty()) {
                                            mailSelectionMenuOpen = false
                                        }
                                    },
                                    onLongPress = { thread ->
                                        selectedMailThreadIds =
                                            if (thread.id in selectedMailThreadIds) {
                                                selectedMailThreadIds
                                            } else {
                                                selectedMailThreadIds + thread.id
                                            }
                                    },
                                    onLoadMore = { userInitiated ->
                                        loadMoreCoreThreads(quiet = !userInitiated)
                                    },
                                    showSenderImages = showSenderImages,
                                    showAccountBadge = selectedCoreAccountId == UNIFIED_ACCOUNT_ID || inMeronFolderView,
                                    // Search shows conversations from Meron folders too; say which.
                                    meronFolderLabel =
                                        if (!inMeronFolderView && visibleMailboxKey?.query?.isNotBlank() == true) {
                                            { thread -> meronFolderName(thread.meronFolderId) }
                                        } else {
                                            { null }
                                        },
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}
