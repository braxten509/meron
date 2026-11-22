// Coordinates mobile changes to configured accounts
// and their connection state. Account setup and
// settings views use these operations to reach the
// shared core.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AccountAliasParams
import jp.nonbili.meron.shared.AccountAliasesParams
import jp.nonbili.meron.shared.AccountAvatarParams
import jp.nonbili.meron.shared.AccountChatWallpaperParams
import jp.nonbili.meron.shared.AccountFlagParams
import jp.nonbili.meron.shared.AccountIdParams
import jp.nonbili.meron.shared.AccountNameParams
import jp.nonbili.meron.shared.AccountReorderParams
import jp.nonbili.meron.shared.AccountRssSyncIntervalParams
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.AddRssAccountParams
import jp.nonbili.meron.shared.ExportOpmlParams
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.accountSummaryIsRss
import jp.nonbili.meron.shared.parseAccountListResponse
import jp.nonbili.meron.shared.parseOpmlExportResponse
import jp.nonbili.meron.shared.parseStorageUsageResponse
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Core reported that it refused to use this account's sign-in (missing, or rejected by the provider).
 *
 * Take the account's reconnect state from a fresh account list, so the drawer and the reconnect banner
 * show it now — and so a report that was overtaken by a reconnect, about credentials already replaced,
 * changes nothing. Watchers repeat the report while they retry; only an account not yet flagged is
 * looked up.
 */
internal fun MeronMobileState.refreshAccountReconnectState(accountId: String) {
    if (!coreLoaded || coreAccounts.none { it.id == accountId && !it.needsReconnect }) return
    scope.launch {
        runCatching {
            withContext(ioDispatcher) { parseAccountListResponse(MobileMailCommandClient(core).listAccounts()) }
        }.onSuccess { fresh ->
            if (fresh.any { it.id == accountId && it.needsReconnect }) {
                coreAccounts = coreAccounts.map { if (it.id == accountId) it.copy(needsReconnect = true) else it }
            }
        }
    }
}

internal fun MeronMobileState.applyAccounts(
    json: String,
    preferEmail: String? = null,
) {
    accountJson = json
    val parsed = parseAccountListResponse(json)
    coreAccounts = parsed
    // Account data is now in state. Mark accounts as loaded so the blocking
    // inbox loader clears even on paths that bypass listAccounts() — e.g. the
    // OAuth exchange after the custom-tab round-trip recreates the state with
    // initialAccountsLoaded=false.
    initialAccountsLoaded = true
    accountsLoading = false
    val previousAccountId = selectedCoreAccountId
    selectedCoreAccountId = preferEmail?.let { wanted -> parsed.firstOrNull { it.email == wanted }?.id }
        ?: selectedCoreAccountId.takeIf { sel ->
            sel == UNIFIED_ACCOUNT_ID || sel == MERON_FOLDERS_ACCOUNT_ID || parsed.any { it.id == sel }
        }
        ?: UNIFIED_ACCOUNT_ID
    if (selectedCoreAccountId != previousAccountId) {
        selectedCoreFolder = loadMailFolderForAccount(prefs, selectedCoreAccountId)
    }
    saveLastMailLocation(prefs, selectedCoreAccountId, selectedCoreFolder)
    // The drawer's Meron folders (and a restored Meron folder view) need the list.
    refreshMeronFolders()
    kanbanBoards = ensureKanbanDefaults(kanbanPrefs, kanbanBoards, parsed)
    if (activeKanbanBoardId.isBlank() || kanbanBoards.none { it.id == activeKanbanBoardId }) {
        activeKanbanBoardId = kanbanBoards.firstOrNull()?.id.orEmpty()
        saveActiveKanbanBoardId(kanbanPrefs, activeKanbanBoardId)
    }
}

internal fun findOAuthResultAccount(
    accounts: List<AccountSummary>,
    previousAccountIds: Set<String>,
    provider: String,
    preferredEmail: String,
): AccountSummary? {
    val normalizedProvider = provider.trim().lowercase()
    val preferred = preferredEmail.trim()
    val providerMatches =
        accounts.filter {
            it.provider.equals(normalizedProvider, ignoreCase = true) ||
                it.authType.equals("${normalizedProvider}_oauth", ignoreCase = true)
        }
    return providerMatches.firstOrNull { it.id !in previousAccountIds }
        ?: preferred.takeIf { it.isNotBlank() }?.let { email ->
            accounts.firstOrNull { it.email.equals(email, ignoreCase = true) }
        }
        ?: providerMatches.firstOrNull()
}

