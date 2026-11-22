// Thread refresh uses a message identity as its paging
// boundary. Equal timestamps or a removed boundary
// message must still yield the right amount of
// history.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.MessageBody
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

// How far back a refresh of the open conversation has to re-read so the older
// pages the reader loaded stay on screen.
class ThreadRefreshBoundaryTest {
    @Test
    fun aSharedTimestampDoesNotStandInForTheBoundaryMessage() {
        val boundary = message("m1", 1000)
        val newestPage = (31..60).map { message("m$it", 1000) }

        assertFalse(refreshReachedBoundary(newestPage, boundary))
        assertTrue(refreshReachedBoundary(listOf(boundary) + newestPage, boundary))
    }

    @Test
    fun anOlderMessageCoversABoundaryThatWasDeleted() {
        val boundary = message("gone", 1000)

        assertFalse(refreshReachedBoundary(listOf(message("m2", 1000), message("m3", 2000)), boundary))
        assertTrue(refreshReachedBoundary(listOf(message("m0", 900), message("m3", 2000)), boundary))
    }

    @Test
    fun nothingLoadedNeedsOnlyTheNewestPage() {
        assertTrue(refreshReachedBoundary(listOf(message("m1", 1000)), null))
    }

    @Test
    fun theBoundaryIsTheOldestMessageTheCoreSent() {
        val messages = listOf(message("local-draft-1", 500), message("m1", 1000), message("local-send-1", 2000))

        assertEquals("m1", oldestServerMessage(messages)?.id)
        assertEquals(null, oldestServerMessage(messages.filterNot { it.id == "m1" }))
    }

    @Test
    fun aBoundaryCollapsedIntoItsCopyInAnotherFolderCounts() {
        val boundary = message("inbox-1", 1000).copy(messageId = "<Twin@example.com>")
        val copy = message("sent-1", 1000).copy(messageId = "twin@example.com")

        assertTrue(refreshReachedBoundary(listOf(copy, message("m2", 2000)), boundary))
        assertFalse(refreshReachedBoundary(listOf(message("m2", 1000)), boundary))
    }

    @Test
    fun aRowCollapsedIntoItsCopyIsNotKeptBesideIt() {
        val twin = message("inbox-1", 1000).copy(messageId = "<Twin@example.com>")
        val copy = message("sent-1", 1000).copy(messageId = "twin@example.com")
        val current = listOf(message("m0", 900), twin, copy, message("m2", 2000))

        assertEquals(listOf("m0"), messagesOlderThan(current, listOf(copy, message("m2", 2000))).map { it.id })
    }

    @Test
    fun pagesNotReadYetStayUntilTheRefreshReachesThem() {
        val current = listOf(message("m1", 1000), message("m2", 2000), message("m3", 3000), message("m4", 4000))
        val newestPage = listOf(message("m3", 3000), message("m4", 4000), message("m5", 5000))

        assertEquals(listOf("m1", "m2"), messagesOlderThan(current, newestPage).map { it.id })
        assertEquals(emptyList(), messagesOlderThan(current, current))
        assertEquals(emptyList(), messagesOlderThan(current, emptyList()))
    }

    @Test
    fun aMessageTheRefreshPassedOverIsNotKept() {
        // m2 was deleted: the refresh read past where it sat without finding it.
        val current = listOf(message("m1", 1000), message("m2", 2000), message("m3", 3000))
        val loaded = listOf(message("m0", 1500), message("m3", 3000))

        assertEquals(listOf("m1"), messagesOlderThan(current, loaded).map { it.id })
    }

    @Test
    fun olderPagesSurviveWhenNothingOnScreenWasReadAgain() {
        val current = listOf(message("local-draft-1", 500), message("m1", 1000), message("m2", 2000))

        assertEquals(listOf("m1"), messagesOlderThan(current, listOf(message("m9", 1500))).map { it.id })
    }

    private fun message(
        id: String,
        dateEpochSeconds: Long,
    ) = MessageBody(id = id, from = "Ada", to = "", subject = "Hello", body = "", dateEpochSeconds = dateEpochSeconds)
}
