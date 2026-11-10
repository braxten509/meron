package jp.nonbili.meron.ui

import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarResult
import jp.nonbili.meron.shared.MeronFolderSummary
import jp.nonbili.meron.shared.MeronFolderThreadsParams
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.SyncMailParams
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.accountSummaryIsRss
import jp.nonbili.meron.shared.parseFolderUnreadChanges
import jp.nonbili.meron.shared.parseMeronFolderList
import jp.nonbili.meron.shared.parseMeronFolderMoveResponse
import jp.nonbili.meron.shared.parseMeronFolderResponse
import jp.nonbili.meron.shared.parseMeronUnarchived
import jp.nonbili.meron.shared.parseThreadListPage
import jp.nonbili.meron.shared.requireCoreAllOk
import jp.nonbili.meron.shared.requireCoreOk
import jp.nonbili.meron.shared.threadIdIsRss
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

// Meron folders live only in this device's store: every command but "Mark all
// read" is a local read or write, so lists update at once and never wait on a
// server. The mail list shows one as the pseudo-account
// MERON_FOLDERS_ACCOUNT_ID with the folder's id as its folder.

/** The name dialog, or the delete confirmation, the drawer or picker opened. */
internal sealed interface MeronFolderDialog {
    /** A new folder; [moveThreads] go into it once it exists (from the picker). */
    data class New(
        val moveThreads: List<ThreadSummary> = emptyList(),
    ) : MeronFolderDialog

    data class Rename(
        val folder: MeronFolderSummary,
    ) : MeronFolderDialog

    data class Delete(
        val folder: MeronFolderSummary,
    ) : MeronFolderDialog
}

internal val MeronMobileState.inMeronFolderView: Boolean get() = selectedCoreAccountId == MERON_FOLDERS_ACCOUNT_ID

internal fun MeronMobileState.meronFolderName(id: String?): String? =
    id?.let { wanted ->
        // The built-in Archive shows under its name in the reader's language.
        if (wanted == MERON_ARCHIVE_FOLDER_ID) return@let trs("folders.roles.archive")
        meronFolders.firstOrNull { it.id == wanted }?.name
    }

/** Whether [folder] is the built-in Archive (always there, can't be renamed
 *  or deleted, filled by archiving). */
internal fun isMeronArchiveFolder(folder: MeronFolderSummary): Boolean = folder.builtin == "archive" || folder.id == MERON_ARCHIVE_FOLDER_ID

/** The open Meron folder, when the mail list shows one. */
internal val MeronMobileState.openMeronFolder: MeronFolderSummary?
    get() = if (inMeronFolderView) meronFolders.firstOrNull { it.id == selectedCoreFolder } else null

/** Feeds and local mail can't go in a Meron folder. A sender row can: the
 *  core moves each of that person's conversations in the row's mailbox. */
internal fun MeronMobileState.canUseMeronFolders(thread: ThreadSummary): Boolean {
    if (threadIdIsRss(thread.id) || thread.id.contains("#rss#")) return false
    if (thread.accountId.isBlank() || thread.accountId == "local") return false
    val account = coreAccounts.firstOrNull { it.id == thread.accountId } ?: return false
    return !accountSummaryIsRss(account) && account.engine != "local"
}

/** Total unread across Meron folders. The built-in Archive's unread counts
 *  only on its own row. */
internal val MeronMobileState.meronUnreadTotal: Int get() = meronFolders.filterNot(::isMeronArchiveFolder).sumOf { it.unread }

internal fun MeronMobileState.applyMeronFolders(folders: List<MeronFolderSummary>) {
    meronFolders = folders
    // The open folder was deleted (here or by a restore): fall back to the inbox.
    if (inMeronFolderView && folders.none { it.id == selectedCoreFolder }) {
        selectCoreMailbox(UNIFIED_ACCOUNT_ID, INBOX_FOLDER)
        syncCoreThreads(accountOverride = UNIFIED_ACCOUNT_ID, folderOverride = INBOX_FOLDER, syncFirst = false)
    }
}

