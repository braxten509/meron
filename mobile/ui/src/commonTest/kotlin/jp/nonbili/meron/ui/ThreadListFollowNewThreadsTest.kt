// Incoming mail can prepend rows to a mobile thread list.
// The tests distinguish following new rows at the top from
// preserving position for someone reading further down.

package jp.nonbili.meron.ui

import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class ThreadListFollowNewThreadsTest {
    // Four new mails arrive against a list the user left at the top: whichever
    // side of the re-anchoring measure this lands on, the laid-out frame has the
    // old first row flush with the viewport top.
    @Test
    fun listAtTopFollowsPrependedMail() {
        assertTrue(
            threadListShouldFollowNewThreads(
                previousFirstId = "thread-9",
                previousFirstIndex = 4,
                firstVisibleKey = "thread-9",
                firstVisibleOffset = 0,
                savedFirstVisibleIndex = 0,
                savedFirstVisibleScrollOffset = 0,
            ),
        )
    }

    @Test
    fun scrolledListKeepsItsPlace() {
        assertFalse(
            threadListShouldFollowNewThreads(
                previousFirstId = "thread-9",
                previousFirstIndex = 4,
                firstVisibleKey = "thread-31",
                firstVisibleOffset = 0,
                savedFirstVisibleIndex = 21,
                savedFirstVisibleScrollOffset = 0,
            ),
        )
    }

    // The trap in reading the scroll index instead of the layout: a user sitting
    // at index 4 has the same index as the old first row's new position, so an
    // index comparison reads them as being at the top. The laid-out row does not.
    @Test
    fun listScrolledExactlyAsFarAsThePrependKeepsItsPlace() {
        assertFalse(
            threadListShouldFollowNewThreads(
                previousFirstId = "thread-9",
                previousFirstIndex = 4,
                firstVisibleKey = "thread-5",
                firstVisibleOffset = 0,
                savedFirstVisibleIndex = 4,
                savedFirstVisibleScrollOffset = 0,
            ),
        )
    }

    // Nudged just off the top — still the user's position, not ours to move.
    @Test
    fun listScrolledPartwayDownTheFirstRowKeepsItsPlace() {
        assertFalse(
            threadListShouldFollowNewThreads(
                previousFirstId = "thread-9",
                previousFirstIndex = 4,
                firstVisibleKey = "thread-9",
                firstVisibleOffset = -37,
                savedFirstVisibleIndex = 0,
                savedFirstVisibleScrollOffset = 37,
            ),
        )
    }

    // Returning to a list that was never laid out, or whose layout was dropped:
    // the retained scroll position is the only account of where the user was,
    // and it has not been re-anchored yet.
    @Test
    fun unlaidListFallsBackToTheRetainedScrollPosition() {
        assertTrue(
            threadListShouldFollowNewThreads(
                previousFirstId = "thread-9",
                previousFirstIndex = 4,
                firstVisibleKey = null,
                firstVisibleOffset = 0,
                savedFirstVisibleIndex = 0,
                savedFirstVisibleScrollOffset = 0,
            ),
        )
        assertFalse(
            threadListShouldFollowNewThreads(
                previousFirstId = "thread-9",
                previousFirstIndex = 4,
                firstVisibleKey = null,
                firstVisibleOffset = 0,
                savedFirstVisibleIndex = 4,
                savedFirstVisibleScrollOffset = 0,
            ),
        )
    }

    // Folder change, search, or the old top row being archived: the list was
    // replaced rather than grown, and the fresh state starts at the top anyway.
    @Test
    fun wholesaleReplacementDoesNotScroll() {
        assertFalse(
            threadListShouldFollowNewThreads(
                previousFirstId = "thread-9",
                previousFirstIndex = -1,
                firstVisibleKey = "thread-9",
                firstVisibleOffset = 0,
                savedFirstVisibleIndex = 0,
                savedFirstVisibleScrollOffset = 0,
            ),
        )
    }

    @Test
    fun listThatDidNotGrowAtTheTopDoesNotScroll() {
        assertFalse(
            threadListShouldFollowNewThreads(
                previousFirstId = "thread-9",
                previousFirstIndex = 0,
                firstVisibleKey = "thread-9",
                firstVisibleOffset = 0,
                savedFirstVisibleIndex = 0,
                savedFirstVisibleScrollOffset = 0,
            ),
        )
    }
}
