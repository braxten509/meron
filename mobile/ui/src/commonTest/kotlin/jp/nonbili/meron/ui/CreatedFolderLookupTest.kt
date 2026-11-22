// After creating a folder the mobile picker must find the
// server's wire name. Tests exercise ASCII labels and
// decoded non-ASCII labels before issuing further folder
// commands.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.FolderSummary
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class CreatedFolderLookupTest {
    private fun folder(
        name: String,
        displayName: String = name,
    ) = FolderSummary(accountId = "acc1", name = name, displayName = displayName)

    @Test
    fun findsAnAsciiFolderWhoseWireNameIsItsLabel() {
        val folders = listOf(folder("INBOX"), folder("Receipts"))

        assertEquals("Receipts", folders.folderCreatedAs("Receipts")?.name)
        assertEquals("Receipts", folders.folderCreatedAs("receipts")?.name)
    }

    // The user types UTF-8 but the server reports (and addresses) the mailbox in
    // modified UTF-7, so matching on the wire name missed it entirely — the column
    // and the MOVE that follow then used a name the server does not know.
    @Test
    fun findsANonAsciiFolderByItsDecodedLabelAndReturnsTheWireName() {
        val folders = listOf(folder("INBOX"), folder(name = "t2/&MMYwuTDI-", displayName = "t2/テスト"))

        assertEquals("t2/&MMYwuTDI-", folders.folderCreatedAs("t2/テスト")?.name)
    }

    @Test
    fun returnsNullWhenTheListDoesNotContainTheFolder() {
        assertNull(listOf(folder("INBOX")).folderCreatedAs("Receipts"))
    }
}