/** A change to Meron folders is starting: folder lists read before it are stale. */
private fun MeronMobileState.nextMeronFoldersGeneration(): Long {
    meronFoldersGeneration += 1
    return meronFoldersGeneration
}

/** The `folders` list a request answered with, unless a later change began since. */
private fun MeronMobileState.applyMeronFolderListFrom(
    generation: Long,
    response: String,
) {
    if (generation != meronFoldersGeneration) return
    parseMeronFolderList(response)?.let(::applyMeronFolders)
}

internal fun MeronMobileState.refreshMeronFolders() {
    if (!coreLoaded) return
    val generation = meronFoldersGeneration
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                requireCoreOk(MobileMailCommandClient(core).meronFolders())
            }
        }.onSuccess { response -> applyMeronFolderListFrom(generation, response) }
            .onFailure { Log.w("MeronFolders", "list failed", it) }
    }
}

/** Re-read the folder list (and so its unread badges) after a burst of sync events. */
internal fun MeronMobileState.scheduleMeronFoldersRefresh() {
    if (!coreLoaded || meronFoldersRefreshJob?.isActive == true) return
    meronFoldersRefreshJob =
        scope.launch {
            delay(MAILBOX_RELOAD_DEBOUNCE_MS)
            refreshMeronFolders()
        }
}

internal fun MeronMobileState.openMeronFolderView(folderId: String) {
    if (selectedCoreAccountId == MERON_FOLDERS_ACCOUNT_ID && selectedCoreFolder == folderId) return
    selectCoreMailbox(MERON_FOLDERS_ACCOUNT_ID, folderId)
    syncCoreThreads(accountOverride = MERON_FOLDERS_ACCOUNT_ID, folderOverride = folderId, syncFirst = false)
}

/** Patch one folder's badge from a page's `folder_unread`. */
internal fun MeronMobileState.patchMeronFolderUnread(
    folderId: String,
    unread: Int,
) {
    if (meronFolders.none { it.id == folderId && it.unread != unread }) return
    meronFolders = meronFolders.map { if (it.id == folderId) it.copy(unread = unread) else it }
}

/**
 * One page of a Meron folder's conversations. A pull-to-refresh ([syncFirst])
 * first syncs every mail account's Inbox, where new replies arrive; a failing
 * account only costs its own news.
 */
internal suspend fun MeronMobileState.loadMeronFolder(
    client: MobileMailCommandClient,
    folderId: String,
    query: String = mailSearch,
    filter: FilterMode = mailFilter,
    attachmentsOnly: Boolean = mailAttachmentsOnly,
    syncFirst: Boolean = false,
    beforeCursor: String? = null,
    limit: Int = MAILBOX_PAGE_SIZE,
): MailboxLoadResult {
    if (syncFirst) {
        coreAccounts.filterNot(::accountSummaryIsRss).forEach { account ->
            runCatching {
                withManagedGoogleAuth(client, account.id) {
                    client.sync(
                        SyncMailParams(
                            accountId = account.id,
                            folderId = INBOX_FOLDER,
                            limit = MAILBOX_FIRST_SYNC_LIMIT,
                            folders = false,
                            deferTail = true,
                        ),
                    )
                }
            }.onFailure { Log.w("MeronFolders", "inbox sync before folder load failed account=${account.id}", it) }
        }
    }
    val page =
        parseThreadListPage(
            requireCoreOk(
                client.meronFolderThreads(
                    MeronFolderThreadsParams(
                        folderId = folderId,
                        query = query.trim(),
                        filter = filter.protocolValue(),
                        attachments = attachmentsOnly,
                        limit = if (beforeCursor == null) limit else MAILBOX_PAGE_SIZE,
                        beforeCursor = beforeCursor,
                    ),
                ),
            ),
        )
    return MailboxLoadResult(
        folders = emptyList(),
        folder = folderId,
        threads = page.threads,
        unreadCount = page.folderUnread,
        nextCursor = page.nextCursor,
    )
}

