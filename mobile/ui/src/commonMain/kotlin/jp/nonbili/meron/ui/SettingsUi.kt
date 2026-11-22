// Composes the main mobile settings pages and their
// navigation. Feature-specific editors are delegated to
// their corresponding account, appearance and mailbox
// panels.

package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Chat
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.AutoAwesome
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.FilterList
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.ViewKanban
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.backhandler.BackHandler
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.FolderSummary
import jp.nonbili.meron.shared.ProxySpec
import jp.nonbili.meron.shared.SignatureSpec
import jp.nonbili.meron.shared.StorageUsage
import jp.nonbili.meron.shared.accountSummaryIsRss
import kotlinx.coroutines.withContext

@OptIn(ExperimentalMaterial3Api::class, ExperimentalComposeUiApi::class)
@Composable
internal fun SettingsScreen(
    rulesPage: @Composable (Modifier) -> Unit,
    aiSortPage: @Composable (Modifier, onOpenLog: () -> Unit) -> Unit,
    aiSortLogPage: @Composable (Modifier) -> Unit,
    setupCopyPage: @Composable (Modifier) -> Unit = {},
    onOpenTags: () -> Unit,
    onBack: () -> Unit,
    initialGeneral: Boolean,
    onConsumeInitialGeneral: () -> Unit,
    initialAccountId: String?,
    initialAccountProxy: Boolean,
    onConsumeInitialAccount: () -> Unit,
    initialKanbanBoardId: String?,
    onConsumeInitialKanbanBoard: () -> Unit,
    accounts: List<AccountSummary>,
    hiddenNavigationAccountIds: Set<String>,
    kanbanBoards: List<KanbanBoardSpec>,
    onSaveKanbanBoard: (
        board: KanbanBoardSpec,
        name: String,
        avatarUrl: String,
        wallpaperPresetId: String,
        wallpaperUrl: String,
    ) -> Unit,
    onDeleteKanbanBoard: (KanbanBoardSpec) -> Unit,
    onCreateKanbanBoard: () -> String,
    onAddMailAccount: () -> Unit,
    onAddFeedAccount: () -> Unit,
    onSaveAccountSettings: (
        account: AccountSummary,
        displayName: String,
        senderName: String,
        avatarUrl: String,
        wallpaperPresetId: String,
        loadRemoteImages: Boolean,
        conversationHtml: Boolean,
        includedInUnified: Boolean,
        showInNavigation: Boolean,
        muted: Boolean,
        paused: Boolean,
        rssSyncIntervalMinutes: Int,
        aliasesText: String,
    ) -> Unit,
    onPickAccountAvatar: (AccountSummary) -> Unit,
    onPickAccountWallpaper: (AccountSummary) -> Unit,
    onPickKanbanBoardAvatar: (KanbanBoardSpec) -> Unit,
    onPickKanbanBoardWallpaper: (KanbanBoardSpec) -> Unit,
    onMoveAccountUp: (AccountSummary) -> Unit,
    onMoveAccountDown: (AccountSummary) -> Unit,
    onRemoveAccount: (AccountSummary) -> Unit,
    themeChoice: ThemeChoice,
    systemDark: Boolean,
    onThemeChoiceChange: (ThemeChoice) -> Unit,
    customThemes: List<CustomTheme>,
    onImportCustomTheme: (CustomTheme) -> Unit,
    onDeleteCustomTheme: (CustomTheme) -> Unit,
    onShareCustomTheme: (CustomTheme) -> Unit,
    appLanguageTag: String,
    onAppLanguageChange: (String) -> Unit,
    showSenderImages: Boolean,
    onToggleSenderImages: () -> Unit,
    darkMailBodies: Boolean,
    onToggleDarkMailBodies: () -> Unit,
    chatFullMessages: Boolean,
    onToggleChatFullMessages: () -> Unit,
    autoFitMessages: Boolean,
    onToggleAutoFitMessages: () -> Unit,
    readerBottomActions: Boolean,
    onToggleReaderBottomActions: () -> Unit,
    blurredBars: Boolean,
    onToggleBlurredBars: () -> Unit,
    showUnreadBadges: Boolean,
    onToggleUnreadBadges: () -> Unit,
    showUnifiedInboxNav: Boolean,
    onToggleUnifiedInboxNav: () -> Unit,
    tasksEnabled: Boolean,
    onToggleTasks: () -> Unit,
    calendarEnabled: Boolean,
    onToggleCalendar: () -> Unit,
    reminderLeadMinutes: Int,
    reminderAllDayDays: Int,
    onAlertDefaultChange: (allDay: Boolean, lead: Int) -> Unit,
    sendShortcutMode: SendShortcutMode,
    onToggleSendShortcut: () -> Unit,
    quoteReplies: Boolean,
    onToggleQuoteReplies: () -> Unit,
    conversationLayout: ConversationLayout,
    onToggleConversationLayout: () -> Unit,
    messageFontScale: Int,
    onMessageFontScaleChange: (Int) -> Unit,
    appProxy: ProxySpec,
    onSaveAppProxy: (ProxySpec) -> Unit,
    appSignatureHtml: String,
    onSaveAppSignature: (String) -> Unit,
    remoteImageSenders: List<String>,
    onRemoveRemoteImageSender: (String) -> Unit,
    onSaveAccountSignature: (AccountSummary, SignatureSpec) -> Unit,
    onSaveAccountProxy: (AccountSummary, ProxySpec) -> Unit,
    onSaveAccountServerSettings: (AccountSummary, ServerSettingsDraft) -> Unit,
    foldersByAccount: Map<String, List<FolderSummary>>,
    onRequestAccountFolders: (AccountSummary) -> Unit,
    onSetFolderNotify: (FolderSummary, Boolean) -> Unit,
    kanbanColumnWidth: Int,
    onCycleKanbanColumnWidth: () -> Unit,
    notificationsNeedPermission: Boolean,
    onEnableNotifications: () -> Unit,
    supportsBackgroundPush: Boolean,
    liveMailPushEnabled: Boolean,
    onToggleLiveMailPush: () -> Unit,
    backgroundSyncEnabled: Boolean,
    onToggleBackgroundSync: () -> Unit,
    onRefreshBackground: () -> Unit,
    readDiagnosticLog: () -> String,
    onShareDiagnosticLog: () -> Unit,
    pollIntervalMinutes: Int,
    onCyclePollInterval: () -> Unit,
    storageUsage: StorageUsage?,
    storageBusy: Boolean,
    storageClearConfirming: Boolean,
    onRefreshStorage: () -> Unit,
    onClearStorageCache: () -> Unit,
    onExportBackup: () -> Unit,
    onRestoreBackup: () -> Unit,
    backupBusy: Boolean,
) {
    var showLanguagePicker by remember { mutableStateOf(false) }
    var showMessageTextSize by remember { mutableStateOf(false) }
    val settingsNavController = rememberNavController()
    val settingsBackStackEntry by settingsNavController.currentBackStackEntryAsState()
    var selectedSettingsAccountId by remember { mutableStateOf<String?>(null) }
    var selectedSettingsBoardId by remember { mutableStateOf<String?>(null) }
    var focusAccountProxy by remember { mutableStateOf(false) }
    var directOpenRoute by remember { mutableStateOf<String?>(null) }
    val page =
        when (settingsBackStackEntry?.destination?.route ?: SettingsRoutes.Root) {
            SettingsRoutes.General -> SettingsPage.General
            SettingsRoutes.Messages -> SettingsPage.Messages
            SettingsRoutes.Composer -> SettingsPage.Composer
            SettingsRoutes.Rules -> SettingsPage.Rules
            SettingsRoutes.AiSort -> SettingsPage.AiSort
            SettingsRoutes.AiSortLog -> SettingsPage.AiSortLog
            SettingsRoutes.SetupCopy -> SettingsPage.SetupCopy
            SettingsRoutes.Account -> selectedSettingsAccountId?.let { SettingsPage.AccountDetail(it) } ?: SettingsPage.Root
            SettingsRoutes.AccountWallpaper -> selectedSettingsAccountId?.let { SettingsPage.AccountWallpaper(it) } ?: SettingsPage.Root
            SettingsRoutes.KanbanBoard -> selectedSettingsBoardId?.let { SettingsPage.KanbanBoardDetail(it) } ?: SettingsPage.Root
            SettingsRoutes.KanbanBoardWallpaper -> selectedSettingsBoardId?.let { SettingsPage.KanbanBoardWallpaper(it) } ?: SettingsPage.Root
            SettingsRoutes.SyncLog -> SettingsPage.SyncLog
            SettingsRoutes.RemoteSenders -> SettingsPage.RemoteSenders
            SettingsRoutes.Theme -> SettingsPage.Theme
            else -> SettingsPage.Root
        }
    LaunchedEffect(initialGeneral) {
        if (initialGeneral) {
            directOpenRoute = SettingsRoutes.General
            settingsNavController.navigate(SettingsRoutes.General) {
                launchSingleTop = true
            }
            onConsumeInitialGeneral()
        }
    }
    LaunchedEffect(initialAccountId) {
        if (!initialAccountId.isNullOrBlank()) {
            selectedSettingsAccountId = initialAccountId
            focusAccountProxy = initialAccountProxy
            directOpenRoute = SettingsRoutes.Account
            settingsNavController.navigate(SettingsRoutes.Account) {
                launchSingleTop = true
            }
            onConsumeInitialAccount()
        }
    }
    LaunchedEffect(initialKanbanBoardId) {
        if (!initialKanbanBoardId.isNullOrBlank()) {
            selectedSettingsBoardId = initialKanbanBoardId
            directOpenRoute = SettingsRoutes.KanbanBoard
            settingsNavController.navigate(SettingsRoutes.KanbanBoard) {
                launchSingleTop = true
            }
            onConsumeInitialKanbanBoard()
        }
    }
    if (showLanguagePicker) {
        LanguagePickerDialog(
            currentTag = appLanguageTag,
            onSelect = onAppLanguageChange,
            onDismiss = { showLanguagePicker = false },
        )
    }
    if (showMessageTextSize) {
        MessageTextSizeDialog(
            scale = messageFontScale,
            onScaleChange = onMessageFontScaleChange,
            onDismiss = { showMessageTextSize = false },
        )
    }
    val handleBack: () -> Unit = {
        if (settingsBackStackEntry?.destination?.route == directOpenRoute) {
            directOpenRoute = null
            onBack()
        } else if (!settingsNavController.popBackStack()) {
            onBack()
        }
    }
    BackHandler(onBack = handleBack)
    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Text(
                        when (page) {
                            SettingsPage.Root -> tr("settings.label")
                            SettingsPage.General -> tr("settings.sections.general")
                            SettingsPage.Messages -> tr("settings.sections.messages")
                            SettingsPage.Composer -> tr("settings.sections.composer")
                            SettingsPage.Rules -> tr("rules.title")
                            SettingsPage.AiSort -> tr("aiSort.title")
                            SettingsPage.AiSortLog -> tr("aiSort.log")
                            SettingsPage.SetupCopy -> tr("setupCopy.title")
                            is SettingsPage.AccountDetail -> tr("settings.account.account")
                            is SettingsPage.AccountWallpaper -> tr("settings.account.chatBackground")
                            is SettingsPage.KanbanBoardDetail -> tr("kanban.board.label")
                            is SettingsPage.KanbanBoardWallpaper -> tr("settings.account.chatBackground")
                            SettingsPage.SyncLog -> tr("settings.viewSyncLog")
                            SettingsPage.RemoteSenders -> tr("settings.privacy.remoteSenders")
                            SettingsPage.Theme -> tr("common.theme")
                        },
                    )
                },
                navigationIcon = {
                    IconButton(onClick = handleBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = tr("buttons.back"))
                    }
                },
                actions = {
                    if (page == SettingsPage.SyncLog) {
                        IconButton(onClick = onShareDiagnosticLog) {
                            Icon(Icons.AutoMirrored.Filled.Send, contentDescription = tr("settings.shareSyncLog"))
                        }
                    }
                },
            )
        },
    ) { innerPadding ->
        NavHost(
            navController = settingsNavController,
            startDestination = SettingsRoutes.Root,
            modifier =
                Modifier
                    .fillMaxSize()
                    .padding(innerPadding)
                    .consumeWindowInsets(innerPadding)
                    .imePadding(),
        ) {
            composable(SettingsRoutes.General) {
                SettingsGeneralPage(
                    themeChoice = themeChoice,
                    systemDark = systemDark,
                    onOpenTheme = { settingsNavController.navigate(SettingsRoutes.Theme) },
                    appLanguageTag = appLanguageTag,
                    onOpenLanguage = { showLanguagePicker = true },
                    showSenderImages = showSenderImages,
                    onToggleSenderImages = onToggleSenderImages,
                    showUnreadBadges = showUnreadBadges,
                    onToggleUnreadBadges = onToggleUnreadBadges,
                    showUnifiedInboxNav = showUnifiedInboxNav,
                    onToggleUnifiedInboxNav = onToggleUnifiedInboxNav,
                    tasksEnabled = tasksEnabled,
                    onToggleTasks = onToggleTasks,
                    calendarEnabled = calendarEnabled,
                    onToggleCalendar = onToggleCalendar,
                    reminderLeadMinutes = reminderLeadMinutes,
                    reminderAllDayDays = reminderAllDayDays,
                    onAlertDefaultChange = onAlertDefaultChange,
                    kanbanColumnWidth = kanbanColumnWidth,
                    onCycleKanbanColumnWidth = onCycleKanbanColumnWidth,
                    appProxy = appProxy,
                    onSaveAppProxy = onSaveAppProxy,
                    notificationsNeedPermission = notificationsNeedPermission,
                    onEnableNotifications = onEnableNotifications,
                    supportsBackgroundPush = supportsBackgroundPush,
                    liveMailPushEnabled = liveMailPushEnabled,
                    onToggleLiveMailPush = onToggleLiveMailPush,
                    backgroundSyncEnabled = backgroundSyncEnabled,
                    onToggleBackgroundSync = onToggleBackgroundSync,
                    onRefreshBackground = onRefreshBackground,
                    onOpenSyncLog = { settingsNavController.navigate(SettingsRoutes.SyncLog) },
                    pollIntervalMinutes = pollIntervalMinutes,
                    onCyclePollInterval = onCyclePollInterval,
                    storageUsage = storageUsage,
                    storageBusy = storageBusy,
                    storageClearConfirming = storageClearConfirming,
                    onRefreshStorage = onRefreshStorage,
                    onClearStorageCache = onClearStorageCache,
                    onExportBackup = onExportBackup,
                    onRestoreBackup = onRestoreBackup,
                    backupBusy = backupBusy,
                    onOpenSetupCopy = { settingsNavController.navigate(SettingsRoutes.SetupCopy) },
                    focusProxy = directOpenRoute == SettingsRoutes.General,
                    modifier = Modifier.fillMaxSize(),
                )
            }

            composable(SettingsRoutes.Messages) {
                SettingsMessagesPage(
                    conversationLayout = conversationLayout,
                    onToggleConversationLayout = onToggleConversationLayout,
                    messageFontScale = messageFontScale,
                    onOpenMessageTextSize = { showMessageTextSize = true },
                    darkMailBodies = darkMailBodies,
                    onToggleDarkMailBodies = onToggleDarkMailBodies,
                    chatFullMessages = chatFullMessages,
                    onToggleChatFullMessages = onToggleChatFullMessages,
                    autoFitMessages = autoFitMessages,
                    onToggleAutoFitMessages = onToggleAutoFitMessages,
                    readerBottomActions = readerBottomActions,
                    onToggleReaderBottomActions = onToggleReaderBottomActions,
                    blurredBars = blurredBars,
                    onToggleBlurredBars = onToggleBlurredBars,
                    remoteImageSenderCount = remoteImageSenders.size,
                    onOpenRemoteSenders = { settingsNavController.navigate(SettingsRoutes.RemoteSenders) },
                    onOpenTags = onOpenTags,
                    modifier = Modifier.fillMaxSize(),
                )
            }

            composable(SettingsRoutes.Rules) {
                rulesPage(Modifier.fillMaxSize())
            }

            composable(SettingsRoutes.AiSort) {
                aiSortPage(Modifier.fillMaxSize()) { settingsNavController.navigate(SettingsRoutes.AiSortLog) }
            }

            composable(SettingsRoutes.AiSortLog) {
                aiSortLogPage(Modifier.fillMaxSize())
            }

            composable(SettingsRoutes.SetupCopy) {
                setupCopyPage(Modifier.fillMaxSize())
            }

            composable(SettingsRoutes.Composer) {
                SettingsComposerPage(
                    sendShortcutMode = sendShortcutMode,
                    onToggleSendShortcut = onToggleSendShortcut,
                    quoteReplies = quoteReplies,
                    onToggleQuoteReplies = onToggleQuoteReplies,
                    appSignatureHtml = appSignatureHtml,
                    onSaveAppSignature = onSaveAppSignature,
                    modifier = Modifier.fillMaxSize(),
                )
            }

            composable(SettingsRoutes.Theme) {
                ThemePickerPage(
                    choice = themeChoice,
                    systemDark = systemDark,
                    onChange = onThemeChoiceChange,
                    customThemes = customThemes,
                    onImportCustomTheme = onImportCustomTheme,
                    onDeleteCustomTheme = onDeleteCustomTheme,
                    onShareCustomTheme = onShareCustomTheme,
                    modifier = Modifier.fillMaxSize(),
                )
            }

            composable(SettingsRoutes.RemoteSenders) {
                RemoteSendersPage(
                    senders = remoteImageSenders,
                    onRemoveSender = onRemoveRemoteImageSender,
                    modifier = Modifier.fillMaxSize(),
                )
            }

            composable(SettingsRoutes.Account) {
                BackHandler(enabled = directOpenRoute == SettingsRoutes.Account) {
                    directOpenRoute = null
                    onBack()
                }
                val account = selectedSettingsAccountId?.let { id -> accounts.firstOrNull { it.id == id } }
                if (account == null) {
                    LaunchedEffect(Unit) { settingsNavController.popBackStack(SettingsRoutes.Root, inclusive = false) }
                } else {
                    val accountIndex = accounts.indexOfFirst { it.id == account.id }
                    SettingsAccountDetailPage(
                        account = account,
                        canMoveUp = accountIndex > 0,
                        canMoveDown = accountIndex >= 0 && accountIndex < accounts.lastIndex,
                        showInNavigation = account.id !in hiddenNavigationAccountIds,
                        onSave = {
                            displayName,
                            senderName,
                            avatarUrl,
                            wallpaperPresetId,
                            loadRemoteImages,
                            conversationHtml,
                            includedInUnified,
                            showInNavigation,
                            muted,
                            paused,
                            interval,
                            aliases,
                            ->
                            onSaveAccountSettings(
                                account,
                                displayName,
                                senderName,
                                avatarUrl,
                                wallpaperPresetId,
                                loadRemoteImages,
                                conversationHtml,
                                includedInUnified,
                                showInNavigation,
                                muted,
                                paused,
                                interval,
                                aliases,
                            )
                        },
                        onSaveProxy = { spec -> onSaveAccountProxy(account, spec) },
                        onSaveServerSettings = { draft -> onSaveAccountServerSettings(account, draft) },
                        onSaveSignature = { spec -> onSaveAccountSignature(account, spec) },
                        onPickAvatar = { onPickAccountAvatar(account) },
                        onOpenWallpaper = { settingsNavController.navigate(SettingsRoutes.AccountWallpaper) },
                        onMoveUp = { onMoveAccountUp(account) },
                        onMoveDown = { onMoveAccountDown(account) },
                        onRemove = { onRemoveAccount(account) },
                        focusProxy = focusAccountProxy && directOpenRoute == SettingsRoutes.Account,
                        folders = foldersByAccount[account.id].orEmpty(),
                        onRequestFolders = { onRequestAccountFolders(account) },
                        onSetFolderNotify = onSetFolderNotify,
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }

            composable(SettingsRoutes.KanbanBoard) {
                BackHandler(enabled = directOpenRoute == SettingsRoutes.KanbanBoard) {
                    directOpenRoute = null
                    onBack()
                }
                val board = selectedSettingsBoardId?.let { id -> kanbanBoards.firstOrNull { it.id == id } }
                if (board == null) {
                    LaunchedEffect(Unit) { settingsNavController.popBackStack(SettingsRoutes.Root, inclusive = false) }
                } else {
                    SettingsKanbanBoardDetailPage(
                        board = board,
                        onSave = { name, avatarUrl, wallpaperPresetId, wallpaperUrl ->
                            onSaveKanbanBoard(board, name, avatarUrl, wallpaperPresetId, wallpaperUrl)
                        },
                        onPickAvatar = { onPickKanbanBoardAvatar(board) },
                        onOpenWallpaper = { settingsNavController.navigate(SettingsRoutes.KanbanBoardWallpaper) },
                        onDelete = {
                            onDeleteKanbanBoard(board)
                            settingsNavController.popBackStack(SettingsRoutes.Root, inclusive = false)
                        },
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }

            composable(SettingsRoutes.AccountWallpaper) {
                val account = selectedSettingsAccountId?.let { id -> accounts.firstOrNull { it.id == id } }
                if (account == null) {
                    LaunchedEffect(Unit) { settingsNavController.popBackStack(SettingsRoutes.Root, inclusive = false) }
                } else {
                    WallpaperPickerPage(
                        selected = if (account.chatWallpaperKind == "custom") "__custom" else account.chatWallpaperPresetId,
                        previewPresetId = account.chatWallpaperPresetId,
                        previewCustomUrl = if (account.chatWallpaperKind == "custom") account.chatWallpaperUrl else "",
                        onSelect = { presetId ->
                            onSaveAccountSettings(
                                account,
                                account.displayName,
                                account.senderName,
                                account.avatarUrl,
                                presetId,
                                account.loadRemoteImages,
                                account.conversationHtml,
                                account.includedInUnified,
                                account.id !in hiddenNavigationAccountIds,
                                account.muted,
                                account.paused,
                                account.rssSyncIntervalMinutes,
                                account.aliases.joinToString("\n") { alias ->
                                    if (alias.name.isBlank()) alias.email else "${alias.email}, ${alias.name}"
                                },
                            )
                        },
                        onUpload = { onPickAccountWallpaper(account) },
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }

            composable(SettingsRoutes.KanbanBoardWallpaper) {
                val board = selectedSettingsBoardId?.let { id -> kanbanBoards.firstOrNull { it.id == id } }
                if (board == null) {
                    LaunchedEffect(Unit) { settingsNavController.popBackStack(SettingsRoutes.Root, inclusive = false) }
                } else {
                    WallpaperPickerPage(
                        selected = if (board.wallpaperUrl.isNotBlank()) "__custom" else board.wallpaperPresetId,
                        previewPresetId = board.wallpaperPresetId,
                        previewCustomUrl = board.wallpaperUrl,
                        onSelect = { presetId ->
                            onSaveKanbanBoard(board, board.name, board.avatarUrl, presetId, "")
                        },
                        onUpload = { onPickKanbanBoardWallpaper(board) },
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }

            composable(SettingsRoutes.SyncLog) {
                // The log is a file read: keep it off the UI thread.
                val logLines by produceState<List<String>?>(null) {
                    value = withContext(ioDispatcher) { diagnosticLogLines(readDiagnosticLog()) }
                }
                SettingsSyncLogPage(
                    logLines = logLines,
                    modifier = Modifier.fillMaxSize(),
                )
            }

            composable(SettingsRoutes.Root) {
                SettingsHome(
                    accounts,
                    hiddenNavigationAccountIds,
                    kanbanBoards,
                    onAddMailAccount,
                    onAddFeedAccount,
                    onCreateKanbanBoard,
                    { settingsNavController.navigate(it) },
                    { selectedSettingsAccountId = it },
                    { selectedSettingsBoardId = it },
                )
            }
        }
    }
}
