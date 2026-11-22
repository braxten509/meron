// Loads account and unified mailbox pages into the mobile conversation list.
// The active view selects which accounts, folders and pagination cursors participate.
// Held rows remain available when another page is requested.

package jp.nonbili.meron.ui

import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.items
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.FolderListParams
import jp.nonbili.meron.shared.FolderSummary
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.StarredItemsParams
import jp.nonbili.meron.shared.SyncMailParams
import jp.nonbili.meron.shared.SyncRssParams
import jp.nonbili.meron.shared.ThreadListParams
import jp.nonbili.meron.shared.accountSummaryIsRss
import jp.nonbili.meron.shared.parseFolderListResponse
import jp.nonbili.meron.shared.parseStarredItemsPage
import jp.nonbili.meron.shared.parseThreadListPage
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.withContext

// Accounts that still have a pagination cursor for the visible mailbox. Shared
// between loadMoreCoreThreads and the UI's canLoadMore flag so the load-more
// affordance never shows when a load would silently no-op (e.g. the only
// remaining cursors belong to accounts excluded from the Unified inbox).
internal fun pageableMailAccounts(
    selectedAccountId: String,
    accounts: List<AccountSummary>,
    mailboxCursor: String,
    // Cards a reload held back for the next "load more" (see
    // `readMailboxToOldestShown`): there is more to show even past the end.
    hasHeld: Boolean = false,
): List<AccountSummary> {
    val accountId = selectedAccountId.ifBlank { UNIFIED_ACCOUNT_ID }
    val more = mailboxCursor.isNotBlank() || hasHeld
    return if (accountId == UNIFIED_ACCOUNT_ID) {
        accounts.filter { it.includedInUnified && more }
    } else {
        accounts.filter { it.id == accountId && more }
    }
}

internal fun MeronMobileState.pageableCoreAccounts(): List<AccountSummary> =
    if (
        (selectedCoreAccountId == UNIFIED_ACCOUNT_ID && isUnifiedStarredFolder(selectedCoreFolder)) ||
        selectedCoreAccountId == MERON_FOLDERS_ACCOUNT_ID
    ) {
        coreAccounts.filter { mailboxCursor.isNotBlank() || mailboxHeldThreads.isNotEmpty() }
    } else {
        pageableMailAccounts(selectedCoreAccountId, coreAccounts, mailboxCursor, mailboxHeldThreads.isNotEmpty())
    }

/**
 * "Load more" when the last reload held cards back: show those, read nothing.
 * Their cursor is already past them, so the next "load more" reads on from it.
 */
private fun MeronMobileState.showHeldThreads(quiet: Boolean) {
    val existingIds = coreThreads.map { it.id }.toSet()
    val appended = threadRemovalGuard.filter(mailboxHeldThreads).filterNot { it.id in existingIds }
    mailboxHeldThreads = emptyList()
    coreThreads = threadRemovalGuard.filter(coreThreads + appended).sortedByDescending { it.dateEpochSeconds }
    // These cards are on screen now, so reloads keep them there.
    mailboxPageDepth = (mailboxPageDepth + appended.size).coerceAtMost(MAILBOX_MAX_RELOAD_DEPTH)
    cacheVisibleMailbox()
    if (!quiet) {
        status = if (appended.isEmpty()) trs("mobile.status.noOlderMessages") else trs("mobile.ios.loadedOlderMessageCount", mapOf("count" to appended.size))
    }
}