/** Unread counts of the server mailboxes a Meron change touched. */
private fun MeronMobileState.applyMeronFolderCounts(response: String) {
    val changes =
        parseFolderUnreadChanges(response).map { change ->
            change.copy(unread = folderReadGuard.recordMutation(change.accountId, change.folderId, change.unread))
        }
    if (changes.isEmpty()) return
    val patch = { folder: jp.nonbili.meron.shared.FolderSummary ->
        changes
            .firstOrNull { it.accountId == folder.accountId && folder.name.equals(it.folderId, ignoreCase = folder.role == INBOX_FOLDER) }
            ?.let { folder.copy(unread = it.unread) } ?: folder
    }
    foldersByAccount = foldersByAccount.mapValues { (_, folders) -> folders.map(patch) }
    coreFolders = coreFolders.map(patch)
}

/** Whether the list on screen drops a conversation that moved into, or out
 *  of, a Meron folder. Search and the all-accounts Starred list show them
 *  anyway (with the "In ‹name›" chip); Drafts, Trash and Junk never hide. */
private fun MeronMobileState.visibleListDropsMoved(
    thread: ThreadSummary,
    movingOut: Boolean,
): Boolean {
    if (inMeronFolderView) return true
    if (movingOut) return false
    if (mailSearch.isNotBlank() || visibleMailboxKey?.query?.isNotBlank() == true) return false
    if (selectedCoreAccountId == UNIFIED_ACCOUNT_ID && isUnifiedStarredFolder(selectedCoreFolder)) return false
    return thread.folderRole !in setOf("drafts", "trash", "junk")
}

private fun kanbanColumnDropsMoved(
    key: String,
    boardSearch: String,
    thread: ThreadSummary,
): Boolean {
    if (boardSearch.isNotBlank() || key.split("\n").size > 2) return false
    val (accountId, folderId) = kanbanKeyAccountFolder(key)
    if (accountId == UNIFIED_ACCOUNT_ID && isUnifiedStarredFolder(folderId)) return false
    return thread.folderRole !in setOf("drafts", "trash", "junk")
}

/** The rows on screen after [threads] moved: dropped where the list hides them,
 *  otherwise kept with their new Meron folder. A sender row kept on screen
 *  stays as it is: it is no conversation, and the re-read after the move
 *  shows what is left of it. */
private fun MeronMobileState.applyMeronMoveLocally(
    threads: List<ThreadSummary>,
    targetFolderId: String?,
) {
    val byId = threads.associateBy { it.id }
    val dropped = threads.filter { visibleListDropsMoved(it, movingOut = targetFolderId == null) }.map { it.id }.toSet()
    coreThreads =
        coreThreads.mapNotNull { row ->
            when {
                row.id in dropped -> null
                row.id in byId && !row.isSenderGroup -> row.copy(meronFolderId = targetFolderId)
                else -> row
            }
        }
    // Board columns are never Meron folders: a move in drops rows where the
    // column hides them; a move out only clears their Meron folder (the rows
    // it brings back come with the board reload after the move).
    kanbanColumns =
        kanbanColumns.mapValues { (key, state) ->
            val next =
                state.threads.mapNotNull { row ->
                    val moved = byId[row.id] ?: return@mapNotNull row
                    if (targetFolderId != null && kanbanColumnDropsMoved(key, kanbanSearch, moved)) {
                        null
                    } else if (moved.isSenderGroup) {
                        row
                    } else {
                        row.copy(meronFolderId = targetFolderId)
                    }
                }
            val unreadDelta = loadedUnreadCount(next) - loadedUnreadCount(state.threads)
            state.copy(threads = next, unreadCount = state.unreadCount?.let { (it + unreadDelta).coerceAtLeast(0) })
        }
    selectedCoreThread =
        selectedCoreThread?.let { open -> if (open.id in byId && !open.isSenderGroup) open.copy(meronFolderId = targetFolderId) else open }
    // A cached Meron folder list no longer matches; it re-reads when opened.
    mailboxCache = mailboxCache.filterKeys { it.accountId != MERON_FOLDERS_ACCOUNT_ID }
}

