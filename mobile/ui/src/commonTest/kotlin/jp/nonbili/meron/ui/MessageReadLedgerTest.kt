// Marking a message read updates its visible state
// while the write is in flight. The ledger tests apply
// success and failure responses to the pending change.

package jp.nonbili.meron.ui

import kotlin.test.Test
import kotlin.test.assertEquals

class MessageReadLedgerTest {
    private fun message(unread: Boolean) = listOf(Triple("a", "INBOX", unread))

    // The state the message is told to show, and whether the ledger saw it move.
    private fun List<MessageReadLedger.Change>.state() = single().let { it.unread to it.moved }

    private fun MessageReadLedger.install(unread: Boolean): Boolean = overlay(listOf(unread), { "a" }, { it }) { _, pending -> pending }.single()

    @Test
    fun failedMarkTurnsTheMessageBack() {
        val ledger = MessageReadLedger()
        val (op, begun) = ledger.begin(message(unread = true), unread = false)

        assertEquals(false to true, begun.state())
        assertEquals(true to true, ledger.finish(op, succeeded = false).state())
    }

    @Test
    fun succeededMarkKeepsTheMessageRead() {
        val ledger = MessageReadLedger()
        val (op, _) = ledger.begin(message(unread = true), unread = false)

        assertEquals(false to false, ledger.finish(op, succeeded = true).state())
    }

    // An automatic read is in flight when the reader marks the message unread;
    // both fail. Neither reached the server, so the message is unread — not the
    // read state the manual toggle saw when it began.
    @Test
    fun queuedManualUnreadFailingAfterAutomaticReadFailureLeavesTheMessageUnread() {
        val ledger = MessageReadLedger()
        val (auto, _) = ledger.begin(message(unread = true), unread = false)
        val (manual, begun) = ledger.begin(message(unread = false), unread = true)
        assertEquals(true to true, begun.state())

        // The pending manual unread still decides what shows.
        assertEquals(true to false, ledger.finish(auto, succeeded = false).state())
        assertEquals(true to false, ledger.finish(manual, succeeded = false).state())
    }

    @Test
    fun manualUnreadFailingAfterWholeThreadSuccessShowsRead() {
        val ledger = MessageReadLedger()
        val (whole, _) = ledger.begin(message(unread = true), unread = false)
        val (manual, _) = ledger.begin(message(unread = false), unread = true)

        assertEquals(true to false, ledger.finish(whole, succeeded = true).state())
        // The server has the whole-thread read; the unread never landed.
        assertEquals(false to true, ledger.finish(manual, succeeded = false).state())
    }

    @Test
    fun wholeThreadSuccessDoesNotOverrideANewerManualUnread() {
        val ledger = MessageReadLedger()
        val (whole, _) = ledger.begin(message(unread = true), unread = false)
        val (manual, _) = ledger.begin(message(unread = false), unread = true)

        assertEquals(true to false, ledger.finish(whole, succeeded = true).state())
        assertEquals(true to false, ledger.finish(manual, succeeded = true).state())
    }

    // A per-message read fails while the whole-thread read covering the same
    // message is still pending: the message stays read, and the whole-thread
    // success confirms it.
    @Test
    fun pendingWholeThreadReadCoversAFailedPerMessageRead() {
        val ledger = MessageReadLedger()
        val (perMessage, _) = ledger.begin(message(unread = true), unread = false)
        val (whole, begun) = ledger.begin(message(unread = false), unread = false)
        assertEquals(false to false, begun.state())

        assertEquals(false to false, ledger.finish(perMessage, succeeded = false).state())
        assertEquals(false to false, ledger.finish(whole, succeeded = true).state())
    }

    @Test
    fun bothReadsFailingTurnTheMessageBackOnce() {
        val ledger = MessageReadLedger()
        val (perMessage, _) = ledger.begin(message(unread = true), unread = false)
        val (whole, _) = ledger.begin(message(unread = false), unread = false)

        assertEquals(false to false, ledger.finish(perMessage, succeeded = false).state())
        assertEquals(true to true, ledger.finish(whole, succeeded = false).state())
    }

    // A reload lands while the read is in flight and hands back the core's
    // still-unread copy. The pending read is laid over it, and the success
    // still reports the message read, so the UI cannot be left showing unread.
    @Test
    fun reloadBetweenBeginAndSuccessStillShowsTheRead() {
        val ledger = MessageReadLedger()
        val (op, _) = ledger.begin(message(unread = true), unread = false)

        assertEquals(false, ledger.install(unread = true))
        assertEquals(false to false, ledger.finish(op, succeeded = true).state())
        // Settled: a later install is the core's own state again.
        assertEquals(true, ledger.install(unread = true))
    }

    // A second op begun on a stale reloaded copy reports the ledger's state,
    // not the stale one it was handed.
    @Test
    fun beginWhilePendingReportsThePendingState() {
        val ledger = MessageReadLedger()
        ledger.begin(message(unread = true), unread = false)

        val (_, begun) = ledger.begin(message(unread = true), unread = false)

        assertEquals(false to false, begun.state())
    }

    @Test
    fun settledMessagesAreForgottenSoAReloadedStateIsTakenAsIs() {
        val ledger = MessageReadLedger()
        val (first, _) = ledger.begin(message(unread = true), unread = false)
        ledger.finish(first, succeeded = true)

        // A reload now shows it unread (marked elsewhere); a new op starts there.
        val (second, _) = ledger.begin(message(unread = true), unread = false)
        assertEquals(true to true, ledger.finish(second, succeeded = false).state())
    }
}