// `quiet` suppresses the "Loaded N older message(s)" status for auto-fired
// pagination — store reloads (e.g. after a background sync event) shrink the
// list back to its first page, and the resulting refetch chain would otherwise
// toast once per page.
internal fun MeronMobileState.loadMoreCoreThreads(quiet: Boolean = false) {
    if (!coreLoaded || loadingMoreThreads) return
    val accountId = selectedCoreAccountId.ifBlank { UNIFIED_ACCOUNT_ID }
    val requestedFolder = selectedCoreFolder.ifBlank { INBOX_FOLDER }
    val query = mailSearch
    val filter = mailFilter
    val attachmentsOnly = mailAttachmentsOnly
    val groupBySender = mailGroupBySender
    val selectedAccounts = pageableCoreAccounts()
    if (selectedAccounts.isEmpty()) return
    if (mailboxHeldThreads.isNotEmpty()) {
        showHeldThreads(quiet)
        return
    }
    loadingMoreThreads = true
    val folderGeneration = mailboxFolderGeneration
    scope.launchThreadListRead(threadRemovalGuard) { read ->
        val folderReadVersion = folderReadGuard.version
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                if (accountId == MERON_FOLDERS_ACCOUNT_ID) {
                    loadMeronFolder(
                        client = client,
                        folderId = requestedFolder,
                        query = query,
                        filter = filter,
                        attachmentsOnly = attachmentsOnly,
                        beforeCursor = mailboxCursor,
                    )
                } else if (accountId == UNIFIED_ACCOUNT_ID && isUnifiedStarredFolder(requestedFolder)) {
                    loadUnifiedStarred(client = client, query = query, filter = filter, attachmentsOnly = attachmentsOnly, beforeCursor = mailboxCursor)
                } else if (accountId == UNIFIED_ACCOUNT_ID) {
                    loadUnifiedInbox(
                        client = client,
                        accounts = selectedAccounts,
                        query = query,
                        filter = filter,
                        attachmentsOnly = attachmentsOnly,
                        groupBySender = groupBySender,
                        syncFirst = false,
                        beforeCursor = mailboxCursor,
                        folderRole = requestedFolder,
                    )
                } else {
                    loadAccountInbox(
                        client,
                        selectedAccounts.first(),
                        requestedFolder,
                        query = query,
                        filter = filter,
                        attachmentsOnly = attachmentsOnly,
                        groupBySender = groupBySender,
                        syncFirst = false,
                        beforeCursor = mailboxCursor,
                    )
                }
            }
        }.onSuccess { result ->
            // A page read before a folder rename holds the old names; drop it.
            if (folderGeneration != mailboxFolderGeneration) {
                loadingMoreThreads = false
                return@onSuccess
            }
            if (result.folders.isNotEmpty()) {
                coreFolders = reconcileFolderUnread(result.folders, folderReadVersion)
                foldersByAccount = foldersByAccount + reconcileFolderUnread(result.folders, folderReadVersion).groupBy { it.accountId }
            }
            val existingIds = coreThreads.map { it.id }.toSet()
            val appended = withLocalDraftFlags(read.filter(result.threads)).filterNot { it.id in existingIds }
            coreThreads = read.filter(coreThreads + appended).sortedByDescending { it.dateEpochSeconds }
            mailboxCursor = result.nextCursor
            mailboxAccountCursors = result.accountCursors
            // One more page is on screen, so event-driven reloads have to re-read
            // this deep to keep it there.
            mailboxPageDepth = (mailboxPageDepth + MAILBOX_PAGE_SIZE).coerceAtMost(MAILBOX_MAX_RELOAD_DEPTH)
            cacheVisibleMailbox()
            loadingMoreThreads = false
            errorBanner = null
            if (result.searchIncomplete) {
                // Older server matches could not be fetched; the list ends at
                // what this device has cached.
                status = trs("threads.searchServerUnavailable")
            } else if (!quiet) {
                status = if (appended.isEmpty()) trs("mobile.status.noOlderMessages") else trs("mobile.ios.loadedOlderMessageCount", mapOf("count" to appended.size))
            }
        }.onFailure {
            loadingMoreThreads = false
            errorBanner = it.message ?: trs("mobile.status.loadMoreFailed")
            status = trsError("mobile.status.loadMoreFailed", it)
        }
    }
}

