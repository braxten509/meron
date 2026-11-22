// Tracks local mark-read operations while folder
// counts are refreshing. A late read must respect the
// pending mutation associated with its account and
// mailbox.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.FolderSummary

internal fun folderReadTarget(
    account: String,
    folder: String,
) = account to if (folder.equals("inbox", true)) "inbox" else folder

// Accessed on the UI dispatcher. A refresh captures version before doing IO;
// counts from before or during a write must not undo its optimistic clear.
internal class FolderReadGuard {
    var version = 0L
        private set
    private val changed = mutableMapOf<Pair<String, String>, Long>()
    private val pending = mutableSetOf<Pair<String, String>>()

    private data class MutationCount(
        val unread: Int,
        val version: Long,
    )

    private val mutations = mutableMapOf<Pair<String, String>, MutationCount>()
    private var oldestVersion = 0L

    fun resolveMutation(
        account: String,
        folder: String,
        unread: Int,
        started: Long,
    ): Int {
        val latest = mutations[folderReadTarget(account, folder)]
        return if (latest != null && latest.version > started) latest.unread else unread
    }

    // Mutation responses bypass the refresh hold. A board response retains the
    // version of its request, including when a sibling reuses that response.
    fun recordMutation(
        account: String,
        folder: String,
        unread: Int,
        started: Long? = null,
    ): Int {
        val key = folderReadTarget(account, folder)
        val latest = mutations[key]
        if (started != null && latest != null && latest.version > started) return latest.unread
        touch(key)
        if (key in pending) mutations[key] = MutationCount(unread, started ?: version)
        return unread
    }

    private fun touch(key: Pair<String, String>) {
        changed.remove(key)
        changed[key] = ++version
        if (changed.size > 512) {
            val oldest = changed.keys.first()
            oldestVersion = changed.remove(oldest)!!
        }
    }

    fun begin(targets: Set<Pair<String, String>>) {
        targets.forEach { (account, folder) ->
            val key = folderReadTarget(account, folder)
            pending += key
            touch(key)
        }
    }

    fun end(targets: Set<Pair<String, String>>) {
        targets.forEach { (account, folder) ->
            val key = folderReadTarget(account, folder)
            pending -= key
            mutations.remove(key)
            touch(key)
        }
    }

    fun reconcile(
        folders: List<FolderSummary>,
        current: Map<String, List<FolderSummary>>,
        started: Long,
        fallback: List<FolderSummary> = emptyList(),
    ): List<FolderSummary> =
        folders.map { folder ->
            val key = folderReadTarget(folder.accountId, folder.name)
            if (started < oldestVersion || key in pending || (changed[key] ?: 0L) > started) {
                val cached =
                    current[folder.accountId]?.firstOrNull { folderReadTarget(it.accountId, it.name) == key }
                        ?: fallback.firstOrNull { folderReadTarget(it.accountId, it.name) == key }
                cached?.let { folder.copy(unread = it.unread) } ?: folder
            } else {
                folder
            }
        }
}

internal fun MeronMobileState.reconcileFolderUnread(
    folders: List<FolderSummary>,
    started: Long,
): List<FolderSummary> = folderReadGuard.reconcile(folders, foldersByAccount, started, coreFolders)
