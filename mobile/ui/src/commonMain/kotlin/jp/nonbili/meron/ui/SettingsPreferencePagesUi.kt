// Renders general, message and composer preferences in the mobile settings screen.
// The settings router provides the current values and change handlers.
// Each page groups related controls and uses the shared settings rows.

package jp.nonbili.meron.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Chat
import androidx.compose.material.icons.automirrored.filled.Reply
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.BugReport
import androidx.compose.material.icons.filled.Checklist
import androidx.compose.material.icons.filled.DarkMode
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Event
import androidx.compose.material.icons.filled.FitScreen
import androidx.compose.material.icons.filled.BlurOn
import androidx.compose.material.icons.filled.FormatQuote
import androidx.compose.material.icons.filled.FormatSize
import androidx.compose.material.icons.filled.HideImage
import androidx.compose.material.icons.filled.Inbox
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.MarkEmailUnread
import androidx.compose.material.icons.filled.Notifications
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Restore
import androidx.compose.material.icons.filled.Save
import androidx.compose.material.icons.filled.Sell
import androidx.compose.material.icons.filled.SyncAlt
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.UnfoldMore
import androidx.compose.material.icons.filled.ViewKanban
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import jp.nonbili.meron.shared.ProxySpec
import jp.nonbili.meron.shared.StorageUsage

