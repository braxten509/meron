// Mailbox badge counts depend on cached folder identity
// and sync state. The fixtures exercise a never-fetched
// column and case-insensitive matching for the Inbox.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.FolderSummary
import jp.nonbili.meron.shared.ThreadSummary
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class UnreadCountHelpersTest {
    @Test
    fun syncsAnEmptyNeverFetchedMailColumnOnCacheOnlyLoad() {
        val column = KanbanColumnSpec(accountId = "acc1", folderId = "Archive")
        val result = MailboxLoadResult(emptyList(), "Archive", emptyList(), folderSynced = false)

        assertTrue(
            shouldSyncUnfetchedKanbanColumn(
                column = column,
                refresh = false,
                query = "",
                result = result,
                accounts = listOf(AccountSummary(id = "acc1", email = "acc1@example.com")),
            ),
        )
        assertFalse(
            shouldSyncUnfetchedKanbanColumn(
                column = column,
                refresh = false,
                query = "",
                result = result.copy(folderSynced = true),
                accounts = listOf(AccountSummary(id = "acc1", email = "acc1@example.com")),
            ),
        )
    }

    @Test
    fun comparesInboxFolderIdsCaseInsensitively() {
        assertTrue(kanbanFolderIdsEqual("inbox", "INBOX"))
        assertFalse(kanbanFolderIdsEqual("archive", "Archive"))
    }

    @Test
    fun folderUnreadTreatsInboxCaseInsensitively() {
        val folders =
            listOf(
                FolderSummary(accountId = "acc1", name = "INBOX", unread = 12),
                FolderSummary(accountId = "acc1", name = "Archive", unread = 5),
            )

        assertEquals(12, folderUnread(folders, "inbox"))
        assertEquals(5, folderUnread(folders, "Archive"))
        assertEquals(0, folderUnread(folders, "archive"))
    }

    @Test
    fun kanbanColumnUnreadUsesTotalReturnedWithPage() {
        val count =
            kanbanColumnUnreadCount(
                column = KanbanColumnSpec(accountId = "acc1", folderId = "inbox"),
                folderUnread = 137,
                loadedThreads =
                    listOf(
                        thread("t1", unread = true),
                        thread("t2", unread = true),
                    ),
            )

        assertEquals(137, count)
    }

    @Test
    fun kanbanColumnUnreadUsesSummedUnifiedTotalReturnedWithPage() {
        val count =
            kanbanColumnUnreadCount(
                column = KanbanColumnSpec(accountId = UNIFIED_ACCOUNT_ID, folderId = INBOX_FOLDER),
                folderUnread = 70,
            )

        assertEquals(70, count)
    }

    @Test
    fun kanbanColumnUnreadTrustsGenuineZeroFolderTotal() {
        val count =
            kanbanColumnUnreadCount(
                column = KanbanColumnSpec(accountId = "acc1", folderId = "inbox"),
                folderUnread = 0,
                loadedThreads = listOf(thread("t1", unread = true)),
            )

        assertEquals(0, count)
    }

    @Test
    fun kanbanColumnUnreadFallsBackToLoadedMessageTotals() {
        val count =
            kanbanColumnUnreadCount(
                column = KanbanColumnSpec(accountId = "acc1", folderId = INBOX_FOLDER),
                folderUnread = null,
                loadedThreads = listOf(thread("t1", unread = true, unreadCount = 2), thread("t2", unread = false)),
            )

        assertEquals(2, count)
    }

    private fun thread(
        id: String,
        unread: Boolean,
        unreadCount: Int = if (unread) 1 else 0,
    ): ThreadSummary =
        ThreadSummary(
            id = id,
            accountId = "acc1",
            folder = "INBOX",
            subject = "Subject",
            sender = "sender@example.com",
            unread = unread,
            unreadCount = unreadCount,
        )
}
