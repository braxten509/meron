// Mobile mailbox state changes for unread counts.
// Called by the Compose routes and their thread actions.
// Server responses are reconciled with the local mailbox state here.
package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.FolderSummary
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.parseFolderUnreadChanges

internal fun MeronMobileState.applyCoreFolderUnreadChanges(response: String) {
    applyCoreFolderUnreadChangesAt(response, null)
}

internal fun MeronMobileState.applyCoreFolderUnreadChangesAt(
    response: String,
    started: Long?,
) {
    val changes =
        parseFolderUnreadChanges(response).map { change ->
            change.copy(unread = folderReadGuard.recordMutation(change.accountId, change.folderId, change.unread, started))
        }
    if (changes.isEmpty()) return
    val nextByAccount = foldersByAccount.toMutableMap()
    changes.groupBy { it.accountId }.forEach { (accountId, accountChanges) ->
        val counts = accountChanges.associateBy { it.folderId }
        nextByAccount[accountId] =
            nextByAccount[accountId].orEmpty().map { folder ->
                counts.entries
                    .firstOrNull { (folderId, _) -> folder.name.equals(folderId, ignoreCase = folder.role == "inbox") }
                    ?.value
                    ?.let { folder.copy(unread = it.unread) } ?: folder
            }
    }
    val nextCoreFolders =
        coreFolders.map { folder ->
            changes
                .firstOrNull {
                    it.accountId == folder.accountId && folder.name.equals(it.folderId, ignoreCase = folder.role == "inbox")
                }?.let { folder.copy(unread = it.unread) } ?: folder
        }
    coreFolders = nextCoreFolders
    foldersByAccount = nextByAccount
}

// The drawer's unread badges read the folder caches, not the list rows, so
// clearing rows optimistically on its own left them showing the pre-mark totals
// until the write answered. These apply the same change to the caches up front;
// the response — and the sync that follows — still replace them with the
// server's own numbers.
private fun MeronMobileState.cachedFolderUnread(
    accountId: String,
    folderId: String,
): Int = folderUnread(foldersByAccount[accountId] ?: coreFolders.filter { it.accountId == accountId }, folderId)

internal fun MeronMobileState.applyLocalFolderUnread(counts: Map<Pair<String, String>, Int>) {
    if (counts.isEmpty()) return
    val patch = { folder: FolderSummary ->
        counts.entries
            .firstOrNull { (target, _) ->
                target.first == folder.accountId && folder.name.equals(target.second, ignoreCase = folder.role == INBOX_FOLDER)
            }?.let { folder.copy(unread = it.value) } ?: folder
    }
    foldersByAccount = foldersByAccount.mapValues { (_, folders) -> folders.map(patch) }
    coreFolders = coreFolders.map(patch)
}

// Folder totals after marking [mailFolders] read folder-wide and taking the
// [rssRows] marked here off their own folders, paired with the totals they had.
internal fun MeronMobileState.plannedFolderUnread(
    mailFolders: List<Pair<String, String>>,
    rssRows: List<ThreadSummary>,
): Pair<Map<Pair<String, String>, Int>, Map<Pair<String, String>, Int>> {
    val next = mutableMapOf<Pair<String, String>, Int>()
    val before = mutableMapOf<Pair<String, String>, Int>()
    mailFolders.forEach { target ->
        before[target] = cachedFolderUnread(target.first, target.second)
        next[target] = 0
    }
    rssRows.forEach { row ->
        val target = row.accountId to row.folder
        if (target.first.isBlank() || target.second.isBlank()) return@forEach
        before.getOrPut(target) { cachedFolderUnread(target.first, target.second) }
        next[target] = (next[target] ?: before.getValue(target)) - 1
    }
    return next.mapValues { (_, value) -> value.coerceAtLeast(0) } to before
}