/** Move whole conversations into [folder], with Undo. */
internal fun MeronMobileState.moveToMeronFolder(
    threads: List<ThreadSummary>,
    folder: MeronFolderSummary,
) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val allowed = threads.filter(::canUseMeronFolders)
    // Already there: nothing to move, and Undo would take them out.
    val moving = allowed.filterNot { it.meronFolderId == folder.id }
    if (moving.isEmpty()) {
        if (allowed.isEmpty() && threads.isNotEmpty()) status = trs("meronFolders.notAllowed")
        return
    }
    val threadsSnapshot = coreThreads
    val kanbanSnapshot = kanbanColumns
    val mailboxKey = visibleMailboxKey
    val openSnapshot = selectedCoreThread
    val removals = moving.filter { visibleListDropsMoved(it, movingOut = false) }.map { suppressRemovedThread(it.id) }
    applyMeronMoveLocally(moving, folder.id)
    val generation = nextMeronFoldersGeneration()
    val committed = CompletableDeferred<List<jp.nonbili.meron.shared.MeronFolderMoveChange>?>()
    // A sender row is however many conversations the core finds for it, so
    // its toast waits for the core's count; plain rows count themselves.
    val countFromCore = moving.any { it.isSenderGroup }

    fun offerUndo(count: Int) =
        scope.launch {
            val result =
                snackbarHost.showSnackbar(
                    message = trs("meronFolders.moved", mapOf("count" to count, "name" to folder.name)),
                    actionLabel = trs("buttons.undo"),
                    duration = SnackbarDuration.Long,
                )
            if (result != SnackbarResult.ActionPerformed) return@launch
            snackbarHost.currentSnackbarData?.dismiss()
            // Rows back at once, like Archive's Undo; the store follows.
            showRestoredThread(moving.map { it.id }.toSet(), mailboxKey, threadsSnapshot, kanbanSnapshot)
            selectedCoreThread = selectedCoreThread?.let { open -> openSnapshot?.takeIf { it.id == open.id } ?: open }
            val changes = committed.await() ?: return@launch
            // Every change the core reported (a sender row's come back keyed by
            // its messages), and the rows hidden on screen.
            undoMeronMove(changes, changes.map { it.threadId }.toSet() + moving.map { it.id })
        }
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                requireCoreOk(MobileMailCommandClient(core).moveToMeronFolder(folder.id, moving.map { it.id }))
            }
        }.onSuccess { response ->
            removals.forEach { it.complete() }
            val result = parseMeronFolderMoveResponse(response)
            applyMeronFolderListFrom(generation, response)
            applyMeronFolderCounts(response)
            // Re-read: other subject-branch rows of a moved conversation went
            // with it, and refused rows (gone, not allowed) show where they are.
            syncCoreThreads(syncFirst = false)
            if (result.skipped.isNotEmpty() && result.moved == 0) status = trs("meronFolders.notAllowed")
            committed.complete(result.changes)
            if (countFromCore && result.moved > 0) offerUndo(result.moved)
        }.onFailure {
            removals.forEach { removal -> removal.rollback() }
            Log.w("MeronFolders", "move in failed", it)
            coreThreads = threadsSnapshot
            kanbanColumns = kanbanSnapshot
            selectedCoreThread = openSnapshot
            // Only plain rows put a toast up before the answer.
            if (!countFromCore) snackbarHost.currentSnackbarData?.dismiss()
            status = meronErrorText(it)
            committed.complete(null)
        }
    }
    if (!countFromCore) offerUndo(moving.size)
}

