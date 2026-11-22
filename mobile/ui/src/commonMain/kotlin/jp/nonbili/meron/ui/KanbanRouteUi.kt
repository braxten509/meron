// Displays the board route in mobile navigation.
// Board selection drives the thread and column actions available from this route.
// The surrounding navigation host supplies account state and opening callbacks.

package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.Box
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
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.MarkEmailRead
import androidx.compose.material.icons.filled.MarkEmailUnread
import androidx.compose.material.icons.filled.Menu
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.SelectAll
import androidx.compose.material.icons.filled.Sell
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Star
import androidx.compose.material.icons.filled.StarBorder
import androidx.compose.material.icons.outlined.FolderOpen
import androidx.compose.material3.DrawerState
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.backhandler.BackHandler
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import jp.nonbili.meron.shared.FolderSummary
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.accountSummaryIsRss
import jp.nonbili.meron.shared.threadIdIsRss
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@OptIn(ExperimentalMaterial3Api::class, ExperimentalComposeUiApi::class)
@Composable
internal fun KanbanRouteContent(
    state: MeronMobileState,
    drawerState: DrawerState,
    drawerFolders: List<FolderSummary>,
    mailSelectionActive: Boolean,
    selectedMailThreads: List<ThreadSummary>,
    activeKanbanBoard: KanbanBoardSpec?,
) {
    with(state) {
        var searchOptionsOpen by remember { mutableStateOf(false) }
        // Search folds to an icon in the app bar, as on desktop, and opens across
        // it when wanted. A running search or scope keeps it open.
        val kanbanSearchActive = kanbanSearch.isNotBlank() || kanbanSearchScope.ifBlank { "all" } != "all"
        // Latched open by any search, restored ones included, so emptying the box
        // by hand can't fold it over results only closing it clears.
        var kanbanSearchOpen by remember { mutableStateOf(kanbanSearchActive) }
        LaunchedEffect(kanbanSearchActive) { if (kanbanSearchActive) kanbanSearchOpen = true }
        val showKanbanSearch = kanbanSearchOpen || kanbanSearchActive
        val kanbanSearchFocus = remember { FocusRequester() }
        var kanbanSearchFocusRequests by remember { mutableStateOf(0) }
        LaunchedEffect(kanbanSearchFocusRequests) {
            if (kanbanSearchFocusRequests > 0) runCatching { kanbanSearchFocus.requestFocus() }
        }
        val openKanbanSearch: () -> Unit = {
            kanbanSearchOpen = true
            kanbanSearchFocusRequests++
        }
        val closeKanbanSearch = {
            kanbanSearchOpen = false
            persistKanbanSearch("")
            persistKanbanSearchScope("all")
            // A search emptied by hand but never resubmitted still has its results
            // on screen, or on their way, so compare what each column was read
            // with, not the box; a load still out is replaced, so its results
            // can't land after the search has closed.
            val showingOtherView =
                activeKanbanBoard?.columns.orEmpty().any { column ->
                    kanbanColumns[kanbanColumnKey(column)]?.let { it.loading || it.cursorView != currentKanbanColumnView(column) } == true
                }
            if (showingOtherView) loadKanbanBoard(refresh = true)
        }
        BackHandler(enabled = showKanbanSearch && !mailSelectionActive && drawerState.isClosed, onBack = closeKanbanSearch)
        if (searchOptionsOpen) {
            SearchOptionsSheet(
                state = state,
                query = kanbanSearch,
                onDismiss = { searchOptionsOpen = false },
                onSearch = { query ->
                    persistKanbanSearch(query)
                    loadKanbanBoard(refresh = true)
                },
                // The sheet narrows these to the column picked under "Search in", or
                // offers them all for a board-wide search. A saved search narrowed by
                // it isn't saved again (see saveKanbanSearchColumn).
                saveColumns = activeKanbanBoard?.columns.orEmpty().filter { it.query.isBlank() },
                columnLabel = { column ->
                    val account = coreAccounts.firstOrNull { it.id == column.accountId }
                    searchScopeTitle(column, coreAccounts, foldersByAccount) to account?.let { it.displayName.ifBlank { it.email } }.orEmpty()
                },
                onSave = { query, column -> saveKanbanSearchColumn(column, query, clearsScopedSearch = false) },
                scopeColumns = activeKanbanBoard?.columns.orEmpty(),
                scope = kanbanSearchScope,
                onScopeChange = ::persistKanbanSearchScope,
            )
        }
        // A saved search column's query, edited in the same form.
        var editingSearchColumn by remember { mutableStateOf<KanbanColumnSpec?>(null) }
        editingSearchColumn?.let { column ->
            SearchOptionsSheet(
                state = state,
                query = column.query,
                onDismiss = { editingSearchColumn = null },
                onSearch = { query -> editKanbanSearchColumn(column, query) },
                contactsAccountId = column.accountId.takeUnless { it == UNIFIED_ACCOUNT_ID }.orEmpty(),
                submitLabel = tr("buttons.save"),
            )
        }
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
                        screen = Screen.Mail
                        openMeronFolderView(folder.id)
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
                        screen = Screen.Mail
                        if (selectedCoreAccountId != UNIFIED_ACCOUNT_ID) {
                            selectCoreMailbox(UNIFIED_ACCOUNT_ID)
                            syncCoreThreads(accountOverride = UNIFIED_ACCOUNT_ID, folderOverride = selectedCoreFolder, syncFirst = false)
                        }
                        scope.launch { drawerState.close() }
                    },
                    onSelectAccount = { account ->
                        screen = Screen.Mail
                        if (selectedCoreAccountId != account.id) {
                            selectCoreMailbox(account.id)
                            syncCoreThreads(accountOverride = account.id, folderOverride = selectedCoreFolder, syncFirst = false)
                        }
                        scope.launch { drawerState.close() }
                    },
                    onSelectKanbanBoard = { board ->
                        if (activeKanbanBoardId != board.id) {
                            activeKanbanBoardId = board.id
                            saveActiveKanbanBoardId(kanbanPrefs, board.id)
                            loadKanbanBoard(refresh = false)
                        }
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
                        previousTopScreen = Screen.Kanban
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
                    TopAppBar(
                        title = {
                            if (mailSelectionActive) {
                                MailSelectionTitle(selectedMailThreads.size, height = 52.dp)
                            } else if (!showKanbanSearch) {
                                Text(
                                    activeKanbanBoard?.name.orEmpty(),
                                    fontSize = 18.sp,
                                    fontWeight = FontWeight.SemiBold,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            } else {
                                KanbanHeaderSearchField(
                                    search = kanbanSearch,
                                    searchScope = kanbanSearchScope,
                                    columns = activeKanbanBoard?.columns.orEmpty(),
                                    accounts = coreAccounts.filterNot { it.id in hiddenNavigationAccountIds },
                                    foldersByAccount = foldersByAccount,
                                    onSearchChange = ::persistKanbanSearch,
                                    onSearchScopeChange = ::persistKanbanSearchScope,
                                    onSearchSubmit = { loadKanbanBoard(refresh = true) },
                                    onOpenOptions = { searchOptionsOpen = true },
                                    focusRequester = kanbanSearchFocus,
                                )
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
                            } else if (showKanbanSearch) {
                                IconButton(onClick = closeKanbanSearch) {
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
                                IconButton(onClick = {
                                    archiveThreads(selectedMailThreads)
                                    selectedMailThreadIds = emptySet()
                                }) {
                                    Icon(Icons.Filled.Archive, contentDescription = tr("threads.actions.archiveThread"))
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
                                        val selectableThreadIds =
                                            kanbanColumns.values
                                                .flatMap { column -> column.threads.map { it.id } }
                                                .toSet()
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
                                        val singleSelectedMailThread =
                                            selectedMailThreads
                                                .singleOrNull()
                                                ?.takeUnless { threadIdIsRss(it.id) }
                                        if (singleSelectedMailThread != null) {
                                            HorizontalDivider(Modifier.padding(vertical = 4.dp))
                                            if (!threadIdIsRss(singleSelectedMailThread.id)) {
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
                            } else if (!showKanbanSearch) {
                                // An open search takes the whole bar, the board's menu included.
                                IconButton(onClick = openKanbanSearch) {
                                    Icon(Icons.Filled.Search, contentDescription = tr("kanban.searchBoardAction"))
                                }
                                Box {
                                    IconButton(onClick = { kanbanMenuOpen = true }) {
                                        Icon(Icons.Filled.MoreVert, contentDescription = tr("kanban.actions.boardOptions"))
                                    }
                                    DropdownMenu(
                                        expanded = kanbanMenuOpen,
                                        onDismissRequest = { kanbanMenuOpen = false },
                                        modifier = Modifier.width(260.dp),
                                    ) {
                                        FilterModeSegmentedControl(
                                            filter = kanbanFilter,
                                            onFilterChange = { mode ->
                                                persistKanbanFilter(mode)
                                                loadKanbanBoard(refresh = true)
                                            },
                                            modifier =
                                                Modifier
                                                    .fillMaxWidth()
                                                    .padding(horizontal = 12.dp, vertical = 8.dp),
                                        )
                                        AttachmentsOnlyMenuItem(
                                            checked = kanbanAttachmentsOnly,
                                            onCheckedChange = { on ->
                                                persistKanbanAttachmentsOnly(on)
                                                loadKanbanBoard(refresh = true)
                                            },
                                        )
                                        HorizontalDivider(Modifier.padding(vertical = 4.dp))
                                        DropdownMenuItem(
                                            text = { Text(tr("kanban.actions.markAllColumnsRead")) },
                                            leadingIcon = { Icon(Icons.Filled.MarkEmailRead, contentDescription = null) },
                                            enabled = !kanbanMarkingRead && activeKanbanBoard?.columns?.isNotEmpty() == true,
                                            onClick = {
                                                kanbanMenuOpen = false
                                                markKanbanBoardAllRead()
                                            },
                                        )
                                        DropdownMenuItem(
                                            text = { Text(tr("mobile.actions.refreshBoard")) },
                                            leadingIcon = { Icon(Icons.Filled.Refresh, contentDescription = null) },
                                            onClick = {
                                                kanbanMenuOpen = false
                                                loadKanbanBoard(refresh = true)
                                            },
                                        )
                                        HorizontalDivider(Modifier.padding(vertical = 4.dp))
                                        DropdownMenuItem(
                                            text = { Text(tr("kanban.actions.addColumn")) },
                                            leadingIcon = { Icon(Icons.Filled.Add, contentDescription = null) },
                                            onClick = {
                                                kanbanMenuOpen = false
                                                coreAccounts.forEach { account ->
                                                    scope.launch {
                                                        runCatching {
                                                            withContext(ioDispatcher) {
                                                                val client = MobileMailCommandClient(core)
                                                                loadAccountFolders(client, account)
                                                            }
                                                        }.onSuccess { foldersByAccount = foldersByAccount + (account.id to it) }
                                                    }
                                                }
                                                showKanbanColumnDialog = true
                                            },
                                        )
                                        DropdownMenuItem(
                                            text = { Text(tr("kanban.actions.boardOptions")) },
                                            leadingIcon = { Icon(Icons.Filled.Settings, contentDescription = null) },
                                            enabled = activeKanbanBoard != null,
                                            onClick = {
                                                kanbanMenuOpen = false
                                                activeKanbanBoard?.let { board ->
                                                    kanbanSettingsTargetId = board.id
                                                    previousTopScreen = Screen.Kanban
                                                    screen = Screen.Settings
                                                }
                                            },
                                        )
                                    }
                                }
                            }
                        },
                    )
                },
                floatingActionButton = {
                    if (coreAccounts.any { !accountSummaryIsRss(it) }) {
                        ExtendedFloatingActionButton(
                            onClick = ::openCompose,
                            icon = { Icon(Icons.Filled.Edit, contentDescription = tr("mobile.tabs.compose")) },
                            text = { Text(tr("mobile.tabs.compose")) },
                        )
                    }
                },
                snackbarHost = { SnackbarHost(snackbarHost) },
            ) { innerPadding ->
                KanbanScreen(
                    modifier = Modifier.fillMaxSize().padding(innerPadding),
                    accounts = coreAccounts.filterNot { it.id in hiddenNavigationAccountIds },
                    accountsLoading = !initialAccountsLoaded || accountsLoading,
                    board = activeKanbanBoard,
                    columns = kanbanColumns,
                    foldersByAccount = foldersByAccount,
                    filter = kanbanFilter,
                    attachmentsOnly = kanbanAttachmentsOnly,
                    search = kanbanSearch,
                    searchScope = kanbanSearchScope,
                    onOpen = { thread, column ->
                        readCoreThread(thread, sourceFolder = column.folderId, browseSource = ThreadBrowseSource.KanbanColumn(column))
                    },
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
                    onToggleStar = ::toggleStar,
                    onRefreshColumn = { loadKanbanColumn(it, refresh = true) },
                    onLoadMoreColumn = ::loadMoreKanbanColumn,
                    onMarkColumnAllRead = ::markKanbanColumnAllRead,
                    markingRead = kanbanMarkingRead,
                    onEmptyColumnFolder = { column, folder ->
                        pendingEmptyFolder =
                            EmptyFolderTarget(
                                accountId = folder.accountId,
                                folderId = folder.name,
                                folderName = folder.displayName,
                                role = folder.role,
                                column = column,
                            )
                    },
                    onDeleteColumnFolder = { column, folder ->
                        pendingDeleteFolder =
                            DeleteFolderTarget(
                                accountId = folder.accountId,
                                folderId = folder.name,
                                folderName = folder.displayName,
                                nested =
                                    nestedFolders(
                                        foldersByAccount[folder.accountId].orEmpty(),
                                        folder.accountId,
                                        folder.name,
                                    ).size,
                                column = column,
                            )
                    },
                    onRenameColumnFolder = ::promptRenameFolder,
                    onNewColumnFolder = { column, account ->
                        promptNewFolder(account, NewFolderPlacement.SwitchColumn(activeKanbanBoardId, column))
                    },
                    onRemoveColumn = ::removeKanbanColumn,
                    onMoveColumn = ::moveKanbanColumn,
                    onSearchColumn = { column ->
                        openKanbanSearch()
                        persistKanbanSearchScope(kanbanColumnKey(column))
                        if (kanbanSearch.isNotBlank()) loadKanbanBoard(refresh = true)
                    },
                    onSaveSearchColumn = ::saveKanbanSearchColumn,
                    onEditSearchColumn = { column -> editingSearchColumn = column },
                    onRequestColumnFolders = { column -> ensureAccountFolders(column.accountId) },
                    onSwitchColumnFolder = ::switchKanbanColumnFolder,
                    onAddColumn = { showKanbanColumnDialog = true },
                    showSenderImages = showSenderImages,
                    kanbanColumnWidth = kanbanColumnWidth.dp,
                )
            }
        }
    }
}
