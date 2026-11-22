// Board move destinations test.
// The mobile host suites exercise these cases with deterministic core responses.
// The tests run without a device or a live mail account.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.FolderSummary
import jp.nonbili.meron.shared.ThreadSummary
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals

/**
 * Two reloads of one Kanban column can be out at once — the Sent copy event's
 * and a quick reply's post-discard one — and the earlier read the store before
 * the discard. Landing last, it used to put the card back with the draft
 * counted and badged; only the newest request may write.
 */

internal class BoardMoveDestinationsTest : BoardLoadFixture() {
    @Test
    fun missingRecentDestinationsDoNotReloadAnAlreadyLoadedAccount() =
        runBlocking {
            val core = GatedCore()
            val state = state(core, this)
            val thread = ThreadSummary(id = "a#Work#t1", accountId = "a", folder = "Work", subject = "Subject", sender = "Sender")
            state.recentThreadTransfers = listOf(RecentThreadTransfer("copy", "a", "a", "Deleted"))
            state.loadRecentTransferFolders(thread)
            assertEquals(1, core.folderListCalls)
            repeat(3) { state.loadRecentTransferFolders(thread) }
            assertEquals(1, core.folderListCalls)
            state.foldersByAccount = mapOf("a" to emptyList())
            state.loadRecentTransferFolders(thread)
            assertEquals(1, core.folderListCalls)
        }

    @Test
    fun recentFoldersLoadWithoutOpeningTheCopyPicker() =
        runBlocking {
            val state = state(GatedCore(), this)
            val thread = ThreadSummary(id = "a#Work#t1", accountId = "a", folder = "Work", subject = "Subject", sender = "Sender")
            state.recentThreadTransfers = listOf(RecentThreadTransfer("copy", "a", "a", "INBOX"))
            assertEquals(emptyList(), state.recentTransferItems(thread))
            state.loadRecentTransferFolders(thread)
            assertEquals(listOf("INBOX"), state.recentTransferItems(thread).map { it.folder.name })
            state.foldersByAccount = mapOf("a" to listOf(FolderSummary("a", "work"), FolderSummary("a", "INBOX")))
            state.recentThreadTransfers = listOf(RecentThreadTransfer("move", "a", "a", "work"), RecentThreadTransfer("copy", "a", "a", "inbox"))
            assertEquals(listOf("work", "INBOX"), state.recentTransferItems(thread).map { it.folder.name })
            assertEquals(listOf("work"), state.recentTransferItems(thread.copy(folder = "inbox")).map { it.folder.name })
        }

    @Test
    fun crossAccountMoveKeepsTheTargetAccountForMenusAndBoardMoves() =
        runBlocking {
            for (boardMove in listOf(false, true)) {
                val core = GatedCore()
                core.gate.complete(Unit)
                val state = state(core, this)
                val thread = ThreadSummary(id = "a#INBOX#t1", accountId = "a", folder = "INBOX", subject = "Subject", sender = "Sender")
                state.coreAccounts = state.coreAccounts + AccountSummary(id = "b", email = "b@example.com")
                if (boardMove) {
                    state.moveThreadToColumn(thread, KanbanColumnSpec("b", "INBOX"))
                } else {
                    state.moveThreadToFolder(thread, "INBOX", "b")
                }
                waitUntil { state.recentThreadTransfers.isNotEmpty() }
                assertEquals(
                    "b",
                    core.actionRequests
                        .single()
                        .second
                        .jsonStringValue("target_account_id"),
                )
                assertEquals(RecentThreadTransfer("move", "a", "b", "INBOX"), state.recentThreadTransfers.single())
            }
        }

    @Test
    fun kanbanDragRecordsItsDestinationAndRejectsZeroMoves() =
        runBlocking {
            for (zero in listOf(false, true)) {
                val core = GatedCore().apply { zeroMove = zero }
                core.gate.complete(Unit)
                val state = state(core, this)
                val thread = ThreadSummary(id = "a#INBOX#t1", accountId = "a", folder = "INBOX", subject = "Subject", sender = "Sender")
                state.moveThreadToColumn(thread, KanbanColumnSpec("a", "Work"))
                waitUntil { core.actionCalls > 0 }
                delay(50)
                assertEquals(if (zero) emptyList() else listOf(RecentThreadTransfer("move", "a", "a", "Work")), state.recentThreadTransfers)
            }
        }

    @Test
    fun successfulMovesPromoteRecentDestinationsWithoutDuplicates() =
        runBlocking {
            val core = GatedCore()
            val state = state(core, this)
            val first = RecentThreadTransfer("move", "a", "a", "Work")
            state.rememberThreadTransfer(first)
            state.rememberThreadTransfer(first.copy(action = "copy", targetFolderId = "Receipts"))
            state.rememberThreadTransfer(first)
            assertEquals(listOf(first, first.copy(action = "copy", targetFolderId = "Receipts")), state.recentThreadTransfers)
            assertEquals(state.recentThreadTransfers, loadRecentThreadTransfers(state.prefs))
        }

    @Test
    fun recentShortcutsExcludeCurrentMissingAndOtherAccountDestinationsAndStopAtTwo() =
        runBlocking {
            val state = state(GatedCore(), this)
            state.coreAccounts = listOf(AccountSummary(id = "a", email = "a@example.com"))
            state.foldersByAccount = mapOf("a" to listOf("INBOX", "Work", "Receipts", "Older").map { FolderSummary("a", it) })
            state.recentThreadTransfers =
                listOf(
                    RecentThreadTransfer("copy", "b", "a", "Work"),
                    RecentThreadTransfer("move", "a", "a", "INBOX"),
                    RecentThreadTransfer("move", "a", "a", "Deleted"),
                    RecentThreadTransfer("move", "a", "a", "Work"),
                    RecentThreadTransfer("copy", "a", "a", "Receipts"),
                    RecentThreadTransfer("move", "a", "a", "Older"),
                    RecentThreadTransfer("move", "a", "a", "Receipts"),
                    RecentThreadTransfer("move", "a", "a", "INBOX"),
                )
            val thread = ThreadSummary(id = "a#INBOX#t1", accountId = "a", folder = "INBOX", subject = "Subject", sender = "Sender")
            val items = state.recentTransferItems(thread)
            assertEquals(listOf("Work", "Receipts", "Older", "Receipts"), items.map { it.destination })
            assertEquals(listOf("Work", "Older", "Receipts"), items.forAction("move").map { it.destination })
            assertEquals(listOf("Receipts"), items.forAction("copy").map { it.destination })
            assertEquals(emptyList(), state.recentTransferItems(null))
        }
}
