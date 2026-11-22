// Mobile mailbox state changes for manage folders.
// Called by the Compose routes and their thread actions.
// Server responses are reconciled with the local mailbox state here.
package jp.nonbili.meron.ui

import androidx.compose.material.icons.filled.Archive
import androidx.compose.material.icons.filled.Delete
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.EmptyFolderParams
import jp.nonbili.meron.shared.FolderCreateParams
import jp.nonbili.meron.shared.FolderDeleteParams
import jp.nonbili.meron.shared.FolderRenameParams
import jp.nonbili.meron.shared.FolderSetNotifyParams
import jp.nonbili.meron.shared.FolderSummary
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.SyncMailParams
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.accountSummaryIsRss
import jp.nonbili.meron.shared.parseFolderDeleteResponse
import jp.nonbili.meron.shared.parseFolderListResponse
import jp.nonbili.meron.shared.parseFolderRenameResponse
import jp.nonbili.meron.shared.requireCoreOk
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

// Permanently delete every message in a Trash or Junk folder. The core re-checks
// the folder role, so a stale menu can never empty anything else. Callers confirm
// first: there is no Trash left to restore from.
internal fun MeronMobileState.emptyMailFolder(
    accountId: String,
    folderId: String,
    column: KanbanColumnSpec? = null,
) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val account = coreAccounts.firstOrNull { it.id == accountId }
    if (accountId == UNIFIED_ACCOUNT_ID || account == null || accountSummaryIsRss(account)) return

    val threadsBefore = coreThreads
    val kanbanBefore = kanbanColumns
    val removal = suppressEmptiedFolder(accountId, folderId)
    val inFolder = { thread: ThreadSummary -> thread.accountId == accountId && thread.folder == folderId }
    coreThreads = coreThreads.filterNot(inFolder)
    kanbanColumns =
        kanbanColumns.mapValues { (_, state) ->
            state.copy(threads = state.threads.filterNot(inFolder))
        }
    selectedMailThreadIds = emptySet()
    mailSelectionMenuOpen = false

    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                requireCoreOk(
                    withManagedGoogleAuth(client, accountId) {
                        client.emptyFolder(EmptyFolderParams(accountId = accountId, folderId = folderId))
                    },
                )
            }
        }.onSuccess { response ->
            removal.complete()
            applyCoreFolderUnreadChanges(response)
            status = trs("mobile.status.folderEmptied")
            if (column != null) loadKanbanColumn(column, refresh = true) else syncCoreThreads(syncFirst = false)
        }.onFailure {
            removal.rollback()
            Log.w("Mail", "empty folder failed", it)
            coreThreads = removal.filter(threadsBefore)
            kanbanColumns = kanbanBefore.mapValues { (_, state) -> state.copy(threads = removal.filter(state.threads)) }
            status = trsError("mobile.status.emptyFolderFailed", it)
        }
    }
}

// Delete a folder on the server, with everything nested under it, their mail and
// any board column showing one of them. The core re-checks that no special-use
// folder is in the subtree, so a stale menu can never delete Sent or Archive.
// Callers confirm first: the server keeps no copy of what the folders held.
internal fun MeronMobileState.deleteMailFolder(
    accountId: String,
    folderId: String,
    column: KanbanColumnSpec? = null,
) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val account = coreAccounts.firstOrNull { it.id == accountId }
    if (accountId == UNIFIED_ACCOUNT_ID || account == null || accountSummaryIsRss(account)) return

    // The delete takes the subtree with it, so the local cleanup below has to
    // cover every folder under it, not just the one the menu named.
    val expectedRemoved =
        (nestedFolders(foldersByAccount[accountId].orEmpty(), accountId, folderId).map { it.name } + folderId)
            .toSet()

    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                val response =
                    requireCoreOk(
                        withManagedGoogleAuth(client, accountId) {
                            client.deleteFolder(FolderDeleteParams(accountId = accountId, folderId = folderId))
                        },
                    )
                parseFolderDeleteResponse(response) to loadAccountFolders(client, account)
            }
        }.onSuccess { (result, folders) ->
            // New cores report the exact successful prefix when a later DELETE
            // fails. Older cores omit it after a complete success.
            val removed = result.removed.ifEmpty { expectedRemoved }
            foldersByAccount = foldersByAccount + (accountId to folders)
            coreFolders = coreFolders.filterNot { it.accountId == accountId && it.name in removed }
            val inFolder = { thread: ThreadSummary -> thread.accountId == accountId && thread.folder in removed }
            coreThreads = coreThreads.filterNot(inFolder)
            selectedMailThreadIds = emptySet()
            mailSelectionMenuOpen = false
            removed.forEach { removeKanbanColumnsForFolder(accountId, it) }
            // Core stopped watching the deleted folders; have the live-push
            // service drop the ones it was counting on.
            mobileHost.syncLiveMailPush(liveMailPushEnabled)
            // The mailbox view may have been sitting in any folder that just went away.
            if (selectedCoreAccountId == accountId && selectedCoreFolder in removed) {
                selectCoreMailbox(accountId, INBOX_FOLDER)
                syncCoreThreads(accountOverride = accountId, folderOverride = INBOX_FOLDER, syncFirst = false)
            }
            status = result.warning ?: trs("mobile.status.folderDeleted")
        }.onFailure {
            Log.w("Mail", "delete folder failed", it)
            status = trsError("mobile.status.deleteFolderFailed", it)
        }
    }
}