// Appearance, Side navigation, Kanban, Network, Sync & notifications and Data
// in one scrollable page; how mail is read and written have pages of their own.
@Composable
internal fun SettingsGeneralPage(
    themeChoice: ThemeChoice,
    systemDark: Boolean,
    onOpenTheme: () -> Unit,
    appLanguageTag: String,
    onOpenLanguage: () -> Unit,
    showSenderImages: Boolean,
    onToggleSenderImages: () -> Unit,
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
    kanbanColumnWidth: Int,
    onCycleKanbanColumnWidth: () -> Unit,
    appProxy: ProxySpec,
    onSaveAppProxy: (ProxySpec) -> Unit,
    notificationsNeedPermission: Boolean,
    onEnableNotifications: () -> Unit,
    supportsBackgroundPush: Boolean,
    liveMailPushEnabled: Boolean,
    onToggleLiveMailPush: () -> Unit,
    backgroundSyncEnabled: Boolean,
    onToggleBackgroundSync: () -> Unit,
    onRefreshBackground: () -> Unit,
    onOpenSyncLog: () -> Unit,
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
    onOpenSetupCopy: () -> Unit = {},
    focusProxy: Boolean = false,
    modifier: Modifier = Modifier,
) {
    // The Network label: Appearance (label + 3 rows), Side navigation (label +
    // 4 rows, plus the 2 alert defaults while the calendar is on) and Kanban
    // (2) come before it on every platform.
    val proxyIndex = if (calendarEnabled) 12 else 10
    val listState = rememberLazyListState(initialFirstVisibleItemIndex = if (focusProxy) proxyIndex else 0)
    LazyColumn(modifier.appScrollbar(listState), state = listState) {
        item { SettingsSectionLabel(tr("settings.pages.appearance")) }
        item {
            // The preview is what is painted now; while following the system
            // the label names both picks.
            val displayedAppearanceMode = themeChoice.resolve(systemDark)
            val themeLabel =
                if (themeChoice.followSystem) "${themeChoice.light.label} / ${themeChoice.dark.label}" else displayedAppearanceMode.label
            SettingsRow(
                icon = Icons.Filled.Visibility,
                title = tr("common.theme"),
                onClick = onOpenTheme,
                trailing = {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Text(
                            themeLabel,
                            color = MaterialTheme.colorScheme.primary,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.widthIn(max = 180.dp),
                        )
                        Box(
                            Modifier
                                .width(44.dp)
                                .clip(RoundedCornerShape(6.dp))
                                .border(1.dp, MaterialTheme.colorScheme.outlineVariant, RoundedCornerShape(6.dp)),
                        ) {
                            ThemePreviewMock(themePreviewColors(displayedAppearanceMode), height = 26.dp)
                        }
                    }
                },
            )
        }
        item {
            SettingsRow(
                icon = Icons.Filled.Settings,
                title = tr("settings.language.label"),
                onClick = onOpenLanguage,
                trailing = {
                    Text(
                        if (appLanguageTag.isBlank()) {
                            tr("settings.language.system")
                        } else {
                            appLanguageDisplayName(appLanguageTag)
                        },
                        color = MaterialTheme.colorScheme.primary,
                    )
                },
            )
        }
        item {
            SettingsToggleRow(
                icon = Icons.Filled.Visibility,
                title = tr("settings.appearance.showSenderImages"),
                hint = tr("settings.appearance.showSenderImagesHint"),
                checked = showSenderImages,
                onToggle = onToggleSenderImages,
            )
        }

        item { SettingsSectionLabel(tr("settings.sections.sideNav")) }
        item {
            SettingsToggleRow(
                icon = Icons.Filled.Inbox,
                title = tr("settings.sideNav.showUnifiedInbox"),
                checked = showUnifiedInboxNav,
                onToggle = onToggleUnifiedInboxNav,
            )
        }
        item {
            SettingsToggleRow(
                icon = Icons.Filled.Inbox,
                title = tr("settings.appearance.showUnreadAccountBadge"),
                hint = tr("settings.appearance.showUnreadAccountBadgeHint"),
                checked = showUnreadBadges,
                onToggle = onToggleUnreadBadges,
            )
        }
        item {
            SettingsToggleRow(
                icon = Icons.Filled.Checklist,
                title = tr("settings.tasks.enable"),
                hint = tr("settings.tasks.enableHint"),
                checked = tasksEnabled,
                onToggle = onToggleTasks,
            )
        }
        item {
            SettingsToggleRow(
                icon = Icons.Filled.Event,
                title = tr("settings.calendar.enable"),
                hint = tr("settings.calendar.enableHint"),
                checked = calendarEnabled,
                onToggle = onToggleCalendar,
            )
        }
        if (calendarEnabled) {
            item {
                AlertDefaultRow(
                    title = tr("settings.calendar.reminderLead"),
                    hint = tr("settings.calendar.reminderLeadHint"),
                    allDay = false,
                    lead = reminderLeadMinutes,
                    onChange = { onAlertDefaultChange(false, it) },
                )
            }
            item {
                AlertDefaultRow(
                    title = tr("settings.calendar.allDayAlert"),
                    hint = tr("settings.calendar.allDayAlertHint"),
                    allDay = true,
                    lead = reminderAllDayDays,
                    onChange = { onAlertDefaultChange(true, it) },
                )
            }
        }

        item { SettingsSectionLabel(tr("settings.sections.kanban")) }
        item {
            SettingsRow(
                icon = Icons.Filled.ViewKanban,
                title = tr("settings.kanban.columnWidth"),
                onClick = onCycleKanbanColumnWidth,
                trailing = { Text(trf("settings.kanban.columnWidthValue", kanbanColumnWidth), color = MaterialTheme.colorScheme.primary) },
            )
        }

        item { SettingsSectionLabel(tr("settings.sections.network")) }
        item {
            SettingsProxyRow(
                spec = appProxy,
                accountScoped = false,
                onSave = onSaveAppProxy,
            )
        }

        item { SettingsSectionLabel(tr("settings.syncNotifications")) }
        item {
            SettingsToggleRow(
                icon = Icons.Filled.Refresh,
                title = tr("settings.backgroundSync"),
                hint = tr("settings.backgroundSyncHint"),
                checked = backgroundSyncEnabled,
                onToggle = onToggleBackgroundSync,
            )
        }
        if (supportsBackgroundPush) {
            // Android: a real background channel keeps mail fresh while suspended.
            item {
                SettingsToggleRow(
                    icon = Icons.Filled.MarkEmailUnread,
                    title = tr("settings.liveMailPush"),
                    hint = tr("settings.liveMailPushHint"),
                    checked = liveMailPushEnabled,
                    onToggle = onToggleLiveMailPush,
                )
            }
            item {
                SettingsRow(
                    icon = Icons.Filled.Refresh,
                    title = tr("settings.refreshBackground"),
                    onClick = onRefreshBackground,
                )
            }
        } else {
            // iOS: no persistent background socket. Offer a foreground poll
            // interval instead; background checks fall back to best-effort.
            item {
                SettingsRow(
                    icon = Icons.Filled.Refresh,
                    title = tr("settings.pollInterval"),
                    hint = tr("settings.pollIntervalHint"),
                    onClick = onCyclePollInterval,
                    trailing = {
                        Text(
                            if (pollIntervalMinutes <= 0) {
                                tr("settings.pollIntervalOff")
                            } else {
                                tr("settings.pollIntervalValue", mapOf("minutes" to pollIntervalMinutes))
                            },
                            color = MaterialTheme.colorScheme.primary,
                        )
                    },
                )
            }
        }
        if (notificationsNeedPermission) {
            item {
                SettingsRow(
                    icon = Icons.Filled.MarkEmailUnread,
                    title = tr("mobile.accounts.enableNotifications"),
                    onClick = onEnableNotifications,
                )
            }
        }
        item {
            SettingsRow(
                icon = Icons.Filled.BugReport,
                title = tr("settings.viewSyncLog"),
                hint = tr("settings.syncDiagnosticLogHint"),
                onClick = onOpenSyncLog,
            )
        }

        item { SettingsSectionLabel(tr("settings.sections.data")) }
        item {
            SettingsRow(
                icon = Icons.Filled.SyncAlt,
                title = tr("setupCopy.title"),
                hint = tr("setupCopy.rowHint"),
                onClick = onOpenSetupCopy,
            )
        }
        item {
            SettingsRow(
                icon = Icons.Filled.Save,
                title = tr("settings.backup.exportTitle"),
                hint = tr("settings.backup.fileHint"),
                onClick = { if (!backupBusy) onExportBackup() },
            )
        }
        item {
            SettingsRow(
                icon = Icons.Filled.Restore,
                title = tr("settings.backup.restoreTitle"),
                subtitle = tr("settings.backup.exportSubtitle"),
                onClick = { if (!backupBusy) onRestoreBackup() },
            )
        }
        item {
            SettingsRow(
                icon = Icons.Filled.Info,
                title = tr("settings.storage.usageTitle"),
                subtitle =
                    storageUsage?.let {
                        trf(
                            "settings.storage.usageSummary",
                            formatBytes(it.cacheBytes),
                            formatBytes(it.dbBytes),
                        )
                    } ?: if (storageBusy) tr("common.loading") else tr("settings.storage.tapToRefresh"),
                onClick = onRefreshStorage,
            )
        }
        item {
            SettingsRow(
                icon = Icons.Filled.Delete,
                title = if (storageClearConfirming) tr("mobile.accounts.confirmClearCache") else tr("settings.storage.clearTitle"),
                subtitle = if (storageBusy) tr("settings.storage.working") else tr("settings.storage.clearCachedAttachmentsOnly"),
                onClick = onClearStorageCache,
                trailing = storageUsage?.cacheBytes?.takeIf { it > 0 }?.let { { Text(formatBytes(it)) } },
            )
        }

        item { Spacer(Modifier.height(24.dp)) }
    }
}