/** Put each conversation back where it was: its earlier Meron folder, or out. */
private fun MeronMobileState.undoMeronMove(
    changes: List<jp.nonbili.meron.shared.MeronFolderMoveChange>,
    threadIds: Set<String>,
) {
    val generation = nextMeronFoldersGeneration()
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                // One call per earlier folder; each answers for the mailboxes it touched.
                changes
                    .filter { it.threadId in threadIds }
                    .groupBy { it.previousFolderId }
                    .map { (previous, group) ->
                        val ids = group.map { it.threadId }
                        requireCoreOk(
                            if (previous == null) client.moveOutOfMeronFolder(ids) else client.moveToMeronFolder(previous, ids),
                        )
                    }
            }
        }.onSuccess { responses ->
            threadIds.forEach(threadRemovalGuard::release)
            // The last answer holds the folder list after every call.
            responses.lastOrNull()?.let { applyMeronFolderListFrom(generation, it) }
            responses.forEach(::applyMeronFolderCounts)
            mailboxCache = mailboxCache.filterKeys { it.accountId != MERON_FOLDERS_ACCOUNT_ID }
            // Re-read so rows and chips match the store again.
            syncCoreThreads(syncFirst = false)
        }.onFailure {
            Log.w("MeronFolders", "undo failed", it)
            status = trsError("mobile.status.undoFailed", it)
            syncCoreThreads(syncFirst = false)
        }
    }
}

/** Take whole conversations out of their Meron folders, with Undo. */
internal fun MeronMobileState.moveOutOfMeronFolder(threads: List<ThreadSummary>) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val moving = threads.filter { it.meronFolderId != null || inMeronFolderView }
    if (moving.isEmpty()) return
    val previous = moving.associate { it.id to (it.meronFolderId ?: selectedCoreFolder.takeIf { inMeronFolderView }) }
    // Out of the built-in Archive is back to the Inbox (un-archived on the server).
    val allArchive = moving.all { previous[it.id] == MERON_ARCHIVE_FOLDER_ID }
    val threadsSnapshot = coreThreads
    val kanbanSnapshot = kanbanColumns
    val mailboxKey = visibleMailboxKey
    val openSnapshot = selectedCoreThread
    val removals = moving.filter { visibleListDropsMoved(it, movingOut = true) }.map { suppressRemovedThread(it.id) }
    applyMeronMoveLocally(moving, null)
    val generation = nextMeronFoldersGeneration()
    // The answer's Inbox ids of conversations taken out of Archive (null: failed).
    val committed = CompletableDeferred<List<String>?>()
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                requireCoreOk(MobileMailCommandClient(core).moveOutOfMeronFolder(moving.map { it.id }))
            }
        }.onSuccess { response ->
            removals.forEach { it.complete() }
            val unarchived = parseMeronUnarchived(response)
            applyMeronFolderListFrom(generation, response)
            applyMeronFolderCounts(response)
            // Other subject-branch rows of the conversation left with it.
            if (inMeronFolderView) syncCoreThreads(syncFirst = false)
            // The conversations show in their board columns again.
            if (kanbanColumns.isNotEmpty()) loadKanbanBoard(refresh = false)
            // Un-archived mail is back in the Inbox: lists showing it re-read.
            if (unarchived.isNotEmpty() && !inMeronFolderView) syncCoreThreads(syncFirst = false)
            committed.complete(unarchived)
        }.onFailure {
            removals.forEach { removal -> removal.rollback() }
            Log.w("MeronFolders", "move out failed", it)
            coreThreads = threadsSnapshot
            kanbanColumns = kanbanSnapshot
            selectedCoreThread = openSnapshot
            snackbarHost.currentSnackbarData?.dismiss()
            status = meronErrorText(it)
            committed.complete(null)
        }
    }
    scope.launch {
        val result =
            snackbarHost.showSnackbar(
                message =
                    trs(
                        if (allArchive) "meronFolders.movedToInbox" else "meronFolders.movedOut",
                        mapOf("count" to moving.size),
                    ),
                actionLabel = trs("buttons.undo"),
                duration = SnackbarDuration.Long,
            )
        if (result != SnackbarResult.ActionPerformed) return@launch
        snackbarHost.currentSnackbarData?.dismiss()
        showRestoredThread(moving.map { it.id }.toSet(), mailboxKey, threadsSnapshot, kanbanSnapshot)
        selectedCoreThread = selectedCoreThread?.let { open -> openSnapshot?.takeIf { it.id == open.id } ?: open }
        val unarchived = committed.await() ?: return@launch
        // Conversations taken out of Archive went back to the Inbox under new
        // ids: the Undo archives those again.
        val changes =
            moving.mapNotNull { thread ->
                previous[thread.id]?.takeIf { it != MERON_ARCHIVE_FOLDER_ID }?.let { folderId ->
                    jp.nonbili.meron.shared.MeronFolderMoveChange(thread.id, thread.accountId, thread.folder, folderId)
                }
            } +
                unarchived.map { threadId ->
                    jp.nonbili.meron.shared.MeronFolderMoveChange(threadId, threadId.substringBefore('#'), INBOX_FOLDER, MERON_ARCHIVE_FOLDER_ID)
                }
        undoMeronMove(changes, moving.map { it.id }.toSet() + unarchived)
    }
}

