// Coordinates optimistic removals with older mobile
// mailbox reads. Completion keeps stale rows out
// while rollback releases only the affected
// operation.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.parseThreadActionLocationResponse
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

internal class ThreadRemoval(
    private val onComplete: () -> Unit,
    private val onRollback: () -> Unit,
    val filter: (List<ThreadSummary>) -> List<ThreadSummary>,
) {
    private var finished = false

    fun complete() {
        if (finished) return
        finished = true
        onComplete()
    }

    fun rollback() {
        if (finished) return
        finished = true
        onRollback()
    }
}

// Accessed on the UI dispatcher. Each mutation owns its marker, including
// overlapping mutations of the same thread. Reads keep completed markers until
// they finish, while failed mutations remove only their own marker.
internal class ThreadRemovalGuard {
    internal class Marker(
        val matches: (ThreadSummary) -> Boolean,
    )

    private val pending = mutableSetOf<Marker>()
    private val reads = mutableSetOf<Read>()

    inner class Read internal constructor() {
        private val removed = pending.associateWith { mutableSetOf<String>() }.toMutableMap()

        fun filter(threads: List<ThreadSummary>): List<ThreadSummary> =
            threads.filterNot { thread ->
                pending.any { it.matches(thread) } || removed.any { (marker, restored) -> marker.matches(thread) && thread.id !in restored }
            }

        internal fun add(marker: Marker) {
            removed[marker] = mutableSetOf()
        }

        internal fun remove(marker: Marker) {
            removed.remove(marker)
        }

        internal fun release(threadId: String) {
            removed.forEach { (marker, restored) -> if (marker !in pending) restored.add(threadId) }
        }

        fun dispose() {
            reads.remove(this)
        }
    }

    fun beginRead(): Read = Read().also { reads.add(it) }

    fun filter(threads: List<ThreadSummary>): List<ThreadSummary> = threads.filterNot { thread -> pending.any { it.matches(thread) } }

    fun suppress(threadId: String): ThreadRemoval = suppressMatching { it.id == threadId }

    fun suppressFolder(
        accountId: String,
        folderId: String,
    ): ThreadRemoval = suppressMatching { it.accountId == accountId && it.folder == folderId }

    private fun suppressMatching(matches: (ThreadSummary) -> Boolean): ThreadRemoval {
        val marker = Marker(matches)
        val read = beginRead()
        pending.add(marker)
        reads.forEach { it.add(marker) }
        return ThreadRemoval(
            onComplete = {
                pending.remove(marker)
                read.dispose()
            },
            onRollback = {
                pending.remove(marker)
                reads.forEach { it.remove(marker) }
                read.dispose()
            },
            filter = read::filter,
        )
    }

    // A successful move reports the destination ID. Allow that ID in older
    // snapshots again; other pending mutations still hide it independently.
    fun release(threadId: String) {
        reads.forEach { it.release(threadId) }
    }
}

internal fun CoroutineScope.launchThreadListRead(
    guard: ThreadRemovalGuard,
    block: suspend CoroutineScope.(ThreadRemovalGuard.Read) -> Unit,
) = launch {
    val read = guard.beginRead()
    try {
        block(read)
    } finally {
        read.dispose()
    }
}

private fun MeronMobileState.suppressCachedThreads(
    removal: ThreadRemoval,
    matches: (ThreadSummary) -> Boolean,
): ThreadRemoval {
    val removed = mailboxCache.mapValues { (_, cached) -> cached.threads.filter(matches) }.filterValues { it.isNotEmpty() }
    return ThreadRemoval(
        onComplete = {
            removal.complete()
            // Pending rows are hidden when caches are restored. Prune them only
            // after success so overlapping failures can recover their snapshots.
            mailboxCache = mailboxCache.mapValues { (_, cached) -> cached.copy(threads = cached.threads.filterNot(matches)) }
        },
        onRollback = {
            removal.rollback()
            mailboxCache =
                mailboxCache.mapValues { (key, cached) ->
                    val existing = cached.threads.map { it.id }.toSet()
                    val restored = removal.filter(removed[key].orEmpty()).filterNot { it.id in existing }
                    if (restored.isEmpty()) cached else cached.copy(threads = (cached.threads + restored).sortedByDescending { it.dateEpochSeconds })
                }
        },
        filter = removal.filter,
    )
}

internal fun MeronMobileState.suppressRemovedThread(threadId: String): ThreadRemoval = suppressCachedThreads(threadRemovalGuard.suppress(threadId)) { it.id == threadId }

internal fun MeronMobileState.suppressEmptiedFolder(
    accountId: String,
    folderId: String,
): ThreadRemoval = suppressCachedThreads(threadRemovalGuard.suppressFolder(accountId, folderId)) { it.accountId == accountId && it.folder == folderId }

internal fun MeronMobileState.releaseMovedThread(response: String) {
    val threadId = parseThreadActionLocationResponse(response).threadId
    if (threadId.isNotBlank()) threadRemovalGuard.release(threadId)
}
