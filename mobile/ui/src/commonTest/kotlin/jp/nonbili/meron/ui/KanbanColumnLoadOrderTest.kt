// The board loads destination folders independently of opening
// the copy picker. These tests control request order to verify
// reuse of folder data and selection of recent destinations.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.MobileCommand
import jp.nonbili.meron.shared.ThreadSummary
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertSame

/**
 * Two reloads of one Kanban column can be out at once — the Sent copy event's
 * and a quick reply's post-discard one — and the earlier read the store before
 * the discard. Landing last, it used to put the card back with the draft
 * counted and badged; only the newest request may write.
 */

internal class KanbanColumnLoadOrderTest : BoardLoadFixture() {
    @Test
    fun staleColumnLoadLandingLastDoesNotOverwriteTheNewerRows() =
        runBlocking {
            val core = GatedCore()
            val state = state(core, this)
            val column = KanbanColumnSpec(accountId = "a", folderId = "INBOX")
            val key = kanbanColumnKey(column)

            state.loadKanbanColumn(column)
            waitUntil { core.threadListCalls == 1 }
            // The post-discard reload, while the first one is still out.
            state.loadKanbanColumn(column)
            waitUntil { state.kanbanColumns[key]?.threads?.isNotEmpty() == true }
            val fresh = state.kanbanColumns[key]?.threads?.single()
            assertEquals(2, fresh?.messageCount)

            core.gate.complete(Unit)
            delay(50)
            val card = state.kanbanColumns[key]?.threads?.single()
            assertEquals(2, card?.messageCount)
            assertFalse(card?.hasDraft ?: true)
            assertFalse(state.kanbanColumns[key]?.loading ?: true)
        }

    @Test
    fun archiveCannotBeRestoredByAnOlderMailboxReload() = removalSurvivesLateRead(kanban = false, loadMore = false)

    @Test
    fun archiveCannotBeRestoredByAnOlderColumnReload() = removalSurvivesLateRead(kanban = true, loadMore = false)

    @Test
    fun deleteCannotBeRestoredByOlderMailboxPagination() = removalSurvivesLateRead(kanban = false, loadMore = true)

    @Test
    fun moveCannotBeRestoredByOlderColumnPagination() = removalSurvivesLateRead(kanban = true, loadMore = true)

    @Test
    fun failedArchiveRestoresRowsAndReleasesTheOlderRead() = removalSurvivesLateRead(kanban = false, loadMore = false, actionFails = true)

    @Test
    fun failedColumnMoveReloadsASourceRowHiddenByAPendingRead() =
        runBlocking {
            val childScope = CoroutineScope(coroutineContext + Job(coroutineContext[Job]))
            try {
                val core = GatedCore(gateActions = true, actionFails = true)
                val state = state(core, childScope)
                val thread = ThreadSummary(id = "a:INBOX:t1", accountId = "a", folder = "INBOX", subject = "Hello", sender = "Ada")
                val column = KanbanColumnSpec(accountId = "a", folderId = "INBOX")
                val key = kanbanColumnKey(column)
                state.kanbanBoards = listOf(KanbanBoardSpec(id = "board", name = "Board", columns = listOf(column)))
                state.activeKanbanBoardId = "board"
                state.kanbanColumns = mapOf(key to KanbanColumnState(threads = listOf(thread)))
                state.loadKanbanColumn(column)
                waitUntil { core.threadListCalls == 1 }
                state.moveThreadToColumn(thread, KanbanColumnSpec(accountId = "a", folderId = "Archive"))
                waitUntil { core.actionCalls == 1 }
                core.gate.complete(Unit)
                waitUntil { state.kanbanColumns[key]?.loading == false }
                assertEquals(emptyList(), state.kanbanColumns[key]?.threads)
                core.actionGate.complete(Unit)
                waitUntil { state.kanbanColumns[key]?.threads?.isNotEmpty() == true && !state.syncing }
                assertEquals(
                    thread.id,
                    state.kanbanColumns[key]
                        ?.threads
                        ?.single()
                        ?.id,
                )
                assertEquals(thread.id, state.coreThreads.single().id)
                waitUntil { state.statusIsError }
                assertEquals("Action rejected", state.status)
                state.status = "Move complete"
                assertFalse(state.statusIsError)
            } finally {
                childScope.cancel()
            }
        }

