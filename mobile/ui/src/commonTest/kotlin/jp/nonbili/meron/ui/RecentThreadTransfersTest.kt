// The mobile destination picker persists recent moves
// and copies. Tests cover account and action identity,
// repeated destinations and malformed stored history.

package jp.nonbili.meron.ui

import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class RecentThreadTransfersTest {
    @Test
    fun historyRoundTripsLocallyAndKeepsActionAndAccountDistinct() {
        val entries =
            listOf(
                RecentThreadTransfer("move", "a", "a", "Work"),
                RecentThreadTransfer("copy", "a", "a", "Work"),
                RecentThreadTransfer("copy", "b", "a", "Work"),
            )
        val prefs = MemoryAppPreferences()
        prefs.putString("recent_thread_transfers", Json.encodeToString(entries))
        assertEquals(entries, loadRecentThreadTransfers(prefs))
    }

    @Test
    fun invalidHistoryIsIgnoredAndDuplicatesAreRemoved() {
        assertEquals(emptyList(), parseRecentThreadTransfers("{"))
        assertEquals(emptyList(), parseRecentThreadTransfers("{}"))
        val valid = RecentThreadTransfer("move", "a", "a", "Work")
        val entries = listOf(valid.copy(action = "delete"), valid.copy(targetFolderId = ""), valid.copy(targetAccountId = "b"), valid, valid)
        assertEquals(listOf(valid.copy(targetAccountId = "b"), valid), parseRecentThreadTransfers(Json.encodeToString(entries)))
    }

    @Test
    fun historyIsBounded() {
        val entries = (1..110).map { RecentThreadTransfer("copy", "a", "b", "Folder $it") }
        assertEquals(entries.take(100), parseRecentThreadTransfers(Json.encodeToString(entries)))
    }

    @Test
    fun zeroAffectedMessagesIsNotASuccess() {
        assertFailsWith<IllegalStateException> { requireTransferAffected("""{"moved":0}""", "moved") }
        assertFailsWith<IllegalStateException> { requireTransferAffected("""{"copied":0}""", "copied") }
        assertEquals("""{"moved":2}""", requireTransferAffected("""{"moved":2}""", "moved"))
    }
}