internal fun MeronMobileState.listAccounts(): Job? {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        initialAccountsLoaded = true
        return null
    }
    val generation = ++accountLoadGeneration
    accountsLoading = true
    return scope.launch {
        runCatching {
            withContext(ioDispatcher) { MobileMailCommandClient(core).listAccounts() }
        }.onSuccess {
            if (generation != accountLoadGeneration) return@onSuccess
            applyAccounts(it)
            mobileHost.syncLiveMailPush(liveMailPushEnabled)
        }.onFailure {
            if (generation != accountLoadGeneration) return@onFailure
            status = trsError("mobile.status.accountListFailed", it)
        }
        if (generation == accountLoadGeneration) {
            accountsLoading = false
            initialAccountsLoaded = true
        }
    }
}

internal fun MeronMobileState.loadStorageUsage(showStatus: Boolean = false) {
    if (!coreLoaded) return
    scope.launch {
        storageBusy = true
        runCatching {
            withContext(ioDispatcher) {
                MobileMailCommandClient(core).storageUsage()
            }
        }.onSuccess {
            storageUsage = parseStorageUsageResponse(it)
            if (showStatus) status = trs("mobile.status.storageUsageLoaded")
        }.onFailure {
            if (showStatus) status = trsError("mobile.status.storageUsageFailed", it)
        }
        storageBusy = false
    }
}

internal fun MeronMobileState.clearStorageCache() {
    if (!storageClearConfirming) {
        storageClearConfirming = true
        status = trs("mobile.status.tapClearCacheAgain")
        return
    }
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    storageClearConfirming = false
    scope.launch {
        storageBusy = true
        runCatching {
            withContext(ioDispatcher) {
                MobileMailCommandClient(core).clearStorageCache()
            }
        }.onSuccess {
            storageUsage = parseStorageUsageResponse(it)
            status = trs("mobile.status.cacheCleared")
        }.onFailure {
            status = trsError("mobile.status.clearCacheFailed", it)
        }
        storageBusy = false
    }
}

internal fun MeronMobileState.addRssAccount() {
    if (rssAccountAdding) return
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    rssAccountAdding = true
    status = trs("mobile.status.addingRssAccount")
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                client.addRssAccount(AddRssAccountParams(feedUrl = rssFeedUrl.trim(), displayName = rssDisplayName.trim()))
                client.listAccounts()
            }
        }.onSuccess { json ->
            rssAccountAdding = false
            val parsedNew = parseAccountListResponse(json)
            val oldIds = coreAccounts.map { it.id }.toSet()
            val newRssAccount = parsedNew.firstOrNull { it.id !in oldIds && accountSummaryIsRss(it) }
            if (newRssAccount != null) {
                // Switch away from Unified before publishing the refreshed account
                // list, so effects observing coreAccounts load the new RSS mailbox.
                selectCoreMailbox(newRssAccount.id, INBOX_FOLDER)
            }
            applyAccounts(json)
            rssDisplayName = ""
            rssFeedUrl = ""
            screen = Screen.Mail
            status = trs("mobile.status.rssAccountAdded")
            // account.addRss already fetched and stored the starter feed's items,
            // so re-fetching here would be a redundant (and slow) network round-trip.
            syncCoreThreads(
                accountOverride = selectedCoreAccountId,
                folderOverride = INBOX_FOLDER,
                syncFirst = false,
                successStatus = trs("mobile.status.rssAccountAdded"),
            )
        }.onFailure {
            rssAccountAdding = false
            status = trsError("mobile.status.addRssFailed", it)
        }
    }
}

internal fun nextRssAccountDisplayName(accounts: List<AccountSummary>): String {
    val names =
        accounts
            .filter(::accountSummaryIsRss)
            .map {
                it.displayName
                    .ifBlank { it.email }
                    .trim()
                    .lowercase()
            }.toSet()
    var suffix = 0
    while (true) {
        val candidate = if (suffix == 0) "RSS" else "RSS$suffix"
        if (candidate.lowercase() !in names) return candidate
        suffix += 1
    }
}

internal fun MeronMobileState.exportOpmlForSelectedAccount() {
    val accountId = selectedCoreAccountId
    if (accountId == UNIFIED_ACCOUNT_ID || accountId.isBlank()) {
        status = trs("mobile.status.selectRssAccountFirst")
        return
    }
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                MobileMailCommandClient(core).exportOpml(ExportOpmlParams(accountId = accountId))
            }
        }.onSuccess {
            val opml = parseOpmlExportResponse(it)
            if (opml.isBlank()) {
                status = trs("mobile.status.noOpmlToExport")
            } else {
                pendingOpmlExport = opml
                launchOpmlExport("meron-feeds.opml")
            }
        }.onFailure {
            status = trsError("mobile.status.opmlExportFailed", it)
        }
    }
}

