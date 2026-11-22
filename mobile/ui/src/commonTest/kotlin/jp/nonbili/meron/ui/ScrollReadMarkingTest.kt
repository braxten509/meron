// The thread screen marks messages read according to their
// visibility in the viewport. The cases cover crossed
// message boundaries and a bubble that fits entirely on
// screen.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.MessageBody
import jp.nonbili.meron.shared.ThreadSummary
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class ScrollReadMarkingTest {
    @Test
    fun countsMessagesAboveTheViewportAsRead() {
        // Items 0-1 scrolled off the top entirely, item 2 is the first visible.
        val visible =
            listOf(
                ListItemGeometry(index = 2, offset = -40, size = 200),
                ListItemGeometry(index = 3, offset = 170, size = 300),
            )

        val read =
            readMessageIndices(
                visible = visible,
                firstVisibleIndex = 2,
                headerItemCount = 0,
                messageCount = 4,
                topSlackPx = 24,
                viewportEndOffset = 800,
            )

        // 2 is read because the reader scrolled through its top, 3 because its
        // bottom (470) is inside the viewport.
        assertEquals(listOf(0, 1, 2, 3), read)
    }

    @Test
    fun bubbleWholeOnScreenCountsWithoutScrollingPast() {
        // A short bubble fully inside the viewport has been read, even though
        // the reader never scrolled its top off the screen.
        val visible = listOf(ListItemGeometry(index = 0, offset = 100, size = 200))

        assertEquals(
            listOf(0),
            readMessageIndices(visible, 0, headerItemCount = 0, messageCount = 2, topSlackPx = 24, viewportEndOffset = 800),
        )
    }

    @Test
    fun tallBubbleCountsOnlyOnceItsTopPasses() {
        // Taller than the viewport: its bottom never comes into view, so passing
        // the top edge is the only thing that can mark it read.
        val started = ListItemGeometry(index = 0, offset = 0, size = 2000)
        val scrolledThrough = ListItemGeometry(index = 0, offset = -200, size = 2000)

        assertEquals(
            emptyList(),
            readMessageIndices(listOf(started), 0, headerItemCount = 0, messageCount = 2, topSlackPx = 24, viewportEndOffset = 800),
        )
        assertEquals(
            listOf(0),
            readMessageIndices(
                listOf(scrolledThrough),
                0,
                headerItemCount = 0,
                messageCount = 2,
                topSlackPx = 24,
                viewportEndOffset = 800,
            ),
        )
    }

    @Test
    fun bubblePeekingInFromTheBottomIsNotRead() {
        val visible =
            listOf(
                ListItemGeometry(index = 0, offset = 0, size = 700),
                ListItemGeometry(index = 1, offset = 710, size = 400),
            )

        assertEquals(
            listOf(0),
            readMessageIndices(visible, 0, headerItemCount = 0, messageCount = 2, topSlackPx = 24, viewportEndOffset = 800),
        )
    }

    @Test
    fun anchoredBubbleAtTheTopEdgeIsNotReadYet() {
        // Opening on the first unread scrolls it flush to the top edge; a pixel
        // of rounding must not count as having scrolled through it.
        val visible = listOf(ListItemGeometry(index = 0, offset = -1, size = 2000))

        assertEquals(
            emptyList(),
            readMessageIndices(visible, 0, headerItemCount = 0, messageCount = 2, topSlackPx = 24, viewportEndOffset = 800),
        )
    }

    @Test
    fun headerRowDoesNotCountAsAReadMessage() {
        // The load-older row (item 0) scrolled off; message 0 (item 1) is a tall
        // bubble the reader has only started.
        val visible = listOf(ListItemGeometry(index = 1, offset = 10, size = 3000))

        val read =
            readMessageIndices(
                visible = visible,
                firstVisibleIndex = 1,
                headerItemCount = 1,
                messageCount = 2,
                topSlackPx = 24,
                viewportEndOffset = 800,
            )

        assertEquals(emptyList(), read)
    }

    @Test
    fun nothingReadAtTheTopOfALongThread() {
        val visible =
            listOf(
                ListItemGeometry(index = 0, offset = 0, size = 900),
                ListItemGeometry(index = 1, offset = 910, size = 900),
            )

        assertEquals(
            emptyList(),
            readMessageIndices(visible, 0, headerItemCount = 0, messageCount = 2, topSlackPx = 24, viewportEndOffset = 800),
        )
    }

    @Test
    fun viewedToBottomRequiresLastItemNearViewportEnd() {
        val lastItemNear = listOf(ListItemGeometry(index = 4, offset = 700, size = 200))
        val lastItemFar = listOf(ListItemGeometry(index = 4, offset = 700, size = 500))
        val notLastItem = listOf(ListItemGeometry(index = 3, offset = 700, size = 100))

        assertTrue(listViewedToBottom(lastItemNear, totalItemCount = 5, viewportEndOffset = 800, bottomSlackPx = 160))
        assertFalse(listViewedToBottom(lastItemFar, totalItemCount = 5, viewportEndOffset = 800, bottomSlackPx = 160))
        assertFalse(listViewedToBottom(notLastItem, totalItemCount = 5, viewportEndOffset = 800, bottomSlackPx = 160))
        assertFalse(listViewedToBottom(emptyList(), totalItemCount = 5, viewportEndOffset = 800, bottomSlackPx = 160))
    }

    @Test
    fun shortThreadThatFitsTheViewportCountsAsViewedToBottom() {
        val visible =
            listOf(
                ListItemGeometry(index = 0, offset = 0, size = 200),
                ListItemGeometry(index = 1, offset = 210, size = 200),
            )

        assertTrue(listViewedToBottom(visible, totalItemCount = 2, viewportEndOffset = 800, bottomSlackPx = 160))
    }

    @Test
    fun manualUnreadIsHeldOutOfScrollMarking() {
        val previousUnread = mapOf("m1" to false, "m2" to false)

        val held = manualUnreadIds(listOf(message("m1", unread = true), message("m2")), previousUnread)

        assertEquals(listOf("m1"), held)
    }

    @Test
    fun messagesTheThreadArrivedUnreadAreNotHeld() {
        // Nothing seen before, or already unread last time: opening a thread on
        // its unread messages must still mark them read as the reader goes
        // through them.
        assertEquals(emptyList(), manualUnreadIds(listOf(message("m1", unread = true)), emptyMap()))
        assertEquals(
            emptyList(),
            manualUnreadIds(listOf(message("m1", unread = true)), mapOf("m1" to true)),
        )
    }

    @Test
    fun messagesJustMarkedReadAreNotHeld() {
        assertEquals(emptyList(), manualUnreadIds(listOf(message("m1")), mapOf("m1" to true)))
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

    @Test
    fun partialReadDecrementsThreadUnreadCount() {
        val updated = threadWithUnreadDelta(thread(unreadCount = 3), delta = -1)

        assertTrue(updated.unread)
        assertEquals(2, updated.unreadCount)
    }

    @Test
    fun readingLastUnreadMessageClearsThreadState() {
        val updated = threadWithUnreadDelta(thread(unreadCount = 1), delta = -1)

        assertFalse(updated.unread)
        assertEquals(0, updated.unreadCount)
    }

    @Test
    fun turningMessagesBackUnreadRaisesThreadUnreadCount() {
        val updated = threadWithUnreadDelta(thread(unreadCount = 0).copy(unread = false), delta = 2)

        assertTrue(updated.unread)
        assertEquals(2, updated.unreadCount)
    }

    @Test
    fun partialReadKeepsUnreadMessagesFromOlderPages() {
        val updated = threadWithUnreadDelta(thread(unreadCount = 4), delta = -2)

        assertTrue(updated.unread)
        assertEquals(2, updated.unreadCount)
    }

    private fun thread(unreadCount: Int) =
        ThreadSummary(
            id = "acc#INBOX#thread",
            accountId = "acc",
            folder = "INBOX",
            subject = "Subject",
            sender = "Sender",
            unread = true,
            unreadCount = unreadCount,
        )
}
