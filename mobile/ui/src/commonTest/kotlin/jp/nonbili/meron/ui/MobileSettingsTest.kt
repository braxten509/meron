// Settings export includes preferences the reader has
// explicitly set. The cases distinguish an absent
// preference from a stored false value when gathering
// mobile settings.

package jp.nonbili.meron.ui

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class MobileSettingsTest {
    @Test
    fun collectsOnlySettingsTheUserHasActuallySet() {
        val app = FakePreferences()
        val kanban = FakePreferences()
        app.putString(APPEARANCE_MODE_PREF, "Indigo")
        app.putString(APP_LANGUAGE_PREF, "ja")
        app.putInt(MESSAGE_FONT_SCALE_PREF, 115)
        app.putBoolean(SHOW_UNREAD_BADGES_PREF, true)
        app.putStringSet(HIDDEN_NAV_ACCOUNTS_PREF, setOf("a@example.com"))
        kanban.putString(KANBAN_BOARDS_PREF, """[{"id":"b1"}]""")

        val collected = collectCachedSettings(app, kanban)

        assertEquals("Indigo", collected["mobile.app.$APPEARANCE_MODE_PREF"])
        assertEquals("ja", collected["mobile.app.$APP_LANGUAGE_PREF"])
        assertEquals(115, collected["mobile.app.$MESSAGE_FONT_SCALE_PREF"])
        assertEquals(true, collected["mobile.app.$SHOW_UNREAD_BADGES_PREF"])
        assertEquals(listOf("a@example.com"), collected["mobile.app.$HIDDEN_NAV_ACCOUNTS_PREF"])
        assertEquals("""[{"id":"b1"}]""", collected["mobile.kanban.$KANBAN_BOARDS_PREF"])
        // Untouched settings stay out, so a restore can't write defaults over
        // choices the target device already made.
        assertFalse(collected.containsKey("mobile.app.$CONVERSATION_LAYOUT_PREF"))
        assertFalse(collected.containsKey("mobile.app.$LIVE_MAIL_PUSH_PREF"))
    }

    /** A `false` the user chose must survive; it is not the same as "unset". */
    @Test
    fun anExplicitFalseIsCollected() {
        val app = FakePreferences()
        app.putBoolean(LIVE_MAIL_PUSH_PREF, false)

        val collected = collectCachedSettings(app, FakePreferences())

        assertTrue(collected.containsKey("mobile.app.$LIVE_MAIL_PUSH_PREF"))
        assertEquals(false, collected["mobile.app.$LIVE_MAIL_PUSH_PREF"])
    }

    /** Likewise an int that happens to equal a probe sentinel. */
    @Test
    fun anExplicitZeroIsCollected() {
        val app = FakePreferences()
        app.putInt(POLL_INTERVAL_MINUTES_PREF, 0)

        val collected = collectCachedSettings(app, FakePreferences())

        assertEquals(0, collected["mobile.app.$POLL_INTERVAL_MINUTES_PREF"])
    }

    @Test
    fun sessionAndOAuthStateIsNeverBackedUp() {
        val app = FakePreferences()
        app.putString(LAST_MAIL_ACCOUNT_PREF, "a@example.com")
        app.putString(LAST_MAIL_FOLDER_PREF, "INBOX")
        app.putString(LAST_TOP_SCREEN_PREF, "kanban")
        app.putString(OAUTH_PENDING_STATE_PREF, "state-token")
        app.putString(OAUTH_PENDING_VERIFIER_PREF, "verifier")
        val kanban = FakePreferences()
        kanban.putString(KANBAN_SEARCH_PREF, "invoice")

        val collected = collectCachedSettings(app, kanban)

        assertEquals(emptyMap(), collected)
    }

    @Test
    fun writingToTheCacheHandlesEveryType() {
        val app = FakePreferences()
        val kanban = FakePreferences()

        val applied: Map<String, Any> =
            writeSettingsToCache(
                app,
                kanban,
                mapOf(
                    "mobile.app.$APPEARANCE_MODE_PREF" to "Indigo",
                    "mobile.app.$MESSAGE_FONT_SCALE_PREF" to 115L,
                    "mobile.app.$SHOW_UNREAD_BADGES_PREF" to true,
                    "mobile.app.$HIDDEN_NAV_ACCOUNTS_PREF" to listOf("a@example.com"),
                    "mobile.kanban.$KANBAN_BOARDS_PREF" to """[{"id":"b1"}]""",
                ),
            )

        assertEquals(5, applied.size)
        assertEquals("Indigo", app.getString(APPEARANCE_MODE_PREF, ""))
        // A JSON number arrives as Long and still lands in an Int pref.
        assertEquals(115, app.getInt(MESSAGE_FONT_SCALE_PREF, 0))
        assertEquals(true, app.getBoolean(SHOW_UNREAD_BADGES_PREF, false))
        assertEquals(setOf("a@example.com"), app.getStringSet(HIDDEN_NAV_ACCOUNTS_PREF, emptySet()))
        assertEquals("""[{"id":"b1"}]""", kanban.getString(KANBAN_BOARDS_PREF, ""))
    }

    @Test
    fun writingToTheCacheIgnoresUnknownKeysAndWrongTypes() {
        val app = FakePreferences()

        val applied: Map<String, Any> =
            writeSettingsToCache(
                app,
                FakePreferences(),
                mapOf(
                    // A key from a newer build.
                    "mobile.app.some_future_setting_v9" to "value",
                    // Right key, wrong type.
                    "mobile.app.$MESSAGE_FONT_SCALE_PREF" to "not a number",
                    "mobile.app.$SHOW_UNREAD_BADGES_PREF" to "true",
                ),
            )

        assertEquals(0, applied.size)
        assertEquals(0, app.getInt(MESSAGE_FONT_SCALE_PREF, 0))
        assertFalse(app.getBoolean(SHOW_UNREAD_BADGES_PREF, false))
    }

    @Test
    fun collectAndApplyRoundTrip() {
        val app = FakePreferences()
        val kanban = FakePreferences()
        app.putString(APPEARANCE_MODE_PREF, "Indigo")
        app.putString(APP_LANGUAGE_PREF, "ja")
        app.putString(CONVERSATION_LAYOUT_PREF, "traditional")
        app.putInt(POLL_INTERVAL_MINUTES_PREF, 30)
        app.putBoolean(BACKGROUND_SYNC_ENABLED_PREF, false)
        app.putStringSet(HIDDEN_NAV_ACCOUNTS_PREF, setOf("x@example.com", "y@example.com"))
        kanban.putString(ACTIVE_KANBAN_BOARD_PREF, "board-1")

        val collected = collectCachedSettings(app, kanban)
        val restoredApp = FakePreferences()
        val restoredKanban = FakePreferences()
        writeSettingsToCache(restoredApp, restoredKanban, collected)

        assertEquals(collected, collectCachedSettings(restoredApp, restoredKanban))
    }

    private class FakePreferences : AppPreferences {
        private val strings = mutableMapOf<String, String>()
        private val booleans = mutableMapOf<String, Boolean>()
        private val ints = mutableMapOf<String, Int>()
        private val stringSets = mutableMapOf<String, Set<String>>()

        override fun getString(
            key: String,
            default: String,
        ): String = strings[key] ?: default

        override fun putString(
            key: String,
            value: String,
        ) {
            strings[key] = value
        }

        override fun getBoolean(
            key: String,
            default: Boolean,
        ): Boolean = booleans[key] ?: default

        override fun putBoolean(
            key: String,
            value: Boolean,
        ) {
            booleans[key] = value
        }

        override fun getInt(
            key: String,
            default: Int,
        ): Int = ints[key] ?: default

        override fun putInt(
            key: String,
            value: Int,
        ) {
            ints[key] = value
        }

        override fun getStringSet(
            key: String,
            default: Set<String>,
        ): Set<String> = stringSets[key] ?: default

        override fun putStringSet(
            key: String,
            value: Set<String>,
        ) {
            stringSets[key] = value
        }

        override fun remove(key: String) {
            strings.remove(key)
            booleans.remove(key)
            ints.remove(key)
            stringSets.remove(key)
        }
    }
}
