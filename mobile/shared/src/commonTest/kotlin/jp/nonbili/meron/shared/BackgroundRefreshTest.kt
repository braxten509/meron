// Background scheduling depends on an active, connected
// account. The shared tests cover account eligibility and
// selecting the RSS refresh command from account metadata.

package jp.nonbili.meron.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class BackgroundRefreshTest {
    @Test
    fun accountEligibilityRequiresActiveConnectedAccount() {
        assertTrue(
            shouldBackgroundRefreshAccount(
                accountId = "user1@mail.localhost",
                paused = false,
                needsReconnect = false,
            ),
        )
        assertFalse(shouldBackgroundRefreshAccount(accountId = "", paused = false, needsReconnect = false))
        assertFalse(shouldBackgroundRefreshAccount(accountId = "a1", paused = true, needsReconnect = false))
        assertFalse(shouldBackgroundRefreshAccount(accountId = "a1", paused = false, needsReconnect = true))
    }

    @Test
    fun rssAccountMetadataUsesRssRefreshProtocol() {
        assertTrue(backgroundRefreshUsesRssProtocol(engine = "rss", provider = "custom", authType = "password"))
        assertTrue(backgroundRefreshUsesRssProtocol(engine = "mail", provider = "rss", authType = "password"))
        assertTrue(backgroundRefreshUsesRssProtocol(engine = "mail", provider = "custom", authType = "rss"))
        assertFalse(backgroundRefreshUsesRssProtocol(engine = "mail", provider = "gmail", authType = "gmail_oauth"))
    }

    @Test
    fun pollIntervalCyclesThroughKnownOptions() {
        assertEquals(5, nextPollIntervalMinutes(0))
        assertEquals(15, nextPollIntervalMinutes(5))
        assertEquals(0, nextPollIntervalMinutes(60))
        // Unknown stored values snap to 15 before advancing.
        assertEquals(15, coercePollIntervalMinutes(7))
        assertEquals(30, coercePollIntervalMinutes(30))
        assertEquals(30, nextPollIntervalMinutes(7))
    }

    @Test
    fun backgroundDelayFloorsAtFifteenMinutes() {
        assertEquals(15 * 60.0, backgroundRefreshDelaySeconds(0))
        assertEquals(15 * 60.0, backgroundRefreshDelaySeconds(5))
        assertEquals(30 * 60.0, backgroundRefreshDelaySeconds(30))
    }

    @Test
    fun summaryPicksTheNotificationMessage() {
        assertEquals("mobile.refresh.refreshedWithFailures", backgroundRefreshSummaryKey(refreshed = 2, skipped = 0, failed = 1))
        assertEquals("mobile.refresh.refreshed", backgroundRefreshSummaryKey(refreshed = 2, skipped = 0, failed = 0))
        assertEquals("mobile.refresh.failed", backgroundRefreshSummaryKey(refreshed = 0, skipped = 0, failed = 2))
        assertEquals("mobile.refresh.skipped", backgroundRefreshSummaryKey(refreshed = 0, skipped = 3, failed = 0))
        assertEquals("mobile.refresh.noAccounts", backgroundRefreshSummaryKey(refreshed = 0, skipped = 0, failed = 0))
    }
}
