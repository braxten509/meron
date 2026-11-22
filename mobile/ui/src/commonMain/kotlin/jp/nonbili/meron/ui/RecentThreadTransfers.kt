// Persists recently used mobile move and copy
// destinations. Account identity and transfer
// type distinguish otherwise similar folder
// selections.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.FolderSummary
import jp.nonbili.meron.shared.MobileMailCommandClient
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.accountSummaryIsRss
import jp.nonbili.meron.shared.threadIdIsRss
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

private const val RECENT_TRANSFERS_PREF = "recent_thread_transfers"
private const val RECENT_TRANSFERS_LIMIT = 100
private const val RECENT_DESTINATIONS_LIMIT = 3
private val transferJson = Json { ignoreUnknownKeys = true }

@Serializable
internal data class RecentThreadTransfer(
    val action: String,
    val sourceAccountId: String,
    val targetAccountId: String,
    val targetFolderId: String,
)

internal fun parseRecentThreadTransfers(raw: String): List<RecentThreadTransfer> =
    runCatching {
        transferJson
            .decodeFromString<List<RecentThreadTransfer>>(raw)
            .filter {
                it.action in listOf("move", "copy") && it.sourceAccountId.isNotBlank() &&
                    it.targetAccountId.isNotBlank() && it.targetFolderId.isNotBlank()
            }.distinct()
            .take(RECENT_TRANSFERS_LIMIT)
    }.getOrDefault(emptyList())

internal fun loadRecentThreadTransfers(prefs: AppPreferences): List<RecentThreadTransfer> = parseRecentThreadTransfers(prefs.cacheStore().getString(RECENT_TRANSFERS_PREF, ""))

internal fun MeronMobileState.rememberThreadTransfer(entry: RecentThreadTransfer) {
    recentThreadTransfers = (listOf(entry) + recentThreadTransfers.filterNot { it == entry }).take(RECENT_TRANSFERS_LIMIT)
    prefs.cacheStore().putString(RECENT_TRANSFERS_PREF, transferJson.encodeToString(recentThreadTransfers))
}

/** Follow a folder rename, so recent targets in it are still offered. */
internal fun MeronMobileState.renameRecentTransferTargets(
    accountId: String,
    renames: Map<String, String>,
) {
    if (recentThreadTransfers.none { it.targetAccountId == accountId && it.targetFolderId in renames }) return
    recentThreadTransfers =
        recentThreadTransfers
            .map { entry ->
                val folderId = if (entry.targetAccountId == accountId) renames[entry.targetFolderId] else null
                if (folderId == null) entry else entry.copy(targetFolderId = folderId)
            }.distinct()
    prefs.cacheStore().putString(RECENT_TRANSFERS_PREF, transferJson.encodeToString(recentThreadTransfers))
}

/** A successful RPC with zero affected messages is not a successful transfer. */
internal fun requireTransferAffected(
    response: String,
    action: String,
): String {
    val count =
        runCatching {
            transferJson
                .parseToJsonElement(response)
                .jsonObject[action]
                ?.jsonPrimitive
                ?.intOrNull
        }.getOrNull()
    check(count == null || count > 0) { "No matching messages found" }
    return response
}

internal data class RecentThreadTransferItem(
    val transfer: RecentThreadTransfer,
    val folder: FolderSummary,
    val destination: String,
)

internal fun MeronMobileState.recentTransferItems(thread: ThreadSummary?): List<RecentThreadTransferItem> {
    if (thread == null || threadIdIsRss(thread.id)) return emptyList()
    return recentThreadTransfers
        .mapNotNull { entry ->
            if (entry.sourceAccountId != thread.accountId ||
                (entry.targetAccountId == thread.accountId && kanbanFolderIdsEqual(entry.targetFolderId, thread.folder))
            ) {
                return@mapNotNull null
            }
            val account = coreAccounts.firstOrNull { it.id == entry.targetAccountId && !accountSummaryIsRss(it) } ?: return@mapNotNull null
            val folder = foldersByAccount[entry.targetAccountId]?.firstOrNull { kanbanFolderIdsEqual(it.name, entry.targetFolderId) } ?: return@mapNotNull null
            val destination = if (entry.targetAccountId == thread.accountId) folder.displayName else "${account.displayName.ifBlank { account.email }} / ${folder.displayName}"
            RecentThreadTransferItem(entry, folder, destination)
        }
}

/** The few recent targets a move or copy destination picker offers first. */
internal fun List<RecentThreadTransferItem>.forAction(action: String): List<RecentThreadTransferItem> = filter { it.transfer.action == action }.take(RECENT_DESTINATIONS_LIMIT)

/** Load only accounts referenced by this source account's history, without a status toast. */
internal suspend fun MeronMobileState.loadRecentTransferFolders(thread: ThreadSummary?) {
    if (!coreLoaded || thread == null || threadIdIsRss(thread.id)) return
    val targetIds = recentThreadTransfers.filter { it.sourceAccountId == thread.accountId }.map { it.targetAccountId }.toSet()
    for (account in coreAccounts.filter { it.id in targetIds && !accountSummaryIsRss(it) }) {
        if (foldersByAccount[account.id] != null) continue
        val version = folderReadGuard.version
        runCatching {
            withContext(ioDispatcher) { loadAccountFolders(MobileMailCommandClient(core), account) }
        }.onSuccess { folders ->
            if (coreAccounts.any { it.id == account.id }) {
                foldersByAccount = foldersByAccount + (account.id to reconcileFolderUnread(folders, version))
            }
        }.onFailure { Log.w("Mail", "load recent transfer folders failed", it) }
    }
}
