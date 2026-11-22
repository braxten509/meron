// A removal can finish before an older read returns.
// The cases cover suppressing stale rows, accepting new
// replies and releasing only the failed removal's
// guard.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.ThreadSummary
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals

class ThreadRemovalGuardTest {
    private val thread = ThreadSummary(id = "a#INBOX#t1", accountId = "a", folder = "INBOX", subject = "Hello", sender = "Ada")
    private val rows get() = listOf(thread)

    @Test
    fun completedRemovalStillFiltersOlderReadsButAllowsNewReplies() {
        val guard = ThreadRemovalGuard()
        val before = guard.beginRead()
        val removal = guard.suppress(thread.id)
        val during = guard.beginRead()
        assertEquals(emptyList(), guard.filter(rows))
        removal.complete()
        assertEquals(emptyList(), before.filter(rows))
        assertEquals(emptyList(), during.filter(rows))
        val after = guard.beginRead()
        assertEquals(rows, after.filter(rows))
        before.dispose()
        during.dispose()
        after.dispose()
    }

    @Test
    fun failedRemovalReleasesReadsAndOnlyItsOwnToken() {
        val guard = ThreadRemovalGuard()
        val read = guard.beginRead()
        val firstRollback = guard.suppress(thread.id)
        val secondRollback = guard.suppress(thread.id)
        firstRollback.rollback()
        assertEquals(emptyList(), read.filter(rows))
        secondRollback.rollback()
        assertEquals(rows, read.filter(rows))
        assertEquals(rows, guard.filter(rows))
        read.dispose()
    }

    @Test
    fun undoReleasesOlderReads() {
        val guard = ThreadRemovalGuard()
        val removal = guard.suppress(thread.id)
        val read = guard.beginRead()
        removal.complete()
        guard.release(thread.id)
        assertEquals(rows, read.filter(rows))
        read.dispose()
    }

    @Test
    fun overlappingCompletionsKeepTheOtherMutationPendingInEitherOrder() {
        for (firstIndex in 0..1) {
            val guard = ThreadRemovalGuard()
            val old = guard.beginRead()
            val removals = listOf(guard.suppress(thread.id), guard.suppress(thread.id))
            removals[firstIndex].complete()
            val between = guard.beginRead()
            assertEquals(emptyList(), between.filter(rows))
            removals[1 - firstIndex].complete()
            assertEquals(emptyList(), old.filter(rows))
            assertEquals(emptyList(), between.filter(rows))
            val fresh = guard.beginRead()
            assertEquals(rows, fresh.filter(rows))
            old.dispose()
            between.dispose()
            fresh.dispose()
        }
    }

    @Test
    fun aFailedSiblingDoesNotEraseASuccessfulRemovalFromOldReads() {
        val guard = ThreadRemovalGuard()
        val read = guard.beginRead()
        val success = guard.suppress(thread.id)
        val failure = guard.suppress(thread.id)
        success.complete()
        failure.rollback()
        assertEquals(emptyList(), read.filter(rows))
        assertEquals(emptyList(), failure.filter(rows))
        assertEquals(rows, guard.filter(rows))
        read.dispose()
    }

    @Test
    fun movedBackThreadIsVisibleWithoutReleasingAnotherPendingMutation() {
        val guard = ThreadRemovalGuard()
        val read = guard.beginRead()
        guard.suppress(thread.id).complete()
        guard.release(thread.id)
        assertEquals(rows, read.filter(rows))
        val laterRemoval = guard.suppress(thread.id)
        guard.release(thread.id)
        assertEquals(emptyList(), read.filter(rows))
        laterRemoval.complete()
        assertEquals(emptyList(), read.filter(rows))
        guard.release(thread.id)
        assertEquals(rows, read.filter(rows))
        // A later removal must create a new marker without the old exception.
        guard.suppress(thread.id).complete()
        assertEquals(emptyList(), read.filter(rows))
        read.dispose()
    }

    @Test
    fun emptyFolderFiltersUnloadedRowsAndAllowsAMovedBackThread() {
        val guard = ThreadRemovalGuard()
        val read = guard.beginRead()
        val offscreen = thread.copy(id = "a#INBOX#unloaded")
        val elsewhere = thread.copy(id = "a#Archive#t1", folder = "Archive")
        val anotherAccount = thread.copy(id = "b#INBOX#t1", accountId = "b")
        val all = rows + listOf(offscreen, elsewhere, anotherAccount)
        val removal = guard.suppressFolder("a", "INBOX")
        removal.complete()
        assertEquals(listOf(elsewhere, anotherAccount), read.filter(all))
        guard.release(thread.id)
        assertEquals(rows + listOf(elsewhere, anotherAccount), read.filter(all))
        read.dispose()
    }

    @Test
    fun cancelledReadIsDisposed() =
        runBlocking {
            val guard = ThreadRemovalGuard()
            val started = CompletableDeferred<ThreadRemovalGuard.Read>()
            val gate = CompletableDeferred<Unit>()
            val job =
                launchThreadListRead(guard) { read ->
                    started.complete(read)
                    gate.await()
                }
            val read = started.await()
            job.cancelAndJoin()
            val removal = guard.suppress(thread.id)
            removal.complete()
            // A disposed read no longer receives later removals.
            assertEquals(rows, read.filter(rows))
        }
}
