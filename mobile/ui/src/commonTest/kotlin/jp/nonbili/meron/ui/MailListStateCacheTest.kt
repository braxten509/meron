// The mobile mailbox retains scroll state across folder
// changes with a bounded cache. Tests distinguish revisiting
// an existing entry from evicting a state to add a new
// mailbox.

package jp.nonbili.meron.ui

import kotlin.test.Test
import kotlin.test.assertEquals

class MailListStateCacheTest {
    @Test
    fun revisitingRememberedMailboxAtCapacityDoesNotEvictAnotherState() {
        val keys = (1..12).map(::key)

        assertEquals(
            emptyList(),
            rememberedMailListKeysToEvict(
                existingKeys = keys,
                requestedKey = keys.last(),
                maxSize = 12,
            ),
        )
    }

    @Test
    fun addingMailboxAtCapacityEvictsOldestState() {
        val keys = (1..12).map(::key)

        assertEquals(
            listOf(keys.first()),
            rememberedMailListKeysToEvict(
                existingKeys = keys,
                requestedKey = key(13),
                maxSize = 12,
            ),
        )
    }

    private fun key(index: Int): MailboxCacheKey =
        mailboxCacheKey(
            accountId = "account-$index",
            folderId = INBOX_FOLDER,
            query = "",
            filter = FilterMode.All,
        )
}
