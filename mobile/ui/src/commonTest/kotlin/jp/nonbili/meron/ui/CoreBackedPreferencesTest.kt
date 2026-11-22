// Mobile preferences are cached locally and persisted by
// the core. These cases cover changes made before
// hydration and language selections received from the
// platform.

package jp.nonbili.meron.ui

import jp.nonbili.meron.shared.CloseableHandle
import jp.nonbili.meron.shared.CoreEvent
import jp.nonbili.meron.shared.CoreEventStream
import jp.nonbili.meron.shared.MeronCore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlin.coroutines.Continuation
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.coroutines.startCoroutine
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * The platform store is a write-through cache in front of the core `settings`
 * table. These pin the two properties that makes it safe: writes reach both, and
 * a hydrate never silently rolls back something the user just changed.
 */
class CoreBackedPreferencesTest {
    private val appearanceKey = "mobile.app.$APPEARANCE_MODE_PREF"
    private val languageKey = "mobile.app.$APP_LANGUAGE_PREF"

    @Test
    fun writesReachBothTheCacheAndTheCore() {
        val cache = FakePreferences()
        val core = FakeCoreStore()
        val prefs = CoreBackedPreferences(cache, PrefStore.App, CoroutineScope(Dispatchers.Unconfined), testMirror(core))

        prefs.putString(APPEARANCE_MODE_PREF, "Indigo")
        prefs.putInt(MESSAGE_FONT_SCALE_PREF, 115)
        prefs.putBoolean(BACKGROUND_SYNC_ENABLED_PREF, false)
        prefs.putStringSet(HIDDEN_NAV_ACCOUNTS_PREF, setOf("a@example.com"))

        // The cache is updated synchronously, so a read right after a write
        // never blocks on the core.
        assertEquals("Indigo", cache.getString(APPEARANCE_MODE_PREF, ""))
        assertEquals(115, cache.getInt(MESSAGE_FONT_SCALE_PREF, 0))

        val written = core.writes()
        assertEquals(""""Indigo"""", written["mobile.app.$APPEARANCE_MODE_PREF"])
        assertEquals("115", written["mobile.app.$MESSAGE_FONT_SCALE_PREF"])
        assertEquals("false", written["mobile.app.$BACKGROUND_SYNC_ENABLED_PREF"])
        assertEquals("""["a@example.com"]""", written["mobile.app.$HIDDEN_NAV_ACCOUNTS_PREF"])
    }

    // ---- Write ordering -----------------------------------------------------

    /**
     * The path MeronApp takes when Android reports a language chosen from system
     * settings: an ordinary write through the store, which stages, journals and
     * mirrors it. Nothing special is needed because nothing bypasses the store
     * any more — LocaleController no longer persists anything.
     */
    @Test
    fun anExternalLanguageChangeIsPersistedAndSurvivesHydration() {
        val app = FakePreferences()
        val core = FakeCoreStore(languageKey to """"en"""")
        val mirror = testMirror(core)
        val prefs = CoreBackedPreferences(app, PrefStore.App, CoroutineScope(Dispatchers.Unconfined), mirror)

        saveAppLanguageTag(prefs, "pt-BR")

        assertEquals("pt-BR", app.getString(APP_LANGUAGE_PREF, ""))
        assertEquals(""""pt-BR"""", core.rowFor(languageKey))

        // The hydrate that follows startup must not read the old row as newer.
        val changed = runSuspend { hydrateSettingsFromCore(app, FakePreferences(), mirror) }
        assertTrue(changed.isEmpty(), "$changed")
        assertEquals("pt-BR", app.getString(APP_LANGUAGE_PREF, ""))
    }

    /** An unsupported tag normalizes to "follow the system" rather than sticking. */
    @Test
    fun anUnsupportedLanguageTagIsStoredAsBlank() {
        val app = FakePreferences()
        val prefs =
            CoreBackedPreferences(app, PrefStore.App, CoroutineScope(Dispatchers.Unconfined), testMirror(FakeCoreStore()))

        saveAppLanguageTag(prefs, "kl")

        assertEquals("", app.getString(APP_LANGUAGE_PREF, "unset"))
    }

    /**
     * The end of the reset path: storing blank clears the language, and that
     * reaches the table like any other setting.
     */
    @Test
    fun resettingTheLanguageToSystemDefaultIsMirrored() {
        val app = FakePreferences()
        val core = FakeCoreStore(languageKey to """"pt-BR"""")
        val mirror = testMirror(core)
        val prefs = CoreBackedPreferences(app, PrefStore.App, CoroutineScope(Dispatchers.Unconfined), mirror)
        app.putString(APP_LANGUAGE_PREF, "pt-BR")

        saveAppLanguageTag(prefs, "")

        assertEquals("", app.getString(APP_LANGUAGE_PREF, "unset"))
        assertEquals("""""""", core.rowFor(languageKey))
    }

    private fun testMirror(
        core: MeronCore,
        loaded: Boolean = true,
    ) = SettingsMirror(core, FakePreferences(), Dispatchers.Unconfined) { loaded }

    /** Runs a suspend block inline; every dispatcher here is Unconfined. */
    private fun <T> runSuspend(block: suspend () -> T): T {
        var value: T? = null
        var error: Throwable? = null
        block.startCoroutine(
            Continuation(EmptyCoroutineContext) {
                it.onSuccess { output -> value = output }
                it.onFailure { thrown -> error = thrown }
            },
        )
        error?.let { throw it }
        @Suppress("UNCHECKED_CAST")
        return value as T
    }

    /**
     * A core that actually stores what it is told, so `prefsGet` reflects earlier
     * `prefsSet` calls. A fixed canned response would make a mirrored write look
     * like a conflicting one and hide real regressions.
     */
    private class FakeCoreStore(
        vararg initial: Pair<String, String>,
    ) : MeronCore {
        private val rows = initial.toMap().toMutableMap()
        private val written = mutableMapOf<String, String>()
        var failWrites: Boolean = false

        /** Runs inside a write, to interleave a concurrent edit deterministically. */
        var onWrite: ((String) -> Unit)? = null

        /** Raw JSON of a row, or null. */
        fun rowFor(settingKey: String): String? = rows[settingKey]

        /** Only the rows written through the mirror during the test. */
        fun writes(): Map<String, String> = written.toMap()

        fun setRow(
            settingKey: String,
            rawJson: String,
        ) {
            rows[settingKey] = rawJson
        }

        override suspend fun invoke(
            command: String,
            payloadJson: String,
        ): String =
            when (command) {
                "app.prefsSet" -> {
                    if (failWrites) throw IllegalStateException("write failed")
                    val key = Regex(""""key":"([^"]+)"""").find(payloadJson)!!.groupValues[1]
                    val value = payloadJson.substringAfter(""""value":""").removeSuffix("}")
                    rows[key] = value
                    written[key] = value
                    onWrite?.invoke(value)
                    "{}"
                }

                "app.prefsGet" -> {
                    rows.entries.joinToString(",", """{"prefs":{""", "}}") { (key, value) ->
                        """"$key":$value"""
                    }
                }

                else -> {
                    "{}"
                }
            }

        override fun events(): CoreEventStream =
            object : CoreEventStream {
                override fun subscribe(listener: (CoreEvent) -> Unit): CloseableHandle = CloseableHandle {}
            }

        override suspend fun protocolVersion(): Int = 0
    }

    private class ThrowingCore : MeronCore {
        override suspend fun invoke(
            command: String,
            payloadJson: String,
        ): String = throw IllegalStateException("core is down")

        override fun events(): CoreEventStream =
            object : CoreEventStream {
                override fun subscribe(listener: (CoreEvent) -> Unit): CloseableHandle = CloseableHandle {}
            }

        override suspend fun protocolVersion(): Int = 0
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