    @Test
    fun notificationMailboxCannotRestoreAnArchivedThread() = removalSurvivesLateRead(kanban = false, loadMore = false, notification = true)

    @Test
    fun aCompletedColumnMoveDoesNotReleaseAnOverlappingArchive() =
        runBlocking {
            val childScope = CoroutineScope(coroutineContext + Job(coroutineContext[Job]))
            try {
                val core = GatedCore()
                val moveGate = CompletableDeferred<Unit>()
                val archiveGate = CompletableDeferred<Unit>()
                core.actionGates[MobileCommand.Move] = moveGate
                core.actionGates[MobileCommand.Archive] = archiveGate
                val state = state(core, childScope)
                val thread = ThreadSummary(id = "a:INBOX:t1", accountId = "a", folder = "INBOX", subject = "Hello", sender = "Ada")
                state.coreThreads = listOf(thread)
                state.moveThreadToColumn(thread, KanbanColumnSpec(accountId = "a", folderId = "Archive"))
                state.archiveOrRemove(thread)
                waitUntil { core.actionCalls == 2 }
                moveGate.complete(Unit)
                waitUntil { state.status == "Move complete" }
                // Begin a mailbox read after the first mutation has completed.
                state.openNotificationThread(NotificationThreadTarget(accountId = "a", folder = "INBOX"))
                waitUntil { core.threadListCalls >= 2 }
                waitUntil { !state.syncing }
                assertEquals(emptyList(), state.coreThreads)
                archiveGate.complete(Unit)
                waitUntil { state.threadRemovalGuard.filter(listOf(thread)).isNotEmpty() }
                core.gate.complete(Unit)
                waitUntil { state.kanbanColumns.values.none { it.loading } }
            } finally {
                childScope.cancel()
            }
        }

    @Test
    fun cachedMailboxesRetainTheirOtherRowsAndCursorsAfterRemoval() = checkCachedRemoval(success = true)

    @Test
    fun overlappingFailuresRestoreCachedRowsAfterAnInterveningRead() = checkCachedRemoval(success = false)

    @Test
    fun manualMoveBackReleasesTheDestinationInASlowMailboxRead() =
        runBlocking {
            val childScope = CoroutineScope(coroutineContext + Job(coroutineContext[Job]))
            try {
                val core = GatedCore()
                val state = state(core, childScope)
                val thread = ThreadSummary(id = "a:INBOX:t1", accountId = "a", folder = "INBOX", subject = "Hello", sender = "Ada")
                state.coreThreads = listOf(thread)
                state.syncCoreThreads(syncFirst = false)
                waitUntil { core.threadListCalls == 1 }
                var moves = 0
                state.moveThreadToFolder(thread, "Archive") { moves++ }
                waitUntil { moves == 1 }
                state.moveThreadToFolder(thread.copy(id = "a:Archive:t1", folder = "Archive"), "INBOX") { moves++ }
                waitUntil { moves == 2 }
                core.gate.complete(Unit)
                waitUntil { !state.syncing }
                assertEquals(thread.id, state.coreThreads.single().id)
            } finally {
                childScope.cancel()
            }
        }