/** "Mark all read" inside a Meron folder: sets read on the servers. */
internal fun MeronMobileState.markMeronFolderAllRead(folderId: String) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val threadsSnapshot = coreThreads
    val foldersSnapshot = meronFolders
    coreThreads = coreThreads.map { it.copy(unread = false, unreadCount = 0) }
    meronFolders = meronFolders.map { if (it.id == folderId) it.copy(unread = 0) else it }
    val generation = nextMeronFoldersGeneration()
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                requireCoreOk(MobileMailCommandClient(core).markMeronFolderRead(folderId))
            }
        }.onSuccess { response ->
            // Per mailbox: some accounts may have failed while the rest were marked.
            applyMeronFolderListFrom(generation, response)
            applyMeronFolderCounts(response)
            mailboxCache = mailboxCache.filterKeys { it.accountId != MERON_FOLDERS_ACCOUNT_ID }
            val partialFailure = runCatching { requireCoreAllOk(response) }.exceptionOrNull()
            if (partialFailure != null) {
                Log.w("MeronFolders", "mark all read partly failed", partialFailure)
                status = trsError("mobile.status.markAllReadFailed", partialFailure)
                // Show which rows are still unread.
                if (inMeronFolderView && selectedCoreFolder == folderId) syncCoreThreads(syncFirst = false)
            }
        }.onFailure {
            Log.w("MeronFolders", "mark all read failed", it)
            if (inMeronFolderView && selectedCoreFolder == folderId) coreThreads = threadsSnapshot
            if (generation == meronFoldersGeneration) meronFolders = foldersSnapshot
            status = trsError("mobile.status.markAllReadFailed", it)
            refreshMeronFolders()
        }
    }
}

internal fun MeronMobileState.promptNewMeronFolder(moveThreads: List<ThreadSummary> = emptyList()) {
    meronFolderNameInput = ""
    meronFolderNameError = ""
    meronFolderDialog = MeronFolderDialog.New(moveThreads)
}

internal fun MeronMobileState.promptRenameMeronFolder(folder: MeronFolderSummary) {
    meronFolderNameInput = folder.name
    meronFolderNameError = ""
    meronFolderDialog = MeronFolderDialog.Rename(folder)
}

internal fun MeronMobileState.promptDeleteMeronFolder(folder: MeronFolderSummary) {
    meronFolderDialog = MeronFolderDialog.Delete(folder)
}

/** The core's English refusals, in the reader's language where we know them. */
private fun MeronMobileState.meronErrorText(error: Throwable): String {
    val message = error.message.orEmpty()
    return when {
        message.contains("already exists") -> trs("meronFolders.nameTaken", mapOf("name" to meronFolderNameInput.trim()))
        message.contains("Enter a name") -> trs("meronFolders.nameRequired")
        message.contains("can't go in a Meron folder") -> trs("meronFolders.notAllowed")
        else -> message.ifBlank { trs("mail.toast.moveFailed") }
    }
}

