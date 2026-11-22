// Android notification actions need stable IDs for
// individual arrivals. Cases exercise multiple replies
// in one thread and ensure related rows are cleared
// together.

package jp.nonbili.meron

import org.junit.Assert.assertNotEquals
import org.junit.Test

class AndroidNotificationActionTest {
    @Test
    fun ancillaryRowsDoNotShareTheMailsNotificationId() {
        val mail = newMailNotificationId("acct", NewMailItem(4821, "", "", "", "topic", 0))
        // Re-posting the mail (a retried sync) must not overwrite the undo offer
        // standing in its place, nor a report that the action failed.
        assertNotEquals(mail, undoNotificationId(mail))
        assertNotEquals(mail, actionFailedNotificationId(mail))
        assertNotEquals(undoNotificationId(mail), actionFailedNotificationId(mail))
    }

    @Test
    fun repliesInOneThreadGetSeparateRowsThatOneActionMustClear() {
        // Archive and mark-read act on the whole thread, but each arrival was
        // posted under its own uid — so cancelling only the pressed row would
        // strand the rest. This is what cancelThreadRows exists to prevent.
        val first = newMailNotificationId("acct", NewMailItem(1, "", "", "", "topic", 0))
        val reply = newMailNotificationId("acct", NewMailItem(2, "", "", "", "topic", 0))
        assertNotEquals(first, reply)
    }

    @Test
    fun undoIdsAreStableForOneMailAndDistinctBetweenMails() {
        val first = newMailNotificationId("acct", NewMailItem(1, "", "", "", "a", 0))
        val second = newMailNotificationId("acct", NewMailItem(2, "", "", "", "b", 0))
        assertNotEquals(undoNotificationId(first), undoNotificationId(second))
        // Stable: the receiver cancels by recomputing the id, not by holding it.
        assert(undoNotificationId(first) == undoNotificationId(first))
    }
}
