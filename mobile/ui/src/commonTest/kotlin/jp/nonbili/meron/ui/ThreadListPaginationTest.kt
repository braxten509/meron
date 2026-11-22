// The list requests another page as the viewport approaches
// its end. The tests check the remaining-row threshold and
// avoid triggering while the reader is far from the
// boundary.

package jp.nonbili.meron.ui

import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class ThreadListPaginationTest {
    @Test
    fun scrollingWithinThreeRowsOfTheEndPaginates() {
        assertTrue(threadListNearBottom(lastVisibleIndex = 47, threadCount = 50))
        assertTrue(threadListNearBottom(lastVisibleIndex = 49, threadCount = 50))
    }

    @Test
    fun scrollPositionWellAboveTheEndDoesNotPaginate() {
        assertFalse(threadListNearBottom(lastVisibleIndex = 12, threadCount = 50))
        assertFalse(threadListNearBottom(lastVisibleIndex = 46, threadCount = 50))
    }

    // Nothing has been measured on the first frame; the naive comparison read
    // that as index 0 and paginated before the list was ever on screen.
    @Test
    fun listWithNothingLaidOutYetDoesNotPaginate() {
        assertFalse(threadListNearBottom(lastVisibleIndex = null, threadCount = 50))
    }

    @Test
    fun emptyListDoesNotPaginate() {
        assertFalse(threadListNearBottom(lastVisibleIndex = null, threadCount = 0))
        assertFalse(threadListNearBottom(lastVisibleIndex = 0, threadCount = 0))
    }

    // A page shorter than the viewport is legitimately at its end, and should
    // keep pulling while a cursor remains.
    @Test
    fun shortListFullyOnScreenPaginates() {
        assertTrue(threadListNearBottom(lastVisibleIndex = 1, threadCount = 2))
    }
}