// How received mail is shown and handled.
@Composable
internal fun SettingsMessagesPage(
    conversationLayout: ConversationLayout,
    onToggleConversationLayout: () -> Unit,
    messageFontScale: Int,
    onOpenMessageTextSize: () -> Unit,
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
    remoteImageSenderCount: Int,
    onOpenRemoteSenders: () -> Unit,
    onOpenTags: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val listState = rememberLazyListState()
    LazyColumn(modifier.appScrollbar(listState), state = listState) {
        item { SettingsSectionLabel(tr("settings.pages.appearance")) }
        item {
            SettingsRow(
                icon = Icons.AutoMirrored.Filled.Chat,
                title = tr("settings.appearance.conversationLayout"),
                hint = tr("settings.appearance.conversationLayoutHint"),
                onClick = onToggleConversationLayout,
                trailing = { Text(conversationLayout.label(), color = MaterialTheme.colorScheme.primary) },
            )
        }
        if (conversationLayout == ConversationLayout.Chat) {
            item {
                SettingsToggleRow(
                    icon = Icons.Filled.UnfoldMore,
                    title = tr("settings.appearance.chatFullMessages"),
                    hint = tr("settings.appearance.chatFullMessagesHint"),
                    checked = chatFullMessages,
                    onToggle = onToggleChatFullMessages,
                )
            }
        }
        item {
            SettingsRow(
                icon = Icons.Filled.FormatSize,
                title = tr("settings.appearance.messageTextSize"),
                onClick = onOpenMessageTextSize,
                trailing = {
                    Text(
                        trf("settings.appearance.textSizeValue", messageFontScale),
                        color = MaterialTheme.colorScheme.primary,
                    )
                },
            )
        }
        item {
            SettingsToggleRow(
                icon = Icons.Filled.DarkMode,
                title = tr("settings.appearance.darkMessageBodies"),
                hint = tr("settings.appearance.darkMessageBodiesHint"),
                checked = darkMailBodies,
                onToggle = onToggleDarkMailBodies,
            )
        }
        if (MailWebViewFitsWideContent) {
            item {
                SettingsToggleRow(
                    icon = Icons.Filled.FitScreen,
                    title = tr("settings.appearance.autoFitMessages"),
                    hint = tr("settings.appearance.autoFitMessagesHint"),
                    checked = autoFitMessages,
                    onToggle = onToggleAutoFitMessages,
                )
            }
        }
        item {
            SettingsToggleRow(
                icon = Icons.AutoMirrored.Filled.Reply,
                title = tr("settings.appearance.readerBottomActions"),
                hint = tr("settings.appearance.readerBottomActionsHint"),
                checked = readerBottomActions,
                onToggle = onToggleReaderBottomActions,
            )
        }
        item {
            SettingsToggleRow(
                icon = Icons.Filled.BlurOn,
                title = tr("settings.appearance.blurredBars"),
                hint = tr("settings.appearance.blurredBarsHint"),
                checked = blurredBars,
                onToggle = onToggleBlurredBars,
            )
        }

        item { SettingsSectionLabel(tr("tags.title")) }
        item {
            SettingsRow(
                icon = Icons.Filled.Sell,
                title = tr("tags.title"),
                hint = tr("tags.manage.hint"),
                onClick = onOpenTags,
            )
        }

        item { SettingsSectionLabel(tr("settings.sections.privacy")) }
        item {
            SettingsRow(
                icon = Icons.Filled.HideImage,
                title = tr("settings.privacy.remoteSenders"),
                hint = tr("settings.privacy.remoteSendersHint"),
                onClick = onOpenRemoteSenders,
                trailing = {
                    Text(
                        if (remoteImageSenderCount == 0) {
                            tr("settings.privacy.remoteSendersNone")
                        } else {
                            tr("settings.privacy.remoteSendersCount", mapOf("count" to remoteImageSenderCount))
                        },
                        color = MaterialTheme.colorScheme.primary,
                    )
                },
            )
        }

        item { Spacer(Modifier.height(24.dp)) }
    }
}