internal suspend fun MeronMobileState.loadAccountInbox(
    client: MobileMailCommandClient,
    account: AccountSummary,
    requestedFolder: String,
    query: String = mailSearch,
    filter: FilterMode = mailFilter,
    attachmentsOnly: Boolean = mailAttachmentsOnly,
    // Off unless the mail list asks: Kanban columns and notification folders
    // stay threaded.
    groupBySender: Boolean = false,
    syncFirst: Boolean = true,
    beforeCursor: String? = null,
    syncLimit: Int = MAILBOX_SYNC_LIMIT,
    listLimit: Int = MAILBOX_PAGE_SIZE,
    refreshSearch: Boolean = true,
): MailboxLoadResult {
    // When syncFirst is false we read whatever the local (encrypted) store
    // already has — used on startup so the inbox shows instantly without a
    // server round-trip. Pull-to-sync / "Sync now" still fetch from server.
    Log.i(
        "MailLoad",
        "loadAccountInbox start account=${account.id} requestedFolder=$requestedFolder syncFirst=$syncFirst beforeCursor=${beforeCursor?.isNotBlank() == true} query=${query.isNotBlank()} filter=${filter.protocolValue()}",
    )
    if (syncFirst) {
        if (accountSummaryIsRss(account)) {
            Log.i("MailLoad", "loadAccountInbox sync rss account=${account.id}")
            client.syncRss(SyncRssParams(accountId = account.id))
        } else {
            Log.i("MailLoad", "loadAccountInbox sync mail account=${account.id} folder=$requestedFolder")
            withManagedGoogleAuth(client, account.id) {
                client.sync(
                    SyncMailParams(
                        accountId = account.id,
                        folderId = requestedFolder,
                        limit = syncLimit,
                        folders = true,
                        deferTail = true,
                    ),
                )
            }
        }
    }
    val foldersJson = client.listFolders(FolderListParams(accountId = account.id))
    val folders = parseFolderListResponse(foldersJson)
    // Server folder names are case-sensitive ("INBOX"), but the default
    // request uses "inbox"; match case-insensitively and fall back to a real
    // inbox before the first folder.
    val folder =
        folders.firstOrNull { it.name.equals(requestedFolder, ignoreCase = true) }?.name
            ?: folders.firstOrNull { it.name.equals(INBOX_FOLDER, ignoreCase = true) }?.name
            ?: folders.firstOrNull()?.name
            ?: requestedFolder
    Log.i("MailLoad", "loadAccountInbox folders account=${account.id} count=${folders.size} resolvedFolder=$folder")
    if (!accountSummaryIsRss(account) && query.isNotBlank() && refreshSearch) {
        // The refreshed search is a live IMAP operation even when syncFirst is
        // false. Keep token upkeep out of the cache-first request so local
        // matches can paint without waiting for the network.
        ensureManagedGoogleToken(client, account.id)
    }
    val threadsJson =
        client.listThreads(
            ThreadListParams(
                accountId = account.id,
                folderId = folder,
                query = query.trim(),
                filter = filter.protocolValue(),
                attachments = attachmentsOnly,
                beforeCursor = beforeCursor,
                refresh = refreshSearch,
                // Paging forward always fetches a single page; only a reload of
                // the whole mailbox re-requests the depth already on screen.
                limit = if (beforeCursor == null) listLimit else MAILBOX_PAGE_SIZE,
                groupBy = threadListGroupBy(groupBySender),
            ),
        )
    val page = parseThreadListPage(threadsJson)
    val foldersWithPageUnread =
        page.folderUnread?.let { unread ->
            folders.map { item ->
                if (item.name.equals(folder, ignoreCase = folder.equals(INBOX_FOLDER, ignoreCase = true))) {
                    item.copy(unread = unread)
                } else {
                    item
                }
            }
        } ?: folders
    Log.i(
        "MailLoad",
        "loadAccountInbox threads account=${account.id} folder=$folder count=${page.threads.size} cursor=${page.nextCursor.isNotBlank()}",
    )
    return MailboxLoadResult(
        folders = foldersWithPageUnread,
        folder = folder,
        threads = page.threads,
        unreadCount = page.folderUnread,
        nextCursor = page.nextCursor,
        folderSynced = page.folderSynced,
        searchIncomplete = page.searchIncomplete,
    )
}

