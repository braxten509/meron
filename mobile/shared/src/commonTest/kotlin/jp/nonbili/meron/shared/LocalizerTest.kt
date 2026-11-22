// Shared UI strings may contain named parameters or be missing
// from a translation. These tests exercise how the map-backed
// localizer formats an argument and falls back to a stable key.

package jp.nonbili.meron.shared

import kotlin.test.Test
import kotlin.test.assertEquals

class LocalizerTest {
    @Test
    fun mapLocalizerFormatsSimpleNamedArguments() {
        val localizer = MapLocalizer(mapOf("mobile.mail.needsReconnect" to "{account} needs credentials"))

        assertEquals(
            "Personal needs credentials",
            localizer.text("mobile.mail.needsReconnect", mapOf("account" to "Personal")),
        )
    }

    @Test
    fun missingKeysFallBackToStableKey() {
        val localizer = MapLocalizer(emptyMap())

        assertEquals("missing.key", localizer.text("missing.key"))
    }
}
