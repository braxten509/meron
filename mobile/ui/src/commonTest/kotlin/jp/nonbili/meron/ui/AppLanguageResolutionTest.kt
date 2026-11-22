// The mobile UI resolves a stored language choice alongside the
// OS preference. These cases cover platform precedence,
// supported regional tags and the fallback for unavailable
// languages.

package jp.nonbili.meron.ui

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * The platform's answer about the in-app language has three states, and
 * collapsing the last two is a real bug: Android 13+ reports "system default" as
 * an empty tag, which is an authoritative choice rather than an absent one.
 */
class AppLanguageResolutionTest {
    @Test
    fun noPlatformOpinionLeavesTheStoredTagInCharge() {
        // Android below 13 and iOS: nothing owns the setting but us.
        assertEquals("ru", resolveAppLanguageTag(systemTag = null, storedTag = "ru"))
        assertEquals("", resolveAppLanguageTag(systemTag = null, storedTag = ""))
        assertFalse(appLanguageNeedsPersisting(systemTag = null, storedTag = "ru"))
    }

    @Test
    fun aPlatformChosenLanguageWins() {
        assertEquals("ru", resolveAppLanguageTag(systemTag = "ru", storedTag = ""))
        assertEquals("ru", resolveAppLanguageTag(systemTag = "ru", storedTag = "fr"))
        assertTrue(appLanguageNeedsPersisting(systemTag = "ru", storedTag = "fr"))
    }

    /**
     * The regression this guards: pick Russian in the app, then set the app's
     * language back to "System default" in Android settings. The empty tag has to
     * override the stored "ru", not be read as "the platform has no opinion".
     */
    @Test
    fun resettingToSystemDefaultOverridesAStoredLanguage() {
        assertEquals("", resolveAppLanguageTag(systemTag = "", storedTag = "ru"))
        assertTrue(
            appLanguageNeedsPersisting(systemTag = "", storedTag = "ru"),
            "the reset has to be written, or it comes back on the next launch",
        )
    }

    @Test
    fun anUnchangedPlatformChoiceIsNotRewritten() {
        assertFalse(appLanguageNeedsPersisting(systemTag = "ru", storedTag = "ru"))
        assertFalse(appLanguageNeedsPersisting(systemTag = "", storedTag = ""))
    }

    // ---- Device locale fallback ---------------------------------------------
    //
    // What "system default" resolves to once no language is chosen for the app.
    // These mirror resolveI18nLanguageFromWebLocale in the desktop frontend, so
    // the same device gets the same translation on desktop and mobile.

    @Test
    fun regionalAndStoredLanguageTagsUseTheThreeCatalogs() {
        assertEquals("ru", resolveDeviceLanguageTag("RU_BY"))
        assertEquals("en", resolveDeviceLanguageTag("en-GB"))
        assertEquals("en", resolveDeviceLanguageTag("ja-JP"))
        assertEquals("", resolveAppLanguageTag(null, "ja"))
        for (tag in listOf("pt", "pt_BR", "PT-pt")) {
            assertEquals("pt-BR", resolveDeviceLanguageTag(tag))
            assertEquals("pt-BR", resolveAppLanguageTag(null, tag))
        }
    }
}