internal fun MeronMobileState.saveAccountSettings(
    account: AccountSummary,
    displayName: String,
    senderName: String,
    avatarUrl: String,
    wallpaperPresetId: String,
    loadRemoteImages: Boolean,
    conversationHtml: Boolean,
    includedInUnified: Boolean,
    muted: Boolean,
    paused: Boolean,
    rssSyncIntervalMinutes: Int,
    aliasesText: String,
) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val aliases =
        aliasesText
            .lineSequence()
            .map { it.trim() }
            .filter { it.isNotBlank() }
            .map { line ->
                val parts = line.split(",", limit = 2).map { it.trim() }
                AccountAliasParams(email = parts[0], name = parts.getOrElse(1) { "" })
            }.filter { it.email.isNotBlank() }
            .toList()
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                client.setAccountName(AccountNameParams(account.id, displayName.trim()))
                client.setAccountAvatar(AccountAvatarParams(account.id, avatarUrl.trim()))
                client.setAccountChatWallpaper(AccountChatWallpaperParams(account.id, presetId = wallpaperPresetId.trim()))
                if (!accountSummaryIsRss(account)) {
                    client.setAccountSenderName(AccountNameParams(account.id, senderName.trim()))
                    client.setAccountAliases(AccountAliasesParams(account.id, aliases))
                }
                client.setAccountImages(AccountFlagParams(account.id, loadRemoteImages))
                client.setAccountConversationHtml(AccountFlagParams(account.id, conversationHtml))
                client.setAccountUnified(AccountFlagParams(account.id, includedInUnified))
                client.setAccountMuted(AccountFlagParams(account.id, muted))
                client.setAccountPaused(AccountFlagParams(account.id, paused))
                if (accountSummaryIsRss(account)) {
                    client.setAccountRssSyncInterval(AccountRssSyncIntervalParams(account.id, rssSyncIntervalMinutes.coerceIn(5, 1440)))
                }
                client.listAccounts()
            }
        }.onSuccess {
            applyAccounts(it)
            accountSettingsTargetId = null
            status = trs("mobile.status.accountSettingsSaved")
        }.onFailure {
            status = trsError("mobile.status.accountSettingsFailed", it)
        }
    }
}

internal fun MeronMobileState.setAccountNavigationVisible(
    account: AccountSummary,
    visible: Boolean,
) {
    hiddenNavigationAccountIds =
        if (visible) {
            hiddenNavigationAccountIds - account.id
        } else {
            hiddenNavigationAccountIds + account.id
        }
    saveAppStringSet(prefs, HIDDEN_NAV_ACCOUNTS_PREF, hiddenNavigationAccountIds)
    if (!visible && selectedCoreAccountId == account.id) {
        selectedCoreAccountId = UNIFIED_ACCOUNT_ID
        selectedCoreFolder = INBOX_FOLDER
        selectedCoreThread = null
        messages = emptyList()
        coreThreads = emptyList()
        mailboxCursor = ""
        mailboxAccountCursors = emptyMap()
        mailboxHeldThreads = emptyList()
    }
}

internal fun MeronMobileState.removeAccount(account: AccountSummary) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                client.removeAccount(AccountIdParams(account.id))
                client.listAccounts()
            }
        }.onSuccess {
            hiddenNavigationAccountIds = hiddenNavigationAccountIds - account.id
            saveAppStringSet(prefs, HIDDEN_NAV_ACCOUNTS_PREF, hiddenNavigationAccountIds)
            selectedCoreThread = null
            messages = emptyList()
            coreThreads = emptyList()
            applyAccounts(it)
            clearMailFolderForAccount(prefs, account.id)
            status = trs("mobile.status.accountRemoved")
            syncCoreThreads(syncFirst = false, successStatus = trs("mobile.status.accountRemoved"))
        }.onFailure {
            status = trsError("mobile.status.removeAccountFailed", it)
        }
    }
}

internal fun MeronMobileState.moveAccount(
    account: AccountSummary,
    delta: Int,
) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val oldIndex = coreAccounts.indexOfFirst { it.id == account.id }
    val newIndex = (oldIndex + delta).coerceIn(0, coreAccounts.lastIndex)
    if (oldIndex < 0 || oldIndex == newIndex) return
    val next = coreAccounts.toMutableList()
    val moved = next.removeAt(oldIndex)
    next.add(newIndex, moved)
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                client.reorderAccounts(AccountReorderParams(next.map { it.id }))
                client.listAccounts()
            }
        }.onSuccess {
            applyAccounts(it, preferEmail = account.email.ifBlank { account.id })
            status = trs("mobile.status.accountMoved")
        }.onFailure {
            status = trsError("mobile.status.moveAccountFailed", it)
        }
    }
}