    @Test
    fun emptyFolderKeepsUnloadedRowsOutOfAnOlderMailboxRead() =
        runBlocking {
            val childScope = CoroutineScope(coroutineContext + Job(coroutineContext[Job]))
            try {
                val core = GatedCore(gateActions = true, emptyAfterActions = true, listedFolder = "Trash")
                val state = state(core, childScope)
                state.selectedCoreFolder = "Trash"
                state.syncCoreThreads(syncFirst = false)
                waitUntil { core.threadListCalls == 1 }
                state.emptyMailFolder("a", "Trash")
                waitUntil { core.actionCalls == 1 }
                core.gate.complete(Unit)
                waitUntil { !state.syncing }
                assertEquals(emptyList(), state.coreThreads)
                core.actionGate.complete(Unit)
                waitUntil { core.threadListCalls == 2 && !state.syncing }
                assertEquals(emptyList(), state.coreThreads)
            } finally {
                childScope.cancel()
            }
        }

    @Test
    fun permanentDeleteCannotReleaseItsSourceIdFromChange() = removalSurvivesLateRead(kanban = false, loadMore = true, permanentDelete = true)

    @Test
    fun undoMovesFromTheTopLevelDestinationInsteadOfTheChangeSource() =
        runBlocking {
            val core = GatedCore()
            val state = state(core, this)
            val thread = ThreadSummary(id = "a#INBOX#t1", accountId = "a", folder = "INBOX", subject = "Hello", sender = "Ada")
            // The Undo tap puts the row back; restoreThread then moves the mail.
            state.showRestoredThread(setOf(thread.id), null, listOf(thread), emptyMap())
            state.restoreThread(
                thread,
                listOf(thread),
                emptyMap(),
                """{"change":{"source_folder":"INBOX","thread_id":"a#INBOX#t1"},"folder":"Archive","moved":1,"ok":true,"target_uids":[7],"thread_id":"a#Archive#t1"}""",
            )
            waitUntil { core.actionCalls == 1 }
            val (command, payload) = core.actionRequests.single()
            assertEquals(MobileCommand.Move, command)
            assertEquals("a#Archive#t1", payload.jsonStringValue("thread_id"))
            assertEquals("INBOX", payload.jsonStringValue("target_folder_id"))
            assertEquals(listOf(thread), state.coreThreads)
        }

    @Test
    fun permanentDeleteWithOnlyAChangeSourceDoesNotOfferUndo() =
        runBlocking {
            val core = GatedCore()
            val state = state(core, this)
            val thread = ThreadSummary(id = "a#Trash#t1", accountId = "a", folder = "Trash", subject = "Hello", sender = "Ada")
            state.restoreThread(
                thread,
                listOf(thread),
                emptyMap(),
                """{"change":{"source_folder":"Trash","thread_id":"a#Trash#t1"},"deleted":1,"ok":true,"permanent":true}""",
            )
            assertEquals("Undo unavailable", state.status)
            assertEquals(emptyList(), core.actionRequests)
            assertEquals(emptyList(), state.coreThreads)
        }

    @Test
    fun cacheRollbackLeavesUnaffectedMailboxOrderAndInstanceAlone() =
        runBlocking {
            val state = state(GatedCore(), this)
            val thread = ThreadSummary(id = "a:INBOX:t1", accountId = "a", folder = "INBOX", subject = "Hello", sender = "Ada")
            val older = thread.copy(id = "a:Archive:older", folder = "Archive", dateEpochSeconds = 1)
            val newer = older.copy(id = "a:Archive:newer", dateEpochSeconds = 2)
            val affectedKey = mailboxCacheKey("a", "INBOX", "", FilterMode.All)
            val untouchedKey = mailboxCacheKey("a", "Archive", "", FilterMode.All)
            val unaffected = MailboxLoadResult(emptyList(), "Archive", listOf(older, newer))
            state.mailboxCache =
                mapOf(
                    affectedKey to MailboxLoadResult(emptyList(), "INBOX", listOf(thread)),
                    untouchedKey to unaffected,
                )
            val removal = state.suppressRemovedThread(thread.id)
            removal.rollback()
            assertSame(unaffected, state.mailboxCache[untouchedKey])
            assertEquals(listOf(older, newer), state.mailboxCache[untouchedKey]?.threads)
        }
}