internal suspend fun MeronMobileState.loadUnifiedInbox(
    client: MobileMailCommandClient,
    accounts: List<AccountSummary>,
    query: String = mailSearch,
    filter: FilterMode = mailFilter,
    attachmentsOnly: Boolean = mailAttachmentsOnly,
    groupBySender: Boolean = false,
    syncFirst: Boolean = true,
    beforeCursor: String? = null,
    syncLimit: Int = MAILBOX_SYNC_LIMIT,
    listLimit: Int = MAILBOX_PAGE_SIZE,
    refreshSearch: Boolean = true,
    /**
     * Which special-use folder the view is on. The unified view addresses
     * mailboxes by role: the core resolves each account's own Sent/Archive/…
     * and leaves out accounts whose server has none.
     */
    folderRole: String = INBOX_FOLDER,
): MailboxLoadResult {
    val role = unifiedFolderRole(folderRole)
    // Every account is checked at the same time: one after another, four
    // accounts made a refresh take four times as long.
    if (syncFirst) {
        coroutineScope {
            accounts
                .map { account ->
                    async {
                        withSyncAccountContext(account.id) {
                            if (accountSummaryIsRss(account)) {
                                if (role == INBOX_FOLDER) {
                                    client.syncRss(SyncRssParams(accountId = account.id))
                                }
                            } else {
                                withManagedGoogleAuth(client, account.id) {
                                    var accountFolders =
                                        parseFolderListResponse(client.listFolders(FolderListParams(accountId = account.id)))
                                    var targetFolder = unifiedAccountFolder(accountFolders, role)
                                    // A cold cache cannot resolve a provider-specific Sent /
                                    // Archive name. Refresh folder metadata through Inbox,
                                    // then resolve the role again before syncing its mailbox.
                                    if (targetFolder == null) {
                                        client.sync(
                                            SyncMailParams(
                                                accountId = account.id,
                                                folderId = INBOX_FOLDER,
                                                limit = syncLimit,
                                                folders = true,
                                                deferTail = true,
                                            ),
                                        )
                                        accountFolders =
                                            parseFolderListResponse(client.listFolders(FolderListParams(accountId = account.id)))
                                        targetFolder = unifiedAccountFolder(accountFolders, role)
                                    }
                                    if (targetFolder != null) {
                                        client.sync(
                                            SyncMailParams(
                                                accountId = account.id,
                                                folderId = targetFolder,
                                                limit = syncLimit,
                                                folders = true,
                                                deferTail = true,
                                            ),
                                        )
                                    } else {
                                        "{}"
                                    }
                                }
                            }
                        }
                    }
                }.awaitAll()
        }
    }
    val folders =
        accounts.flatMap { account ->
            withSyncAccountContext(account.id) {
                parseFolderListResponse(client.listFolders(FolderListParams(accountId = account.id)))
            }
        }
    if (query.isNotBlank() && refreshSearch) {
        // Unified live search fans out inside one core call and falls back per
        // account, so refresh all managed mail tokens before issuing it.
        accounts.filterNot(::accountSummaryIsRss).forEach { account ->
            ensureManagedGoogleToken(client, account.id)
        }
    }
    val page =
        parseThreadListPage(
            client.listThreads(
                ThreadListParams(
                    accountId = UNIFIED_ACCOUNT_ID,
                    folderId = role,
                    folderRole = role,
                    query = query.trim(),
                    filter = filter.protocolValue(),
                    attachments = attachmentsOnly,
                    beforeCursor = beforeCursor,
                    refresh = refreshSearch,
                    // The core fans this out per account, so the limit is a
                    // per-account depth here too — matching how load-more appends
                    // one page from each account.
                    limit = if (beforeCursor == null) listLimit else MAILBOX_PAGE_SIZE,
                    groupBy = threadListGroupBy(groupBySender),
                ),
            ),
        )
    return MailboxLoadResult(
        folders = folders,
        folder = role,
        threads = page.threads,
        // Only the inbox rolls up into the drawer's unread badge; the other
        // unified folders have no badge to keep in sync.
        unreadCount = page.folderUnread.takeIf { role == INBOX_FOLDER },
        nextCursor = page.nextCursor,
        folderSynced = page.folderSynced,
        searchIncomplete = page.searchIncomplete,
    )
}

/**
 * The unified starred listing. Unlike the other unified folders this is not a
 * mailbox any account owns: the core returns the starred *items* themselves —
 * single messages and feed entries across every account — which the list shows
 * as rows the same way a thread is shown.
 */
internal suspend fun MeronMobileState.loadUnifiedStarred(
    client: MobileMailCommandClient,
    query: String = mailSearch,
    filter: FilterMode = mailFilter,
    attachmentsOnly: Boolean = mailAttachmentsOnly,
    beforeCursor: String? = null,
    limit: Int = MAILBOX_PAGE_SIZE,
): MailboxLoadResult {
    val page =
        parseStarredItemsPage(
            client.listStarredItems(
                StarredItemsParams(
                    query = query.trim(),
                    filter = filter.protocolValue(),
                    attachments = attachmentsOnly,
                    limit = limit,
                    beforeCursor = beforeCursor,
                ),
            ),
        )
    val rows = page.items.map { it.toThreadSummary() }
    return MailboxLoadResult(
        folders = emptyList(),
        folder = STARRED_FOLDER,
        threads = rows,
        nextCursor = page.nextCursor,
    )
}

internal suspend fun MeronMobileState.loadAccountFolders(
    client: MobileMailCommandClient,
    account: AccountSummary,
): List<FolderSummary> {
    val foldersJson = client.listFolders(FolderListParams(accountId = account.id))
    return parseFolderListResponse(foldersJson)
}
