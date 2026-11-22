// A unified folder resolves separately for each provider
// and account. Tests cover the Inbox bootstrap path and
// missing special folders when a role cannot be resolved.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.FolderSummary
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

class UnifiedFoldersTest {
    private val folders =
        listOf(
            FolderSummary(accountId = "acc", name = "INBOX", role = "inbox"),
            FolderSummary(accountId = "acc", name = "Postausgang", role = "sent"),
        )

    @Test
    fun resolvesRolesToProviderMailboxNames() {
        assertEquals("Postausgang", unifiedAccountFolder(folders, "sent"))
    }

    @Test
    fun inboxHasABootstrapFallbackButMissingRolesAreSkipped() {
        assertEquals(INBOX_FOLDER, unifiedAccountFolder(emptyList(), "inbox"))
        assertNull(unifiedAccountFolder(folders, "archive"))
    }

    // Starred is the one switcher entry that is not a per-account mailbox, so
    // it must never resolve to one — the inbox fallback would silently show the
    // wrong list.
    @Test
    fun starredSitsNextToTheInboxInTheSwitcherButIsNoMailboxRole() {
        assertEquals(listOf(INBOX_FOLDER, STARRED_FOLDER), UNIFIED_VIEW_ROLES.take(2))
        assertTrue(isUnifiedStarredFolder("Starred"))
        assertFalse(isUnifiedStarredFolder("sent"))
        assertFalse(UNIFIED_FOLDER_ROLES.contains(STARRED_FOLDER))
    }

    @Test
    fun unifiedColumnsMatchProviderFolderNamesInMailEvents() {
        assertTrue(unifiedColumnMatchesFolder("sent", folders, "Postausgang"))
        assertFalse(unifiedColumnMatchesFolder("sent", folders, "INBOX"))
        assertTrue(unifiedColumnMatchesFolder(STARRED_FOLDER, folders, "INBOX"))
    }

    // A thread spanning INBOX and Sent must only take its own folder's reads
    // off each card, however the card names its folder.
    @Test
    fun threadCardCountsOnlyItsOwnFolder() {
        assertTrue(threadCardCoversFolder("INBOX", folders, "INBOX"))
        assertFalse(threadCardCoversFolder("INBOX", folders, "Postausgang"))
        assertFalse(threadCardCoversFolder("Postausgang", folders, "INBOX"))
        // Opened from a unified Kanban column the card carries the column's
        // role, which resolves to this account's own mailbox.
        assertTrue(threadCardCoversFolder("sent", folders, "Postausgang"))
        assertFalse(threadCardCoversFolder("sent", folders, "INBOX"))
        assertTrue(threadCardCoversFolder("inbox", folders, "INBOX"))
        // An unresolvable role must not fall back to the inbox and count it.
        assertFalse(threadCardCoversFolder("archive", folders, "INBOX"))
        // A starred column id is no mailbox: it must not cover other folders,
        // or reading a Sent reply would clear a starred INBOX card.
        assertFalse(threadCardCoversFolder(STARRED_FOLDER, folders, "Postausgang"))
        // A blank message folder is the thread's own.
        assertTrue(threadCardCoversFolder("INBOX", folders, ""))
    }
}
