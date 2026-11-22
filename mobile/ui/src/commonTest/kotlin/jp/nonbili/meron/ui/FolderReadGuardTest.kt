// Exercises folder counts when a read-state write overlaps
// a refresh. Versioned responses and rollback snapshots
// protect a later local change from an older server result.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.FolderSummary
import kotlin.test.Test
import kotlin.test.assertEquals

class FolderReadGuardTest {
    @Test
    fun refreshStartedBeforeWriteCannotRestoreCountsAfterCompletion() {
        val guard = FolderReadGuard()
        val targets = setOf("a" to "INBOX")
        val started = guard.version
        guard.begin(targets)
        guard.end(targets)
        val current = mapOf("a" to listOf(folder("INBOX", 0), folder("Archive", 2)))
        val refreshed = guard.reconcile(listOf(folder("INBOX", 6), folder("Archive", 3)), current, started)
        assertEquals(listOf(0, 3), refreshed.map { it.unread })
        assertEquals(6, guard.reconcile(listOf(folder("INBOX", 6)), current, guard.version).single().unread)
    }

    @Test
    fun delayedRefreshCannotUndoFailureRollback() {
        val guard = FolderReadGuard()
        val targets = setOf("a" to "inbox")
        guard.begin(targets)
        val started = guard.version
        guard.end(targets)
        val current = mapOf("a" to listOf(folder("INBOX", 6)))
        assertEquals(6, guard.reconcile(listOf(folder("INBOX", 0)), current, started).single().unread)
    }

    @Test
    fun currentVersionSnapshotRestoreDoesNotDependOnPreviouslyTouchedFolders() {
        val guard = FolderReadGuard()
        val target = setOf("a" to "INBOX")
        guard.begin(target)
        guard.end(target)
        val cached = listOf(folder("INBOX", 6), folder("Archive", 8))
        val current = mapOf("a" to listOf(folder("INBOX", 0), folder("Archive", 0)))
        assertEquals(listOf(6, 8), guard.reconcile(cached, current, guard.version).map { it.unread })
    }

    @Test
    fun evictedHistoryStillRejectsOldResponsesButAcceptsFreshOnes() {
        val guard = FolderReadGuard()
        val started = guard.version
        repeat(600) { index ->
            val targets = setOf("a" to "folder-$index")
            guard.begin(targets)
            guard.end(targets)
        }
        val current = mapOf("a" to listOf(folder("folder-0", 0)))
        val response = listOf(folder("folder-0", 6))
        assertEquals(0, guard.reconcile(response, current, started).single().unread)
        assertEquals(6, guard.reconcile(response, current, guard.version).single().unread)
    }

    @Test
    fun mutationCountsAreOrderedByRequestVersion() {
        val guard = FolderReadGuard()
        guard.begin(setOf("a" to "INBOX"))
        val oldRequest = guard.version
        assertEquals(1, guard.recordMutation("a", "INBOX", 1))
        assertEquals(1, guard.recordMutation("a", "inbox", 0, oldRequest))
        assertEquals(1, guard.resolveMutation("a", "INBOX", 0, oldRequest))
        val newRequest = guard.version
        assertEquals(0, guard.recordMutation("a", "INBOX", 0, newRequest))
        assertEquals(0, guard.resolveMutation("a", "INBOX", 6, oldRequest))
        guard.end(setOf("a" to "INBOX"))
    }

    private fun folder(
        name: String,
        unread: Int,
    ) = FolderSummary(accountId = "a", name = name, unread = unread)
}