/** Create or rename from the name dialog; the dialog stays open on a refusal. */
internal fun MeronMobileState.submitMeronFolderName() {
    val dialog = meronFolderDialog ?: return
    val name = meronFolderNameInput.trim()
    if (name.isEmpty()) {
        meronFolderNameError = trs("meronFolders.nameRequired")
        return
    }
    val taken =
        meronFolders.any { folder ->
            folder.name.equals(name, ignoreCase = true) && (dialog !is MeronFolderDialog.Rename || folder.id != dialog.folder.id)
        }
    if (taken) {
        meronFolderNameError = trs("meronFolders.nameTaken", mapOf("name" to name))
        return
    }
    if (!coreLoaded || meronFolderBusy) return
    meronFolderBusy = true
    val generation = nextMeronFoldersGeneration()
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                val client = MobileMailCommandClient(core)
                requireCoreOk(
                    when (dialog) {
                        is MeronFolderDialog.Rename -> client.renameMeronFolder(dialog.folder.id, name)
                        else -> client.createMeronFolder(name)
                    },
                )
            }
        }.onSuccess { response ->
            meronFolderBusy = false
            meronFolderDialog = null
            applyMeronFolderListFrom(generation, response)
            val folder = parseMeronFolderResponse(response)
            if (dialog is MeronFolderDialog.New && folder != null && dialog.moveThreads.isNotEmpty()) {
                moveToMeronFolder(dialog.moveThreads, folder)
                selectedMailThreadIds = emptySet()
            }
        }.onFailure {
            meronFolderBusy = false
            Log.w("MeronFolders", "name change failed", it)
            meronFolderNameError = meronErrorText(it)
        }
    }
}

/** Turn new-mail notifications for a Meron folder on or off (this device
 *  only): new mail AI sorting files there notifies, as an opted-in folder's
 *  does. Same toasts as a server folder's. */
internal fun MeronMobileState.setMeronFolderNotify(
    folder: MeronFolderSummary,
    enabled: Boolean,
) {
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val generation = nextMeronFoldersGeneration()
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                requireCoreOk(MobileMailCommandClient(core).setMeronFolderNotify(folder.id, enabled))
            }
        }.onSuccess { response ->
            applyMeronFolderListFrom(generation, response)
            status =
                trs(
                    if (enabled) "folders.notify.enabled" else "folders.notify.disabled",
                    mapOf("folder" to folder.name),
                )
        }.onFailure {
            Log.w("MeronFolders", "set notify failed", it)
            status = it.message ?: trs("folders.notify.failed", mapOf("folder" to folder.name))
        }
    }
}

/** Delete a Meron folder; its conversations go back where their mail is. */
internal fun MeronMobileState.deleteMeronFolder(folder: MeronFolderSummary) {
    meronFolderDialog = null
    if (!coreLoaded) {
        status = coreUnavailableMessage
        return
    }
    val generation = nextMeronFoldersGeneration()
    scope.launch {
        runCatching {
            withContext(ioDispatcher) {
                requireCoreOk(MobileMailCommandClient(core).deleteMeronFolder(folder.id))
            }
        }.onSuccess { response ->
            mailboxCache = mailboxCache.filterKeys { it.accountId != MERON_FOLDERS_ACCOUNT_ID }
            applyMeronFolderCounts(response)
            val wasOpen = inMeronFolderView && selectedCoreFolder == folder.id
            applyMeronFolderListFrom(generation, response)
            // The conversations show in their mailboxes again.
            if (!wasOpen) syncCoreThreads(syncFirst = false)
            if (kanbanColumns.isNotEmpty()) loadKanbanBoard(refresh = false)
        }.onFailure {
            Log.w("MeronFolders", "delete failed", it)
            status = meronErrorText(it)
        }
    }
}