/** Ask for a new folder's name in [account]; see [createMailFolder]. */
internal fun MeronMobileState.promptNewFolder(
    account: AccountSummary,
    placement: NewFolderPlacement,
) {
    folderNameInput = ""
    pendingNewFolder = NewFolderTarget(account, placement)
}

// Create a folder on the server and show it where the action asked for it: a
// new board column, in place of a column, or in the mail view.
internal fun MeronMobileState.createMailFolder(
    target: NewFolderTarget,
    name: String,
) {
    val trimmed = name.trim()
    if (trimmed.isBlank()) {
        status = trs("folders.nameRequired")
        return
    }
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val account = target.account
    scope.launch {
        val folderReadVersion = folderReadGuard.version
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                withManagedGoogleAuth(client, account.id) {
                    client.createFolder(FolderCreateParams(accountId = account.id, name = trimmed))
                }
                loadAccountFolders(client, account)
            }
        }.onSuccess { folders ->
            val reconciled = reconcileFolderUnread(folders, folderReadVersion)
            foldersByAccount = foldersByAccount + (account.id to reconciled)
            if (selectedCoreAccountId == account.id) coreFolders = reconciled
            val created = folders.folderCreatedAs(trimmed)?.name ?: trimmed
            // A slow server may answer after the user went elsewhere; the folder
            // is made either way, but the view stays where they are.
            when (val placement = target.placement) {
                is NewFolderPlacement.AddColumn -> {
                    if (activeKanbanBoardId == placement.boardId) {
                        addKanbanColumn(KanbanColumnSpec(account.id, created))
                    }
                }

                is NewFolderPlacement.SwitchColumn -> {
                    if (activeKanbanBoardId == placement.boardId) {
                        switchKanbanColumnFolder(placement.column, created)
                    }
                }

                is NewFolderPlacement.OpenMailbox -> {
                    if (selectedCoreAccountId == account.id && selectedCoreFolder == placement.folderId) {
                        selectCoreMailbox(account.id, created)
                        syncCoreThreads(accountOverride = account.id, folderOverride = created, syncFirst = false)
                    }
                }
            }
            if (pendingNewFolder == target) pendingNewFolder = null
            folderNameInput = ""
            status = trs("mobile.status.folderCreated")
        }.onFailure {
            status = trsError("mobile.status.createFolderFailed", it)
        }
    }
}

/** Ask for [folder]'s new name; see [renameMailFolder]. */
internal fun MeronMobileState.promptRenameFolder(folder: FolderSummary) {
    val leaf = folderLeafName(folder)
    folderNameInput = leaf
    pendingRenameFolder =
        RenameFolderTarget(
            accountId = folder.accountId,
            folderId = folder.name,
            folderName = leaf,
            parent = folder.displayName.removeSuffix(leaf),
        )
}

