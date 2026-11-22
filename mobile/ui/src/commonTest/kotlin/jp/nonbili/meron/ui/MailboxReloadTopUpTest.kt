// Refreshing the visible mailbox must fetch enough pages to
// cover the previous view. Tests distinguish an open list
// from a background mailbox that needs only its first page.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.ThreadSummary
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals

class MailboxReloadTopUpTest {
    private fun thread(
        id: String,
        date: Long,
    ) = ThreadSummary(id = id, accountId = "a", folder = "INBOX", subject = id, sender = "s", dateEpochSeconds = date)

    private fun page(
        cursor: String,
        vararg threads: ThreadSummary,
    ) = MailboxLoadResult(folders = emptyList(), folder = "INBOX", threads = threads.toList(), nextCursor = cursor)

    // Each request answers one row whatever its limit — as an attachments
    // filter or long threads can — so the page depth alone falls short.
    private val pages = mapOf("c1" to page("c2", thread("t2", 200)), "c2" to page("", thread("t1", 100)))

    @Test
    fun readsOnUntilTheOldestRowShownIsBack() =
        runBlocking<Unit> {
            val asked = mutableListOf<String>()
            val (result, depth) =
                readMailboxToOldestShown(page("c1", thread("t3", 300)), listLimit = 100, oldestShown = 200) { cursor, _ ->
                    asked += cursor
                    pages.getValue(cursor)
                }
            assertEquals(listOf("c1"), asked)
            assertEquals(listOf("t3", "t2"), result.threads.map { it.id })
            assertEquals("c2", result.nextCursor)
            // Reading on does not deepen the next reload: the depth stays what
            // was on screen, or every refresh would add a page of older mail.
            assertEquals(100, depth)
        }

    @Test
    fun aListNotOnScreenReadsOnePage() =
        runBlocking<Unit> {
            val (result, depth) =
                readMailboxToOldestShown(page("c1", thread("t3", 300)), listLimit = MAILBOX_PAGE_SIZE, oldestShown = null) { cursor, _ ->
                    pages.getValue(cursor)
                }
            assertEquals(listOf("t3"), result.threads.map { it.id })
            assertEquals(MAILBOX_PAGE_SIZE, depth)
        }
}
