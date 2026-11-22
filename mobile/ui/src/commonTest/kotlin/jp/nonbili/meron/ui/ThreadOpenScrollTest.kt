// Opening a conversation needs a scroll target among
// message and header rows. The tests exercise an unread
// destination and the offset contributed by a load-older
// header.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.MessageBody
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class ThreadOpenScrollTest {
    @Test
    fun jumpsToFirstUnreadMessage() {
        val messages = listOf(message("m1"), message("m2", unread = true), message("m3", unread = true))

        // Message 1, one item below the subject header.
        assertEquals(2, threadOpenScrollIndex(messages, hasLoadOlderRow = false))
    }

    @Test
    fun offsetsForLoadOlderHeaderRow() {
        val messages = listOf(message("m1"), message("m2", unread = true))

        assertEquals(3, threadOpenScrollIndex(messages, hasLoadOlderRow = true))
    }

    @Test
    fun jumpsToNewestMessageWhenAllRead() {
        val messages = listOf(message("m1"), message("m2"), message("m3"))

        assertEquals(3, threadOpenScrollIndex(messages, hasLoadOlderRow = false))
        assertEquals(4, threadOpenScrollIndex(messages, hasLoadOlderRow = true))
    }

    @Test
    fun staysPutWhenTargetIsAlreadyAtTop() {
        assertNull(threadOpenScrollIndex(listOf(message("m1", unread = true), message("m2")), hasLoadOlderRow = false))
        assertNull(threadOpenScrollIndex(listOf(message("m1")), hasLoadOlderRow = false))
        assertNull(threadOpenScrollIndex(emptyList(), hasLoadOlderRow = true))
    }

    @Test
    fun firstUnreadStillNeedsScrollPastHeaderRow() {
        val messages = listOf(message("m1", unread = true), message("m2"))

        // Landing on the load-older row would auto-load the older page, so the
        // subject header scrolls away instead.
        assertEquals(2, threadOpenScrollIndex(messages, hasLoadOlderRow = true))
    }

    private fun message(
        id: String,
        unread: Boolean = false,
    ): MessageBody =
        MessageBody(
            id = id,
            from = "sender@example.com",
            to = "me@example.com",
            subject = "Subject",
            body = "Body",
            unread = unread,
        )
}