@Composable
internal fun SettingsComposerPage(
    sendShortcutMode: SendShortcutMode,
    onToggleSendShortcut: () -> Unit,
    quoteReplies: Boolean,
    onToggleQuoteReplies: () -> Unit,
    appSignatureHtml: String,
    onSaveAppSignature: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val listState = rememberLazyListState()
    LazyColumn(modifier.appScrollbar(listState), state = listState) {
        item { SettingsSectionLabel(tr("settings.sections.general")) }
        item {
            SettingsRow(
                icon = Icons.AutoMirrored.Filled.Send,
                title = tr("settings.composer.sendMessageWith"),
                onClick = onToggleSendShortcut,
                trailing = { Text(sendShortcutMode.label(), color = MaterialTheme.colorScheme.primary) },
            )
        }
        item {
            SettingsToggleRow(
                icon = Icons.Filled.FormatQuote,
                title = tr("settings.composer.quoteReplies"),
                hint = tr("settings.composer.quoteRepliesHint"),
                checked = quoteReplies,
                onToggle = onToggleQuoteReplies,
            )
        }

        item { SettingsSectionLabel(tr("settings.sections.signature")) }
        item {
            SettingsSignatureRow(
                html = appSignatureHtml,
                onSave = onSaveAppSignature,
            )
        }

        item { Spacer(Modifier.height(24.dp)) }
    }
}

/** A default event alert, picked from a menu. */
@Composable
private fun AlertDefaultRow(
    title: String,
    hint: String,
    allDay: Boolean,
    lead: Int,
    onChange: (Int) -> Unit,
) {
    var menu by remember { mutableStateOf(false) }
    Box {
        SettingsRow(
            icon = Icons.Filled.Notifications,
            title = title,
            hint = hint,
            onClick = { menu = true },
            trailing = { Text(alertLabel(lead, allDay), color = MaterialTheme.colorScheme.primary) },
        )
        DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
            (listOf(NO_ALERT) + alertOptions(allDay)).forEach { option ->
                DropdownMenuItem(
                    text = { Text(alertLabel(option, allDay)) },
                    onClick = {
                        menu = false
                        if (option != lead) onChange(option)
                    },
                )
            }
        }
    }
}