// Rename a folder on the server, subfolders included. The folder stays where it
// is nested; only its own name changes. Whatever showed a folder that moved
// follows it: the mail view, board columns and recent move/copy targets. The
// core re-checks that no special-use folder is in the subtree.
internal fun MeronMobileState.renameMailFolder(
    target: RenameFolderTarget,
    name: String,
) {
    val trimmed = name.trim()
    if (trimmed.isBlank()) {
        status = trs("folders.nameRequired")
        return
    }
    if (trimmed == target.folderName) {
        pendingRenameFolder = null
        return
    }
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val accountId = target.accountId
    val account = coreAccounts.firstOrNull { it.id == accountId }
    if (accountId == UNIFIED_ACCOUNT_ID || account == null || accountSummaryIsRss(account)) return

    scope.launch {
        val folderReadVersion = folderReadGuard.version
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                val response =
                    requireCoreOk(
                        withManagedGoogleAuth(client, accountId) {
                            client.renameFolder(
                                FolderRenameParams(
                                    accountId = accountId,
                                    folderId = target.folderId,
                                    name = target.parent + trimmed,
                                ),
                            )
                        },
                    )
                parseFolderRenameResponse(response) to loadAccountFolders(client, account)
            }
        }.onSuccess { (renames, folders) ->
            // Mailbox and column reads still in flight hold the old names.
            mailboxFolderGeneration += 1
            val reconciled = reconcileFolderUnread(folders, folderReadVersion)
            foldersByAccount = foldersByAccount + (accountId to reconciled)
            if (selectedCoreAccountId == accountId) coreFolders = reconciled
            // A thread's id names its old folder, and the core fetches the
            // folder's mail again under the new name.
            coreThreads = coreThreads.filterNot { it.accountId == accountId && it.folder in renames }
            // So does an open conversation's, wherever it was opened from; a
            // read still in flight for it is dropped along with it.
            if (selectedCoreThread?.let { it.accountId == accountId && it.folder in renames } == true) {
                selectedCoreThread = null
                messages = emptyList()
            }
            selectedMailThreadIds = emptySet()
            mailSelectionMenuOpen = false
            renameKanbanColumnsForFolder(accountId, renames)
            renameRecentTransferTargets(accountId, renames)
            renameMailFolderForAccount(prefs, accountId, renames)
            // The core moved the opt-ins along; have the live-push service
            // watch the new names.
            mobileHost.syncLiveMailPush(liveMailPushEnabled)
            // Cached mailboxes hold the old folder list (and threads under the
            // old names), which restoring one would write back.
            mailboxCache = mailboxCache.filterKeys { it.accountId != accountId && it.accountId != UNIFIED_ACCOUNT_ID }
            val renamedSelected = renames[selectedCoreFolder].takeIf { selectedCoreAccountId == accountId }
            if (renamedSelected != null) {
                selectCoreMailbox(accountId, renamedSelected)
                // The rename dropped the folder's cached mail, so fetch it from
                // the server rather than reading an empty cache.
                syncCoreThreads(accountOverride = accountId, folderOverride = renamedSelected)
            } else if (selectedCoreAccountId == accountId || selectedCoreAccountId == UNIFIED_ACCOUNT_ID) {
                // Supersedes any read in flight, which would show the old names.
                syncCoreThreads(syncFirst = false)
            }
            resyncRenamedFolders(account, renames.values - setOfNotNull(renamedSelected))
            if (pendingRenameFolder == target) pendingRenameFolder = null
            folderNameInput = ""
            status = trs("folders.rename.done", mapOf("folder" to trimmed))
        }.onFailure {
            Log.w("Mail", "rename folder failed", it)
            status = trsError("folders.rename.failed", it, mapOf("folder" to target.folderName))
        }
    }
}

// The rename dropped every moved folder's cached mail, so their unread counts
// and starred mail are missing until each is fetched again: fetch them in the
// background, then refresh the folder list's counts and the views showing
// them. Best effort; opening a folder fetches it anyway.
private fun MeronMobileState.resyncRenamedFolders(
    account: AccountSummary,
    folderIds: Collection<String>,
) {
    if (folderIds.isEmpty() || account.paused || account.needsReconnect) return
    scope.launch {
        val folderReadVersion = folderReadGuard.version
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                folderIds.forEach { folderId ->
                    runCatching {
                        withManagedGoogleAuth(client, account.id) {
                            client.sync(SyncMailParams(accountId = account.id, folderId = folderId, folders = false))
                        }
                    }.onFailure { Log.w("Mail", "resync renamed folder failed", it) }
                }
                loadAccountFolders(client, account)
            }
        }.onSuccess { folders ->
            val reconciled = reconcileFolderUnread(folders, folderReadVersion)
            foldersByAccount = foldersByAccount + (account.id to reconciled)
            if (selectedCoreAccountId == account.id) coreFolders = reconciled
            // These syncs raise no mail event, so re-read the views that can
            // show the fetched mail (Starred, unified) from the cache.
            if (selectedCoreAccountId == account.id || selectedCoreAccountId == UNIFIED_ACCOUNT_ID) {
                syncCoreThreads(syncFirst = false)
            }
            kanbanBoards
                .firstOrNull { it.id == activeKanbanBoardId }
                ?.columns
                ?.filter { it.accountId == account.id || it.accountId == UNIFIED_ACCOUNT_ID }
                ?.forEach { loadKanbanColumn(it) }
        }
    }
}

/** Opt a folder in to (or out of) background sync and new-mail notifications. */
internal fun MeronMobileState.setMailFolderNotify(
    accountId: String,
    folderId: String,
    folderName: String,
    enabled: Boolean,
) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val account = coreAccounts.firstOrNull { it.id == accountId }
    if (accountId == UNIFIED_ACCOUNT_ID || account == null || accountSummaryIsRss(account)) return

    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                parseFolderListResponse(
                    requireCoreOk(
                        client.setFolderNotify(
                            FolderSetNotifyParams(accountId = accountId, folderId = folderId, enabled = enabled),
                        ),
                    ),
                )
            }
        }.onSuccess { folders ->
            foldersByAccount = foldersByAccount + (accountId to folders)
            coreFolders =
                coreFolders.map {
                    if (it.accountId == accountId && it.name == folderId) it.copy(notify = enabled) else it
                }
            // The live-push service watches opted-in folders; have it re-read them.
            mobileHost.syncLiveMailPush(liveMailPushEnabled)
            status =
                trs(
                    if (enabled) "folders.notify.enabled" else "folders.notify.disabled",
                    mapOf("folder" to folderName),
                )
        }.onFailure {
            Log.w("Mail", "set folder notify failed", it)
            status = it.message ?: trs("folders.notify.failed", mapOf("folder" to folderName))
        }
    }
}
