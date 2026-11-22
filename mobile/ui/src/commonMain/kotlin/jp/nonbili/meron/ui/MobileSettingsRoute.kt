// Connects the settings pages to the mobile app state.
// Account, theme and storage actions persist their choices through the host.
// Navigation and file pickers are supplied by the app that owns their lifecycle.

package jp.nonbili.meron.ui

import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.ExperimentalComposeUiApi
import jp.nonbili.meron.shared.nextPollIntervalMinutes
import kotlin.io.encoding.ExperimentalEncodingApi

@OptIn(ExperimentalMaterial3Api::class, ExperimentalEncodingApi::class, ExperimentalComposeUiApi::class)
@Composable
internal fun MeronMobileState.MobileSettingsRoute(
    themeChoice: ThemeChoice,
    systemDark: Boolean,
    onThemeChoiceChange: (ThemeChoice) -> Unit,
    customThemes: List<CustomTheme>,
    onImportCustomTheme: (CustomTheme) -> Unit,
    onDeleteCustomTheme: (CustomTheme) -> Unit,
    appLanguageTag: String,
    onAppLanguageChange: (String) -> Unit,
    popAppBack: () -> Unit,
    pickAccountMedia: () -> Unit,
    pickKanbanBoardMedia: () -> Unit,
    pickBackupFile: () -> Unit,
) {
    LaunchedEffect(Unit) { loadStorageUsage() }
    SettingsScreen(
        rulesPage = { modifier -> RulesPage(modifier) },
        aiSortPage = { modifier, onOpenLog -> AiSortPage(modifier, onOpenLog) },
        aiSortLogPage = { modifier -> AiSortLogPage(modifier) },
        setupCopyPage = { modifier -> SetupCopyPage(modifier) },
        onOpenTags = { showTagsDialog = true },
        onBack = popAppBack,
        initialGeneral = settingsGeneralTarget,
        onConsumeInitialGeneral = { settingsGeneralTarget = false },
        initialAccountId = accountSettingsTargetId,
        initialAccountProxy =
            accountSettingsTargetId != null &&
                accountSettingsTargetId == accountSettingsProxyTargetId,
        onConsumeInitialAccount = {
            accountSettingsTargetId = null
            accountSettingsProxyTargetId = null
        },
        initialKanbanBoardId = kanbanSettingsTargetId,
        onConsumeInitialKanbanBoard = { kanbanSettingsTargetId = null },
        accounts = coreAccounts,
        hiddenNavigationAccountIds = hiddenNavigationAccountIds,
        kanbanBoards = kanbanBoards,
        onSaveKanbanBoard = { board, name, avatarUrl, wallpaperPresetId, wallpaperUrl ->
            updateKanbanBoard(board.id, name, avatarUrl, wallpaperPresetId, wallpaperUrl)
        },
        onDeleteKanbanBoard = { board ->
            deleteKanbanBoard(board.id)
        },
        onCreateKanbanBoard = ::createKanbanBoard,
        onAddMailAccount = {
            resetPasswordAccountForm()
            addSection = 0
            previousTopScreen = Screen.Settings
            screen = Screen.AddAccount
        },
        onAddFeedAccount = {
            resetPasswordAccountForm()
            addSection = 2
            previousTopScreen = Screen.Settings
            screen = Screen.AddAccount
        },
        onSaveAccountSettings = {
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
            ->
            setAccountNavigationVisible(account, showInNavigation)
            saveAccountSettings(
                account,
                displayName,
                senderName,
                avatarUrl,
                wallpaperPresetId,
                loadRemoteImages,
                conversationHtml,
                includedInUnified,
                muted,
                paused,
                interval,
                aliases,
            )
        },
        onPickAccountAvatar = { account ->
            accountMediaUploadTarget = AccountMediaUploadTarget(account, wallpaper = false)
            pickAccountMedia()
        },
        onPickAccountWallpaper = { account ->
            accountMediaUploadTarget = AccountMediaUploadTarget(account, wallpaper = true)
            pickAccountMedia()
        },
        onPickKanbanBoardAvatar = { board ->
            kanbanBoardMediaTarget = KanbanBoardMediaTarget(board, wallpaper = false)
            pickKanbanBoardMedia()
        },
        onPickKanbanBoardWallpaper = { board ->
            kanbanBoardMediaTarget = KanbanBoardMediaTarget(board, wallpaper = true)
            pickKanbanBoardMedia()
        },
        onMoveAccountUp = { account -> moveAccount(account, -1) },
        onMoveAccountDown = { account -> moveAccount(account, 1) },
        onRemoveAccount = ::removeAccount,
        themeChoice = themeChoice,
        systemDark = systemDark,
        onThemeChoiceChange = onThemeChoiceChange,
        customThemes = customThemes,
        onImportCustomTheme = onImportCustomTheme,
        onDeleteCustomTheme = onDeleteCustomTheme,
        onShareCustomTheme = { theme -> mobileHost.shareText(serializeThemeSource(theme.source)) },
        appLanguageTag = appLanguageTag,
        onAppLanguageChange = onAppLanguageChange,
        showSenderImages = showSenderImages,
        onToggleSenderImages = {
            showSenderImages = !showSenderImages
            saveAppBoolean(prefs, SHOW_SENDER_IMAGES_PREF, showSenderImages)
        },
        darkMailBodies = darkMailBodies,
        onToggleDarkMailBodies = {
            darkMailBodies = !darkMailBodies
            saveAppBoolean(prefs, DARK_MAIL_BODIES_PREF, darkMailBodies)
        },
        chatFullMessages = chatFullMessages,
        onToggleChatFullMessages = {
            chatFullMessages = !chatFullMessages
            saveAppBoolean(prefs, CHAT_FULL_MESSAGES_PREF, chatFullMessages)
        },
        autoFitMessages = autoFitMessages,
        onToggleAutoFitMessages = {
            autoFitMessages = !autoFitMessages
            saveAppBoolean(prefs, AUTO_FIT_MESSAGES_PREF, autoFitMessages)
        },
        readerBottomActions = readerBottomActions,
        onToggleReaderBottomActions = {
            readerBottomActions = !readerBottomActions
            saveAppBoolean(prefs, READER_BOTTOM_ACTIONS_PREF, readerBottomActions)
        },
        blurredBars = blurredBars,
        onToggleBlurredBars = {
            blurredBars = !blurredBars
            saveAppBoolean(prefs, BLURRED_BARS_PREF, blurredBars)
        },
        showUnreadBadges = showUnreadBadges,
        onToggleUnreadBadges = {
            showUnreadBadges = !showUnreadBadges
            saveAppBoolean(prefs, SHOW_UNREAD_BADGES_PREF, showUnreadBadges)
        },
        showUnifiedInboxNav = showUnifiedInboxNav,
        onToggleUnifiedInboxNav = {
            showUnifiedInboxNav = !showUnifiedInboxNav
            saveAppBoolean(prefs, SHOW_UNIFIED_INBOX_PREF, showUnifiedInboxNav)
        },
        tasksEnabled = tasksEnabled,
        onToggleTasks = {
            tasksEnabled = !tasksEnabled
            saveAppBoolean(prefs, TASKS_ENABLED_PREF, tasksEnabled)
            if (!tasksEnabled && screen == Screen.Tasks) screen = Screen.Mail
        },
        calendarEnabled = calendarEnabled,
        onToggleCalendar = {
            calendarEnabled = !calendarEnabled
            saveAppBoolean(prefs, CALENDAR_ENABLED_PREF, calendarEnabled)
            if (!calendarEnabled && screen == Screen.Calendar) screen = Screen.Mail
        },
        reminderLeadMinutes = reminderLeadMinutes,
        reminderAllDayDays = reminderAllDayDays,
        onAlertDefaultChange = ::saveAlertDefault,
        sendShortcutMode = sendShortcutMode,
        onToggleSendShortcut = {
            val next = sendShortcutMode.next()
            sendShortcutMode = next
            saveSendShortcutMode(prefs, next)
        },
        quoteReplies = quoteReplies,
        onToggleQuoteReplies = {
            quoteReplies = !quoteReplies
            saveAppBoolean(prefs, QUOTE_REPLIES_PREF, quoteReplies)
        },
        conversationLayout = conversationLayout,
        onToggleConversationLayout = {
            val next = conversationLayout.next()
            conversationLayout = next
            saveConversationLayout(prefs, next)
        },
        messageFontScale = messageFontScale,
        onMessageFontScaleChange = { scale ->
            messageFontScale = coerceMessageFontScale(scale)
            saveMessageFontScale(prefs, messageFontScale)
        },
        appProxy = appProxy,
        onSaveAppProxy = ::saveAppProxy,
        appSignatureHtml = appSignatureHtml,
        onSaveAppSignature = ::saveAppSignature,
        remoteImageSenders = remoteImageSenders,
        onRemoveRemoteImageSender = { addr -> setRemoteImageSender(addr, false) },
        onSaveAccountSignature = ::saveAccountSignature,
        onSaveAccountProxy = ::saveAccountProxy,
        onSaveAccountServerSettings = ::saveAccountServerSettings,
        foldersByAccount = foldersByAccount,
        onRequestAccountFolders = { account -> ensureAccountFolders(account.id) },
        onSetFolderNotify = { folder, enabled ->
            setMailFolderNotify(
                accountId = folder.accountId,
                folderId = folder.name,
                folderName = folder.displayName,
                enabled = enabled,
            )
        },
        kanbanColumnWidth = kanbanColumnWidth,
        onCycleKanbanColumnWidth = {
            val next = nextKanbanColumnWidth(kanbanColumnWidth)
            kanbanColumnWidth = next
            saveAppInt(prefs, KANBAN_COLUMN_WIDTH_PREF, next)
        },
        notificationsNeedPermission = !notificationPermissionGranted,
        onEnableNotifications = {
            mobileHost.requestNotificationPermission()
            notificationPermissionGranted = mobileHost.notificationsEnabled()
        },
        supportsBackgroundPush = mobileHost.supportsBackgroundPush,
        liveMailPushEnabled = liveMailPushEnabled,
        onToggleLiveMailPush = {
            val next = !liveMailPushEnabled
            liveMailPushEnabled = next
            saveAppBoolean(prefs, LIVE_MAIL_PUSH_PREF, next)
            mobileHost.syncLiveMailPush(next)
            status = trs(if (next) "mobile.status.liveMailPushEnabled" else "mobile.ios.liveMailPushDisabled")
        },
        backgroundSyncEnabled = backgroundSyncEnabled,
        onToggleBackgroundSync = {
            val next = !backgroundSyncEnabled
            backgroundSyncEnabled = next
            saveAppBoolean(prefs, BACKGROUND_SYNC_ENABLED_PREF, next)
            mobileHost.syncBackgroundRefresh(next)
            status = trs(if (next) "mobile.status.backgroundSyncEnabled" else "mobile.status.backgroundSyncDisabled")
        },
        onRefreshBackground = {
            mobileHost.runBackgroundRefreshOnce()
            status = trs("mobile.status.backgroundRefreshQueued")
        },
        readDiagnosticLog = { mobileHost.readDiagnosticLog() },
        onShareDiagnosticLog = { mobileHost.shareDiagnosticLog() },
        pollIntervalMinutes = pollIntervalMinutes,
        onCyclePollInterval = {
            val next = nextPollIntervalMinutes(pollIntervalMinutes)
            pollIntervalMinutes = next
            saveAppInt(prefs, POLL_INTERVAL_MINUTES_PREF, next)
        },
        storageUsage = storageUsage,
        storageBusy = storageBusy,
        storageClearConfirming = storageClearConfirming,
        onRefreshStorage = { loadStorageUsage(showStatus = true) },
        onClearStorageCache = ::clearStorageCache,
        onExportBackup = {
            backupPassphraseError = ""
            backupPassphraseMode = BackupPassphraseMode.Export
        },
        onRestoreBackup = pickBackupFile,
        backupBusy = backupBusy,
    )
    backupPassphraseMode?.let { mode ->
        BackupPassphraseDialog(
            mode = mode,
            busy = backupBusy,
            error = backupPassphraseError,
            onDismiss = ::closeBackupPassphrase,
            onConfirm = { passphrase, includeSecrets ->
                when (mode) {
                    BackupPassphraseMode.Export -> exportBackup(includeSecrets, passphrase)
                    BackupPassphraseMode.Restore -> retryBackupRestore(passphrase)
                }
            },
        )
    }
}
